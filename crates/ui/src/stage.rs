//! Direct manipulation in the preview: click what you see to select it; drag it to move, a corner to
//! scale, the knob above it to turn. A move sticks to the frame's edges and centre lines, the safe
//! area 5 % inside the edges, and the edges and centres of the other layers, and draws the line it
//! sticks to; Shift turns in 15° steps. Alt-click takes the layer under the selected one at that
//! point (and again, the one under that); a double-click on a title asks to edit its words.
//!
//! While a gesture runs the preview is drawn from a copy of the project with the clip's new
//! placement; letting go commits it as one `clip.set`, so it is one undo step.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use reelforge_model::ClipTransform;
use reelforge_render::Outline;

use crate::theme::color;

/// How close, in points, the pointer must be to a handle to take it.
const HANDLE_REACH: f32 = 9.0;
/// How far above the box the turning knob sits, in points.
const KNOB: f32 = 26.0;
/// How close, in points, a move sticks to a line.
const SNAP: f32 = 6.0;
/// The safe area's margin, as a fraction of the frame.
const SAFE: f32 = 0.05;

/// What a drag is doing.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Kind {
    Move,
    Scale,
    Rotate,
}

/// A drag in progress on one clip.
pub(crate) struct Gesture {
    pub(crate) clip: String,
    kind: Kind,
    /// Where the pointer went down.
    from: Pos2,
    /// The layer's centre on screen when it did.
    centre: Pos2,
    start: ClipTransform,
    /// The placement so far.
    pub(crate) now: ClipTransform,
    /// The lines the move sticks to, in output pixels: `[x of a vertical one, y of a horizontal one]`.
    guides: [Option<f32>; 2],
    /// The clip's box when the gesture began (output pixels, upright bounds).
    bounds: Option<Rect>,
}

/// What the person did in the preview this frame.
#[derive(Default)]
pub(crate) struct StageOutput {
    /// Change the selection (`Some(None)` clears it).
    pub select: Option<Option<String>>,
    /// A layer was double-clicked: edit its words, if it is a title.
    pub edit: Option<String>,
    /// A gesture ended: the clip's new placement.
    pub commit: Option<(String, ClipTransform)>,
}

/// The preview on screen: where the frame is drawn and how big an output pixel is.
#[derive(Copy, Clone)]
pub(crate) struct View {
    pub frame: Rect,
    /// Points per output pixel.
    pub k: f32,
}

impl View {
    fn point(&self, p: [f32; 2]) -> Pos2 {
        self.frame.min + vec2(p[0], p[1]) * self.k
    }

    fn quad(&self, o: &Outline) -> [Pos2; 4] {
        o.map(|p| self.point(p))
    }
}

/// The upright bounds of an outline, in output pixels.
fn bounds(o: &Outline) -> Rect {
    let (mut lo, mut hi) = (pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN));
    for [x, y] in o {
        lo = lo.min(pos2(*x, *y));
        hi = hi.max(pos2(*x, *y));
    }
    Rect::from_min_max(lo, hi)
}

/// Where a box moved by `d` sticks along one axis: the shift to add and the line it sticks to.
/// `edges` are the box's low edge, centre and high edge; `lines` what they may stick to.
fn stick(edges: [f32; 3], d: f32, lines: &[f32], reach: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for e in edges {
        for &l in lines {
            let off = l - (e + d);
            if off.abs() <= reach && best.is_none_or(|(b, _)| off.abs() < b.abs()) {
                best = Some((off, l));
            }
        }
    }
    best
}

fn centre(q: &[Pos2; 4]) -> Pos2 {
    pos2((q[0].x + q[2].x) * 0.5, (q[0].y + q[2].y) * 0.5)
}

/// Whether `p` is inside the convex quad `q` (either winding).
fn inside(q: &[Pos2; 4], p: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    let s: Vec<f32> = (0..4).map(|i| side(q[i], q[(i + 1) % 4])).collect();
    s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
}

/// The turning knob: above the middle of the box's top edge, along its turn.
fn knob(q: &[Pos2; 4]) -> (Pos2, Pos2) {
    let top = pos2((q[0].x + q[1].x) * 0.5, (q[0].y + q[1].y) * 0.5);
    let up = (top - centre(q)).normalized();
    let up = if up.is_finite() && up != Vec2::ZERO { up } else { vec2(0.0, -1.0) };
    (top, top + up * KNOB)
}

/// Handles the pointer over the preview and draws the selection. `outlines` are the visible
/// layers, bottom first; `transform` gives a clip's committed placement; `canvas` is the frame's
/// size in output pixels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interact(
    ui: &egui::Ui,
    response: &egui::Response,
    view: View,
    canvas: [f32; 2],
    outlines: &[(String, Outline)],
    selected: Option<&str>,
    gesture: &mut Option<Gesture>,
    transform: impl Fn(&str) -> Option<ClipTransform>,
) -> StageOutput {
    let mut out = StageOutput::default();
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let selected_quad =
        selected.and_then(|id| outlines.iter().find(|(l, _)| l == id)).map(|(id, o)| (id.clone(), view.quad(o)));
    let topmost = |p: Pos2| outlines.iter().rev().find(|(_, o)| inside(&view.quad(o), p)).map(|(id, _)| id.clone());

    // what the pointer would take, for the cursor and for a drag that starts here
    let take = |p: Pos2| -> Option<(String, Kind, Pos2)> {
        if let Some((id, q)) = &selected_quad {
            if knob(q).1.distance(p) <= HANDLE_REACH {
                return Some((id.clone(), Kind::Rotate, centre(q)));
            }
            if q.iter().any(|c| c.distance(p) <= HANDLE_REACH) {
                return Some((id.clone(), Kind::Scale, centre(q)));
            }
            if inside(q, p) {
                return Some((id.clone(), Kind::Move, centre(q)));
            }
        }
        let id = topmost(p)?;
        let q = view.quad(&outlines.iter().find(|(l, _)| *l == id)?.1);
        Some((id, Kind::Move, centre(&q)))
    };

    if gesture.is_none()
        && let Some(p) = pointer
        && response.hovered()
    {
        match take(p).map(|t| t.1) {
            Some(Kind::Move) => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
            Some(Kind::Scale) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe),
            Some(Kind::Rotate) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            None => {}
        }
    }

    if response.double_clicked()
        && let Some(p) = pointer
        && let Some(id) = topmost(p)
    {
        out.edit = Some(id);
    } else if response.clicked()
        && let Some(p) = pointer
    {
        let hit = if ui.input(|i| i.modifiers.alt) {
            // the layer under the selected one here, top first, round again at the bottom
            let under: Vec<&String> =
                outlines.iter().rev().filter(|(_, o)| inside(&view.quad(o), p)).map(|(id, _)| id).collect();
            let at = selected.and_then(|s| under.iter().position(|id| *id == s));
            match at {
                Some(i) => under.get((i + 1) % under.len()).map(|id| (*id).clone()),
                None => under.first().map(|id| (*id).clone()),
            }
        } else {
            take(p).map(|t| t.0)
        };
        if hit.as_deref() != selected {
            out.select = Some(hit);
        }
    }

    // a drag is known only once the pointer has moved a little: judge it from where it went down
    if response.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos())
        && let Some((clip, kind, c)) = take(p)
        && let Some(start) = transform(&clip)
    {
        if Some(clip.as_str()) != selected {
            out.select = Some(Some(clip.clone()));
        }
        let bounds = outlines.iter().find(|(l, _)| *l == clip).map(|(_, o)| bounds(o));
        *gesture = Some(Gesture { clip, kind, from: p, centre: c, start, now: start, guides: [None; 2], bounds });
    }

    if let Some(g) = gesture.as_mut()
        && let Some(p) = response.interact_pointer_pos()
    {
        let shift = ui.input(|i| i.modifiers.shift);
        g.now = g.start;
        match g.kind {
            Kind::Move => {
                let mut d = (p - g.from) / view.k;
                let reach = SNAP / view.k;
                g.guides = [None; 2];
                if let Some(b) = g.bounds {
                    // the frame's edges, centre and safe area, and the other layers' edges and centres
                    let mut xs = vec![0.0, canvas[0] * SAFE, canvas[0] * 0.5, canvas[0] * (1.0 - SAFE), canvas[0]];
                    let mut ys = vec![0.0, canvas[1] * SAFE, canvas[1] * 0.5, canvas[1] * (1.0 - SAFE), canvas[1]];
                    for (id, o) in outlines {
                        if *id != g.clip {
                            let r = bounds(o);
                            xs.extend([r.left(), r.center().x, r.right()]);
                            ys.extend([r.top(), r.center().y, r.bottom()]);
                        }
                    }
                    if let Some((off, line)) = stick([b.left(), b.center().x, b.right()], d.x, &xs, reach) {
                        d.x += off;
                        g.guides[0] = Some(line);
                    }
                    if let Some((off, line)) = stick([b.top(), b.center().y, b.bottom()], d.y, &ys, reach) {
                        d.y += off;
                        g.guides[1] = Some(line);
                    }
                }
                (g.now.x, g.now.y) = (g.start.x + d.x / canvas[0], g.start.y + d.y / canvas[1]);
            }
            Kind::Scale => {
                let ratio = p.distance(g.centre) / g.from.distance(g.centre).max(1.0);
                g.now.scale = (g.start.scale * ratio).clamp(0.01, 10.0);
            }
            Kind::Rotate => {
                let angle = |q: Pos2| (q.y - g.centre.y).atan2(q.x - g.centre.x).to_degrees();
                let mut r = g.start.rotation + angle(p) - angle(g.from);
                if shift {
                    r = (r / 15.0).round() * 15.0;
                }
                g.now.rotation = (r + 180.0).rem_euclid(360.0) - 180.0;
            }
        }
    }

    if response.drag_stopped()
        && let Some(g) = gesture.take()
        && g.now != g.start
    {
        out.commit = Some((g.clip, g.now));
    }
    if !response.dragged() && !response.drag_stopped() {
        // a drag the preview lost (the window let go of the pointer)
        if let Some(g) = gesture.take()
            && g.now != g.start
        {
            out.commit = Some((g.clip, g.now));
        }
    }
    paint(ui, view, outlines, selected, gesture.as_ref());
    out
}

/// The selection box, its handles, and the centre lines a move sticks to.
fn paint(ui: &egui::Ui, view: View, outlines: &[(String, Outline)], selected: Option<&str>, gesture: Option<&Gesture>) {
    let painter = ui.painter_at(ui.clip_rect());
    if let Some(g) = gesture
        && g.kind == Kind::Move
    {
        let guide = Stroke::new(1.0, color::PLAYHEAD.gamma_multiply(0.8));
        let f = view.frame;
        if let Some(x) = g.guides[0] {
            let x = view.point([x, 0.0]).x;
            painter.line_segment([pos2(x, f.top()), pos2(x, f.bottom())], guide);
        }
        if let Some(y) = g.guides[1] {
            let y = view.point([0.0, y]).y;
            painter.line_segment([pos2(f.left(), y), pos2(f.right(), y)], guide);
        }
        // the safe area, faintly, while moving
        let safe = Rect::from_min_max(f.min + f.size() * SAFE, f.max - f.size() * SAFE);
        painter.rect_stroke(safe, 0.0, Stroke::new(1.0, Color32::from_white_alpha(40)), egui::StrokeKind::Middle);
    }
    let Some((_, o)) = selected.and_then(|id| outlines.iter().find(|(l, _)| l == id)) else { return };
    let q = view.quad(o);
    let line = Stroke::new(1.5, color::ACCENT);
    for i in 0..4 {
        painter.line_segment([q[i], q[(i + 1) % 4]], line);
    }
    let (top, knob) = knob(&q);
    painter.line_segment([top, knob], line);
    painter.circle(knob, 5.0, color::STAGE, line);
    for c in q {
        let r = Rect::from_center_size(c, vec2(9.0, 9.0));
        painter.rect(r, 2.0, Color32::WHITE, line, egui::StrokeKind::Middle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_inside_turned_boxes() {
        let q = [pos2(10.0, 0.0), pos2(20.0, 10.0), pos2(10.0, 20.0), pos2(0.0, 10.0)];
        assert!(inside(&q, pos2(10.0, 10.0)));
        assert!(!inside(&q, pos2(1.0, 1.0)));
        let reversed = [q[3], q[2], q[1], q[0]];
        assert!(inside(&reversed, pos2(12.0, 9.0)));
    }

    #[test]
    fn the_knob_sits_above_the_top_edge() {
        let q = [pos2(0.0, 0.0), pos2(20.0, 0.0), pos2(20.0, 10.0), pos2(0.0, 10.0)];
        assert_eq!(knob(&q), (pos2(10.0, 0.0), pos2(10.0, -KNOB)));
    }

    #[test]
    fn a_moved_box_sticks_to_the_nearest_line() {
        // a box from 100 to 200 moved 47 right: its right edge (247) is 3 from 250
        assert_eq!(stick([100.0, 150.0, 200.0], 47.0, &[0.0, 250.0, 540.0], 6.0), Some((3.0, 250.0)));
        // its centre is nearer another line than its edges are to theirs
        assert_eq!(stick([100.0, 150.0, 200.0], 389.0, &[540.0, 295.0], 6.0), Some((1.0, 540.0)));
        assert_eq!(stick([100.0, 150.0, 200.0], 20.0, &[540.0], 6.0), None);
        let o: Outline = [[10.0, 0.0], [20.0, 10.0], [10.0, 20.0], [0.0, 10.0]];
        assert_eq!(bounds(&o), Rect::from_min_max(pos2(0.0, 0.0), pos2(20.0, 20.0)));
    }
}
