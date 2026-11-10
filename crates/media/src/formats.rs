//! Exports that are not an MP4: an animated GIF, the sound alone (M4A or WAV), and one frame as a
//! picture file (PNG or JPEG).
//!
//! Each writer takes its input a step at a time, so a caller that cannot block (the browser) can
//! yield between steps; [`crate::export`] drives them to the end.

use std::io::{Read, Seek, SeekFrom, Write};

use filmcraft_isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use reelforge_render::{Image, Quality, render_frame};
use reelforge_renderspec::{AudioCodec, AudioEncoding, OutputFormat, RenderSpec};
use reelforge_time::{TICKS_PER_SECOND, Tick};

use crate::Footage;
use crate::export::{CHANNELS, ExportError, ExportStats, Progress, check, encoded_size, mix, mux_err, video_encoding};

fn write_err(e: impl std::fmt::Display) -> ExportError {
    ExportError::Write(e.to_string())
}

/// The frame at `t`, drawn `scale` times the output's size, as an export draws it.
pub fn still(spec: &RenderSpec, media: &Footage, t: Tick, scale: f32) -> Result<Image, ExportError> {
    check(spec)?;
    let last = spec.output.fps.snap((spec.output.duration - Tick(1)).max(Tick::ZERO));
    let t = spec.output.fps.snap(t.clamp(Tick::ZERO, last));
    Ok(render_frame(spec, t, scale, media, Quality::Final)?.0)
}

/// A picture file's kind.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StillFormat {
    /// Lossless, with transparency.
    Png,
    /// Smaller; what is transparent comes out black.
    Jpeg,
}

impl StillFormat {
    /// The extension of such a file.
    pub fn extension(self) -> &'static str {
        match self {
            StillFormat::Png => "png",
            StillFormat::Jpeg => "jpg",
        }
    }
}

/// `img` as the bytes of a picture file.
pub fn encode_still(img: &Image, format: StillFormat) -> Result<Vec<u8>, ExportError> {
    let rgba = img.to_srgba8();
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    match format {
        StillFormat::Png => image::write_buffer_with_format(
            &mut cursor,
            &rgba,
            img.width,
            img.height,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| ExportError::Encode(e.to_string()))?,
        StillFormat::Jpeg => {
            // a JPEG has no transparency: over black
            let rgb: Vec<u8> =
                rgba.chunks_exact(4).flat_map(|p| [0, 1, 2].map(|c| (p[c] as u32 * p[3] as u32 / 255) as u8)).collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 92)
                .encode(&rgb, img.width, img.height, image::ExtendedColorType::Rgb8)
                .map_err(|e| ExportError::Encode(e.to_string()))?;
        }
    }
    Ok(out)
}

/// An animated GIF being written, a frame at a time.
pub struct GifWriter<W: Write> {
    enc: gif::Encoder<W>,
    size: [u32; 2],
    /// How long each frame shows, in hundredths of a second.
    delay: u16,
    total: u64,
    pushed: u64,
}

impl<W: Write> GifWriter<W> {
    /// Starts writing `spec` to `out`: its frames at its encoded size, played round and round.
    pub fn new(spec: &RenderSpec, out: W) -> Result<GifWriter<W>, ExportError> {
        check(spec)?;
        let o = &spec.output;
        let size = encoded_size(spec);
        if size[0] > u16::MAX as u32 || size[1] > u16::MAX as u32 {
            return Err(ExportError::Invalid("a GIF is at most 65535 pixels a side".into()));
        }
        let fps = o.fps;
        // a GIF counts time in hundredths of a second, and players slow anything under two down
        let delay = ((100.0 * fps.den as f64 / fps.num as f64).round() as u16).max(2);
        let mut enc = gif::Encoder::new(out, size[0] as u16, size[1] as u16, &[]).map_err(write_err)?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(write_err)?;
        let total = (fps.frame_at(o.duration - Tick(1)) + 1).max(1) as u64;
        Ok(GifWriter { enc, size, delay, total, pushed: 0 })
    }

    /// Frames in all.
    pub fn frames(&self) -> u64 {
        self.total
    }

    /// The size of every frame.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The instant of the next frame to push, and how much larger than the output to draw it.
    pub fn next(&self, spec: &RenderSpec) -> (Tick, f32) {
        (spec.output.fps.tick_of(self.pushed as i64), video_encoding(spec).scale(spec.output.width))
    }

    /// Writes the next frame; `img` must be of [`size`](Self::size).
    pub fn push(&mut self, img: &Image) -> Result<(), ExportError> {
        if [img.width, img.height] != self.size {
            return Err(ExportError::Encode(format!(
                "a {}x{} frame in a {}x{} GIF",
                img.width, img.height, self.size[0], self.size[1]
            )));
        }
        let mut rgba = img.to_srgba8();
        // each frame gets the 256 colours that suit it best
        let mut frame = gif::Frame::from_rgba_speed(self.size[0] as u16, self.size[1] as u16, &mut rgba, 10);
        frame.delay = self.delay;
        self.enc.write_frame(&frame).map_err(write_err)?;
        self.pushed += 1;
        Ok(())
    }

    /// Finishes the file.
    pub fn finish(self) -> Result<(W, ExportStats), ExportError> {
        let stats =
            ExportStats { frames: self.pushed, audio_samples: 0, renderer: "cpu".into(), encoder: "gif".into() };
        Ok((self.enc.into_inner().map_err(write_err)?, stats))
    }
}

enum Sink<W: Read + Write + Seek> {
    /// 16-bit PCM after a header whose sizes are filled in at the end.
    Wav(W),
    /// AAC in an MP4.
    M4a { mux: Box<Mp4Writer<W>>, enc: Box<filmcraft_aac::Encoder>, track: usize },
}

/// The sound of a spec being written on its own, a second at a time.
pub struct AudioWriter<W: Read + Write + Seek> {
    sink: Sink<W>,
    rate: u32,
    /// Samples written so far, and in all.
    mixed: i64,
    total: i64,
}

const WAV_HEADER: usize = 44;

impl<W: Read + Write + Seek> AudioWriter<W> {
    /// Starts writing the sound of `spec` to `out`, as its format says: a WAV, or AAC in an M4A.
    pub fn new(spec: &RenderSpec, mut out: W) -> Result<AudioWriter<W>, ExportError> {
        check(spec)?;
        if spec.audio.is_empty() {
            return Err(ExportError::Invalid("there is no sound to export".into()));
        }
        let a = spec.output.audio.clone().unwrap_or(AudioEncoding {
            codec: AudioCodec::Aac,
            sample_rate: 48_000,
            bitrate_kbps: 192,
        });
        let rate = a.sample_rate;
        let total = (spec.output.duration.0 as i128 * rate as i128 / TICKS_PER_SECOND as i128) as i64;
        let sink = match spec.output.format {
            OutputFormat::Wav => {
                out.write_all(&[0u8; WAV_HEADER]).map_err(write_err)?;
                Sink::Wav(out)
            }
            OutputFormat::M4a => {
                if a.codec != AudioCodec::Aac {
                    return Err(ExportError::Invalid(format!("an M4A holds AAC sound, not {:?}", a.codec)));
                }
                let enc = filmcraft_aac::Encoder::new(filmcraft_aac::EncoderConfig::cbr(
                    rate,
                    CHANNELS,
                    a.bitrate_kbps * 1000,
                ))
                .map_err(|e| ExportError::Encode(e.to_string()))?;
                let mut mux = Mp4Writer::new(out, WriterOptions::new(Brand::Mp4)).map_err(mux_err)?;
                let mut track =
                    TrackConfig::new(SampleEntry::aac(enc.audio_specific_config(), CHANNELS as u32, rate), rate);
                track.media_start = Some(i64::from(enc.priming_samples()));
                let track = mux.add_track(track).map_err(mux_err)?;
                Sink::M4a { mux: Box::new(mux), enc: Box::new(enc), track }
            }
            other => {
                return Err(ExportError::Invalid(format!("{} is not a sound file", other.extension())));
            }
        };
        Ok(AudioWriter { sink, rate, mixed: 0, total })
    }

    /// How many [`step`](Self::step)s the whole takes.
    pub fn steps(&self) -> u64 {
        (self.total as u64).div_ceil(self.rate as u64).max(1)
    }

    /// Mixes and writes the next second. False once everything is written.
    pub fn step(&mut self, spec: &RenderSpec, media: &Footage) -> Result<bool, ExportError> {
        let n = (self.total - self.mixed).min(self.rate as i64);
        if n <= 0 {
            return Ok(false);
        }
        let planar = mix(spec, media, self.rate, self.mixed, n as usize)?;
        self.mixed += n;
        match &mut self.sink {
            Sink::Wav(out) => {
                let mut bytes = Vec::with_capacity(n as usize * CHANNELS * 2);
                for i in 0..n as usize {
                    for ch in &planar {
                        bytes.extend(((ch[i].clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
                    }
                }
                out.write_all(&bytes).map_err(write_err)?;
            }
            Sink::M4a { mux, enc, track } => {
                let refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
                for au in enc.encode(&refs) {
                    mux.write_sample(
                        *track,
                        WriteSample { data: &au, duration: 1024, composition_offset: 0, is_sync: true },
                    )
                    .map_err(mux_err)?;
                }
            }
        }
        Ok(self.mixed < self.total)
    }

    /// Finishes the file.
    pub fn finish(self) -> Result<(W, ExportStats), ExportError> {
        let mut stats = ExportStats {
            frames: 0,
            audio_samples: self.mixed as u64,
            renderer: String::new(),
            encoder: String::new(),
        };
        match self.sink {
            Sink::Wav(mut out) => {
                let data = (self.mixed as u64 * CHANNELS as u64 * 2).min(u32::MAX as u64 - WAV_HEADER as u64) as u32;
                let mut h = Vec::with_capacity(WAV_HEADER);
                h.extend(b"RIFF");
                h.extend((data + WAV_HEADER as u32 - 8).to_le_bytes());
                h.extend(b"WAVEfmt ");
                h.extend(16u32.to_le_bytes());
                // PCM, the channels, the rate, bytes a second, bytes a sample frame, bits a sample
                h.extend(1u16.to_le_bytes());
                h.extend((CHANNELS as u16).to_le_bytes());
                h.extend(self.rate.to_le_bytes());
                h.extend((self.rate * CHANNELS as u32 * 2).to_le_bytes());
                h.extend((CHANNELS as u16 * 2).to_le_bytes());
                h.extend(16u16.to_le_bytes());
                h.extend(b"data");
                h.extend(data.to_le_bytes());
                out.seek(SeekFrom::Start(0)).map_err(write_err)?;
                out.write_all(&h).map_err(write_err)?;
                out.seek(SeekFrom::End(0)).map_err(write_err)?;
                stats.encoder = "pcm 16-bit".into();
                Ok((out, stats))
            }
            Sink::M4a { mut mux, mut enc, track } => {
                for au in enc.flush() {
                    mux.write_sample(
                        track,
                        WriteSample { data: &au, duration: 1024, composition_offset: 0, is_sync: true },
                    )
                    .map_err(mux_err)?;
                }
                stats.encoder = "aac".into();
                Ok((mux.finish_faststart().map_err(mux_err)?, stats))
            }
        }
    }
}

/// [`crate::export`] for a GIF: every frame drawn by the CPU compositor at the encoded size.
pub(crate) fn export_gif<W: Write>(
    spec: &RenderSpec,
    media: &Footage,
    out: W,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(W, ExportStats), ExportError> {
    let mut w = GifWriter::new(spec, out)?;
    let total = w.frames();
    for i in 0..total {
        let (t, scale) = w.next(spec);
        let (img, _) = render_frame(spec, t, scale, media, Quality::Final)?;
        w.push(&img)?;
        if !progress(Progress { done: i + 1, total }) {
            return Err(ExportError::Cancelled);
        }
    }
    w.finish()
}

/// [`crate::export`] for the sound alone.
pub(crate) fn export_audio<W: Read + Write + Seek>(
    spec: &RenderSpec,
    media: &Footage,
    out: W,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(W, ExportStats), ExportError> {
    let mut w = AudioWriter::new(spec, out)?;
    let total = w.steps();
    let mut done = 0;
    loop {
        let more = w.step(spec, media)?;
        done += 1;
        if !progress(Progress { done: done.min(total), total }) {
            return Err(ExportError::Cancelled);
        }
        if !more {
            break;
        }
    }
    w.finish()
}
