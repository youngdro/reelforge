//! Storyboards in the interface: a card for each built-in one in the templates tab, and the
//! project's copy as a form. A card draws the storyboard as the row of its scenes, each as wide
//! as it is long: a picture's place in the colour of its slot, a backdrop in the accent.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, vec2};
use reelforge_engine::{Storyboard, field_label, scene_label, slot_colour};
use reelforge_model::Project;

use crate::i18n::{Lang, Strings, fill};
use crate::theme::color;

/// The engine's name for the interface's language.
pub(crate) fn lang(lang: Lang) -> reelforge_engine::Lang {
    match lang {
        Lang::En => reelforge_engine::Lang::En,
        Lang::Zh => reelforge_engine::Lang::Zh,
        Lang::Ja => reelforge_engine::Lang::Ja,
    }
}

/// Seconds in as few digits as say it: 16, 16.5.
fn seconds(t: f64) -> String {
    let tenths = (t * 10.0).round() / 10.0;
    if tenths.fract() == 0.0 { format!("{tenths:.0}") } else { format!("{tenths:.1}") }
}

/// One storyboard, `w` wide: its scenes in a row, its name, how much it takes and what it is for.
/// Clicking it uses it.
pub(crate) fn card(ui: &mut egui::Ui, board: &Storyboard, at: Lang, s: &Strings, w: f32) -> egui::Response {
    let said = lang(at);
    let pad = 12.0;
    let about = {
        let mut job = egui::text::LayoutJob::single_section(
            board.about.get(said).to_string(),
            egui::TextFormat::simple(FontId::proportional(11.0), color::WEAK),
        );
        job.wrap.max_width = w - pad * 2.0;
        job.wrap.max_rows = 3;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let strip_h = 34.0;
    let h = pad + strip_h + 8.0 + 18.0 + 4.0 + about.size().y.max(28.0) + pad;
    let (rect, r) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 14.0, if r.hovered() { color::GLASS_HOVER } else { color::ROW });
    let stroke =
        if r.hovered() { Stroke::new(1.0, color::ACCENT) } else { Stroke::new(1.0, Color32::from_white_alpha(18)) };
    painter.rect_stroke(rect, 14.0, stroke, StrokeKind::Inside);

    // the scenes, each as wide as it is long, with what part of the video it is
    let strip = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(w - pad * 2.0, strip_h));
    let total = board.seconds().max(0.001) as f32;
    let gap = 2.0;
    let room = strip.width() - gap * board.scenes.len().saturating_sub(1) as f32;
    let (mut x, mut slot) = (strip.left(), 0);
    for scene in &board.scenes {
        let piece = Rect::from_min_size(egui::pos2(x, strip.top()), vec2(room * scene.seconds as f32 / total, strip_h));
        let tone = if scene.shot.is_some() {
            slot += 1;
            let [r, g, b, _] = slot_colour(slot);
            Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        } else {
            color::ACCENT.gamma_multiply(0.55)
        };
        painter.rect_filled(piece, 6.0, tone.gamma_multiply(if r.hovered() { 1.0 } else { 0.9 }));
        let name = painter.layout_no_wrap(
            scene_label(&scene.id, said).to_string(),
            FontId::proportional(10.5),
            Color32::from_white_alpha(215),
        );
        if name.size().x + 6.0 <= piece.width() {
            painter.galley(piece.center() - name.size() / 2.0, name, Color32::WHITE);
        }
        x = piece.right() + gap;
    }

    let line = strip.bottom() + 8.0;
    let info = fill(s.remix_info, &[&board.slots().to_string(), &seconds(board.seconds())]);
    let info = painter.layout_no_wrap(info, FontId::proportional(10.5), color::WEAK);
    let info_w = info.size().x;
    painter.galley(egui::pos2(rect.right() - pad - info_w, line + 3.0), info, color::WEAK);
    painter
        .with_clip_rect(Rect::from_min_max(rect.min, egui::pos2(rect.right() - pad - info_w - 6.0, rect.bottom())))
        .text(
            egui::pos2(rect.left() + pad, line),
            Align2::LEFT_TOP,
            board.name.get(said),
            FontId::proportional(13.5),
            color::TEXT,
        );
    painter.galley(egui::pos2(rect.left() + pad, line + 18.0 + 4.0), about, color::WEAK);
    r
}

/// The project's copy as it is being typed over: a row for each field.
#[derive(Default)]
pub(crate) struct CopyForm {
    /// Each field's name and what its box holds.
    rows: Vec<(String, String)>,
    /// The field being typed in, by its place.
    typing: Option<usize>,
}

impl CopyForm {
    /// Takes the project's copy for what the boxes hold, but for the one being typed in.
    fn follow(&mut self, p: &Project) {
        let same = self.rows.len() == p.copy.len() && self.rows.iter().zip(&p.copy).all(|(row, f)| row.0 == f.name);
        if !same {
            self.typing = None;
        }
        let typing = self.typing;
        let old = std::mem::take(&mut self.rows);
        self.rows = p
            .copy
            .iter()
            .enumerate()
            .map(|(i, f)| match (typing == Some(i), old.get(i)) {
                (true, Some(row)) => row.clone(),
                _ => (f.name.clone(), f.value.clone()),
            })
            .collect();
    }

    /// Draws the form: each field's name, then its words to type over. Returns a field and what
    /// it was just given, when a box is left holding something else than the project has.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, p: &Project, at: Lang, s: &Strings) -> Option<(String, String)> {
        self.follow(p);
        let said = lang(at);
        let mut given = None;
        crate::theme::section(ui, s.copy_title, |ui| {
            ui.label(egui::RichText::new(s.copy_tip).small().color(color::WEAK));
            ui.add_space(4.0);
            for (i, (name, text)) in self.rows.iter_mut().enumerate() {
                ui.label(egui::RichText::new(field_label(name, said)).size(11.0).color(color::WEAK));
                let r = ui.add(
                    egui::TextEdit::singleline(text).id_salt(("copy", name.as_str())).desired_width(f32::INFINITY),
                );
                if r.has_focus() {
                    self.typing = Some(i);
                }
                if r.lost_focus() {
                    if self.typing == Some(i) {
                        self.typing = None;
                    }
                    if p.copy.get(i).is_some_and(|f| f.value != *text) {
                        given = Some((name.clone(), text.clone()));
                    }
                }
                ui.add_space(2.0);
            }
        });
        given
    }

    /// Lets go of what was typed: the project's copy shows again.
    pub(crate) fn reset(&mut self) {
        self.typing = None;
        self.rows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_storyboard_says_what_it_is_in_every_language() {
        for at in Lang::ALL {
            for board in reelforge_engine::storyboards() {
                assert!(!board.name.get(lang(at)).is_empty() && !board.about.get(lang(at)).is_empty());
                for scene in &board.scenes {
                    assert_ne!(scene_label(&scene.id, lang(at)), scene.id, "{at:?} {}", scene.id);
                }
                for field in &board.fields {
                    assert_ne!(field_label(&field.id, lang(at)), field.id, "{at:?} {}", field.id);
                }
            }
        }
    }

    #[test]
    fn the_copy_form_follows_the_project_but_for_the_box_being_typed_in() {
        let mut p = Project {
            copy: vec![
                reelforge_model::CopyField { name: "product".into(), value: "保温杯".into() },
                reelforge_model::CopyField { name: "price".into(), value: "129".into() },
            ],
            ..Default::default()
        };
        let mut form = CopyForm::default();
        form.follow(&p);
        assert_eq!(form.rows[1], ("price".to_string(), "129".to_string()));
        // typing in one box: the other follows the project, this one keeps what was typed
        form.typing = Some(0);
        form.rows[0].1 = "保温".into();
        p.copy[1].value = "99".into();
        form.follow(&p);
        assert_eq!((form.rows[0].1.as_str(), form.rows[1].1.as_str()), ("保温", "99"));
        // other fields altogether: nothing typed is kept
        p.copy.remove(0);
        form.follow(&p);
        assert_eq!(form.rows, [("price".to_string(), "99".to_string())]);
        form.reset();
        assert!(form.rows.is_empty());
    }
}
