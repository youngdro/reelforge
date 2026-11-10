//! VideoToolbox (macOS): H.264 and HEVC, in hardware where the Mac has it.
//!
//! Frames go in as NV12 pixel buffers; VideoToolbox calls back, on a thread of its own, with
//! sample buffers that already hold length-prefixed NAL units, and whose format description
//! carries the `avcC` / `hvcC` record the MP4 needs.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    CMSampleBuffer, CMTime, CMTimeFlags, kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms,
    kCMVideoCodecType_H264, kCMVideoCodecType_HEVC,
};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_video_toolbox::{
    VTCompressionSession, VTEncodeInfoFlags, VTSessionSetProperty, kVTCompressionPropertyKey_AllowFrameReordering,
    kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ColorPrimaries,
    kVTCompressionPropertyKey_ExpectedFrameRate, kVTCompressionPropertyKey_MaxKeyFrameInterval,
    kVTCompressionPropertyKey_ProfileLevel, kVTCompressionPropertyKey_RealTime,
    kVTCompressionPropertyKey_TransferFunction, kVTCompressionPropertyKey_YCbCrMatrix,
    kVTProfileLevel_H264_High_AutoLevel, kVTProfileLevel_HEVC_Main_AutoLevel,
    kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder,
};
use reelforge_media::{EncodedFrame, ExportError, VideoConfig, VideoEncoder, Yuv420};
use reelforge_renderspec::{VideoCodec, VideoEncoding};
use reelforge_time::FrameRate;

/// What the callback fills, on VideoToolbox's thread.
#[derive(Default)]
struct Shared {
    out: Vec<EncodedFrame>,
    config: Option<VideoConfig>,
    error: Option<String>,
}

/// A VideoToolbox compression session.
pub(crate) struct Encoder {
    session: CFRetained<VTCompressionSession>,
    /// The callback's reference: it holds a pointer to this, so it outlives the session.
    shared: Arc<(VideoCodec, FrameRate, Mutex<Shared>)>,
    size: [u32; 2],
    fps: FrameRate,
    codec: VideoCodec,
    /// Frames handed over, less frames back.
    sent: u64,
    got: u64,
}

fn refused(what: &str, status: i32) -> ExportError {
    ExportError::Encode(format!("VideoToolbox: {what} ({status})"))
}

impl Encoder {
    pub(crate) fn open(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Result<Encoder, ExportError> {
        let kind = match venc.codec {
            VideoCodec::H264 => kCMVideoCodecType_H264,
            VideoCodec::Hevc => kCMVideoCodecType_HEVC,
            VideoCodec::Av1 => return Err(ExportError::Encode("VideoToolbox has no AV1 encoder".into())),
        };
        let shared = Arc::new((venc.codec, fps, Mutex::new(Shared::default())));
        // hardware where there is some; VideoToolbox's own software encoder otherwise
        let spec = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(
                &[kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder],
                &[CFBoolean::new(true).as_ref()],
            )
        };
        let mut session: *mut VTCompressionSession = std::ptr::null_mut();
        let status = unsafe {
            VTCompressionSession::create(
                None,
                size[0] as i32,
                size[1] as i32,
                kind,
                Some(spec.as_opaque()),
                None,
                None,
                Some(output),
                Arc::as_ptr(&shared) as *mut c_void,
                NonNull::from(&mut session),
            )
        };
        let Some(session) = NonNull::new(session).filter(|_| status == 0) else {
            return Err(refused("no encoder", status));
        };
        // SAFETY: created above, and the create rule hands over one reference
        let session = unsafe { CFRetained::from_raw(session) };
        let enc = Encoder { session, shared, size, fps, codec: venc.codec, sent: 0, got: 0 };

        let kbps = crate::bitrate_kbps(size, fps, venc);
        let keyint = (fps.fps() * 2.0).round().max(1.0) as i32;
        let profile = unsafe {
            match venc.codec {
                VideoCodec::Hevc => kVTProfileLevel_HEVC_Main_AutoLevel,
                _ => kVTProfileLevel_H264_High_AutoLevel,
            }
        };
        // an export, not a call: quality over latency, and no B-frames so decode order is display
        // order (the MP4 then needs no composition offsets)
        unsafe {
            enc.set(kVTCompressionPropertyKey_RealTime, CFBoolean::new(false).as_ref())?;
            enc.set(kVTCompressionPropertyKey_AllowFrameReordering, CFBoolean::new(false).as_ref())?;
            enc.set(kVTCompressionPropertyKey_ProfileLevel, profile.as_ref())?;
            enc.set(kVTCompressionPropertyKey_AverageBitRate, CFNumber::new_i32(kbps as i32 * 1000).as_ref())?;
            enc.set(kVTCompressionPropertyKey_MaxKeyFrameInterval, CFNumber::new_i32(keyint).as_ref())?;
            enc.set(kVTCompressionPropertyKey_ExpectedFrameRate, CFNumber::new_f64(fps.fps()).as_ref())?;
            // what `Yuv420` is: BT.709, limited range
            let bt709 = CFString::from_static_str("ITU_R_709_2");
            enc.set(kVTCompressionPropertyKey_ColorPrimaries, bt709.as_ref())?;
            enc.set(kVTCompressionPropertyKey_TransferFunction, bt709.as_ref())?;
            enc.set(kVTCompressionPropertyKey_YCbCrMatrix, bt709.as_ref())?;
        }
        Ok(enc)
    }

    /// Sets a session property; one the encoder does not have is an error, so the caller falls
    /// back to software rather than export with settings that were not taken.
    unsafe fn set(&self, key: &CFString, value: &CFType) -> Result<(), ExportError> {
        let session: &CFType = self.session.as_ref();
        match unsafe { VTSessionSetProperty(session, key, Some(value)) } {
            0 => Ok(()),
            status => Err(refused(&format!("property {key} refused"), status)),
        }
    }

    /// The frame as an NV12 pixel buffer.
    fn picture(&self, yuv: &Yuv420) -> Result<CFRetained<CVPixelBuffer>, ExportError> {
        let (w, h) = (self.size[0] as usize, self.size[1] as usize);
        let (cw, ch) = (yuv.chroma_width(), h.div_ceil(2));
        if yuv.width != w || yuv.y.len() < w * h || yuv.u.len() < cw * ch || yuv.v.len() < cw * ch {
            return Err(ExportError::Encode("a frame of another size than the encoder was opened for".into()));
        }
        let mut buffer: *mut CVPixelBuffer = std::ptr::null_mut();
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                w,
                h,
                kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                None,
                NonNull::from(&mut buffer),
            )
        };
        let Some(buffer) = NonNull::new(buffer).filter(|_| status == 0) else {
            return Err(refused("no pixel buffer", status));
        };
        // SAFETY: created above; the create rule hands over one reference
        let buffer = unsafe { CFRetained::from_raw(buffer) };
        let flags = CVPixelBufferLockFlags(0);
        let status = unsafe { CVPixelBufferLockBaseAddress(&buffer, flags) };
        if status != 0 {
            return Err(refused("pixel buffer not locked", status));
        }
        // SAFETY: the buffer is locked; each plane is `bytes per row` × its height, and a row
        // holds at least the picture's width (NV12: the chroma plane's rows are `2 × cw` wide)
        unsafe {
            let (luma, stride) = (
                CVPixelBufferGetBaseAddressOfPlane(&buffer, 0) as *mut u8,
                CVPixelBufferGetBytesPerRowOfPlane(&buffer, 0),
            );
            for (row, line) in yuv.y.chunks_exact(w).take(h).enumerate() {
                std::ptr::copy_nonoverlapping(line.as_ptr(), luma.add(row * stride), w);
            }
            let (chroma, stride) = (
                CVPixelBufferGetBaseAddressOfPlane(&buffer, 1) as *mut u8,
                CVPixelBufferGetBytesPerRowOfPlane(&buffer, 1),
            );
            // an odd width has `cw × 2 = w + 1` chroma bytes; the plane's row is that wide too
            let pairs = cw.min(stride / 2);
            for (row, (us, vs)) in yuv.u.chunks_exact(cw).zip(yuv.v.chunks_exact(cw)).take(ch).enumerate() {
                let line = std::slice::from_raw_parts_mut(chroma.add(row * stride), pairs * 2);
                for (px, (u, v)) in line.as_chunks_mut::<2>().0.iter_mut().zip(us.iter().zip(vs)) {
                    *px = [*u, *v];
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, flags);
        }
        Ok(buffer)
    }

    fn failed(&self) -> Result<(), ExportError> {
        match &self.shared.2.lock().unwrap_or_else(|e| e.into_inner()).error {
            Some(e) => Err(ExportError::Encode(e.clone())),
            None => Ok(()),
        }
    }
}

fn time(value: i64, scale: u32) -> CMTime {
    CMTime { value, timescale: scale as i32, flags: CMTimeFlags(1), epoch: 0 }
}

/// VideoToolbox's callback: one encoded frame (or a failure), on its own thread.
unsafe extern "C-unwind" fn output(
    refcon: *mut c_void,
    _frame: *mut c_void,
    status: i32,
    _flags: VTEncodeInfoFlags,
    sample: *mut CMSampleBuffer,
) {
    // SAFETY: `refcon` is the `Arc` the encoder holds until after the session is invalidated
    let (codec, fps, shared) = unsafe { &*(refcon as *const (VideoCodec, FrameRate, Mutex<Shared>)) };
    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
    if status != 0 {
        shared.error.get_or_insert(format!("VideoToolbox: frame not encoded ({status})"));
        return;
    }
    // SAFETY: a sample buffer that is ours for the length of the call, or null for a dropped frame
    let Some(sample) = (unsafe { sample.as_ref() }) else {
        shared.error.get_or_insert("VideoToolbox dropped a frame".into());
        return;
    };
    let got = unsafe { read(sample, *codec, *fps, shared.config.is_none()) };
    match got {
        Ok((frame, config)) => {
            if config.is_some() {
                shared.config = config;
            }
            shared.out.push(frame);
        }
        Err(e) => {
            shared.error.get_or_insert(e);
        }
    }
}

/// The frame in a sample buffer and, when `want_config`, the configuration record with it.
unsafe fn read(
    sample: &CMSampleBuffer,
    codec: VideoCodec,
    fps: FrameRate,
    want_config: bool,
) -> Result<(EncodedFrame, Option<VideoConfig>), String> {
    let block = unsafe { sample.data_buffer() }.ok_or("VideoToolbox: a frame without data")?;
    let mut data = vec![0u8; unsafe { block.data_length() }];
    if data.is_empty() {
        return Err("VideoToolbox: an empty frame".into());
    }
    // SAFETY: `data` is as long as the block buffer says it is
    let status = unsafe { block.copy_data_bytes(0, data.len(), NonNull::new_unchecked(data.as_mut_ptr().cast())) };
    if status != 0 {
        return Err(format!("VideoToolbox: frame data not read ({status})"));
    }
    // times come back in the timescale they went in with (the frame rate's numerator), but that
    // is not promised
    let shown = unsafe { sample.presentation_time_stamp() };
    let pts = if shown.timescale == fps.num as i32 || shown.timescale <= 0 {
        shown.value
    } else {
        (shown.value as i128 * i128::from(fps.num) / i128::from(shown.timescale)) as i64
    };
    let config = if want_config {
        let name = match codec {
            VideoCodec::Hevc => "hvcC",
            _ => "avcC",
        };
        let format = unsafe { sample.format_description() }.ok_or("VideoToolbox: a frame without a format")?;
        let atoms = unsafe { format.extension(kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms) }
            .ok_or("VideoToolbox: a format without its configuration record")?;
        let atoms: &CFType = atoms.as_ref();
        let atoms =
            atoms.downcast_ref::<CFDictionary>().ok_or("VideoToolbox: configuration atoms are not a dictionary")?;
        // SAFETY: the atoms dictionary maps names (strings) to property-list values
        let atoms = unsafe { atoms.cast_unchecked::<CFString, CFType>() };
        let record = atoms.get(&CFString::from_static_str(name)).ok_or(format!("VideoToolbox: no {name} record"))?;
        let record =
            record.downcast_ref::<CFData>().ok_or(format!("VideoToolbox: the {name} record is not data"))?.to_vec();
        Some(match codec {
            VideoCodec::Hevc => VideoConfig::Hevc(record),
            _ => VideoConfig::Avc(record),
        })
    } else {
        None
    };
    let keyframe = crate::nal::is_keyframe(codec, &data);
    Ok((EncodedFrame { data, pts, dts: pts, keyframe }, config))
}

impl VideoEncoder for Encoder {
    fn encode(&mut self, yuv: &Yuv420, index: u64) -> Result<(), ExportError> {
        self.failed()?;
        let picture = self.picture(yuv)?;
        let at = time(index as i64 * i64::from(self.fps.den), self.fps.num);
        let length = time(i64::from(self.fps.den), self.fps.num);
        let status = unsafe {
            self.session.encode_frame(&picture, at, length, None, std::ptr::null_mut(), std::ptr::null_mut())
        };
        if status != 0 {
            return Err(refused("frame refused", status));
        }
        self.sent += 1;
        Ok(())
    }

    fn take(&mut self) -> Result<Vec<EncodedFrame>, ExportError> {
        self.failed()?;
        let out = std::mem::take(&mut self.shared.2.lock().unwrap_or_else(|e| e.into_inner()).out);
        self.got += out.len() as u64;
        Ok(out)
    }

    fn config(&self) -> Option<VideoConfig> {
        self.shared.2.lock().unwrap_or_else(|e| e.into_inner()).config.clone()
    }

    fn finish(&mut self) -> Result<(), ExportError> {
        // an invalid time: every frame; returns when they have all been called back
        let all = CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 };
        match unsafe { self.session.complete_frames(all) } {
            0 => self.failed(),
            status => Err(refused("frames not finished", status)),
        }
    }

    fn backlog(&self) -> u32 {
        let waiting = self.shared.2.lock().unwrap_or_else(|e| e.into_inner()).out.len() as u64;
        self.sent.saturating_sub(self.got + waiting) as u32
    }

    fn name(&self) -> String {
        match self.codec {
            VideoCodec::Hevc => "hevc (VideoToolbox)".into(),
            _ => "h264 (VideoToolbox)".into(),
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // after this returns the callback is not called again, so `shared` may go
        unsafe { self.session.invalidate() };
    }
}
