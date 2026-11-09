//! How the editor looks: one dark palette for desktop and browser, the faces it draws with, and
//! the icons (a subset of Phosphor, MIT, in `fonts/`; egui's own faces have no icons, and emoji
//! fall back to boxes on many desktops).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// The palette.
pub(crate) mod color {
    use super::Color32;

    /// Behind everything; the preview's surroundings.
    pub const STAGE: Color32 = Color32::from_rgb(0x0e, 0x0f, 0x12);
    /// Panels.
    pub const PANEL: Color32 = Color32::from_rgb(0x18, 0x19, 0x1e);
    /// Raised areas on panels: headers, section frames, inputs.
    pub const RAISED: Color32 = Color32::from_rgb(0x21, 0x22, 0x29);
    /// Hovered controls.
    pub const HOVER: Color32 = Color32::from_rgb(0x2c, 0x2e, 0x37);
    /// Hairlines between areas.
    pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x2d, 0x35);
    /// Timeline rows.
    pub const ROW: Color32 = Color32::from_rgb(0x13, 0x14, 0x18);
    pub const ROW_ALT: Color32 = Color32::from_rgb(0x16, 0x17, 0x1c);
    pub const TEXT: Color32 = Color32::from_rgb(0xe4, 0xe5, 0xea);
    pub const WEAK: Color32 = Color32::from_rgb(0x8a, 0x8d, 0x99);
    /// Selection, the primary action, focus.
    pub const ACCENT: Color32 = Color32::from_rgb(0x2b, 0xd4, 0xc4);
    /// Text on the accent.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x06, 0x22, 0x20);
    pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0x4f, 0x6d);
    pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x41);
    /// Clips, by what they hold.
    pub const VIDEO: Color32 = Color32::from_rgb(0x3b, 0x68, 0xc9);
    pub const TEXT_CLIP: Color32 = Color32::from_rgb(0xb9, 0x78, 0x2e);
    pub const AUDIO: Color32 = Color32::from_rgb(0x27, 0x95, 0x6c);
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
    s.spacing.button_padding = egui::vec2(9.0, 5.0);
    s.spacing.interact_size.y = 26.0;
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
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    let w = &mut v.widgets;
    let radius = CornerRadius::same(6);
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

/// The primary action: filled with the accent.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn primary(ui: &mut egui::Ui, icon: &str, word: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(format!("{icon}  {word}")).color(color::ON_ACCENT).strong())
            .fill(color::ACCENT),
    )
}

/// A titled group on a panel.
pub(crate) fn section<R>(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(color::RAISED.gamma_multiply(0.6))
        .stroke(Stroke::new(1.0, color::LINE))
        .corner_radius(CornerRadius::same(8))
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
