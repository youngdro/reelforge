//! Text (`text/1`): shaping, line breaking and anti-aliased glyphs from FilmCraft's text engine
//! (pure Rust: harfrust shaping, skrifa outlines), composited here in linear light.
//!
//! The default face is the bundled Noto Sans SC subset (Regular and Bold), which covers GB 2312
//! Chinese and Latin, so a spec renders the same everywhere, browser included. A spec can bring
//! its own face as a `font` asset. System fonts are never consulted: they differ per machine.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use filmcraft_text::fonts;
use filmcraft_text::layout::{Align, Glyph, Layout, ParagraphStyle, StyleRun, TextStyle};
use filmcraft_text::mask::{StrokeKind, blur, offset, stroke};
use filmcraft_text::raster::{IDENTITY, Mask, Path, Xform, compose, fill_into};
use filmcraft_text::render::{device_bounds, draw, glyph_path};
use reelforge_renderspec::{CharPose, TextAlign, TextBacking, TextContent, TextUnit};

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
    /// For a layer that is stretched, turned in space or carried: the map from the picture's
    /// pixels to target pixels. It was drawn flat and is placed like a picture, and `x` and `y`
    /// are where it was drawn. `None` for a sprite that lands pixel on pixel.
    pub map: Option<crate::plan::Map>,
    /// With `map`, for a layer smeared along its movement: the map at each instant its shutter is
    /// open, drawn in place of `map`.
    pub smear: Vec<crate::plan::Map>,
}

/// Composites drawn text into `out` on the CPU: the layer's effects, opacity and blend.
pub(crate) fn draw_sprite(out: &mut Image, layer: &PlannedLayer, sp: &Sprite, lut: Option<&crate::Lut>) {
    let w = sp.image.width as i32;
    let h = sp.image.height as f32;
    let warp = crate::distort::Warp::of(&layer.fx);
    for (i, px) in sp.image.pixels.iter().enumerate() {
        let (x, y) = (sp.x + i as i32 % w, sp.y + i as i32 / w);
        if x < 0 || y < 0 || x >= out.width as i32 || y >= out.height as i32 {
            continue;
        }
        let uv = [((i as i32 % w) as f32 + 0.5) / w as f32, ((i as i32 / w) as f32 + 0.5) / h];
        let mut px = match &warp {
            Some(warp) => warp.sample(uv, [w as f32, h], |x, y| sp.image.sample(x, y)),
            None => *px,
        };
        if px[3] <= 0.0 {
            continue;
        }
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

/// `text` laid out in `family`, in output pixels.
fn laid(text: &TextContent, family: &str) -> Arc<filmcraft_text::Layout> {
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
    filmcraft_text::layout_rich(&text.text, &style, &missing(&text.text, &style), &para)
}

/// The size, in output pixels, of the box a title's anchor is a point of: as tall as its lines
/// together, and as wide as its longest line reaches to either side of the middle of the block
/// it is laid out in (which is what sits at the layer's centre). Words centred in a wide wrap
/// are measured as words, and a title aligned to one side starts at that side of the box.
pub(crate) fn block(text: &TextContent, family: &str) -> [f32; 2] {
    bundled();
    let l = laid(text, family);
    let b = l.bounds;
    let middle = (b[0] + b[2]) / 2.0;
    let (x0, x1) = l.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
    let half = if x0 <= x1 { (middle - x0).abs().max((x1 - middle).abs()) } else { (b[2] - b[0]) / 2.0 };
    [2.0 * half, b[3] - b[1]]
}

fn draw_uncached(layer: &PlannedLayer, text: &TextContent, family: &str, k: f32, target: [u32; 2]) -> Option<Sprite> {
    bundled();
    if text.text.trim().is_empty() {
        return None;
    }
    let mut l = laid(text, family);
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
    // written on: a character not yet done leaves the layout; a pen follows as much of its
    // outline as it has got to, and its ink comes in behind the pen
    let mut written: Vec<(Glyph, f32)> = vec![];
    if let Some(dr) = &text.draw
        && let p = dr.progress.sample(reelforge_time::Tick::ZERO).clamp(0.0, 1.0)
        && p < 1.0
    {
        let starts = starts(&text.text);
        let step = dr.step.max(0.0);
        let own = 1.0 / (1.0 + step * starts.len().saturating_sub(1) as f32);
        let along = |g: &Glyph| {
            let i = starts.partition_point(|s| *s <= g.cluster).saturating_sub(1) as f32;
            ((p - i * step * own) / own).clamp(0.0, 1.0)
        };
        let mut rest = (*l).clone();
        written = rest.glyphs.iter().map(|g| (*g, along(g))).filter(|(_, u)| *u > 0.0 && *u < 1.0).collect();
        rest.glyphs.retain(|g| along(g) >= 1.0);
        rest.underlines.clear();
        rest.underline_runs.clear();
        l = Arc::new(rest);
    }
    // characters on the move are drawn one by one, each about its own centre
    let mut movers = vec![];
    if !text.chars.is_empty() || text.scatter.is_some() || text.arc != 0.0 {
        let mut rest = (*l).clone();
        movers = moved(text, &mut rest);
        l = Arc::new(rest);
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
    let em = text.size * s;
    // each shadow's offset (on the frame) and softness, in target pixels: the first, then the
    // ones beneath it
    let shadows: Vec<(i32, i32, f32)> = text
        .shadow
        .iter()
        .chain(&text.shadows)
        .map(|sh| ((sh.offset[0] * em).round() as i32, (sh.offset[1] * em).round() as i32, sh.blur * em))
        .collect();
    let finish = Finish {
        stroke_px,
        outer_px,
        shadows,
        depth: text.depth.as_ref().map(|d| ((d.offset[0] * em).round() as i32, (d.offset[1] * em).round() as i32)),
        hollow_px: text.hollow.map(|w| (w * em).max(1.0)),
        bevel: text.bevel.as_ref().map(|bv| {
            let (sin, cos) = bv.angle.to_radians().sin_cos();
            let reach = (bv.size * em).max(1.0);
            ((sin * reach).round() as i32, (-cos * reach).round() as i32, reach)
        }),
    };
    // where a target pixel is in the text's own space, as the lines were laid out flat
    let det = m[0] * m[3] - m[1] * m[2];
    let round = bend(text, &l);
    let flat = move |x: f32, y: f32| -> (f32, f32) {
        if det.abs() <= 1e-9 {
            return (b[0], b[1]);
        }
        let (px, py) = (x - m[4], y - m[5]);
        let (tx, ty) = ((m[3] * px - m[2] * py) / det, (m[0] * py - m[1] * px) / det);
        match round {
            None => (tx, ty),
            // back off the circle the lines were bent round
            Some((mx, yref, r)) => {
                let (vx, vy, side) = (tx - mx, ty - (yref + r), r.signum());
                let rho = (vx * vx + vy * vy).sqrt() * side;
                (mx + (vx * side).atan2(-vy * side) * r, yref + r - rho)
            }
        }
    };
    // the box (or one per line) hugs the inked lines, not the wrap width
    let backing: Vec<Path> = text.background.as_ref().map_or(vec![], |bg| {
        let pad = bg.padding * text.size;
        let r = bg.radius * text.size;
        // its sides lean about its own middle
        let lean = bg.skew.to_radians().tan();
        let boxed = |x0: f32, y0: f32, x1: f32, y1: f32| {
            let shear: Xform = [1.0, 0.0, -lean, 1.0, lean * (y0 + y1) / 2.0, 0.0];
            Path::round_rect(x0, y0, x1, y1, r).transformed(&compose(&m, &shear))
        };
        match bg.shape {
            TextBacking::Box => {}
            // a band behind the lower half of each line, as far as the line shows
            TextBacking::Marker => {
                return l
                    .lines
                    .iter()
                    .filter(|ln| ln.range.start < cut && ln.width > 0.0)
                    .map(|ln| {
                        let end = if cut >= ln.range.end {
                            ln.x + ln.width
                        } else {
                            ln.carets.iter().filter(|(at, _)| *at <= cut).map(|(_, x)| *x).fold(ln.x, f32::max)
                        };
                        boxed(ln.x - pad, ln.baseline - ln.ascent * 0.42, end + pad, ln.baseline + ln.descent * 0.6)
                    })
                    .collect();
            }
            // a tile behind each character that shows
            TextBacking::Tiles => {
                // each stands about its character: the space the tracking adds after one is
                // shared between its sides, and a hair is left between two tiles
                let (lead, gap) = ((text.tracking * text.size).max(0.0) / 2.0, text.size * 0.05);
                return starts(&text.text)
                    .into_iter()
                    .filter(|at| *at < cut)
                    .filter_map(|at| {
                        let (k, x0, x1) = span(&l, at)?;
                        let ln = &l.lines[k];
                        let (top, bottom) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
                        Some(boxed(x0 - lead + gap, top + gap, x1 - lead - gap, bottom - gap))
                    })
                    .collect();
            }
        }
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
                    boxed(ln.x - pad, top - pad * 0.5, end + pad, bottom + pad * 0.5)
                })
                .collect();
        }
        let (x0, x1) = l.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
        let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (b[0], b[2]) };
        vec![boxed(x0 - pad, b[1] - pad, x1 + pad, b[3] + pad)]
    });
    // the pen's lines on the target, and how much of each shows: it goes as the ink arrives
    let pen_half = text.draw.as_ref().map_or(0.0, |dr| (dr.width * text.size * s / 2.0).max(0.5));
    let mut pens: Vec<(Vec<Vec<[f32; 2]>>, f32)> = vec![];
    for (g, u) in &written {
        let outline = glyph_path(g, &compose(&m, &[1.0, 0.0, 0.0, 1.0, g.x, g.y]));
        let end = 1.0 - (1.0 - (u / 0.7).min(1.0)).powi(3);
        let lines: Vec<Vec<[f32; 2]>> = outline
            .contours
            .iter()
            .flat_map(|c| {
                let one = crate::shape::Contour { points: c.iter().map(|p| [p.0, p.1]).collect(), closed: true };
                crate::shape::trimmed(&[one], 0.0, end)
            })
            .collect();
        let ink = ((u - 0.6) / 0.4).clamp(0.0, 1.0);
        pens.push((lines, 1.0 - ink));
        if ink > 0.0 {
            let bounds = [g.x, g.y - g.size, g.x + g.size, g.y + g.size * 0.3];
            let one = Layout { glyphs: vec![*g], bounds, ..Layout::default() };
            movers.push(Mover { one, to: IDENTITY, opacity: ink, tint: None });
        }
    }
    let mut d = device_bounds(&l, &m);
    for p in pens.iter().flat_map(|(lines, _)| lines.iter().flatten()) {
        d = [
            d[0].min(p[0] - pen_half),
            d[1].min(p[1] - pen_half),
            d[2].max(p[0] + pen_half),
            d[3].max(p[1] + pen_half),
        ];
    }
    for mv in &mut movers {
        mv.to = compose(&m, &mv.to);
        let at = device_bounds(&mv.one, &mv.to);
        d = [d[0].min(at[0]), d[1].min(at[1]), d[2].max(at[2]), d[3].max(at[3])];
    }
    let grow = (stroke_px + outer_px + finish.reach()).ceil() + 2.0;
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
    // units that take colours in turn: what each colour inks, to weigh the colours by
    let turns = text.palette.as_ref().map(|pal| pal.colors.len().max(1));
    let mut tints: Vec<Mask> = turns.map_or(vec![], |n| vec![Mask::new(r.w, r.h); n]);
    if let Some(n) = turns {
        let whole = laid(text, family);
        let of = tints_of(text, &whole).unwrap_or_default();
        let starts = starts(&text.text);
        for (j, tint) in tints.iter_mut().enumerate() {
            let mut part = (*l).clone();
            part.underlines.clear();
            part.underline_runs.clear();
            part.glyphs.retain(|g| {
                starts
                    .partition_point(|s| *s <= g.cluster)
                    .checked_sub(1)
                    .and_then(|i| of.get(i))
                    .is_some_and(|t| t % n == j)
            });
            if !part.glyphs.is_empty() {
                draw(&part, &m, tint, (r.x, r.y));
            }
        }
    }
    // a character that shows whole is inked with the rest, so their outlines run together; one
    // that shows in part is inked on its own and laid over
    let mut faint: Vec<(Region, Coat, f32, Option<usize>)> = vec![];
    for mv in &movers {
        if mv.opacity >= 1.0 {
            draw(&mv.one, &mv.to, &mut fill, (r.x, r.y));
            if let Some(tint) = mv.tint.and_then(|t| tints.get_mut(t)) {
                draw(&mv.one, &mv.to, tint, (r.x, r.y));
            }
            continue;
        }
        let at = device_bounds(&mv.one, &mv.to);
        let (ax, ay) = (((at[0] - grow).floor() as i32).max(r.x), ((at[1] - grow).floor() as i32).max(r.y));
        let (bx, by) = (
            ((at[2] + grow).ceil() as i32).min(r.x + r.w as i32),
            ((at[3] + grow).ceil() as i32).min(r.y + r.h as i32),
        );
        if bx <= ax || by <= ay {
            continue;
        }
        let sub = Region { x: ax, y: ay, w: (bx - ax) as usize, h: (by - ay) as usize };
        let mut own = Mask::new(sub.w, sub.h);
        draw(&mv.one, &mv.to, &mut own, (sub.x, sub.y));
        faint.push((sub, Coat::of(own, &finish), mv.opacity, mv.tint));
    }
    let coat = Coat::of(fill, &finish);
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
    let edge = text
        .background
        .as_ref()
        .and_then(|bg| bg.border.as_ref())
        .zip(behind.as_ref())
        .map(|(st, all)| (stroke(all, (st.width * s).max(1.0), StrokeKind::Inner), premultiply_srgb(st.color.0)));

    let fill_c = premultiply_srgb(text.color.0);
    // a line's ground runs halfway to its neighbours
    let whole = laid(text, family);
    let between: Vec<f32> =
        whole.lines.windows(2).map(|w| (w[0].baseline + w[0].descent + w[1].baseline - w[1].ascent) / 2.0).collect();
    // the fill's colour where the text is flat: through several colours at an angle, or from the
    // title's colour at the block's top to another at its bottom
    let (ink_x0, ink_x1) =
        whole.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
    let (ink_x0, ink_x1) = if ink_x0 <= ink_x1 { (ink_x0, ink_x1) } else { (b[0], b[2]) };
    let run = text.fill.as_ref().filter(|f| f.colors.len() >= 2).map(|f| {
        let stops: Vec<[f32; 4]> = f.colors.iter().map(|c| premultiply_srgb(c.0)).collect();
        let (sin, cos) = f.angle.to_radians().sin_cos();
        // from the top of a line's em box to its bottom, where the ink is: a line's own box is
        // taller, and the first and last colours would fall where there is none
        let (rise, sink) = (text.size * 0.88, text.size * 0.12);
        let boxes: Vec<[f32; 4]> = if f.lines {
            whole.lines.iter().map(|ln| [ln.x, ln.baseline - rise, ln.x + ln.width, ln.baseline + sink]).collect()
        } else {
            let top = whole.lines.first().map_or(b[1], |ln| ln.baseline - rise);
            let bottom = whole.lines.last().map_or(b[3], |ln| ln.baseline + sink);
            vec![[ink_x0, top, ink_x1, bottom]]
        };
        let (bands, repeat, between) = (f.bands, f.repeat.max(0.1), between.clone());
        move |tx: f32, ty: f32| -> [f32; 4] {
            let k = if boxes.len() > 1 { between.partition_point(|edge| *edge <= ty).min(boxes.len() - 1) } else { 0 };
            let Some(bx) = boxes.get(k) else { return stops[0] };
            let (w, h) = ((bx[2] - bx[0]).max(1e-3), (bx[3] - bx[1]).max(1e-3));
            let along = (tx - (bx[0] + bx[2]) / 2.0) * cos + (ty - (bx[1] + bx[3]) / 2.0) * sin;
            let t = (along / (w * cos.abs() + h * sin.abs()).max(1e-3) + 0.5).clamp(0.0, 1.0);
            let n = stops.len();
            // once along the way from the first colour to the last; more than once, round and round
            let (at, last) = if repeat == 1.0 {
                (t * if bands { n as f32 } else { (n - 1) as f32 }, n - 1)
            } else {
                ((t * repeat).fract() * n as f32, n)
            };
            let i = (at.floor() as usize).min(n - 1);
            if bands || i >= last {
                return if bands { stops[i] } else { mix(stops[i], stops[(i + 1) % n], at - i as f32) };
            }
            mix(stops[i], stops[i + 1], at - i as f32)
        }
    });
    let fade = text.gradient.as_ref().map(|g| {
        let bottom = premultiply_srgb(g.0);
        let (top_y, height) = (b[1], (b[3] - b[1]).max(1e-3));
        move |ty: f32| -> [f32; 4] { mix(fill_c, bottom, ((ty - top_y) / height).clamp(0.0, 1.0)) }
    });
    // what is being said takes another colour: by where a target pixel is in the text
    let said = text.highlight.as_ref().and_then(|h| {
        let spans = said(text, &whole, h.unit, h.progress.sample(reelforge_time::Tick::ZERO));
        if spans.is_empty() {
            return None;
        }
        let between = between.clone();
        let color = premultiply_srgb(h.color.0);
        Some(move |tx: f32, ty: f32| -> ([f32; 4], f32) {
            let line = between.partition_point(|edge| *edge <= ty);
            let edge = |d: f32| (d * s + 0.5).clamp(0.0, 1.0);
            let cover = spans
                .iter()
                .filter(|(at, ..)| *at == line)
                .map(|(_, from, to)| edge(tx - from).min(edge(to - tx)))
                .fold(0.0, f32::max);
            (color, cover)
        })
    });
    let turn_c: Vec<[f32; 4]> =
        text.palette.as_ref().map_or(vec![], |pal| pal.colors.iter().map(|c| premultiply_srgb(c.0)).collect());
    let depth_c = text.depth.as_ref().map(|d| {
        let near = premultiply_srgb(d.color.0);
        (near, d.fade.as_ref().map_or(near, |f| premultiply_srgb(f.0)))
    });
    let bevel_c = text.bevel.as_ref().map(|bv| (premultiply_srgb(bv.light.0), premultiply_srgb(bv.dark.0)));
    let shade_c: Vec<[f32; 4]> =
        text.shadow.iter().chain(&text.shadows).map(|sh| premultiply_srgb(sh.color.0)).collect();
    let outer_c = text.outer.as_ref().map(|st| premultiply_srgb(st.color.0));
    let stroke_c = text.stroke.as_ref().map(|st| premultiply_srgb(st.color.0));
    let back_c = text.background.as_ref().map(|bg| premultiply_srgb(bg.color.0));
    let over = |top: [f32; 4], cov: f32, under: [f32; 4]| -> [f32; 4] {
        let a = top[3] * cov;
        std::array::from_fn(|i| top[i] * cov + under[i] * (1.0 - a))
    };
    // a coat's shadows, depth, outlines and glyphs at its pixel `i`, target pixel `(x, y)`, laid
    // over `px`; `own` is the colour a coat of one turn takes, `main` whether it is the whole
    // text's coat, whose turns are weighed by `tints`
    let ink = |coat: &Coat, i: usize, x: i32, y: i32, mut px: [f32; 4], own: Option<usize>, main: bool| -> [f32; 4] {
        for (mk, c) in coat.shades.iter().zip(&shade_c).rev() {
            px = over(*c, mk.a[i].clamp(0.0, 1.0), px);
        }
        if let (Some((mk, far)), Some((near, end))) = (&coat.depth, depth_c)
            && mk.a[i] > 0.0
        {
            px = over(mix(near, end, far[i]), mk.a[i].clamp(0.0, 1.0), px);
        }
        if let (Some(mk), Some(c)) = (&coat.outer, outer_c) {
            px = over(c, mk.a[i].clamp(0.0, 1.0), px);
        }
        if let (Some(mk), Some(c)) = (&coat.outline, stroke_c) {
            px = over(c, mk.a[i].clamp(0.0, 1.0), px);
        }
        let cover = coat.fill.a[i].clamp(0.0, 1.0);
        if cover <= 0.0 {
            return px;
        }
        let (tx, ty) = flat(x as f32 + 0.5, y as f32 + 0.5);
        let mut c = match (&run, &fade) {
            (Some(run), _) => run(tx, ty),
            (None, Some(fade)) => fade(ty),
            _ => fill_c,
        };
        // a colour in turn, over whatever else fills it
        if let Some(turn) = own.and_then(|t| turn_c.get(t)) {
            c = *turn;
        } else if main && !tints.is_empty() {
            let (mut sum, mut weight) = ([0.0f32; 4], 0.0f32);
            for (tint, turn) in tints.iter().zip(&turn_c) {
                let w = tint.a[i].clamp(0.0, 1.0);
                weight += w;
                for k in 0..4 {
                    sum[k] += turn[k] * w;
                }
            }
            if weight > 0.0 {
                c = sum.map(|v| v / weight);
            }
        }
        if let Some(said) = &said {
            let (color, cover) = said(tx, ty);
            c = mix(c, color, cover);
        }
        if let (Some((light, dark)), Some((lit, shaded))) = (bevel_c, &coat.bevel) {
            c = over(light, lit.a[i].clamp(0.0, 1.0), c);
            c = over(dark, shaded.a[i].clamp(0.0, 1.0), c);
        }
        over(c, cover, px)
    };
    let mut image = Image::filled(r.w as u32, r.h as u32, [0.0; 4]);
    for (i, px) in image.pixels.iter_mut().enumerate() {
        if let (Some(mk), Some(c)) = (&behind, back_c) {
            *px = over(c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        if let Some((mk, c)) = &edge {
            *px = over(*c, mk.a[i].clamp(0.0, 1.0), *px);
        }
        *px = ink(&coat, i, r.x + (i % r.w) as i32, r.y + (i / r.w) as i32, *px, None, true);
    }
    for (sub, coat, opacity, tint) in &faint {
        for i in 0..sub.w * sub.h {
            let (x, y) = (sub.x + (i % sub.w) as i32, sub.y + (i / sub.w) as i32);
            let top = ink(coat, i, x, y, [0.0; 4], *tint, false);
            if top[3] > 0.0 {
                let o = (y - r.y) as usize * r.w + (x - r.x) as usize;
                image.pixels[o] = over(top, *opacity, image.pixels[o]);
            }
        }
    }
    // the pen's line over everything, in the outline's colour or the fill's
    let pen_c = stroke_c.unwrap_or(fill_c);
    for (lines, shows) in &pens {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in lines.iter().flatten() {
            (lo, hi) = ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]);
        }
        let reach = pen_half + 1.0;
        let (ax, ay) = (((lo[0] - reach).floor() as i32).max(r.x), ((lo[1] - reach).floor() as i32).max(r.y));
        let (bx, by) = (
            ((hi[0] + reach).ceil() as i32).min(r.x + r.w as i32),
            ((hi[1] + reach).ceil() as i32).min(r.y + r.h as i32),
        );
        if bx <= ax || by <= ay {
            continue;
        }
        let (w, h) = ((bx - ax) as usize, (by - ay) as usize);
        let line = crate::shape::inked(lines, [ax, ay], [w, h], pen_half);
        for (i, cover) in line.a.iter().enumerate().filter(|(_, c)| **c > 0.0) {
            let o = (ay - r.y) as usize * r.w + (ax - r.x) as usize + (i / w) * r.w + i % w;
            image.pixels[o] = over(pen_c, cover * shows, image.pixels[o]);
        }
    }
    Some(Sprite { x: r.x, y: r.y, image: Arc::new(image), map: None, smear: Vec::new() })
}

/// The byte each character of `text` starts at, white space left out.
fn starts(text: &str) -> Vec<usize> {
    text.char_indices().filter(|(_, c)| !c.is_whitespace()).map(|(i, _)| i).collect()
}

/// The line of `l` the character at byte `at` is on, and its left and right edges there.
fn span(l: &Layout, at: usize) -> Option<(usize, f32, f32)> {
    let (k, ln) = l.lines.iter().enumerate().find(|(_, ln)| ln.range.contains(&at))?;
    let here = ln.carets.iter().find(|(b, _)| *b == at)?.1;
    let next = ln.carets.iter().filter(|(b, _)| *b > at).min_by_key(|(b, _)| *b)?.1;
    Some((k, here.min(next), here.max(next)))
}

/// Which of its units each character of the text is in (white space left out), counted from 0.
fn units(text: &str, l: &Layout, starts: &[usize], unit: TextUnit) -> Vec<usize> {
    match unit {
        TextUnit::Char => (0..starts.len()).collect(),
        TextUnit::Word => reelforge_renderspec::words(text),
        // the lines that have characters, in order
        TextUnit::Line => {
            let mut out: Vec<usize> = Vec::with_capacity(starts.len());
            let mut on: Option<usize> = None;
            for at in starts {
                let line = l.lines.iter().position(|ln| ln.range.contains(at)).or(on).unwrap_or(0);
                let before = out.last().copied();
                out.push(match (before, on) {
                    (Some(u), Some(was)) if was != line => u + 1,
                    (Some(u), _) => u,
                    (None, _) => 0,
                });
                on = Some(line);
            }
            out
        }
    }
}

/// What of the text a highlight at `progress` colours: for each stretch its line and the edges
/// it runs between, in the text's own space.
fn said(text: &TextContent, l: &Layout, unit: TextUnit, progress: f32) -> Vec<(usize, f32, f32)> {
    let starts = starts(&text.text);
    let n = starts.len();
    if n == 0 || progress.is_nan() {
        return vec![];
    }
    let at = progress.clamp(0.0, 1.0) * n as f32;
    let i = (at.floor() as usize).min(n - 1);
    if unit == TextUnit::Char {
        // everything so far: the lines above whole, this one up to where the voice is
        if progress <= 0.0 {
            return vec![];
        }
        let Some((line, x0, x1)) = span(l, starts[i]) else { return vec![] };
        let mut out: Vec<(usize, f32, f32)> = (0..line).map(|k| (k, f32::MIN, f32::MAX)).collect();
        out.push((line, f32::MIN, x0 + (x1 - x0) * (at - i as f32).min(1.0)));
        return out;
    }
    // the word or line the voice is in, alone
    if !(0.0..1.0).contains(&progress) {
        return vec![];
    }
    let of = units(&text.text, l, &starts, unit);
    let mut out: Vec<(usize, f32, f32)> = vec![];
    for (c, _) in of.iter().enumerate().filter(|(_, u)| **u == of[i]) {
        let Some((line, x0, x1)) = span(l, starts[c]) else { continue };
        match out.iter_mut().find(|(k, ..)| *k == line) {
            Some(s) => (s.1, s.2) = (s.1.min(x0), s.2.max(x1)),
            None => out.push((line, x0, x1)),
        }
    }
    if unit == TextUnit::Line {
        for s in &mut out {
            (s.1, s.2) = (f32::MIN, f32::MAX);
        }
    }
    out
}

/// Colour `a` on its way to `b`: all `a` at 0, all `b` at 1.
fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// What is done to a coat of ink besides filling it, in target pixels.
struct Finish {
    stroke_px: f32,
    outer_px: f32,
    /// Each shadow's offset and softness, the top one first.
    shadows: Vec<(i32, i32, f32)>,
    /// How far the glyphs are drawn out into a block.
    depth: Option<(i32, i32)>,
    /// The width of the line inked just inside a hollow glyph's edge.
    hollow_px: Option<f32>,
    /// Lit and shaded edges: the step towards the light, and how wide they are.
    bevel: Option<(i32, i32, f32)>,
}

impl Finish {
    /// How far beyond the glyphs and their outlines anything of it lands.
    fn reach(&self) -> f32 {
        let depth = self.depth.unwrap_or((0, 0));
        self.shadows
            .iter()
            .map(|(dx, dy, sigma)| (dx.abs() + depth.0.abs()).max(dy.abs() + depth.1.abs()) as f32 + 3.0 * sigma)
            .fold(depth.0.abs().max(depth.1.abs()) as f32, f32::max)
    }
}

/// One coat of ink as coverage: the glyphs, their outlines, the block they are drawn out into and
/// their shadows.
struct Coat {
    fill: Mask,
    outline: Option<Mask>,
    outer: Option<Mask>,
    /// The block, and for each of its pixels how far along it that pixel is, 0 to 1.
    depth: Option<(Mask, Vec<f32>)>,
    /// The shadows, the top one first.
    shades: Vec<Mask>,
    /// The lit and the shaded edges inside the glyphs.
    bevel: Option<(Mask, Mask)>,
}

impl Coat {
    /// What `finish` makes of the glyphs in `fill`.
    fn of(fill: Mask, finish: &Finish) -> Coat {
        // hollow glyphs are their own edges, and everything else is of those
        let fill = match finish.hollow_px {
            Some(w) => stroke(&fill, w, StrokeKind::Inner),
            None => fill,
        };
        let (stroke_px, outer_px) = (finish.stroke_px, finish.outer_px);
        let outline = (stroke_px > 0.0).then(|| stroke(&fill, stroke_px, StrokeKind::Outer));
        let outer = (outer_px > 0.0).then(|| stroke(&fill, stroke_px + outer_px, StrokeKind::Outer));
        let mut shape = fill.clone();
        for m in [&outline, &outer].into_iter().flatten() {
            shape.max_with(m);
        }
        let depth = finish.depth.filter(|d| *d != (0, 0)).map(|(dx, dy)| {
            let n = dx.abs().max(dy.abs());
            let (w, h) = (shape.w as i32, shape.h as i32);
            let mut block = Mask::new(shape.w, shape.h);
            let mut far = vec![0.0f32; shape.w * shape.h];
            for k in 1..=n {
                let (ox, oy) = ((dx * k + n / 2 * dx.signum()) / n, (dy * k + n / 2 * dy.signum()) / n);
                for y in oy.max(0)..(h + oy).min(h) {
                    for x in ox.max(0)..(w + ox).min(w) {
                        let v = shape.a[((y - oy) * w + x - ox) as usize];
                        let i = (y * w + x) as usize;
                        if v > block.a[i] {
                            block.a[i] = v;
                            far[i] = k as f32 / n as f32;
                        }
                    }
                }
            }
            (block, far)
        });
        if let Some((block, _)) = &depth {
            shape.max_with(block);
        }
        let shades = finish.shadows.iter().map(|(dx, dy, sigma)| blur(&offset(&shape, *dx, *dy), *sigma)).collect();
        let bevel = finish.bevel.map(|(dx, dy, reach)| {
            // an edge is where the glyph is and, a step to one side, is not
            let edge = |dx: i32, dy: i32| {
                let beside = offset(&fill, -dx, -dy);
                let mut m = Mask::new(fill.w, fill.h);
                for (o, (a, b)) in m.a.iter_mut().zip(fill.a.iter().zip(&beside.a)) {
                    *o = (a - b).max(0.0);
                }
                let mut soft = blur(&m, reach / 3.0);
                for (o, a) in soft.a.iter_mut().zip(&fill.a) {
                    *o = (*o * 1.6).min(1.0) * a;
                }
                soft
            };
            (edge(dx, dy), edge(-dx, -dy))
        });
        Coat { fill, outline, outer, depth, shades, bevel }
    }
}

/// The circle `text`'s lines are bent round, when they are: the middle of the block across, the
/// height that keeps its place, and the radius there (negative when the lines hang down).
fn bend(text: &TextContent, l: &Layout) -> Option<(f32, f32, f32)> {
    let (x0, x1) = l.lines.iter().fold((f32::MAX, f32::MIN), |(a, z), ln| (a.min(ln.x), z.max(ln.x + ln.width)));
    let turn = text.arc.to_radians();
    if turn.abs() < 1e-4 || x1 - x0 <= 0.0 {
        return None;
    }
    let b = l.bounds;
    Some(((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, (x1 - x0) / turn))
}

/// Which colour of the palette each character takes (white space left out), before the colours
/// go round: the number of its character, word or line.
fn tints_of(text: &TextContent, l: &Layout) -> Option<Vec<usize>> {
    let pal = text.palette.as_ref()?;
    Some(units(&text.text, l, &starts(&text.text), pal.unit))
}

/// A number from 0 to 1 that depends only on what it is given.
fn chance(seed: u32, i: usize, salt: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B1) ^ (i as u32).wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Characters somewhere else than the layout puts them.
struct Mover {
    /// Their glyphs, as a layout of their own.
    one: Layout,
    /// Takes them there: in the text's own space as [`moved`] returns it, then on the target.
    to: Xform,
    /// How much of them shows; above 0.
    opacity: f32,
    /// The colour of the palette they take, when the text has one.
    tint: Option<usize>,
}

/// Takes the characters that `text.chars` moves out of `l`, and those an arc or a scatter stands
/// somewhere else: those on their way or out of line are returned (together where they go the
/// same way, as the characters of a word do), those that do not show at all are dropped, and
/// those at rest where the layout put them stay.
fn moved(text: &TextContent, l: &mut Layout) -> Vec<Mover> {
    // characters are counted without white space
    let starts = starts(&text.text);
    let n = starts.len();
    let index = |g: &Glyph| starts.partition_point(|s| *s <= g.cluster).checked_sub(1);
    // each character's box (between its carets, about its line) and the size it is set in
    let mut boxes: Vec<Option<[f32; 4]>> = vec![None; n];
    let mut sizes = vec![text.size; n];
    for g in &l.glyphs {
        let Some(i) = index(g).filter(|i| boxes[*i].is_none()) else { continue };
        let (x0, x1) = span(l, g.cluster).map_or((g.x, g.x + g.size * 0.6), |(_, a, b)| (a, b));
        boxes[i] = Some([x0, g.y - g.size, x1, g.y + g.size * 0.3]);
        sizes[i] = g.size;
    }
    // where each character goes: what moves its line, then its word, then itself
    let mut to: Vec<Xform> = vec![IDENTITY; n];
    let mut shown = vec![1.0f32; n];
    for unit in [TextUnit::Char, TextUnit::Word, TextUnit::Line] {
        let motions: Vec<_> = text.chars.iter().filter(|m| m.unit == unit).collect();
        if motions.is_empty() {
            continue;
        }
        let of = units(&text.text, l, &starts, unit);
        let count = of.iter().max().map_or(0, |m| m + 1);
        // a unit's centre: the middle of its characters, a little above their baselines
        let mut span = vec![(f32::MAX, f32::MIN, 0.0f32, 0.0f32); count];
        for (i, b) in boxes.iter().enumerate() {
            if let Some(b) = b {
                let s = &mut span[of[i]];
                *s = (s.0.min(b[0]), s.1.max(b[2]), s.2 + b[1] + sizes[i] * 0.64, s.3 + 1.0);
            }
        }
        for i in 0..n {
            let (u, s) = (of[i], span[of[i]]);
            let pose = motions
                .iter()
                .fold(CharPose::REST, |p, m| p.and(m.at(u, count, m.progress.sample(reelforge_time::Tick::ZERO))));
            if pose == CharPose::REST || s.3 == 0.0 {
                continue;
            }
            let (cx, cy) = ((s.0 + s.1) / 2.0, s.2 / s.3);
            let (sin, cos) = pose.rotation.to_radians().sin_cos();
            let (a, b) = (pose.scale * cos, pose.scale * sin);
            let about: Xform = [
                a,
                b,
                -b,
                a,
                cx + pose.offset[0] * sizes[i] - (a * cx - b * cy),
                cy + pose.offset[1] * sizes[i] - (b * cx + a * cy),
            ];
            to[i] = if to[i] == IDENTITY { about } else { compose(&about, &to[i]) };
            shown[i] *= pose.opacity.clamp(0.0, 1.0);
        }
    }
    // where each stands when nothing moves it: set out of line by chance, then bent round the arc
    let round = bend(text, l);
    for i in 0..n {
        let Some(bx) = boxes[i] else { continue };
        let (cx, base) = ((bx[0] + bx[2]) / 2.0, bx[1] + sizes[i]);
        let mut stands = IDENTITY;
        if let Some(sc) = &text.scatter {
            let by = |salt: u32| chance(sc.seed, i, salt) * 2.0 - 1.0;
            let (sin, cos) = (sc.rotate * by(1)).to_radians().sin_cos();
            let k = (1.0 + sc.scale * by(3)).max(0.05);
            let (a, b, cy) = (k * cos, k * sin, base - sizes[i] * 0.36);
            stands = [a, b, -b, a, cx - (a * cx - b * cy), cy + sc.shift * sizes[i] * by(2) - (b * cx + a * cy)];
        }
        if let Some((mx, yref, r)) = round {
            let (sin, cos) = ((cx - mx) / r).sin_cos();
            let rho = r - (base - yref);
            let (px, py) = (mx + rho * sin, yref + r - rho * cos);
            let on: Xform = [cos, sin, -sin, cos, px - (cos * cx - sin * base), py - (sin * cx + cos * base)];
            stands = if stands == IDENTITY { on } else { compose(&on, &stands) };
        }
        if stands != IDENTITY {
            to[i] = if to[i] == IDENTITY { stands } else { compose(&stands, &to[i]) };
        }
    }
    let tints = tints_of(text, l);
    let turns = text.palette.as_ref().map_or(1, |pal| pal.colors.len().max(1));
    let tint = |i: usize| tints.as_ref().and_then(|t| t.get(i)).map(|t| t % turns);
    let rests = |i: usize| to[i] == IDENTITY && shown[i] >= 1.0;
    let mut out: Vec<Mover> = vec![];
    for g in &l.glyphs {
        let Some(i) = index(g).filter(|i| !rests(*i) && shown[*i] > 0.0) else { continue };
        let b = boxes[i].unwrap_or([g.x, g.y - g.size, g.x + g.size * 0.6, g.y + g.size * 0.3]);
        match out.iter_mut().find(|mv| mv.to == to[i] && mv.opacity == shown[i] && mv.tint == tint(i)) {
            Some(mv) => {
                mv.one.glyphs.push(*g);
                let o = mv.one.bounds;
                mv.one.bounds = [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])];
            }
            None => {
                let one = Layout { glyphs: vec![*g], bounds: b, ..Layout::default() };
                out.push(Mover { one, to: to[i], opacity: shown[i], tint: tint(i) });
            }
        }
    }
    l.glyphs.retain(|g| index(g).is_none_or(rests));
    out
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
