//! Timeline templates (剪同款) in the interface: a card for each built-in one, on the drafts page
//! and in the asset panel's templates tab. A card draws the template as the row of its slots, each
//! as wide as it is long and in the colour it has while it is empty, so the rhythm of the edit
//! shows before anything is rendered.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, vec2};
use reelforge_engine::{TimelineTemplate, slot_colour, timeline_templates};

use crate::i18n::{Strings, fill};
use crate::theme::color;

/// A template's shown name.
pub(crate) fn name(id: &str, s: &Strings) -> &'static str {
    timeline_templates().iter().position(|t| t.id == id).map_or("", |i| s.remix_names[i])
}

/// Seconds in as few digits as say it: 16, 16.9.
fn seconds(t: f64) -> String {
    let tenths = (t * 10.0).round() / 10.0;
    if tenths.fract() == 0.0 { format!("{tenths:.0}") } else { format!("{tenths:.1}") }
}

/// The colour an empty slot has, for the screen.
fn tone(n: u32) -> Color32 {
    let [r, g, b, _] = slot_colour(n);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// One template, `w` wide: its slots in a row, its name, how much it takes and what it is for.
/// Clicking it uses it.
pub(crate) fn card(ui: &mut egui::Ui, index: usize, t: &TimelineTemplate, s: &Strings, w: f32) -> egui::Response {
    let pad = 10.0;
    let about = {
        let mut job = egui::text::LayoutJob::single_section(
            s.remix_abouts[index].to_string(),
            egui::TextFormat::simple(FontId::proportional(11.0), color::WEAK),
        );
        job.wrap.max_width = w - pad * 2.0;
        job.wrap.max_rows = 3;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let strip_h = 34.0;
    // room for two lines of what it is for, so the cards of a row are one height
    let h = pad + strip_h + 8.0 + 18.0 + 4.0 + about.size().y.max(28.0) + pad;
    let (rect, r) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, if r.hovered() { color::RAISED } else { color::ROW });
    let stroke = if r.hovered() { Stroke::new(1.0, color::ACCENT) } else { Stroke::new(1.0, color::LINE) };
    painter.rect_stroke(rect, 8.0, stroke, StrokeKind::Inside);

    // the slots, each as wide as it is long
    let strip = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(w - pad * 2.0, strip_h));
    let total = t.seconds().max(0.001) as f32;
    let gap = 2.0;
    let room = strip.width() - gap * (t.slots().saturating_sub(1)) as f32;
    let mut x = strip.left();
    for (i, len) in t.shots().enumerate() {
        let n = i as u32 + 1;
        let piece = Rect::from_min_size(egui::pos2(x, strip.top()), vec2(room * len as f32 / total, strip_h));
        painter.rect_filled(piece, 4.0, tone(n).gamma_multiply(if r.hovered() { 1.0 } else { 0.9 }));
        if piece.width() >= 14.0 {
            painter.text(
                piece.center(),
                Align2::CENTER_CENTER,
                n.to_string(),
                FontId::proportional(11.0),
                Color32::from_white_alpha(210),
            );
        }
        x = piece.right() + gap;
    }

    let line = strip.bottom() + 8.0;
    let info = fill(s.remix_info, &[&t.slots().to_string(), &seconds(t.seconds())]);
    let info = painter.layout_no_wrap(info, FontId::proportional(10.5), color::WEAK);
    let info_w = info.size().x;
    painter.galley(egui::pos2(rect.right() - pad - info_w, line + 3.0), info, color::WEAK);
    painter
        .with_clip_rect(Rect::from_min_max(rect.min, egui::pos2(rect.right() - pad - info_w - 6.0, rect.bottom())))
        .text(
            egui::pos2(rect.left() + pad, line),
            Align2::LEFT_TOP,
            s.remix_names[index],
            FontId::proportional(13.5),
            color::TEXT,
        );
    painter.galley(egui::pos2(rect.left() + pad, line + 18.0 + 4.0), about, color::WEAK);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_are_said_briefly() {
        assert_eq!(seconds(16.0), "16");
        assert_eq!(seconds(16.94), "16.9");
        assert_eq!(seconds(12.04), "12");
    }

    #[test]
    fn every_template_has_a_name_in_every_language() {
        for lang in crate::i18n::Lang::ALL {
            let s = lang.strings();
            for t in timeline_templates() {
                assert!(!name(t.id, s).is_empty(), "{lang:?} {}", t.id);
            }
            assert!(s.remix_abouts.iter().all(|a| !a.is_empty()), "{lang:?}");
        }
    }
}
