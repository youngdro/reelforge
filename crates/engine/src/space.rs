//! Clips in space: placed by another point than their middle, one carried by another
//! (`clip.setParent`), and travelling along a path (`clip.setPath`).
//!
//! A clip's `x` and `y` place its anchor, counted from the canvas centre, or from its parent's
//! anchor along the parent's axes when it has one. So giving a clip another anchor or another
//! parent would move it; the commands here change its `x` and `y` (and scale and rotation, for a
//! parent) with it, so that it stays where it shows unless told not to.

use reelforge_model::{Clip, ClipContent, Ease, MotionPath, Project, Prop, TrackKind};
use reelforge_renderspec::LayerContent;
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{ease_of, fps, not_found};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// A map of the canvas that keeps lines parallel: `x' = a x + b y + c`, `y' = d x + e y + f`.
type Flat = [f32; 6];

fn then(a: &Flat, b: &Flat) -> Flat {
    [
        a[0] * b[0] + a[1] * b[3],
        a[0] * b[1] + a[1] * b[4],
        a[0] * b[2] + a[1] * b[5] + a[2],
        a[3] * b[0] + a[4] * b[3],
        a[3] * b[1] + a[4] * b[4],
        a[3] * b[2] + a[4] * b[5] + a[5],
    ]
}

fn apply(m: &Flat, p: [f32; 2]) -> [f32; 2] {
    [m[0] * p[0] + m[1] * p[1] + m[2], m[3] * p[0] + m[4] * p[1] + m[5]]
}

fn inverse(m: &Flat) -> Option<Flat> {
    let det = m[0] * m[4] - m[1] * m[3];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, d, e) = (m[4] / det, -m[1] / det, -m[3] / det, m[0] / det);
    Some([a, b, -(a * m[2] + b * m[5]), d, e, -(d * m[2] + e * m[5])])
}

/// A clip's own placement at timeline time `at` (held at its ends), before its animations: where
/// its anchor is in canvas fractions, its scale, its stretch and its rotation.
struct Placed {
    at: [f32; 2],
    scale: f32,
    stretch: [f32; 2],
    rotation: f32,
}

fn placed(clip: &Clip, at: Tick, canvas: [f32; 2]) -> Placed {
    let t = (at - clip.start).max(Tick::ZERO).min(clip.duration);
    let on_path = clip.on_path(t, canvas[0] / canvas[1]);
    Placed {
        at: on_path.map_or([clip.value_at(Prop::X, t), clip.value_at(Prop::Y, t)], |(to, _)| to),
        scale: clip.value_at(Prop::Scale, t),
        stretch: [clip.value_at(Prop::ScaleX, t), clip.value_at(Prop::ScaleY, t)],
        rotation: clip.value_at(Prop::Rotation, t) + on_path.map_or(0.0, |(_, heading)| heading),
    }
}

/// What a clip does to what it carries, in canvas pixels: the canvas centre goes to the clip's
/// anchor, along its axes as it is turned in the plane, scaled and stretched. (Its turn in space
/// is left out: what is carried is kept where it shows while its parent lies flat.)
fn carry(p: &Placed, canvas: [f32; 2]) -> Flat {
    let [w, h] = canvas;
    let (sin, cos) = p.rotation.to_radians().sin_cos();
    let (sx, sy) = (p.scale * p.stretch[0], p.scale * p.stretch[1]);
    let (cx, cy) = (w * (0.5 + p.at[0]), h * (0.5 + p.at[1]));
    let m = [cos * sx, -sin * sy, cx, sin * sx, cos * sy, cy];
    then(&m, &[1.0, 0.0, -w / 2.0, 0.0, 1.0, -h / 2.0])
}

/// Everything that carries `clip` at `at`, as one map, with the scale and the rotation it adds up
/// to. The chain is followed as far as the renderer follows it.
fn carried(proj: &Project, parent: Option<&str>, at: Tick, canvas: [f32; 2]) -> (Flat, f32, f32) {
    let mut chain = vec![];
    let mut up = parent;
    while let Some((_, c)) = up.and_then(|id| proj.clip(id)) {
        if chain.len() == reelforge_renderspec::PARENTS {
            break;
        }
        chain.push(placed(c, at, canvas));
        up = c.parent.as_deref();
    }
    let mut out: (Flat, f32, f32) = ([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 1.0, 0.0);
    // the nearest first: what is farther up moves what is nearer
    for p in &chain {
        out = (then(&carry(p, canvas), &out.0), out.1 * p.scale, out.2 + p.rotation);
    }
    out
}

/// How a move across the canvas moves the placement of clip `id` at timeline time `at`, both in
/// canvas pixels: `[a, b, c, d]` with `x' = a x + b y` and `y' = c x + d y`. A clip that nothing
/// carries moves as far as it is moved; one that is carried is placed along its parent's axes,
/// as they are turned in the plane and scaled.
pub fn carried_move(proj: &Project, id: &str, at: Tick) -> [f32; 4] {
    let canvas = [proj.canvas.width as f32, proj.canvas.height as f32];
    let parent = proj.clip(id).and_then(|(_, c)| c.parent.as_deref());
    let back = inverse(&carried(proj, parent, at, canvas).0);
    back.map_or([1.0, 0.0, 0.0, 1.0], |m| [m[0], m[1], m[3], m[4]])
}

/// Moves a clip's anchor by `by` (canvas fractions), wherever it is over time: its `x` and `y`,
/// their keyframes and its path's points.
fn shift(clip: &mut Clip, by: [f32; 2]) {
    for (prop, d) in [(Prop::X, by[0]), (Prop::Y, by[1])] {
        clip.set_base(prop, clip.base(prop) + d);
        for k in clip.keys.get_mut(&prop).into_iter().flatten() {
            k.v += d;
        }
    }
    for p in clip.path.iter_mut().flat_map(|p| p.points.iter_mut()) {
        *p = [p[0] + by[0], p[1] + by[1]];
    }
}

/// The size, in canvas pixels at the clip's own scale, of the box its anchor is a point of: a
/// picture's as it is fitted and cropped, a shape's, a title's block of text.
fn box_of(proj: &Project, clip: &Clip, scale: f32) -> [f32; 2] {
    let canvas = [proj.canvas.width as f32, proj.canvas.height as f32];
    let crop = clip.look.crop;
    let cropped = |w: f32, h: f32| [w * (1.0 - crop.left - crop.right).max(1e-3), h * (1.0 - crop.top - crop.bottom)];
    let [w, h] = match &clip.content {
        ClipContent::Media { media, .. } => {
            let [w, h] = proj.media.get(media).and_then(|m| m.size).map_or(canvas, |s| s.map(|v| v.max(1) as f32));
            let [w, h] = cropped(w, h.max(1e-3));
            let fit = (canvas[0] / w).min(canvas[1] / h);
            [w * fit, h * fit]
        }
        ClipContent::Compound { .. } => cropped(canvas[0], canvas[1]),
        ClipContent::Shape { shape } => shape.size,
        // as it is laid out, in the face the renderer has when it is not given the title's own
        ClipContent::Text { .. } => {
            let options = reelforge_compile::CompileOptions { encode: false };
            let spec = reelforge_compile::compile(proj, &options).ok();
            let block =
                spec.as_ref().and_then(|s| s.layers.iter().find(|l| l.id == clip.id)).and_then(|l| match &l.content {
                    LayerContent::Text(text) => Some(reelforge_render::text_block(text)),
                    _ => None,
                });
            block.unwrap_or([0.0, 0.0])
        }
        ClipContent::Solid { .. } | ClipContent::Adjustment => canvas,
    };
    [w * scale, h * scale]
}

/// Gives `clip` another anchor. With `keep`, its `x` and `y` change with it so that the picture
/// stays where it lies at timeline time `at` (as it is scaled, stretched and turned in the plane
/// there).
pub(crate) fn set_anchor(proj: &mut Project, id: &str, anchor: [f32; 2], keep: bool, at: Option<Tick>) {
    let Some((_, clip)) = proj.clip(id) else { return };
    let canvas = [proj.canvas.width as f32, proj.canvas.height as f32];
    let before = clip.transform.anchor;
    let mut by = [0.0, 0.0];
    if keep && before != anchor {
        let p = placed(clip, at.unwrap_or(clip.start), canvas);
        let [w, h] = box_of(proj, clip, p.scale);
        // from the old anchor to the new one, in the picture as it lies
        let (dx, dy) = ((anchor[0] - before[0]) * w * p.stretch[0], (anchor[1] - before[1]) * h * p.stretch[1]);
        // a path turns the clip as it goes; the anchor is moved as the clip itself is turned
        let t = (at.unwrap_or(clip.start) - clip.start).max(Tick::ZERO).min(clip.duration);
        let (sin, cos) = clip.value_at(Prop::Rotation, t).to_radians().sin_cos();
        by = [(cos * dx - sin * dy) / canvas[0], (sin * dx + cos * dy) / canvas[1]];
    }
    let clip = proj.clip_mut(id).expect("found above");
    clip.transform.anchor = anchor;
    shift(clip, by);
}

/// Gives `clip` another anchor the way CSS's `transform-origin` does: its box stays where it lies
/// when it is neither scaled nor turned, so its `x` and `y` (their keyframes, its path) go on
/// saying where its middle rests. Counted from the middle: the anchor it has is taken for 0.5.
pub(crate) fn set_origin(proj: &mut Project, id: &str, anchor: [f32; 2]) {
    let Some((_, clip)) = proj.clip(id) else { return };
    let canvas = [proj.canvas.width as f32, proj.canvas.height as f32];
    let [w, h] = box_of(proj, clip, 1.0);
    let by = [(anchor[0] - 0.5) * w / canvas[0], (anchor[1] - 0.5) * h / canvas[1]];
    let clip = proj.clip_mut(id).expect("found above");
    clip.transform.anchor = anchor;
    shift(clip, by);
}

fn pictured(proj: &Project, id: &str, what: &str) -> Result<(), EngineError> {
    let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    if proj.tracks[ti].kind == TrackKind::Audio || clip.is_adjustment() {
        return Err(EngineError::Params(format!("{what} applies to picture, title and shape clips")));
    }
    Ok(())
}

/// `clip.setParent`.
pub(crate) fn clip_set_parent(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let parent = p.opt_str("parent")?.map(str::to_string);
    let keep = p.opt_bool("keep")?.unwrap_or(true);
    let at = p.opt_time("at")?;
    s.edit("Parent", |proj| {
        pictured(proj, &id, "a parent")?;
        if let Some(parent) = &parent {
            pictured(proj, parent, "carrying another clip")?;
            // up from the parent: the clip itself must not be there, nor too many others
            let (mut up, mut steps) = (Some(parent.as_str()), 0);
            while let Some(c) = up {
                steps += 1;
                if c == id {
                    return Err(EngineError::Params("a clip cannot carry itself, or what carries it".into()));
                }
                if steps > reelforge_renderspec::PARENTS {
                    return Err(EngineError::Params(format!(
                        "at most {} clips deep may carry one another",
                        reelforge_renderspec::PARENTS
                    )));
                }
                up = proj.clip(c).and_then(|(_, c)| c.parent.as_deref());
            }
        }
        let canvas = [proj.canvas.width as f32, proj.canvas.height as f32];
        let (_, clip) = proj.clip(&id).expect("found above");
        let when = at.unwrap_or(clip.start);
        let mut change = None;
        if keep && clip.parent != parent {
            // where it shows now, then the same seen from its new parent
            let own = placed(clip, when, canvas);
            let (was, was_scale, was_turn) = carried(proj, clip.parent.as_deref(), when, canvas);
            let (now, now_scale, now_turn) = carried(proj, parent.as_deref(), when, canvas);
            let Some(back) = inverse(&now) else {
                return Err(EngineError::Params(
                    "the parent is scaled to nothing there: pass `keep: false`, or another `at`".into(),
                ));
            };
            let centre = [canvas[0] * (0.5 + own.at[0]), canvas[1] * (0.5 + own.at[1])];
            let to = apply(&back, apply(&was, centre));
            let by = [to[0] / canvas[0] - 0.5 - own.at[0], to[1] / canvas[1] - 0.5 - own.at[1]];
            let grown = if now_scale.abs() > 1e-6 { was_scale / now_scale } else { 1.0 };
            change = Some((by, grown, was_turn - now_turn));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        if let Some((by, grown, turned)) = change {
            shift(clip, by);
            clip.set_base(Prop::Scale, clip.base(Prop::Scale) * grown);
            clip.set_base(Prop::Rotation, clip.base(Prop::Rotation) + turned);
            for k in clip.keys.get_mut(&Prop::Scale).into_iter().flatten() {
                k.v *= grown;
            }
            for k in clip.keys.get_mut(&Prop::Rotation).into_iter().flatten() {
                k.v += turned;
            }
        }
        clip.parent = parent;
        Ok(json!({
            "clip": id,
            "parent": clip.parent,
            "x": clip.base(Prop::X),
            "y": clip.base(Prop::Y),
            "scale": clip.base(Prop::Scale),
            "rotation": clip.base(Prop::Rotation),
        }))
    })
}

/// `clip.setPath`.
pub(crate) fn clip_set_path(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let remove = v.get("points").is_some_and(Value::is_null);
    let points: Option<Vec<[f32; 2]>> = match p.opt("points") {
        None => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|_| {
            EngineError::Params("`points` must list the points the path passes through, each [x, y]".into())
        })?),
    };
    if let Some(points) = &points {
        if !(2..=MotionPath::MOST).contains(&points.len()) {
            return Err(EngineError::Params(format!("a path has 2 to {} points", MotionPath::MOST)));
        }
        if points.iter().flatten().any(|v| !v.is_finite() || v.abs() > 10.0) {
            return Err(EngineError::Params("a path's points are within 10 canvases of the centre".into()));
        }
    }
    let (closed, orient) = (p.opt_bool("closed")?, p.opt_bool("orient")?);
    let (start, duration) = (p.opt_time("start")?, p.opt_time("duration")?);
    if start.is_some_and(|t| t < Tick::ZERO) || duration.is_some_and(|d| d <= Tick::ZERO) {
        return Err(EngineError::Params("`start` must not be negative and `duration` must be positive".into()));
    }
    let ease = ease_of(&p)?;
    s.edit("Motion Path", |proj| {
        pictured(proj, &id, "a path")?;
        let clip = proj.clip_mut(&id).expect("found above");
        if remove {
            clip.path = None;
            return Ok(json!({ "clip": id, "path": null }));
        }
        let before = clip.path.take();
        let Some(points) = points.or(before.as_ref().map(|b| b.points.clone())) else {
            return Err(EngineError::Params("`points` is required: the clip has no path yet".into()));
        };
        let start = start.or(before.as_ref().map(|b| b.start)).unwrap_or(Tick::ZERO);
        // to the clip's end unless told, or as long as it took before
        let rest = (clip.duration - start).max(Tick(1));
        let path = MotionPath {
            points,
            closed: closed.or(before.as_ref().map(|b| b.closed)).unwrap_or(false),
            orient: orient.or(before.as_ref().map(|b| b.orient)).unwrap_or(false),
            start,
            duration: duration.or(before.as_ref().map(|b| b.duration)).unwrap_or(rest),
            ease: ease.or(before.as_ref().map(|b| b.ease)).unwrap_or(Ease::InOut),
        };
        let out = serde_json::to_value(&path).expect("serialises");
        clip.path = Some(path);
        Ok(json!({ "clip": id, "path": out }))
    })
}
