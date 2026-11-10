//! The captions tab of the left panel: every title of a text track as a list, to read through and
//! put right in one place. A row is a caption: its time and its words, edited where they stand.
//! Enter cuts a caption at the cursor, Backspace at its start joins it to the one before; the bar
//! above finds and replaces, moves them all in time, gives them all one caption's look, and
//! reads and writes SubRip files (a second file as a second language's line).
//!
//! Like the rest of the panel it changes nothing itself: it reports an [`Ask`] and the editor
//! runs the commands (`captions.*`).

use eframe::egui::{self, Align, Color32, FontId, Sense, Stroke, StrokeKind, vec2};
use reelforge_model::{ClipContent, Project, Track, TrackKind};
use reelforge_time::Tick;

use crate::i18n::{Lang, fill};
use crate::theme::{self, color, icon};

/// What the person asked of the list this frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ask {
    /// Show this caption: select it and go to where it starts.
    Focus { clip: String, start: Tick },
    /// New words for a caption.
    SetText { clip: String, text: String },
    /// Cut a caption after `at` characters of `text` (its words as typed, saved first if changed).
    Split { clip: String, text: String, at: usize },
    /// Join two neighbours; `typed` is the one of them being edited and its words as typed.
    Merge { first: String, second: String, typed: Option<(String, String)> },
    /// Take a caption away.
    Delete(String),
    /// Replace words in every caption of a track.
    Replace { track: String, find: String, with: String },
    /// Give every caption on its track this one's look.
    Restyle(String),
    /// Move every caption of a track in time.
    Shift { track: String, seconds: f64 },
    /// Pick a SubRip file; `second` as a second language.
    Import { second: bool },
    /// Save a track's captions as a SubRip file.
    Export(String),
}

/// The tab's state.
#[derive(Default)]
pub(crate) struct CaptionsTab {
    /// The text track shown, when the person chose one.
    track: Option<String>,
    /// The caption being typed in, and its words so far.
    editing: Option<(String, String)>,
    /// Where the cursor was in it last frame, in characters, when nothing is selected.
    cursor: Option<usize>,
    finding: bool,
    find: String,
    with: String,
    shift: f64,
    /// The caption under the playhead last frame, to scroll to the next one once.
    current: Option<String>,
}

/// The track the list shows by default: the last one named as captions tracks are, else the
/// last text track with anything on it, else the last text track.
fn default_track(p: &Project) -> Option<&Track> {
    let text = || p.tracks.iter().rev().filter(|t| t.kind == TrackKind::Text);
    text()
        .find(|t| t.name == reelforge_engine::CAPTIONS_TRACK)
        .or_else(|| text().find(|t| !t.clips.is_empty()))
        .or_else(|| text().next())
}

impl CaptionsTab {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        lang: Lang,
        playhead: Tick,
        selected: &[String],
        playing: bool,
    ) -> Option<Ask> {
        let s = lang.strings();
        let mut ask = None;
        let chosen = self.track.as_ref().and_then(|id| p.track(id)).filter(|t| t.kind == TrackKind::Text);
        let track = chosen.or_else(|| default_track(p));
        if chosen.is_none() {
            self.track = None;
        }
        ui.horizontal_wrapped(|ui| {
            if theme::tool(ui, icon::IMPORT, s.cap_import, s.cap_import_tip, true).clicked() {
                ask = Some(Ask::Import { second: false });
            }
            if theme::tool(ui, icon::TRANSLATE, s.cap_second, s.cap_second_tip, true).clicked() {
                ask = Some(Ask::Import { second: true });
            }
            let any = track.is_some_and(|t| !t.clips.is_empty());
            if theme::tool(ui, icon::SAVE, "", s.cap_export, any).clicked() {
                ask = track.map(|t| Ask::Export(t.id.clone()));
            }
            let model = track
                .filter(|t| t.clips.len() > 1)
                .and_then(|t| selected.iter().find(|id| t.clips.iter().any(|c| &c.id == *id)));
            if theme::tool(ui, icon::BRUSH, "", s.cap_match_tip, model.is_some()).clicked() {
                ask = model.map(|id| Ask::Restyle(id.clone()));
            }
            let find = ui
                .add_enabled(
                    any,
                    egui::Button::new(egui::RichText::new(icon::SEARCH).size(14.0)).selected(self.finding),
                )
                .on_hover_text(s.cap_find_tip);
            if find.clicked() {
                self.finding = !self.finding;
            }
        });
        let Some(track) = track else {
            empty(ui, s.cap_none);
            return ask;
        };
        let texts: Vec<&Track> = p.tracks.iter().filter(|t| t.kind == TrackKind::Text).collect();
        if texts.len() > 1 {
            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("captions-track")
                .width(ui.available_width() - 8.0)
                .selected_text(lang.track_name(&track.name))
                .show_ui(ui, |ui| {
                    for t in texts.iter().rev() {
                        let name = format!("{}  ·  {}", lang.track_name(&t.name), t.clips.len());
                        if ui.selectable_label(t.id == track.id, name).clicked() {
                            self.track = Some(t.id.clone());
                        }
                    }
                });
        }
        if self.finding {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let w = ((ui.available_width() - 8.0) / 2.0).max(60.0);
                ui.add(egui::TextEdit::singleline(&mut self.find).hint_text(s.cap_find).desired_width(w));
                ui.add(egui::TextEdit::singleline(&mut self.with).hint_text(s.cap_replace_with).desired_width(w));
            });
            ui.horizontal(|ui| {
                let hits = if self.find.is_empty() {
                    0
                } else {
                    track.clips.iter().filter_map(words).map(|w| w.matches(self.find.as_str()).count()).sum::<usize>()
                };
                if ui.add_enabled(hits > 0, egui::Button::new(s.cap_replace_all)).clicked() {
                    ask = Some(Ask::Replace {
                        track: track.id.clone(),
                        find: self.find.clone(),
                        with: self.with.clone(),
                    });
                }
                if !self.find.is_empty() {
                    ui.label(egui::RichText::new(hits.to_string()).color(color::WEAK));
                }
            });
        }
        let cues: Vec<(&str, Tick, Tick, &str)> =
            track.clips.iter().filter_map(|c| words(c).map(|w| (c.id.as_str(), c.start, c.end(), w))).collect();
        if cues.is_empty() {
            empty(ui, s.cap_none);
            return ask;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(fill(s.cap_count, &[&cues.len()])).small().color(color::WEAK))
                .on_hover_text(s.cap_keys_hint);
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if theme::tool(ui, icon::CLOCK, s.cap_shift, s.cap_shift_tip, self.shift != 0.0).clicked() {
                    ask = Some(Ask::Shift { track: track.id.clone(), seconds: self.shift });
                    self.shift = 0.0;
                }
                ui.add(egui::DragValue::new(&mut self.shift).speed(0.01).fixed_decimals(2).suffix(" s"))
                    .on_hover_text(s.cap_shift_tip);
            });
        });
        ui.add_space(2.0);

        // a caption that went (merged, removed, undone) is not being edited any more
        if self.editing.as_ref().is_some_and(|(id, _)| !cues.iter().any(|c| c.0 == id)) {
            self.editing = None;
        }
        let now = cues.iter().find(|c| c.1 <= playhead && playhead < c.2).map(|c| c.0.to_string());
        let moved_on = now != self.current && now.is_some() && self.editing.is_none();
        self.current = now.clone();
        let (enter, backspace) =
            ui.input(|i| (i.key_pressed(egui::Key::Enter) && !i.modifiers.shift, i.key_pressed(egui::Key::Backspace)));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            for (i, &(id, start, end, text)) in cues.iter().enumerate() {
                let here = now.as_deref() == Some(id);
                let picked = selected.iter().any(|c| c == id);
                let mine = self.editing.as_ref().is_some_and(|(c, _)| c == id);
                let mut buf = match &self.editing {
                    Some((c, typed)) if c == id => typed.clone(),
                    _ => text.to_string(),
                };
                let fill_ = if picked {
                    color::ACCENT.gamma_multiply(0.18)
                } else if here {
                    color::RAISED
                } else {
                    Color32::TRANSPARENT
                };
                let frame = egui::Frame::new()
                    .fill(fill_)
                    .stroke(Stroke::new(1.0, if picked { color::ACCENT } else { color::LINE.gamma_multiply(0.6) }))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(6, 4));
                let row = frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut asked = None;
                    let head = ui.horizontal(|ui| {
                        let times =
                            format!("{}  –  {}", theme::timecode(start.seconds()), theme::timecode(end.seconds()));
                        let tint = if here { color::ACCENT } else { color::WEAK };
                        let r = ui.add(
                            egui::Label::new(egui::RichText::new(times).font(FontId::monospace(10.5)).color(tint))
                                .sense(Sense::click()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let small = |ui: &mut egui::Ui, glyph: &str, tip: &str, on: bool| {
                                ui.add_enabled(
                                    on,
                                    egui::Button::new(egui::RichText::new(glyph).size(12.0)).frame(false).small(),
                                )
                                .on_hover_text(tip)
                                .clicked()
                            };
                            if small(ui, icon::DELETE, lang.command("clip.remove"), true) {
                                asked = Some(Ask::Delete(id.to_string()));
                            }
                            if small(ui, icon::MERGE, s.cap_merge_tip, i + 1 < cues.len()) {
                                let typed = self.editing.clone().filter(|(c, _)| c == id);
                                asked = cues.get(i + 1).map(|n| Ask::Merge {
                                    first: id.to_string(),
                                    second: n.0.to_string(),
                                    typed,
                                });
                            }
                            let at = self.cursor.filter(|_| mine);
                            if small(ui, icon::SPLIT, s.cap_split_tip, buf.chars().count() > 1) {
                                let n = buf.chars().count();
                                let at = at.filter(|a| *a > 0 && *a < n).unwrap_or(n / 2);
                                asked = Some(Ask::Split { clip: id.to_string(), text: buf.clone(), at });
                            }
                        });
                        r
                    });
                    if head.inner.clicked() {
                        asked = Some(Ask::Focus { clip: id.to_string(), start });
                    }
                    let before = buf.clone();
                    let out = egui::TextEdit::multiline(&mut buf)
                        .id_salt(("caption", id))
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE)
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
                        .show(ui);
                    let r = &out.response;
                    let caret = out.cursor_range.filter(|c| c.is_empty()).map(|c| c.primary.index.0);
                    if r.gained_focus() {
                        asked = Some(Ask::Focus { clip: id.to_string(), start });
                    }
                    if r.has_focus() {
                        let n = buf.chars().count();
                        match (enter, backspace, self.cursor.filter(|_| mine)) {
                            (true, _, Some(at)) if at > 0 && at < n => {
                                asked = Some(Ask::Split { clip: id.to_string(), text: buf.clone(), at });
                            }
                            (_, true, Some(0)) if i > 0 && caret == Some(0) && buf == before => {
                                asked = Some(Ask::Merge {
                                    first: cues[i - 1].0.to_string(),
                                    second: id.to_string(),
                                    typed: Some((id.to_string(), buf.clone())),
                                });
                            }
                            _ => {}
                        }
                        self.cursor = caret;
                        self.editing = Some((id.to_string(), buf.clone()));
                    } else if r.lost_focus() || mine {
                        // done typing: the words are kept if they changed and there are any
                        self.editing = None;
                        if asked.is_none() && buf != text && !buf.trim().is_empty() {
                            asked = Some(Ask::SetText {
                                clip: id.to_string(),
                                text: buf.trim_end_matches('\n').to_string(),
                            });
                        }
                    }
                    asked
                });
                if here {
                    let mark = egui::Rect::from_min_size(row.response.rect.min, vec2(2.0, row.response.rect.height()));
                    ui.painter().rect_filled(mark, 1.0, color::ACCENT);
                    if moved_on && (playing || !ui.rect_contains_pointer(ui.clip_rect())) {
                        row.response.scroll_to_me(Some(Align::Center));
                    }
                }
                if picked {
                    ui.painter().rect_stroke(
                        row.response.rect,
                        6.0,
                        Stroke::new(1.0, color::ACCENT),
                        StrokeKind::Inside,
                    );
                }
                if row.inner.is_some() {
                    ask = row.inner;
                }
            }
        });
        if matches!(ask, Some(Ask::Split { .. } | Ask::Merge { .. } | Ask::Delete(_))) {
            // the rows change under the cursor: start afresh
            self.editing = None;
            self.cursor = None;
        }
        ask
    }
}

/// A title's words.
fn words(c: &reelforge_model::Clip) -> Option<&str> {
    match &c.content {
        ClipContent::Text { text, .. } => Some(text.as_str()),
        _ => None,
    }
}

fn empty(ui: &mut egui::Ui, hint: &str) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(icon::CAPTIONS).size(28.0).color(color::LINE));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(hint).color(color::WEAK));
    });
}
