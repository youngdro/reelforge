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
}

/// A picture effect's shown name.
pub(crate) fn fx_name(kind: FxKind, s: &Strings) -> &'static str {
    s.fx_names[FxKind::ALL.iter().position(|k| *k == kind).unwrap_or(0)]
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
            Item::Sticker(_) | Item::Effect(_) => TrackKind::Video,
        }
    }

    /// How long it lasts once placed, as far as the drop preview needs to know.
    pub(crate) fn length(&self, p: &Project) -> Tick {
        match self {
            Item::Media(id) => p.media.get(id).and_then(|m| m.duration).unwrap_or(STILL_PREVIEW),
            Item::Sound { length, .. } => *length,
            Item::Title(_) => TITLE_LENGTH,
            Item::Sticker(_) | Item::Effect(_) => OVERLAY_LENGTH,
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
    /// Something asked of the captions list.
    Captions(crate::captions::Ask),
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
            Filter::Video => kind == MediaKind::Video,
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
    /// Pictures of the media by [`thumb_key`]; `None` for one that could not be made.
    pictures: HashMap<String, Option<egui::TextureHandle>>,
    /// Files that were not there at [`Footage::generation`] `seen`.
    missing: HashSet<String>,
    seen: u64,
    /// The library sounds' lengths, measured the first time the sounds are shown.
    sound_lengths: HashMap<&'static str, Tick>,
    /// The stickers' pictures, drawn the first time the stickers are shown.
    stickers: Vec<egui::TextureHandle>,
    captions: crate::captions::CaptionsTab,
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
    /// Draws the panel. `looks` are the pictures of the title templates (made by the caller when
    /// the text tab shows; see [`AssetPanel::wants_looks`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Project,
        media: &Footage,
        lang: crate::i18n::Lang,
        looks: &[(&'static str, egui::TextureHandle)],
        playing: bool,
        playhead: Tick,
        selected: &[String],
    ) -> Option<Action> {
        let s = lang.strings();
        let mut action = None;
        self.tabs(ui, s);
        ui.add_space(6.0);
        match self.tab {
            Tab::Media => self.media_tab(ui, p, media, s, playing, &mut action),
            Tab::Audio => self.audio_tab(ui, s, &mut action),
            Tab::Text => text_tab(ui, s, looks, &mut action),
            Tab::Stickers => self.stickers_tab(ui, s, &mut action),
            Tab::Effects => effects_tab(ui, s, &mut action),
            Tab::Captions => {
                action = self.captions.show(ui, p, lang, playhead, selected, playing).map(Action::Captions);
            }
        }
        drag_preview(ui.ctx(), p, s);
        action
    }

    /// Whether the text tab is showing (and so needs the templates' pictures).
    pub(crate) fn wants_looks(&self) -> bool {
        self.tab == Tab::Text
    }

    fn tabs(&mut self, ui: &mut egui::Ui, s: &Strings) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tabs = [
                (Tab::Media, icon::IMAGE, s.tab_media),
                (Tab::Audio, icon::MUSIC, s.tab_audio),
                (Tab::Text, icon::TEXT, s.tab_text),
                (Tab::Stickers, icon::STICKER, s.tab_stickers),
                (Tab::Effects, icon::SPARKLE, s.tab_effects),
                (Tab::Captions, icon::CAPTIONS, s.tab_captions),
            ];
            // as wide as the panel lets them be
            let w = ((ui.available_width() - 2.0 * (tabs.len() - 1) as f32) / tabs.len() as f32).clamp(36.0, 54.0);
            for (tab, glyph, word) in tabs {
                let on = self.tab == tab;
                let (rect, r) = ui.allocate_exact_size(vec2(w.floor(), 44.0), Sense::click());
                if r.hovered() && !on {
                    ui.painter().rect_filled(rect, 6.0, color::HOVER);
                }
                let tint = if on {
                    color::ACCENT
                } else if r.hovered() {
                    color::TEXT
                } else {
                    color::WEAK
                };
                let painter = ui.painter();
                painter.text(
                    rect.center_top() + vec2(0.0, 6.0),
                    Align2::CENTER_TOP,
                    glyph,
                    FontId::proportional(17.0),
                    tint,
                );
                // a long word is drawn smaller rather than over its neighbours
                let mut size = 11.5;
                while size > 8.0
                    && painter.layout_no_wrap(word.to_string(), FontId::proportional(size), tint).size().x
                        > rect.width()
                {
                    size -= 0.5;
                }
                painter.text(
                    rect.center_bottom() - vec2(0.0, 6.0),
                    Align2::CENTER_BOTTOM,
                    word,
                    FontId::proportional(size),
                    tint,
                );
                if on {
                    let bar = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 1.0), vec2(22.0, 2.0));
                    painter.rect_filled(bar, 1.0, color::ACCENT);
                }
                if r.clicked() {
                    self.tab = tab;
                }
            }
        });
        let y = ui.cursor().top();
        let x = ui.max_rect().x_range();
        ui.painter().hline(x, y, Stroke::new(1.0, color::LINE));
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
        ui.horizontal(|ui| {
            if theme::primary(ui, icon::IMPORT, s.import, true).on_hover_text(s.import_tip).clicked() {
                *action = Some(Action::Import);
            }
            ui.spacing_mut().item_spacing.x = 4.0;
            for (f, word) in [
                (Filter::All, s.filter_all),
                (Filter::Video, s.filter_video),
                (Filter::Image, s.filter_image),
                (Filter::Audio, s.filter_audio),
            ] {
                let on = self.filter == f;
                let text = egui::RichText::new(word).small().color(if on { color::TEXT } else { color::WEAK });
                if ui.add(egui::Button::new(text).selected(on).frame(on)).clicked() {
                    self.filter = f;
                }
            }
        });
        ui.add_space(8.0);
        // in the order they were imported (ids are minted in order: m2 before m10)
        let mut all: Vec<&Media> =
            p.media.values().filter(|m| !matches!(m.kind, MediaKind::Lut | MediaKind::Font)).collect();
        all.sort_by_key(|m| (m.id.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().ok(), &m.id));
        if all.is_empty() {
            empty(ui, s, action);
            return;
        }
        let shown: Vec<&Media> = all.iter().copied().filter(|m| self.filter.admits(m.kind)).collect();
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
                        if let Some(a) = self.card(ui, m, w, used, s, &mut wanted) {
                            *action = Some(a);
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            if shown.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| ui.label(egui::RichText::new(s.media_none_here).color(color::WEAK)));
            }
        });
        if !playing && self.make_pictures(ui.ctx(), footage, &wanted) {
            ui.ctx().request_repaint();
        }
    }

    /// One media card: its picture, length, use and state. Pictures it lacks go to `wanted`.
    fn card<'m>(
        &mut self,
        ui: &mut egui::Ui,
        m: &'m Media,
        w: f32,
        used: usize,
        s: &Strings,
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
                    let glyph = if m.kind == MediaKind::Video { icon::VIDEO } else { icon::IMAGE };
                    painter.text(thumb.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(22.0), color::WEAK);
                }
            },
        }
        // what it is, top left
        let glyph = match m.kind {
            MediaKind::Video => icon::VIDEO,
            MediaKind::Image => icon::IMAGE,
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
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, cat) in [(s.sound_effects, Category::Effect), (s.sound_music, Category::Music)] {
                ui.label(egui::RichText::new(title).small().strong().color(color::WEAK));
                ui.add_space(2.0);
                for (snd, name) in SOUNDS.iter().zip(s.sound_names).filter(|(x, _)| x.category == cat) {
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
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = 8.0;
            let room = ui.available_width();
            let columns = ((room + gap) / (72.0 + gap)).floor().max(2.0) as usize;
            let w = ((room - gap * (columns - 1) as f32) / columns as f32).floor();
            let all: Vec<_> = STICKERS.iter().zip(&self.stickers).zip(s.sticker_names).collect();
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
            ui.add_space(4.0);
            ui.label(egui::RichText::new(s.sound_free).small().color(color::WEAK));
        });
    }
}

/// Picture effects by what they do, and the empty adjustment layer.
fn effects_tab(ui: &mut egui::Ui, s: &Strings, action: &mut Option<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(egui::RichText::new(s.fx_tip).small().color(color::WEAK));
        ui.add_space(6.0);
        if let Some(a) = fx_row(ui, icon::SLIDERS, s.fx_adjustment, Item::Effect(None), s.fx_adjustment_tip) {
            *action = Some(a);
        }
        ui.add_space(8.0);
        for (title, group) in s.fx_groups.into_iter().zip(FxGroup::ALL) {
            ui.label(egui::RichText::new(title).small().strong().color(color::WEAK));
            ui.add_space(2.0);
            for kind in FxKind::ALL.into_iter().filter(|k| k.group() == group) {
                if let Some(a) = fx_row(ui, icon::SPARKLE, fx_name(kind, s), Item::Effect(Some(kind)), s.item_tip) {
                    *action = Some(a);
                }
            }
            ui.add_space(10.0);
        }
    });
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
    let h = 150.0f32.min(ui.available_height().max(100.0));
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let tint = if r.hovered() { color::ACCENT } else { color::WEAK };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, if r.hovered() { color::RAISED } else { color::ROW });
    // dashed border
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    for pair in corners.windows(2) {
        painter.add(egui::Shape::dashed_line(&[pair[0], pair[1]], Stroke::new(1.0, tint), 5.0, 4.0));
    }
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
fn text_tab(
    ui: &mut egui::Ui,
    s: &Strings,
    looks: &[(&'static str, egui::TextureHandle)],
    action: &mut Option<Action>,
) {
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
        ui.label(egui::RichText::new(s.sec_templates).small().strong().color(color::WEAK));
        ui.add_space(2.0);
        for pair in looks.chunks(2) {
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
        (Item::Sticker(_), _) => icon::STICKER,
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
        // a file never found is offline
        assert!(offline(&p.media[&m], &HashSet::new()));
        let len = sound_length(SOUNDS[0].id);
        assert!(len > Tick::ZERO && len < Tick::SECOND * 60);
    }
}
