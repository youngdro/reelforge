//! Lottie animations as footage: read once, and any instant drawn as a picture.
//!
//! An animation has no frames of its own, so a picture of it is drawn for the very instant
//! asked, at the size the asset says (see [`picture_size`]), and smaller for a draft. Past its
//! end it goes round again.

use std::sync::Arc;

use reelforge_lottie::{Animation, Bitmap, Files};
use reelforge_render::Image;
use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::{FrameRate, Tick};

use crate::{Footage, Probe, key, sized};

/// Whether a file's name says it may be a Lottie animation (`.json`, `.lottie`).
pub fn is_lottie(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("lottie"))
}

/// Decodes the pictures an animation carries, and finds the ones it names beside itself.
struct Pictures {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    folder: Option<String>,
}

impl Files for Pictures {
    fn decode(&self, bytes: &[u8]) -> Option<Bitmap> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        Some(Bitmap { width: img.width(), height: img.height(), rgba: img.into_raw() })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn beside(&self, path: &str) -> Option<Vec<u8>> {
        // only what is under the animation's own folder
        let path = std::path::Path::new(path);
        if path.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return None;
        }
        std::fs::read(std::path::Path::new(self.folder.as_ref()?).join(path)).ok()
    }
}

/// Reads an animation from a file's bytes; `folder` is where the file is, when it is on disk,
/// for the pictures it names beside itself.
pub(crate) fn open(bytes: &[u8], folder: Option<String>) -> Result<Animation, String> {
    Animation::open(bytes, &Pictures { folder })
}

/// The size pictures of an animation are drawn at: its own, or larger so that its longer side
/// is 1080 pixels (no more than four times its own), since most are drawn small and placed
/// large. Sides are even.
pub fn picture_size(a: &Animation) -> [u32; 2] {
    let k = (1080.0 / a.width.max(a.height)).clamp(1.0, 4.0);
    let even = |v: f32| (((v * k).round() as u32).max(2) / 2) * 2;
    [even(a.width), even(a.height)]
}

/// What importing it measures.
pub(crate) fn probe(a: &Animation) -> Probe {
    Probe {
        kind: AssetKind::Lottie,
        size: Some(picture_size(a)),
        duration: Some(Tick::from_seconds(a.seconds())),
        fps: Some(FrameRate { num: (a.fps as f64 * 1000.0).round() as u32, den: 1000 }).filter(|f| f.is_valid()),
        has_audio: false,
        family: None,
        notes: a.unsupported().to_vec(),
    }
}

/// A picture drawn of an animation, with the instant it shows in sixty-fourths of its frames.
pub(crate) type Drawn = (i64, Arc<Image>);

/// How many pictures of an animation are kept: a clip and its blurred backdrop ask for the same
/// one, and a paused preview for the same one again.
const KEPT: usize = 4;

impl Footage {
    /// Hands over an animation read from bytes, for `lottie` assets with this hash.
    pub(crate) fn insert_lottie(&self, sha256: &str, animation: Animation) {
        self.lotties.lock().expect("lottie lock").insert(sha256.to_string(), Some(Arc::new(animation)));
        self.bump();
    }

    fn lottie(&self, asset: &Asset) -> Option<Arc<Animation>> {
        let k = key(asset);
        if let Some(a) = self.lotties.lock().expect("lottie lock").get(&k) {
            return a.clone();
        }
        // a library decoration is in the program itself, the browser's too
        let opened = match crate::decor::of(&asset.uri) {
            Some(_) => crate::decor::open(&asset.uri),
            None => self.read_lottie(asset),
        }
        .map(Arc::new);
        self.lotties.lock().expect("lottie lock").insert(k, opened.clone());
        opened
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_lottie(&self, asset: &Asset) -> Option<Animation> {
        if asset.uri.contains("://") && !asset.uri.starts_with("file://") {
            return None;
        }
        let path = std::path::Path::new(asset.uri.strip_prefix("file://").unwrap_or(&asset.uri));
        let folder = path.parent().map(|p| p.to_string_lossy().into_owned());
        open(&std::fs::read(path).ok()?, folder).ok()
    }

    #[cfg(target_arch = "wasm32")]
    fn read_lottie(&self, _: &Asset) -> Option<Animation> {
        None
    }

    /// The picture of a `lottie` asset at source instant `at`, `shrink` times smaller each way.
    pub(crate) fn lottie_picture(&self, asset: &Asset, at: Tick, shrink: u32) -> Result<Option<Arc<Image>>, String> {
        let Some(anim) = self.lottie(asset) else { return Ok(None) };
        let own = picture_size(&anim);
        let probe = asset.probe.as_ref();
        let (w, h) = (probe.and_then(|p| p.width).unwrap_or(own[0]), probe.and_then(|p| p.height).unwrap_or(own[1]));
        let shrink = shrink.max(1);
        let (w, h) = ((w / shrink).clamp(1, 8192), (h / shrink).clamp(1, 8192));
        // seconds only here, where the animation's own frames begin
        let frame = anim.frame_at(at.seconds());
        let slot = (frame as f64 * 64.0).round() as i64;
        let k = format!("{}#{w}x{h}", sized(&key(asset), shrink));
        if let Some(img) = self
            .lottie_made
            .lock()
            .expect("lottie lock")
            .get(&k)
            .and_then(|v| v.iter().find(|(s, _)| *s == slot).map(|(_, img)| img.clone()))
        {
            return Ok(Some(img));
        }
        let img = Arc::new(Image::from_srgba8(w, h, &anim.render(frame, w, h)));
        let mut made = self.lottie_made.lock().expect("lottie lock");
        let kept = made.entry(k).or_default();
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push((slot, img.clone()));
        Ok(Some(img))
    }

    /// A picture of a `lottie` asset `height` pixels tall, for the timeline.
    pub(crate) fn lottie_thumbnail(&self, asset: &Asset, at: Tick, height: u32) -> Option<crate::Thumbnail> {
        let anim = self.lottie(asset)?;
        let height = height.clamp(1, 2160);
        let width = ((height as f32 * anim.width / anim.height).round() as u32).clamp(1, 8192);
        let rgba = anim.render(anim.frame_at(at.max(Tick::ZERO).seconds()), width, height);
        Some(crate::Thumbnail { width, height, rgba })
    }
}
