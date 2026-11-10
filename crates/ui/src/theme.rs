//! How the editor looks: one dark palette for desktop and browser, the faces it draws with, and
//! the icons (a subset of Phosphor, MIT, in `fonts/`; egui's own faces have no icons, and emoji
//! fall back to boxes on many desktops).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// The palette: neutral dark glass, so the colour on screen is the picture's, seen through the
/// panels. What is live (play, the selection, focus) is one blue; kinds of clip are told apart by
/// a quiet tint.
pub(crate) mod color {
    use super::Color32;

    /// Behind everything; the preview's surroundings.
    pub const STAGE: Color32 = Color32::from_rgb(0x05, 0x05, 0x06);
    /// Panels: islands on the stage.
    pub const PANEL: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1e);
    /// Raised areas on panels: headers, section frames, inputs.
    pub const RAISED: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2e);
    /// Hovered controls.
    pub const HOVER: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3c);
    /// Hairlines between areas.
    pub const LINE: Color32 = Color32::from_rgb(0x38, 0x38, 0x3a);
    /// Timeline rows.
    pub const ROW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 84);
    pub const ROW_ALT: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 64);
    /// Glass: what panels are made of. The stage's light shows through it.
    pub const GLASS: Color32 = Color32::from_rgba_premultiplied(27, 27, 30, 168);
    /// A control on glass, and one under the pointer: a little light caught, not a colour.
    pub const GLASS_CONTROL: Color32 = Color32::from_rgba_premultiplied(22, 22, 24, 22);
    pub const GLASS_HOVER: Color32 = Color32::from_rgba_premultiplied(40, 40, 44, 40);
    /// A group set into glass: a section of a panel, the chosen tab.
    pub const GLASS_INSET: Color32 = Color32::from_rgba_premultiplied(13, 13, 14, 13);
    pub const TEXT: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf7);
    pub const WEAK: Color32 = Color32::from_rgb(0x98, 0x98, 0x9f);
    /// Selection, the primary action, focus.
    pub const ACCENT: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);
    /// Where a surface in the accent shades to.
    pub const ACCENT_2: Color32 = Color32::from_rgb(0x40, 0x9c, 0xff);
    /// Text on the accent.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
    /// What records, and what is about to be cut out.
    pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0x45, 0x3a);
    pub const WARN: Color32 = Color32::from_rgb(0xff, 0x9f, 0x0a);
    /// Clips, by what they hold.
    pub const VIDEO: Color32 = Color32::from_rgb(0x64, 0xa8, 0xff);
    pub const TEXT_CLIP: Color32 = Color32::from_rgb(0xff, 0xb3, 0x40);
    pub const AUDIO: Color32 = Color32::from_rgb(0x32, 0xd1, 0xa0);
    /// Adjustment clips: effects and colour for what is below.
    pub const FX: Color32 = Color32::from_rgb(0xbf, 0x7a, 0xf0);
    /// Compound clips.
    pub const COMPOUND: Color32 = Color32::from_rgb(0x5a, 0xc8, 0xe0);
}

/// Icon glyphs from the bundled Phosphor subset; usable in any text. Not every one is used on every
/// platform (the browser has no Export).
#[allow(dead_code)]
pub(crate) mod icon {
    pub const PLAY: &str = "\u{e3d0}";
    pub const PAUSE: &str = "\u{e39e}";
    pub const SPLIT: &str = "\u{eae0}";
    pub const DELETE: &str = "\u{e4a6}";
    pub const UNDO: &str = "\u{e038}";
    pub const REDO: &str = "\u{e036}";
    pub const EXPORT: &str = "\u{eaf0}";
    pub const TEXT: &str = "\u{e48a}";
    pub const COLOUR: &str = "\u{e6c8}";
    pub const FILM: &str = "\u{e792}";
    pub const MUSIC: &str = "\u{e340}";
    pub const IMAGE: &str = "\u{e2ca}";
    pub const VIDEO: &str = "\u{e740}";
    pub const SNAP: &str = "\u{e680}";
    pub const ZOOM_IN: &str = "\u{e310}";
    pub const ZOOM_OUT: &str = "\u{e30e}";
    pub const TO_START: &str = "\u{e5a4}";
    pub const TO_END: &str = "\u{e5a6}";
    pub const FRAME_BACK: &str = "\u{e138}";
    pub const FRAME_ON: &str = "\u{e13a}";
    pub const PEOPLE: &str = "\u{e68e}";
    pub const CONNECTED: &str = "\u{eb5a}";
    pub const SOUND: &str = "\u{e44a}";
    pub const MUTED: &str = "\u{e45a}";
    pub const ADD: &str = "\u{e3d4}";
    pub const IMPORT: &str = "\u{e236}";
    pub const AGENT: &str = "\u{e762}";
    pub const WEB: &str = "\u{e288}";
    pub const SCREEN: &str = "\u{e32e}";
    pub const WARNING: &str = "\u{e4e0}";
    pub const DONE: &str = "\u{e184}";
    pub const UPLOAD: &str = "\u{e1ae}";
    pub const FIT: &str = "\u{e534}";
    pub const SHOWN: &str = "\u{e220}";
    pub const HIDDEN: &str = "\u{e224}";
    pub const LOCKED: &str = "\u{e2fa}";
    pub const UNLOCKED: &str = "\u{e306}";
    pub const LOOP: &str = "\u{e3f6}";
    pub const ROW_HEIGHT: &str = "\u{eb04}";
    pub const MARKER: &str = "\u{e244}";
    pub const DETACH: &str = "\u{e2e4}";
    pub const WAVEFORM: &str = "\u{e802}";
    pub const ROLL: &str = "\u{e0a0}";
    pub const CAPTIONS: &str = "\u{e1a8}";
    pub const SEARCH: &str = "\u{e30c}";
    pub const MERGE: &str = "\u{ed3e}";
    pub const TRANSLATE: &str = "\u{e4a2}";
    pub const SAVE: &str = "\u{e20c}";
    pub const BRUSH: &str = "\u{e6f0}";
    pub const CLOCK: &str = "\u{e19a}";
    pub const CROP: &str = "\u{e1d4}";
    pub const STICKER: &str = "\u{e5ac}";
    pub const SPARKLE: &str = "\u{e6a2}";
    pub const SLIDERS: &str = "\u{e432}";
}

static ICONS: &[u8] = include_bytes!("../fonts/phosphor-icons.ttf");
/// Kana and the JIS level-1 kanji, for the Japanese interface (SIL OFL, see
/// `fonts/NOTO-SANS-JP-LICENSE`).
static NOTO_SANS_JP: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.subset.otf");

/// Gives `ctx` the palette, spacing and faces. Call once.
pub(crate) fn install(ctx: &egui::Context, japanese: bool) {
    set_fonts(ctx, japanese);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(style);
}

/// egui's own faces have no CJK or icons: after them come the face titles are drawn with (Noto
/// Sans SC), a Japanese face and the icon subset. For the Japanese interface the Japanese face
/// goes first, so kanji take their Japanese shapes.
pub(crate) fn set_fonts(ctx: &egui::Context, japanese: bool) {
    let mut defs = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("Noto Sans SC", reelforge_render::text::NOTO_SANS_SC_REGULAR),
        ("Noto Sans JP", NOTO_SANS_JP),
        ("Phosphor", ICONS),
    ] {
        defs.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
    }
    let cjk = if japanese { ["Noto Sans JP", "Noto Sans SC"] } else { ["Noto Sans SC", "Noto Sans JP"] };
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(family).or_default();
        list.extend(cjk.iter().map(|n| n.to_string()));
        list.push("Phosphor".into());
    }
    ctx.set_fonts(defs);
}

fn style(s: &mut egui::Style) {
    use color::*;
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::proportional(15.0)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    s.spacing.interact_size.y = 28.0;
    s.spacing.window_margin = egui::Margin::same(10);

    let v: &mut Visuals = &mut s.visuals;
    *v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = ROW;
    v.faint_bg_color = RAISED;
    v.code_bg_color = RAISED;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.override_text_color = None;
    v.hyperlink_color = ACCENT;
    // what is switched on is lighter glass with its mark in the accent, not a block of colour
    v.selection.bg_fill = Color32::from_white_alpha(36);
    v.selection.stroke = Stroke::new(1.0, ACCENT_2);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    let w = &mut v.widgets;
    let radius = CornerRadius::same(14);
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, WEAK);
    w.noninteractive.corner_radius = radius;
    for (state, fill, text) in [
        (&mut w.inactive, GLASS_CONTROL, TEXT),
        (&mut w.hovered, GLASS_HOVER, TEXT),
        (&mut w.active, GLASS_HOVER, TEXT),
        (&mut w.open, GLASS_HOVER, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.5, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(30));
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(14));
}

/// A button showing an icon (and, if given, a word), with a tooltip.
pub(crate) fn tool(ui: &mut egui::Ui, icon: &str, word: &str, tip: &str, enabled: bool) -> egui::Response {
    let text = if word.is_empty() { icon.to_string() } else { format!("{icon}  {word}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(14.0))).on_hover_text(tip)
}

/// The primary action: filled with the accent.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn primary(ui: &mut egui::Ui, icon: &str, word: &str, enabled: bool) -> egui::Response {
    accent_button(ui, &format!("{icon}  {word}"), 13.0, egui::vec2(0.0, 28.0), enabled)
}

/// A capsule filled with the accent, for the one thing a bar is for (play, export). It glows a
/// little, more under the pointer.
pub(crate) fn accent_button(
    ui: &mut egui::Ui,
    text: &str,
    size: f32,
    min: egui::Vec2,
    enabled: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(size), color::ON_ACCENT);
    let want = (galley.size() + egui::vec2(28.0, 10.0)).max(min);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(want, sense);
    if ui.is_rect_visible(rect) {
        let lit = ui.ctx().animate_bool(response.id, enabled && response.hovered());
        let radius = rect.height() / 2.0;
        let painter = ui.painter();
        if enabled {
            glow(painter, rect, radius, color::ACCENT.gamma_multiply(0.2 + 0.2 * lit), 12 + (8.0 * lit) as u8);
        }
        accent_rect(painter, rect, radius, if enabled { 1.0 } else { 0.3 });
        if response.is_pointer_button_down_on() {
            painter.rect_filled(rect, CornerRadius::same(radius as u8), Color32::from_black_alpha(50));
        }
        painter.galley(rect.center() - galley.size() / 2.0, galley, color::ON_ACCENT);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Keys as they are printed on them.
pub(crate) fn keycap(ui: &mut egui::Ui, keys: &str) -> egui::Response {
    egui::Frame::new()
        .fill(color::RAISED)
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(18)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| ui.label(egui::RichText::new(keys).monospace().size(11.5).color(color::TEXT)))
        .response
}

/// Keys and what they are for, as one thing to click.
pub(crate) fn hint_chip(ui: &mut egui::Ui, keys: &str, word: &str) -> egui::Response {
    let text = format!("{keys}   {word}");
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(12.5), color::WEAK);
    let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(24.0, 12.0), egui::Sense::click());
    let lit = ui.ctx().animate_bool(response.id, response.hovered());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(rect.height() as u8 / 2),
        color::RAISED.lerp_to_gamma(color::HOVER, lit),
        Stroke::new(1.0, color::ACCENT.gamma_multiply(0.15 + 0.5 * lit)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color::TEXT);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The mark and (where there is room for it) the name, at the start of the top bar.
pub(crate) fn brand(ui: &mut egui::Ui, name: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    let painter = ui.painter();
    accent_rect(painter, rect, 7.0, 1.0);
    // a play head, cut out of the light
    let c = rect.center() + egui::vec2(0.8, 0.0);
    let cut = vec![c + egui::vec2(-3.6, -5.0), c + egui::vec2(4.6, 0.0), c + egui::vec2(-3.6, 5.0)];
    painter.add(egui::Shape::convex_polygon(cut, color::ON_ACCENT, Stroke::NONE));
    if name {
        ui.label(egui::RichText::new("reelforge").strong().size(15.0).color(color::TEXT));
    }
}

/// The picture as the stage and the glass over it show it: where it is drawn, its light (`glow`:
/// the picture far out of focus, a few texels, the compositor's or [`out_of_focus`]) and, where
/// there is one, the picture as frosted glass shows it (the compositor's or [`frosted`]).
#[derive(Clone, Copy)]
pub(crate) struct Lit {
    pub glow: egui::TextureId,
    pub frost: Option<egui::TextureId>,
    pub frame: egui::Rect,
}

/// Where in the glow the light at `pos` comes from, and how much of it there is: light beside the
/// picture comes from the side of it that is nearest, and falls off with the distance.
fn light(frame: egui::Rect, strength: f32, pos: egui::Pos2) -> (egui::Pos2, Color32) {
    let reach = 1.5 * frame.width().max(frame.height());
    let fall = 0.3 + 0.7 * (1.0 - frame.distance_to_pos(pos) / reach * 1.6).clamp(0.0, 1.0).powi(2);
    // where along the picture (-1 to 1) the light at `a` half-widths from its middle comes from:
    // over half of the way out at its edge, and nearer the edge the further off
    let from = |a: f32| a.signum() * if a.abs() < 1.0 { 0.6 * a.abs() } else { 1.0 - 0.4 / a.abs() };
    let off = (pos - frame.center()) / (frame.size() * 0.5);
    let uv = egui::pos2(0.5 + 0.5 * from(off.x), 0.5 + 0.5 * from(off.y));
    (uv, Color32::WHITE.gamma_multiply(strength * 0.62 * fall))
}

/// The picture's light, on the stage and through the glass over it. `glow` is the picture far out
/// of focus, drawn once over all it lights. `strength` is 1 for a picture that is the only light.
pub(crate) fn ambient(painter: &egui::Painter, glow: egui::TextureId, frame: egui::Rect, strength: f32) {
    /// Cells across the lit area: the fall of the light is drawn through their corners.
    const CELLS: usize = 32;
    let reach = 1.5 * frame.width().max(frame.height());
    let lit = frame.expand(reach).intersect(painter.clip_rect());
    if lit.width() <= 0.0 || lit.height() <= 0.0 || frame.width() <= 0.0 || frame.height() <= 0.0 {
        return;
    }
    let (across, down) = (CELLS, ((CELLS as f32 * lit.height() / lit.width()).ceil() as usize).clamp(2, CELLS * 4));
    let mut mesh = egui::Mesh::with_texture(glow);
    for j in 0..=down {
        for i in 0..=across {
            let pos = lit.min + lit.size() * egui::vec2(i as f32 / across as f32, j as f32 / down as f32);
            let (uv, color) = light(frame, strength, pos);
            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
        }
    }
    let row = across as u32 + 1;
    for j in 0..down as u32 {
        for i in 0..across as u32 {
            let at = j * row + i;
            mesh.add_triangle(at, at + 1, at + row + 1);
            mesh.add_triangle(at, at + row + 1, at + row);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A capsule (a rectangle with round ends) of `texture`, each corner of its cells drawn as `at`
/// says: a clip rectangle has no round ends, a mesh of this shape does.
fn capsule_mesh(
    capsule: egui::Rect,
    texture: egui::TextureId,
    at: impl Fn(egui::Pos2) -> (egui::Pos2, Color32),
) -> egui::Mesh {
    const ACROSS: usize = 24;
    /// Rows, evenly along the round ends.
    const DOWN: usize = 16;
    let r = capsule.height() / 2.0;
    let mut mesh = egui::Mesh::with_texture(texture);
    for j in 0..=DOWN {
        let turn = std::f32::consts::PI * j as f32 / DOWN as f32;
        let y = capsule.center().y - r * turn.cos();
        // how far in the round ends are at this height
        let inset = (r - r * turn.sin()).min(capsule.width() / 2.0);
        for i in 0..=ACROSS {
            let x = egui::lerp(capsule.left() + inset..=capsule.right() - inset, i as f32 / ACROSS as f32);
            let pos = egui::pos2(x, y);
            let (uv, color) = at(pos);
            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
        }
    }
    let row = ACROSS as u32 + 1;
    for j in 0..DOWN as u32 {
        for i in 0..ACROSS as u32 {
            let at = j * row + i;
            mesh.add_triangle(at, at + 1, at + row + 1);
            mesh.add_triangle(at, at + row + 1, at + row);
        }
    }
    mesh
}

/// A capsule of glass floating over the stage. Where the picture lies behind it, the glass shows
/// it frosted: what is behind is covered by the stage and its light again, then by the picture
/// as the glass blurs it (egui cannot blur what it has drawn; the picture comes blurred from the
/// compositor), under the tint and the rim of the glass.
pub(crate) fn floating_capsule(painter: &egui::Painter, capsule: egui::Rect, lit: Option<Lit>) {
    let radius = capsule.height() / 2.0;
    if let Some(lit) = lit.filter(|l| l.frame.width() > 0.0 && l.frame.height() > 0.0) {
        // all of one shape, so that no layer shows beside another at the edge
        let stage = |_| (egui::epaint::WHITE_UV, color::STAGE);
        painter.add(egui::Shape::mesh(capsule_mesh(capsule, egui::TextureId::default(), stage)));
        painter.add(egui::Shape::mesh(capsule_mesh(capsule, lit.glow, |pos| light(lit.frame, 1.0, pos))));
        if let Some(frost) = lit.frost {
            let whole = lit.frame.expand2(lit.frame.size() * FROST_ROOM);
            // a little dimmed: what is on the glass stays readable over a white picture
            let seen = |pos: egui::Pos2| (((pos - whole.min) / whole.size()).to_pos2(), Color32::from_gray(184));
            painter.add(egui::Shape::mesh(capsule_mesh(capsule, frost, seen)));
        }
    }
    painter.rect(
        capsule,
        radius,
        color::GLASS,
        Stroke::new(1.0, Color32::from_white_alpha(34)),
        egui::StrokeKind::Inside,
    );
}

/// How much of each of the picture's sides the frost shows around the picture.
use reelforge_render::gpu::FROST_ROOM;

/// `image` as frosted glass over it shows it: blurred, with [`FROST_ROOM`] of its sides around it
/// where its light ends, and nothing in the outermost pixels (drawn past its edge, a texture
/// repeats them). What the GPU compositor makes of its own frames (`fs_frost`), for a frame drawn
/// without it: coarser, and blurred as the screen's numbers rather than as light.
pub(crate) fn frosted(image: &egui::ColorImage) -> egui::ColorImage {
    const SIDE: usize = 64;
    const TAPS: usize = 10;
    /// One deviation of the blur, as a part of the picture's longer side.
    const SPREAD: f32 = 0.025;
    let [w, h] = image.size;
    if w == 0 || h == 0 {
        return egui::ColorImage::filled([1, 1], Color32::TRANSPARENT);
    }
    let size = egui::vec2(w as f32, h as f32);
    let spread = SPREAD * size.max_elem();
    let mut pixels = vec![Color32::TRANSPARENT; SIDE * SIDE];
    for y in 1..SIDE - 1 {
        for x in 1..SIDE - 1 {
            let at = (egui::vec2(x as f32 + 0.5, y as f32 + 0.5) / SIDE as f32 * (1.0 + 2.0 * FROST_ROOM)
                - egui::Vec2::splat(FROST_ROOM))
                * size;
            let (mut sum, mut weight) = ([0.0f32; 4], 0.0);
            for j in 0..TAPS {
                for i in 0..TAPS {
                    // out to two and a half deviations
                    let d = egui::vec2(i as f32 + 0.5, j as f32 + 0.5) / TAPS as f32 * 5.0 - egui::Vec2::splat(2.5);
                    let k = (-0.5 * d.length_sq()).exp();
                    weight += k;
                    let p = at + d * spread;
                    if p.x >= 0.0 && p.y >= 0.0 && p.x < size.x && p.y < size.y {
                        let c = image.pixels[p.y as usize * w + p.x as usize].to_array();
                        for (s, v) in sum.iter_mut().zip(c) {
                            *s += k * v as f32;
                        }
                    }
                }
            }
            let [r, g, b, a] = sum.map(|s| (s / weight).round() as u8);
            pixels[y * SIDE + x] = Color32::from_rgba_premultiplied(r, g, b, a);
        }
    }
    egui::ColorImage::new([SIDE, SIDE], pixels)
}

/// `image` far out of focus, a few pixels a side: each the light of a wide part of the picture.
/// What the GPU compositor makes of its own frames (`fs_glow`), for a frame drawn without it.
pub(crate) fn out_of_focus(image: &egui::ColorImage) -> egui::ColorImage {
    const SIDE: usize = 16;
    const TAPS: usize = 16;
    /// One deviation of the blur, as a part of the picture's side.
    const SPREAD: f32 = 0.15;
    let [w, h] = image.size;
    if w == 0 || h == 0 {
        return egui::ColorImage::filled([1, 1], Color32::BLACK);
    }
    let mut pixels = Vec::with_capacity(SIDE * SIDE);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0);
            for j in 0..TAPS {
                for i in 0..TAPS {
                    // out to three deviations
                    let d = egui::vec2(i as f32 + 0.5, j as f32 + 0.5) / TAPS as f32 * 6.0 - egui::vec2(3.0, 3.0);
                    let k = (-0.5 * d.length_sq()).exp();
                    let at = (egui::vec2(x as f32 + 0.5, y as f32 + 0.5) / SIDE as f32 + d * SPREAD)
                        .clamp(egui::Vec2::ZERO, egui::Vec2::splat(1.0));
                    let px = ((at.x * (w - 1) as f32).round() as usize, (at.y * (h - 1) as f32).round() as usize);
                    let c = egui::Rgba::from(image.pixels[px.1 * w + px.0]);
                    for (s, v) in sum.iter_mut().zip([c.r(), c.g(), c.b()]) {
                        *s += k * v;
                    }
                    weight += k;
                }
            }
            let [r, g, b] = sum.map(|s| s / weight);
            pixels.push(Color32::from(egui::Rgba::from_rgb(r, g, b)));
        }
    }
    egui::ColorImage::new([SIDE, SIDE], pixels)
}

/// A rounded rectangle filled with the accent, shaded a little from left to right. A dimmed one
/// (`alpha` under 1) is for what is live but not the first thing to look at.
pub(crate) fn accent_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, alpha: f32) {
    gradient_rect(painter, rect, radius, color::ACCENT.gamma_multiply(alpha), color::ACCENT_2.gamma_multiply(alpha));
}

/// A rounded rectangle filled from `left` to `right`.
pub(crate) fn gradient_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, left: Color32, right: Color32) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let tint = |x: f32| left.lerp_to_gamma(right, ((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), tint(rect.center().x));
    // the corners, clockwise from the top left, each a quarter turn
    const STEPS: usize = 6;
    let corners = [
        (rect.left_top() + egui::vec2(r, r), 2),
        (rect.right_top() + egui::vec2(-r, r), 3),
        (rect.right_bottom() + egui::vec2(-r, -r), 0),
        (rect.left_bottom() + egui::vec2(r, -r), 1),
    ];
    for (centre, quarter) in corners {
        for i in 0..=STEPS {
            let a = (quarter as f32 + i as f32 / STEPS as f32) * std::f32::consts::FRAC_PI_2;
            let at = centre + r * egui::vec2(a.cos(), a.sin());
            mesh.colored_vertex(at, tint(at.x));
        }
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
    // a mesh has hard edges: a hairline in the middle colour softens them
    painter.rect_stroke(
        rect,
        CornerRadius::same(r as u8),
        Stroke::new(1.0, left.lerp_to_gamma(right, 0.5).gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
}

/// Light around `rect`, as what is selected or live gives off.
pub(crate) fn glow(painter: &egui::Painter, rect: egui::Rect, radius: f32, tint: Color32, blur: u8) {
    let shadow = egui::Shadow { offset: [0, 0], blur, spread: 0, color: tint };
    painter.add(shadow.as_shape(rect, CornerRadius::same(radius as u8)));
}

/// What a panel sits in: an island on the stage, with a rim that catches a little light.
pub(crate) fn island(margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(color::GLASS)
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(34)))
        .corner_radius(CornerRadius::same(ISLAND_RADIUS))
        .inner_margin(egui::Margin::same(margin))
}

/// How round an island's corners are.
pub(crate) const ISLAND_RADIUS: u8 = 22;
/// The gap between islands, and to the window's edge.
pub(crate) const GAP: i8 = 8;

/// A titled group on a panel: set into the glass, with no line around it.
pub(crate) fn section<R>(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(color::GLASS_INSET)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).size(11.5).color(color::WEAK).strong());
            ui.add_space(4.0);
            body(ui)
        })
        .inner
}

/// One of a few, as parts of one capsule as wide as there is room: the chosen part is raised.
/// Returns the part clicked, when it is another.
pub(crate) fn segmented(ui: &mut egui::Ui, words: &[&str], chosen: usize) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(14), color::ROW_ALT);
    let w = (rect.width() - 4.0) / words.len().max(1) as f32;
    let mut picked = None;
    for (i, word) in words.iter().enumerate() {
        let part = egui::Rect::from_min_size(rect.min + egui::vec2(2.0 + w * i as f32, 2.0), egui::vec2(w, 24.0));
        let r = ui.interact(part, ui.scope_id().with(("segment", *word)), egui::Sense::click());
        let on = i == chosen;
        let lit = ui.ctx().animate_bool_with_time(r.id, on, 0.12);
        let painter = ui.painter();
        if lit > 0.0 {
            painter.rect(
                part,
                CornerRadius::same(12),
                color::GLASS_HOVER.gamma_multiply(lit),
                Stroke::new(1.0, Color32::from_white_alpha((22.0 * lit) as u8)),
                egui::StrokeKind::Inside,
            );
        }
        let ink = if on || r.hovered() { color::TEXT } else { color::WEAK };
        painter.text(part.center(), egui::Align2::CENTER_CENTER, word, FontId::proportional(12.0), ink);
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}

/// How wide the names of a panel's fields are, at least: the values after them start in one line
/// down the panel, whatever section they are in.
pub(crate) const FIELD_NAME: f32 = 76.0;

/// The name of a field, before its value.
pub(crate) fn field_name(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.0), color::WEAK);
    let size = egui::vec2(galley.size().x.max(FIELD_NAME), galley.size().y.max(ui.spacing().interact_size.y - 4.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color::WEAK);
    response
}

/// A line that opens and closes what is below it: a small arrow and a name. Says whether it is
/// open; `open` is how it starts.
pub(crate) fn disclosure(ui: &mut egui::Ui, name: &str, open: bool) -> bool {
    let id = ui.scope_id().with(("disclosure", name));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
    let tone = if r.hovered() { color::TEXT } else { color::WEAK };
    let c = egui::pos2(rect.left() + 5.0, rect.center().y);
    let arrow = if open {
        vec![c + egui::vec2(-4.0, -2.5), c + egui::vec2(4.0, -2.5), c + egui::vec2(0.0, 3.0)]
    } else {
        vec![c + egui::vec2(-2.5, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.5, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(arrow, tone, egui::Stroke::NONE));
    let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(12.0), tone);
    ui.painter().galley(egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0), galley, tone);
    if r.clicked() {
        open = !open;
    }
    ui.data_mut(|d| d.insert_temp(id, open));
    open
}

/// `t` as `m:ss.cc`, or `h:mm:ss.cc` past an hour.
pub(crate) fn timecode(seconds: f64) -> String {
    let cs = (seconds.max(0.0) * 100.0).round() as u64;
    let (h, m, s, c) = (cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{c:02}") } else { format!("{m:02}:{s:02}.{c:02}") }
}

/// A ruler label: `0:05`, `1:30`.
pub(crate) fn ruler_label(seconds: f64) -> String {
    let s = seconds.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_behind_glass_is_blurred_and_ends_softly_around_itself() {
        use eframe::egui::{Color32, ColorImage};
        let frost = super::frosted(&ColorImage::filled([40, 70], Color32::from_rgb(200, 100, 0)));
        let [w, h] = frost.size;
        let at = |x: usize, y: usize| frost.pixels[y * w + x];
        // the picture's own colour in the middle, half of it at its edge, nothing at the texture's
        assert_eq!(at(w / 2, h / 2), Color32::from_rgb(200, 100, 0));
        let edge = (super::FROST_ROOM / (1.0 + 2.0 * super::FROST_ROOM) * w as f32) as usize;
        assert!((90..=165).contains(&at(edge, h / 2).a()), "{:?}", at(edge, h / 2));
        assert!((0..w / 2).all(|x| at(x, h / 2).a() <= at(x + 1, h / 2).a()));
        assert!((0..w).all(|i| [at(i, 0), at(i, h - 1), at(0, i), at(w - 1, i)] == [Color32::TRANSPARENT; 4]));
        // a hard edge in the picture is a soft one behind the glass
        let mut half = ColorImage::filled([256, 256], Color32::BLACK);
        for y in 0..256 {
            for x in 0..128 {
                half.pixels[y * 256 + x] = Color32::WHITE;
            }
        }
        let frost = super::frosted(&half);
        let row: Vec<u8> = (edge + 6..w - edge - 6).map(|x| frost.pixels[h / 2 * w + x].r()).collect();
        assert!(row[0] > 250 && *row.last().unwrap() < 5, "{row:?}");
        assert!(row.windows(2).all(|p| p[0] >= p[1] && p[0] - p[1] < 120), "{row:?}");
    }

    #[test]
    fn glass_over_the_stage_is_a_mesh_with_round_ends() {
        use eframe::egui;
        let capsule = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(400.0, 50.0));
        let mesh = super::capsule_mesh(capsule, egui::TextureId::default(), |pos| (pos, egui::Color32::WHITE));
        assert!(mesh.is_valid());
        let r = 25.0;
        let (left, right) = (egui::pos2(125.0, 65.0), egui::pos2(475.0, 65.0));
        for v in &mesh.vertices {
            // no further from the line through the middle than the ends are round
            let nearest = egui::pos2(v.pos.x.clamp(left.x, right.x), 65.0);
            assert!(v.pos.distance(nearest) <= r + 0.01, "{:?}", v.pos);
        }
        // and as far as the capsule goes
        let bounds = mesh.calc_bounds();
        assert!((bounds.min - capsule.min).length() < 0.01 && (bounds.max - capsule.max).length() < 0.01, "{bounds:?}");
    }

    #[test]
    fn a_picture_out_of_focus_keeps_its_light_and_loses_its_shapes() {
        use eframe::egui::{Color32, ColorImage};
        // one colour stays that colour
        let flat = super::out_of_focus(&ColorImage::filled([40, 70], Color32::from_rgb(200, 100, 0)));
        assert!(flat.pixels.iter().all(|p| *p == Color32::from_rgb(200, 100, 0)), "{:?}", flat.pixels[0]);
        // white on the left, black on the right: bright, then dark, with no edge between
        let mut half = ColorImage::filled([64, 64], Color32::BLACK);
        for y in 0..64 {
            for x in 0..32 {
                half.pixels[y * 64 + x] = Color32::WHITE;
            }
        }
        let glow = super::out_of_focus(&half);
        let row: Vec<u8> = (0..16).map(|x| glow.pixels[8 * 16 + x].r()).collect();
        assert!(row[0] > 240 && row[15] < 40, "{row:?}");
        assert!(row.windows(2).all(|p| p[0] >= p[1]), "{row:?}");
        assert!(row.iter().filter(|v| (20..236).contains(*v)).count() >= 5, "{row:?}");
    }

    #[test]
    fn timecodes_read_as_editors_write_them() {
        assert_eq!(super::timecode(0.0), "00:00.00");
        assert_eq!(super::timecode(65.432), "01:05.43");
        assert_eq!(super::timecode(3725.0), "1:02:05.00");
        assert_eq!(super::ruler_label(90.0), "1:30");
    }
}
