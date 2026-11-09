//! The timeline panel: a ruler, one row per track with its header, clips as blocks. It reports
//! gestures (seek, select, move, trim, track toggles); the editor turns them into commands.
//!
//! The wheel pans, Ctrl + wheel (or a pinch) zooms around the pointer. Dragging a clip moves it,
//! to another track too; dragging one of its edges trims it. Both snap to the playhead, to the
//! start and to other clips' edges when snapping is on. Shift- or Ctrl-clicking adds a clip to the
//! selection (or takes it out), dragging over empty lanes selects the clips the box touches, and
//! dragging one of several selected clips moves them all. Right-click opens the clip's menu.
//!
//! With Alt held, dragging the cut between two touching clips rolls it (one gets longer, the other
//! shorter, nothing else moves), and dragging a video or sound clip slips it (it stays in place and
//! shows other frames of its source).

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use reelforge_media::Footage;
use reelforge_model::{ClipContent, MediaKind, Project, TrackKind};
use reelforge_time::Tick;

use crate::assets::Item;
use crate::i18n::{Lang, Strings};
use crate::strips::{Source, Strips};
use crate::theme::{color, icon};

const RULER: f32 = 26.0;
const ROW: f32 = 54.0;
const HEADER: f32 = 156.0;
/// How close, in points, a drag must come to a snap target to stick to it.
const SNAP_POINTS: f32 = 8.0;
/// How close to a clip's edge, in points, a drag trims instead of moving.
const EDGE_POINTS: f32 = 6.0;

/// Pan and zoom of the timeline, plus an in-progress drag.
pub(crate) struct TimelineView {
    px_per_second: f32,
    /// The time at the left edge of the lanes, in seconds.
    offset: f64,
    drag: Option<Drag>,
    /// Snap moves and trims to edges and the playhead.
    pub(crate) snap: bool,
    strips: Strips,
    /// The open menu was opened on a clip (else on empty lanes).
    menu_on_clip: bool,
    /// The clip the open menu is for can have its sound detached.
    menu_detachable: bool,
    /// The open menu was opened on this track's header, or on this marker in the ruler.
    menu_track: Option<String>,
    menu_marker: Option<String>,
    /// Where the open menu was opened.
    menu_at: Tick,
    /// How tall a track's row is, in points.
    pub(crate) row: f32,
}

/// The heights a row cycles through.
pub(crate) const ROW_HEIGHTS: [f32; 3] = [40.0, ROW, 76.0];

#[derive(Clone)]
enum Drag {
    /// A clip, grabbed `grab` points after its start.
    Move { clip: String, grab: f32, track: String },
    /// One edge of a clip.
    Trim { clip: String, end: bool },
    /// The cut between two touching clips on a track.
    Roll { left: String, right: String },
    /// A video or sound clip's source under it, grabbed at `from` (x).
    Slip { clip: String, from: f32 },
    /// The playhead.
    Scrub,
    /// A selection box from where the drag began; adding to the selection when `add`.
    Marquee { from: Pos2, add: bool },
}

/// How the selection changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Select {
    /// Only this clip.
    Only(String),
    /// This clip joins the selection, or leaves it.
    Toggle(String),
    /// Exactly these clips (none clears it).
    Set(Vec<String>),
}

/// Something picked in a track header's menu.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum TrackAction {
    /// One place up among its kind (drawn above).
    Up,
    Down,
    Delete,
}

/// Something done to the markers in the ruler.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MarkerAction {
    Add(Tick),
    Remove(String),
}

/// Something picked in a clip's menu (or the menu of empty lanes).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Split,
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
    RippleDelete,
    TrimStart,
    TrimEnd,
    SelectAll,
    DetachAudio,
}

impl Default for TimelineView {
    fn default() -> TimelineView {
        TimelineView {
            px_per_second: 80.0,
            offset: 0.0,
            drag: None,
            snap: true,
            strips: Strips::default(),
            menu_on_clip: false,
            menu_detachable: false,
            menu_track: None,
            menu_marker: None,
            menu_at: Tick::ZERO,
            row: ROW,
        }
    }
}

/// What the user did in the timeline this frame.
#[derive(Default)]
pub(crate) struct TimelineOutput {
    /// Move the playhead here.
    pub seek: Option<Tick>,
    /// Change the selection.
    pub select: Option<Select>,
    /// Several selected clips were dropped this much later (or earlier), each on its own track.
    pub shifted: Option<(Vec<String>, Tick)>,
    /// Picked in a menu.
    pub action: Option<Action>,
    /// Picked in a track header's menu: `(track, what)`.
    pub track_action: Option<(String, TrackAction)>,
    /// Picked in the ruler's menu.
    pub marker: Option<MarkerAction>,
    /// A clip was dropped: `(clip, new start, track)`.
    pub moved: Option<(String, Tick, String)>,
    /// A clip's edge was dropped: `(clip, new start, new end)`, one of them set.
    pub trimmed: Option<(String, Option<Tick>, Option<Tick>)>,
    /// The cut after a clip was dragged: `(clip, by)`.
    pub rolled: Option<(String, Tick)>,
    /// A clip was slipped: `(clip, by)`, how much later in its source it now starts.
    pub slipped: Option<(String, Tick)>,
    /// A track's eye or speaker was clicked: `(track, "hidden" | "muted", new value)`.
    pub toggled: Option<(String, &'static str, bool)>,
    /// Something from the asset panel was dropped: `(item, start, track)`; no track when it was
    /// dropped on a row of another kind (the editor finds or makes one).
    pub dropped: Option<(Item, Tick, Option<String>)>,
}

/// Where the pointer is over a clip.
#[derive(Clone)]
struct Hit {
    clip: String,
    track: String,
    /// Points from the clip's start.
    grab: f32,
    /// Near the start edge, the end edge, or neither.
    edge: Option<bool>,
}

impl TimelineView {
    /// Zooms in (`factor` > 1) or out around the middle of the lanes.
    pub(crate) fn zoom(&mut self, factor: f32) {
        let mid = self.offset + 400.0 / self.px_per_second as f64;
        self.px_per_second = (self.px_per_second * factor).clamp(4.0, 1200.0);
        self.offset = (mid - 400.0 / self.px_per_second as f64).max(0.0);
    }

    /// Fits `seconds` into the last width the lanes had.
    pub(crate) fn fit(&mut self, seconds: f64, width: f32) {
        self.px_per_second = ((width - HEADER - 40.0).max(100.0) / seconds.max(1.0) as f32).clamp(4.0, 1200.0);
        self.offset = 0.0;
    }

    /// Draws the timeline and reads what the pointer did. `media` gives clips their pictures and
    /// waveforms.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        playhead: Tick,
        selected: &[String],
        playing: bool,
        can_paste: bool,
        range: Option<(Tick, Tick)>,
        lang: Lang,
        media: &Footage,
    ) -> TimelineOutput {
        let words = lang.strings();
        let mut out = TimelineOutput::default();
        let row_h = self.row;
        // only what the rows need: taking all the panel offers would make a resizable panel grow every frame
        let height = RULER + row_h * p.tracks.len() as f32 + 6.0;
        let width = ui.available_width().max(HEADER + 120.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let lanes = Rect::from_min_max(rect.min + vec2(HEADER, 0.0), rect.max);
        let fps = p.canvas.fps;

        // Pan and zoom.
        if response.hovered() {
            let (zoom, scroll, pointer) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zoom != 1.0 {
                let x = pointer.map_or(lanes.center().x, |p| p.x.max(lanes.left()));
                let at = self.offset + ((x - lanes.left()) / self.px_per_second) as f64;
                self.px_per_second = (self.px_per_second * zoom).clamp(4.0, 1200.0);
                self.offset = (at - ((x - lanes.left()) / self.px_per_second) as f64).max(0.0);
            } else if scroll != egui::Vec2::ZERO {
                let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
                self.offset = (self.offset - (d / self.px_per_second) as f64).max(0.0);
            }
        }
        // keep the playhead in view while playing
        let view = (lanes.width() / self.px_per_second) as f64;
        if playing && (playhead.seconds() > self.offset + view * 0.95 || playhead.seconds() < self.offset) {
            self.offset = (playhead.seconds() - view * 0.1).max(0.0);
        }
        let (pps, offset) = (self.px_per_second, self.offset);
        let x_of = move |t: Tick| lanes.left() + ((t.seconds() - offset) as f32) * pps;
        let t_of = move |x: f32| Tick::from_seconds(offset + ((x - lanes.left()) / pps) as f64);
        let seconds_of = move |x: f32| offset + ((x - lanes.left()) / pps) as f64;

        painter.rect_filled(rect, 0.0, color::ROW);

        // Ruler.
        let ruler = Rect::from_min_max(lanes.min, egui::pos2(lanes.right(), lanes.top() + RULER));
        painter.rect_filled(Rect::from_min_max(rect.min, egui::pos2(rect.right(), ruler.bottom())), 0.0, color::PANEL);
        let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
            .into_iter()
            .find(|s| s * pps as f64 >= 70.0)
            .unwrap_or(1200.0);
        let minor = if step <= 2.0 { step / 4.0 } else { step / 5.0 };
        let mut s = (offset / minor).floor() * minor;
        let ruler_painter = painter.with_clip_rect(ruler);
        while s <= offset + view {
            let x = lanes.left() + ((s - offset) as f32) * pps;
            let major = (s / step - (s / step).round()).abs() < 1e-6;
            let h = if major { 10.0 } else { 5.0 };
            ruler_painter.line_segment(
                [egui::pos2(x, ruler.bottom() - h), egui::pos2(x, ruler.bottom())],
                Stroke::new(1.0, if major { color::WEAK } else { color::LINE }),
            );
            if major {
                ruler_painter.text(
                    egui::pos2(x + 4.0, ruler.top() + 4.0),
                    Align2::LEFT_TOP,
                    crate::theme::ruler_label(s),
                    FontId::monospace(10.5),
                    color::WEAK,
                );
            }
            s += minor;
        }
        painter.line_segment(
            [rect.left_top() + vec2(0.0, RULER), egui::pos2(rect.right(), ruler.bottom())],
            Stroke::new(1.0, color::LINE),
        );

        // The range that loops, and the markers.
        if let Some((a, b)) = range {
            let r = Rect::from_min_max(egui::pos2(x_of(a), ruler.top()), egui::pos2(x_of(b), ruler.bottom()));
            ruler_painter.rect_filled(r, 0.0, color::ACCENT.gamma_multiply(0.18));
            for x in [r.left(), r.right()] {
                ruler_painter.line_segment(
                    [egui::pos2(x, ruler.top()), egui::pos2(x, ruler.bottom())],
                    Stroke::new(1.5, color::ACCENT),
                );
            }
        }
        for m in &p.markers {
            let x = x_of(m.at);
            let flag = [
                egui::pos2(x, ruler.bottom() - 2.0),
                egui::pos2(x - 5.0, ruler.bottom() - 10.0),
                egui::pos2(x + 5.0, ruler.bottom() - 10.0),
            ];
            ruler_painter.add(egui::Shape::convex_polygon(flag.to_vec(), color::WARN, Stroke::NONE));
            ruler_painter.line_segment(
                [egui::pos2(x, ruler.bottom() - 10.0), egui::pos2(x, ruler.top() + 2.0)],
                Stroke::new(1.0, color::WARN),
            );
            if !m.note.is_empty() {
                ruler_painter.text(
                    egui::pos2(x + 4.0, ruler.top() + 2.0),
                    Align2::LEFT_TOP,
                    &m.note,
                    FontId::proportional(10.0),
                    color::WARN,
                );
            }
        }
        let marker_at = |x: f32| p.markers.iter().find(|m| (x_of(m.at) - x).abs() <= 5.0);

        // Where the pointer is.
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        // picture and title tracks top layer first, as they stack; sound below them
        let order: Vec<usize> = (0..p.tracks.len())
            .rev()
            .filter(|&i| p.tracks[i].kind != TrackKind::Audio)
            .chain((0..p.tracks.len()).filter(|&i| p.tracks[i].kind == TrackKind::Audio))
            .collect();
        let row_at = |y: f32| {
            let row = ((y - rect.top() - RULER) / row_h).floor();
            (row >= 0.0).then(|| order.get(row as usize).copied()).flatten()
        };
        let hit_at = |pos: Pos2| -> Option<Hit> {
            if !lanes.contains(pos) || pos.y <= ruler.bottom() {
                return None;
            }
            let track = &p.tracks[row_at(pos.y)?];
            let mut hit = None;
            for clip in &track.clips {
                let (a, b) = (x_of(clip.start), x_of(clip.end()));
                if pos.x >= a - EDGE_POINTS / 2.0 && pos.x <= b + EDGE_POINTS / 2.0 {
                    let wide = b - a > EDGE_POINTS * 3.0;
                    let edge = if wide && pos.x - a < EDGE_POINTS {
                        Some(false)
                    } else if wide && b - pos.x < EDGE_POINTS {
                        Some(true)
                    } else {
                        None
                    };
                    hit = Some(Hit { clip: clip.id.clone(), track: track.id.clone(), grab: pos.x - a, edge });
                }
            }
            hit
        };
        let hit = pointer.and_then(hit_at);

        // Snap targets: the start, the playhead and every other clip's edges.
        let dragged_clip = match &self.drag {
            Some(Drag::Move { clip, .. } | Drag::Trim { clip, .. } | Drag::Slip { clip, .. }) => Some(clip.clone()),
            Some(Drag::Roll { left, .. }) => Some(left.clone()),
            _ => None,
        };
        let rolled_right = match &self.drag {
            Some(Drag::Roll { right, .. }) => Some(right.as_str()),
            _ => None,
        };
        // dragging one of several selected clips moves them all
        let group: Vec<String> = match &dragged_clip {
            Some(c) if matches!(self.drag, Some(Drag::Move { .. })) && selected.len() > 1 && selected.contains(c) => {
                selected.to_vec()
            }
            _ => Vec::new(),
        };
        let earliest = group.iter().filter_map(|id| p.clip(id)).map(|(_, c)| c.start).min();
        let targets: Vec<Tick> = if self.snap {
            let mut t = vec![Tick::ZERO, playhead];
            t.extend(p.markers.iter().map(|m| m.at));
            for track in &p.tracks {
                for c in &track.clips {
                    if dragged_clip.as_deref() != Some(c.id.as_str())
                        && rolled_right != Some(c.id.as_str())
                        && !group.contains(&c.id)
                    {
                        t.extend([c.start, c.end()]);
                    }
                }
            }
            t
        } else {
            Vec::new()
        };
        let reach = Tick::from_seconds((SNAP_POINTS / pps) as f64);
        let snap_to = |t: Tick| -> Option<Tick> {
            targets.iter().copied().filter(|s| (*s - t).0.abs() <= reach.0).min_by_key(|s| (*s - t).0.abs())
        };
        let mut snapped_at: Option<Tick> = None;

        // The drag in progress, as it would land now. egui no longer counts the frame the button
        // comes up on as dragged, so that frame is asked for too: it is where the drop happens.
        let mut ghost: Option<(String, Tick, Tick, String)> = None;
        // a cut being rolled: `(left, right, where)`; a clip being slipped: `(clip, by)`
        let mut cut: Option<(String, String, Tick)> = None;
        let mut slip: Option<(String, Tick)> = None;
        if let (Some(drag), Some(pos)) = (&self.drag, response.interact_pointer_pos())
            && (response.dragged() || response.drag_stopped())
        {
            match drag {
                Drag::Move { clip, grab, track } => {
                    if let Some((_, c)) = p.clip(clip) {
                        let mut start = fps.snap(t_of(pos.x - grab).max(Tick::ZERO));
                        let len = c.duration;
                        if let Some(s) = snap_to(start) {
                            snapped_at = Some(s);
                            start = s;
                        } else if let Some(e) = snap_to(start + len) {
                            snapped_at = Some(e);
                            start = (e - len).max(Tick::ZERO);
                        }
                        let mut to = row_at(pos.y).map_or(track.clone(), |i| p.tracks[i].id.clone());
                        if let Some(first) = earliest {
                            // a group stays on its tracks, and never starts before 0
                            to = track.clone();
                            start = start.max(c.start - first);
                        }
                        ghost = Some((clip.clone(), start, start + len, to));
                    }
                }
                Drag::Trim { clip, end } => {
                    if let Some((ti, c)) = p.clip(clip) {
                        let mut t = fps.snap(t_of(pos.x).max(Tick::ZERO));
                        if let Some(s) = snap_to(t) {
                            snapped_at = Some(s);
                            t = s;
                        }
                        let one = fps.frame_duration();
                        let (a, b) =
                            if *end { (c.start, t.max(c.start + one)) } else { (t.min(c.end() - one), c.end()) };
                        ghost = Some((clip.clone(), a, b, p.tracks[ti].id.clone()));
                    }
                }
                Drag::Roll { left, right } => {
                    if let (Some((_, l)), Some((_, r))) = (p.clip(left), p.clip(right)) {
                        let mut t = fps.snap(t_of(pos.x).max(Tick::ZERO));
                        if let Some(s) = snap_to(t) {
                            snapped_at = Some(s);
                            t = s;
                        }
                        let one = fps.frame_duration();
                        let t = t.max(l.start + one).min(r.end() - one);
                        cut = Some((left.clone(), right.clone(), t));
                    }
                }
                Drag::Slip { clip, from } => {
                    if let Some((_, c)) = p.clip(clip) {
                        // the picture follows the pointer: dragging right shows earlier frames
                        let by = Tick(-(t_of(pos.x) - t_of(*from)).0);
                        slip = Some((clip.clone(), slip_range(p, c).map_or(Tick::ZERO, |(lo, hi)| by.max(lo).min(hi))));
                    }
                }
                Drag::Scrub | Drag::Marquee { .. } => {}
            }
        }
        // how far a dragged group has moved
        let group_by: Option<Tick> = match (&ghost, &dragged_clip) {
            (Some((_, a, _, _)), Some(c)) if !group.is_empty() => p.clip(c).map(|(_, clip)| *a - clip.start),
            _ => None,
        };

        // Something from the asset panel held over the lanes: where it would land.
        let incoming = egui::DragAndDrop::payload::<Item>(ui.ctx());
        let landing: Option<(Tick, Option<usize>)> = match (&incoming, ui.input(|i| i.pointer.latest_pos())) {
            (Some(item), Some(pos)) if lanes.contains(pos) && pos.y > ruler.bottom() => {
                let len = item.length(p);
                let mut start = fps.snap(t_of(pos.x).max(Tick::ZERO));
                if let Some(s) = snap_to(start) {
                    snapped_at = Some(s);
                    start = s;
                } else if let Some(e) = snap_to(start + len) {
                    snapped_at = Some(e);
                    start = (e - len).max(Tick::ZERO);
                }
                let kind = item.track_kind(p);
                Some((start, row_at(pos.y).filter(|&i| p.tracks[i].kind == kind)))
            }
            _ => None,
        };

        // Rows.
        for (row, (index, track)) in order.iter().map(|&i| (i, &p.tracks[i])).enumerate() {
            let top = rect.top() + RULER + row as f32 * row_h;
            let row_rect = Rect::from_min_size(egui::pos2(lanes.left(), top), vec2(lanes.width(), row_h));
            painter.rect_filled(row_rect, 0.0, if row % 2 == 0 { color::ROW } else { color::ROW_ALT });
            painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(1.0, color::LINE));

            let lane = painter.with_clip_rect(row_rect.intersect(lanes));
            let dim = track.hidden || track.muted;
            for clip in &track.clips {
                let (mut a, mut b) = (clip.start, clip.end());
                if let Some(by) = group_by
                    && group.contains(&clip.id)
                {
                    (a, b) = (a + by, b + by);
                } else if let Some((g, ga, gb, gt)) = &ghost
                    && *g == clip.id
                {
                    if *gt != track.id {
                        // drawn on the row it would land on, below
                        continue;
                    }
                    (a, b) = (*ga, *gb);
                } else if let Some((l, r, at)) = &cut {
                    if *l == clip.id {
                        b = *at;
                    } else if *r == clip.id {
                        a = *at;
                    }
                }
                let r = Rect::from_min_max(
                    egui::pos2(x_of(a), top + 5.0),
                    egui::pos2(x_of(b).max(x_of(a) + 3.0), top + row_h - 5.0),
                );
                // a moved clip carries its source along; a trimmed one keeps it in place
                let mut start = if b - a == clip.duration { a } else { clip.start };
                if let Some((s, by)) = &slip
                    && *s == clip.id
                {
                    start -= *by;
                }
                paint_clip(
                    &lane,
                    r,
                    p,
                    track.kind,
                    clip,
                    start,
                    Look {
                        selected: selected.contains(&clip.id),
                        dim,
                        dragging: ghost.as_ref().is_some_and(|g| g.0 == clip.id)
                            || cut.as_ref().is_some_and(|(l, r, _)| *l == clip.id || *r == clip.id)
                            || slip.as_ref().is_some_and(|(s, _)| *s == clip.id)
                            || (group_by.is_some() && group.contains(&clip.id)),
                    },
                    words,
                    &mut self.strips,
                    &seconds_of,
                );
            }
            // transitions on the cuts they play on
            if track.kind == TrackKind::Video && ghost.is_none() && cut.is_none() {
                for pair in track.clips.windows(2) {
                    if let Some(t) = pair[1].transition
                        && pair[0].end() == pair[1].start
                    {
                        let len = t.duration.min(pair[0].duration).min(pair[1].duration);
                        let w = x_of(pair[1].start + len) - x_of(pair[1].start);
                        paint_cut(&lane, x_of(pair[1].start), top + row_h / 2.0, w);
                    }
                }
            }
            // a clip dragged here from another track
            if let Some((g, ga, gb, gt)) = &ghost
                && *gt == track.id
                && !track.clips.iter().any(|c| c.id == *g)
                && let Some((from, clip)) = p.clip(g)
            {
                let r = Rect::from_min_max(egui::pos2(x_of(*ga), top + 5.0), egui::pos2(x_of(*gb), top + row_h - 5.0));
                let look = Look { selected: true, dim: false, dragging: true };
                paint_clip(&lane, r, p, p.tracks[from].kind, clip, *ga, look, words, &mut self.strips, &seconds_of);
            }

            // what is held over this row
            if let (Some(item), Some((start, Some(i)))) = (&incoming, landing)
                && i == index
            {
                let r = Rect::from_min_max(
                    egui::pos2(x_of(start), top + 5.0),
                    egui::pos2(x_of(start + item.length(p)).max(x_of(start) + 3.0), top + row_h - 5.0),
                );
                lane.rect_filled(r, CornerRadius::same(5), color::ACCENT.gamma_multiply(0.25));
                lane.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.5, color::ACCENT), StrokeKind::Inside);
                lane.with_clip_rect(r.intersect(row_rect)).text(
                    r.left_center() + vec2(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    item.label(p, words),
                    FontId::proportional(12.0),
                    color::TEXT,
                );
            }

            // Header.
            let head = Rect::from_min_size(egui::pos2(rect.left(), top), vec2(HEADER, row_h));
            painter.rect_filled(head, 0.0, color::PANEL);
            painter.line_segment([head.right_top(), head.right_bottom()], Stroke::new(1.0, color::LINE));
            painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, color::LINE));
            let (glyph, tint) = match track.kind {
                TrackKind::Video => (icon::FILM, color::VIDEO),
                TrackKind::Text => (icon::TEXT, color::TEXT_CLIP),
                TrackKind::Audio => (icon::MUSIC, color::AUDIO),
            };
            painter.text(
                head.left_center() + vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                glyph,
                FontId::proportional(15.0),
                tint,
            );
            painter.with_clip_rect(head.shrink(2.0)).text(
                head.left_center() + vec2(34.0, 0.0),
                Align2::LEFT_CENTER,
                lang.track_name(&track.name),
                FontId::proportional(12.5),
                if dim { color::WEAK } else { color::TEXT },
            );
            let (key, on, glyph_on, glyph_off, tip) = if track.kind == TrackKind::Audio {
                ("muted", track.muted, icon::MUTED, icon::SOUND, if track.muted { words.unmute } else { words.mute })
            } else {
                ("hidden", track.hidden, icon::HIDDEN, icon::SHOWN, if track.hidden { words.show } else { words.hide })
            };
            let toggle = Rect::from_center_size(egui::pos2(head.right() - 20.0, head.center().y), vec2(26.0, 26.0));
            let r = ui
                .put(
                    toggle,
                    egui::Button::new(
                        egui::RichText::new(if on { glyph_on } else { glyph_off }).size(14.0).color(if on {
                            color::WARN
                        } else {
                            color::WEAK
                        }),
                    )
                    .frame(false),
                )
                .on_hover_text(tip);
            if r.clicked() {
                out.toggled = Some((track.id.clone(), key, !on));
            }
            // a locked track's clips stay as they are
            let lock = toggle.translate(vec2(if track.kind == TrackKind::Video { -52.0 } else { -26.0 }, 0.0));
            let r = ui
                .put(
                    lock,
                    egui::Button::new(
                        egui::RichText::new(if track.locked { icon::LOCKED } else { icon::UNLOCKED })
                            .size(14.0)
                            .color(if track.locked { color::WARN } else { color::WEAK }),
                    )
                    .frame(false),
                )
                .on_hover_text(if track.locked { words.unlock } else { words.lock });
            if r.clicked() {
                out.toggled = Some((track.id.clone(), "locked", !track.locked));
            }
            if track.locked {
                lane.rect_filled(row_rect, 0.0, Color32::from_black_alpha(70));
            }
            // a magnetic track keeps its clips end to end
            if track.kind == TrackKind::Video {
                let magnet = toggle.translate(vec2(-26.0, 0.0));
                let r = ui
                    .put(
                        magnet,
                        egui::Button::new(egui::RichText::new(icon::SNAP).size(14.0).color(if track.magnetic {
                            color::ACCENT
                        } else {
                            color::WEAK
                        }))
                        .frame(false),
                    )
                    .on_hover_text(if track.magnetic { words.magnetic_on } else { words.magnetic_off });
                if r.clicked() {
                    out.toggled = Some((track.id.clone(), "magnetic", !track.magnetic));
                }
            }
        }
        // the corner above the headers
        let corner = Rect::from_min_size(rect.min, vec2(HEADER, RULER));
        painter.rect_filled(corner, 0.0, color::PANEL);
        painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0, color::LINE));

        // Held over a row of another kind: it goes on a track of its own kind, at this time.
        if let Some((start, None)) = landing {
            let x = x_of(start);
            painter.with_clip_rect(lanes).line_segment(
                [egui::pos2(x, ruler.bottom()), egui::pos2(x, rect.bottom())],
                Stroke::new(2.0, color::ACCENT),
            );
        }
        if let Some(item) = response.dnd_release_payload::<Item>()
            && let Some((start, row)) = landing
        {
            out.dropped = Some(((*item).clone(), start, row.map(|i| p.tracks[i].id.clone())));
        }

        // Snap guide.
        if let Some(t) = snapped_at {
            let x = x_of(t);
            painter.with_clip_rect(lanes).line_segment(
                [egui::pos2(x, ruler.bottom()), egui::pos2(x, rect.bottom())],
                Stroke::new(1.0, color::ACCENT),
            );
        }

        // Playhead.
        let x = x_of(playhead);
        if x >= lanes.left() - 1.0 && x <= lanes.right() + 1.0 {
            let on_lanes = painter.with_clip_rect(lanes);
            on_lanes.line_segment(
                [egui::pos2(x, ruler.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(1.5, color::PLAYHEAD),
            );
            let handle = Rect::from_center_size(egui::pos2(x, ruler.top() + 7.0), vec2(11.0, 14.0));
            on_lanes.rect_filled(handle, CornerRadius::same(3), color::PLAYHEAD);
        }

        // Cursor.
        if self.drag.is_none() && hit.as_ref().is_some_and(|h| h.edge.is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        match &self.drag {
            Some(Drag::Trim { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            Some(Drag::Move { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            Some(Drag::Roll { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn),
            Some(Drag::Slip { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::AllScroll),
            _ => {}
        }

        // Gestures.
        let in_lanes = |pos: Pos2| pos.x >= lanes.left();
        // egui calls it a drag only once the pointer has travelled a few points, so what was grabbed
        // is read where the button went down: an edge is only a few points wide.
        let pressed_at = ui.input(|i| i.pointer.press_origin());
        let adding = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let alt = ui.input(|i| i.modifiers.alt);
        if response.drag_started()
            && let Some(pos) = pressed_at.or(response.interact_pointer_pos())
            && in_lanes(pos)
        {
            let locked = |h: &Hit| p.track(&h.track).is_some_and(|t| t.locked);
            self.drag = Some(match hit_at(pos) {
                Some(h) if pos.y > ruler.bottom() && locked(&h) => {
                    out.select = Some(Select::Only(h.clip.clone()));
                    Drag::Marquee { from: pos, add: adding }
                }
                Some(h) if pos.y > ruler.bottom() => match h.edge {
                    Some(end) if alt && touching(p, &h.clip, end).is_some() => {
                        out.select = Some(Select::Only(h.clip.clone()));
                        let (left, right) = touching(p, &h.clip, end).expect("checked");
                        Drag::Roll { left, right }
                    }
                    None if alt && p.clip(&h.clip).is_some_and(|(_, c)| slip_range(p, c).is_some()) => {
                        out.select = Some(Select::Only(h.clip.clone()));
                        Drag::Slip { clip: h.clip, from: pos.x }
                    }
                    Some(end) => {
                        out.select = Some(Select::Only(h.clip.clone()));
                        Drag::Trim { clip: h.clip, end }
                    }
                    None => {
                        // grabbing one of a selection keeps it, to move them all
                        if !selected.contains(&h.clip) {
                            out.select = Some(Select::Only(h.clip.clone()));
                        }
                        Drag::Move { clip: h.clip, grab: h.grab, track: h.track }
                    }
                },
                _ if pos.y > ruler.bottom() => Drag::Marquee { from: pos, add: adding },
                _ => Drag::Scrub,
            });
        }
        // the selection box: every clip it touches
        if let (Some(Drag::Marquee { from, add }), Some(pos)) = (&self.drag, response.interact_pointer_pos())
            && (response.dragged() || response.drag_stopped())
        {
            let boxed = Rect::from_two_pos(*from, pos);
            painter.with_clip_rect(lanes).rect(
                boxed,
                CornerRadius::same(2),
                color::ACCENT.gamma_multiply(0.12),
                Stroke::new(1.0, color::ACCENT),
                StrokeKind::Inside,
            );
            if response.drag_stopped() {
                let mut ids: Vec<String> = if *add { selected.to_vec() } else { Vec::new() };
                for (row, &i) in order.iter().enumerate() {
                    let top = rect.top() + RULER + row as f32 * row_h;
                    for c in &p.tracks[i].clips {
                        let r = Rect::from_min_max(
                            egui::pos2(x_of(c.start), top + 5.0),
                            egui::pos2(x_of(c.end()), top + row_h - 5.0),
                        );
                        if r.intersects(boxed) && !ids.contains(&c.id) {
                            ids.push(c.id.clone());
                        }
                    }
                }
                out.select = Some(Select::Set(ids));
            }
        }
        if response.dragged()
            && matches!(self.drag, Some(Drag::Scrub))
            && let Some(pos) = response.interact_pointer_pos()
        {
            out.seek = Some(t_of(pos.x).max(Tick::ZERO));
        }
        if response.drag_stopped() {
            if let Some((left, _, at)) = cut.take()
                && let Some((_, l)) = p.clip(&left)
                && at != l.end()
            {
                out.rolled = Some((left.clone(), at - l.end()));
            }
            if let Some((clip, by)) = slip.take()
                && by != Tick::ZERO
            {
                out.slipped = Some((clip, by));
            }
            if let (Some(drag), Some((clip, a, b, track))) = (self.drag.take(), ghost) {
                match drag {
                    Drag::Move { .. } if group_by.is_some() => {
                        out.shifted = group_by.filter(|by| *by != Tick::ZERO).map(|by| (group.clone(), by));
                    }
                    Drag::Move { .. } => out.moved = Some((clip, a, track)),
                    Drag::Trim { end: true, .. } => out.trimmed = Some((clip, None, Some(b))),
                    Drag::Trim { end: false, .. } => out.trimmed = Some((clip, Some(a), None)),
                    Drag::Scrub | Drag::Marquee { .. } | Drag::Roll { .. } | Drag::Slip { .. } => {}
                }
            }
            self.drag = None;
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && in_lanes(pos)
        {
            if pos.y <= ruler.bottom() {
                out.seek = Some(marker_at(pos.x).map_or(t_of(pos.x).max(Tick::ZERO), |m| m.at));
            } else {
                match hit {
                    Some(h) if adding => out.select = Some(Select::Toggle(h.clip)),
                    Some(h) => out.select = Some(Select::Only(h.clip)),
                    None => {
                        if !adding {
                            out.select = Some(Select::Set(Vec::new()));
                        }
                        out.seek = Some(t_of(pos.x).max(Tick::ZERO));
                    }
                }
            }
        }
        // Right-click: the clip's menu (it becomes the selection unless it is part of it already).
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.menu_track = None;
            self.menu_marker = None;
            self.menu_on_clip = false;
            self.menu_detachable = false;
            self.menu_at = fps.snap(t_of(pos.x).max(Tick::ZERO));
            if !in_lanes(pos) {
                self.menu_track =
                    (pos.y > ruler.bottom()).then(|| row_at(pos.y)).flatten().map(|i| p.tracks[i].id.clone());
            } else if pos.y <= ruler.bottom() {
                self.menu_marker = Some(marker_at(pos.x).map(|m| m.id.clone()).unwrap_or_default());
            }
        }
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && in_lanes(pos)
            && pos.y > ruler.bottom()
        {
            self.menu_on_clip = match hit_at(pos) {
                Some(h) => {
                    if !selected.contains(&h.clip) {
                        out.select = Some(Select::Only(h.clip.clone()));
                    }
                    self.menu_detachable = detachable(p, &h.clip);
                    true
                }
                None => false,
            };
        }
        let on_clip = self.menu_on_clip;
        let can_detach = self.menu_detachable;
        let (menu_track, menu_marker, menu_at) = (self.menu_track.clone(), self.menu_marker.clone(), self.menu_at);
        response.context_menu(|ui| {
            ui.set_min_width(190.0);
            if let Some(track) = &menu_track {
                let kind = p.track(track).map(|t| t.kind);
                let same: Vec<&str> = p.tracks.iter().filter(|t| Some(t.kind) == kind).map(|t| t.id.as_str()).collect();
                let at = same.iter().position(|t| t == track).unwrap_or(0);
                for (label, what, ok) in [
                    (words.track_up, TrackAction::Up, at + 1 < same.len()),
                    (words.track_down, TrackAction::Down, at > 0),
                    (words.track_delete, TrackAction::Delete, same.len() > 1),
                ] {
                    if ui.add_enabled(ok, egui::Button::new(label)).clicked() {
                        out.track_action = Some((track.clone(), what));
                        ui.close();
                    }
                }
                return;
            }
            if let Some(marker) = &menu_marker {
                if marker.is_empty() {
                    if ui.add(egui::Button::new(words.marker_add).shortcut_text("M")).clicked() {
                        out.marker = Some(MarkerAction::Add(menu_at));
                        ui.close();
                    }
                } else if ui.button(words.marker_remove).clicked() {
                    out.marker = Some(MarkerAction::Remove(marker.clone()));
                    ui.close();
                }
                return;
            }
            let mut item = |ui: &mut egui::Ui, label: &str, keys: &str, action: Action, enabled: bool| {
                let button = egui::Button::new(label).shortcut_text(crate::i18n::keys(ui.ctx(), keys));
                if ui.add_enabled(enabled, button).clicked() {
                    out.action = Some(action);
                    ui.close();
                }
            };
            if on_clip {
                item(ui, words.split, "S", Action::Split, true);
                item(ui, words.trim_start, "Q", Action::TrimStart, true);
                item(ui, words.trim_end, "W", Action::TrimEnd, true);
                if can_detach {
                    item(ui, words.detach_audio, "Alt+D", Action::DetachAudio, true);
                }
                ui.separator();
                item(ui, words.copy, "Ctrl+C", Action::Copy, true);
                item(ui, words.cut, "Ctrl+X", Action::Cut, true);
            }
            item(ui, words.paste, "Ctrl+V", Action::Paste, can_paste);
            if on_clip {
                item(ui, words.duplicate, "Ctrl+D", Action::Duplicate, true);
                ui.separator();
                item(ui, words.delete, "Delete", Action::Delete, true);
                item(ui, words.ripple_delete, "Shift+Delete", Action::RippleDelete, true);
            } else {
                item(ui, words.select_all, "Ctrl+A", Action::SelectAll, true);
            }
        });
        if self.strips.work(ui.ctx(), media, playing) {
            ui.ctx().request_repaint();
        }
        out
    }
}

/// How a clip block is drawn.
#[derive(Copy, Clone)]
struct Look {
    selected: bool,
    dim: bool,
    dragging: bool,
}

/// One clip block, its source placed as if it started at `start`.
#[allow(clippy::too_many_arguments)]
fn paint_clip(
    painter: &egui::Painter,
    r: Rect,
    p: &Project,
    kind: TrackKind,
    clip: &reelforge_model::Clip,
    start: Tick,
    look: Look,
    words: &Strings,
    strips: &mut Strips,
    seconds_of: &dyn Fn(f32) -> f64,
) {
    let Look { selected, dim, dragging } = look;
    let radius = CornerRadius::same(5);
    let (glyph, label, mut fill) = match &clip.content {
        ClipContent::Media { media, .. } => {
            let m = p.media.get(media);
            let glyph = match m.map(|m| m.kind) {
                Some(MediaKind::Image) => icon::IMAGE,
                Some(MediaKind::Audio) => icon::MUSIC,
                _ => icon::VIDEO,
            };
            let fill = if kind == TrackKind::Audio { color::AUDIO } else { color::VIDEO };
            (glyph, m.map_or(media.as_str(), |m| m.name.as_str()).to_string(), fill)
        }
        ClipContent::Text { text, .. } => (icon::TEXT, text.replace('\n', " "), color::TEXT_CLIP),
        ClipContent::Solid { color: c, .. } => {
            let swatch = Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
            (icon::COLOUR, words.kind_colour.to_string(), swatch.lerp_to_gamma(color::VIDEO, 0.35))
        }
    };
    if dim {
        fill = fill.gamma_multiply(0.45);
    }
    if dragging {
        fill = fill.gamma_multiply(0.8);
    }
    painter.rect_filled(r, radius, fill);
    let mut strip = false;
    if let ClipContent::Media { media, .. } = &clip.content
        && let Some(m) = p.media.get(media)
        && let Some(timing) = clip.timing()
    {
        let src = Source { timing, start };
        let inner = painter.with_clip_rect(painter.clip_rect().intersect(r.shrink(1.0)));
        let fade = if dim {
            0.45
        } else if dragging {
            0.8
        } else {
            1.0
        };
        if kind == TrackKind::Audio {
            let gain = 10f32.powf(clip.volume_db / 20.0);
            strips.paint_wave(
                &inner,
                r.shrink2(vec2(0.0, 3.0)),
                m,
                src,
                seconds_of,
                gain,
                Color32::from_white_alpha(150).gamma_multiply(fade),
            );
        } else {
            strips.paint_pictures(&inner, r.shrink(1.0), m, src, seconds_of, Color32::WHITE.gamma_multiply(fade));
            strip = true;
        }
    }
    // a lighter top edge, so blocks read as raised
    painter.with_clip_rect(r).line_segment(
        [r.left_top() + vec2(3.0, 1.0), r.right_top() + vec2(-3.0, 1.0)],
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
    if !dragging {
        paint_motion(painter, r, clip, selected);
    }
    let stroke =
        if selected { Stroke::new(2.0, color::ACCENT) } else { Stroke::new(1.0, Color32::from_black_alpha(90)) };
    painter.rect_stroke(r, radius, stroke, if selected { StrokeKind::Outside } else { StrokeKind::Inside });
    if selected && r.width() > 18.0 {
        // trim grips
        for x in [r.left() + 3.0, r.right() - 3.0] {
            painter.line_segment(
                [egui::pos2(x, r.center().y - 7.0), egui::pos2(x, r.center().y + 7.0)],
                Stroke::new(2.0, Color32::from_white_alpha(200)),
            );
        }
    }
    if r.width() > 24.0 {
        let text = painter.with_clip_rect(r.shrink2(vec2(8.0, 0.0)));
        let text_color = if dim { Color32::from_white_alpha(140) } else { Color32::WHITE };
        let name = format!("{glyph}  {label}");
        if strip {
            // over pictures the name sits on a dark tab in the corner
            let galley = painter.layout_no_wrap(name, FontId::proportional(11.5), text_color);
            let tab = Rect::from_min_size(r.left_top() + vec2(4.0, 4.0), galley.size() + vec2(10.0, 4.0));
            text.rect_filled(tab, CornerRadius::same(4), Color32::from_black_alpha(150));
            text.galley(tab.min + vec2(5.0, 2.0), galley, text_color);
        } else {
            let at =
                if kind == TrackKind::Audio { r.left_top() + vec2(9.0, 4.0) } else { r.left_center() + vec2(9.0, 0.0) };
            let align = if kind == TrackKind::Audio { Align2::LEFT_TOP } else { Align2::LEFT_CENTER };
            text.text(at, align, name, FontId::proportional(12.0), text_color);
        }
    }
}

/// In and out animations and sound fades as ramps at the clip's ends, and its keyframes as diamonds
/// along its foot.
fn paint_motion(painter: &egui::Painter, r: Rect, clip: &reelforge_model::Clip, selected: bool) {
    let len = clip.duration.0 as f32;
    if len <= 0.0 {
        return;
    }
    let x_of = |t: Tick| r.left() + r.width() * (t.0 as f32 / len);
    let on = painter.with_clip_rect(painter.clip_rect().intersect(r));
    let (a, b) = clip.anim.spans(clip.duration);
    let wash = Color32::from_white_alpha(46);
    if clip.anim.enter.is_some() && a > Tick::ZERO {
        let x = x_of(a);
        on.add(egui::Shape::convex_polygon(
            vec![r.left_top(), egui::pos2(x, r.top()), r.left_bottom()],
            wash,
            Stroke::NONE,
        ));
        on.line_segment([r.left_bottom(), egui::pos2(x, r.top())], Stroke::new(1.0, Color32::from_white_alpha(150)));
    }
    if clip.anim.exit.is_some() && b > Tick::ZERO {
        let x = x_of(clip.duration - b);
        on.add(egui::Shape::convex_polygon(
            vec![egui::pos2(x, r.top()), r.right_top(), r.right_bottom()],
            wash,
            Stroke::NONE,
        ));
        on.line_segment([egui::pos2(x, r.top()), r.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(150)));
    }
    // sound fades: the level rising from the left corner and falling to the right one
    let (fi, fo) = clip.fade.spans(clip.duration);
    let shade = Color32::from_black_alpha(90);
    let edge = Stroke::new(1.2, Color32::from_white_alpha(200));
    if fi > Tick::ZERO {
        let x = x_of(fi);
        on.add(egui::Shape::convex_polygon(
            vec![r.left_top(), egui::pos2(x, r.top()), r.left_bottom()],
            shade,
            Stroke::NONE,
        ));
        on.line_segment([r.left_bottom(), egui::pos2(x, r.top())], edge);
    }
    if fo > Tick::ZERO {
        let x = x_of(clip.duration - fo);
        on.add(egui::Shape::convex_polygon(
            vec![egui::pos2(x, r.top()), r.right_top(), r.right_bottom()],
            shade,
            Stroke::NONE,
        ));
        on.line_segment([egui::pos2(x, r.top()), r.right_bottom()], edge);
    }
    let mut times: Vec<Tick> =
        clip.keys.values().flatten().map(|k| k.t).filter(|t| *t >= Tick::ZERO && *t <= clip.duration).collect();
    times.sort();
    times.dedup();
    let fill = if selected { color::ACCENT } else { Color32::from_white_alpha(220) };
    for t in times {
        diamond(
            &on,
            egui::pos2(x_of(t), r.bottom() - 6.0),
            3.5,
            fill,
            Stroke::new(1.0, Color32::from_black_alpha(160)),
        );
    }
}

/// A transition's mark on a cut: a bow tie in a small badge, `w` wide where the transition plays.
pub(crate) fn paint_cut(painter: &egui::Painter, x: f32, mid: f32, w: f32) {
    let w = w.clamp(14.0, 40.0);
    let badge = Rect::from_center_size(egui::pos2(x, mid), vec2(w, 14.0));
    painter.rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(170));
    painter.rect_stroke(badge, CornerRadius::same(4), Stroke::new(1.0, color::ACCENT), StrokeKind::Inside);
    let c = badge.center();
    for s in [-1.0f32, 1.0] {
        painter.add(egui::Shape::convex_polygon(
            vec![c, c + vec2(4.0 * s, -4.0), c + vec2(4.0 * s, 4.0)],
            color::ACCENT,
            Stroke::NONE,
        ));
    }
}

/// A keyframe mark.
pub(crate) fn diamond(painter: &egui::Painter, c: egui::Pos2, h: f32, fill: Color32, stroke: Stroke) {
    let pts = vec![c + vec2(0.0, -h), c + vec2(h, 0.0), c + vec2(0.0, h), c + vec2(-h, 0.0)];
    painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
}

/// The two clips either side of the cut at `clip`'s end (or start): `(left, right)`.
fn touching(p: &Project, clip: &str, end: bool) -> Option<(String, String)> {
    let (ti, c) = p.clip(clip)?;
    let clips = &p.tracks[ti].clips;
    if end {
        clips.iter().find(|o| o.start == c.end()).map(|o| (c.id.clone(), o.id.clone()))
    } else {
        clips.iter().find(|o| o.end() == c.start && o.id != c.id).map(|o| (o.id.clone(), c.id.clone()))
    }
}

/// How far a video or sound clip can slip, earlier and later: `(lo, hi)`; none for other clips,
/// freeze frames and clips whose source has no length to spare.
pub(crate) fn slip_range(p: &Project, c: &reelforge_model::Clip) -> Option<(Tick, Tick)> {
    let ClipContent::Media { media, source_in, speed, .. } = &c.content else { return None };
    if *speed <= 0.0 {
        return None;
    }
    let length = p.media.get(media)?.duration?;
    let span = c.timing().map(|t| Tick(t.travel(c.duration).0.abs()))?;
    let lo = Tick(-source_in.0).scale(1.0 / speed);
    let hi = (length - span - *source_in).scale(1.0 / speed);
    (lo < hi).then_some((lo, hi.max(Tick::ZERO)))
}

/// Whether `clip` is a video clip that still plays its own sound.
pub(crate) fn detachable(p: &Project, clip: &str) -> bool {
    p.clip(clip).is_some_and(|(ti, c)| {
        p.tracks[ti].kind == TrackKind::Video
            && matches!(&c.content, ClipContent::Media { media, silent: false, .. }
                if p.media.get(media).is_some_and(|m| m.kind == MediaKind::Video))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput};
    use reelforge_engine::Session;
    use serde_json::{Value, json};

    /// A timeline driven by synthetic pointer events, its gestures applied to a session the way
    /// the editor applies them.
    struct Rig {
        ctx: egui::Context,
        view: TimelineView,
        session: Session,
        selected: Vec<String>,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                ctx: egui::Context::default(),
                view: TimelineView::default(),
                session: Session::default(),
                selected: vec![],
            }
        }

        fn run(&mut self, cmd: &str, params: Value) -> Value {
            self.session.execute(cmd, &params).unwrap_or_else(|e| panic!("{cmd}: {e}"))
        }

        fn frame(&mut self, events: Vec<Event>) {
            let p = self.session.project().clone();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0))),
                events,
                ..Default::default()
            };
            let mut out = None;
            let mut full = self.ctx.run_ui(input, |ui| {
                out = Some(self.view.show(
                    ui,
                    &p,
                    Tick::ZERO,
                    &self.selected,
                    false,
                    false,
                    None,
                    Lang::En,
                    &Footage::new(),
                ));
            });
            full.textures_delta.clear();
            let out = out.expect("drawn");
            match out.select {
                Some(Select::Only(id)) => self.selected = vec![id],
                Some(Select::Toggle(id)) => {
                    if let Some(i) = self.selected.iter().position(|s| *s == id) {
                        self.selected.remove(i);
                    } else {
                        self.selected.push(id);
                    }
                }
                Some(Select::Set(ids)) => self.selected = ids,
                None => {}
            }
            if let Some((clips, by)) = out.shifted {
                self.run("clip.shift", json!({"clips": clips, "by": by}));
            }
            if let Some((clip, start, track)) = out.moved {
                self.run("clip.move", json!({"clip": clip, "start": start, "track": track}));
            }
            if let Some((clip, start, end)) = out.trimmed {
                let mut params = json!({"clip": clip});
                if let Some(t) = start {
                    params["start"] = json!(t);
                }
                if let Some(t) = end {
                    params["end"] = json!(t);
                }
                self.run("clip.trim", params);
            }
        }

        /// Presses at `from`, moves to `to` in steps and lets go there.
        fn drag(&mut self, from: Pos2, to: Pos2) {
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![button(from, true)]);
            for i in 1..=10 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * (i as f32 / 10.0))]);
            }
            self.frame(vec![button(to, false)]);
            self.frame(vec![]);
        }

        /// The middle of `track`'s row, `seconds` along (the view starts at 0, 80 points a second).
        fn at(&self, track: &str, seconds: f32) -> Pos2 {
            let p = self.session.project();
            let order: Vec<usize> = (0..p.tracks.len())
                .rev()
                .filter(|&i| p.tracks[i].kind != TrackKind::Audio)
                .chain((0..p.tracks.len()).filter(|&i| p.tracks[i].kind == TrackKind::Audio))
                .collect();
            let row = order.iter().position(|&i| p.tracks[i].id == track).expect("a row");
            Pos2::new(HEADER + seconds * 80.0, RULER + ROW * (row as f32 + 0.5))
        }
    }

    fn setup() -> (Rig, String, String, String) {
        let mut rig = Rig::new();
        let low = rig.session.project().tracks[0].id.clone();
        let high = rig.run("track.add", json!({"kind": "video"}))["track"].as_str().unwrap().to_string();
        let clip = rig.run("clip.add", json!({"track": low, "color": [1, 0, 0], "duration": {"seconds": 2}}));
        let clip = clip["clip"].as_str().unwrap().to_string();
        rig.frame(vec![]);
        (rig, low, high, clip)
    }

    #[test]
    fn dragging_a_clip_onto_another_track_moves_it_there() {
        let (mut rig, low, high, clip) = setup();
        rig.view.snap = false;
        rig.drag(rig.at(&low, 1.0), rig.at(&high, 2.0));
        let p = rig.session.project();
        let (ti, c) = p.clip(&clip).expect("still there");
        assert_eq!(p.tracks[ti].id, high, "landed on the other track");
        assert_eq!(c.start, Tick::from_seconds(1.0));
    }

    #[test]
    fn dragging_a_clip_along_its_track_moves_it() {
        let (mut rig, low, _, clip) = setup();
        rig.view.snap = false;
        rig.drag(rig.at(&low, 1.0), rig.at(&low, 3.0));
        let p = rig.session.project();
        let (ti, c) = p.clip(&clip).expect("still there");
        assert_eq!(p.tracks[ti].id, low);
        assert_eq!(c.start, Tick::from_seconds(2.0));
    }

    #[test]
    fn dragging_a_clips_edges_trims_it() {
        let (mut rig, low, _, clip) = setup();
        rig.view.snap = false;
        // the end edge, from 2 s back to 1.5 s
        rig.drag(rig.at(&low, 2.0) - vec2(2.0, 0.0), rig.at(&low, 1.5));
        let c = rig.session.project().clip(&clip).expect("there").1.clone();
        assert_eq!((c.start, c.end()), (Tick::ZERO, Tick::from_seconds(1.5)));
        // the start edge, from 0 s on to 0.5 s
        rig.drag(rig.at(&low, 0.0) + vec2(2.0, 0.0), rig.at(&low, 0.5));
        let c = rig.session.project().clip(&clip).expect("there").1.clone();
        assert_eq!((c.start, c.end()), (Tick::from_seconds(0.5), Tick::from_seconds(1.5)));
    }

    #[test]
    fn a_box_over_empty_lanes_selects_what_it_touches_and_the_group_moves_together() {
        let (mut rig, low, high, first) = setup();
        rig.view.snap = false;
        let second = rig.run(
            "clip.add",
            json!({"track": high, "color": [0, 1, 0], "start": {"seconds": 3}, "duration": {"seconds": 1}}),
        );
        let second = second["clip"].as_str().unwrap().to_string();
        rig.frame(vec![]);
        // from empty space after both clips, back over both rows
        rig.drag(rig.at(&high, 5.0) - vec2(0.0, 20.0), rig.at(&low, 1.0) + vec2(0.0, 10.0));
        assert_eq!(rig.selected.len(), 2, "{:?}", rig.selected);
        assert!(rig.selected.contains(&first) && rig.selected.contains(&second));
        // dragging one of them moves both by the same time, each on its own track
        rig.drag(rig.at(&low, 1.0), rig.at(&high, 2.0));
        let p = rig.session.project();
        let (ti, a) = p.clip(&first).unwrap();
        assert_eq!((p.tracks[ti].id.as_str(), a.start), (low.as_str(), Tick::from_seconds(1.0)));
        assert_eq!(p.clip(&second).unwrap().1.start, Tick::from_seconds(4.0));
        assert_eq!(rig.selected.len(), 2, "the selection stays");
    }
}
