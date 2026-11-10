//! A RenderSpec to an MP4 file: every frame drawn at full size (on the GPU where there is one,
//! else by the CPU reference compositor), encoded as H.264 High, the sound mixed and encoded as AAC-LC, muxed with the index in front
//! ("fast start") so a browser can play it while it downloads.

use std::io::{Read, Seek, Write};

use filmcraft_isobmff::{
    Av1Config, AvcConfig, Brand, HevcConfig, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use reelforge_render::{Quality, RenderError, render_frame, unsupported_features};
use reelforge_renderspec::{AudioCodec, AudioEncoding, OutputFormat, RenderSpec, VideoCodec, VideoEncoding, validate};
use reelforge_time::{FrameRate, TICKS_PER_SECOND, Tick};
use thiserror::Error;

use crate::Footage;
pub use crate::yuv::Yuv420;
use crate::yuv::to_yuv420;

/// Why an export stopped.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The spec is not valid, or asks for something this exporter does not write.
    #[error("{0}")]
    Invalid(String),
    /// A frame could not be drawn.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Footage could not be read.
    #[error("media: {0}")]
    Media(String),
    /// The encoder refused.
    #[error("encode: {0}")]
    Encode(String),
    /// Writing the file failed.
    #[error("write: {0}")]
    Write(String),
    /// The GPU failed mid-export.
    #[error("gpu: {0}")]
    Gpu(String),
    /// The progress callback asked to stop.
    #[error("cancelled")]
    Cancelled,
}

/// Which compositor draws an export's frames.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// The GPU when there is one, else the CPU.
    #[default]
    Auto,
    /// The CPU reference compositor.
    Cpu,
    /// The wgpu compositor; the export fails without a GPU.
    Gpu,
}

/// How far an export has come.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Frames written.
    pub done: u64,
    /// Frames in all.
    pub total: u64,
}

/// What an export wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportStats {
    /// Video frames.
    pub frames: u64,
    /// Audio sample frames (per channel), or 0 without a sound track.
    pub audio_samples: u64,
    /// What drew the frames: `cpu`, or the GPU adapter.
    pub renderer: String,
    /// What encoded the video ([`VideoEncoder::name`]), such as `h264 (software)`.
    pub encoder: String,
}

pub(crate) const CHANNELS: usize = 2;

pub(crate) fn mux_err(e: impl std::fmt::Display) -> ExportError {
    ExportError::Write(e.to_string())
}

/// Renders `spec` and writes the file its `output.format` asks for to `out`: an MP4 unless it
/// says otherwise, or an animated GIF, or the sound alone ([`crate::formats`]). `progress` is
/// called after every frame (every second of a sound file); returning `false` stops the export
/// with [`ExportError::Cancelled`].
///
/// Video is H.264 at the spec's quality (`crf`, default 20) or bitrate, at the spec's encoded
/// size. Sound is written when the spec has audio clips: AAC-LC, stereo, at the spec's rate and
/// bitrate (default 48 kHz, 192 kbit/s).
pub fn export<W: Read + Write + Seek>(
    spec: &RenderSpec,
    media: &Footage,
    out: W,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(W, ExportStats), ExportError> {
    export_with(spec, media, out, Renderer::Auto, progress)
}

/// [`export`], choosing the compositor.
pub fn export_with<W: Read + Write + Seek>(
    spec: &RenderSpec,
    media: &Footage,
    out: W,
    renderer: Renderer,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(W, ExportStats), ExportError> {
    check(spec)?;
    match spec.output.format {
        OutputFormat::Mp4 => {}
        OutputFormat::Gif => return crate::formats::export_gif(spec, media, out, progress),
        OutputFormat::M4a | OutputFormat::Wav => return crate::formats::export_audio(spec, media, out, progress),
    }
    let venc = video_encoding(spec);
    let encoder = open_encoder(spec)?;
    let mut drawer = Drawer::new(renderer)?;
    let mut writer = ExportWriter::new(spec, out, encoder)?;
    let total = writer.frames();
    let scale = venc.scale(spec.output.width);
    each_frame(spec, media, &mut drawer, scale, total, &mut |i, yuv| {
        writer.push(spec, media, i, yuv)?;
        if progress(Progress { done: i + 1, total }) { Ok(()) } else { Err(ExportError::Cancelled) }
    })?;
    writer.end()?;
    let (out, mut stats) = writer.finish(spec, media)?;
    stats.renderer = drawer.name();
    Ok((out, stats))
}

/// Checks a spec can be exported: valid, and drawn by this renderer.
pub(crate) fn check(spec: &RenderSpec) -> Result<(), ExportError> {
    validate(spec).map_err(|e| ExportError::Invalid(e.to_string()))?;
    let missing = unsupported_features(spec);
    if !missing.is_empty() {
        return Err(RenderError::Unsupported(missing).into());
    }
    Ok(())
}

/// The spec's video encoding, or the default one.
pub fn video_encoding(spec: &RenderSpec) -> VideoEncoding {
    spec.output.video.clone().unwrap_or(VideoEncoding {
        codec: VideoCodec::H264,
        crf: Some(20),
        bitrate_kbps: None,
        size: None,
    })
}

/// The size frames are encoded at.
pub fn encoded_size(spec: &RenderSpec) -> [u32; 2] {
    spec.output.video.as_ref().and_then(|v| v.size).unwrap_or([spec.output.width, spec.output.height])
}

/// Draws frame `i` of `spec` on the CPU, at its encoded size, into `yuv`.
pub fn draw_frame(spec: &RenderSpec, media: &Footage, i: u64, yuv: &mut Yuv420) -> Result<(), ExportError> {
    let scale = video_encoding(spec).scale(spec.output.width);
    let (img, _) = render_frame(spec, spec.output.fps.tick_of(i as i64), scale, media, Quality::Final)?;
    to_yuv420(&img, yuv);
    Ok(())
}

/// A video encoder's configuration record, as the MP4 sample entry carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoConfig {
    /// `avcC`.
    Avc(Vec<u8>),
    /// `hvcC`.
    Hevc(Vec<u8>),
    /// `av1C`.
    Av1(Vec<u8>),
}

/// One encoded picture. Times are in the frame rate's timescale (`fps.num` per second, one frame
/// is `fps.den`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedFrame {
    /// The picture's data: length-prefixed NAL units for H.264 and HEVC, OBUs for AV1.
    pub data: Vec<u8>,
    /// When it is shown.
    pub pts: i64,
    /// When it is decoded.
    pub dts: i64,
    /// Whether decoding can start here.
    pub keyframe: bool,
}

/// A video encoder an export feeds. An encoder may work asynchronously (the browser's): frames go
/// in with [`encode`](Self::encode), and whatever is ready comes out with [`take`](Self::take).
pub trait VideoEncoder {
    /// Hands over frame `index`.
    fn encode(&mut self, frame: &Yuv420, index: u64) -> Result<(), ExportError>;
    /// The frames encoded since the last call, in decode order.
    fn take(&mut self) -> Result<Vec<EncodedFrame>, ExportError>;
    /// The configuration record, once known (at the latest with the first frame out).
    fn config(&self) -> Option<VideoConfig>;
    /// Asks for the frames still inside; [`take`](Self::take) returns them once ready.
    fn finish(&mut self) -> Result<(), ExportError>;
    /// The first decode time when frames are reordered (B-frames): one frame's `-dts`.
    fn decode_delay(&self) -> i64 {
        0
    }
    /// Frames handed over but not yet back (an asynchronous encoder's queue, or a flush under
    /// way); 0 for an encoder that answers at once.
    fn backlog(&self) -> u32 {
        0
    }
    /// What encodes, for reports.
    fn name(&self) -> String;
}

/// Makes an encoder for frames of this size and rate with these settings, or `None` when it cannot
/// (no such codec, no device, the system refuses), and then the next one is asked.
pub type EncoderFactory = fn(size: [u32; 2], fps: FrameRate, venc: &VideoEncoding) -> Option<Box<dyn VideoEncoder>>;

static ENCODERS: std::sync::Mutex<Vec<EncoderFactory>> = std::sync::Mutex::new(Vec::new());

/// Puts an encoder in front of FilmCraft's: [`open_encoder`] (and so [`export`]) asks the
/// registered ones first, the last registered first. The operating system's hardware encoders
/// come in this way (`reelforge-hwenc`), so this crate stays pure Rust.
pub fn register_video_encoder(make: EncoderFactory) {
    let mut all = ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    if !all.iter().any(|f| std::ptr::fn_addr_eq(*f, make)) {
        all.push(make);
    }
}

/// The encoder `spec` exports with: a registered one that takes it, else FilmCraft's H.264. HEVC
/// and AV1 need a registered (hardware) encoder.
pub fn open_encoder(spec: &RenderSpec) -> Result<Box<dyn VideoEncoder>, ExportError> {
    let venc = video_encoding(spec);
    let size = encoded_size(spec);
    let fps = spec.output.fps;
    let all = ENCODERS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(enc) = all.iter().rev().find_map(|make| make(size, fps, &venc)) {
        return Ok(enc);
    }
    if venc.codec != VideoCodec::H264 {
        return Err(ExportError::Invalid(format!(
            "no encoder for {:?} here: it needs the system's hardware encoder; H.264 always works",
            venc.codec
        )));
    }
    Ok(Box::new(SoftwareH264::new(size[0], size[1], fps, &venc)?))
}

/// FilmCraft's H.264 encoder: pure Rust, on the CPU.
pub struct SoftwareH264 {
    enc: filmcraft_h264enc::Encoder,
    den: u32,
    out: Vec<EncodedFrame>,
}

impl SoftwareH264 {
    /// An encoder for `width` by `height` frames at `fps`, at `venc`'s quality.
    pub fn new(width: u32, height: u32, fps: FrameRate, venc: &VideoEncoding) -> Result<SoftwareH264, ExportError> {
        let mut cfg = filmcraft_h264enc::EncoderConfig::new(width, height, fps.num, fps.den);
        cfg.format = filmcraft_h264enc::PacketFormat::LengthPrefixed;
        cfg.rate = match (venc.crf, venc.bitrate_kbps) {
            (Some(crf), _) => filmcraft_h264enc::RateControl::Crf(f32::from(crf)),
            (None, Some(kbps)) => filmcraft_h264enc::RateControl::Vbr { target_kbps: kbps, max_kbps: kbps * 2 },
            (None, None) => filmcraft_h264enc::RateControl::Crf(20.0),
        };
        cfg.keyint = (fps.fps() * 2.0).round().max(1.0) as u32;
        let enc = filmcraft_h264enc::Encoder::new(cfg).map_err(|e| ExportError::Encode(e.to_string()))?;
        Ok(SoftwareH264 { enc, den: fps.den, out: Vec::new() })
    }

    fn keep(&mut self, ps: Vec<filmcraft_h264enc::Packet>) {
        self.out.extend(ps.into_iter().map(|p| EncodedFrame {
            data: p.data,
            pts: p.pts,
            dts: p.dts,
            keyframe: p.keyframe,
        }));
    }
}

impl VideoEncoder for SoftwareH264 {
    fn encode(&mut self, yuv: &Yuv420, index: u64) -> Result<(), ExportError> {
        let frame = filmcraft_h264enc::YuvFrame {
            y: &yuv.y,
            u: &yuv.u,
            v: &yuv.v,
            y_stride: yuv.width,
            uv_stride: yuv.chroma_width(),
        };
        let ps = self
            .enc
            .try_encode(&frame, index as i64 * i64::from(self.den))
            .map_err(|e| ExportError::Encode(e.to_string()))?;
        self.keep(ps);
        Ok(())
    }

    fn take(&mut self) -> Result<Vec<EncodedFrame>, ExportError> {
        Ok(std::mem::take(&mut self.out))
    }

    fn config(&self) -> Option<VideoConfig> {
        Some(VideoConfig::Avc(self.enc.avcc()))
    }

    fn finish(&mut self) -> Result<(), ExportError> {
        let ps = self.enc.flush();
        self.keep(ps);
        Ok(())
    }

    fn decode_delay(&self) -> i64 {
        if self.enc.delay() > 0 { i64::from(self.den) } else { 0 }
    }

    fn name(&self) -> String {
        "h264 (software)".into()
    }
}

/// An export being written: frames go in one at a time ([`push`](Self::push)), so a caller can
/// draw them however it likes and yield between them (the browser does).
pub struct ExportWriter<W: Read + Write + Seek> {
    mux: Mp4Writer<W>,
    enc: Box<dyn VideoEncoder>,
    vtrack: usize,
    /// The sample entry has the encoder's real configuration.
    configured: bool,
    /// Frames out of the encoder before its configuration was known.
    held: Vec<EncodedFrame>,
    sound: Option<Sound>,
    size: [u32; 2],
    fps: FrameRate,
    total: u64,
    /// Frames handed to the encoder.
    pushed: u64,
}

impl<W: Read + Write + Seek> ExportWriter<W> {
    /// Starts writing `spec` to `out`, encoding with `enc`.
    pub fn new(spec: &RenderSpec, out: W, enc: Box<dyn VideoEncoder>) -> Result<ExportWriter<W>, ExportError> {
        check(spec)?;
        let o = &spec.output;
        let fps = o.fps;
        let total = (fps.frame_at(o.duration - Tick(1)) + 1).max(1) as u64;
        let size = encoded_size(spec);
        let mut mux = Mp4Writer::new(out, WriterOptions::new(Brand::Mp4)).map_err(mux_err)?;
        let known = enc.config();
        let entry = match &known {
            Some(c) => sample_entry(c, size)?,
            // replaced once the encoder says
            None => sample_entry(&placeholder(video_encoding(spec).codec), size)?,
        };
        let mut vtrack = TrackConfig::new(entry, fps.num);
        if enc.decode_delay() > 0 {
            // with B-frames the first decode time is one frame before the first presentation time
            vtrack.media_start = Some(enc.decode_delay());
        }
        let vtrack = mux.add_track(vtrack).map_err(mux_err)?;
        let sound = if spec.audio.is_empty() {
            None
        } else {
            let a = o.audio.clone().unwrap_or(AudioEncoding {
                codec: AudioCodec::Aac,
                sample_rate: 48_000,
                bitrate_kbps: 192,
            });
            if a.codec != AudioCodec::Aac {
                return Err(ExportError::Invalid(format!("this exporter writes AAC sound only, not {:?}", a.codec)));
            }
            let aenc = filmcraft_aac::Encoder::new(filmcraft_aac::EncoderConfig::cbr(
                a.sample_rate,
                CHANNELS,
                a.bitrate_kbps * 1000,
            ))
            .map_err(|e| ExportError::Encode(e.to_string()))?;
            let mut track = TrackConfig::new(
                SampleEntry::aac(aenc.audio_specific_config(), CHANNELS as u32, a.sample_rate),
                a.sample_rate,
            );
            track.media_start = Some(i64::from(aenc.priming_samples()));
            let track = mux.add_track(track).map_err(mux_err)?;
            Some(Sound { enc: aenc, track, rate: a.sample_rate, mixed: 0 })
        };
        Ok(ExportWriter {
            mux,
            configured: known.is_some(),
            enc,
            vtrack,
            held: Vec::new(),
            sound,
            size,
            fps,
            total,
            pushed: 0,
        })
    }

    /// Frames in all.
    pub fn frames(&self) -> u64 {
        self.total
    }

    /// The encoded picture size.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Encodes frame `i` (in order, from 0) and writes whatever is ready.
    pub fn push(&mut self, spec: &RenderSpec, media: &Footage, i: u64, yuv: &Yuv420) -> Result<(), ExportError> {
        self.enc.encode(yuv, i)?;
        self.pushed = i + 1;
        self.poll(spec, media)
    }

    /// Writes what the encoder has finished since, with the sound up to the same time.
    pub fn poll(&mut self, spec: &RenderSpec, media: &Footage) -> Result<(), ExportError> {
        let mut frames = std::mem::take(&mut self.held);
        frames.extend(self.enc.take()?);
        if !self.configured {
            match self.enc.config() {
                Some(c) => {
                    self.mux.set_sample_entry(self.vtrack, sample_entry(&c, self.size)?).map_err(mux_err)?;
                    self.configured = true;
                }
                None => {
                    self.held = frames;
                    return Ok(());
                }
            }
        }
        let den = self.fps.den;
        for f in &frames {
            let s = WriteSample {
                data: &f.data,
                duration: den,
                composition_offset: (f.pts - f.dts) as i32,
                is_sync: f.keyframe,
            };
            self.mux.write_sample(self.vtrack, s).map_err(mux_err)?;
        }
        if let Some(s) = &mut self.sound {
            // keep the sound about as far along as the picture so the file interleaves
            let until = self.fps.tick_of(self.pushed as i64).min(spec.output.duration);
            s.mix_until(spec, media, &mut self.mux, until)?;
        }
        Ok(())
    }

    /// Tells the encoder no more frames come.
    pub fn end(&mut self) -> Result<(), ExportError> {
        self.enc.finish()
    }

    /// Writes the rest (call [`end`](Self::end) first, and wait for an asynchronous encoder) and
    /// finishes the file.
    pub fn finish(mut self, spec: &RenderSpec, media: &Footage) -> Result<(W, ExportStats), ExportError> {
        self.poll(spec, media)?;
        if !self.configured {
            return Err(ExportError::Encode("the encoder produced nothing".into()));
        }
        let mut stats =
            ExportStats { frames: self.total, audio_samples: 0, renderer: String::new(), encoder: self.enc.name() };
        if let Some(mut s) = self.sound.take() {
            s.mix_until(spec, media, &mut self.mux, spec.output.duration)?;
            for au in s.enc.flush() {
                self.mux
                    .write_sample(
                        s.track,
                        WriteSample { data: &au, duration: 1024, composition_offset: 0, is_sync: true },
                    )
                    .map_err(mux_err)?;
            }
            stats.audio_samples = s.mixed as u64;
        }
        let out = self.mux.finish_faststart().map_err(mux_err)?;
        Ok((out, stats))
    }

    /// Frames inside the encoder: wait (and [`poll`](Self::poll)) while it is high, and until it
    /// is 0 after [`end`](Self::end).
    pub fn backlog(&self) -> u32 {
        self.enc.backlog()
    }

    /// What encodes.
    pub fn encoder(&self) -> String {
        self.enc.name()
    }
}

/// A configuration to start a track with until the encoder gives its own.
fn placeholder(codec: VideoCodec) -> VideoConfig {
    match codec {
        VideoCodec::H264 => VideoConfig::Avc(AvcConfig::default().to_bytes()),
        VideoCodec::Hevc => VideoConfig::Hevc(HevcConfig::default().to_bytes()),
        VideoCodec::Av1 => VideoConfig::Av1(Av1Config::default().to_bytes()),
    }
}

fn sample_entry(c: &VideoConfig, [w, h]: [u32; 2]) -> Result<SampleEntry, ExportError> {
    let (w, h) = (w as u16, h as u16);
    Ok(match c {
        VideoConfig::Avc(b) => SampleEntry::avc(AvcConfig::parse(b).map_err(mux_err)?, w, h),
        VideoConfig::Hevc(b) => SampleEntry::hevc(HevcConfig::parse(b).map_err(mux_err)?, w, h),
        VideoConfig::Av1(b) => SampleEntry::av1(Av1Config::parse(b).map_err(mux_err)?, w, h),
    })
}

/// The compositor drawing an export, and the conversion to the encoder's input that goes with it.
enum Drawer {
    Cpu,
    #[cfg(not(target_arch = "wasm32"))]
    Gpu(Box<reelforge_render::gpu::GpuCompositor>),
}

impl Drawer {
    fn new(renderer: Renderer) -> Result<Drawer, ExportError> {
        match renderer {
            Renderer::Cpu => Ok(Drawer::Cpu),
            #[cfg(not(target_arch = "wasm32"))]
            Renderer::Auto | Renderer::Gpu => match reelforge_render::gpu::GpuCompositor::headless() {
                Ok(g) => Ok(Drawer::Gpu(Box::new(g))),
                Err(_) if renderer == Renderer::Auto => Ok(Drawer::Cpu),
                Err(e) => Err(ExportError::Gpu(e.to_string())),
            },
            #[cfg(target_arch = "wasm32")]
            Renderer::Auto => Ok(Drawer::Cpu),
            #[cfg(target_arch = "wasm32")]
            Renderer::Gpu => Err(ExportError::Gpu("the browser exports on the CPU".into())),
        }
    }

    fn name(&self) -> String {
        match self {
            Drawer::Cpu => "cpu".into(),
            #[cfg(not(target_arch = "wasm32"))]
            Drawer::Gpu(g) => g.adapter().to_string(),
        }
    }

    fn draw(
        &mut self,
        spec: &RenderSpec,
        media: &Footage,
        i: u64,
        scale: f32,
        yuv: &mut Yuv420,
    ) -> Result<(), ExportError> {
        let t = spec.output.fps.tick_of(i as i64);
        match self {
            Drawer::Cpu => {
                let (img, _) = render_frame(spec, t, scale, media, Quality::Final)?;
                to_yuv420(&img, yuv);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Drawer::Gpu(g) => {
                g.render(spec, t, scale, media, Quality::Final)?;
                g.read_yuv420(&mut yuv.y, &mut yuv.u, &mut yuv.v).map_err(|e| ExportError::Gpu(e.to_string()))?;
                yuv.width =
                    spec.output.video.as_ref().and_then(|v| v.size).map_or(spec.output.width, |[w, _]| w) as usize;
            }
        }
        Ok(())
    }
}

fn blank() -> Yuv420 {
    Yuv420 { y: vec![], u: vec![], v: vec![], width: 0 }
}

/// Draws frames `0..total` in order and hands each to `encode`.
///
/// Natively the next frames are drawn on another thread while `encode` runs, so drawing and
/// encoding overlap; in the browser (no threads) they take turns.
fn each_frame(
    spec: &RenderSpec,
    media: &Footage,
    drawer: &mut Drawer,
    scale: f32,
    total: u64,
    encode: &mut dyn FnMut(u64, &Yuv420) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::mpsc;
        let (to_encoder, frames) = mpsc::sync_channel::<Result<Yuv420, ExportError>>(2);
        let (to_drawer, spare) = mpsc::channel::<Yuv420>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for i in 0..total {
                    let mut yuv = spare.try_recv().unwrap_or_else(|_| blank());
                    let r = drawer.draw(spec, media, i, scale, &mut yuv).map(|()| yuv);
                    let failed = r.is_err();
                    // the encoder has stopped (an error, a cancel): stop drawing too
                    if to_encoder.send(r).is_err() || failed {
                        return;
                    }
                }
            });
            // owned here, so returning early (an error, a cancel) drops it and unblocks the drawer
            let frames = frames;
            for i in 0..total {
                let yuv = frames.recv().map_err(|_| ExportError::Encode("the drawing thread stopped".into()))??;
                encode(i, &yuv)?;
                let _ = to_drawer.send(yuv);
            }
            Ok(())
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut yuv = blank();
        for i in 0..total {
            drawer.draw(spec, media, i, scale, &mut yuv)?;
            encode(i, &yuv)?;
        }
        Ok(())
    }
}

struct Sound {
    enc: filmcraft_aac::Encoder,
    track: usize,
    rate: u32,
    /// Samples mixed and handed to the encoder so far.
    mixed: i64,
}

impl Sound {
    fn sample_at(&self, t: Tick) -> i64 {
        (t.0 as i128 * self.rate as i128 / TICKS_PER_SECOND as i128) as i64
    }

    fn mix_until<W: Write + Seek>(
        &mut self,
        spec: &RenderSpec,
        media: &Footage,
        mux: &mut Mp4Writer<W>,
        until: Tick,
    ) -> Result<(), ExportError> {
        let end = self.sample_at(until);
        if end <= self.mixed {
            return Ok(());
        }
        let planar = mix(spec, media, self.rate, self.mixed, (end - self.mixed) as usize)?;
        self.mixed = end;
        let refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
        for au in self.enc.encode(&refs) {
            mux.write_sample(
                self.track,
                WriteSample { data: &au, duration: 1024, composition_offset: 0, is_sync: true },
            )
            .map_err(mux_err)?;
        }
        Ok(())
    }
}

/// `frames` samples from `from` samples after a clip's start, read like tape wherever `source(i)`
/// puts the clip `i` samples in: backwards, or at a changing speed, its pitch moving with it.
fn tape(
    media: &Footage,
    asset: &reelforge_renderspec::Asset,
    source: impl Fn(i64) -> Tick,
    from: i64,
    frames: usize,
    rate: u32,
) -> Result<Option<Vec<Vec<f32>>>, String> {
    let at: Vec<f64> =
        (0..frames as i64).map(|i| source(from + i).0 as f64 * rate as f64 / TICKS_PER_SECOND as f64).collect();
    let (lo, hi) = at.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let lo = (lo.floor() as i64).max(0);
    let hi = hi.ceil() as i64 + 2;
    if hi <= lo {
        return Ok(Some(vec![vec![0.0; frames]; CHANNELS]));
    }
    let Some(src) = media.audio(asset, lo, (hi - lo) as usize, rate)? else { return Ok(None) };
    if src.is_empty() {
        return Ok(Some(src));
    }
    let out = src
        .iter()
        .map(|ch| {
            at.iter()
                .map(|&p| {
                    if p < 0.0 {
                        return 0.0;
                    }
                    let j = p - lo as f64;
                    let (i, f) = (j.floor() as usize, (j - j.floor()) as f32);
                    let (x0, x1) = (ch.get(i).copied().unwrap_or(0.0), ch.get(i + 1).copied().unwrap_or(0.0));
                    x0 + (x1 - x0) * f
                })
                .collect()
        })
        .collect();
    Ok(Some(out))
}

/// `n` samples of `clip`'s own sound from `from` samples after its start, before its effects and
/// its gain: in [`CHANNELS`] channels, silent before the clip's start and after its end (it is
/// `len` samples long). `None` when it has no sound.
fn clip_sound(
    media: &Footage,
    asset: &reelforge_renderspec::Asset,
    clip: &reelforge_renderspec::AudioClip,
    rate: u32,
    len: i64,
    from: i64,
    n: usize,
) -> Result<Option<Vec<Vec<f32>>>, String> {
    let (lo, hi) = (from.max(0), (from + n as i64).min(len));
    if hi <= lo {
        return Ok(Some(vec![vec![0.0; n]; CHANNELS]));
    }
    let m = (hi - lo) as usize;
    let at_sample = |t: Tick, rate: u32| (t.0 as i128 * rate as i128 / TICKS_PER_SECOND as i128) as i64;
    let time = &clip.time;
    let speed = time.speed;
    let offset = |i: i64| Tick((i as i128 * TICKS_PER_SECOND as i128 / rate as i128) as i64);
    let src = if clip.pitch != 0.0 && !time.reverses() {
        // read faster (or slower), which moves the pitch, and stretched back to its length
        let tape = if clip.keep_pitch || !time.ramp.is_empty() { 1.0 } else { speed };
        let ratio = 2f64.powf(f64::from(clip.pitch) / 12.0) * tape;
        let src_rate = ((rate as f64 / ratio).round() as u32).max(1);
        let src_in = time.source_in.0 as f64 * src_rate as f64 / TICKS_PER_SECOND as f64;
        let position = |i: i64| {
            if time.ramp.is_empty() {
                (src_in + i as f64 * speed * src_rate as f64 / rate as f64).round() as i64
            } else {
                at_sample(time.source_at(offset(i)), src_rate)
            }
        };
        crate::stretch::stretched(media, asset, position, lo, m, src_rate)
    } else if clip.keep_pitch && !time.reverses() && (speed != 1.0 || !time.ramp.is_empty()) {
        // read at the output rate and stretched in time
        let src_in = at_sample(time.source_in, rate);
        let position = |i: i64| {
            if time.ramp.is_empty() {
                src_in + (i as f64 * speed).round() as i64
            } else {
                at_sample(time.source_at(offset(i)), rate)
            }
        };
        crate::stretch::stretched(media, asset, position, lo, m, rate)
    } else if time.ramp.is_empty() && speed > 0.0 {
        // the source read at `rate / speed` advances `speed` source seconds per output second
        let src_rate = ((rate as f64 / speed).round() as u32).max(1);
        media.audio(asset, at_sample(time.source_in, src_rate) + lo, m, src_rate)
    } else {
        tape(media, asset, |i| time.source_at(offset(i)), lo, m, rate)
    };
    let Some(src) = src? else { return Ok(None) };
    if src.is_empty() {
        return Ok(None);
    }
    let mut out = vec![vec![0f32; n]; CHANNELS];
    for (c, ch) in out.iter_mut().enumerate() {
        let s = &src[c.min(src.len() - 1)];
        let k = s.len().min(m);
        ch[(lo - from) as usize..][..k].copy_from_slice(&s[..k]);
    }
    Ok(Some(out))
}

/// The stereo mix of `frames` samples from output sample `start`.
///
/// A clip at another speed is stretched in time keeping its pitch when it asks to
/// (`keep_pitch`, [`crate::stretch`]), and otherwise read resampled, its pitch moving with the
/// speed as on tape; played backwards it is always read like tape. A freeze frame is silent. A
/// clip's pitch is moved by reading it faster or slower and stretching it back (`audio.pitch/1`),
/// and its effects ([`crate::sound`]) are done before its gain.
pub fn mix(
    spec: &RenderSpec,
    media: &Footage,
    rate: u32,
    start: i64,
    frames: usize,
) -> Result<Vec<Vec<f32>>, ExportError> {
    let mut out = mix_clips(spec, &spec.audio, media, rate, start, frames, 0)?;
    for ch in &mut out {
        for s in ch {
            *s = s.clamp(-1.0, 1.0);
        }
    }
    Ok(out)
}

/// How deep compositions may play compositions (a validated spec has no loop).
const DEPTH: usize = 8;

/// The sum of `clips` over `frames` samples from sample `start` of their timeline (the output's,
/// or a composition's), not yet limited to full scale.
fn mix_clips(
    spec: &RenderSpec,
    clips: &[reelforge_renderspec::AudioClip],
    media: &Footage,
    rate: u32,
    start: i64,
    frames: usize,
    depth: usize,
) -> Result<Vec<Vec<f32>>, ExportError> {
    let mut out = vec![vec![0f32; frames]; CHANNELS];
    let end = start + frames as i64;
    let at_sample = |t: Tick| (t.0 as i128 * rate as i128 / TICKS_PER_SECOND as i128) as i64;
    for clip in clips {
        let (c0, c1) = (at_sample(clip.at), at_sample(clip.at + clip.dur));
        let (a, b) = (start.max(c0), end.min(c1));
        if a >= b {
            continue;
        }
        if clip.time.holds() {
            // a freeze frame is silent
            continue;
        }
        let n = (b - a) as usize;
        // a composition's own mix, from where the clip starts in it; or an asset's sound
        let inner = clip.comp.as_ref().and_then(|id| spec.comps.get(id)).filter(|_| depth < DEPTH);
        let asset = spec.assets.get(&clip.asset);
        if inner.is_none() && asset.is_none() {
            continue;
        }
        let read = |from: i64, n: usize| match (inner, asset) {
            (Some(comp), _) => {
                // silent before the clip's start and after its end, like any clip's sound
                let (lo, hi) = (from.max(0), (from + n as i64).min(c1 - c0));
                let mut all = vec![vec![0f32; n]; CHANNELS];
                if hi > lo {
                    let from_in = at_sample(clip.time.source_in) + lo;
                    let part = mix_clips(spec, &comp.audio, media, rate, from_in, (hi - lo) as usize, depth + 1)
                        .map_err(|e| e.to_string())?;
                    for (ch, src) in all.iter_mut().zip(&part) {
                        ch[(lo - from) as usize..][..src.len()].copy_from_slice(src);
                    }
                }
                Ok(Some(all))
            }
            (None, Some(asset)) => clip_sound(media, asset, clip, rate, c1 - c0, from, n),
            (None, None) => Ok(None),
        };
        let Some(src) = crate::sound::processed(&read, &clip.fx, a - c0, n, rate).map_err(ExportError::Media)? else {
            continue;
        };
        for i in 0..n {
            let rel = Tick(((a - c0 + i as i64) as i128 * TICKS_PER_SECOND as i128 / rate as i128) as i64);
            let gain = 10f32.powf(clip.gain_db.sample(rel) / 20.0);
            for (c, ch) in out.iter_mut().enumerate() {
                let s = src[c.min(src.len() - 1)].get(i).copied().unwrap_or(0.0);
                ch[(a - start) as usize + i] += s * gain;
            }
        }
    }
    Ok(out)
}
