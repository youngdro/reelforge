//! Lottie animations, read and drawn in plain Rust.
//!
//! [`Animation::open`] reads a Bodymovin JSON file or a dotLottie archive (`.lottie`), and
//! [`Animation::render`] draws any frame of it, whole or between two, at any size: the animation
//! is vector, so it is as sharp as the picture asked for.
//!
//! What is drawn: shape layers (paths, rectangles, ellipses, stars and polygons; fills, strokes
//! with dashes, linear and radial gradients; groups and their transforms; trim paths, rounded
//! corners, repeaters), solids, pictures, precompositions (with time stretch and remapping),
//! text set from the glyph outlines the file carries, parenting, masks (add, subtract,
//! intersect, inverted), track mattes (alpha and luma), and keys with their easing curves and
//! paths through space. What is not: expressions (the keys beside one are used), effects and
//! layer styles, text animators, blend modes, layers turned in space. [`Animation::unsupported`]
//! says which of those a file uses.
//!
//! Nothing here touches the file system, the clock or threads: the caller hands over bytes and
//! says how pictures are decoded ([`Files`]).

mod geom;
mod model;
mod raster;
mod render;
mod value;
mod zip;

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;

pub use raster::Bitmap;

use geom::Affine;
use model::{Content, Glyph, Layer, Picture, Reading};
use raster::Surface;

/// What reading an animation needs from outside.
pub trait Files {
    /// A picture file's bytes (PNG, JPEG, WebP…) as pixels.
    fn decode(&self, bytes: &[u8]) -> Option<Bitmap>;

    /// A file the animation names beside itself (`images/img_0.png`), when there is such a place.
    fn beside(&self, path: &str) -> Option<Vec<u8>> {
        let _ = path;
        None
    }
}

/// An animation, read.
pub struct Animation {
    /// Its picture's width, in its own units.
    pub width: f32,
    /// Its picture's height.
    pub height: f32,
    /// Frames a second.
    pub fps: f32,
    /// Its first frame.
    pub first: f32,
    /// The frame it ends before.
    pub end: f32,
    layers: Vec<Layer>,
    comps: HashMap<String, Vec<Layer>>,
    pictures: HashMap<String, Picture>,
    glyphs: HashMap<(String, String), Glyph>,
    families: HashMap<String, String>,
    notes: Vec<String>,
}

/// Whether a file is likely a Lottie animation: a dotLottie archive by its name, or JSON with
/// the fields every Bodymovin export has.
pub fn is_lottie(name: &str, bytes: &[u8]) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if ext == "lottie" {
        return zip::Archive::is(bytes);
    }
    if ext != "json" {
        return false;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    head.trim_start().starts_with('{')
        && ["\"layers\"", "\"fr\"", "\"ip\"", "\"op\"", "\"v\""].iter().filter(|k| head.contains(*k)).count() >= 2
}

impl Animation {
    /// Reads `bytes`: Bodymovin JSON, or a dotLottie archive (its first animation).
    pub fn open(bytes: &[u8], files: &dyn Files) -> Result<Animation, String> {
        if !zip::Archive::is(bytes) {
            return Animation::parse(bytes, &|path| files.beside(path), files);
        }
        let archive = zip::Archive::read(bytes).ok_or("not a dotLottie archive")?;
        // the manifest names the animations; without one, the first that is there
        let named = archive
            .file("manifest.json")
            .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
            .and_then(|m| m.get("animations")?.as_array()?.first()?.get("id")?.as_str().map(str::to_string));
        let json = named
            .iter()
            .flat_map(|id| [format!("animations/{id}.json"), format!("a/{id}.json")])
            .find_map(|name| archive.file(&name))
            .or_else(|| {
                let name = archive
                    .names()
                    .find(|n| (n.starts_with("animations/") || n.starts_with("a/")) && n.ends_with(".json"))?
                    .to_string();
                archive.file(&name)
            })
            .ok_or("no animation in the dotLottie archive")?;
        let inside = |path: &str| {
            let file = path.rsplit('/').next().unwrap_or(path);
            [path.to_string(), format!("images/{file}"), format!("i/{file}")].iter().find_map(|n| archive.file(n))
        };
        Animation::parse(&json, &inside, files)
    }

    fn parse(json: &[u8], beside: &dyn Fn(&str) -> Option<Vec<u8>>, files: &dyn Files) -> Result<Animation, String> {
        let root: Value = serde_json::from_slice(json).map_err(|e| format!("not a Lottie animation: {e}"))?;
        let num = |k: &str| root.get(k).and_then(value::number);
        let (Some(width), Some(height), Some(fps)) = (num("w"), num("h"), num("fr")) else {
            return Err("not a Lottie animation: it has no size or frame rate".to_string());
        };
        let (first, end) = (num("ip").unwrap_or(0.0), num("op").unwrap_or(0.0));
        if !(width > 0.0 && height > 0.0 && fps > 0.0 && end > first)
            || !root.get("layers").is_some_and(Value::is_array)
        {
            return Err("not a Lottie animation: it has no layers, no size or no length".to_string());
        }
        let mut reading = Reading::new(&root);
        let layers = model::layers(&mut reading, root.get("layers"));
        let mut comps = HashMap::new();
        let mut pictures = HashMap::new();
        for asset in root.get("assets").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = asset.get("id").and_then(Value::as_str) else { continue };
            if asset.get("layers").is_some() {
                comps.insert(id.to_string(), model::layers(&mut reading, asset.get("layers")));
                continue;
            }
            let Some(p) = asset.get("p").and_then(Value::as_str) else { continue };
            let bytes = match p.strip_prefix("data:").and_then(|d| d.split_once(";base64,")) {
                Some((_, data)) => base64::engine::general_purpose::STANDARD.decode(data.trim()).ok(),
                None => beside(&format!("{}{p}", asset.get("u").and_then(Value::as_str).unwrap_or(""))),
            };
            let bitmap = bytes
                .and_then(|b| files.decode(&b))
                .filter(|b| b.width > 0 && b.height > 0 && b.rgba.len() == b.width as usize * b.height as usize * 4);
            let size = |k: &str, of: Option<u32>| {
                asset.get(k).and_then(value::number).filter(|v| *v > 0.0).or(of.map(|v| v as f32)).unwrap_or(0.0)
            };
            let size = [size("w", bitmap.as_ref().map(|b| b.width)), size("h", bitmap.as_ref().map(|b| b.height))];
            pictures.insert(id.to_string(), Picture { size, bitmap });
        }
        let glyphs = model::glyphs(&mut reading, &root);
        let mut anim = Animation {
            width,
            height,
            fps,
            first,
            end,
            layers,
            comps,
            pictures,
            glyphs,
            families: model::families(&root),
            notes: Vec::new(),
        };
        // what the layers ask for that is not there
        let mut notes: Vec<String> = reading.notes.iter().map(|n| n.to_string()).collect();
        let all = || anim.layers.iter().chain(anim.comps.values().flatten());
        if all().any(
            |l| matches!(&l.content, Content::Image(id) if anim.pictures.get(id).is_none_or(|p| p.bitmap.is_none())),
        ) {
            notes.push("pictures that are not in the file".to_string());
        }
        let missing = all().any(|l| match &l.content {
            Content::Text(t) => {
                let family = anim.families.get(&t.font).cloned().unwrap_or_else(|| t.font.clone());
                t.text.chars().any(|c| !c.is_whitespace() && !c.is_control() && anim.glyph(c, &family).is_none())
            }
            _ => false,
        });
        if missing {
            notes.push("text whose letter shapes are not in the file".to_string());
        }
        notes.sort();
        anim.notes = notes;
        Ok(anim)
    }

    fn glyph(&self, c: char, family: &str) -> Option<&Glyph> {
        let ch = c.to_string();
        self.glyphs
            .get(&(ch.clone(), family.to_string()))
            .or_else(|| self.glyphs.iter().find(|(k, _)| k.0 == ch).map(|(_, g)| g))
    }

    /// How many frames it has.
    pub fn frames(&self) -> f32 {
        self.end - self.first
    }

    /// How long it is, in seconds.
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.fps as f64
    }

    /// What the file uses that is not drawn, in words; empty when everything is.
    pub fn unsupported(&self) -> &[String] {
        &self.notes
    }

    /// The frame that shows `seconds` from its start, going round again past its end.
    pub fn frame_at(&self, seconds: f64) -> f32 {
        let frames = self.frames() as f64;
        self.first + (seconds * self.fps as f64).rem_euclid(frames) as f32
    }

    /// Draws `frame` (from [`first`](Self::first) up to [`end`](Self::end), whole or between
    /// two) as a picture `width` by `height`: straight RGBA, 8 bits each, see-through where
    /// nothing is drawn.
    pub fn render(&self, frame: f32, width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        let mut target = Surface::new(w, h);
        let base = Affine::scale(w as f32 / self.width, h as f32 / self.height);
        render::Renderer::new(self, w, h).comp(&self.layers, frame, base, &mut target, 0, None);
        target.to_rgba8()
    }
}

#[cfg(test)]
mod tests;
