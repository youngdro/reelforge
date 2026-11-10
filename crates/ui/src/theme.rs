//! How the editor looks: one dark palette for desktop and browser, the faces it draws with, and
//! the icons (a subset of Phosphor, MIT, in `fonts/`; egui's own faces have no icons, and emoji
//! fall back to boxes on many desktops).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// The palette: ink-dark glass that takes its light from the picture, and one aurora (violet to
/// cyan) for what is live: play, the playhead, the selection.
pub(crate) mod color {
    use super::Color32;

    /// Behind everything; the preview's surroundings.
    pub const STAGE: Color32 = Color32::from_rgb(0x07, 0x08, 0x0c);
    /// Panels: islands on the stage.
    pub const PANEL: Color32 = Color32::from_rgb(0x11, 0x12, 0x19);
    /// Raised areas on panels: headers, section frames, inputs.
    pub const RAISED: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x26);
    /// Hovered controls.
    pub const HOVER: Color32 = Color32::from_rgb(0x27, 0x2a, 0x3a);
    /// Hairlines between areas.
    pub const LINE: Color32 = Color32::from_rgb(0x22, 0x24, 0x32);
    /// Timeline rows.
    pub const ROW: Color32 = Color32::from_rgb(0x0c, 0x0d, 0x13);
    pub const ROW_ALT: Color32 = Color32::from_rgb(0x0f, 0x10, 0x17);
    pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xef, 0xf6);
    pub const WEAK: Color32 = Color32::from_rgb(0x86, 0x8a, 0xa3);
    /// Selection, the primary action, focus: the aurora's violet end.
    pub const ACCENT: Color32 = Color32::from_rgb(0x93, 0x82, 0xff);
    /// The aurora's cyan end.
    pub const ACCENT_2: Color32 = Color32::from_rgb(0x36, 0xe2, 0xd0);
    /// Text on the accent.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0b, 0x09, 0x22);
    /// What records, and what is about to be cut out.
    pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0x4f, 0x6d);
    pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x41);
    /// Clips, by what they hold.
    pub const VIDEO: Color32 = Color32::from_rgb(0x4b, 0x6b, 0xe8);
    pub const TEXT_CLIP: Color32 = Color32::from_rgb(0xe0, 0x8a, 0x3c);
    pub const AUDIO: Color32 = Color32::from_rgb(0x21, 0xb0, 0x8a);
    /// Adjustment clips: effects and colour for what is below.
    pub const FX: Color32 = Color32::from_rgb(0xa0, 0x62, 0xe8);
    /// Compound clips.
    pub const COMPOUND: Color32 = Color32::from_rgb(0x2f, 0x9c, 0xc4);

    /// A point along the aurora: 0 is violet, 1 cyan.
    pub fn aurora(t: f32) -> Color32 {
        ACCENT.lerp_to_gamma(ACCENT_2, t.clamp(0.0, 1.0))
    }
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
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    let w = &mut v.widgets;
    let radius = CornerRadius::same(8);
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, WEAK);
    w.noninteractive.corner_radius = radius;
    for (state, fill, text) in [
        (&mut w.inactive, RAISED, TEXT),
        (&mut w.hovered, HOVER, TEXT),
        (&mut w.active, HOVER, ACCENT),
        (&mut w.open, HOVER, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.5, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, LINE);
}

/// A button showing an icon (and, if given, a word), with a tooltip.
pub(crate) fn tool(ui: &mut egui::Ui, icon: &str, word: &str, tip: &str, enabled: bool) -> egui::Response {
    let text = if word.is_empty() { icon.to_string() } else { format!("{icon}  {word}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(14.0))).on_hover_text(tip)
}

/// The primary action: filled with the aurora.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn primary(ui: &mut egui::Ui, icon: &str, word: &str, enabled: bool) -> egui::Response {
    aurora_button(ui, &format!("{icon}  {word}"), 13.0, egui::vec2(0.0, 28.0), enabled)
}

/// A capsule filled with the aurora, for the one thing a bar is for (play, export). It glows, more
/// under the pointer.
pub(crate) fn aurora_button(
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
            glow(painter, rect, radius, color::aurora(0.4).gamma_multiply(0.22 + 0.3 * lit), 14 + (10.0 * lit) as u8);
        }
        aurora_rect(painter, rect, radius, if enabled { 1.0 } else { 0.3 });
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

/// The mark and the name, at the start of the top bar.
pub(crate) fn brand(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    let painter = ui.painter();
    glow(painter, rect, 7.0, color::aurora(0.4).gamma_multiply(0.3), 12);
    aurora_rect(painter, rect, 7.0, 1.0);
    // a play head, cut out of the light
    let c = rect.center() + egui::vec2(0.8, 0.0);
    let cut = vec![c + egui::vec2(-3.6, -5.0), c + egui::vec2(4.6, 0.0), c + egui::vec2(-3.6, 5.0)];
    painter.add(egui::Shape::convex_polygon(cut, color::ON_ACCENT, Stroke::NONE));
    ui.label(egui::RichText::new("reelforge").strong().size(15.0).color(color::TEXT));
}

/// The picture's light on the stage around it. There is no blurred copy of the frame to draw, so
/// the light is laid down as layers of flat tiles, each tile one point of the picture, each layer
/// shifted against the others: together they are the picture out of focus. `strength` is 1 for a
/// picture that is the only light.
pub(crate) fn ambient(painter: &egui::Painter, texture: egui::TextureId, frame: egui::Rect, strength: f32) {
    const LAYERS: usize = 24;
    /// Tiles across the lit area.
    const TILES: usize = 9;
    let reach = 0.55 * frame.width().max(frame.height());
    let lit = frame.expand(reach).intersect(painter.clip_rect());
    if lit.width() <= 0.0 || lit.height() <= 0.0 || reach <= 0.0 {
        return;
    }
    let tile = (lit.width().max(lit.height()) / TILES as f32).max(8.0);
    // light falls off with the distance from the picture's edge
    let fall = |at: egui::Pos2| {
        let d = frame.distance_to_pos(at) / reach;
        (1.0 - d).clamp(0.0, 1.0).powi(2)
    };
    let alpha = strength * 0.6 / LAYERS as f32;
    for layer in 0..LAYERS {
        let shift = tile * egui::vec2((layer as f32 * 0.618).fract(), (layer as f32 * 0.382 + 0.5).fract());
        let mut mesh = egui::Mesh::with_texture(texture);
        let mut y = lit.top() - shift.y;
        while y < lit.bottom() {
            let mut x = lit.left() - shift.x;
            while x < lit.right() {
                let cell = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(tile, tile));
                // the point of the picture this tile shows: the nearest part, pulled a little inwards
                let from = (cell.center() - frame.center()) / (frame.size() + egui::vec2(reach, reach) * 2.0);
                let uv = egui::pos2((0.5 + from.x * 1.5).clamp(0.04, 0.96), (0.5 + from.y * 1.5).clamp(0.04, 0.96));
                let at = mesh.vertices.len() as u32;
                for pos in [cell.left_top(), cell.right_top(), cell.right_bottom(), cell.left_bottom()] {
                    let color = Color32::WHITE.gamma_multiply(alpha * fall(pos));
                    mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
                }
                mesh.add_triangle(at, at + 1, at + 2);
                mesh.add_triangle(at, at + 2, at + 3);
                x += tile;
            }
            y += tile;
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// A rounded rectangle filled along the aurora, left to right. A dimmed one (`alpha` under 1) is for
/// what is live but not the first thing to look at.
pub(crate) fn aurora_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, alpha: f32) {
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
        .fill(color::PANEL)
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(10)))
        .corner_radius(CornerRadius::same(ISLAND_RADIUS))
        .inner_margin(egui::Margin::same(margin))
}

/// How round an island's corners are.
pub(crate) const ISLAND_RADIUS: u8 = 14;
/// The gap between islands, and to the window's edge.
pub(crate) const GAP: i8 = 6;

/// A titled group on a panel.
pub(crate) fn section<R>(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(color::RAISED.gamma_multiply(0.6))
        .stroke(Stroke::new(1.0, color::LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).small().color(color::WEAK).strong());
            ui.add_space(2.0);
            body(ui)
        })
        .inner
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
    fn timecodes_read_as_editors_write_them() {
        assert_eq!(super::timecode(0.0), "00:00.00");
        assert_eq!(super::timecode(65.432), "01:05.43");
        assert_eq!(super::timecode(3725.0), "1:02:05.00");
        assert_eq!(super::ruler_label(90.0), "1:30");
    }
}
