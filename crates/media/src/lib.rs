//! Media in and out, in pure Rust.
//!
//! - [`Footage`] is the [`MediaSource`] the editor, the CLI and a render service hand to
//!   `reelforge-render`: still images, and video decoded by FilmCraft's clean-room codecs (H.264,
//!   HEVC, VP8/9, AV1, ProRes, … in MP4/MOV, Matroska/WebM, MXF, MPEG-TS, AVI). It also reads the
//!   sound of video and audio assets for the export mix.
//! - [`probe_path`] and [`probe_bytes`] measure a file at import: kind, picture size, length.
//! - [`Footage::thumbnail`] and [`Footage::audio`] give the timeline its pictures and waveforms.
//! - [`export::export`] draws every frame of a RenderSpec and writes an H.264 + AAC MP4.
//!
//! No FFmpeg and no C libraries, so the same code builds for the browser. There the page hands
//! file bytes over with [`Footage::insert_bytes`]; natively a `file:` URI is read on demand.

pub mod export;
pub mod library;
pub mod stretch;
pub mod webcodecs;
mod yuv;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use filmcraft_media::{FrameRequest, MediaKind, SharedSource};
use reelforge_render::{Image, Lut, MediaSource, StillImages};
use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::{FrameRate, Tick};

pub use export::{
    EncodedFrame, ExportError, ExportStats, ExportWriter, Progress, Renderer, SoftwareH264, VideoConfig, VideoEncoder,
    Yuv420, draw_frame, encoded_size, export, export_with, video_encoding,
};

/// What [`probe_path`] measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Video, image or audio.
    pub kind: AssetKind,
    /// Picture size, for pictures.
    pub size: Option<[u32; 2]>,
    /// Length, for time-based media.
    pub duration: Option<Tick>,
    /// The video's frame rate, when the container gives one.
    pub fps: Option<FrameRate>,
    /// Whether there is sound.
    pub has_audio: bool,
}

fn to_ft(t: Tick) -> filmcraft_time::Tick {
    filmcraft_time::Tick(t.0)
}

fn from_ft(t: filmcraft_time::Tick) -> Tick {
    Tick(t.0)
}

fn probe_source(src: &SharedSource) -> Result<Probe, String> {
    let info = src.info();
    let kind = match info.kind {
        MediaKind::Still => AssetKind::Image,
        MediaKind::AudioOnly => AssetKind::Audio,
        MediaKind::Movie | MediaKind::ImageSequence | MediaKind::Synthetic if info.video.is_some() => AssetKind::Video,
        _ if !info.audio_streams.is_empty() => AssetKind::Audio,
        _ => return Err(format!("{}: no picture or sound", info.name)),
    };
    let v = info.video.as_ref();
    Ok(Probe {
        kind,
        size: v.map(|v| [v.width, v.height]),
        duration: (kind != AssetKind::Image).then(|| from_ft(info.duration)),
        fps: v
            .map(|v| FrameRate { num: v.frame_rate.num as u32, den: v.frame_rate.den as u32 })
            .filter(|f| f.is_valid()),
        has_audio: !info.audio_streams.is_empty(),
    })
}

/// Measures a file in memory (`name` gives the extension, for formats that need one).
pub fn probe_bytes(name: &str, bytes: Arc<[u8]>) -> Result<Probe, String> {
    probe_source(&filmcraft_codecs::open_bytes(name, bytes).map_err(|e| e.to_string())?)
}

/// Measures a file on disk, reading only what the container index needs.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_path(path: &std::path::Path) -> Result<Probe, String> {
    probe_source(&open_path(path)?)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_path(path: &std::path::Path) -> Result<SharedSource, String> {
    let reader = filmcraft_media::reader::FileReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    filmcraft_media::reader::open_reader(
        &name,
        Arc::new(reader),
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
    )
    .map_err(|e| e.to_string())
}

/// A decoded frame and the picture made from it.
type Converted = (Arc<filmcraft_frame::VideoFrame>, Arc<Image>);

/// Footage for the renderer: stills, decoded video frames and sound, cached by content hash.
#[derive(Default)]
pub struct Footage {
    stills: StillImages,
    /// Opened containers by asset key; `None` remembers a file that could not be opened.
    sources: Mutex<HashMap<String, Option<SharedSource>>>,
    /// The last frame converted per asset, so a preview that gets the same decoded frame again
    /// (a held frame, a slow clip, a paused playhead) does not convert it again. Holding the
    /// decoded frame keeps the pointer comparison sound.
    last: Mutex<HashMap<String, Converted>>,
    /// Font files by asset key.
    fonts: Mutex<HashMap<String, Arc<[u8]>>>,
    /// Lookup tables by asset key.
    luts: Mutex<HashMap<String, Arc<Lut>>>,
    /// Counts files handed over and clears, so callers know when offline footage may be back.
    generation: std::sync::atomic::AtomicU64,
    /// The bytes of video files handed over, by asset key, for a decoder outside this crate.
    raw: Mutex<HashMap<String, Arc<[u8]>>>,
    /// Pictures decoded elsewhere (the browser's decoder), by asset key and source instant: a
    /// [`picture`](MediaSource::picture) asking for exactly that instant gets them.
    prepared: Mutex<HashMap<String, HashMap<Tick, Arc<Image>>>>,
}

/// Whether a file name is a lookup table (`.cube`).
pub fn is_lut(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("cube"))
}

fn key(asset: &Asset) -> String {
    if asset.sha256.is_empty() { asset.uri.clone() } else { asset.sha256.clone() }
}

impl Footage {
    /// An empty cache.
    pub fn new() -> Footage {
        Footage::default()
    }

    /// Hands over a file's bytes (the browser has no file system) and measures them. `name` gives
    /// the extension.
    pub fn insert_bytes(&self, sha256: &str, name: &str, bytes: Vec<u8>) -> Result<Probe, String> {
        if is_lut(name) {
            let lut = Lut::parse_cube(&String::from_utf8_lossy(&bytes))?;
            self.luts.lock().expect("luts lock").insert(sha256.to_string(), Arc::new(lut));
            self.bump();
            return Ok(Probe { kind: AssetKind::Lut, size: None, duration: None, fps: None, has_audio: false });
        }
        let bytes: Arc<[u8]> = bytes.into();
        let src = filmcraft_codecs::open_bytes(name, bytes.clone()).map_err(|e| e.to_string())?;
        let probe = probe_source(&src)?;
        if probe.kind == AssetKind::Image {
            self.stills.insert_bytes(sha256, &bytes)?;
        } else {
            self.sources.lock().expect("sources lock").insert(sha256.to_string(), Some(src));
            if probe.kind == AssetKind::Video {
                self.raw.lock().expect("raw lock").insert(sha256.to_string(), bytes);
            }
        }
        self.bump();
        Ok(probe)
    }

    /// Hands over a font file's bytes (TrueType or OpenType) for `font` assets with this hash.
    pub fn insert_font(&self, sha256: &str, bytes: Vec<u8>) {
        self.fonts.lock().expect("fonts lock").insert(sha256.to_string(), bytes.into());
    }

    /// The bytes of a video file handed over with [`insert_bytes`](Self::insert_bytes).
    pub fn video_bytes(&self, asset: &Asset) -> Option<Arc<[u8]>> {
        self.raw.lock().expect("raw lock").get(&key(asset)).cloned()
    }

    /// Hands over `asset`'s picture at source instant `at`, decoded elsewhere; it is used for
    /// exactly that instant until [`unprepare`](Self::unprepare).
    pub fn prepare(&self, asset: &Asset, at: Tick, picture: Arc<Image>) {
        self.prepared.lock().expect("prepared lock").entry(key(asset)).or_default().insert(at, picture);
    }

    /// Drops the pictures handed over for `asset`.
    pub fn unprepare(&self, asset: &Asset) {
        self.prepared.lock().expect("prepared lock").remove(&key(asset));
    }

    /// Forgets every picture handed over with [`prepare`](Self::prepare).
    pub fn clear_prepared(&self) {
        self.prepared.lock().expect("prepared lock").clear();
    }

    /// Forgets everything decoded so far (files changed on disk).
    pub fn clear(&self) {
        self.prepared.lock().expect("prepared lock").clear();
        self.sources.lock().expect("sources lock").clear();
        self.last.lock().expect("frame lock").clear();
        self.fonts.lock().expect("fonts lock").clear();
        self.luts.lock().expect("luts lock").clear();
        self.bump();
    }

    /// Changes whenever files are handed over or forgotten: what was offline may be online now.
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn bump(&self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn source(&self, asset: &Asset) -> Option<SharedSource> {
        let k = key(asset);
        if let Some(s) = self.sources.lock().expect("sources lock").get(&k) {
            return s.clone();
        }
        let opened = match library::id_of(&asset.uri) {
            // made, not read: the same bytes everywhere, browser included
            Some(id) => library::wav(id).and_then(|b| filmcraft_codecs::open_bytes("sound.wav", b.into()).ok()),
            None => self.open(asset),
        };
        self.sources.lock().expect("sources lock").insert(k, opened.clone());
        opened
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open(&self, asset: &Asset) -> Option<SharedSource> {
        if asset.uri.contains("://") && !asset.uri.starts_with("file://") {
            return None;
        }
        let path = asset.uri.strip_prefix("file://").unwrap_or(&asset.uri);
        open_path(std::path::Path::new(path)).ok()
    }

    #[cfg(target_arch = "wasm32")]
    fn open(&self, _: &Asset) -> Option<SharedSource> {
        None
    }

    /// `frames` samples per channel of `asset`'s sound from sample `start`, at `rate` Hz.
    /// `Ok(None)` when the file is offline or has no sound.
    pub fn audio(&self, asset: &Asset, start: i64, frames: usize, rate: u32) -> Result<Option<Vec<Vec<f32>>>, String> {
        if !matches!(asset.kind, AssetKind::Video | AssetKind::Audio) {
            return Ok(None);
        }
        let Some(src) = self.source(asset) else { return Ok(None) };
        if src.info().audio_streams.is_empty() {
            return Ok(None);
        }
        match src.audio(start, frames, rate) {
            Ok(buf) => Ok(Some(buf.channels)),
            Err(filmcraft_media::MediaError::NoStream(_)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A small picture for the timeline: `width × height` pixels of sRGB `[r, g, b, a]` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, row by row.
    pub rgba: Vec<u8>,
}

impl Thumbnail {
    /// `rgba` (`width × height`) shrunk to `height` pixels tall, each pixel the mean of the pixels
    /// it covers. Never enlarged.
    pub fn shrink(width: u32, height: u32, rgba: &[u8], to_height: u32) -> Thumbnail {
        let to_h = to_height.clamp(1, height.max(1));
        let to_w = ((width as u64 * to_h as u64) / height.max(1) as u64).max(1) as u32;
        let (w, h) = (width as usize, height as usize);
        let mut out = Vec::with_capacity(to_w as usize * to_h as usize * 4);
        for ty in 0..to_h as usize {
            let (y0, y1) = (ty * h / to_h as usize, ((ty + 1) * h / to_h as usize).max(ty * h / to_h as usize + 1));
            for tx in 0..to_w as usize {
                let (x0, x1) = (tx * w / to_w as usize, ((tx + 1) * w / to_w as usize).max(tx * w / to_w as usize + 1));
                let mut sum = [0u32; 4];
                for y in y0..y1.min(h) {
                    for px in rgba[(y * w + x0) * 4..(y * w + x1.min(w)) * 4].as_chunks::<4>().0 {
                        for (total, &v) in sum.iter_mut().zip(px) {
                            *total += v as u32;
                        }
                    }
                }
                let n = ((y1.min(h) - y0) * (x1.min(w) - x0)).max(1) as u32;
                out.extend(sum.map(|v| ((v + n / 2) / n) as u8));
            }
        }
        Thumbnail { width: to_w, height: to_h, rgba: out }
    }
}

impl Footage {
    /// A picture of `asset` at source instant `at`, `height` pixels tall, for the timeline. Decodes
    /// on the calling thread. `Ok(None)` when the file is offline or has no picture.
    pub fn thumbnail(&self, asset: &Asset, at: Tick, height: u32) -> Result<Option<Thumbnail>, String> {
        match asset.kind {
            AssetKind::Image => {
                let Some(img) = self.stills.picture("", asset, Tick::ZERO)? else { return Ok(None) };
                Ok(Some(Thumbnail::shrink(img.width, img.height, &img.to_srgba8(), height)))
            }
            AssetKind::Video => {
                let Some(src) = self.source(asset) else { return Ok(None) };
                let Some(v) = src.info().video.as_ref().map(|v| v.height) else { return Ok(None) };
                let duration = from_ft(src.info().duration);
                let at = at.max(Tick::ZERO).min(duration - Tick(1)).max(Tick::ZERO);
                let scale = (height as f32 / v.max(1) as f32).clamp(0.01, 1.0);
                let frame = src.video_frame(FrameRequest { time: to_ft(at), scale }).map_err(|e| e.to_string())?;
                Ok(Some(Thumbnail::shrink(frame.width, frame.height, &frame.to_rgba8(), height)))
            }
            _ => Ok(None),
        }
    }
}

impl MediaSource for Footage {
    fn lut(&self, _: &str, asset: &Asset) -> Result<Option<Arc<Lut>>, String> {
        if asset.kind != AssetKind::Lut {
            return Ok(None);
        }
        let k = key(asset);
        if let Some(l) = self.luts.lock().expect("luts lock").get(&k) {
            return Ok(Some(l.clone()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if asset.uri.contains("://") && !asset.uri.starts_with("file://") {
                return Ok(None);
            }
            let path = asset.uri.strip_prefix("file://").unwrap_or(&asset.uri);
            let Ok(text) = std::fs::read_to_string(path) else { return Ok(None) };
            let lut = Arc::new(Lut::parse_cube(&text).map_err(|e| format!("{path}: {e}"))?);
            self.luts.lock().expect("luts lock").insert(k, lut.clone());
            Ok(Some(lut))
        }
        #[cfg(target_arch = "wasm32")]
        Ok(None)
    }

    fn font(&self, _: &str, asset: &Asset) -> Result<Option<Arc<[u8]>>, String> {
        if asset.kind != AssetKind::Font {
            return Ok(None);
        }
        let k = key(asset);
        if let Some(b) = self.fonts.lock().expect("fonts lock").get(&k) {
            return Ok(Some(b.clone()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if asset.uri.contains("://") && !asset.uri.starts_with("file://") {
                return Ok(None);
            }
            let path = asset.uri.strip_prefix("file://").unwrap_or(&asset.uri);
            let Ok(bytes) = std::fs::read(path) else { return Ok(None) };
            let bytes: Arc<[u8]> = bytes.into();
            self.fonts.lock().expect("fonts lock").insert(k, bytes.clone());
            Ok(Some(bytes))
        }
        #[cfg(target_arch = "wasm32")]
        Ok(None)
    }

    fn picture(&self, asset_id: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Image>>, String> {
        match asset.kind {
            AssetKind::Image => self.stills.picture(asset_id, asset, at),
            AssetKind::Video => {
                if let Some(img) =
                    self.prepared.lock().expect("prepared lock").get(&key(asset)).and_then(|m| m.get(&at))
                {
                    return Ok(Some(img.clone()));
                }
                let Some(src) = self.source(asset) else { return Ok(None) };
                let duration = from_ft(src.info().duration);
                // past the end the last frame holds
                let at = at.max(Tick::ZERO).min(duration - Tick(1)).max(Tick::ZERO);
                let frame = src.video_frame(FrameRequest::full(to_ft(at))).map_err(|e| e.to_string())?;
                let k = key(asset);
                if let Some((held, img)) = self.last.lock().expect("frame lock").get(&k)
                    && Arc::ptr_eq(held, &frame)
                {
                    return Ok(Some(img.clone()));
                }
                let img = Arc::new(Image::from_srgba8(frame.width, frame.height, &frame.to_rgba8()));
                self.last.lock().expect("frame lock").insert(k, (frame, img.clone()));
                Ok(Some(img))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
