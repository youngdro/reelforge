//! The command palette: one field that finds every action, every file, marker and title, and
//! goes to a time typed into it. Ctrl+K opens it; the arrows choose, Enter runs, Esc closes.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Modifiers, Sense, Stroke, vec2};
use reelforge_time::Tick;

use crate::i18n::{Lang, Strings};
use crate::theme::{self, color, icon};
use crate::timeline::Action;

/// What choosing an entry does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Run {
    /// An edit of the selection, as the timeline's menu asks for it.
    Act(Action),
    Undo,
    Redo,
    Play,
    Title,
    Colour,
    Import,
    Marker,
    Fit,
    ZoomIn,
    ZoomOut,
    PreviewOnly,
    /// The list of keys.
    Keys,
    Seek(Tick),
    /// Place this file at the playhead.
    Media(String),
    /// Select this clip and go to where it starts.
    Clip(String, Tick),
    Lang(Lang),
}

/// One row.
pub(crate) struct Entry {
    pub icon: &'static str,
    pub label: String,
    /// At the row's end: its keys, or what kind of thing it is.
    pub hint: String,
    pub enabled: bool,
    pub run: Run,
}

impl Entry {
    pub(crate) fn new(icon: &'static str, label: impl Into<String>, hint: impl Into<String>, run: Run) -> Entry {
        Entry { icon, label: label.into(), hint: hint.into(), enabled: true, run }
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Entry {
        self.enabled = enabled;
        self
    }
}

/// How dark the editor gets behind the palette: a little, so it stays readable, as the editor is
/// dark already.
const SHADE: f32 = 72.0;

#[derive(Default)]
pub(crate) struct Palette {
    pub open: bool,
    query: String,
    chosen: usize,
    /// The field takes the keyboard on the first frame, and the chosen row is scrolled to when
    /// the keys moved it.
    fresh: bool,
    moved: bool,
}

impl Palette {
    pub(crate) fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.chosen = 0;
        self.fresh = true;
    }

    /// The palette over everything, while open. `entries` is all there is to find, in the order
    /// shown for an empty field; what was chosen is returned once.
    pub(crate) fn show(&mut self, ctx: &egui::Context, words: &Strings, entries: Vec<Entry>) -> Option<Run> {
        let shown = ctx.animate_bool_with_time(egui::Id::unique("palette shown"), self.open, 0.12);
        if !self.open {
            return None;
        }
        let screen = ctx.content_rect();
        // a time typed in goes first
        let mut rows: Vec<Entry> = Vec::new();
        if let Some(t) = parse_time(&self.query) {
            let at = theme::timecode(t.seconds());
            rows.push(Entry::new(icon::CLOCK, crate::i18n::fill(words.palette_go, &[&at]), "", Run::Seek(t)));
        }
        let mut found: Vec<(u32, Entry)> =
            entries.into_iter().filter_map(|e| score(&self.query, &e.label).map(|s| (s, e))).collect();
        // the best first; equals stay in the order given
        found.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        rows.extend(found.into_iter().map(|(_, e)| e));

        let take = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if take(Modifiers::NONE, Key::Escape) {
            self.open = false;
            return None;
        }
        if take(Modifiers::NONE, Key::ArrowDown) && !rows.is_empty() {
            self.chosen = (self.chosen + 1) % rows.len();
            self.moved = true;
        }
        if take(Modifiers::NONE, Key::ArrowUp) && !rows.is_empty() {
            self.chosen = (self.chosen + rows.len() - 1) % rows.len();
            self.moved = true;
        }
        self.chosen = self.chosen.min(rows.len().saturating_sub(1));
        let mut run = None;
        if take(Modifiers::NONE, Key::Enter)
            && let Some(e) = rows.get(self.chosen)
            && e.enabled
        {
            run = Some(e.run.clone());
        }

        // the editor dims behind it; a click there closes it
        egui::Area::new(egui::Id::unique("palette shade")).order(egui::Order::Foreground).fixed_pos(screen.min).show(
            ctx,
            |ui| {
                let r = ui.allocate_rect(screen, Sense::click());
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha((SHADE * shown) as u8));
                if r.clicked() {
                    self.open = false;
                }
            },
        );
        let width = (screen.width() - 32.0).min(580.0);
        let top = (screen.height() * 0.14).max(12.0) - 10.0 * (1.0 - shown);
        // over its shade, always: a click on the shade (which closes the palette) brings the
        // shade's layer to the top, and the next time the palette would open under it, dimmed
        // and out of reach
        ctx.move_to_top(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("palette")));
        egui::Area::new(egui::Id::unique("palette"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, vec2(0.0, top))
            .show(ctx, |ui| {
                ui.set_opacity(shown);
                egui::Frame::new()
                    .fill(color::PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_white_alpha(26)))
                    .corner_radius(CornerRadius::same(16))
                    .shadow(egui::Shadow {
                        offset: [0, 18],
                        blur: 60,
                        spread: 0,
                        color: color::ACCENT.gamma_multiply(0.18),
                    })
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(icon::SEARCH).size(17.0).color(color::ACCENT));
                            let before = self.query.clone();
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(egui::RichText::new(words.palette_hint).color(color::WEAK))
                                    .font(FontId::proportional(16.0))
                                    .margin(egui::Margin::symmetric(4, 8))
                                    .desired_width(f32::INFINITY),
                            );
                            if std::mem::take(&mut self.fresh) || !field.has_focus() {
                                field.request_focus();
                            }
                            if self.query != before {
                                self.chosen = 0;
                                self.moved = true;
                            }
                        });
                        let line = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover()).0;
                        ui.painter().rect_filled(line, 0.0, color::LINE);
                        if rows.is_empty() {
                            ui.add_space(14.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new(words.palette_empty).color(color::WEAK));
                            });
                            ui.add_space(14.0);
                            return;
                        }
                        let moved = std::mem::take(&mut self.moved);
                        let tallest = (screen.height() * 0.5).clamp(120.0, 372.0);
                        egui::ScrollArea::vertical().max_height(tallest).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, e) in rows.iter().enumerate() {
                                let (rect, r) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
                                // the pointer chooses only when it moves, so the keys are not fought
                                if r.hovered() && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                    self.chosen = i;
                                }
                                let on = i == self.chosen;
                                if on && moved {
                                    r.scroll_to_me(None);
                                }
                                let lit = ctx.animate_bool_with_time(r.id, on, 0.08);
                                let painter = ui.painter();
                                if lit > 0.0 {
                                    theme::accent_rect(painter, rect, 9.0, 0.2 * lit);
                                }
                                let ink = if !e.enabled {
                                    color::WEAK.gamma_multiply(0.6)
                                } else if on {
                                    Color32::WHITE
                                } else {
                                    color::TEXT
                                };
                                painter.text(
                                    rect.left_center() + vec2(12.0, 0.0),
                                    Align2::LEFT_CENTER,
                                    e.icon,
                                    FontId::proportional(15.0),
                                    if on && e.enabled { color::ACCENT_2 } else { color::WEAK },
                                );
                                let hint = painter.layout_no_wrap(
                                    crate::i18n::keys(ctx, &e.hint),
                                    FontId::monospace(11.0),
                                    color::WEAK,
                                );
                                painter.with_clip_rect(rect.with_max_x(rect.right() - hint.size().x - 24.0)).text(
                                    rect.left_center() + vec2(38.0, 0.0),
                                    Align2::LEFT_CENTER,
                                    &e.label,
                                    FontId::proportional(13.5),
                                    ink,
                                );
                                painter.galley(
                                    egui::pos2(
                                        rect.right() - hint.size().x - 12.0,
                                        rect.center().y - hint.size().y / 2.0,
                                    ),
                                    hint,
                                    color::WEAK,
                                );
                                if r.clicked() && e.enabled {
                                    run = Some(e.run.clone());
                                }
                            }
                        });
                    });
            });
        if run.is_some() {
            self.open = false;
        }
        run
    }
}

/// How well `label` answers `query`, or `None` when it does not: the words in order score best,
/// then its letters in order; an empty query is answered by everything alike.
pub(crate) fn score(query: &str, label: &str) -> Option<u32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    if let Some(at) = label.find(&query) {
        // earlier is better, and the start best of all
        return Some(1000 - (at as u32).min(500) + if at == 0 { 500 } else { 0 });
    }
    let mut rest = label.chars();
    query.chars().filter(|c| !c.is_whitespace()).all(|c| rest.any(|l| l == c)).then_some(1)
}

/// A time typed the way they are shown: `12` or `12.5` is seconds, `1:05` a minute and five
/// seconds, `1:02:03` hours too.
pub(crate) fn parse_time(text: &str) -> Option<Tick> {
    let text = text.trim();
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.') {
        return None;
    }
    let mut seconds = 0.0;
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    for (i, part) in parts.iter().enumerate() {
        // only the last part has a fraction
        if part.is_empty() || (part.contains('.') && i + 1 != parts.len()) {
            return None;
        }
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(Tick::from_seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_time_reads_as_it_is_shown() {
        assert_eq!(parse_time("12"), Some(Tick::from_seconds(12.0)));
        assert_eq!(parse_time(" 1:05 "), Some(Tick::from_seconds(65.0)));
        assert_eq!(parse_time("1:02:03.5"), Some(Tick::from_seconds(3723.5)));
        for not in ["", "split", "1:", ":5", "1.5:00", "1:2:3:4", "-3"] {
            assert_eq!(parse_time(not), None, "{not}");
        }
    }

    #[test]
    fn words_in_order_score_over_letters_in_order() {
        assert_eq!(score("", "Split"), Some(0));
        assert!(score("spl", "Split") > score("spl", "Ripple split"));
        assert!(score("split", "Ripple split") > score("rst", "Ripple split"));
        assert_eq!(score("rst", "Ripple split"), Some(1));
        assert!(score("分割", "在播放头处分割").is_some());
        assert_eq!(score("xyz", "Split"), None);
    }
}
