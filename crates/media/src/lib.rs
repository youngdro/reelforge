//! Media in and out, in pure Rust.
//!
//! - [`Footage`] is the [`MediaSource`] the editor, the CLI and a render service hand to
//!   `reelforge-render`: still images, and video decoded by FilmCraft's clean-room codecs (H.264,
//!   HEVC, VP8/9, AV1, ProRes, … in MP4/MOV, Matroska/WebM, MXF, MPEG-TS, AVI). It also reads the
//!   sound of video and audio assets for the export mix.
//! - [`probe_path`] and [`probe_bytes`] measure a file at import: kind, picture size, length.
//! - [`export::export`] draws every frame of a RenderSpec and writes an H.264 + AAC MP4.
//!
//! No FFmpeg and no C libraries, so the same code builds for the browser. There the page hands
//! file bytes over with [`Footage::insert_bytes`]; natively a `file:` URI is read on demand.

pub mod export;
mod yuv;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use filmcraft_media::{FrameRequest, MediaKind, SharedSource};
use reelforge_render::{Image, MediaSource, StillImages};
use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::{FrameRate, Tick};

pub use export::{ExportError, ExportStats, Progress, Renderer, export, export_with};

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
        let bytes: Arc<[u8]> = bytes.into();
        let src = filmcraft_codecs::open_bytes(name, bytes.clone()).map_err(|e| e.to_string())?;
        let probe = probe_source(&src)?;
        if probe.kind == AssetKind::Image {
            self.stills.insert_bytes(sha256, &bytes)?;
        } else {
            self.sources.lock().expect("sources lock").insert(sha256.to_string(), Some(src));
        }
        Ok(probe)
    }

    /// Hands over a font file's bytes (TrueType or OpenType) for `font` assets with this hash.
    pub fn insert_font(&self, sha256: &str, bytes: Vec<u8>) {
        self.fonts.lock().expect("fonts lock").insert(sha256.to_string(), bytes.into());
    }

    /// Forgets everything decoded so far (files changed on disk).
    pub fn clear(&self) {
        self.sources.lock().expect("sources lock").clear();
        self.last.lock().expect("frame lock").clear();
        self.fonts.lock().expect("fonts lock").clear();
    }

    fn source(&self, asset: &Asset) -> Option<SharedSource> {
        let k = key(asset);
        if let Some(s) = self.sources.lock().expect("sources lock").get(&k) {
            return s.clone();
        }
        let opened = self.open(asset);
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

impl MediaSource for Footage {
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
