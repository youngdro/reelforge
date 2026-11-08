//! A RenderSpec to an MP4 file: every frame drawn at full size (on the GPU where there is one,
//! else by the CPU reference compositor), encoded as H.264 High, the sound mixed and encoded as AAC-LC, muxed with the index in front
//! ("fast start") so a browser can play it while it downloads.

use std::io::{Read, Seek, Write};

use filmcraft_isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use reelforge_render::{Quality, RenderError, render_frame, unsupported_features};
use reelforge_renderspec::{AudioCodec, AudioEncoding, RenderSpec, VideoCodec, VideoEncoding, validate};
use reelforge_time::{TICKS_PER_SECOND, Tick};
use thiserror::Error;

use crate::Footage;
use crate::yuv::{Yuv420, to_yuv420};

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
}

const CHANNELS: usize = 2;

fn mux_err(e: impl std::fmt::Display) -> ExportError {
    ExportError::Write(e.to_string())
}

/// Renders `spec` and writes an MP4 to `out`. `progress` is called after every frame; returning
/// `false` stops the export with [`ExportError::Cancelled`].
///
/// Video is H.264 at the spec's quality (`crf`, default 20) or bitrate. Sound is written when the
/// spec has audio clips: AAC-LC, stereo, at the spec's rate and bitrate (default 48 kHz,
/// 192 kbit/s).
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
    validate(spec).map_err(|e| ExportError::Invalid(e.to_string()))?;
    let missing = unsupported_features(spec);
    if !missing.is_empty() {
        return Err(RenderError::Unsupported(missing).into());
    }
    let o = &spec.output;
    let venc = o.video.clone().unwrap_or(VideoEncoding { codec: VideoCodec::H264, crf: Some(20), bitrate_kbps: None });
    if venc.codec != VideoCodec::H264 {
        return Err(ExportError::Invalid(format!("this exporter writes H.264 only, not {:?}", venc.codec)));
    }
    let mut drawer = Drawer::new(renderer)?;
    let renderer = drawer.name();
    let fps = o.fps;
    let total = (fps.frame_at(o.duration - Tick(1)) + 1).max(1) as u64;

    let mut cfg = filmcraft_h264enc::EncoderConfig::new(o.width, o.height, fps.num, fps.den);
    cfg.format = filmcraft_h264enc::PacketFormat::LengthPrefixed;
    cfg.rate = match (venc.crf, venc.bitrate_kbps) {
        (Some(crf), _) => filmcraft_h264enc::RateControl::Crf(f32::from(crf)),
        (None, Some(kbps)) => filmcraft_h264enc::RateControl::Vbr { target_kbps: kbps, max_kbps: kbps * 2 },
        (None, None) => filmcraft_h264enc::RateControl::Crf(20.0),
    };
    cfg.keyint = (fps.fps() * 2.0).round().max(1.0) as u32;
    let mut enc = filmcraft_h264enc::Encoder::new(cfg).map_err(|e| ExportError::Encode(e.to_string()))?;

    let mut mux = Mp4Writer::new(out, WriterOptions::new(Brand::Mp4)).map_err(mux_err)?;
    let avc = AvcConfig::parse(&enc.avcc()).map_err(mux_err)?;
    let mut vtrack = TrackConfig::new(SampleEntry::avc(avc, o.width as u16, o.height as u16), fps.num);
    if enc.delay() > 0 {
        // with B-frames the first decode time is one frame before the first presentation time
        vtrack.media_start = Some(i64::from(fps.den));
    }
    let vtrack = mux.add_track(vtrack).map_err(mux_err)?;

    let mut sound = if spec.audio.is_empty() {
        None
    } else {
        let a =
            o.audio.clone().unwrap_or(AudioEncoding { codec: AudioCodec::Aac, sample_rate: 48_000, bitrate_kbps: 192 });
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

    let write_video = |mux: &mut Mp4Writer<W>, ps: Vec<filmcraft_h264enc::Packet>| -> Result<(), ExportError> {
        for p in ps {
            let s = WriteSample {
                data: &p.data,
                duration: fps.den,
                composition_offset: (p.pts - p.dts) as i32,
                is_sync: p.keyframe,
            };
            mux.write_sample(vtrack, s).map_err(mux_err)?;
        }
        Ok(())
    };
    each_frame(spec, media, &mut drawer, total, &mut |i, yuv| {
        let frame = filmcraft_h264enc::YuvFrame {
            y: &yuv.y,
            u: &yuv.u,
            v: &yuv.v,
            y_stride: yuv.width,
            uv_stride: yuv.chroma_width(),
        };
        let ps =
            enc.try_encode(&frame, i as i64 * i64::from(fps.den)).map_err(|e| ExportError::Encode(e.to_string()))?;
        write_video(&mut mux, ps)?;
        if let Some(s) = &mut sound {
            // keep the sound about as far along as the picture so the file interleaves
            s.mix_until(spec, media, &mut mux, fps.tick_of(i as i64 + 1).min(o.duration))?;
        }
        if progress(Progress { done: i + 1, total }) { Ok(()) } else { Err(ExportError::Cancelled) }
    })?;
    write_video(&mut mux, enc.flush())?;
    let mut stats = ExportStats { frames: total, audio_samples: 0, renderer };
    if let Some(mut s) = sound {
        s.mix_until(spec, media, &mut mux, o.duration)?;
        for au in s.enc.flush() {
            mux.write_sample(s.track, WriteSample { data: &au, duration: 1024, composition_offset: 0, is_sync: true })
                .map_err(mux_err)?;
        }
        stats.audio_samples = s.mixed as u64;
    }
    let out = mux.finish_faststart().map_err(mux_err)?;
    Ok((out, stats))
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

    fn draw(&mut self, spec: &RenderSpec, media: &Footage, i: u64, yuv: &mut Yuv420) -> Result<(), ExportError> {
        let t = spec.output.fps.tick_of(i as i64);
        match self {
            Drawer::Cpu => {
                let (img, _) = render_frame(spec, t, 1.0, media, Quality::Final)?;
                to_yuv420(&img, yuv);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Drawer::Gpu(g) => {
                g.render(spec, t, 1.0, media, Quality::Final)?;
                g.read_yuv420(&mut yuv.y, &mut yuv.u, &mut yuv.v).map_err(|e| ExportError::Gpu(e.to_string()))?;
                yuv.width = spec.output.width as usize;
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
                    let r = drawer.draw(spec, media, i, &mut yuv).map(|()| yuv);
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
            drawer.draw(spec, media, i, &mut yuv)?;
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

/// The stereo mix of `frames` samples from output sample `start`.
///
/// A clip at another speed is read resampled (its pitch moves with the speed, as on tape);
/// pitch-preserving stretch comes with the audio engine.
pub fn mix(
    spec: &RenderSpec,
    media: &Footage,
    rate: u32,
    start: i64,
    frames: usize,
) -> Result<Vec<Vec<f32>>, ExportError> {
    let mut out = vec![vec![0f32; frames]; CHANNELS];
    let end = start + frames as i64;
    let at_sample = |t: Tick| (t.0 as i128 * rate as i128 / TICKS_PER_SECOND as i128) as i64;
    for clip in &spec.audio {
        let Some(asset) = spec.assets.get(&clip.asset) else { continue };
        let (c0, c1) = (at_sample(clip.at), at_sample(clip.at + clip.dur));
        let (a, b) = (start.max(c0), end.min(c1));
        if a >= b {
            continue;
        }
        let speed = clip.time.speed;
        // the source read at `rate / speed` advances `speed` source seconds per output second
        let src_rate = ((rate as f64 / speed).round() as u32).max(1);
        let src_in = (clip.time.source_in.0 as i128 * src_rate as i128 / TICKS_PER_SECOND as i128) as i64;
        let n = (b - a) as usize;
        let Some(src) = media.audio(asset, src_in + (a - c0), n, src_rate).map_err(ExportError::Media)? else {
            continue;
        };
        if src.is_empty() {
            continue;
        }
        for i in 0..n {
            let rel = Tick(((a - c0 + i as i64) as i128 * TICKS_PER_SECOND as i128 / rate as i128) as i64);
            let gain = 10f32.powf(clip.gain_db.sample(rel) / 20.0);
            for (c, ch) in out.iter_mut().enumerate() {
                let s = src[c.min(src.len() - 1)].get(i).copied().unwrap_or(0.0);
                ch[(a - start) as usize + i] += s * gain;
            }
        }
    }
    for ch in &mut out {
        for s in ch {
            *s = s.clamp(-1.0, 1.0);
        }
    }
    Ok(out)
}
