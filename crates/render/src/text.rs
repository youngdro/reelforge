//! Text (`text/1`): shaping, line breaking and anti-aliased glyphs from FilmCraft's text engine
//! (pure Rust: harfrust shaping, skrifa outlines), composited here in linear light.
//!
//! The default face is the bundled Noto Sans SC subset (Regular and Bold), which covers GB 2312
//! Chinese and Latin, so a spec renders the same everywhere, browser included. A spec can bring
//! its own face as a `font` asset. System fonts are never consulted: they differ per machine.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use filmcraft_text::fonts;
use filmcraft_text::layout::{Align, ParagraphStyle, StyleRun, TextStyle};
use filmcraft_text::mask::{StrokeKind, blur, offset, stroke};
use filmcraft_text::raster::{Mask, Path, Xform, compose, fill_into};
use filmcraft_text::render::{device_bounds, draw};
use reelforge_renderspec::{TextAlign, TextContent};

use crate::Image;
use crate::cpu::{apply_fx, blend};
use crate::image::premultiply_srgb;
use crate::plan::PlannedLayer;

/// The family of the bundled default face.
pub const DEFAULT_FAMILY: &str = "Noto Sans SC";

/// The bundled default face, Regular: Noto Sans SC subset to GB 2312 and Latin (OFL). The editor
/// gives it to egui too, so its own labels show Chinese.
pub static NOTO_SANS_SC_REGULAR: &[u8] = include_bytes!("../fonts/NotoSansSC-Regular.subset.ttf");
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
pub fn font_family(sha256: &str, bytes: &[u8]) -> Result<String, String> {
    registered(sha256, bytes).map(|(family, _)| family)
}

/// What to call a `font` asset: its family, and its style when that is not the regular one
/// ("Inter", "Inter Bold Italic"). Registers the bytes as [`font_family`] does.
pub fn font_label(sha256: &str, bytes: &[u8]) -> Result<String, String> {
    registered(sha256, bytes).map(|(family, style)| {
        if style.is_empty() || style.eq_ignore_ascii_case("regular") { family } else { format!("{family} {style}") }
    })
}

/// A font file's family and style, its faces added to the text engine the first time.
fn registered(sha256: &str, bytes: &[u8]) -> Result<(String, String), String> {
    static NAMES: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    let names = NAMES.get_or_init(Mutex::default);
    // the bundled faces come first, whatever is drawn first
    bundled();
    if let Some(n) = names.lock().expect("font lock").get(sha256) {
        return Ok(n.clone());
    }
    let faces = fonts::add_font_data(bytes.to_vec());
    let first = faces.first().ok_or("not a font file this renderer reads (TrueType or OpenType)")?;
    let info = &fonts::face(*first).info;
    let name = (info.family.clone(), info.style.clone());
    names.lock().expect("font lock").insert(sha256.to_string(), name.clone());
    Ok(name)
}

/// The characters `style`'s family has no glyph for and the bundled face has (Chinese in a Latin
/// font, mostly), as runs set in the bundled face at the same weight and slant. What neither has
/// is left to the text engine's own search.
fn missing(text: &str, style: &TextStyle) -> Vec<StyleRun> {
    if style.family.eq_ignore_ascii_case(DEFAULT_FAMILY) {
        return vec![];
    }
    let own = fonts::face(fonts::resolve(&style.family, &style.style).face);
    let ours = fonts::face(fonts::resolve(DEFAULT_FAMILY, &style.style).face);
    let mut runs: Vec<StyleRun> = vec![];
    for (at, c) in text.char_indices() {
        if c.is_whitespace() || c.is_control() || own.has_char(c) || !ours.has_char(c) {
            continue;
        }
        let end = at + c.len_utf8();
        match runs.last_mut() {
            Some(r) if r.range.end == at => r.range.end = end,
            _ => runs.push(StyleRun {
                range: at..end,
                style: TextStyle { family: DEFAULT_FAMILY.to_string(), ..style.clone() },
            }),
        }
    }
    runs
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

/// Text drawn but not yet composited: a premultiplied picture whose top-left pixel lands on target
/// pixel `(x, y)`, before the layer's effects, opacity and blend. Both compositors draw it.
#[derive(Clone, Debug)]
pub struct Sprite {
    /// Target column of the picture's left edge.
    pub x: i32,
    /// Target row of the picture's top edge.
    pub y: i32,
    /// The picture, shared: the same title on the next frame is the same sprite.
    pub image: Arc<Image>,
}

/// Composites drawn text into `out` on the CPU: the layer's effects, opacity and blend.
pub(crate) fn draw_sprite(out: &mut Image, layer: &PlannedLayer, sp: &Sprite, lut: Option<&crate::Lut>) {
    let w = sp.image.width as i32;
    let h = sp.image.height as f32;
    for (i, px) in sp.image.pixels.iter().enumerate() {
        let (x, y) = (sp.x + i as i32 % w, sp.y + i as i32 / w);
        if px[3] <= 0.0 || x < 0 || y < 0 || x >= out.width as i32 || y >= out.height as i32 {
            continue;
        }
        let mut px = *px;
        let uv = [((i as i32 % w) as f32 + 0.5) / w as f32, ((i as i32 / w) as f32 + 0.5) / h];
        apply_fx(&mut px, &layer.fx, uv, lut);
        let k = layer.opacity * layer.shape.map_or(1.0, |m| m.coverage(uv, w as f32, h));
        for c in &mut px {
            *c *= k;
        }
        let o = (y as u32 * out.width + x as u32) as usize;
        out.pixels[o] = blend(layer.blend, px, out.pixels[o]);
    }
}

/// Lays out and draws `text` for `layer` on a `target`-sized frame. `k` is target pixels per output
/// pixel (the preview scale), clipped to the frame (plus the
/// outline's reach). `None` when nothing lands in the frame.
///
/// Titles mostly sit still, so the last few sprites are kept and a frame that draws the same text
/// the same way gets the same sprite back (the GPU then skips the upload too).
pub fn rasterize(layer: &PlannedLayer, text: &TextContent, family: &str, k: f32, target: [u32; 2]) -> Option<Sprite> {
    type Recent = Mutex<VecDeque<(String, Option<Sprite>)>>;
    static RECENT: OnceLock<Recent> = OnceLock::new();
    const KEEP: usize = 16;
    let key = format!("{text:?}|{family}|{k}|{:?}|{}|{}|{target:?}", layer.center, layer.scale, layer.rotation);
    let recent = RECENT.get_or_init(Mutex::default);
    if let Some((_, sp)) = recent.lock().expect("sprite lock").iter().find(|(k, _)| *k == key) {
        return sp.clone();
    }
    let sp = draw_uncached(layer, text, family, k, target);
    let mut r = recent.lock().expect("sprite lock");
    if r.len() == KEEP {
        r.pop_front();
    }
    r.push_back((key, sp.clone()));
    sp
}

fn draw_uncached(layer: &PlannedLayer, text: &TextContent, family: &str, k: f32, target: [u32; 2]) -> Option<Sprite> {
    bundled();
    if text.text.trim().is_empty() {
        return None;
    }
    let style = TextStyle {
        family: family.to_string(),
        // a family without an italic of that weight is sheared by the text engine
        style: if text.italic {
            format!("{} Italic", style_name(text.weight))
        } else {
            style_name(text.weight).to_string()
        },
        size: text.size,
        tracking: text.tracking * 1000.0,
        underline: text.underline,
        ..TextStyle::default()
    };
    let para = ParagraphStyle {
        align: match text.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
        width: text.width,
        leading: text.leading * text.size,
        ..ParagraphStyle::default()
    };
    let mut l = filmcraft_text::layout_rich(&text.text, &style, &missing(&text.text, &style), &para);
    let b = l.bounds;
    // a typewriter: the glyphs of the first characters only, where the whole text puts them
    let cut = shown_bytes(text);
    if cut == 0 {
        return None;
    }
    if cut < text.text.len() {
        let mut part = (*l).clone();
        part.glyphs.retain(|g| g.cluster < cut);
        // a line's underline goes as far as the line shows
        let shown: Vec<(f32, f32, f32)> = part
            .lines
            .iter()
            .map(|ln| {
                let end = if cut >= ln.range.end {
                    ln.x + ln.width
                } else if cut <= ln.range.start {
                    ln.x
                } else {
                    ln.carets.iter().filter(|(at, _)| *at <= cut).map(|(_, x)| *x).fold(ln.x, f32::max)
                };
                (ln.baseline - ln.ascent, ln.baseline + ln.descent, end)
            })
            .collect();
        let runs = std::mem::take(&mut part.underline_runs);
        let lines = std::mem::take(&mut part.underlines);
        for (u, run) in lines.into_iter().zip(runs) {
            let end = shown.iter().find(|(top, bottom, _)| u[1] >= *top && u[1] <= *bottom).map_or(u[2], |s| s.2);
            if end > u[0] {
                part.underlines.push([u[0], u[1], u[2].min(end), u[3]]);
                part.underline_runs.push(run);
            }
        }
        l = Arc::new(part);
    }
    // the block's centre goes to the layer's centre, scaled and turned about it
    let s = k * layer.scale;
    if s <= 0.0 {
        return None;
    }
    let (sin, cos) = layer.rotation.sin_cos();
    let centre: Xform = [1.0, 0.0, 0.0, 1.0, -(b[0] + b[2]) / 2.0, -(b[1] + b[3]) / 2.0];
    let place: Xform = [cos * s, sin * s, -sin * s, cos * s, layer.center[0], layer.center[1]];
    let m = compose(&place, &centre);

    let stroke_px = text.stroke.as_ref().map_or(0.0, |st| st.width * s);
    let outer_px = text.outer.as_ref().map_or(0.0, |st| st.width * s);
    // the shadow's offset (on the frame) and softness, in target pixels
    let shadow = text.shadow.as_ref().map(|sh| {
        let em = text.size * s;
        ((sh.offset[0] * em).round() as i32, (sh.offset[1] * em).round() as i32, sh.blur * em)
    });
    // the box (or one per line) hugs the inked lines, not the wrap width
    let backing: Vec<Path> = text.background.as_ref().map_or(vec![], |bg| {
        let pad = bg.padding * text.size;
        let r = bg.radius * text.size;
        if bg.lines {
            return l
                .lines
                .iter()
                .filter(|ln| ln.range.start < cut && ln.width > 0.0)
                .map(|ln| {
                    // as far as the line shows
                    let end = if cut >= ln.range.end {
                        ln.x + ln.width
                    } else {
                        ln.carets.iter().filter(|(at, _)| *at <= cut).map(|(_, x)| *x).fold(ln.x, f32::max)
                    };
                    let (top, bottom) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
                    Path::round_rect(ln.x - pad, top - pad * 0.5, end + pad, bottom + pad * 0.5, r).transformed(&m)
                })
                .collect();
        }
        let (x0, x1) = l.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
        let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (b[0], b[2]) };
        vec![Path::round_rect(x0 - pad, b[1] - pad, x1 + pad, b[3] + pad, r).transformed(&m)]
    });
    let mut d = device_bounds(&l, &m);
    let reach = shadow.map_or(0.0, |(dx, dy, sigma)| dx.abs().max(dy.abs()) as f32 + 3.0 * sigma);
    let grow = (stroke_px + outer_px + reach).ceil() + 2.0;
    d = [d[0] - grow, d[1] - grow, d[2] + grow, d[3] + grow];
    for (x0, y0, x1, y1) in backing.iter().filter_map(Path::bounds) {
        d = [d[0].min(x0), d[1].min(y0), d[2].max(x1), d[3].max(y1)];
    }
    // only what lands in the frame (plus the stroke's reach, so edges stroke correctly)
    let x0 = (d[0].floor() as i32).max(-(grow as i32));
    let y0 = (d[1].floor() as i32).max(-(grow as i32));
    let x1 = (d[2].ceil() as i32).min(target[0] as i32 + grow as i32);
    let y1 = (d[3].ceil() as i32).min(target[1] as i32 + grow as i32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let r = Region { x: x0, y: y0, w: (x1 - x0) as usize, h: (y1 - y0) as usize };

    let mut fill = Mask::new(r.w, r.h);
    draw(&l, &m, &mut fill, (r.x, r.y));
    let outline = (stroke_px > 0.0).then(|| stroke(&fill, stroke_px, StrokeKind::Outer));
    let outer = (outer_px > 0.0).then(|| stroke(&fill, stroke_px + outer_px, StrokeKind::Outer));
    let shade = shadow.map(|(dx, dy, sigma)| {
        let mut shape = fill.clone();
        for m in [&outline, &outer].into_iter().flatten() {
            shape.max_with(m);
        }
        blur(&offset(&shape, dx, dy), sigma)
    });
    // each box filled on its own and joined, so where line boxes meet they do not cancel out
    let behind = (!backing.is_empty()).then(|| {
        let mut all = Mask::new(r.w, r.h);
        for p in &backing {
            let mut mk = Mask::new(r.w, r.h);
            fill_into(p, &mut mk, -r.x as f32, -r.y as f32);
            all.max_with(&mk);
        }
        all
    });

    let fill_c = premultiply_srgb(text.color.0);
    // with a gradient, the fill's colour follows the block's height
    let fade = text.gradient.as_ref().map(|g| {
        let bottom = premultiply_srgb(g.0);
        let det = m[0] * m[3] - m[1] * m[2];
        let (top_y, height) = (b[1], (b[3] - b[1]).max(1e-3));
        move |x: f32, y: f32| -> [f32; 4] {
            // the text-space height of a target pixel
            let (px, py) = (x - m[4], y - m[5]);
            let ty = if det.abs() > 1e-9 { (m[0] * py - m[1] * px) / det } else { top_y };
            let t = ((ty - top_y) / height).clamp(0.0, 1.0);
            std::array::from_fn(|i| fill_c[i] + (bottom[i] - fill_c[i]) * t)
        }
    });
    let outer_c = text.outer.as_ref().map(|st| premultiply_srgb(st.color.0));
    let shadow_c = text.shadow.as_ref().map(|sh| premultiply_srgb(sh.color.0));
    let stroke_c = text.stroke.as_ref().map(|st| premultiply_srgb(st.color.0));
    let back_c = text.background.as_ref().map(|bg| premultiply_srgb(bg.color.0));
    let over = |top: [f32; 4], cov: f32, under: [f32; 4]| -> [f32; 4] {
        let a = top[3] * cov;
        std::array::from_fn(|i| top[i] * cov + under[i] * (1.0 - a))
    };
    let mut image = Image::filled(r.w as u32, r.h as u32, [0.0; 4]);
    for (i, px) in image.pixels.iter_mut().enumerate() {
        if let (Some(mk), Some(c)) = (&behind, back_c) {
            *px = over(c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        if let (Some(mk), Some(c)) = (&shade, shadow_c) {
            *px = over(c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        if let (Some(mk), Some(c)) = (&outer, outer_c) {
            *px = over(c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        if let (Some(mk), Some(c)) = (&outline, stroke_c) {
            *px = over(c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        let c = match &fade {
            Some(f) if fill.a[i] > 0.0 => {
                f((r.x + (i % r.w) as i32) as f32 + 0.5, (r.y + (i / r.w) as i32) as f32 + 0.5)
            }
            _ => fill_c,
        };
        *px = over(c, fill.a[i].clamp(0.0, 1.0), *px);
    }
    Some(Sprite { x: r.x, y: r.y, image: Arc::new(image) })
}

/// How many bytes of `text` show: all of it, or the characters a `reveal` lets through.
fn shown_bytes(text: &TextContent) -> usize {
    let Some(r) = &text.reveal else { return text.text.len() };
    let shown = r.sample(reelforge_time::Tick::ZERO).clamp(0.0, 1.0);
    if shown >= 1.0 {
        return text.text.len();
    }
    let n = (text.text.chars().count() as f32 * shown).round() as usize;
    text.text.char_indices().nth(n).map_or(text.text.len(), |(i, _)| i)
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
