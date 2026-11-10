//! Media in and out, in pure Rust.
//!
//! - [`Footage`] is the [`MediaSource`] the editor, the CLI and a render service hand to
//!   `reelforge-render`: still images, and video decoded by FilmCraft's clean-room codecs (H.264,
//!   HEVC, VP8/9, AV1, ProRes, … in MP4/MOV, Matroska/WebM, MXF, MPEG-TS, AVI). It also reads the
//!   sound of video and audio assets for the export mix.
//! - Lottie animations (Bodymovin JSON, dotLottie) are footage too: any instant of one is drawn
//!   as a picture, as sharp as asked ([`lottie`]).
//! - [`probe_path`] and [`probe_bytes`] measure a file at import: kind, picture size, length.
//! - [`Footage::thumbnail`] and [`Footage::audio`] give the timeline its pictures and waveforms.
//! - [`export::export`] draws every frame of a RenderSpec and writes an H.264 + AAC MP4.
//!
//! No FFmpeg and no C libraries, so the same code builds for the browser. There the page hands
//! file bytes over with [`Footage::insert_bytes`]; natively a `file:` URI is read on demand.

pub mod analysis;
pub mod between;
pub mod decor;
pub mod export;
pub mod formats;
pub mod library;
pub mod lottie;
pub mod picture;
mod shrunk;
pub mod sound;
pub mod steady;
pub mod stickers;
pub mod stretch;
pub mod webcodecs;
mod yuv;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use filmcraft_media::{FrameRequest, MediaKind, SharedSource};
use reelforge_render::{Image, Lut, MediaSource, StillImages, Yuv};
use reelforge_renderspec::{Asset, AssetKind, Between};
use reelforge_time::{FrameRate, Tick};

pub use export::{
    EncodedFrame, EncoderFactory, ExportError, ExportStats, ExportWriter, Progress, Renderer, SoftwareH264,
    VideoConfig, VideoEncoder, Yuv420, draw_frame, encoded_size, export, export_with, open_encoder,
    register_video_encoder, video_encoding,
};
pub use formats::{AudioWriter, GifWriter, StillFormat, encode_still, still};

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
    /// What a font is called: its family, and its style when not the regular one.
    pub family: Option<String>,
    /// What the file uses that is not drawn, in words (a Lottie animation's expressions, effects…).
    pub notes: Vec<String>,
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
        family: None,
        notes: Vec::new(),
    })
}

/// Measures a file in memory (`name` gives the extension, for formats that need one).
pub fn probe_bytes(name: &str, bytes: Arc<[u8]>) -> Result<Probe, String> {
    if lottie::is_lottie(name) {
        return lottie::open(&bytes, None).map(|a| lottie::probe(&a));
    }
    probe_source(&filmcraft_codecs::open_bytes(name, bytes).map_err(|e| e.to_string())?)
}

/// Measures a file on disk, reading only what the container index needs.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_path(path: &std::path::Path) -> Result<Probe, String> {
    if lottie::is_lottie(&path.to_string_lossy()) {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let folder = path.parent().map(|p| p.to_string_lossy().into_owned());
        return lottie::open(&bytes, folder).map(|a| lottie::probe(&a));
    }
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

/// A picture made ahead and the source instant it shows.
type Made = (Tick, Arc<Image>);

/// A decoded frame and the picture made from it.
type Converted = (Arc<filmcraft_frame::VideoFrame>, Arc<Image>);
type Wrapped = (Arc<filmcraft_frame::VideoFrame>, Arc<Yuv>);

/// What is kept of an asset's pictures made between its frames ([`MediaSource::picture_between`]).
#[derive(Default)]
struct Tween {
    /// The frames last made pictures of, the two the last picture lies between: playing on, the
    /// later one is the earlier one of the next pair.
    frames: Vec<Converted>,
    /// How the picture moved from one frame to the next, for the pair it was last looked for.
    flow: Option<(Arc<filmcraft_frame::VideoFrame>, Arc<filmcraft_frame::VideoFrame>, Arc<between::Flow>)>,
    /// The last pictures made, oldest first: a clip and its blurred backdrop ask for the same one.
    made: Vec<(Tick, Between, Arc<Image>)>,
}

/// How many pictures made between frames are kept per asset: as many as are made ahead.
const TWEEN_PICTURES: usize = AHEAD_PICTURES;
/// An instant this near a frame (as a part of the way to the next) shows that frame as it is.
const ON_A_FRAME: f32 = 0.04;

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
    /// The last frame handed over unconverted per asset, the same way: a compositor keeps what it
    /// uploaded by the picture's address.
    last_yuv: Mutex<HashMap<String, Wrapped>>,
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
    /// Pictures made ahead of the playhead ([`prefetch`](Footage::prefetch)), by asset key: the
    /// few source instants a playing preview reaches next, oldest first.
    ahead: Mutex<HashMap<String, Vec<Made>>>,
    /// Pictures made between frames, and what they were made from, by asset key and size.
    tween: Mutex<HashMap<String, Tween>>,
    /// Lottie animations by asset key; `None` remembers a file that could not be read.
    lotties: Mutex<HashMap<String, Option<Arc<reelforge_lottie::Animation>>>>,
    /// The last pictures drawn of each animation, by asset key and size, oldest first, each with
    /// the instant it shows (in sixty-fourths of the animation's frames).
    lottie_made: Mutex<HashMap<String, Vec<lottie::Drawn>>>,
}

/// How many pictures [`Footage::prefetch`] keeps per asset. Full-size pictures are large (a
/// 1080×1920 one is 33 MB), so only the next few frames.
pub const AHEAD_PICTURES: usize = 4;

/// Whether a file name is a lookup table (`.cube`).
pub fn is_lut(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("cube"))
}

/// Whether a file name is a font (`.ttf`, `.otf`, `.ttc`).
pub fn is_font(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|e| ["ttf", "otf", "ttc"].iter().any(|f| e.eq_ignore_ascii_case(f)))
}

/// What to call a font file: its family, with its style when that is not the regular one
/// (`sha256` is its content hash, under which the text engine keeps it); an error when it is not
/// a font this renderer reads.
pub fn font_name(sha256: &str, bytes: &[u8]) -> Result<String, String> {
    reelforge_render::text::font_label(sha256, bytes)
}

fn key(asset: &Asset) -> String {
    if asset.sha256.is_empty() { asset.uri.clone() } else { asset.sha256.clone() }
}

/// The key of an asset's pictures made `shrink` times smaller.
fn sized(key: &str, shrink: u32) -> String {
    if shrink > 1 { format!("{key}@{shrink}") } else { key.to_string() }
}

impl Footage {
    /// An empty cache.
    pub fn new() -> Footage {
        Footage::default()
    }

    /// Hands over a file's bytes (the browser has no file system) and measures them. `name` gives
    /// the extension.
    pub fn insert_bytes(&self, sha256: &str, name: &str, bytes: Vec<u8>) -> Result<Probe, String> {
        if is_font(name) {
            let family = Some(font_name(sha256, &bytes)?);
            self.insert_font(sha256, bytes);
            return Ok(Probe {
                kind: AssetKind::Font,
                size: None,
                duration: None,
                fps: None,
                has_audio: false,
                family,
                notes: Vec::new(),
            });
        }
        if is_lut(name) {
            let lut = Lut::parse_cube(&String::from_utf8_lossy(&bytes))?;
            self.luts.lock().expect("luts lock").insert(sha256.to_string(), Arc::new(lut));
            self.bump();
            return Ok(Probe {
                kind: AssetKind::Lut,
                size: None,
                duration: None,
                fps: None,
                has_audio: false,
                family: None,
                notes: Vec::new(),
            });
        }
        if lottie::is_lottie(name) {
            let animation = lottie::open(&bytes, None)?;
            let probe = lottie::probe(&animation);
            self.insert_lottie(sha256, animation);
            return Ok(probe);
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
        self.bump();
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
        self.ahead.lock().expect("ahead lock").clear();
        self.sources.lock().expect("sources lock").clear();
        self.last.lock().expect("frame lock").clear();
        self.last_yuv.lock().expect("frame lock").clear();
        self.tween.lock().expect("tween lock").clear();
        self.fonts.lock().expect("fonts lock").clear();
        self.luts.lock().expect("luts lock").clear();
        self.lotties.lock().expect("lottie lock").clear();
        self.lottie_made.lock().expect("lottie lock").clear();
        self.bump();
    }

    /// Changes whenever files are handed over or forgotten: what was offline may be online now.
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn bump(&self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// A still image; a library sticker is drawn the first time it is asked for.
    fn still(&self, asset_id: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Image>>, String> {
        if let Some(id) = stickers::id_of(&asset.uri)
            && !self.stills.has(&asset.sha256)
            && let Some(px) = stickers::render(id, stickers::SIZE)
        {
            self.stills.insert(&asset.sha256, Image::from_srgba8(stickers::SIZE, stickers::SIZE, &px));
        }
        self.stills.picture(asset_id, asset, at)
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
                let Some(img) = self.still("", asset, Tick::ZERO)? else { return Ok(None) };
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
            AssetKind::Lottie => Ok(self.lottie_thumbnail(asset, at, height)),
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
            AssetKind::Image => self.still(asset_id, asset, at),
            AssetKind::Video => {
                if let Some(img) =
                    self.prepared.lock().expect("prepared lock").get(&key(asset)).and_then(|m| m.get(&at))
                {
                    return Ok(Some(img.clone()));
                }
                let k = key(asset);
                if let Some(img) = self
                    .ahead
                    .lock()
                    .expect("ahead lock")
                    .get(&k)
                    .and_then(|v| v.iter().find(|(t, _)| *t == at).map(|(_, img)| img.clone()))
                {
                    return Ok(Some(img));
                }
                self.decode_picture(asset, &k, at, 1)
            }
            AssetKind::Lottie => self.lottie_picture(asset, at, 1),
            _ => Ok(None),
        }
    }

    fn picture_shrunk(
        &self,
        asset_id: &str,
        asset: &Asset,
        at: Tick,
        shrink: u32,
    ) -> Result<Option<Arc<Image>>, String> {
        if asset.kind == AssetKind::Lottie {
            // drawn at the smaller size, not drawn large and shrunk
            return self.lottie_picture(asset, at, shrink);
        }
        if asset.kind != AssetKind::Video || shrink <= 1 {
            return self.picture(asset_id, asset, at);
        }
        // pictures prepared for an export are full size: they are used as they are
        if let Some(img) = self.prepared.lock().expect("prepared lock").get(&key(asset)).and_then(|m| m.get(&at)) {
            return Ok(Some(img.clone()));
        }
        let k = sized(&key(asset), shrink);
        if let Some(img) = self
            .ahead
            .lock()
            .expect("ahead lock")
            .get(&k)
            .and_then(|v| v.iter().find(|(t, _)| *t == at).map(|(_, img)| img.clone()))
        {
            return Ok(Some(img));
        }
        self.decode_picture(asset, &k, at, shrink)
    }

    fn picture_between(
        &self,
        asset_id: &str,
        asset: &Asset,
        at: Tick,
        how: Between,
        shrink: u32,
    ) -> Result<Option<Arc<Image>>, String> {
        let nearest = || {
            if shrink > 1 {
                self.picture_shrunk(asset_id, asset, at, shrink)
            } else {
                self.picture(asset_id, asset, at)
            }
        };
        if asset.kind != AssetKind::Video || how.is_nearest() {
            return nearest();
        }
        let k = sized(&key(asset), shrink);
        let made = |t: &Tween| t.made.iter().find(|(t, h, _)| (*t, *h) == (at, how)).map(|(_, _, img)| img.clone());
        if let Some(img) = self.tween.lock().expect("tween lock").get(&k).and_then(made) {
            return Ok(Some(img));
        }
        // the frames at the instants of the file's own rate before and after: one frame length
        // apart from the file's start
        let Some(length) = self.frame_length(asset).filter(|l| *l > Tick::ZERO && at >= Tick::ZERO) else {
            return nearest();
        };
        let start = length * (at.0 / length.0);
        let (Some(first), Some(second)) = (self.frame(asset, start)?, self.frame(asset, start + length)?) else {
            return nearest();
        };
        // the file's last frame has nothing after it
        if Arc::ptr_eq(&first, &second) {
            return nearest();
        }
        let gap = length;
        let f = (((at - start).0 as f64 / gap.0 as f64) as f32).clamp(0.0, 1.0);
        let (a, b) = (self.tween_picture(&k, &first, shrink), self.tween_picture(&k, &second, shrink));
        let flow = |t: &Tween| match &t.flow {
            Some((x, y, flow)) if Arc::ptr_eq(x, &first) && Arc::ptr_eq(y, &second) => Some(flow.clone()),
            _ => None,
        };
        let mut flowed = None;
        let img = if f < ON_A_FRAME {
            a.clone()
        } else if f > 1.0 - ON_A_FRAME {
            b.clone()
        } else if how == Between::Flow && (a.width, a.height) == (b.width, b.height) {
            let flow = match self.tween.lock().expect("tween lock").get(&k).and_then(flow) {
                Some(flow) => flow,
                None => Arc::new(between::flow(&a, &b)),
            };
            let img = Arc::new(between::between(&a, &b, &flow, f));
            flowed = Some(flow);
            img
        } else if (a.width, a.height) == (b.width, b.height) {
            Arc::new(between::blend(&a, &b, f))
        } else {
            a.clone()
        };
        let mut tween = self.tween.lock().expect("tween lock");
        let t = tween.entry(k).or_default();
        if let Some(flow) = flowed {
            t.flow = Some((first.clone(), second.clone(), flow));
        }
        t.frames = vec![(first, a), (second, b)];
        t.made.push((at, how, img.clone()));
        if t.made.len() > TWEEN_PICTURES {
            t.made.remove(0);
        }
        Ok(Some(img))
    }

    fn yuv(&self, _: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Yuv>>, String> {
        if asset.kind != AssetKind::Video {
            return Ok(None);
        }
        let k = key(asset);
        // a picture made already (for an export, or ahead) is used as it is
        if self.prepared.lock().expect("prepared lock").get(&k).is_some_and(|m| m.contains_key(&at))
            || self.ahead.lock().expect("ahead lock").get(&k).is_some_and(|v| v.iter().any(|(t, _)| *t == at))
        {
            return Ok(None);
        }
        let Some(frame) = self.frame(asset, at)? else { return Ok(None) };
        if let Some((held, y)) = self.last_yuv.lock().expect("frame lock").get(&k)
            && Arc::ptr_eq(held, &frame)
        {
            return Ok(Some(y.clone()));
        }
        let Some(y) = as_yuv(&frame) else { return Ok(None) };
        let y = Arc::new(y);
        self.last_yuv.lock().expect("frame lock").insert(k, (frame, y.clone()));
        Ok(Some(y))
    }
}

/// The picture of a decoded frame, `shrink` times smaller each way.
fn converted(frame: &filmcraft_frame::VideoFrame, shrink: u32) -> Image {
    if shrink > 1 {
        let (w, h, bytes) = shrunk::srgba8(frame, shrink as usize);
        Image::from_srgba8(w, h, &bytes)
    } else {
        Image::from_srgba8(frame.width, frame.height, &frame.to_rgba8())
    }
}

/// A decoded frame's planes as they are, when the compositor's shader can convert them: 8-bit,
/// no alpha, a display (BT.709 or sRGB) transfer.
fn as_yuv(frame: &filmcraft_frame::VideoFrame) -> Option<Yuv> {
    use filmcraft_color::{Range, Transfer};
    use filmcraft_frame::PixelData;
    let PixelData::Yuv8 { planes, chroma, alpha: None } = &frame.data else { return None };
    if !matches!(frame.color.transfer, Transfer::Bt709 | Transfer::Srgb) {
        return None;
    }
    let y = Yuv {
        width: frame.width,
        height: frame.height,
        planes: planes.clone(),
        shift: chroma.shifts(),
        kr_kb: frame.color.matrix.kr_kb(),
        full_range: frame.color.range == Range::Full,
    };
    y.is_whole().then_some(y)
}

impl Footage {
    /// How long one frame of `asset` (a video) lasts, when the file says.
    pub fn frame_length(&self, asset: &Asset) -> Option<Tick> {
        let src = self.source(asset)?;
        let rate = src.info().video.as_ref()?.frame_rate;
        let rate = FrameRate { num: rate.num as u32, den: rate.den as u32 };
        rate.is_valid().then(|| rate.frame_duration())
    }

    /// The frame of `asset` (a video) at source instant `at`, small and grey, its longer side
    /// about [`steady::SIDE`]: what the camera's movement is measured on. `Ok(None)` when the file
    /// is offline.
    pub fn gray(&self, asset: &Asset, at: Tick) -> Result<Option<steady::Gray>, String> {
        let Some(frame) = self.frame(asset, at)? else { return Ok(None) };
        let longer = frame.width.max(frame.height);
        // the decoder's own planes shrunk first, where that is enough
        let shrink = [8u32, 4, 2].into_iter().find(|s| longer / s >= steady::SIDE).unwrap_or(1);
        let (w, h, bytes) = if shrink > 1 {
            shrunk::srgba8(&frame, shrink as usize)
        } else {
            (frame.width, frame.height, frame.to_rgba8())
        };
        let to_height = (u64::from(h) * u64::from(steady::SIDE) / u64::from(w.max(h).max(1))).max(1) as u32;
        let small = Thumbnail::shrink(w, h, &bytes, to_height);
        Ok(Some(steady::Gray::from_rgba(small.width as usize, small.height as usize, &small.rgba)))
    }

    /// The decoded frame of `asset` (a video) at `at`; past the end the last frame holds.
    fn frame(&self, asset: &Asset, at: Tick) -> Result<Option<Arc<filmcraft_frame::VideoFrame>>, String> {
        let Some(src) = self.source(asset) else { return Ok(None) };
        let duration = from_ft(src.info().duration);
        let at = at.max(Tick::ZERO).min(duration - Tick(1)).max(Tick::ZERO);
        src.video_frame(FrameRequest::full(to_ft(at))).map(Some).map_err(|e| e.to_string())
    }

    /// Decodes and converts `asset`'s picture at `at` (a video), `shrink` times smaller each way,
    /// reusing the last one converted when the decoder hands back the same frame. `k` is
    /// [`sized`]`(key, shrink)`.
    fn decode_picture(&self, asset: &Asset, k: &str, at: Tick, shrink: u32) -> Result<Option<Arc<Image>>, String> {
        let Some(frame) = self.frame(asset, at)? else { return Ok(None) };
        if let Some((held, img)) = self.last.lock().expect("frame lock").get(k)
            && Arc::ptr_eq(held, &frame)
        {
            return Ok(Some(img.clone()));
        }
        let img = Arc::new(converted(&frame, shrink));
        self.last.lock().expect("frame lock").insert(k.to_string(), (frame, img.clone()));
        Ok(Some(img))
    }

    /// The picture of `frame` for a picture made between frames: the one kept under `k` when it
    /// is one of the last two, else made now.
    fn tween_picture(&self, k: &str, frame: &Arc<filmcraft_frame::VideoFrame>, shrink: u32) -> Arc<Image> {
        let kept = |t: &Tween| t.frames.iter().find(|(held, _)| Arc::ptr_eq(held, frame)).map(|(_, img)| img.clone());
        if let Some(img) = self.tween.lock().expect("tween lock").get(k).and_then(kept) {
            return img;
        }
        Arc::new(converted(frame, shrink))
    }

    /// Makes `asset`'s picture at source instant `at` ahead of time, on the calling thread, so a
    /// later [`picture`](MediaSource::picture) for exactly that instant only looks it up. For a
    /// playing preview: a worker thread calls this for the frames the playhead reaches next. Only
    /// videos are made ahead (stills are kept anyway); the oldest picture goes past
    /// [`AHEAD_PICTURES`]. Returns whether a picture is now ready.
    /// `shrink` is as for [`MediaSource::picture_shrunk`] (1: full size).
    pub fn prefetch(&self, asset: &Asset, at: Tick, shrink: u32) -> bool {
        if asset.kind != AssetKind::Video {
            return false;
        }
        let k = key(asset);
        if self.prepared.lock().expect("prepared lock").get(&k).is_some_and(|m| m.contains_key(&at)) {
            return true;
        }
        let k = sized(&k, shrink);
        if self.ahead.lock().expect("ahead lock").get(&k).is_some_and(|v| v.iter().any(|(t, _)| *t == at)) {
            return true;
        }
        let Ok(Some(img)) = self.decode_picture(asset, &k, at, shrink) else { return false };
        let mut ahead = self.ahead.lock().expect("ahead lock");
        let list = ahead.entry(k).or_default();
        if !list.iter().any(|(t, _)| *t == at) {
            list.push((at, img));
            if list.len() > AHEAD_PICTURES {
                list.remove(0);
            }
        }
        true
    }

    /// Only decodes `asset`'s frame at `at` (a video), filling the decoder's own frame cache
    /// without converting it: further ahead than [`prefetch`](Self::prefetch) keeps pictures.
    pub fn warm(&self, asset: &Asset, at: Tick) {
        if asset.kind != AssetKind::Video {
            return;
        }
        let Some(src) = self.source(asset) else { return };
        let duration = from_ft(src.info().duration);
        let at = at.max(Tick::ZERO).min(duration - Tick(1)).max(Tick::ZERO);
        let _ = src.video_frame(FrameRequest::full(to_ft(at)));
    }

    /// Forgets the pictures made ahead (the preview stopped playing).
    pub fn clear_ahead(&self) {
        self.ahead.lock().expect("ahead lock").clear();
    }
}

#[cfg(test)]
mod tests;
