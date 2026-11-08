//! The timeline panel: a ruler, one row per track, clips as blocks. It reports gestures; the editor
//! turns them into commands.

use eframe::egui;
use reelforge_model::{ClipContent, Project, TrackKind};
use reelforge_time::Tick;

const RULER: f32 = 22.0;
const ROW: f32 = 40.0;
const HEADER: f32 = 80.0;

/// Pan and zoom of the timeline, plus an in-progress drag.
pub(crate) struct TimelineView {
    px_per_second: f32,
    drag: Option<Drag>,
}

struct Drag {
    clip: String,
    grab_offset: f32,
    track: String,
}

impl Default for TimelineView {
    fn default() -> TimelineView {
        TimelineView { px_per_second: 80.0, drag: None }
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
}

impl TimelineView {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        playhead: Tick,
        selected: Option<&str>,
    ) -> TimelineOutput {
        let mut out = TimelineOutput::default();
        let height = RULER + ROW * p.tracks.len() as f32 + 8.0;
        let width = ui.available_width().max(HEADER + 100.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

        if response.hovered() {
            let zoom = ui.input(|i| i.zoom_delta());
            if zoom != 1.0 {
                self.px_per_second = (self.px_per_second * zoom).clamp(5.0, 1000.0);
            }
        }
        let x_of = |t: Tick| rect.left() + HEADER + t.seconds() as f32 * self.px_per_second;
        let t_of = |x: f32| Tick::from_seconds(((x - rect.left() - HEADER) / self.px_per_second) as f64);

        // Ruler.
        let step = if self.px_per_second > 60.0 {
            1.0
        } else if self.px_per_second > 15.0 {
            5.0
        } else {
            10.0
        };
        let mut s = 0.0;
        while rect.left() + HEADER + s as f32 * self.px_per_second < rect.right() {
            let x = rect.left() + HEADER + s as f32 * self.px_per_second;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + RULER)],
                visuals.widgets.noninteractive.fg_stroke,
            );
            painter.text(
                egui::pos2(x + 3.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{s}s"),
                egui::FontId::monospace(10.0),
                visuals.weak_text_color(),
            );
            s += step;
        }

        // Tracks and clips; the bottom track is drawn as the bottom row.
        let mut clip_hit = None;
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        for (row, track) in p.tracks.iter().rev().enumerate() {
            let top = rect.top() + RULER + row as f32 * ROW;
            let row_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(width, ROW));
            painter.text(
                egui::pos2(rect.left() + 6.0, top + ROW / 2.0),
                egui::Align2::LEFT_CENTER,
                &track.name,
                egui::FontId::proportional(12.0),
                visuals.text_color(),
            );
            painter.line_segment(
                [row_rect.left_bottom(), row_rect.right_bottom()],
                visuals.widgets.noninteractive.bg_stroke,
            );
            let tint = match track.kind {
                TrackKind::Video => egui::Color32::from_rgb(70, 110, 190),
                TrackKind::Text => egui::Color32::from_rgb(170, 110, 60),
                TrackKind::Audio => egui::Color32::from_rgb(60, 150, 100),
            };
            for clip in &track.clips {
                let mut start = clip.start;
                if let (Some(d), Some(pos)) = (&self.drag, pointer)
                    && d.clip == clip.id
                    && response.dragged()
                {
                    start = t_of(pos.x - d.grab_offset);
                }
                let r = egui::Rect::from_min_max(
                    egui::pos2(x_of(start), top + 3.0),
                    egui::pos2(x_of(start + clip.duration).max(x_of(start) + 2.0), top + ROW - 3.0),
                );
                let fill = if selected == Some(clip.id.as_str()) { tint.gamma_multiply(1.4) } else { tint };
                painter.rect_filled(r, 4.0, fill);
                if selected == Some(clip.id.as_str()) {
                    painter.rect_stroke(r, 4.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
                }
                let label = match &clip.content {
                    ClipContent::Media { media, .. } => {
                        p.media.get(media).map_or(media.as_str(), |m| m.name.as_str()).to_string()
                    }
                    ClipContent::Text { text, .. } => format!("T  {text}"),
                    ClipContent::Solid { .. } => "Colour".to_string(),
                };
                painter.with_clip_rect(r).text(
                    r.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
                if let Some(pos) = pointer
                    && r.contains(pos)
                {
                    clip_hit = Some((clip.id.clone(), pos.x - x_of(clip.start), track.id.clone()));
                }
            }
        }

        // Playhead.
        let x = x_of(playhead);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(230, 60, 60)),
        );

        // Gestures.
        let in_ruler = |pos: egui::Pos2| pos.y < rect.top() + RULER;
        if response.drag_started()
            && let (Some(pos), Some((clip, grab, track))) = (response.interact_pointer_pos(), clip_hit.clone())
            && !in_ruler(pos)
        {
            out.select = Some(Some(clip.clone()));
            self.drag = Some(Drag { clip, grab_offset: grab, track });
        }
        if response.dragged()
            && self.drag.is_none()
            && let Some(pos) = response.interact_pointer_pos()
        {
            out.seek = Some(t_of(pos.x));
        }
        if response.drag_stopped()
            && let (Some(d), Some(pos)) = (self.drag.take(), response.interact_pointer_pos())
        {
            let row = ((pos.y - rect.top() - RULER) / ROW).floor() as isize;
            let track = if row >= 0 && (row as usize) < p.tracks.len() {
                p.tracks[p.tracks.len() - 1 - row as usize].id.clone()
            } else {
                d.track
            };
            out.moved = Some((d.clip, t_of(pos.x - d.grab_offset), track));
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            if in_ruler(pos) {
                out.seek = Some(t_of(pos.x));
            } else {
                out.select = Some(clip_hit.map(|c| c.0));
            }
        }
        out
    }
}
