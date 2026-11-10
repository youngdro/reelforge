//! Layouts: several picture clips sharing the frame (`layout.list`, `layout.apply`).
//!
//! A layout gives each clip a cell of the frame. A clip fills its cell by being cropped to the
//! cell's shape, scaled to its size and put at its middle: all things a clip already has, so a
//! laid-out clip is an ordinary clip that can be moved, cropped again or keyed afterwards. A
//! small picture can be a circle (a mask), a comparison wipes the upper picture off along a line
//! that travels (a keyed mask), and the outlines and the divider a layout draws are shape clips
//! carried by the clips they belong to.

use reelforge_model::{
    Clip, ClipContent, ClipTransform, Crop, Ease, Key, Mask, MaskKind, Project, Prop, Shape, ShapeKind, ShapeShadow,
    Stroke, TrackKind,
};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{fps, not_found};
use crate::params::P;
use crate::promo::{free_track, plain_clip};
use crate::{CmdResult, EngineError, Session};

/// A way for clips to share the frame.
pub struct Layout {
    /// Its id.
    pub id: &'static str,
    /// What it is, in English.
    pub en: &'static str,
    /// In Chinese.
    pub zh: &'static str,
    /// The fewest and the most clips it takes.
    pub clips: (usize, usize),
    /// What it does with them, in English.
    pub about: &'static str,
}

/// The layouts `layout.apply` knows.
pub const LAYOUTS: [Layout; 6] = [
    Layout {
        id: "columns",
        en: "Side by side",
        zh: "左右分屏",
        clips: (2, 4),
        about: "Equal columns, the first clip on the left.",
    },
    Layout {
        id: "rows", en: "Stacked", zh: "上下分屏", clips: (2, 4), about: "Equal rows, the first clip on top."
    },
    Layout {
        id: "grid",
        en: "Grid",
        zh: "宫格",
        clips: (3, 6),
        about: "Two to a row on an upright frame, two to a column on a wide one; with an odd number the first clip takes a whole row or column.",
    },
    Layout {
        id: "focus",
        en: "One large",
        zh: "一大多小",
        clips: (2, 4),
        about: "The first clip takes two thirds of the frame (the top of an upright one, the left of a wide one); the rest share the strip beside it.",
    },
    Layout {
        id: "pip",
        en: "Picture in picture",
        zh: "画中画",
        clips: (2, 4),
        about: "The first clip fills the frame; the others are small pictures in a `corner`, `size` of the frame's width each (circles with `round`, `size` of its shorter side across), outlined unless `border` is null. They must be on tracks above the first.",
    },
    Layout {
        id: "compare",
        en: "Compare slider",
        zh: "对比滑杆",
        clips: (2, 2),
        about: "Both clips fill the frame, the first under the second; a divider travels `from` one place `to` another (0 to 1 across the frame) and the second shows only past it: before and after. `axis` \"y\" makes it travel down; `divider: false` leaves the line out. The second must be on a track above the first.",
    },
];

/// The corners a small picture can sit in.
pub(crate) const CORNERS: [&str; 4] = ["topLeft", "topRight", "bottomLeft", "bottomRight"];

fn wrong(name: &str, what: &str) -> EngineError {
    EngineError::Params(format!("`{name}` must be {what}"))
}

/// A part of the frame: where its middle is from the frame's, and how large it is, as shares of
/// the frame's width and height.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Cell {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// What a layout is asked for besides its clips.
struct Ask {
    /// The space between cells and round the lot, as shares of the frame's width and height.
    gap: [f32; 2],
    margin: [f32; 2],
    /// Which corner small pictures sit in: right, then bottom.
    corner: (bool, bool),
    /// A small picture's width, as a share of the frame's.
    size: f32,
    round: bool,
}

/// `n` cells in a line across `across` of the frame, `gap` between them: each one's middle and
/// length.
fn line(n: usize, across: f32, gap: f32) -> Vec<(f32, f32)> {
    let each = (across - gap * (n as f32 - 1.0)) / n as f32;
    (0..n).map(|i| (-across / 2.0 + each / 2.0 + i as f32 * (each + gap), each)).collect()
}

/// The cells of `layout` for `n` clips on a frame `w` by `h`.
fn cells(layout: &str, n: usize, [w, h]: [f32; 2], ask: &Ask) -> Vec<Cell> {
    let (aw, ah) = (1.0 - 2.0 * ask.margin[0], 1.0 - 2.0 * ask.margin[1]);
    let [gx, gy] = ask.gap;
    let upright = h >= w;
    let whole = Cell { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    match layout {
        "columns" => line(n, aw, gx).into_iter().map(|(x, w)| Cell { x, y: 0.0, w, h: ah }).collect(),
        "rows" => line(n, ah, gy).into_iter().map(|(y, h)| Cell { x: 0.0, y, w: aw, h }).collect(),
        "grid" => {
            // lines of two along the frame's longer side; the first line is one clip when they
            // do not come out even
            let lines = n.div_ceil(2);
            let mut out = vec![];
            let (long, short, gl, gs) = if upright { (ah, aw, gy, gx) } else { (aw, ah, gx, gy) };
            let mut left = n;
            for (i, (at, len)) in line(lines, long, gl).into_iter().enumerate() {
                let here = if i == 0 && n % 2 == 1 { 1 } else { 2 };
                for (across, wide) in line(here.min(left), short, gs) {
                    out.push(if upright {
                        Cell { x: across, y: at, w: wide, h: len }
                    } else {
                        Cell { x: at, y: across, w: len, h: wide }
                    });
                }
                left -= here.min(left);
            }
            out
        }
        "focus" => {
            let (long, short, gl, gs) = if upright { (ah, aw, gy, gx) } else { (aw, ah, gx, gy) };
            let large = (long - gl) * 2.0 / 3.0;
            let small = long - gl - large;
            let (first, strip) = (-long / 2.0 + large / 2.0, long / 2.0 - small / 2.0);
            let mut out = vec![if upright {
                Cell { x: 0.0, y: first, w: short, h: large }
            } else {
                Cell { x: first, y: 0.0, w: large, h: short }
            }];
            for (across, wide) in line(n - 1, short, gs) {
                out.push(if upright {
                    Cell { x: across, y: strip, w: wide, h: small }
                } else {
                    Cell { x: strip, y: across, w: small, h: wide }
                });
            }
            out
        }
        "pip" => {
            // a small picture has the frame's own shape; a circle is as tall as it is wide, and
            // measured on the frame's shorter side
            let short = w.min(h);
            let (mut sw, mut sh) =
                if ask.round { (ask.size * short / w, ask.size * short / h) } else { (ask.size, ask.size) };
            // several are made smaller rather than run off the frame
            let many = (n - 1) as f32;
            let room = 1.0 - 2.0 * ask.margin[1] - gy * (many - 1.0);
            if sh * many > room {
                (sw, sh) = (sw * room / (sh * many), room / many);
            }
            let (right, bottom) = ask.corner;
            let x = (0.5 - ask.margin[0] - sw / 2.0) * if right { 1.0 } else { -1.0 };
            let mut out = vec![whole];
            for i in 0..n - 1 {
                // one after another along the frame's side, away from the corner
                let y = 0.5 - ask.margin[1] - sh / 2.0 - i as f32 * (sh + gy);
                out.push(Cell { x, y: if bottom { y } else { -y }, w: sw, h: sh });
            }
            out
        }
        _ => vec![whole; n],
    }
}

/// The size of the picture `clip` shows, in pixels.
fn picture_size(proj: &Project, clip: &Clip) -> Result<[f32; 2], EngineError> {
    let (cw, ch) = (proj.canvas.width as f32, proj.canvas.height as f32);
    match &clip.content {
        ClipContent::Media { media, .. } => match proj.media.get(media).and_then(|m| m.size) {
            Some([w, h]) if w > 0 && h > 0 => Ok([w as f32, h as f32]),
            _ => Err(EngineError::Params(format!("clip {:?} has no picture", clip.id))),
        },
        // a compound clip's picture is a frame of the project
        _ => Ok([cw, ch]),
    }
}

/// Makes `clip`, whose picture is `size`, fill `cell` of a frame `w` by `h`: cropped to the
/// cell's shape about its own middle, scaled and placed. Its own moves, keys of them, path and
/// carrier go, since they would take it out of its cell. Returns its scale.
fn fill(clip: &mut Clip, size: [f32; 2], cell: Cell, [w, h]: [f32; 2]) -> f32 {
    let wanted = (cell.w * w) / (cell.h * h);
    let own = size[0] / size[1];
    let mut crop = Crop::default();
    if own > wanted {
        let cut = (1.0 - wanted / own).clamp(0.0, 0.98);
        (crop.left, crop.right) = (cut / 2.0, cut / 2.0);
    } else {
        let cut = (1.0 - own / wanted).clamp(0.0, 0.98);
        (crop.top, crop.bottom) = (cut / 2.0, cut / 2.0);
    }
    clip.look.crop = crop;
    clip.look.mask = None;
    // what shows is fitted inside the frame before it is scaled: a cell wider than the frame's
    // shape is then as wide as the frame, any other as tall
    let scale = if wanted >= w / h { cell.w } else { cell.h };
    clip.transform =
        ClipTransform { x: cell.x, y: cell.y, scale, motion_blur: clip.transform.motion_blur, ..Default::default() };
    let moved =
        [Prop::X, Prop::Y, Prop::Scale, Prop::Rotation, Prop::ScaleX, Prop::ScaleY, Prop::RotationX, Prop::RotationY];
    clip.keys.retain(|p, _| !moved.contains(p) && !Prop::CROP.contains(p) && !Prop::MASK.contains(p));
    clip.path = None;
    clip.parent = None;
    scale
}

/// The shapes `clips` carry for their whole length, directly or through one another: what a
/// layout drew for them before.
fn drawn(proj: &Project, clips: &[String]) -> Vec<String> {
    let mut found: Vec<(String, Tick, Tick)> = vec![];
    for id in clips {
        if let Some((_, c)) = proj.clip(id) {
            found.push((id.clone(), c.start, c.duration));
        }
    }
    let first = found.len();
    loop {
        let more: Vec<(String, Tick, Tick)> = proj
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| matches!(c.content, ClipContent::Shape { .. }))
            .filter(|c| found.iter().all(|(id, ..)| *id != c.id))
            .filter(|c| {
                let by = c.parent.as_deref();
                found
                    .iter()
                    .any(|(id, start, long)| by == Some(id.as_str()) && c.start == *start && c.duration == *long)
            })
            .map(|c| (c.id.clone(), c.start, c.duration))
            .collect();
        if more.is_empty() {
            break;
        }
        found.extend(more);
    }
    found.split_off(first).into_iter().map(|(id, ..)| id).collect()
}

/// Puts `shape` on a free track above `over`, for as long as `carrier` lasts and carried by it:
/// `at` is where its middle is from the carrier's, as shares of the frame, and `turn` its
/// rotation. Returns its id.
fn carried(proj: &mut Project, carrier: &str, over: usize, shape: Shape, at: [f32; 2], turn: f32) -> (String, usize) {
    let (_, by) = proj.clip(carrier).expect("a clip of the layout");
    let (start, long) = (by.start, by.duration);
    // how large the carrier is with all that carries it in turn
    let mut scale = by.transform.scale;
    let mut up = by.parent.clone();
    for _ in 0..8 {
        let Some((_, next)) = up.as_deref().and_then(|id| proj.clip(id)) else { break };
        scale *= next.transform.scale;
        up = next.parent.clone();
    }
    let ti = free_track(proj, TrackKind::Video, over + 1, start, long);
    let id = proj.mint("c");
    let mut clip = plain_clip(id.clone(), start, long, ClipContent::Shape { shape });
    // counted along the carrier's own scaled axes, and as large as drawn whatever its scale
    let by = 1.0 / scale.max(1e-6);
    clip.transform = ClipTransform { x: at[0] * by, y: at[1] * by, scale: by, rotation: turn, ..Default::default() };
    clip.parent = Some(carrier.to_string());
    proj.tracks[ti].clips.push(clip);
    proj.tracks[ti].clips.sort_by_key(|c| c.start);
    (id, ti)
}

/// `layout.list`.
pub(crate) fn layout_list(_: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({
        "layouts": LAYOUTS
            .iter()
            .map(|l| json!({"id": l.id, "name": l.en, "zh": l.zh, "clips": [l.clips.0, l.clips.1], "about": l.about}))
            .collect::<Vec<_>>()
    }))
}

/// `layout.apply`.
pub(crate) fn layout_apply(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let name = p.str("layout")?;
    let layout = LAYOUTS
        .iter()
        .find(|l| l.id == name)
        .ok_or_else(|| wrong("layout", &format!("one of {}", LAYOUTS.map(|l| l.id).join(", "))))?;
    let clips = p.strings("clips")?;
    let (least, most) = layout.clips;
    if clips.len() < least || clips.len() > most {
        let takes = if least == most { format!("{least}") } else { format!("{least} to {most}") };
        return Err(EngineError::Params(format!("the {name} layout takes {takes} clips, not {}", clips.len())));
    }
    if clips.iter().enumerate().any(|(i, id)| clips[..i].contains(id)) {
        return Err(wrong("clips", "different clips"));
    }
    let share = |name: &str, most: f64, unless: f64| -> Result<f32, EngineError> {
        match p.opt_f64(name)? {
            Some(x) if !(0.0..=most).contains(&x) => Err(wrong(name, &format!("0 to {most}"))),
            x => Ok(x.unwrap_or(unless) as f32),
        }
    };
    let small = name == "pip";
    let (gap, margin) = (share("gap", 0.2, 0.012)?, share("margin", 0.3, if small { 0.04 } else { 0.0 })?);
    let size = match p.opt_f64("size")? {
        Some(x) if !(0.1..=0.6).contains(&x) => return Err(wrong("size", "0.1 to 0.6")),
        x => x.unwrap_or(0.32) as f32,
    };
    let corner = match p.opt_str("corner")? {
        None => (true, false),
        Some(c) if CORNERS.contains(&c) => (c.ends_with("Right"), c.starts_with("bottom")),
        Some(_) => return Err(wrong("corner", &format!("one of {}", CORNERS.join(", ")))),
    };
    let round = p.opt_bool("round")? == Some(true);
    // an outline: on small pictures unless told not to, on the cells of other layouts when asked
    let border = match v.get("border") {
        None if small => Some(([1.0, 1.0, 1.0, 1.0], None)),
        None | Some(Value::Null) => None,
        Some(b) if b.is_object() => {
            let known = b.as_object().is_some_and(|o| o.keys().all(|k| ["color", "width"].contains(&k.as_str())));
            let what = "{\"color\"?: [r, g, b, a], \"width\"?: pixels} or null";
            if !known {
                return Err(wrong("border", what));
            }
            let width = match b.get("width") {
                None => None,
                Some(x) => match x.as_f64().filter(|x| *x > 0.0 && *x <= 200.0) {
                    Some(x) => Some(x as f32),
                    None => return Err(wrong("border", what)),
                },
            };
            Some((P::new(b, fps(s)).opt_color("color")?.unwrap_or([1.0, 1.0, 1.0, 1.0]), width))
        }
        Some(_) => return Err(wrong("border", "{\"color\"?: [r, g, b, a], \"width\"?: pixels} or null")),
    };
    let (from, to) = (share("from", 1.0, 0.85)?, share("to", 1.0, 0.15)?);
    let down = match p.opt_str("axis")? {
        None | Some("x") => false,
        Some("y") => true,
        Some(_) => return Err(wrong("axis", "\"x\" or \"y\"")),
    };
    let divider = p.opt_bool("divider")? != Some(false);

    s.edit("Layout", |proj| {
        let (cw, ch) = (proj.canvas.width as f32, proj.canvas.height as f32);
        let short = cw.min(ch);
        // lengths drawn in pixels are for a frame 1080 on its shorter side
        let k = short / 1080.0;
        let mut on: Vec<usize> = vec![];
        for id in &clips {
            let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            if proj.tracks[ti].kind != TrackKind::Video || !clip.is_picture() {
                return Err(EngineError::Params(format!("clip {id:?} is not a picture clip on a video track")));
            }
            on.push(ti);
        }
        // one picture over another has to be on a track above it wherever the two meet
        if small || name == "compare" {
            let (_, under) = proj.clip(&clips[0]).expect("found above");
            for (id, ti) in clips.iter().zip(&on).skip(1) {
                let (_, over) = proj.clip(id).expect("found above");
                if *ti <= on[0] && over.range().overlaps(under.range()) {
                    return Err(EngineError::Refused(format!(
                        "clip {id:?} would be hidden under {:?}: put it on a track above (clip.move)",
                        clips[0]
                    )));
                }
            }
        }
        // what an earlier layout drew for these clips goes
        let old = drawn(proj, &clips);
        for track in &mut proj.tracks {
            track.clips.retain(|c| !old.contains(&c.id));
        }
        let ask = Ask {
            gap: [gap * short / cw, gap * short / ch],
            margin: [margin * short / cw, margin * short / ch],
            corner,
            size,
            round,
        };
        let placed = cells(name, clips.len(), [cw, ch], &ask);
        let mut out = vec![];
        let mut added: Vec<String> = vec![];
        for (i, (id, cell)) in clips.iter().zip(&placed).enumerate() {
            let size = picture_size(proj, proj.clip(id).expect("found above").1)?;
            let clip = proj.clip_mut(id).expect("found above");
            let scale = fill(clip, size, *cell, [cw, ch]);
            let window = small && i > 0;
            if window && round {
                // what shows is square: a circle as wide as it
                clip.look.mask = Some(Mask { size: [1.0, 1.0], feather: 0.0, ..Mask::new(MaskKind::Circle) });
            }
            let (pw, ph) = (cell.w * cw, cell.h * ch);
            if let (Some((color, width)), true) = (border, window || !small) {
                let kind = if window && round { ShapeKind::Ellipse } else { ShapeKind::Rectangle };
                let shape = Shape {
                    fill: None,
                    stroke: Some(Stroke { color, width: width.unwrap_or(6.0 * k) }),
                    ..Shape::new(kind, [pw, ph], color)
                };
                added.push(carried(proj, id, on[i], shape, [0.0, 0.0], 0.0).0);
            }
            out.push(json!({
                "clip": id, "x": cell.x, "y": cell.y, "scale": scale,
                "width": pw.round(), "height": ph.round(),
            }));
        }
        if name == "compare" {
            // the upper picture shows past a line that travels: to its right, or below it
            let upper = proj.clip_mut(&clips[1]).expect("found above");
            let long = upper.duration;
            let keys = |a: f32, b: f32| {
                vec![
                    Key { t: Tick(long.0 * 15 / 100), v: a, ease: Ease::InOut },
                    Key { t: Tick(long.0 * 85 / 100), v: b, ease: Ease::Linear },
                ]
            };
            let mut mask = Mask { feather: 0.0, ..Mask::new(MaskKind::Linear) };
            let along = if down {
                mask.rotation = 180.0;
                mask.center[1] = from;
                Prop::MaskY
            } else {
                mask.rotation = 90.0;
                mask.center[0] = from;
                Prop::MaskX
            };
            upper.look.mask = Some(mask);
            upper.keys.insert(along, keys(from, to));
            if divider {
                let white = [1.0, 1.0, 1.0, 1.0];
                let shadow = Some(ShapeShadow { color: [0.0, 0.0, 0.0, 0.35], offset: [0.0, 2.0 * k], blur: 10.0 * k });
                // a line right across the frame, a knob at its middle, and an arrow each way
                let across = if down { [cw + 40.0, 6.0 * k] } else { [6.0 * k, ch + 40.0] };
                let bar = Shape { shadow: shadow.clone(), ..Shape::new(ShapeKind::Rectangle, across, white) };
                let (line, mut over) = carried(proj, &clips[1], on[1], bar, [0.0, 0.0], 0.0);
                let moves = if down { Prop::Y } else { Prop::X };
                let travel = keys(from - 0.5, to - 0.5);
                proj.clip_mut(&line).expect("just made").keys.insert(moves, travel);
                let group = proj.mint("g");
                let knob = Shape { shadow, ..Shape::new(ShapeKind::Ellipse, [84.0 * k, 84.0 * k], white) };
                let mut parts = vec![(knob, [0.0, 0.0], 0.0)];
                for way in [-1.0f32, 1.0] {
                    let tip = Shape {
                        points: 3,
                        ..Shape::new(ShapeKind::Polygon, [24.0 * k, 24.0 * k], [0.2, 0.2, 0.24, 1.0])
                    };
                    let off = way * 20.0 * k;
                    let (at, turn) =
                        if down { ([0.0, off / ch], 90.0 + way * 90.0) } else { ([off / cw, 0.0], way * 90.0) };
                    parts.push((tip, at, turn));
                }
                proj.clip_mut(&line).expect("just made").group = Some(group.clone());
                added.push(line.clone());
                for (shape, at, turn) in parts {
                    let (id, ti) = carried(proj, &line, over, shape, at, turn);
                    over = ti;
                    proj.clip_mut(&id).expect("just made").group = Some(group.clone());
                    added.push(id);
                }
            }
        }
        Ok(json!({ "layout": name, "clips": out, "added": added }))
    })
}
