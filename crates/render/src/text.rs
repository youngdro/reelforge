//! Text (`text/1`): shaping, line breaking and anti-aliased glyphs from FilmCraft's text engine
//! (pure Rust: harfrust shaping, skrifa outlines), composited here in linear light.
//!
//! The default face is the bundled Noto Sans SC subset (Regular and Bold), which covers GB 2312
//! Chinese and Latin, so a spec renders the same everywhere, browser included. A spec can bring
//! its own face as a `font` asset. System fonts are never consulted: they differ per machine.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use filmcraft_text::fonts;
use filmcraft_text::layout::{Align, ParagraphStyle, TextStyle};
use filmcraft_text::mask::{StrokeKind, stroke};
use filmcraft_text::raster::{Mask, Path, Xform, compose, fill_into};
use filmcraft_text::render::{device_bounds, draw};
use reelforge_renderspec::{TextAlign, TextContent};

use crate::Image;
use crate::cpu::{apply_fx, blend};
use crate::image::premultiply_srgb;
use crate::plan::PlannedLayer;

/// The family of the bundled default face.
pub const DEFAULT_FAMILY: &str = "Noto Sans SC";

static NOTO_SANS_SC_REGULAR: &[u8] = include_bytes!("../fonts/NotoSansSC-Regular.subset.ttf");
static NOTO_SANS_SC_BOLD: &[u8] = include_bytes!("../fonts/NotoSansSC-Bold.subset.ttf");

/// Registers the bundled faces once.
fn bundled() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        fonts::add_font_data(NOTO_SANS_SC_REGULAR.to_vec());
        fonts::add_font_data(NOTO_SANS_SC_BOLD.to_vec());
    });
}

/// Registers a `font` asset's bytes (once per content hash) and returns its family name.
pub(crate) fn font_family(sha256: &str, bytes: &[u8]) -> Result<String, String> {
    static FAMILIES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let families = FAMILIES.get_or_init(Mutex::default);
    if let Some(f) = families.lock().expect("font lock").get(sha256) {
        return Ok(f.clone());
    }
    let faces = fonts::add_font_data(bytes.to_vec());
    let first = faces.first().ok_or("not a font file this renderer reads (TrueType or OpenType)")?;
    let family = fonts::face(*first).info.family.clone();
    families.lock().expect("font lock").insert(sha256.to_string(), family.clone());
    Ok(family)
}

fn style_name(weight: u16) -> &'static str {
    match (weight + 50) / 100 {
        0 | 1 => "Thin",
        2 => "ExtraLight",
        3 => "Light",
        4 => "Regular",
        5 => "Medium",
        6 => "SemiBold",
        7 => "Bold",
        8 => "ExtraBold",
        _ => "Black",
    }
}

/// A box of mask pixels starting at device pixel `(x, y)`.
struct Region {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
}

/// Draws `text` for `layer` into `out`. `k` is target pixels per output pixel (the preview scale).
pub(crate) fn draw_text(out: &mut Image, layer: &PlannedLayer, text: &TextContent, family: &str, k: f32) {
    bundled();
    if text.text.trim().is_empty() {
        return;
    }
    let style = TextStyle {
        family: family.to_string(),
        style: style_name(text.weight).to_string(),
        size: text.size,
        ..TextStyle::default()
    };
    let para = ParagraphStyle {
        align: match text.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
        width: text.width,
        ..ParagraphStyle::default()
    };
    let l = filmcraft_text::layout(&text.text, &style, &para);
    let b = l.bounds;
    // the block's centre goes to the layer's centre, scaled and turned about it
    let s = k * layer.scale;
    if s <= 0.0 {
        return;
    }
    let (sin, cos) = layer.rotation.sin_cos();
    let centre: Xform = [1.0, 0.0, 0.0, 1.0, -(b[0] + b[2]) / 2.0, -(b[1] + b[3]) / 2.0];
    let place: Xform = [cos * s, sin * s, -sin * s, cos * s, layer.center[0], layer.center[1]];
    let m = compose(&place, &centre);

    let stroke_px = text.stroke.as_ref().map_or(0.0, |st| st.width * s);
    let backing = text.background.as_ref().map(|bg| {
        // the box hugs the inked lines, not the wrap width
        let pad = bg.padding * text.size;
        let (x0, x1) = l.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
        let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (b[0], b[2]) };
        Path::rect(x0 - pad, b[1] - pad, x1 + pad, b[3] + pad).transformed(&m)
    });
    let mut d = device_bounds(&l, &m);
    let grow = stroke_px.ceil() + 2.0;
    d = [d[0] - grow, d[1] - grow, d[2] + grow, d[3] + grow];
    if let Some((x0, y0, x1, y1)) = backing.as_ref().and_then(Path::bounds) {
        d = [d[0].min(x0), d[1].min(y0), d[2].max(x1), d[3].max(y1)];
    }
    // only what lands in the frame (plus the stroke's reach, so edges stroke correctly)
    let x0 = (d[0].floor() as i32).max(-(grow as i32));
    let y0 = (d[1].floor() as i32).max(-(grow as i32));
    let x1 = (d[2].ceil() as i32).min(out.width as i32 + grow as i32);
    let y1 = (d[3].ceil() as i32).min(out.height as i32 + grow as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let r = Region { x: x0, y: y0, w: (x1 - x0) as usize, h: (y1 - y0) as usize };

    let mut fill = Mask::new(r.w, r.h);
    draw(&l, &m, &mut fill, (r.x, r.y));
    let outline = (stroke_px > 0.0).then(|| stroke(&fill, stroke_px, StrokeKind::Outer));
    let behind = backing.map(|p| {
        let mut mk = Mask::new(r.w, r.h);
        fill_into(&p, &mut mk, -r.x as f32, -r.y as f32);
        mk
    });

    let fill_c = premultiply_srgb(text.color.0);
    let stroke_c = text.stroke.as_ref().map(|st| premultiply_srgb(st.color.0));
    let back_c = text.background.as_ref().map(|bg| premultiply_srgb(bg.color.0));
    let over = |top: [f32; 4], cov: f32, under: [f32; 4]| -> [f32; 4] {
        let a = top[3] * cov;
        std::array::from_fn(|i| top[i] * cov + under[i] * (1.0 - a))
    };
    for my in 0..r.h {
        let y = r.y + my as i32;
        if y < 0 || y >= out.height as i32 {
            continue;
        }
        for mx in 0..r.w {
            let x = r.x + mx as i32;
            if x < 0 || x >= out.width as i32 {
                continue;
            }
            let i = my * r.w + mx;
            let mut px = [0.0; 4];
            if let (Some(mk), Some(c)) = (&behind, back_c) {
                px = over(c, mk.a[i].clamp(0.0, 1.0), px);
            }
            if let (Some(mk), Some(c)) = (&outline, stroke_c) {
                px = over(c, mk.a[i].clamp(0.0, 1.0), px);
            }
            px = over(fill_c, fill.a[i].clamp(0.0, 1.0), px);
            if px[3] <= 0.0 {
                continue;
            }
            apply_fx(&mut px, &layer.fx);
            for c in &mut px {
                *c *= layer.opacity;
            }
            let o = (y as u32 * out.width + x as u32) as usize;
            out.pixels[o] = blend(layer.blend, px, out.pixels[o]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_face_covers_chinese_and_latin() {
        bundled();
        let r = fonts::resolve(DEFAULT_FAMILY, "Bold");
        let f = fonts::face(r.face);
        assert_eq!(
            (f.info.family.as_str(), f.info.weight, r.missing, r.synth_bold),
            (DEFAULT_FAMILY, 700, false, false)
        );
        assert!(f.covers_text("短视频剪辑 Reelforge 2026！，。"));
        let r = fonts::resolve(DEFAULT_FAMILY, "Regular");
        assert_eq!(fonts::face(r.face).info.weight, 400);
    }

    #[test]
    fn weights_name_the_nearest_style() {
        assert_eq!(style_name(400), "Regular");
        assert_eq!(style_name(649), "SemiBold");
        assert_eq!(style_name(700), "Bold");
        assert_eq!(style_name(900), "Black");
        assert_eq!(style_name(100), "Thin");
    }
}
