//! Drafts: the projects of an editor with no file and no server. Each is kept in the app's storage
//! (a file in the app's data folder on the desktop, the page's local storage in the browser) with
//! a line about it in an index, and a small picture of its cover. The drafts page lists them, to
//! open one, start a new one, or rename, copy or delete one.
//!
//! The page decides nothing: it reports an [`Ask`], and the editor, which holds the storage and
//! the session, does it.

use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, vec2};
use reelforge_model::Project;
use reelforge_time::Tick;
use serde::{Deserialize, Serialize};

use crate::i18n::{Strings, fill};
use crate::theme::{self, color, icon};

/// Where the index of the drafts is kept.
pub(crate) const INDEX_KEY: &str = "reelforge.drafts";
/// The longest side of a draft's picture, in pixels.
pub(crate) const COVER_SIDE: u32 = 320;
/// The narrowest a draft's card is drawn.
const CARD_MIN: f32 = 168.0;

/// Where the project of draft `id` is kept.
pub(crate) fn project_key(id: &str) -> String {
    format!("reelforge.draft.{id}")
}

fn cover_key(id: &str) -> String {
    format!("reelforge.draft.{id}.cover")
}

/// Where drafts are kept: eframe's storage in the app, a map in tests.
pub(crate) trait Store {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: String);
    fn remove(&mut self, key: &str);
}

impl Store for HashMap<String, String> {
    fn get(&self, key: &str) -> Option<String> {
        HashMap::get(self, key).cloned()
    }

    fn set(&mut self, key: &str, value: String) {
        self.insert(key.to_string(), value);
    }

    fn remove(&mut self, key: &str) {
        HashMap::remove(self, key);
    }
}

/// eframe's storage as a [`Store`].
pub(crate) struct Kept<'a>(pub &'a mut dyn eframe::Storage);

impl Store for Kept<'_> {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get_string(key)
    }

    fn set(&mut self, key: &str, value: String) {
        self.0.set_string(key, value);
    }

    fn remove(&mut self, key: &str) {
        self.0.remove_string(key);
    }
}

/// What the index says of one draft: enough to draw its card without reading the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Draft {
    pub id: String,
    pub name: String,
    /// When it last changed, in seconds since 1970.
    pub modified: u64,
    pub duration: Tick,
    /// The canvas, `[width, height]`.
    pub size: [u32; 2],
    /// How many clips are on its timeline.
    pub clips: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Index {
    /// The draft being edited.
    current: String,
    drafts: Vec<Draft>,
    /// The number the next draft's id gets.
    next: u64,
}

/// What the person asked of the drafts page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ask {
    /// Show the drafts page.
    Home,
    /// Edit this draft.
    Open(String),
    /// Start an empty one and edit it.
    New,
    /// Start one from this built-in timeline template.
    Template(&'static str),
    Rename(String, String),
    Duplicate(String),
    Delete(String),
    /// Back to the draft being edited.
    Back,
}

/// The drafts, and the page that shows them.
#[derive(Default)]
pub(crate) struct Drafts {
    index: Index,
    /// Whether the page shows instead of the editor.
    pub home: bool,
    /// The cards' pictures, each with the `modified` it was read at; `None` for a draft with none.
    covers: HashMap<String, (u64, Option<egui::TextureHandle>)>,
    /// A draft's name being typed: its id, the words so far, and whether the field has been
    /// given the keyboard.
    naming: Option<(String, String, bool)>,
    /// The draft whose card asks whether to delete it.
    deleting: Option<String>,
}

/// Seconds since 1970, by the machine's clock.
pub(crate) fn now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long ago `then` was, in words.
fn ago(s: &Strings, now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..60 => s.ago_now.to_string(),
        60..3600 => fill(s.ago_minutes, &[&(secs / 60)]),
        3600..86_400 => fill(s.ago_hours, &[&(secs / 3600)]),
        _ => fill(s.ago_days, &[&(secs / 86_400)]),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    let digit = |c: u8| char::from(c).to_digit(16);
    if !rest.is_empty() {
        return None;
    }
    pairs.iter().map(|[high, low]| Some((digit(*high)? * 16 + digit(*low)?) as u8)).collect()
}

impl Drafts {
    /// The drafts storage holds: `index` is what it has under [`INDEX_KEY`].
    pub(crate) fn load(index: Option<String>) -> Drafts {
        let index = index.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        Drafts { index, ..Drafts::default() }
    }

    /// The name of draft `id` (nothing when there is no such draft).
    pub(crate) fn name_of(&self, id: &str) -> String {
        self.index.drafts.iter().find(|d| d.id == id).map(|d| d.name.clone()).unwrap_or_default()
    }

    /// Whether the draft being edited has a picture.
    pub(crate) fn has_cover(&self, store: &dyn Store) -> bool {
        store.get(&cover_key(&self.index.current)).is_some()
    }

    /// The draft being edited, if the index knows one.
    pub(crate) fn current(&self) -> Option<&Draft> {
        self.index.drafts.iter().find(|d| d.id == self.index.current)
    }

    /// Every draft, the one changed last first.
    pub(crate) fn list(&self) -> Vec<&Draft> {
        let mut all: Vec<&Draft> = self.index.drafts.iter().collect();
        all.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.id.cmp(&a.id)));
        all
    }

    /// The project of draft `id`, when storage holds one that is whole.
    pub(crate) fn project(&self, store: &dyn Store, id: &str) -> Option<Project> {
        let project: Project = serde_json::from_str(&store.get(&project_key(id))?).ok()?;
        project.check().is_ok().then_some(project)
    }

    /// A name no draft has yet: `template` filled with the first free number.
    pub(crate) fn free_name(&self, template: &str) -> String {
        (self.index.drafts.len() + 1..)
            .map(|n| fill(template, &[&n]))
            .find(|name| self.index.drafts.iter().all(|d| &d.name != name))
            .expect("some number is free")
    }

    /// Starts a draft called `name` and makes it the one being edited. Its project is written by
    /// the next [`Drafts::keep`].
    pub(crate) fn start(&mut self, name: String, now: u64) -> String {
        self.index.next += 1;
        let id = format!("d{}", self.index.next);
        self.index.drafts.push(Draft {
            id: id.clone(),
            name,
            modified: now,
            duration: Tick::ZERO,
            size: [0, 0],
            clips: 0,
        });
        self.index.current = id.clone();
        id
    }

    fn write_index(&self, store: &mut dyn Store) {
        store.set(INDEX_KEY, serde_json::to_string(&self.index).expect("serialises"));
    }

    /// Keeps `project` as the draft being edited. The draft counts as changed (at `now`) only when
    /// the project is not what storage already held.
    pub(crate) fn keep(&mut self, store: &mut dyn Store, project: &Project, now: u64) {
        let id = self.index.current.clone();
        let Some(draft) = self.index.drafts.iter_mut().find(|d| d.id == id) else { return };
        let text = serde_json::to_string(project).expect("serialises");
        if store.get(&project_key(&id)).as_deref() != Some(text.as_str()) {
            store.set(&project_key(&id), text);
            draft.modified = now;
        }
        draft.duration = project.duration();
        draft.size = [project.canvas.width, project.canvas.height];
        draft.clips = project.tracks.iter().map(|t| t.clips.len()).sum();
        self.write_index(store);
    }

    /// Keeps a picture (a JPEG) as the cover of the draft being edited; `None` when there is
    /// nothing to show.
    pub(crate) fn keep_cover(&mut self, store: &mut dyn Store, jpeg: Option<&[u8]>) {
        let id = self.index.current.clone();
        match jpeg {
            Some(jpeg) => store.set(&cover_key(&id), hex(jpeg)),
            None => store.remove(&cover_key(&id)),
        }
        self.covers.remove(&id);
    }

    /// Makes draft `id` the one being edited and gives its project, when storage holds it whole.
    pub(crate) fn open(&mut self, store: &mut dyn Store, id: &str) -> Option<Project> {
        let project = self.project(store, id)?;
        self.index.current = id.to_string();
        self.write_index(store);
        Some(project)
    }

    pub(crate) fn rename(&mut self, store: &mut dyn Store, id: &str, name: &str) {
        let name = name.trim();
        if let Some(d) = self.index.drafts.iter_mut().find(|d| d.id == id)
            && !name.is_empty()
        {
            d.name = name.to_string();
            self.write_index(store);
        }
    }

    /// A copy of draft `id` called `name`, beside it. The copy is not opened.
    pub(crate) fn duplicate(&mut self, store: &mut dyn Store, id: &str, name: String, now: u64) -> Option<String> {
        let from = self.index.drafts.iter().find(|d| d.id == id)?.clone();
        let project = store.get(&project_key(id))?;
        self.index.next += 1;
        let copy = format!("d{}", self.index.next);
        store.set(&project_key(&copy), project);
        if let Some(cover) = store.get(&cover_key(id)) {
            store.set(&cover_key(&copy), cover);
        }
        self.index.drafts.push(Draft { id: copy.clone(), name, modified: now, ..from });
        self.write_index(store);
        Some(copy)
    }

    /// Deletes draft `id` for good. When it was the one being edited, no draft is until the editor
    /// opens or starts another.
    pub(crate) fn delete(&mut self, store: &mut dyn Store, id: &str) {
        self.index.drafts.retain(|d| d.id != id);
        store.remove(&project_key(id));
        store.remove(&cover_key(id));
        self.covers.remove(id);
        if self.index.current == id {
            self.index.current.clear();
        }
        self.write_index(store);
    }

    /// The picture of `draft`, read from `store` once per change.
    fn cover(&mut self, ctx: &egui::Context, store: &dyn Store, draft: &Draft) -> Option<egui::TextureHandle> {
        if let Some((at, texture)) = self.covers.get(&draft.id)
            && *at == draft.modified
        {
            return texture.clone();
        }
        let texture = store
            .get(&cover_key(&draft.id))
            .and_then(|text| unhex(&text))
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|picture| {
                let rgba = picture.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                ctx.load_texture(format!("draft-{}", draft.id), image, egui::TextureOptions::LINEAR)
            });
        self.covers.insert(draft.id.clone(), (draft.modified, texture.clone()));
        texture
    }

    /// The drafts page. `now` is the time, to say how long ago each draft changed.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, s: &Strings, store: &dyn Store, now: u64) -> Option<Ask> {
        let mut ask = None;
        // Escape leaves the page, unless it is leaving a name or a question
        if self.naming.is_none()
            && self.deleting.is_none()
            && self.current().is_some()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            ask = Some(Ask::Back);
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(s.drafts).size(22.0).strong());
            ui.add_space(8.0);
            ui.label(egui::RichText::new(self.index.drafts.len().to_string()).color(color::WEAK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary(ui, icon::ADD, s.draft_new, true).clicked() {
                    ask = Some(Ask::New);
                }
                if self.current().is_some() && ui.button(s.draft_back).clicked() {
                    ask = Some(Ask::Back);
                }
            });
        });
        ui.label(egui::RichText::new(s.drafts_note).small().color(color::WEAK));
        ui.add_space(12.0);
        let all: Vec<Draft> = self.list().into_iter().cloned().collect();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = 14.0;
            let room = ui.available_width();
            let columns = ((room + gap) / (CARD_MIN + gap)).floor().clamp(1.0, 6.0) as usize;
            let w = ((room - gap * (columns - 1) as f32) / columns as f32).floor().min(260.0);
            // the built-in timeline templates: each starts a draft with its slots to fill
            ui.label(egui::RichText::new(s.remix_start).strong().color(color::WEAK));
            ui.add_space(6.0);
            let templates: Vec<_> = reelforge_engine::timeline_templates().iter().enumerate().collect();
            for row in templates.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (i, t) in row {
                        if crate::remix::card(ui, *i, t, s, w).clicked() {
                            ask = Some(Ask::Template(t.id));
                        }
                    }
                });
                ui.add_space(gap);
            }
            ui.add_space(4.0);
            for row in all.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for draft in row {
                        if let Some(a) = self.card(ui, s, store, draft, w, now) {
                            ask = Some(a);
                        }
                    }
                });
                ui.add_space(gap);
            }
        });
        ask
    }

    /// One draft: its picture, its name, how long it is and when it changed.
    fn card(&mut self, ui: &mut egui::Ui, s: &Strings, store: &dyn Store, d: &Draft, w: f32, now: u64) -> Option<Ask> {
        let mut ask = None;
        let current = d.id == self.index.current;
        let thumb_h = (w * 0.75).round();
        ui.vertical(|ui| {
            ui.set_width(w);
            let (thumb, r) = ui.allocate_exact_size(vec2(w, thumb_h), Sense::click());
            let painter = ui.painter_at(thumb.expand(2.0));
            painter.rect_filled(thumb, 14.0, color::ROW);
            match self.cover(ui.ctx(), store, d) {
                Some(t) => {
                    // the whole picture, whatever its shape, in the middle of the card
                    let size = t.size_vec2();
                    let scale = (thumb.width() / size.x).min(thumb.height() / size.y);
                    let at = Rect::from_center_size(thumb.center(), size * scale);
                    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.add(
                        egui::epaint::RectShape::filled(at, CornerRadius::same(10), Color32::WHITE)
                            .with_texture(t.id(), uv),
                    );
                }
                None => {
                    painter.text(
                        thumb.center(),
                        Align2::CENTER_CENTER,
                        icon::FILM,
                        FontId::proportional(30.0),
                        color::WEAK,
                    );
                }
            }
            let stroke = if current {
                Stroke::new(2.0, color::ACCENT)
            } else if r.hovered() {
                Stroke::new(1.0, color::WEAK)
            } else {
                Stroke::new(1.0, Color32::from_white_alpha(18))
            };
            painter.rect_stroke(thumb, 14.0, stroke, StrokeKind::Inside);
            if current {
                let label =
                    painter.layout_no_wrap(s.draft_current.to_string(), FontId::proportional(10.5), color::ON_ACCENT);
                let pad = vec2(6.0, 3.0);
                let tag = Rect::from_min_size(thumb.left_top() + vec2(8.0, 8.0), label.size() + pad * 2.0);
                painter.rect_filled(tag, 8.0, color::ACCENT);
                painter.galley(tag.min + pad, label, color::ON_ACCENT);
            }
            if d.duration > Tick::ZERO {
                let text = theme::timecode(d.duration.seconds());
                let label = painter.layout_no_wrap(text, FontId::monospace(10.5), color::TEXT);
                let pad = vec2(5.0, 2.0);
                let tag = Rect::from_min_size(
                    thumb.right_bottom() - label.size() - pad * 2.0 - vec2(8.0, 8.0),
                    label.size() + pad * 2.0,
                );
                painter.rect_filled(tag, 8.0, Color32::from_black_alpha(170));
                painter.galley(tag.min + pad, label, color::TEXT);
            }
            if r.clicked() {
                ask = Some(if current { Ask::Back } else { Ask::Open(d.id.clone()) });
            }
            r.context_menu(|ui| self.menu(ui, s, d, &mut ask));

            ui.add_space(4.0);
            // the name, or the field it is typed in
            let mut named = None;
            match &mut self.naming {
                Some((id, text, focused)) if *id == d.id => {
                    let field = ui.add(egui::TextEdit::singleline(text).char_limit(60).desired_width(w));
                    if !*focused {
                        field.request_focus();
                        *focused = true;
                    }
                    if field.lost_focus() {
                        // Escape leaves the name as it was; anything else takes what was typed
                        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        named = Some((!escaped && !text.trim().is_empty()).then(|| text.trim().to_string()));
                    }
                }
                _ => {
                    ui.horizontal(|ui| {
                        let mut job = egui::text::LayoutJob::single_section(
                            d.name.clone(),
                            egui::TextFormat::simple(FontId::proportional(13.0), color::TEXT),
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(w - 30.0);
                        let name = ui.add(egui::Label::new(job).sense(Sense::click()));
                        if name.double_clicked() {
                            self.naming = Some((d.id.clone(), d.name.clone(), false));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button(egui::RichText::new("···").strong().color(color::WEAK), |ui| {
                                self.menu(ui, s, d, &mut ask)
                            });
                        });
                    });
                }
            }
            if let Some(name) = named {
                self.naming = None;
                if let Some(name) = name.filter(|n| *n != d.name) {
                    ask = Some(Ask::Rename(d.id.clone(), name));
                }
            }
            if self.deleting.as_deref() == Some(d.id.as_str()) {
                ui.horizontal(|ui| {
                    let sure = egui::RichText::new(format!("{}  {}", icon::DELETE, s.draft_delete_sure));
                    let sure = egui::Button::new(sure.small().color(Color32::WHITE))
                        .fill(color::PLAYHEAD.gamma_multiply(0.75));
                    if ui.add(sure).clicked() {
                        ask = Some(Ask::Delete(d.id.clone()));
                        self.deleting = None;
                    }
                    if ui.add(egui::Button::new(egui::RichText::new(s.cancel).small())).clicked() {
                        self.deleting = None;
                    }
                });
            } else {
                let what =
                    if d.clips == 0 { s.draft_empty.to_string() } else { format!("{}×{}", d.size[0], d.size[1]) };
                let facts = format!("{what} · {}", ago(s, now, d.modified));
                ui.label(egui::RichText::new(facts).small().color(color::WEAK));
            }
        });
        ask
    }

    /// What can be done with draft `d`, in its menus.
    fn menu(&mut self, ui: &mut egui::Ui, s: &Strings, d: &Draft, ask: &mut Option<Ask>) {
        if ui.button(s.media_rename).clicked() {
            self.naming = Some((d.id.clone(), d.name.clone(), false));
            self.deleting = None;
            ui.close();
        }
        if ui.button(s.draft_duplicate).clicked() {
            *ask = Some(Ask::Duplicate(d.id.clone()));
            self.deleting = None;
            ui.close();
        }
        ui.separator();
        // deleting cannot be undone: the card asks once more
        if ui.button(format!("{}  {}", icon::DELETE, s.draft_delete)).clicked() {
            self.deleting = Some(d.id.clone());
            self.naming = None;
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(clips: usize) -> Project {
        let mut s = reelforge_engine::Session::default();
        let track = s.project().tracks[0].id.clone();
        for _ in 0..clips {
            s.execute("clip.add", &serde_json::json!({"track": track, "color": [1, 0, 0], "duration": {"seconds": 2}}))
                .expect("a clip");
        }
        (*s.project()).clone()
    }

    #[test]
    fn drafts_are_kept_opened_copied_and_deleted() {
        let mut store: HashMap<String, String> = HashMap::new();
        let mut drafts = Drafts::load(Store::get(&store, INDEX_KEY));
        assert!(drafts.current().is_none() && drafts.list().is_empty());

        // the first draft: started, then kept with what is on the timeline
        let a = drafts.start(drafts.free_name("Draft {}"), 100);
        drafts.keep(&mut store, &project(2), 100);
        let d = drafts.current().unwrap().clone();
        assert_eq!((d.name.as_str(), d.clips, d.duration, d.size), ("Draft 1", 2, Tick::SECOND * 4, [1080, 1920]));
        // kept again unchanged, it has not changed
        drafts.keep(&mut store, &project(2), 500);
        assert_eq!(drafts.current().unwrap().modified, 100);
        drafts.keep(&mut store, &project(3), 600);
        assert_eq!(drafts.current().unwrap().modified, 600);

        // a second one; the list has the one changed last first
        let b = drafts.start(drafts.free_name("Draft {}"), 700);
        drafts.keep(&mut store, &project(0), 700);
        assert_eq!(drafts.list().iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Draft 2", "Draft 1"]);
        assert_eq!(drafts.current().unwrap().id, b);

        // the next run reads the same drafts, and opens the first again
        let mut drafts = Drafts::load(Store::get(&store, INDEX_KEY));
        assert_eq!(drafts.current().unwrap().id, b);
        let back = drafts.open(&mut store, &a).unwrap();
        assert_eq!(back, project(3));
        assert_eq!(drafts.current().unwrap().id, a);
        assert!(drafts.open(&mut store, "d99").is_none());
        assert_eq!(drafts.current().unwrap().id, a, "a draft that cannot be opened changes nothing");

        // a name, a copy (not opened), and a picture
        drafts.rename(&mut store, &a, "  Beach cut ");
        drafts.rename(&mut store, &a, "   ");
        assert_eq!(drafts.current().unwrap().name, "Beach cut");
        drafts.keep_cover(&mut store, Some(&[0xff, 0xd8, 0x01]));
        let c = drafts.duplicate(&mut store, &a, "Beach cut copy".into(), 800).unwrap();
        assert_eq!(drafts.project(&store, &c), Some(project(3)));
        assert_eq!(store.get(&cover_key(&c)).map(String::as_str), Some("ffd801"));
        assert_eq!(unhex("ffd801"), Some(vec![0xff, 0xd8, 0x01]));
        assert_eq!((unhex("ffd"), unhex("zz")), (None, None));
        assert_eq!(drafts.current().unwrap().id, a);
        assert_eq!(drafts.free_name("Draft {}"), "Draft 4");

        // deleting the one being edited leaves none being edited, and nothing of it in storage
        drafts.delete(&mut store, &a);
        assert!(drafts.current().is_none());
        assert!(drafts.project(&store, &a).is_none());
        assert_eq!(drafts.list().len(), 2);
        assert_eq!(Drafts::load(Store::get(&store, INDEX_KEY)).list().len(), 2);
    }

    #[test]
    fn how_long_ago_is_said_in_the_largest_unit() {
        let s = crate::i18n::Lang::En.strings();
        assert_eq!(ago(s, 1000, 990), s.ago_now);
        assert_eq!(ago(s, 1000, 2000), s.ago_now, "a clock set back is not a time in the future");
        assert!(ago(s, 10_000, 10_000 - 5 * 60).starts_with('5'));
        assert!(ago(s, 100_000, 100_000 - 3 * 3600).starts_with('3'));
        assert!(ago(s, 1_000_000, 1_000_000 - 2 * 86_400).starts_with('2'));
    }
}
