//! The timeline panel: a ruler, one row per track with its header, clips as blocks. It reports
//! gestures (seek, select, move, trim, track toggles); the editor turns them into commands.
//!
//! The wheel pans, Ctrl + wheel (or a pinch) zooms around the pointer. Dragging a clip moves it,
//! to another track too; dragging one of its edges trims it. Both snap to the playhead, to the
//! start and to other clips' edges when snapping is on.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use reelforge_model::{ClipContent, MediaKind, Project, TrackKind};
use reelforge_time::Tick;

use crate::i18n::{Lang, Strings};
use crate::theme::{color, icon};

const RULER: f32 = 26.0;
const ROW: f32 = 46.0;
const HEADER: f32 = 132.0;
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
}

#[derive(Clone)]
enum Drag {
    /// A clip, grabbed `grab` points after its start.
    Move { clip: String, grab: f32, track: String },
    /// One edge of a clip.
    Trim { clip: String, end: bool },
    /// The playhead.
    Scrub,
}

impl Default for TimelineView {
    fn default() -> TimelineView {
        TimelineView { px_per_second: 80.0, offset: 0.0, drag: None, snap: true }
    }
}

/// What the user did in the timeline this frame.
#[derive(Default)]
pub(crate) struct TimelineOutput {
    /// Move the playhead here.
    pub seek: Option<Tick>,
    /// Change the selection (`Some(None)` clears it).
    pub select: Option<Option<String>>,
    /// A clip was dropped: `(clip, new start, track)`.
    pub moved: Option<(String, Tick, String)>,
    /// A clip's edge was dropped: `(clip, new start, new end)`, one of them set.
    pub trimmed: Option<(String, Option<Tick>, Option<Tick>)>,
    /// A track's eye or speaker was clicked: `(track, "hidden" | "muted", new value)`.
    pub toggled: Option<(String, &'static str, bool)>,
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

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        playhead: Tick,
        selected: Option<&str>,
        playing: bool,
        lang: Lang,
    ) -> TimelineOutput {
        let words = lang.strings();
        let mut out = TimelineOutput::default();
        // only what the rows need: taking all the panel offers would make a resizable panel grow every frame
        let height = RULER + ROW * p.tracks.len() as f32 + 6.0;
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

        // Where the pointer is.
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        // picture and title tracks top layer first, as they stack; sound below them
        let order: Vec<usize> = (0..p.tracks.len())
            .rev()
            .filter(|&i| p.tracks[i].kind != TrackKind::Audio)
            .chain((0..p.tracks.len()).filter(|&i| p.tracks[i].kind == TrackKind::Audio))
            .collect();
        let row_at = |y: f32| {
            let row = ((y - rect.top() - RULER) / ROW).floor();
            (row >= 0.0).then(|| order.get(row as usize).copied()).flatten()
        };
        let mut hit: Option<Hit> = None;
        if let Some(pos) = pointer
            && lanes.contains(pos)
            && pos.y > ruler.bottom()
            && let Some(ti) = row_at(pos.y)
        {
            let track = &p.tracks[ti];
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
        }

        // Snap targets: the start, the playhead and every other clip's edges.
        let dragged_clip = match &self.drag {
            Some(Drag::Move { clip, .. } | Drag::Trim { clip, .. }) => Some(clip.clone()),
            _ => None,
        };
        let targets: Vec<Tick> = if self.snap {
            let mut t = vec![Tick::ZERO, playhead];
            for track in &p.tracks {
                for c in &track.clips {
                    if dragged_clip.as_deref() != Some(c.id.as_str()) {
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

        // The drag in progress, as it would land now.
        let mut ghost: Option<(String, Tick, Tick, String)> = None;
        if let (Some(drag), Some(pos)) = (&self.drag, response.interact_pointer_pos())
            && response.dragged()
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
                        let to = row_at(pos.y).map_or(track.clone(), |i| p.tracks[i].id.clone());
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
                Drag::Scrub => {}
            }
        }

        // Rows.
        for (row, track) in order.iter().map(|&i| &p.tracks[i]).enumerate() {
            let top = rect.top() + RULER + row as f32 * ROW;
            let row_rect = Rect::from_min_size(egui::pos2(lanes.left(), top), vec2(lanes.width(), ROW));
            painter.rect_filled(row_rect, 0.0, if row % 2 == 0 { color::ROW } else { color::ROW_ALT });
            painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(1.0, color::LINE));

            let lane = painter.with_clip_rect(row_rect.intersect(lanes));
            let dim = track.hidden || track.muted;
            for clip in &track.clips {
                let (mut a, mut b) = (clip.start, clip.end());
                if let Some((g, ga, gb, gt)) = &ghost
                    && *g == clip.id
                {
                    if *gt != track.id {
                        // drawn on the row it would land on, below
                        continue;
                    }
                    (a, b) = (*ga, *gb);
                }
                let r = Rect::from_min_max(
                    egui::pos2(x_of(a), top + 5.0),
                    egui::pos2(x_of(b).max(x_of(a) + 3.0), top + ROW - 5.0),
                );
                paint_clip(
                    &lane,
                    r,
                    p,
                    track.kind,
                    clip,
                    selected == Some(clip.id.as_str()),
                    dim,
                    ghost.as_ref().is_some_and(|g| g.0 == clip.id),
                    words,
                );
            }
            // a clip dragged here from another track
            if let Some((g, ga, gb, gt)) = &ghost
                && *gt == track.id
                && !track.clips.iter().any(|c| c.id == *g)
                && let Some((from, clip)) = p.clip(g)
            {
                let r = Rect::from_min_max(egui::pos2(x_of(*ga), top + 5.0), egui::pos2(x_of(*gb), top + ROW - 5.0));
                paint_clip(&lane, r, p, p.tracks[from].kind, clip, true, false, true, words);
            }

            // Header.
            let head = Rect::from_min_size(egui::pos2(rect.left(), top), vec2(HEADER, ROW));
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
        }
        // the corner above the headers
        let corner = Rect::from_min_size(rect.min, vec2(HEADER, RULER));
        painter.rect_filled(corner, 0.0, color::PANEL);
        painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0, color::LINE));

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
            _ => {}
        }

        // Gestures.
        let in_lanes = |pos: Pos2| pos.x >= lanes.left();
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
            && in_lanes(pos)
        {
            self.drag = Some(match hit.clone() {
                Some(h) if pos.y > ruler.bottom() => {
                    out.select = Some(Some(h.clip.clone()));
                    match h.edge {
                        Some(end) => Drag::Trim { clip: h.clip, end },
                        None => Drag::Move { clip: h.clip, grab: h.grab, track: h.track },
                    }
                }
                _ => Drag::Scrub,
            });
        }
        if response.dragged()
            && matches!(self.drag, Some(Drag::Scrub))
            && let Some(pos) = response.interact_pointer_pos()
        {
            out.seek = Some(t_of(pos.x).max(Tick::ZERO));
        }
        if response.drag_stopped() {
            if let (Some(drag), Some((clip, a, b, track))) = (self.drag.take(), ghost) {
                match drag {
                    Drag::Move { .. } => out.moved = Some((clip, a, track)),
                    Drag::Trim { end: true, .. } => out.trimmed = Some((clip, None, Some(b))),
                    Drag::Trim { end: false, .. } => out.trimmed = Some((clip, Some(a), None)),
                    Drag::Scrub => {}
                }
            }
            self.drag = None;
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && in_lanes(pos)
        {
            if pos.y <= ruler.bottom() {
                out.seek = Some(t_of(pos.x).max(Tick::ZERO));
            } else {
                match hit {
                    Some(h) => out.select = Some(Some(h.clip)),
                    None => {
                        out.select = Some(None);
                        out.seek = Some(t_of(pos.x).max(Tick::ZERO));
                    }
                }
            }
        }
        out
    }
}

/// One clip block.
#[allow(clippy::too_many_arguments)]
fn paint_clip(
    painter: &egui::Painter,
    r: Rect,
    p: &Project,
    kind: TrackKind,
    clip: &reelforge_model::Clip,
    selected: bool,
    dim: bool,
    dragging: bool,
    words: &Strings,
) {
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
    // a lighter top edge, so blocks read as raised
    painter.with_clip_rect(r).line_segment(
        [r.left_top() + vec2(3.0, 1.0), r.right_top() + vec2(-3.0, 1.0)],
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
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
        text.text(
            r.left_center() + vec2(9.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{glyph}  {label}"),
            FontId::proportional(12.0),
            text_color,
        );
    }
}
