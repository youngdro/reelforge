//! Direct manipulation in the preview: click what you see to select it; drag it to move, a corner to
//! scale, the knob above it to turn. A move sticks to the frame's edges and centre lines, the safe
//! area 5 % inside the edges, and the edges and centres of the other layers, and draws the line it
//! sticks to; Shift turns in 15° steps. Alt-click takes the layer under the selected one at that
//! point (and again, the one under that); a double-click on a title asks to edit its words.
//!
//! A selected clip's mask is drawn on it with handles of its own: the dot in its middle moves it,
//! the dots on its edge size it, its knob turns it. In crop mode (asked for in the inspector) the
//! whole picture shows with the crop as a box over it: its corners and edges are dragged, or the
//! box itself is moved.
//!
//! While a gesture runs the preview is drawn from a copy of the project with the clip's new
//! placement; letting go commits it as one `clip.set`, so it is one undo step.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use reelforge_model::{ClipTransform, Mask, MaskKind};
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
    /// The mask: moved, its width or height dragged, turned.
    MaskMove,
    MaskWidth,
    MaskHeight,
    MaskTurn,
    /// The crop's edges that follow the pointer (bits [`LEFT`], [`TOP`], [`RIGHT`], [`BOTTOM`]);
    /// all four move the box.
    Crop(u8),
}

const LEFT: u8 = 1;
const TOP: u8 = 2;
const RIGHT: u8 = 4;
const BOTTOM: u8 = 8;
/// The least of the picture a crop dragged in the preview keeps, each way.
const CROP_MIN: f32 = 0.05;

/// What of the selected clip, besides its placement, is shaped in the preview.
#[derive(Copy, Clone, Default)]
pub(crate) struct Shaping {
    /// Its mask, where the playhead is.
    pub mask: Option<Mask>,
    /// In crop mode: what is cut off the picture as it is shown (left, top, right, bottom). The
    /// outline is then the whole picture's.
    pub crop: Option<[f32; 4]>,
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
    /// How a move across the canvas moves the clip's placement (see [`interact`]).
    back: [f32; 4],
    /// The placement so far.
    pub(crate) now: ClipTransform,
    /// The lines the move sticks to, in output pixels: `[x of a vertical one, y of a horizontal one]`.
    guides: [Option<f32>; 2],
    /// The clip's box when the gesture began (output pixels, upright bounds).
    bounds: Option<Rect>,
    /// The picture's corners on screen when the gesture began.
    quad: [Pos2; 4],
    /// The mask when a mask gesture began, and so far.
    mask: Option<(Mask, Mask)>,
    /// The crop when a crop gesture began, and so far.
    crop: Option<([f32; 4], [f32; 4])>,
}

impl Gesture {
    /// The mask so far, while the mask is what is dragged.
    pub(crate) fn mask(&self) -> Option<Mask> {
        self.mask.map(|(_, now)| now)
    }
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
    /// A mask gesture ended: the clip's mask now.
    pub mask: Option<(String, Mask)>,
    /// A crop gesture ended: what is cut off the picture as shown (left, top, right, bottom).
    pub crop: Option<(String, [f32; 4])>,
    /// A click outside the picture in crop mode: leave it.
    pub leave_crop: bool,
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

/// The picture's own axes on screen: unit vectors along its width and height, their lengths in
/// points, and the points one unit of a mask (the picture's shorter side) covers.
struct Axes {
    origin: Pos2,
    ux: Vec2,
    uy: Vec2,
    w: f32,
    h: f32,
    unit: f32,
}

impl Axes {
    fn of(q: &[Pos2; 4]) -> Axes {
        let (ex, ey) = (q[1] - q[0], q[3] - q[0]);
        let (w, h) = (ex.length().max(1e-3), ey.length().max(1e-3));
        Axes { origin: q[0], ux: ex / w, uy: ey / h, w, h, unit: w.min(h) }
    }

    /// Where the point `uv` of the picture (fractions of its width and height) is on screen.
    fn at(&self, uv: [f32; 2]) -> Pos2 {
        self.origin + self.ux * (uv[0] * self.w) + self.uy * (uv[1] * self.h)
    }

    /// A screen vector as fractions of the picture's width and height.
    fn uv(&self, d: Vec2) -> [f32; 2] {
        [d.dot(self.ux) / self.w, d.dot(self.uy) / self.h]
    }

    /// A mask's own axes on screen (turned with it), one unit long.
    fn mask_axes(&self, m: &Mask) -> (Vec2, Vec2) {
        let (sin, cos) = m.rotation.to_radians().sin_cos();
        ((self.ux * cos + self.uy * sin) * self.unit, (self.uy * cos - self.ux * sin) * self.unit)
    }
}

/// A mask's handles on screen: its middle, the dot that sizes its width (shapes with one), the dot
/// that sizes its height (shapes with one), and its turning knob.
fn mask_handles(ax: &Axes, m: &Mask) -> (Pos2, Option<Pos2>, Option<Pos2>, Pos2) {
    let c = ax.at(m.center);
    let (mx, my) = ax.mask_axes(m);
    let (hw, hh) = (m.size[0] * 0.5, m.size[1] * 0.5);
    let wide = matches!(m.kind, MaskKind::Circle | MaskKind::Rectangle).then(|| c + mx * hw);
    let high = (m.kind != MaskKind::Linear).then(|| c + my * hh);
    let reach = if m.kind == MaskKind::Linear { 0.0 } else { hh };
    let up = -my.normalized();
    let up = if up.is_finite() { up } else { vec2(0.0, -1.0) };
    (c, wide, high, c + up * (reach * ax.unit + KNOB))
}

/// The crop box's corners on screen (as the picture's: top-left first, clockwise).
fn crop_quad(ax: &Axes, c: [f32; 4]) -> [Pos2; 4] {
    let [l, t, r, b] = c;
    [[l, t], [1.0 - r, t], [1.0 - r, 1.0 - b], [l, 1.0 - b]].map(|uv| ax.at(uv))
}

/// Which of the crop box's edges the pointer takes at `p`: a corner (two), the middle of an edge
/// (one), or inside the box (all four: it moves).
fn crop_take(q: &[Pos2; 4], p: Pos2) -> Option<u8> {
    let corners = [LEFT | TOP, RIGHT | TOP, RIGHT | BOTTOM, LEFT | BOTTOM];
    if let Some(i) = (0..4).find(|&i| q[i].distance(p) <= HANDLE_REACH) {
        return Some(corners[i]);
    }
    let edges = [TOP, RIGHT, BOTTOM, LEFT];
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let along = (b - a).normalized();
        let t = (p - a).dot(along);
        let off = ((p - a) - along * t).length();
        if off <= HANDLE_REACH && t > 0.0 && t < (b - a).length() {
            return Some(edges[i]);
        }
    }
    inside(q, p).then_some(LEFT | TOP | RIGHT | BOTTOM)
}

/// `start` with the edges in `bits` moved by `d` (fractions of the picture), keeping
/// [`CROP_MIN`] of it and staying on it; all four edges move the box without resizing it.
fn crop_drag(start: [f32; 4], bits: u8, d: [f32; 2]) -> [f32; 4] {
    let [l, t, r, b] = start;
    if bits == LEFT | TOP | RIGHT | BOTTOM {
        let dx = d[0].clamp(-l, r);
        let dy = d[1].clamp(-t, b);
        return [l + dx, t + dy, r - dx, b - dy];
    }
    let mut now = start;
    if bits & LEFT != 0 {
        now[0] = (l + d[0]).clamp(0.0, 1.0 - r - CROP_MIN);
    }
    if bits & RIGHT != 0 {
        now[2] = (r - d[0]).clamp(0.0, 1.0 - l - CROP_MIN);
    }
    if bits & TOP != 0 {
        now[1] = (t + d[1]).clamp(0.0, 1.0 - b - CROP_MIN);
    }
    if bits & BOTTOM != 0 {
        now[3] = (b - d[1]).clamp(0.0, 1.0 - t - CROP_MIN);
    }
    now
}

/// Handles the pointer over the preview and draws the selection. `outlines` are the visible
/// layers, bottom first; `transform` gives a clip's committed placement, and how a move across
/// the canvas moves it (`[a, b, c, d]`: `x' = a x + b y`, `y' = c x + d y`; a clip another carries
/// is placed along that clip's axes); `canvas` is the frame's size in output pixels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interact(
    ui: &egui::Ui,
    response: &egui::Response,
    view: View,
    canvas: [f32; 2],
    outlines: &[(String, Outline)],
    selected: Option<&str>,
    shaping: Shaping,
    gesture: &mut Option<Gesture>,
    transform: impl Fn(&str) -> Option<(ClipTransform, [f32; 4])>,
) -> StageOutput {
    let mut out = StageOutput::default();
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let selected_quad =
        selected.and_then(|id| outlines.iter().find(|(l, _)| l == id)).map(|(id, o)| (id.clone(), view.quad(o)));
    let topmost = |p: Pos2| outlines.iter().rev().find(|(_, o)| inside(&view.quad(o), p)).map(|(id, _)| id.clone());

    // what the pointer would take, for the cursor and for a drag that starts here
    let take = |p: Pos2| -> Option<(String, Kind, Pos2)> {
        if let Some((id, q)) = &selected_quad {
            let ax = Axes::of(q);
            if let Some(c) = shaping.crop {
                // in crop mode only the crop is dragged
                return crop_take(&crop_quad(&ax, c), p).map(|bits| (id.clone(), Kind::Crop(bits), centre(q)));
            }
            if let Some(m) = &shaping.mask {
                let (mid, wide, high, turn) = mask_handles(&ax, m);
                let near = |h: Pos2| h.distance(p) <= HANDLE_REACH;
                let kind = if near(turn) {
                    Some(Kind::MaskTurn)
                } else if wide.is_some_and(near) {
                    Some(Kind::MaskWidth)
                } else if high.is_some_and(near) {
                    Some(Kind::MaskHeight)
                } else if near(mid) {
                    Some(Kind::MaskMove)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    return Some((id.clone(), kind, mid));
                }
            }
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
        if shaping.crop.is_some() {
            return None;
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
            Some(Kind::Move | Kind::MaskMove) => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
            Some(Kind::Scale) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe),
            Some(Kind::Rotate | Kind::MaskTurn) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            Some(Kind::MaskWidth) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            Some(Kind::MaskHeight) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            Some(Kind::Crop(bits)) => ui.ctx().set_cursor_icon(match bits {
                b if b == LEFT | TOP | RIGHT | BOTTOM => egui::CursorIcon::Move,
                b if b == LEFT | TOP || b == RIGHT | BOTTOM => egui::CursorIcon::ResizeNwSe,
                b if b == RIGHT | TOP || b == LEFT | BOTTOM => egui::CursorIcon::ResizeNeSw,
                b if b & (LEFT | RIGHT) != 0 => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::ResizeVertical,
            }),
            None => {}
        }
    }

    if shaping.crop.is_some() {
        if response.clicked()
            && let Some(p) = pointer
            && take(p).is_none()
        {
            out.leave_crop = true;
        }
    } else if response.double_clicked()
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
        && let Some((start, back)) = transform(&clip)
    {
        if Some(clip.as_str()) != selected {
            out.select = Some(Some(clip.clone()));
        }
        let outline = outlines.iter().find(|(l, _)| *l == clip).map(|(_, o)| o);
        let bounds = outline.map(bounds);
        let quad = outline.map_or([p; 4], |o| view.quad(o));
        let mask = shaping
            .mask
            .filter(|_| matches!(kind, Kind::MaskMove | Kind::MaskWidth | Kind::MaskHeight | Kind::MaskTurn));
        let crop = shaping.crop.filter(|_| matches!(kind, Kind::Crop(_)));
        *gesture = Some(Gesture {
            clip,
            kind,
            from: p,
            centre: c,
            start,
            back,
            now: start,
            guides: [None; 2],
            bounds,
            quad,
            mask: mask.map(|m| (m, m)),
            crop: crop.map(|c| (c, c)),
        });
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
                let [a, b, c, e] = g.back;
                let (dx, dy) = (a * d.x + b * d.y, c * d.x + e * d.y);
                (g.now.x, g.now.y) = (g.start.x + dx / canvas[0], g.start.y + dy / canvas[1]);
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
            Kind::MaskMove | Kind::MaskWidth | Kind::MaskHeight | Kind::MaskTurn => {
                let ax = Axes::of(&g.quad);
                if let Some((was, now)) = &mut g.mask {
                    *now = *was;
                    let (mx, my) = ax.mask_axes(was);
                    // how far along one of the mask's own axes the pointer is from its middle, in units
                    let along = |axis: Vec2| ((p - g.centre).dot(axis) / (ax.unit * ax.unit)).abs();
                    match g.kind {
                        Kind::MaskMove => {
                            let [du, dv] = ax.uv(p - g.from);
                            now.center = [was.center[0] + du, was.center[1] + dv];
                        }
                        Kind::MaskWidth => now.size[0] = (along(mx) * 2.0).clamp(0.01, 4.0),
                        Kind::MaskHeight => now.size[1] = (along(my) * 2.0).clamp(0.01, 4.0),
                        _ => {
                            let angle = |q: Pos2| (q.y - g.centre.y).atan2(q.x - g.centre.x).to_degrees();
                            let mut r = was.rotation + angle(p) - angle(g.from);
                            if shift {
                                r = (r / 15.0).round() * 15.0;
                            }
                            now.rotation = (r + 180.0).rem_euclid(360.0) - 180.0;
                        }
                    }
                }
            }
            Kind::Crop(bits) => {
                let ax = Axes::of(&g.quad);
                if let Some((was, now)) = &mut g.crop {
                    *now = crop_drag(*was, bits, ax.uv(p - g.from));
                }
            }
        }
    }

    // letting go, or a drag the preview lost (the window let go of the pointer)
    if (response.drag_stopped() || !response.dragged())
        && let Some(g) = gesture.take()
    {
        if g.now != g.start {
            out.commit = Some((g.clip.clone(), g.now));
        }
        if let Some((was, now)) = g.mask
            && was != now
        {
            out.mask = Some((g.clip.clone(), now));
        }
        if let Some((was, now)) = g.crop
            && was != now
        {
            out.crop = Some((g.clip, now));
        }
    }
    paint(ui, view, outlines, selected, shaping, gesture.as_ref());
    out
}

/// The selection box, its handles, and the centre lines a move sticks to.
fn paint(
    ui: &egui::Ui,
    view: View,
    outlines: &[(String, Outline)],
    selected: Option<&str>,
    shaping: Shaping,
    gesture: Option<&Gesture>,
) {
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
    let ax = Axes::of(&q);
    if let Some(c) = gesture.and_then(|g| g.crop.map(|(_, now)| now)).or(shaping.crop) {
        // what the crop cuts off is dimmed; the box has thirds and handles on its corners and edges
        let b = crop_quad(&ax, c);
        let dim = Color32::from_black_alpha(150);
        for i in 0..4 {
            let j = (i + 1) % 4;
            painter.add(egui::Shape::convex_polygon(vec![q[i], q[j], b[j], b[i]], dim, Stroke::NONE));
        }
        let thin = Stroke::new(1.0, Color32::from_white_alpha(70));
        for k in [1.0 / 3.0, 2.0 / 3.0] {
            painter.line_segment([b[0] + (b[1] - b[0]) * k, b[3] + (b[2] - b[3]) * k], thin);
            painter.line_segment([b[0] + (b[3] - b[0]) * k, b[1] + (b[2] - b[1]) * k], thin);
        }
        let edge = Stroke::new(1.5, Color32::WHITE);
        for i in 0..4 {
            painter.line_segment([b[i], b[(i + 1) % 4]], edge);
        }
        for i in 0..4 {
            let mid = b[i] + (b[(i + 1) % 4] - b[i]) * 0.5;
            for h in [b[i], mid] {
                painter.rect(
                    Rect::from_center_size(h, vec2(9.0, 9.0)),
                    2.0,
                    Color32::WHITE,
                    line,
                    egui::StrokeKind::Middle,
                );
            }
        }
        return;
    }
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
    if let Some(m) = gesture.and_then(Gesture::mask).or(shaping.mask) {
        paint_mask(&painter, &ax, &m);
    }
}

/// A mask's shape over the picture, with its handles.
fn paint_mask(painter: &egui::Painter, ax: &Axes, m: &Mask) {
    let tint = color::WARN;
    let line = Stroke::new(1.5, tint);
    let (c, wide, high, turn) = mask_handles(ax, m);
    let (mx, my) = ax.mask_axes(m);
    let (hw, hh) = (m.size[0] * 0.5, m.size[1] * 0.5);
    let far = (ax.w + ax.h) / ax.unit;
    match m.kind {
        MaskKind::Circle => {
            let pts: Vec<Pos2> = (0..64)
                .map(|i| {
                    let a = i as f32 / 64.0 * std::f32::consts::TAU;
                    c + mx * (hw * a.cos()) + my * (hh * a.sin())
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, line));
        }
        MaskKind::Rectangle => {
            let pts =
                [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(a, b)| c + mx * (hw * a) + my * (hh * b));
            painter.add(egui::Shape::closed_line(pts.to_vec(), line));
        }
        MaskKind::Mirror => {
            for side in [-1.0, 1.0] {
                painter.line_segment([c - mx * far + my * (hh * side), c + mx * far + my * (hh * side)], line);
            }
        }
        MaskKind::Linear => {
            painter.line_segment([c - mx * far, c + mx * far], line);
        }
    }
    let top = if m.kind == MaskKind::Linear { c } else { c - my * hh };
    painter.line_segment([top, turn], line);
    painter.circle(turn, 5.0, color::STAGE, line);
    painter.circle(c, 4.5, tint, Stroke::new(1.0, color::STAGE));
    for h in [wide, high].into_iter().flatten() {
        painter.circle(h, 4.5, Color32::WHITE, line);
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

    #[test]
    fn a_crop_box_is_dragged_by_its_edges_and_moved_whole() {
        let start = [0.1, 0.2, 0.0, 0.0];
        // the bottom-right corner comes in; the other two edges stay
        let c = crop_drag(start, RIGHT | BOTTOM, [-0.25, -0.5]);
        assert!((c[2] - 0.25).abs() < 1e-6 && (c[3] - 0.5).abs() < 1e-6 && c[0] == 0.1 && c[1] == 0.2);
        // an edge stops where the least of the picture is left, and at the picture's own edge
        assert!((crop_drag(start, LEFT, [2.0, 0.0])[0] - (1.0 - CROP_MIN)).abs() < 1e-6);
        assert_eq!(crop_drag(start, LEFT, [-2.0, 0.0])[0], 0.0);
        // the whole box moves without changing size, and stays on the picture
        let m = crop_drag(start, LEFT | TOP | RIGHT | BOTTOM, [-0.5, -0.1]);
        assert!((m[0] - 0.0).abs() < 1e-6 && (m[2] - 0.1).abs() < 1e-6 && (m[1] - 0.1).abs() < 1e-6);
        assert!((m[0] + m[2] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn the_crop_box_is_taken_by_corner_edge_or_middle() {
        let q = [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 50.0), pos2(0.0, 50.0)];
        assert_eq!(crop_take(&q, pos2(98.0, 49.0)), Some(RIGHT | BOTTOM));
        assert_eq!(crop_take(&q, pos2(50.0, 2.0)), Some(TOP));
        assert_eq!(crop_take(&q, pos2(3.0, 25.0)), Some(LEFT));
        assert_eq!(crop_take(&q, pos2(50.0, 25.0)), Some(LEFT | TOP | RIGHT | BOTTOM));
        assert_eq!(crop_take(&q, pos2(150.0, 25.0)), None);
    }

    #[test]
    fn a_masks_handles_sit_on_its_shape_and_turn_with_it() {
        // a 200 by 100 picture: one unit is 100 points
        let ax = Axes::of(&[pos2(0.0, 0.0), pos2(200.0, 0.0), pos2(200.0, 100.0), pos2(0.0, 100.0)]);
        let mut m = Mask::new(MaskKind::Circle);
        m.size = [1.0, 0.5];
        let (mid, wide, high, turn) = mask_handles(&ax, &m);
        assert_eq!(mid, pos2(100.0, 50.0));
        assert_eq!(wide, Some(pos2(150.0, 50.0)));
        assert_eq!(high, Some(pos2(100.0, 75.0)));
        assert_eq!(turn, pos2(100.0, 25.0 - KNOB));
        // a quarter turn clockwise: the width's handle points down
        m.rotation = 90.0;
        let (_, wide, ..) = mask_handles(&ax, &m);
        assert!(wide.unwrap().distance(pos2(100.0, 100.0)) < 1e-3);
        // a line has no size to drag
        let (_, wide, high, _) = mask_handles(&ax, &Mask::new(MaskKind::Linear));
        assert_eq!((wide, high), (None, None));
    }
}
