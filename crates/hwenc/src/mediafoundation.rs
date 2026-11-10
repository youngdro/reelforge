//! Media Foundation (Windows): the H.264 and HEVC encoder transforms, the graphics card's first
//! (Intel Quick Sync, NVIDIA, AMD), Microsoft's software H.264 encoder when there is none.
//!
//! Frames go in as NV12 samples and come out as Annex B access units, which are taken apart into
//! MP4 samples and the parameter sets of the configuration record ([`crate::nal`]). Hardware
//! transforms are asynchronous: they say with events when they want a frame and when they have
//! one; software ones are asked directly.

use std::mem::ManuallyDrop;
use std::sync::OnceLock;

use reelforge_media::{EncodedFrame, ExportError, VideoConfig, VideoEncoder, Yuv420};
use reelforge_renderspec::{VideoCodec, VideoEncoding};
use reelforge_time::FrameRate;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::{GUID, Interface, PWSTR};

use crate::nal::{ParameterSets, is_keyframe};

/// A Media Foundation encoder transform.
pub(crate) struct Encoder {
    transform: IMFTransform,
    /// The transform's events, when it is asynchronous.
    events: Option<IMFMediaEventGenerator>,
    /// Frames an asynchronous transform has asked for and not been given.
    wanted: u32,
    drained: bool,
    ids: (u32, u32),
    size: [u32; 2],
    fps: FrameRate,
    codec: VideoCodec,
    label: String,
    sets: ParameterSets,
    out: Vec<EncodedFrame>,
    last_pts: Option<i64>,
    sent: u64,
    got: u64,
}

fn failed(what: &str, e: windows::core::Error) -> ExportError {
    ExportError::Encode(format!("Media Foundation: {what}: {e}"))
}

/// Starts Media Foundation once for the process, and COM on this thread. Neither is stopped: both
/// are counted, and an encoder may be dropped on another thread than the one that opened it.
fn start() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    // a thread already in another apartment is fine too: the transforms are free-threaded
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    *STARTED.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.is_ok())
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn number(value: u32) -> VARIANT {
    let mut v = VARIANT::default();
    // SAFETY: a default VARIANT is VT_EMPTY; the type and the value are set together
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = value;
    }
    v
}

fn flag(value: bool) -> VARIANT {
    let mut v = VARIANT::default();
    // SAFETY: as in `number`
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = value.into();
    }
    v
}

impl Encoder {
    /// The first encoder transform for these frames that takes the settings, hardware ones first.
    pub(crate) fn open(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Result<Encoder, ExportError> {
        let subtype = match venc.codec {
            VideoCodec::H264 => MFVideoFormat_H264,
            VideoCodec::Hevc => MFVideoFormat_HEVC,
            VideoCodec::Av1 => return Err(ExportError::Encode("Media Foundation: no AV1 here".into())),
        };
        if !size[0].is_multiple_of(2) || !size[1].is_multiple_of(2) {
            return Err(ExportError::Encode("Media Foundation: odd picture sizes are not encoded".into()));
        }
        if !start() {
            return Err(ExportError::Encode("Media Foundation did not start".into()));
        }
        let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
        let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        let flags =
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER;
        unsafe { MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, Some(&input), Some(&output), &mut list, &mut count) }
            .map_err(|e| failed("no encoders listed", e))?;
        // SAFETY: `list` is `count` activation objects in memory that is ours to free
        let found: Vec<IMFActivate> = unsafe {
            let all = if list.is_null() { &mut [][..] } else { std::slice::from_raw_parts_mut(list, count as usize) };
            let found = all.iter_mut().filter_map(Option::take).collect();
            CoTaskMemFree(Some(list as *const _));
            found
        };
        let mut why = ExportError::Encode(format!("Media Foundation: no {:?} encoder", venc.codec));
        for activate in found {
            match unsafe { Encoder::with(&activate, subtype, size, fps, venc) } {
                Ok(enc) => return Ok(enc),
                Err(e) => {
                    why = e;
                    let _ = unsafe { activate.ShutdownObject() };
                }
            }
        }
        Err(why)
    }

    /// Activates one transform and gives it the picture types and settings.
    unsafe fn with(
        activate: &IMFActivate,
        subtype: GUID,
        size: [u32; 2],
        fps: FrameRate,
        venc: &VideoEncoding,
    ) -> Result<Encoder, ExportError> {
        unsafe {
            let mut label = String::from("Media Foundation");
            let (mut name, mut len) = (PWSTR::null(), 0u32);
            if activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name, &mut len).is_ok() && !name.is_null()
            {
                label = name.to_string().unwrap_or(label);
                CoTaskMemFree(Some(name.0 as *const _));
            }
            let transform: IMFTransform = activate.ActivateObject().map_err(|e| failed("encoder not started", e))?;
            let attributes = transform.GetAttributes().ok();
            let asynchronous =
                attributes.as_ref().is_some_and(|a| a.GetUINT32(&MF_TRANSFORM_ASYNC).is_ok_and(|v| v != 0));
            if let (true, Some(a)) = (asynchronous, &attributes) {
                a.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(|e| failed("encoder not unlocked", e))?;
            }
            // most transforms have one stream each way, numbered 0, and do not implement this
            let (mut ins, mut outs) = ([0u32; 1], [0u32; 1]);
            let ids = match transform.GetStreamIDs(&mut ins, &mut outs) {
                Ok(()) => (ins[0], outs[0]),
                Err(_) => (0, 0),
            };

            let kbps = crate::bitrate_kbps(size, fps, venc);
            let keyint = (fps.fps() * 2.0).round().max(1.0) as u32;
            let common = |t: &IMFMediaType, subtype: &GUID| -> windows::core::Result<()> {
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, pack(size[0], size[1]))?;
                t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps.num, fps.den))?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            };
            // what `Yuv420` is: BT.709, limited range
            let colours = |t: &IMFMediaType| -> windows::core::Result<()> {
                t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
                t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            };
            // an encoder's output type is set first: its input types depend on it
            let out = MFCreateMediaType().map_err(|e| failed("no media type", e))?;
            common(&out, &subtype)
                .and_then(|()| out.SetUINT32(&MF_MT_AVG_BITRATE, kbps.saturating_mul(1000)))
                .and_then(|()| match venc.codec {
                    VideoCodec::H264 => out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32),
                    _ => out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH265VProfile_Main_420_8.0 as u32),
                })
                .map_err(|e| failed("output type not made", e))?;
            let _ = colours(&out);
            transform.SetOutputType(ids.1, &out, 0).map_err(|e| failed("output type refused", e))?;

            let tagged = MFCreateMediaType().map_err(|e| failed("no media type", e))?;
            common(&tagged, &MFVideoFormat_NV12)
                .and_then(|()| colours(&tagged))
                .map_err(|e| failed("input type not made", e))?;
            if transform.SetInputType(ids.0, &tagged, 0).is_err() {
                // some transforms take no colour description; the pictures are the same
                let plain = MFCreateMediaType().map_err(|e| failed("no media type", e))?;
                common(&plain, &MFVideoFormat_NV12).map_err(|e| failed("input type not made", e))?;
                transform.SetInputType(ids.0, &plain, 0).map_err(|e| failed("input type refused", e))?;
            }

            // an export: the bitrate on average, a keyframe every two seconds, and no B-frames, so
            // decode order is display order. A transform that lacks one of these keeps its default;
            // frames that then come out of order stop the export (see `keep`).
            if let Ok(api) = transform.cast::<ICodecAPI>() {
                let mode = eAVEncCommonRateControlMode_UnconstrainedVBR.0 as u32;
                let _ = api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &number(mode));
                let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &number(kbps.saturating_mul(1000)));
                let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &number(keyint));
                let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &number(0));
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &flag(true));
            }

            let events = if asynchronous {
                Some(transform.cast::<IMFMediaEventGenerator>().map_err(|e| failed("encoder has no events", e))?)
            } else {
                None
            };
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .and_then(|()| transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0))
                .map_err(|e| failed("encoder not started", e))?;
            Ok(Encoder {
                transform,
                events,
                wanted: 0,
                drained: false,
                ids,
                size,
                fps,
                codec: venc.codec,
                label,
                sets: ParameterSets::default(),
                out: Vec::new(),
                last_pts: None,
                sent: 0,
                got: 0,
            })
        }
    }

    /// Frame `index`'s time in Media Foundation's 100 ns units.
    fn ticks(&self, index: u64) -> i64 {
        (index as i128 * 10_000_000 * i128::from(self.fps.den) / i128::from(self.fps.num)) as i64
    }

    /// The frame as an NV12 sample.
    unsafe fn sample(&self, yuv: &Yuv420, index: u64) -> Result<IMFSample, ExportError> {
        let (w, h) = (self.size[0] as usize, self.size[1] as usize);
        let (cw, ch) = (w / 2, h / 2);
        if yuv.width != w || yuv.y.len() < w * h || yuv.u.len() < cw * ch || yuv.v.len() < cw * ch {
            return Err(ExportError::Encode("a frame of another size than the encoder was opened for".into()));
        }
        let bytes = w * h + 2 * cw * ch;
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes as u32).map_err(|e| failed("no buffer", e))?;
            let mut at: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut at, None, None).map_err(|e| failed("buffer not locked", e))?;
            // SAFETY: the buffer was made `bytes` long and is locked
            let picture = std::slice::from_raw_parts_mut(at, bytes);
            let (luma, chroma) = picture.split_at_mut(w * h);
            luma.copy_from_slice(&yuv.y[..w * h]);
            for (px, (u, v)) in chroma.as_chunks_mut::<2>().0.iter_mut().zip(yuv.u.iter().zip(&yuv.v)) {
                *px = [*u, *v];
            }
            let _ = buffer.Unlock();
            buffer.SetCurrentLength(bytes as u32).map_err(|e| failed("buffer not filled", e))?;
            let sample = MFCreateSample().map_err(|e| failed("no sample", e))?;
            sample
                .AddBuffer(&buffer)
                .and_then(|()| sample.SetSampleTime(self.ticks(index)))
                .and_then(|()| sample.SetSampleDuration(self.ticks(index + 1) - self.ticks(index)))
                .map_err(|e| failed("sample not made", e))?;
            Ok(sample)
        }
    }

    /// Asks the transform for one encoded frame; `false` when it has none (it wants more input).
    unsafe fn pull(&mut self) -> Result<bool, ExportError> {
        unsafe {
            let info = self.transform.GetOutputStreamInfo(self.ids.1).map_err(|e| failed("no output stream", e))?;
            let provides = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32
                != 0;
            let ours = if provides {
                None
            } else {
                let room = info.cbSize.max(self.size[0] * self.size[1] * 2);
                let buffer = MFCreateMemoryBuffer(room).map_err(|e| failed("no buffer", e))?;
                let sample = MFCreateSample().map_err(|e| failed("no sample", e))?;
                sample.AddBuffer(&buffer).map_err(|e| failed("sample not made", e))?;
                Some(sample)
            };
            let mut slot = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: self.ids.1,
                pSample: ManuallyDrop::new(ours),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0u32;
            let result = self.transform.ProcessOutput(0, &mut slot, &mut status);
            // what is in the slot is ours to release, whatever happened
            let sample = ManuallyDrop::take(&mut slot[0].pSample);
            drop(ManuallyDrop::take(&mut slot[0].pEvents));
            match result {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // the transform settled on another output type: take it and ask again
                    let t = self
                        .transform
                        .GetOutputAvailableType(self.ids.1, 0)
                        .map_err(|e| failed("no output type", e))?;
                    self.transform.SetOutputType(self.ids.1, &t, 0).map_err(|e| failed("output type refused", e))?;
                    return Ok(true);
                }
                Err(e) => return Err(failed("frame not encoded", e)),
            }
            let Some(sample) = sample else { return Ok(true) };
            self.keep(&sample)?;
            Ok(true)
        }
    }

    /// Takes an encoded sample apart into an MP4 sample and the parameter sets.
    unsafe fn keep(&mut self, sample: &IMFSample) -> Result<(), ExportError> {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| failed("frame not read", e))?;
            let (mut at, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
            buffer.Lock(&mut at, None, Some(&mut len)).map_err(|e| failed("frame not read", e))?;
            // SAFETY: locked, and `len` bytes long
            let data = self.sets.sample(self.codec, std::slice::from_raw_parts(at, len as usize));
            let _ = buffer.Unlock();
            let time = sample.GetSampleTime().map_err(|e| failed("frame without a time", e))?;
            // back to the frame it was: times went in as whole frames
            let index = (time as i128 * i128::from(self.fps.num) + 5_000_000 * i128::from(self.fps.den))
                / (10_000_000 * i128::from(self.fps.den));
            let pts = index as i64 * i64::from(self.fps.den);
            self.got += 1;
            if data.is_empty() {
                return Ok(());
            }
            if self.last_pts.is_some_and(|last| pts <= last) {
                return Err(ExportError::Encode(format!(
                    "{}: frames came back out of order (B-frames it would not turn off); set REELFORGE_ENCODER=software",
                    self.label
                )));
            }
            self.last_pts = Some(pts);
            let keyframe = is_keyframe(self.codec, &data);
            self.out.push(EncodedFrame { data, pts, dts: pts, keyframe });
            Ok(())
        }
    }

    /// Handles the events of an asynchronous transform: all that are waiting, or, when `wait`,
    /// one more than that. `false` when there was none.
    unsafe fn listen(&mut self, wait: bool) -> Result<bool, ExportError> {
        let Some(events) = self.events.clone() else { return Ok(false) };
        let how = if wait { MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0) } else { MF_EVENT_FLAG_NO_WAIT };
        let event = match unsafe { events.GetEvent(how) } {
            Ok(e) => e,
            Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(false),
            Err(e) => return Err(failed("encoder stopped", e)),
        };
        let kind = unsafe { event.GetType() }.map_err(|e| failed("encoder stopped", e))?;
        if kind == METransformNeedInput.0 as u32 {
            self.wanted += 1;
        } else if kind == METransformHaveOutput.0 as u32 {
            unsafe { self.pull()? };
        } else if kind == METransformDrainComplete.0 as u32 {
            self.drained = true;
        }
        Ok(true)
    }
}

impl VideoEncoder for Encoder {
    fn encode(&mut self, yuv: &Yuv420, index: u64) -> Result<(), ExportError> {
        unsafe {
            let sample = self.sample(yuv, index)?;
            if self.events.is_some() {
                while self.wanted == 0 {
                    self.listen(true)?;
                }
                self.wanted -= 1;
                self.transform.ProcessInput(self.ids.0, &sample, 0).map_err(|e| failed("frame refused", e))?;
                while self.listen(false)? {}
            } else {
                loop {
                    match self.transform.ProcessInput(self.ids.0, &sample, 0) {
                        Ok(()) => break,
                        // full: take what it has, then it takes the frame
                        Err(e) if e.code() == MF_E_NOTACCEPTING => {
                            if !self.pull()? {
                                return Err(failed("frame refused", e));
                            }
                        }
                        Err(e) => return Err(failed("frame refused", e)),
                    }
                }
                while self.pull()? {}
            }
        }
        self.sent += 1;
        Ok(())
    }

    fn take(&mut self) -> Result<Vec<EncodedFrame>, ExportError> {
        unsafe { while self.listen(false)? {} }
        Ok(std::mem::take(&mut self.out))
    }

    fn config(&self) -> Option<VideoConfig> {
        match self.codec {
            VideoCodec::Hevc => self.sets.hvcc().map(VideoConfig::Hevc),
            _ => self.sets.avcc().map(VideoConfig::Avc),
        }
    }

    fn finish(&mut self) -> Result<(), ExportError> {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                .and_then(|()| self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0))
                .map_err(|e| failed("frames not finished", e))?;
            if self.events.is_some() {
                while !self.drained {
                    self.listen(true)?;
                }
            } else {
                while self.pull()? {}
            }
        }
        Ok(())
    }

    fn backlog(&self) -> u32 {
        self.sent.saturating_sub(self.got) as u32
    }

    fn name(&self) -> String {
        let codec = if self.codec == VideoCodec::Hevc { "hevc" } else { "h264" };
        format!("{codec} ({})", self.label)
    }
}
