//! The panel on the left: what can go on the timeline. Six tabs, as short-video editors have them:
//! the project's media (imported files, with a picture, their length, whether a clip uses them and
//! whether they are offline), the built-in sound library, titles (plain or wearing a template),
//! the built-in stickers, picture effects (each lands as an adjustment clip over the tracks below),
//! and the captions as a list to edit ([`crate::captions`]).
//!
//! Anything in the panel is dragged onto the timeline (an [`Item`] is the drag's payload; the
//! timeline draws where it would land and reports the drop) or added at the playhead. Like the rest
//! of the interface the panel decides nothing: it reports an [`Action`] and the editor runs the
//! commands.
//!
//! Pictures of the media are made on the editor's thread, one or two per frame and none while
//! playing, like the timeline's.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use reelforge_media::Footage;
use reelforge_media::decor::DECOR;
use reelforge_media::library::{Category, SOUNDS};
use reelforge_media::stickers::STICKERS;
use reelforge_model::{FxGroup, FxKind, Media, MediaKind, Project, TrackKind};
use reelforge_time::Tick;
use web_time::{Duration, Instant};

use crate::i18n::{Strings, fill};
use crate::theme::{self, color, icon};

/// How long a title or a still lasts when it is dropped (the engine's own default for stills is
/// used when it is placed; this is only the drop preview's length).
const STILL_PREVIEW: Tick = Tick(5 * reelforge_time::TICKS_PER_SECOND);
/// A new title's length.
pub(crate) const TITLE_LENGTH: Tick = Tick(3 * reelforge_time::TICKS_PER_SECOND);
/// A new sticker's or effect's length.
pub(crate) const OVERLAY_LENGTH: Tick = Tick(3 * reelforge_time::TICKS_PER_SECOND);
/// Time spent making pictures per frame.
const BUDGET: Duration = Duration::from_millis(8);
/// The narrowest a media card is drawn.
const CARD_MIN: f32 = 104.0;
/// How wide the tabs down the panel's side are, and how tall a panel has room for them.
const RAIL: f32 = 58.0;
const RAIL_ROOM: f32 = 7.0 * 54.0;

/// The tabs, in the order shown: the kinds of thing there are to add.
fn tab_list(s: &Strings) -> [(Tab, &'static str, &'static str); 7] {
    [
        (Tab::Media, icon::IMAGE, s.tab_media),
        (Tab::Audio, icon::MUSIC, s.tab_audio),
        (Tab::Text, icon::TEXT, s.tab_text),
        (Tab::Stickers, icon::STICKER, s.tab_stickers),
        (Tab::Effects, icon::SPARKLE, s.tab_effects),
        (Tab::Captions, icon::CAPTIONS, s.tab_captions),
        (Tab::Templates, icon::FILM, s.tab_templates),
    ]
}

/// Something dragged from the panel onto the timeline, or added at the playhead.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    /// A file of the project's media, by id.
    Media(String),
    /// A sound of the built-in library: its id, its shown name and its length.
    Sound { id: &'static str, name: &'static str, length: Tick },
    /// A title, plain or wearing a template.
    Title(Option<&'static str>),
    /// A sticker of the built-in library, by id.
    Sticker(&'static str),
    /// A picture effect on an adjustment clip of its own; `None` is an empty adjustment clip.
    Effect(Option<FxKind>),
    /// A drawn shape.
    Shape(reelforge_model::ShapeKind),
    /// A ready-made promotional piece (`promo.add`), by id.
    Promo(&'static str),
    /// An animated decoration of the built-in library (`decor.add`), by id.
    Decor(&'static str),
}

/// A shape's shown name.
fn shape_name(kind: reelforge_model::ShapeKind, s: &Strings) -> &'static str {
    s.shape_names[reelforge_model::ShapeKind::ALL.iter().position(|k| *k == kind).unwrap_or(0)]
}

/// A promotional piece's shown name.
fn promo_name(id: &str, s: &Strings) -> &'static str {
    reelforge_engine::PROMOS.iter().position(|p| p.id == id).map_or("", |i| s.promo_names[i])
}

/// A picture effect's shown name.
pub(crate) fn fx_name(kind: FxKind, s: &Strings) -> &'static str {
    s.fx_names[FxKind::ALL.iter().position(|k| *k == kind).unwrap_or(0)]
}

/// A decoration's shown name.
fn decor_name(id: &str, s: &Strings) -> &'static str {
    DECOR.iter().position(|d| d.id == id).map_or("", |i| s.decor_names[i])
}

/// A sticker's shown name.
fn sticker_name(id: &str, s: &Strings) -> &'static str {
    STICKERS.iter().position(|st| st.id == id).map_or("", |i| s.sticker_names[i])
}

impl Item {
    /// The kind of track it goes on.
    pub(crate) fn track_kind(&self, p: &Project) -> TrackKind {
        match self {
            Item::Media(id) if p.media.get(id).is_some_and(|m| m.kind == MediaKind::Audio) => TrackKind::Audio,
            Item::Media(_) => TrackKind::Video,
            Item::Sound { .. } => TrackKind::Audio,
            Item::Title(_) => TrackKind::Text,
            Item::Sticker(_) | Item::Effect(_) | Item::Shape(_) | Item::Promo(_) | Item::Decor(_) => TrackKind::Video,
        }
    }

    /// How long it lasts once placed, as far as the drop preview needs to know.
    pub(crate) fn length(&self, p: &Project) -> Tick {
        match self {
            Item::Media(id) => p.media.get(id).and_then(|m| m.duration).unwrap_or(STILL_PREVIEW),
            Item::Sound { length, .. } => *length,
            Item::Title(_) => TITLE_LENGTH,
            Item::Sticker(_) | Item::Effect(_) | Item::Shape(_) | Item::Promo(_) => OVERLAY_LENGTH,
            Item::Decor(id) => {
                Tick::from_seconds(f64::from(reelforge_media::decor::find(id).map_or(3.0, |d| d.seconds)))
            }
        }
    }

    /// What the drop preview says.
    pub(crate) fn label(&self, p: &Project, s: &Strings) -> String {
        match self {
            Item::Media(id) => p.media.get(id).map_or_else(String::new, |m| m.name.clone()),
            Item::Sound { name, .. } => (*name).to_string(),
            Item::Title(None) => s.text_default.to_string(),
            Item::Title(Some(t)) => template_name(t, s).to_string(),
            Item::Sticker(id) => sticker_name(id, s).to_string(),
            Item::Effect(None) => s.fx_adjustment.to_string(),
            Item::Effect(Some(k)) => fx_name(*k, s).to_string(),
            Item::Shape(k) => shape_name(*k, s).to_string(),
            Item::Promo(id) => promo_name(id, s).to_string(),
            Item::Decor(id) => decor_name(id, s).to_string(),
        }
    }
}

/// What the person asked of the panel this frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Pick files to add to the project's media.
    Import,
    /// Put this at the playhead.
    Add(Item),
    /// Take a file out of the project's media.
    Remove(String),
    /// Call a file of the project's media this.
    Rename(String, String),
    /// File these under a folder of the panel, or take them out of theirs.
    File(Vec<String>, Option<String>),
    /// Something asked of the captions list.
    Captions(crate::captions::Ask),
    /// Use this built-in timeline template.
    Template(&'static str),
    /// Pick files to put in the empty slots of the timeline's template.
    Fill,
    /// Use this built-in storyboard.
    Storyboard(&'static str),
    /// Give a field of the project's copy these words.
    Copy(String, String),
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Media,
    Audio,
    Text,
    Stickers,
    Effects,
    Captions,
    Templates,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Video,
    Image,
    Audio,
}

impl Filter {
    fn admits(self, kind: MediaKind) -> bool {
        match self {
            Filter::All => !matches!(kind, MediaKind::Lut | MediaKind::Font),
            Filter::Video => kind.moves(),
            Filter::Image => kind == MediaKind::Image,
            Filter::Audio => kind == MediaKind::Audio,
        }
    }
}

/// The panel's state.
#[derive(Default)]
pub(crate) struct AssetPanel {
    tab: Tab,
    filter: Filter,
    /// The media card last clicked.
    selected: Option<String>,
    /// What the tab's search box holds: only what has every word of it in its name is shown.
    query: String,
    /// The folder whose media is shown; all of it otherwise.
    folder: Option<String>,
    /// A name being typed.
    naming: Option<Naming>,
    /// Pictures of the media by [`thumb_key`]; `None` for one that could not be made.
    pictures: HashMap<String, Option<egui::TextureHandle>>,
    /// Files that were not there at [`Footage::generation`] `seen`.
    missing: HashSet<String>,
    seen: u64,
    /// The library sounds' lengths, measured the first time the sounds are shown.
    sound_lengths: HashMap<&'static str, Tick>,
    /// The stickers' pictures, drawn the first time the stickers are shown.
    stickers: Vec<egui::TextureHandle>,
    /// The decorations, read and drawn the first time the stickers are shown: each one's picture
    /// and the animation it is of.
    decor: Vec<(egui::TextureHandle, reelforge_media::decor::Preview)>,
    /// The decoration playing under the pointer: which, and when (egui's time) it began.
    decor_playing: Option<(usize, f64)>,
    /// Which part of the stickers tab shows: the stickers and shapes, the animated decorations
    /// or the ready-made pieces.
    stuck: usize,
    captions: crate::captions::CaptionsTab,
    /// Whether the templates tab shows the storyboards, not the timeline templates.
    boards: bool,
    /// The project's copy as it is typed over there.
    copy: crate::storyboard::CopyForm,
}

/// A name being typed in the media tab.
#[derive(Clone, Debug, PartialEq)]
enum Naming {
    /// A file's new name.
    Rename { media: String, text: String, focused: bool },
    /// A new folder for a file.
    Folder { media: String, text: String, focused: bool },
}

/// Whether `name` has every word of `query` in it, whatever the case. An empty query admits all.
pub(crate) fn finds(query: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    query.split_whitespace().all(|word| name.contains(&word.to_lowercase()))
}

/// The search box at the top of a tab.
fn search_box(ui: &mut egui::Ui, query: &mut String, s: &Strings) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| search_field(ui, query, s));
    });
    ui.add_space(6.0);
}

/// A field to search in, filling what is left of a row laid out from its right.
fn search_field(ui: &mut egui::Ui, query: &mut String, s: &Strings) {
    if !query.is_empty() && ui.add(egui::Button::new("×").small().frame(false)).clicked() {
        query.clear();
    }
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(egui::RichText::new(format!("{}  {}", icon::SEARCH, s.search_hint)).color(color::WEAK))
            .margin(egui::Margin::symmetric(10, 5))
            .desired_width(ui.available_width()),
    );
}

/// What a tab says when the search leaves nothing.
fn nothing_matches(ui: &mut egui::Ui, s: &Strings) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| ui.label(egui::RichText::new(s.search_none).color(color::WEAK)));
}

/// A picture is made once per file (a relinked file gets a new one).
fn thumb_key(m: &Media) -> String {
    format!("{}:{}", m.id, m.sha256.as_deref().unwrap_or(&m.uri))
}

/// A template's shown name.
fn template_name(id: &str, s: &Strings) -> &'static str {
    reelforge_engine::title_templates().iter().position(|t| t.id == id).map_or("", |i| s.template_names[i])
}

/// The length of a library sound, from its file.
fn sound_length(id: &str) -> Tick {
    let frames = reelforge_media::library::wav(id).map_or(0, |b| b.len().saturating_sub(44) / 4);
    Tick::from_seconds(frames as f64 / f64::from(reelforge_media::library::RATE))
}

/// Whether `m` cannot be read: never found (an offline file from a CapCut draft) or gone since.
pub(crate) fn offline(m: &Media, missing: &HashSet<String>) -> bool {
    m.sha256.is_none() && !m.uri.starts_with(reelforge_media::library::SCHEME) || missing.contains(&thumb_key(m))
}

impl AssetPanel {
    /// Whether some file of `p` is known not to be readable.
    pub(crate) fn any_offline(&self, p: &Project) -> bool {
        p.media.values().any(|m| offline(m, &self.missing))
    }

    /// Draws the panel. `looks` are the pictures of the title templates (made by the caller when
    /// the text tab shows; see [`AssetPanel::wants_looks`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        media: &Footage,
        lang: crate::i18n::Lang,
        looks: &[crate::looks::Look],
        playing: bool,
        playhead: Tick,
        selected: &[String],
    ) -> Option<Action> {
        let s = lang.strings();
        let mut action = None;
        // what the chosen tab holds
        let body = |panel: &mut AssetPanel, ui: &mut egui::Ui, action: &mut Option<Action>| match panel.tab {
            Tab::Media => panel.media_tab(ui, p, media, s, playing, action),
            Tab::Audio => panel.audio_tab(ui, s, action),
            Tab::Text => text_tab(ui, s, looks, action),
            Tab::Stickers => panel.stickers_tab(ui, s, action),
            Tab::Effects => effects_tab(ui, &mut panel.query, s, action),
            Tab::Templates => panel.templates_tab(ui, p, lang, action),
            Tab::Captions => {
                if let Some(ask) = panel.captions.show(ui, p, lang, playhead, selected, playing) {
                    *action = Some(Action::Captions(ask));
                }
            }
        };
        // a tall panel has its tabs down its side, where each has room for its word; a low one (the
        // sheet of a narrow window) has them along its top
        if ui.available_height() >= RAIL_ROOM {
            let whole = ui.available_rect_before_wrap();
            let side = whole.with_max_x(whole.left() + RAIL);
            ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| self.rail(ui, s));
            let x = side.right() + 6.0;
            ui.painter().vline(x, whole.y_range(), Stroke::new(1.0, Color32::from_white_alpha(14)));
            let rest = whole.with_min_x(x + 9.0);
            ui.scope_builder(egui::UiBuilder::new().max_rect(rest), |ui| body(self, ui, &mut action));
        } else {
            self.tabs(ui, s);
            ui.add_space(6.0);
            body(self, ui, &mut action);
        }
        drag_preview(ui.ctx(), p, s);
        action
    }

    /// Shows the timeline templates: one was just used, and its slots are counted there.
    pub(crate) fn show_templates(&mut self) {
        self.tab = Tab::Templates;
        self.query.clear();
    }

    /// Shows the storyboards: one was just used, and its copy is to be typed over there.
    pub(crate) fn show_storyboards(&mut self) {
        self.show_templates();
        self.boards = true;
        self.copy.reset();
    }

    /// Whether the text tab is showing (and so needs the templates' pictures).
    pub(crate) fn wants_looks(&self) -> bool {
        self.tab == Tab::Text
    }

    /// Goes to a tab; each is searched for its own things.
    fn go(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.query.clear();
        }
    }

    /// One tab: its picture over its word, lit when it is the one showing.
    fn tab_button(&mut self, ui: &mut egui::Ui, size: egui::Vec2, (tab, glyph, word): (Tab, &str, &str)) {
        let on = self.tab == tab;
        let (rect, r) = ui.allocate_exact_size(size, Sense::click());
        let lit = ui.ctx().animate_bool_with_time(r.id, on, 0.12);
        let painter = ui.painter();
        if lit > 0.0 {
            painter.rect_filled(rect, 12.0, color::GLASS_HOVER.gamma_multiply(lit));
        } else if r.hovered() {
            painter.rect_filled(rect, 12.0, color::GLASS_CONTROL);
        }
        let (ink, word_ink) = if on {
            (color::ACCENT, color::TEXT)
        } else if r.hovered() {
            (color::TEXT, color::TEXT)
        } else {
            (color::WEAK, color::WEAK)
        };
        painter.text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, glyph, FontId::proportional(18.0), ink);
        // a long word is drawn smaller rather than over its neighbours
        let mut font = 10.5;
        while font > 8.0
            && painter.layout_no_wrap(word.to_string(), FontId::proportional(font), ink).size().x > rect.width() - 4.0
        {
            font -= 0.5;
        }
        painter.text(
            rect.center() + vec2(0.0, 13.0),
            Align2::CENTER_CENTER,
            word,
            FontId::proportional(font),
            word_ink,
        );
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.go(tab);
        }
    }

    /// The tabs down the panel's side.
    fn rail(&mut self, ui: &mut egui::Ui, s: &Strings) {
        ui.spacing_mut().item_spacing.y = 4.0;
        for tab in tab_list(s) {
            self.tab_button(ui, vec2(RAIL, 50.0), tab);
        }
    }

    /// The tabs along the panel's top.
    fn tabs(&mut self, ui: &mut egui::Ui, s: &Strings) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tabs = tab_list(s);
            // as wide as the panel lets them be
            let w = ((ui.available_width() - 2.0 * (tabs.len() - 1) as f32) / tabs.len() as f32).clamp(36.0, 64.0);
            for tab in tabs {
                self.tab_button(ui, vec2(w.floor(), 50.0), tab);
            }
        });
    }

    fn media_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        footage: &Footage,
        s: &Strings,
        playing: bool,
        action: &mut Option<Action>,
    ) {
        if footage.generation() != self.seen {
            self.seen = footage.generation();
            self.missing.clear();
            self.pictures.retain(|_, t| t.is_some());
        }
        // in the order they were imported (ids are minted in order: m2 before m10)
        let mut all: Vec<&Media> =
            p.media.values().filter(|m| !matches!(m.kind, MediaKind::Lut | MediaKind::Font)).collect();
        all.sort_by_key(|m| (m.id.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().ok(), &m.id));
        if all.is_empty() {
            // nothing yet: the whole tab asks for files
            empty(ui, s, action);
            return;
        }
        // what to find, and beside it the way to bring more in
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary(ui, icon::IMPORT, s.import, true).on_hover_text(s.import_tip).clicked() {
                    *action = Some(Action::Import);
                }
                search_field(ui, &mut self.query, s);
            });
        });
        ui.add_space(2.0);
        let filters = [
            (Filter::All, s.filter_all),
            (Filter::Video, s.filter_video),
            (Filter::Image, s.filter_image),
            (Filter::Audio, s.filter_audio),
        ];
        let at = filters.iter().position(|(f, _)| *f == self.filter).unwrap_or(0);
        if let Some(i) = theme::segmented(ui, &filters.map(|(_, word)| word), at) {
            self.filter = filters[i].0;
        }
        ui.add_space(6.0);
        // the folders there are, with how much is in each
        let mut folders: Vec<(&str, usize)> = Vec::new();
        for f in all.iter().filter_map(|m| m.folder.as_deref()) {
            match folders.iter_mut().find(|(name, _)| *name == f) {
                Some((_, n)) => *n += 1,
                None => folders.push((f, 1)),
            }
        }
        folders.sort();
        if self.folder.as_deref().is_some_and(|f| !folders.iter().any(|(name, _)| *name == f)) {
            self.folder = None;
        }
        if !folders.is_empty() {
            self.folders_row(ui, &folders, s, action);
        }
        self.naming_row(ui, s, action);
        let shown: Vec<&Media> = all
            .iter()
            .copied()
            .filter(|m| self.filter.admits(m.kind))
            .filter(|m| self.folder.is_none() || m.folder == self.folder)
            .filter(|m| finds(&self.query, &m.name))
            .collect();
        let folder_names: Vec<&str> = folders.iter().map(|(name, _)| *name).collect();
        let mut uses: HashMap<&str, usize> = HashMap::new();
        for m in &shown {
            uses.insert(&m.id, reelforge_engine::media_uses(p, &m.id));
        }
        if self.selected.as_ref().is_some_and(|id| !p.media.contains_key(id)) {
            self.selected = None;
        }
        let mut wanted: Vec<&Media> = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = 8.0;
            let room = ui.available_width();
            let columns = ((room + gap) / (CARD_MIN + gap)).floor().max(1.0) as usize;
            let w = ((room - gap * (columns - 1) as f32) / columns as f32).floor();
            for row in shown.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for m in row {
                        let used = uses.get(m.id.as_str()).copied().unwrap_or(0);
                        if let Some(a) = self.card(ui, m, w, used, s, &folder_names, &mut wanted) {
                            *action = Some(a);
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            if shown.is_empty() {
                let why = if self.query.trim().is_empty() { s.media_none_here } else { s.search_none };
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.label(egui::RichText::new(why).color(color::WEAK)));
            }
        });
        if !playing && self.make_pictures(ui.ctx(), footage, &wanted) {
            ui.ctx().request_repaint();
        }
    }

    /// The folders as a row of chips: a click shows one folder's media, and a card dropped on a
    /// chip is filed there.
    fn folders_row(&mut self, ui: &mut egui::Ui, folders: &[(&str, usize)], s: &Strings, action: &mut Option<Action>) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let chip = |ui: &mut egui::Ui, on: bool, label: String| {
                let text = egui::RichText::new(label).small().color(if on { color::TEXT } else { color::WEAK });
                ui.add(egui::Button::new(text).selected(on).fill(if on { color::HOVER } else { color::RAISED }))
            };
            let all = chip(ui, self.folder.is_none(), s.folders_all.to_string());
            if all.clicked() {
                self.folder = None;
            }
            // dropped on "all": out of its folder
            if let Some(item) = all.dnd_release_payload::<Item>()
                && let Item::Media(id) = &*item
            {
                *action = Some(Action::File(vec![id.clone()], None));
            }
            for (name, n) in folders {
                let on = self.folder.as_deref() == Some(*name);
                let r = chip(ui, on, format!("{name}  {n}"));
                if r.dnd_hover_payload::<Item>().is_some_and(|i| matches!(&*i, Item::Media(_))) {
                    ui.painter().rect_stroke(r.rect, 4.0, Stroke::new(1.5, color::ACCENT), StrokeKind::Outside);
                }
                if let Some(item) = r.dnd_release_payload::<Item>()
                    && let Item::Media(id) = &*item
                {
                    *action = Some(Action::File(vec![id.clone()], Some(name.to_string())));
                }
                if r.on_hover_text(s.folder_drop_tip).clicked() {
                    self.folder = if on { None } else { Some(name.to_string()) };
                }
            }
        });
        ui.add_space(6.0);
    }

    /// The name being typed, if one is: Enter (or the tick) takes it, Escape leaves things be.
    fn naming_row(&mut self, ui: &mut egui::Ui, s: &Strings, action: &mut Option<Action>) {
        let Some(naming) = &mut self.naming else { return };
        let (label, hint, text, focused, limit) = match naming {
            Naming::Rename { text, focused, .. } => (s.media_rename, s.media_name_hint, text, focused, 200),
            Naming::Folder { text, focused, .. } => {
                (s.folder_new, s.folder_name_hint, text, focused, reelforge_model::Media::FOLDER_NAME_MAX)
            }
        };
        let mut done = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).small().color(color::WEAK));
            // the buttons first, from the right, so the field takes what is left and no more
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.add(egui::Button::new("×").small()).clicked();
                done = ui.add(egui::Button::new(icon::DONE).small()).clicked();
                let r = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .char_limit(limit)
                        .desired_width(ui.available_width()),
                );
                if !*focused {
                    r.request_focus();
                    *focused = true;
                }
                done |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        ui.add_space(6.0);
        if done {
            match self.naming.take() {
                Some(Naming::Rename { media, text, .. }) if !text.trim().is_empty() => {
                    *action = Some(Action::Rename(media, text.trim().to_string()));
                }
                Some(Naming::Folder { media, text, .. }) if !text.trim().is_empty() => {
                    *action = Some(Action::File(vec![media], Some(text.trim().to_string())));
                }
                _ => {}
            }
        } else if cancel {
            self.naming = None;
        }
    }

    /// One media card: its picture, length, use and state. Pictures it lacks go to `wanted`.
    /// `folders` are the folders it can be filed under.
    #[allow(clippy::too_many_arguments)]
    fn card<'m>(
        &mut self,
        ui: &mut egui::Ui,
        m: &'m Media,
        w: f32,
        used: usize,
        s: &Strings,
        folders: &[&str],
        wanted: &mut Vec<&'m Media>,
    ) -> Option<Action> {
        let mut action = None;
        let thumb_h = (w * 9.0 / 16.0).round();
        let (rect, r) = ui.allocate_exact_size(vec2(w, thumb_h + 22.0), Sense::click_and_drag());
        if r.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), Item::Media(m.id.clone()));
        }
        let thumb = Rect::from_min_size(rect.min, vec2(w, thumb_h));
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(thumb, 6.0, color::ROW);
        let off = offline(m, &self.missing);
        match m.kind {
            MediaKind::Audio => {
                painter.rect_filled(thumb, 6.0, color::AUDIO.gamma_multiply(0.35));
                painter.text(
                    thumb.center(),
                    Align2::CENTER_CENTER,
                    icon::MUSIC,
                    FontId::proportional(26.0),
                    color::TEXT,
                );
            }
            _ => match self.pictures.get(&thumb_key(m)) {
                Some(Some(t)) => {
                    let uv = cover(t.size_vec2(), thumb.size());
                    painter.add(
                        egui::epaint::RectShape::filled(thumb, CornerRadius::same(6), Color32::WHITE)
                            .with_texture(t.id(), uv),
                    );
                }
                Some(None) => {}
                None if off => {}
                None => {
                    wanted.push(m);
                    let glyph = match m.kind {
                        MediaKind::Video => icon::VIDEO,
                        MediaKind::Lottie => icon::SPARKLE,
                        _ => icon::IMAGE,
                    };
                    painter.text(thumb.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(22.0), color::WEAK);
                }
            },
        }
        // what it is, top left
        let glyph = match m.kind {
            MediaKind::Video => icon::VIDEO,
            MediaKind::Image => icon::IMAGE,
            MediaKind::Lottie => icon::SPARKLE,
            _ => icon::MUSIC,
        };
        badge(&painter, thumb.left_top() + vec2(5.0, 5.0), Align2::LEFT_TOP, glyph, color::TEXT, None);
        if let Some(d) = m.duration.filter(|_| m.kind != MediaKind::Image) {
            badge(
                &painter,
                thumb.right_bottom() - vec2(5.0, 5.0),
                Align2::RIGHT_BOTTOM,
                &short_time(d),
                color::TEXT,
                None,
            );
        }
        if used > 0 {
            badge(
                &painter,
                thumb.left_bottom() + vec2(5.0, -5.0),
                Align2::LEFT_BOTTOM,
                &format!("{} {}", icon::DONE, s.media_used),
                color::ON_ACCENT,
                Some(color::ACCENT),
            );
        }
        if off {
            painter.rect_filled(thumb, 6.0, Color32::from_black_alpha(150));
            painter.text(
                thumb.center(),
                Align2::CENTER_CENTER,
                format!("{}  {}", icon::WARNING, s.media_offline),
                FontId::proportional(12.5),
                color::WARN,
            );
        }
        let selected = self.selected.as_deref() == Some(m.id.as_str());
        let stroke = if selected {
            Stroke::new(2.0, color::ACCENT)
        } else if r.hovered() {
            Stroke::new(1.0, color::WEAK)
        } else {
            Stroke::new(1.0, color::LINE)
        };
        painter.rect_stroke(thumb, 6.0, stroke, StrokeKind::Inside);
        // the name, cut to one line
        let mut job = egui::text::LayoutJob::single_section(
            m.name.clone(),
            egui::TextFormat::simple(FontId::proportional(11.5), if off { color::WARN } else { color::TEXT }),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(w - 2.0);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(pos2(rect.left() + 1.0, thumb.bottom() + 5.0), galley, color::TEXT);

        // add, on hover
        if r.hovered() && !egui::DragAndDrop::has_any_payload(ui.ctx()) {
            let plus = Rect::from_min_size(thumb.right_top() + vec2(-27.0, 5.0), vec2(22.0, 22.0));
            let b = ui.put(
                plus,
                egui::Button::new(egui::RichText::new(icon::ADD).size(13.0).color(color::ON_ACCENT))
                    .fill(color::ACCENT)
                    .corner_radius(11.0),
            );
            if b.on_hover_text(s.add_to_timeline).clicked() {
                action = Some(Action::Add(Item::Media(m.id.clone())));
            }
        }
        if r.clicked() {
            self.selected = Some(m.id.clone());
        }
        if r.double_clicked() {
            action = Some(Action::Add(Item::Media(m.id.clone())));
        }
        let r = r.on_hover_ui(|ui| {
            ui.label(egui::RichText::new(&m.name).strong());
            let mut facts = vec![];
            if let Some([w, h]) = m.size {
                facts.push(format!("{w}×{h}"));
            }
            if let Some(d) = m.duration.filter(|_| m.kind != MediaKind::Image) {
                facts.push(theme::timecode(d.seconds()));
            }
            if used > 0 {
                facts.push(fill(s.media_used_n, &[&used]));
            }
            if let Some(f) = &m.folder {
                facts.push(f.clone());
            }
            if !facts.is_empty() {
                ui.label(egui::RichText::new(facts.join(" · ")).color(color::WEAK));
            }
            if off {
                ui.label(egui::RichText::new(s.media_offline_tip).color(color::WARN));
            }
            ui.label(egui::RichText::new(s.media_card_tip).small().color(color::WEAK));
        });
        r.context_menu(|ui| {
            if ui.button(format!("{}  {}", icon::ADD, s.add_to_timeline)).clicked() {
                action = Some(Action::Add(Item::Media(m.id.clone())));
                ui.close();
            }
            if ui.button(s.media_rename).clicked() {
                self.naming = Some(Naming::Rename { media: m.id.clone(), text: m.name.clone(), focused: false });
                ui.close();
            }
            ui.menu_button(s.media_move_to, |ui| {
                for f in folders {
                    let here = m.folder.as_deref() == Some(*f);
                    if ui.add(egui::Button::new(*f).selected(here)).clicked() {
                        if !here {
                            action = Some(Action::File(vec![m.id.clone()], Some(f.to_string())));
                        }
                        ui.close();
                    }
                }
                if !folders.is_empty() {
                    ui.separator();
                }
                if ui.button(format!("{}  {}", icon::ADD, s.folder_new)).clicked() {
                    self.naming = Some(Naming::Folder { media: m.id.clone(), text: String::new(), focused: false });
                    ui.close();
                }
                if m.folder.is_some() && ui.button(s.folder_none).clicked() {
                    action = Some(Action::File(vec![m.id.clone()], None));
                    ui.close();
                }
            });
            ui.separator();
            let remove = ui
                .add_enabled(used == 0, egui::Button::new(format!("{}  {}", icon::DELETE, s.media_remove)))
                .on_disabled_hover_text(fill(s.media_in_use, &[&used]));
            if remove.clicked() {
                action = Some(Action::Remove(m.id.clone()));
                ui.close();
            }
        });
        action
    }

    /// Makes pictures for up to the frame's budget; true when more are waiting.
    fn make_pictures(&mut self, ctx: &egui::Context, footage: &Footage, wanted: &[&Media]) -> bool {
        let begun = Instant::now();
        let height = (90.0 * ctx.pixels_per_point()).round() as u32;
        for (i, m) in wanted.iter().enumerate() {
            if begun.elapsed() >= BUDGET {
                return i < wanted.len();
            }
            let key = thumb_key(m);
            // a little way in, past a fade from black
            let at = m.duration.map_or(Tick::ZERO, |d| Tick(d.0 / 10).min(Tick::SECOND));
            match footage.thumbnail(&crate::strips::asset(m), at, height) {
                Ok(None) => {
                    self.missing.insert(key);
                }
                made => {
                    let texture = made.ok().flatten().map(|t| {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
                        ctx.load_texture(format!("asset-{key}"), image, egui::TextureOptions::LINEAR)
                    });
                    self.pictures.insert(key, texture);
                }
            }
        }
        false
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui, s: &Strings, action: &mut Option<Action>) {
        if self.sound_lengths.is_empty() {
            self.sound_lengths = SOUNDS.iter().map(|snd| (snd.id, sound_length(snd.id))).collect();
        }
        search_box(ui, &mut self.query, s);
        let query = self.query.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !SOUNDS.iter().zip(s.sound_names).any(|(_, name)| finds(&query, name)) {
                nothing_matches(ui, s);
                return;
            }
            for (title, cat) in [(s.sound_effects, Category::Effect), (s.sound_music, Category::Music)] {
                let found: Vec<_> = SOUNDS
                    .iter()
                    .zip(s.sound_names)
                    .filter(|(x, name)| x.category == cat && finds(&query, name))
                    .collect();
                if found.is_empty() {
                    continue;
                }
                ui.label(egui::RichText::new(title).small().strong().color(color::WEAK));
                ui.add_space(2.0);
                for (snd, name) in found {
                    let length = self.sound_lengths.get(snd.id).copied().unwrap_or(Tick::ZERO);
                    let item = Item::Sound { id: snd.id, name, length };
                    let glyph = if cat == Category::Music { icon::MUSIC } else { icon::SOUND };
                    let shown = if length < Tick::SECOND * 10 {
                        format!("{:.1}s", length.seconds())
                    } else {
                        short_time(length)
                    };
                    if let Some(a) = row(ui, glyph, name, &shown, item, s) {
                        *action = Some(a);
                    }
                }
                ui.add_space(10.0);
            }
            ui.label(egui::RichText::new(s.sound_free).small().color(color::WEAK));
        });
    }
}

impl AssetPanel {
    /// The stickers as a grid of pictures.
    fn stickers_tab(&mut self, ui: &mut egui::Ui, s: &Strings, action: &mut Option<Action>) {
        if self.stickers.is_empty() {
            let side = 128;
            self.stickers = STICKERS
                .iter()
                .filter_map(|st| {
                    let px = reelforge_media::stickers::render(st.id, side as u32)?;
                    let image = egui::ColorImage::from_rgba_unmultiplied([side, side], &px);
                    Some(ui.ctx().load_texture(format!("sticker-{}", st.id), image, egui::TextureOptions::LINEAR))
                })
                .collect();
            self.decor = DECOR
                .iter()
                .filter_map(|d| {
                    let preview = reelforge_media::decor::Preview::of(d)?;
                    let still = decor_image(&preview, None);
                    let name = format!("decor-{}", d.id);
                    Some((ui.ctx().load_texture(name, still, egui::TextureOptions::LINEAR), preview))
                })
                .collect();
        }
        search_box(ui, &mut self.query, s);
        let query = self.query.clone();
        // one part at a time, unless something is being looked for: then whatever matches
        let part = query.trim().is_empty().then_some(self.stuck);
        if part.is_some() {
            ui.add_space(6.0);
            if let Some(picked) = theme::segmented(ui, &s.sticker_parts, self.stuck) {
                self.stuck = picked;
            }
            ui.add_space(8.0);
        }
        let shown = |which: usize| part.is_none_or(|p| p == which);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = 8.0;
            let room = ui.available_width();
            let columns = ((room + gap) / (72.0 + gap)).floor().max(2.0) as usize;
            let w = ((room - gap * (columns - 1) as f32) / columns as f32).floor();
            let all: Vec<_> = STICKERS
                .iter()
                .zip(&self.stickers)
                .zip(s.sticker_names)
                .filter(|(_, name)| shown(0) && finds(&query, name))
                .collect();
            let mut lists: Vec<(String, Vec<(Item, &str)>)> = reelforge_engine::Category::ALL
                .into_iter()
                .zip(s.promo_categories)
                .map(|(category, heading)| {
                    let items = reelforge_engine::PROMOS
                        .iter()
                        .filter(|p| p.category == category)
                        .map(|p| (Item::Promo(p.id), promo_name(p.id, s)))
                        .filter(|(_, name)| shown(2) && (finds(&query, name) || finds(&query, heading)))
                        .collect();
                    (format!("{} · {heading}", s.sec_promo), items)
                })
                .collect();
            let shapes: Vec<(Item, &str)> = reelforge_model::ShapeKind::ALL
                .into_iter()
                .map(|k| (Item::Shape(k), shape_name(k, s)))
                .filter(|(_, name)| shown(0) && finds(&query, name))
                .collect();
            lists.insert(0, (s.sec_shapes.to_string(), shapes));
            let decor: Vec<usize> = (0..self.decor.len())
                .filter(|i| finds(&query, decor_name(self.decor[*i].1.decor().id, s)) || finds(&query, s.sec_decor))
                .filter(|_| shown(1))
                .collect();
            if all.is_empty() && decor.is_empty() && lists.iter().all(|(_, items)| items.is_empty()) {
                nothing_matches(ui, s);
                return;
            }
            // the animated decorations: each plays while the pointer is over it
            if !decor.is_empty() {
                let heading = if part.is_some() { s.decor_tip } else { s.sec_decor };
                ui.label(egui::RichText::new(heading).small().color(color::WEAK));
                ui.add_space(4.0);
            }
            let now = ui.input(|i| i.time);
            let mut playing = None;
            for row in decor.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for &i in row {
                        let id = self.decor[i].1.decor().id;
                        let (rect, r) = ui.allocate_exact_size(vec2(w, w), Sense::click_and_drag());
                        if r.drag_started() {
                            egui::DragAndDrop::set_payload(ui.ctx(), Item::Decor(id));
                        }
                        if r.hovered() {
                            let began = self.decor_playing.filter(|(was, _)| *was == i).map_or(now, |(_, began)| began);
                            playing = Some((i, began));
                            let (texture, preview) = &mut self.decor[i];
                            texture.set(decor_image(preview, Some((now - began) as f32)), egui::TextureOptions::LINEAR);
                            ui.ctx().request_repaint();
                        }
                        let texture = &self.decor[i].0;
                        let painter = ui.painter();
                        painter.rect_filled(rect, 6.0, if r.hovered() { color::HOVER } else { color::RAISED });
                        let size = texture.size_vec2();
                        painter.image(
                            texture.id(),
                            Rect::from_center_size(rect.center(), size * (w * 0.84 / size.max_elem())),
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        if r.hovered() {
                            painter.rect_stroke(rect, 6.0, Stroke::new(1.0, color::ACCENT), StrokeKind::Inside);
                        }
                        if r.on_hover_text(format!("{}\n{}", decor_name(id, s), s.item_tip)).clicked() {
                            *action = Some(Action::Add(Item::Decor(id)));
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            // the one the pointer left is still again
            if let Some((was, _)) = self.decor_playing.filter(|(was, _)| playing.is_none_or(|(i, _)| i != *was)) {
                let (texture, preview) = &mut self.decor[was];
                texture.set(decor_image(preview, None), egui::TextureOptions::LINEAR);
            }
            self.decor_playing = playing;
            if !decor.is_empty() {
                ui.add_space(8.0);
            }
            if !all.is_empty() && part.is_none() {
                ui.label(egui::RichText::new(s.tab_stickers).small().color(color::WEAK));
            }
            for row in all.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for ((st, texture), name) in row {
                        let (rect, r) = ui.allocate_exact_size(vec2(w, w), Sense::click_and_drag());
                        if r.drag_started() {
                            egui::DragAndDrop::set_payload(ui.ctx(), Item::Sticker(st.id));
                        }
                        let painter = ui.painter();
                        painter.rect_filled(rect, 6.0, if r.hovered() { color::HOVER } else { color::RAISED });
                        painter.image(
                            texture.id(),
                            rect.shrink(w * 0.1),
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        if r.hovered() {
                            painter.rect_stroke(rect, 6.0, Stroke::new(1.0, color::ACCENT), StrokeKind::Inside);
                        }
                        if r.on_hover_text(format!("{name}\n{}", s.item_tip)).clicked() {
                            *action = Some(Action::Add(Item::Sticker(st.id)));
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            if !all.is_empty() {
                ui.add_space(8.0);
            }
            // the shapes and the ready-made pieces, list by list, as words to click or drag
            for (heading, items) in lists {
                if items.is_empty() {
                    continue;
                }
                ui.label(egui::RichText::new(heading).small().color(color::WEAK));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (item, name) in items {
                        let r = ui.add(egui::Button::new(name).sense(Sense::click_and_drag()));
                        if r.drag_started() {
                            egui::DragAndDrop::set_payload(ui.ctx(), item.clone());
                        }
                        if r.on_hover_text(s.item_tip).clicked() {
                            *action = Some(Action::Add(item));
                        }
                    }
                });
                ui.add_space(8.0);
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(s.sound_free).small().color(color::WEAK));
        });
    }
}

/// A decoration's picture for the panel: `seconds` into playing it, or still.
fn decor_image(preview: &reelforge_media::decor::Preview, seconds: Option<f32>) -> egui::ColorImage {
    let p = preview.picture(seconds, 128);
    egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba)
}

/// Picture effects by what they do, and the empty adjustment layer.
fn effects_tab(ui: &mut egui::Ui, query: &mut String, s: &Strings, action: &mut Option<Action>) {
    search_box(ui, query, s);
    let query = query.as_str();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let adjustment = finds(query, s.fx_adjustment);
        if !adjustment && !FxKind::ALL.into_iter().any(|k| finds(query, fx_name(k, s))) {
            nothing_matches(ui, s);
            return;
        }
        ui.label(egui::RichText::new(s.fx_tip).small().color(color::WEAK));
        ui.add_space(6.0);
        if adjustment
            && let Some(a) = fx_row(ui, icon::SLIDERS, s.fx_adjustment, Item::Effect(None), s.fx_adjustment_tip)
        {
            *action = Some(a);
        }
        ui.add_space(8.0);
        for (title, group) in s.fx_groups.into_iter().zip(FxGroup::ALL) {
            let found: Vec<FxKind> =
                FxKind::ALL.into_iter().filter(|k| k.group() == group && finds(query, fx_name(*k, s))).collect();
            if found.is_empty() {
                continue;
            }
            ui.label(egui::RichText::new(title).small().strong().color(color::WEAK));
            ui.add_space(2.0);
            for kind in found {
                if let Some(a) = fx_row(ui, icon::SPARKLE, fx_name(kind, s), Item::Effect(Some(kind)), s.item_tip) {
                    *action = Some(a);
                }
            }
            ui.add_space(10.0);
        }
    });
}

impl AssetPanel {
    /// Timeline templates (剪同款) or storyboards: how far the slots of the timeline being edited
    /// are filled, with a way to pick files for them; the project's copy to type over, where a
    /// storyboard wrote it; then the built-in ones to use.
    fn templates_tab(&mut self, ui: &mut egui::Ui, p: &Project, lang: crate::i18n::Lang, action: &mut Option<Action>) {
        let s = lang.strings();
        if let Some(picked) = theme::segmented(ui, &[s.sec_remix, s.sec_boards], usize::from(self.boards)) {
            self.boards = picked == 1;
        }
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            slots_progress(ui, p, s, action);
            if self.boards {
                if !p.copy.is_empty() {
                    if let Some((name, words)) = self.copy.show(ui, p, lang, s) {
                        *action = Some(Action::Copy(name, words));
                    }
                    ui.add_space(10.0);
                }
                ui.label(egui::RichText::new(s.boards_tip).small().color(color::WEAK));
                ui.add_space(8.0);
                for board in reelforge_engine::storyboards() {
                    let w = ui.available_width();
                    if crate::storyboard::card(ui, board, lang, s, w).clicked() {
                        *action = Some(Action::Storyboard(board.id.as_str()));
                    }
                    ui.add_space(8.0);
                }
            } else {
                ui.label(egui::RichText::new(s.remix_tip).small().color(color::WEAK));
                ui.add_space(8.0);
                for (i, t) in reelforge_engine::timeline_templates().iter().enumerate() {
                    let w = ui.available_width();
                    if crate::remix::card(ui, i, t, s, w).clicked() {
                        *action = Some(Action::Template(t.id));
                    }
                    ui.add_space(8.0);
                }
            }
        });
    }
}

/// How far the slots of the timeline being edited are filled, with a way to pick files for them;
/// nothing when it has none.
fn slots_progress(ui: &mut egui::Ui, p: &Project, s: &Strings, action: &mut Option<Action>) {
    let slots = reelforge_engine::slot_states(p);
    if slots.is_empty() {
        return;
    }
    let filled = slots.iter().filter(|(_, _, filled)| *filled).count();
    ui.label(egui::RichText::new(fill(s.remix_progress, &[&filled.to_string(), &slots.len().to_string()])).strong());
    ui.add_space(2.0);
    // one piece of the bar for each slot
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let gap = 2.0;
    let w = (bar.width() - gap * (slots.len() - 1) as f32) / slots.len() as f32;
    for (i, (_, _, filled)) in slots.iter().enumerate() {
        let piece =
            Rect::from_min_size(bar.left_top() + vec2((w + gap) * i as f32, 0.0), vec2(w.max(1.0), bar.height()));
        ui.painter().rect_filled(piece, 2.0, if *filled { color::ACCENT } else { color::RAISED });
    }
    ui.add_space(6.0);
    if filled < slots.len() {
        if theme::primary(ui, icon::IMPORT, s.remix_pick, true).on_hover_text(s.remix_pick_tip).clicked() {
            *action = Some(Action::Fill);
        }
    } else {
        ui.label(egui::RichText::new(format!("{}  {}", icon::DONE, s.remix_done)).small().color(color::WEAK));
    }
    ui.add_space(8.0);
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, color::LINE));
    ui.add_space(8.0);
}

/// A row of the effects list: click adds at the playhead, drag goes to the timeline.
fn fx_row(ui: &mut egui::Ui, glyph: &str, name: &str, item: Item, tip: &str) -> Option<Action> {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click_and_drag());
    if r.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), item.clone());
    }
    let painter = ui.painter();
    if r.hovered() {
        painter.rect_filled(rect, 6.0, color::HOVER);
    }
    let tile = Rect::from_min_size(rect.left_top() + vec2(4.0, 4.0), vec2(24.0, 24.0));
    painter.rect_filled(tile, 5.0, color::FX.gamma_multiply(0.6));
    painter.text(tile.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color::TEXT);
    painter.text(
        rect.left_center() + vec2(38.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        color::TEXT,
    );
    if r.hovered() {
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            icon::ADD,
            FontId::proportional(14.0),
            color::ACCENT,
        );
    }
    r.on_hover_text(tip).clicked().then_some(Action::Add(item))
}

/// Nothing imported yet: a large target that imports.
fn empty(ui: &mut egui::Ui, s: &Strings, action: &mut Option<Action>) {
    let h = 180.0f32.min(ui.available_height().max(100.0));
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let tint = if r.hovered() { color::ACCENT } else { color::WEAK };
    let painter = ui.painter();
    // a well in the glass, its rim lit under the pointer
    painter.rect(
        rect,
        CornerRadius::same(16),
        if r.hovered() { color::GLASS_CONTROL } else { color::GLASS_INSET },
        Stroke::new(1.0, if r.hovered() { color::ACCENT.gamma_multiply(0.7) } else { Color32::from_white_alpha(22) }),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center() - vec2(0.0, 22.0),
        Align2::CENTER_CENTER,
        icon::IMPORT,
        FontId::proportional(26.0),
        tint,
    );
    painter.text(
        rect.center() + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        s.media_empty,
        FontId::proportional(13.5),
        color::TEXT,
    );
    let mut job = egui::text::LayoutJob::single_section(
        s.drop_hint.to_string(),
        egui::TextFormat::simple(FontId::proportional(11.0), color::WEAK),
    );
    job.wrap.max_width = rect.width() - 20.0;
    job.halign = egui::Align::Center;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(pos2(rect.center().x, rect.center().y + 22.0), galley, color::WEAK);
    if r.on_hover_text(s.import_tip).clicked() {
        *action = Some(Action::Import);
    }
}

/// A row of the sound list: click adds at the playhead, drag goes to the timeline.
fn row(ui: &mut egui::Ui, glyph: &str, name: &str, length: &str, item: Item, s: &Strings) -> Option<Action> {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click_and_drag());
    if r.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), item.clone());
    }
    let painter = ui.painter();
    if r.hovered() {
        painter.rect_filled(rect, 6.0, color::HOVER);
    }
    let tile = Rect::from_min_size(rect.left_top() + vec2(4.0, 4.0), vec2(24.0, 24.0));
    painter.rect_filled(tile, 5.0, color::AUDIO.gamma_multiply(0.45));
    painter.text(tile.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color::TEXT);
    painter.text(
        rect.left_center() + vec2(38.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        color::TEXT,
    );
    if r.hovered() {
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            icon::ADD,
            FontId::proportional(14.0),
            color::ACCENT,
        );
    } else {
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            length,
            FontId::monospace(10.5),
            color::WEAK,
        );
    }
    let r = r.on_hover_text(s.item_tip);
    r.clicked().then_some(Action::Add(item))
}

/// Titles: a plain one and the templates (花字), as pictures.
fn text_tab(ui: &mut egui::Ui, s: &Strings, looks: &[crate::looks::Look], action: &mut Option<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(egui::RichText::new(s.text_basic).small().strong().color(color::WEAK));
        ui.add_space(2.0);
        let gap = 8.0;
        let w = ((ui.available_width() - gap) / 2.0).floor();
        let h = (w * crate::looks::SIZE[1] as f32 / crate::looks::SIZE[0] as f32).round();
        ui.horizontal(|ui| {
            if let Some(a) = title_card(ui, vec2(w, h), None, Item::Title(None), s.text_default, s) {
                *action = Some(a);
            }
        });
        ui.add_space(10.0);
        // the templates, shelf by shelf
        for (g, shelf) in crate::looks::shelves(looks) {
            let head = format!("{} · {}", s.template_groups[g], shelf.len());
            ui.label(egui::RichText::new(head).small().strong().color(color::WEAK));
            ui.add_space(2.0);
            for pair in shelf.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (id, texture) in pair {
                        let item = Item::Title(Some(id));
                        if let Some(a) = title_card(ui, vec2(w, h), Some(texture), item, template_name(id, s), s) {
                            *action = Some(a);
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            ui.add_space(6.0);
        }
    });
}

fn title_card(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    picture: Option<&egui::TextureHandle>,
    item: Item,
    name: &str,
    s: &Strings,
) -> Option<Action> {
    let (rect, r) = ui.allocate_exact_size(size + vec2(0.0, 20.0), Sense::click_and_drag());
    if r.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), item.clone());
    }
    let tile = Rect::from_min_size(rect.min, size);
    let painter = ui.painter();
    match picture {
        Some(t) => {
            painter.image(t.id(), tile, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(tile, 6.0, color::RAISED);
            painter.text(tile.center(), Align2::CENTER_CENTER, "Aa", FontId::proportional(26.0), color::TEXT);
        }
    }
    let stroke = if r.hovered() { Stroke::new(1.0, color::ACCENT) } else { Stroke::new(1.0, color::LINE) };
    painter.rect_stroke(tile, 6.0, stroke, StrokeKind::Inside);
    if r.hovered() {
        let plus = Rect::from_center_size(tile.right_bottom() - vec2(15.0, 15.0), vec2(20.0, 20.0));
        painter.circle_filled(plus.center(), 10.0, color::ACCENT);
        painter.text(plus.center(), Align2::CENTER_CENTER, icon::ADD, FontId::proportional(12.0), color::ON_ACCENT);
    }
    painter.text(
        pos2(rect.left() + 1.0, tile.bottom() + 4.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(11.5),
        color::TEXT,
    );
    let r = r.on_hover_text(s.item_tip);
    r.clicked().then_some(Action::Add(item))
}

/// A small label on a picture.
fn badge(painter: &egui::Painter, at: egui::Pos2, align: Align2, text: &str, ink: Color32, fill: Option<Color32>) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(10.5), ink);
    let size = galley.size() + vec2(8.0, 3.0);
    let rect = align.anchor_size(at, size);
    painter.rect_filled(rect, 4.0, fill.unwrap_or(Color32::from_black_alpha(170)));
    painter.galley(rect.min + vec2(4.0, 1.5), galley, ink);
}

/// The part of a picture of `size` that fills `room`, centred (cropping the rest).
fn cover(size: egui::Vec2, room: egui::Vec2) -> Rect {
    let (a, b) = (size.x / size.y.max(1.0), room.x / room.y.max(1.0));
    if a > b {
        let w = b / a;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = a / b;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
fn short_time(t: Tick) -> String {
    let s = t.seconds().max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// While something from the panel is dragged, its name follows the pointer.
fn drag_preview(ctx: &egui::Context, p: &Project, s: &Strings) {
    let Some(item) = egui::DragAndDrop::payload::<Item>(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("asset drag")));
    let glyph = match (&*item, item.track_kind(p)) {
        (Item::Sticker(_) | Item::Shape(_) | Item::Promo(_), _) => icon::STICKER,
        (Item::Decor(_), _) => icon::SPARKLE,
        (Item::Effect(_), _) => icon::SPARKLE,
        (_, TrackKind::Audio) => icon::MUSIC,
        (_, TrackKind::Text) => icon::TEXT,
        (_, TrackKind::Video) => icon::FILM,
    };
    let galley =
        painter.layout_no_wrap(format!("{glyph}  {}", item.label(p, s)), FontId::proportional(12.0), color::ON_ACCENT);
    let rect = Rect::from_min_size(pos + vec2(14.0, 10.0), galley.size() + vec2(14.0, 8.0));
    painter.rect_filled(rect, 6.0, color::ACCENT.gamma_multiply(0.92));
    painter.galley(rect.min + vec2(7.0, 4.0), galley, color::ON_ACCENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fill_their_card() {
        let wide = cover(vec2(200.0, 100.0), vec2(160.0, 90.0));
        assert!((wide.width() - (16.0 / 9.0) / 2.0).abs() < 1e-5 && wide.height() == 1.0);
        let tall = cover(vec2(90.0, 160.0), vec2(160.0, 90.0));
        assert!(tall.width() == 1.0 && (tall.height() - (90.0 / 160.0) / (16.0 / 9.0)).abs() < 1e-5);
        assert_eq!(short_time(Tick::from_seconds(65.4)), "1:05");
        assert_eq!(short_time(Tick::from_seconds(3725.0)), "1:02:05");
    }

    #[test]
    fn a_search_finds_names_with_every_word_whatever_the_case() {
        assert!(finds("", "anything"));
        assert!(finds("  ", "anything"));
        assert!(finds("beach", "Beach walk.MP4"));
        assert!(finds("walk BEACH", "Beach walk.MP4"));
        assert!(!finds("beach night", "Beach walk.MP4"));
        assert!(finds("海", "海边 01.mp4"));
        assert!(!finds("山", "海边 01.mp4"));
    }

    #[test]
    fn items_know_their_track_and_length() {
        let mut s = reelforge_engine::Session::default();
        let m = s
            .execute("media.import", &serde_json::json!({"path": "a.mp3", "kind": "audio", "duration": {"seconds": 7}}))
            .unwrap()["media"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let p = s.project();
        assert_eq!(Item::Media(m.clone()).track_kind(&p), TrackKind::Audio);
        assert_eq!(Item::Media(m.clone()).length(&p), Tick::SECOND * 7);
        assert_eq!(Item::Title(None).track_kind(&p), TrackKind::Text);
        // stickers and effects go over the picture
        for over in [Item::Sticker("heart"), Item::Effect(Some(FxKind::Shake)), Item::Effect(None)] {
            assert_eq!((over.track_kind(&p), over.length(&p)), (TrackKind::Video, OVERLAY_LENGTH));
        }
        let zh = crate::i18n::Lang::Zh.strings();
        assert_eq!(Item::Sticker("heart").label(&p, zh), "爱心");
        assert_eq!(Item::Effect(Some(FxKind::Flash)).label(&p, zh), "闪白");
        assert_eq!(Item::Effect(None).label(&p, zh), "调节层");
        // a decoration is as long as it says, and called in the interface's language
        let rain = Item::Decor("confettiRain");
        assert_eq!(
            (rain.track_kind(&p), rain.length(&p), rain.label(&p, zh)),
            (TrackKind::Video, Tick::SECOND * 3, "彩纸雨".into())
        );
        // a file never found is offline
        assert!(offline(&p.media[&m], &HashSet::new()));
        let len = sound_length(SOUNDS[0].id);
        assert!(len > Tick::ZERO && len < Tick::SECOND * 60);
    }
}
