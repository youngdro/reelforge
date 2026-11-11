//! Title templates ("花字"): ready-made looks a title takes in one step, by the UI's picker or an
//! agent's `"template"` parameter. A template sets the title's colour and style; the words, size
//! and placement stay.

use reelforge_model::{Backing, Bevel, Depth, Outline, Palette, PaletteUnit, Scatter, Shadow, TitleFill, TitleStyle};

/// A ready-made title look.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleTemplate {
    /// Stable id, as commands take it.
    pub id: &'static str,
    /// English name.
    pub name: &'static str,
    /// Chinese name.
    pub name_zh: &'static str,
    /// The shelf a picker shows it on.
    pub group: TitleGroup,
    /// Text colour, straight sRGB.
    pub color: [f32; 4],
    /// Everything else.
    pub style: TitleStyle,
}

/// The shelves a picker sorts the templates onto.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TitleGroup {
    /// Plain subtitles.
    Basic,
    /// Loud variety-show lettering.
    Variety,
    /// Fills through several colours.
    Gradient,
    /// Metal and gloss.
    Texture,
    /// Neon and glows.
    Glow,
    /// Letters with depth.
    Solid,
    /// Hollow letters and stripes.
    Hollow,
    /// A colour per character, letters out of line or round a curve.
    Playful,
    /// Boxes, bands and tiles behind the words.
    Backing,
    /// Sale and price lettering.
    Promo,
}

impl TitleGroup {
    /// Every group, in a picker's order.
    pub const ALL: [TitleGroup; 10] = [
        TitleGroup::Basic,
        TitleGroup::Variety,
        TitleGroup::Gradient,
        TitleGroup::Texture,
        TitleGroup::Glow,
        TitleGroup::Solid,
        TitleGroup::Hollow,
        TitleGroup::Playful,
        TitleGroup::Backing,
        TitleGroup::Promo,
    ];

    /// The name commands use.
    pub fn id(self) -> &'static str {
        match self {
            TitleGroup::Basic => "basic",
            TitleGroup::Variety => "variety",
            TitleGroup::Gradient => "gradient",
            TitleGroup::Texture => "texture",
            TitleGroup::Glow => "glow",
            TitleGroup::Solid => "solid",
            TitleGroup::Hollow => "hollow",
            TitleGroup::Playful => "playful",
            TitleGroup::Backing => "backing",
            TitleGroup::Promo => "promo",
        }
    }
}

fn rgb(hex: u32) -> [f32; 4] {
    rgba(hex, 1.0)
}

fn rgba(hex: u32, alpha: f32) -> [f32; 4] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0, alpha]
}

fn line(color: [f32; 4], width: f32) -> Option<Outline> {
    Some(Outline { color, width })
}

fn look(
    id: &'static str,
    name: &'static str,
    name_zh: &'static str,
    group: TitleGroup,
    color: [f32; 4],
    style: TitleStyle,
) -> TitleTemplate {
    TitleTemplate { id, name, name_zh, group, color, style }
}

/// The templates, in the order a picker shows them: group by group.
#[rustfmt::skip]
pub fn title_templates() -> Vec<TitleTemplate> {
    let base = TitleStyle::default();
    vec![
        look("classic", "Classic", "经典字幕", TitleGroup::Basic, rgb(0xffffff), TitleStyle { outline: line(rgb(0x000000), 0.08), ..base.clone() }),
        look("soft", "Soft shadow", "柔和投影", TitleGroup::Basic, rgb(0xffffff), TitleStyle { shadow: Some(Shadow { color: rgba(0x000000, 0.7), offset: [0.0, 0.04], blur: 0.1 }), ..base.clone() }),
        look("clean", "Clean", "简洁细边", TitleGroup::Basic, rgb(0xffffff), TitleStyle { weight: 500, outline: line(rgb(0x1c1c22), 0.035), shadow: Some(Shadow { color: rgba(0x000000, 0.45), offset: [0.0, 0.03], blur: 0.06 }), ..base.clone() }),
        look("ink", "Ink", "墨黑白边", TitleGroup::Basic, rgb(0x16161a), TitleStyle { weight: 900, outline: line(rgb(0xffffff), 0.09), shadow: Some(Shadow { color: rgba(0x000000, 0.35), offset: [0.0, 0.04], blur: 0.06 }), ..base.clone() }),
        look("cream", "Cream", "奶油", TitleGroup::Basic, rgb(0xfff4d6), TitleStyle { weight: 900, outline: line(rgb(0x5b3a1e), 0.08), ..base.clone() }),
        look("variety", "Variety", "综艺黄字", TitleGroup::Variety, rgb(0xffe14d), TitleStyle { weight: 900, outline: line(rgb(0x000000), 0.1), shadow: Some(Shadow { color: rgba(0x000000, 1.0), offset: [0.04, 0.06], blur: 0.0 }), ..base.clone() }),
        look("sticker", "Sticker", "贴纸", TitleGroup::Variety, rgb(0xffffff), TitleStyle { weight: 900, outline: line(rgb(0x1d1d2b), 0.07), outer: line(rgb(0xffffff), 0.09), shadow: Some(Shadow { color: rgba(0x000000, 0.45), offset: [0.0, 0.05], blur: 0.06 }), ..base.clone() }),
        look("retro", "Retro", "复古", TitleGroup::Variety, rgb(0xff9f1c), TitleStyle { weight: 900, outline: line(rgb(0x1b1f3b), 0.06), shadow: Some(Shadow { color: rgba(0x2ec4b6, 1.0), offset: [0.07, 0.07], blur: 0.0 }), ..base.clone() }),
        look("pop", "Pop", "波普", TitleGroup::Variety, rgb(0xffe600), TitleStyle { weight: 900, outline: line(rgb(0x15151a), 0.07), depth: Some(Depth { color: rgb(0xff2d55), offset: [0.08, 0.08], fade: None }), ..base.clone() }),
        look("comic", "Comic", "漫画", TitleGroup::Variety, rgb(0xffffff), TitleStyle { weight: 900, outline: line(rgb(0x15151a), 0.08), depth: Some(Depth { color: rgb(0x2979ff), offset: [0.09, 0.09], fade: None }), ..base.clone() }),
        look("bubble", "Bubble", "泡泡", TitleGroup::Variety, rgb(0xff8ac0), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffb3d6), rgb(0xff5fa2)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.08), outer: line(rgb(0xe0457f), 0.05), bevel: Some(Bevel { light: rgba(0xffffff, 0.85), dark: rgba(0xb0155a, 0.35), size: 0.05, angle: 315.0 }), shadow: Some(Shadow { color: rgba(0xb0155a, 0.35), offset: [0.0, 0.06], blur: 0.08 }), ..base.clone() }),
        look("shout", "Shout", "呐喊", TitleGroup::Variety, rgb(0xff1744), TitleStyle { weight: 900, outline: line(rgb(0xffffff), 0.08), outer: line(rgb(0x15151a), 0.06), scatter: Some(Scatter { rotate: 7.0, shift: 0.06, scale: 0.1, seed: 7 }), ..base.clone() }),
        look("candy", "Candy", "糖果渐变", TitleGroup::Gradient, rgb(0xfff27a), TitleStyle { weight: 900, gradient: Some(rgb(0xff5fa2)), outline: line(rgb(0xffffff), 0.07), outer: line(rgb(0xc2185b), 0.06), ..base.clone() }),
        look("fire", "Fire", "火焰", TitleGroup::Gradient, rgb(0xfff3a0), TitleStyle { weight: 900, gradient: Some(rgb(0xff3d00)), outline: line(rgb(0x5a0a00), 0.08), shadow: Some(Shadow { color: rgba(0xff5900, 0.6), offset: [0.0, 0.0], blur: 0.18 }), ..base.clone() }),
        look("sunset", "Sunset", "日落", TitleGroup::Gradient, rgb(0xffd86b), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffd86b), rgb(0xff7a59), rgb(0xd9368b)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x4a1042), 0.06), ..base.clone() }),
        look("ocean", "Ocean", "海洋", TitleGroup::Gradient, rgb(0x9be7ff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xc9f3ff), rgb(0x3d8bff), rgb(0x1b3fa0)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.06), shadow: Some(Shadow { color: rgba(0x0b2a7a, 0.6), offset: [0.0, 0.05], blur: 0.08 }), ..base.clone() }),
        look("aurora", "Aurora", "极光", TitleGroup::Gradient, rgb(0x5ef2c2), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0x5ef2c2), rgb(0x4aa8ff), rgb(0xb46bff)], angle: 0.0, lines: false, bands: false, repeat: 1.0 }), shadow: Some(Shadow { color: rgba(0x4aa8ff, 0.55), offset: [0.0, 0.0], blur: 0.16 }), ..base.clone() }),
        look("rainbow", "Rainbow", "彩虹", TitleGroup::Gradient, rgb(0xff5252), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xff5252), rgb(0xffb74d), rgb(0xfff176), rgb(0x69f0ae), rgb(0x40c4ff), rgb(0xb388ff)], angle: 0.0, lines: false, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.07), outer: line(rgb(0x22222a), 0.04), ..base.clone() }),
        look("mint", "Mint", "薄荷", TitleGroup::Gradient, rgb(0xeafff3), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xf2fff8), rgb(0x5fe3a1)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x0f6b4a), 0.07), ..base.clone() }),
        look("peach", "Peach", "蜜桃", TitleGroup::Gradient, rgb(0xffd9c2), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xfff0e0), rgb(0xffa07a), rgb(0xff6f91)], angle: 60.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.07), shadow: Some(Shadow { color: rgba(0xd9486e, 0.5), offset: [0.0, 0.05], blur: 0.07 }), ..base.clone() }),
        look("gold", "Gold", "鎏金", TitleGroup::Texture, rgb(0xf5c542), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xfff6c2), rgb(0xf5c542), rgb(0x9a6a00), rgb(0xffe27a), rgb(0xb8860b)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x4a2e00), 0.05), bevel: Some(Bevel { light: rgba(0xfffbe0, 0.9), dark: rgba(0x4a2e00, 0.55), size: 0.035, angle: 315.0 }), shadow: Some(Shadow { color: rgba(0x000000, 0.5), offset: [0.0, 0.05], blur: 0.07 }), ..base.clone() }),
        look("silver", "Silver", "亮银", TitleGroup::Texture, rgb(0xd9dfe6), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffffff), rgb(0xc9d1d9), rgb(0x6b7785), rgb(0xe6ebf0), rgb(0x8a94a3)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x2b3038), 0.05), bevel: Some(Bevel { light: rgba(0xffffff, 0.9), dark: rgba(0x1b2028, 0.5), size: 0.035, angle: 315.0 }), shadow: Some(Shadow { color: rgba(0x000000, 0.5), offset: [0.0, 0.05], blur: 0.07 }), ..base.clone() }),
        look("chrome", "Chrome", "镀铬", TitleGroup::Texture, rgb(0xeaf7ff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0x6fc3ff), rgb(0xeaf7ff), rgb(0x1a2540), rgb(0xb98b5e), rgb(0xfff1d0)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.04), outer: line(rgb(0x1a2540), 0.05), ..base.clone() }),
        look("rose", "Rose gold", "玫瑰金", TitleGroup::Texture, rgb(0xf4a6a0), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffe9e4), rgb(0xf4a6a0), rgb(0xb76e79), rgb(0xffd3cc)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x5e2a33), 0.05), bevel: Some(Bevel { light: rgba(0xfff3f0, 0.9), dark: rgba(0x5e2a33, 0.5), size: 0.035, angle: 315.0 }), ..base.clone() }),
        look("bronze", "Bronze", "古铜", TitleGroup::Texture, rgb(0xc77b30), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffd9a0), rgb(0xc77b30), rgb(0x6b3a12), rgb(0xe0a060)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x2e1505), 0.06), bevel: Some(Bevel { light: rgba(0xffe9c4, 0.8), dark: rgba(0x2e1505, 0.5), size: 0.035, angle: 315.0 }), ..base.clone() }),
        look("emboss", "Emboss", "浮雕", TitleGroup::Texture, rgb(0xe9e4d8), TitleStyle { weight: 900, bevel: Some(Bevel { light: rgba(0xffffff, 0.95), dark: rgba(0x000000, 0.55), size: 0.06, angle: 315.0 }), shadow: Some(Shadow { color: rgba(0x000000, 0.55), offset: [0.03, 0.04], blur: 0.04 }), ..base.clone() }),
        look("jelly", "Jelly", "果冻", TitleGroup::Texture, rgb(0x7fe3ff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xb8f3ff), rgb(0x35c5f0)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.06), bevel: Some(Bevel { light: rgba(0xffffff, 0.9), dark: rgba(0x00668a, 0.45), size: 0.07, angle: 315.0 }), shadow: Some(Shadow { color: rgba(0x008fc0, 0.5), offset: [0.0, 0.06], blur: 0.08 }), ..base.clone() }),
        look("neon", "Neon", "霓虹", TitleGroup::Glow, rgb(0xe9fffd), TitleStyle { outline: line(rgb(0x2bd4c4), 0.04), shadow: Some(Shadow { color: rgba(0x2bd4c4, 0.9), offset: [0.0, 0.0], blur: 0.22 }), ..base.clone() }),
        look("neonpink", "Neon pink", "粉色霓虹", TitleGroup::Glow, rgb(0xfff0fa), TitleStyle { outline: line(rgb(0xff4fd8), 0.035), shadow: Some(Shadow { color: rgba(0xff4fd8, 0.95), offset: [0.0, 0.0], blur: 0.1 }), shadows: vec![Shadow { color: rgba(0xff2bb5, 0.7), offset: [0.0, 0.0], blur: 0.34 }], ..base.clone() }),
        look("tube", "Neon tube", "霓虹灯管", TitleGroup::Glow, rgb(0xfffbe6), TitleStyle { weight: 900, hollow: Some(0.045), shadow: Some(Shadow { color: rgba(0xffb300, 0.95), offset: [0.0, 0.0], blur: 0.07 }), shadows: vec![Shadow { color: rgba(0xff6d00, 0.75), offset: [0.0, 0.0], blur: 0.28 }], ..base.clone() }),
        look("glitch", "Glitch", "故障", TitleGroup::Glow, rgb(0xffffff), TitleStyle { weight: 900, shadow: Some(Shadow { color: rgba(0x00f0ff, 1.0), offset: [-0.045, 0.0], blur: 0.0 }), shadows: vec![Shadow { color: rgba(0xff00aa, 1.0), offset: [0.045, 0.0], blur: 0.0 }], ..base.clone() }),
        look("halo", "Halo", "光晕", TitleGroup::Glow, rgb(0xffffff), TitleStyle { shadow: Some(Shadow { color: rgba(0xffffff, 0.9), offset: [0.0, 0.0], blur: 0.12 }), shadows: vec![Shadow { color: rgba(0x7ab8ff, 0.7), offset: [0.0, 0.0], blur: 0.4 }], ..base.clone() }),
        look("laser", "Laser", "镭射", TitleGroup::Glow, rgb(0xc774e8), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xff6ad5), rgb(0xc774e8), rgb(0xad8cff), rgb(0x8795e8), rgb(0x94d0ff)], angle: 20.0, lines: false, bands: false, repeat: 1.0 }), outline: line(rgb(0xffffff), 0.03), shadow: Some(Shadow { color: rgba(0xad8cff, 0.8), offset: [0.0, 0.0], blur: 0.2 }), ..base.clone() }),
        look("block", "3D block", "立体块", TitleGroup::Solid, rgb(0xffd60a), TitleStyle { weight: 900, outline: line(rgb(0x1b1b1b), 0.05), depth: Some(Depth { color: rgb(0xc77700), offset: [0.1, 0.1], fade: Some(rgb(0x7a3e00)) }), ..base.clone() }),
        look("tower", "Tower", "高楼", TitleGroup::Solid, rgb(0xffffff), TitleStyle { weight: 900, outline: line(rgb(0x1a237e), 0.04), depth: Some(Depth { color: rgb(0x3d5afe), offset: [0.0, 0.16], fade: Some(rgb(0x1a237e)) }), ..base.clone() }),
        look("longshadow", "Long shadow", "长投影", TitleGroup::Solid, rgb(0xffffff), TitleStyle { weight: 900, depth: Some(Depth { color: rgba(0x000000, 0.4), offset: [0.4, 0.4], fade: Some(rgba(0x000000, 0.0)) }), ..base.clone() }),
        look("layers", "Layers", "多层叠色", TitleGroup::Solid, rgb(0xffffff), TitleStyle { weight: 900, outline: line(rgb(0x15151a), 0.05), shadow: Some(Shadow { color: rgba(0xff5fa2, 1.0), offset: [0.05, 0.05], blur: 0.0 }), shadows: vec![Shadow { color: rgba(0x4dd0e1, 1.0), offset: [0.1, 0.1], blur: 0.0 }, Shadow { color: rgba(0xffe14d, 1.0), offset: [0.15, 0.15], blur: 0.0 }], ..base.clone() }),
        look("arcade", "Arcade", "街机", TitleGroup::Solid, rgb(0xff9800), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xfff59d), rgb(0xff9800), rgb(0xf44336)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x311b92), 0.06), depth: Some(Depth { color: rgb(0x7c4dff), offset: [0.07, 0.1], fade: Some(rgb(0x311b92)) }), ..base.clone() }),
        look("milk", "Milk 3D", "奶油立体", TitleGroup::Solid, rgb(0xfff8e7), TitleStyle { weight: 900, outline: line(rgb(0xff8fab), 0.06), depth: Some(Depth { color: rgb(0xff8fab), offset: [0.05, 0.08], fade: Some(rgb(0xe05780)) }), shadow: Some(Shadow { color: rgba(0x8a2846, 0.35), offset: [0.0, 0.06], blur: 0.08 }), ..base.clone() }),
        look("hollow", "Hollow", "空心", TitleGroup::Hollow, rgb(0xffffff), TitleStyle { weight: 900, hollow: Some(0.05), ..base.clone() }),
        look("hollowpop", "Hollow pop", "空心叠色", TitleGroup::Hollow, rgb(0xffffff), TitleStyle { weight: 900, hollow: Some(0.05), shadow: Some(Shadow { color: rgba(0xff4f9a, 1.0), offset: [0.06, 0.06], blur: 0.0 }), ..base.clone() }),
        look("stripes", "Stripes", "斜条纹", TitleGroup::Hollow, rgb(0xffffff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffffff), rgb(0xff5252)], angle: 45.0, lines: false, bands: true, repeat: 8.0 }), outline: line(rgb(0x7a0000), 0.06), outer: line(rgb(0xffffff), 0.05), ..base.clone() }),
        look("candycane", "Candy cane", "拐杖糖", TitleGroup::Hollow, rgb(0xffffff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffffff), rgb(0xe53935), rgb(0xffffff), rgb(0x43a047)], angle: 45.0, lines: false, bands: true, repeat: 5.0 }), outline: line(rgb(0x3b0d0d), 0.06), ..base.clone() }),
        look("split", "Split", "上下双色", TitleGroup::Hollow, rgb(0xffffff), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xffffff), rgb(0xffd400)], angle: 90.0, lines: true, bands: true, repeat: 1.0 }), outline: line(rgb(0x15151a), 0.08), ..base.clone() }),
        look("scan", "Scanlines", "扫描线", TitleGroup::Hollow, rgb(0xb2ff59), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xccff90), rgb(0x4caf50)], angle: 90.0, lines: true, bands: true, repeat: 4.0 }), shadow: Some(Shadow { color: rgba(0x64dd17, 0.8), offset: [0.0, 0.0], blur: 0.16 }), ..base.clone() }),
        look("confetti", "Confetti", "彩色跳字", TitleGroup::Playful, rgb(0xffffff), TitleStyle { weight: 900, palette: Some(Palette { colors: vec![rgb(0xff5252), rgb(0xffb300), rgb(0x00c853), rgb(0x2979ff), rgb(0xd500f9)], unit: PaletteUnit::Char }), outline: line(rgb(0xffffff), 0.08), outer: line(rgb(0x22222a), 0.04), scatter: Some(Scatter { rotate: 8.0, shift: 0.08, scale: 0.1, seed: 7 }), ..base.clone() }),
        look("macaron", "Macaron", "马卡龙", TitleGroup::Playful, rgb(0xffffff), TitleStyle { weight: 900, palette: Some(Palette { colors: vec![rgb(0xffb3c1), rgb(0xffd6a5), rgb(0xfdffb6), rgb(0xcaffbf), rgb(0x9bf6ff), rgb(0xbdb2ff)], unit: PaletteUnit::Char }), outline: line(rgb(0x5a4a6a), 0.07), ..base.clone() }),
        look("duo", "Two-tone", "双色交替", TitleGroup::Playful, rgb(0xffffff), TitleStyle { weight: 900, palette: Some(Palette { colors: vec![rgb(0xffffff), rgb(0xffe14d)], unit: PaletteUnit::Char }), outline: line(rgb(0x15151a), 0.08), ..base.clone() }),
        look("dance", "Dance", "错落", TitleGroup::Playful, rgb(0xffffff), TitleStyle { weight: 900, outline: line(rgb(0x15151a), 0.08), scatter: Some(Scatter { rotate: 10.0, shift: 0.12, scale: 0.15, seed: 3 }), ..base.clone() }),
        look("arch", "Arch", "拱形", TitleGroup::Playful, rgb(0xffe14d), TitleStyle { weight: 900, outline: line(rgb(0x15151a), 0.08), arc: 90.0, ..base.clone() }),
        look("smile", "Smile", "微笑弧", TitleGroup::Playful, rgb(0xff8ac0), TitleStyle { weight: 900, outline: line(rgb(0xffffff), 0.08), outer: line(rgb(0xc2185b), 0.04), arc: -80.0, ..base.clone() }),
        look("boxed", "Boxed", "黑底白字", TitleGroup::Backing, rgb(0xffffff), TitleStyle { background: Some(rgba(0x000000, 0.6)), radius: 0.2, ..base.clone() }),
        look("label", "Label", "黄色标签", TitleGroup::Backing, rgb(0x1a1a1a), TitleStyle { weight: 900, background: Some(rgb(0xffd400)), radius: 0.3, ..base.clone() }),
        look("bars", "Line bars", "逐行底条", TitleGroup::Backing, rgb(0xffe14d), TitleStyle { weight: 900, background: Some(rgba(0x111114, 0.88)), line_boxes: true, ..base.clone() }),
        look("marker", "Highlighter", "荧光笔", TitleGroup::Backing, rgb(0xffffff), TitleStyle { weight: 900, background: Some(rgb(0xff3d7f)), backing: Backing::Marker, shadow: Some(Shadow { color: rgba(0x000000, 0.45), offset: [0.0, 0.03], blur: 0.05 }), ..base.clone() }),
        look("markeryellow", "Yellow marker", "黄色荧光笔", TitleGroup::Backing, rgb(0x16161a), TitleStyle { weight: 900, background: Some(rgb(0xffe600)), backing: Backing::Marker, outline: line(rgb(0xffffff), 0.07), ..base.clone() }),
        look("tiles", "Tiles", "字块", TitleGroup::Backing, rgb(0xffffff), TitleStyle { weight: 900, background: Some(rgb(0x111114)), backing: Backing::Tiles, radius: 0.12, ..base.clone() }),
        look("keycap", "Keycaps", "键帽", TitleGroup::Backing, rgb(0x16161a), TitleStyle { weight: 900, background: Some(rgb(0xffffff)), backing: Backing::Tiles, radius: 0.18, border: line(rgb(0x16161a), 0.05), ..base.clone() }),
        look("frame", "Frame", "线框", TitleGroup::Backing, rgb(0xffffff), TitleStyle { background: Some(rgba(0x000000, 0.35)), border: line(rgb(0xffffff), 0.04), ..base.clone() }),
        look("slant", "Slant banner", "斜切横幅", TitleGroup::Backing, rgb(0xffffff), TitleStyle { weight: 900, background: Some(rgb(0xff1744)), skew: 14.0, ..base.clone() }),
        look("ticket", "Ticket", "票券", TitleGroup::Backing, rgb(0xd84315), TitleStyle { weight: 900, background: Some(rgb(0xfff3e0)), radius: 0.25, border: line(rgb(0xd84315), 0.035), ..base.clone() }),
        look("sale", "Sale", "大促", TitleGroup::Promo, rgb(0xffeb3b), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xfffde7), rgb(0xffeb3b), rgb(0xff9800)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0xb71c1c), 0.08), outer: line(rgb(0xffffff), 0.05), depth: Some(Depth { color: rgb(0x7f0000), offset: [0.05, 0.08], fade: None }), ..base.clone() }),
        look("price", "Price", "爆款价", TitleGroup::Promo, rgb(0xff1f1f), TitleStyle { weight: 900, outline: line(rgb(0xffffff), 0.09), outer: line(rgb(0xff1f1f), 0.04), shadow: Some(Shadow { color: rgba(0x000000, 0.4), offset: [0.0, 0.05], blur: 0.06 }), ..base.clone() }),
        look("flash", "Flash sale", "限时秒杀", TitleGroup::Promo, rgb(0xd50000), TitleStyle { weight: 900, background: Some(rgb(0xffea00)), skew: 12.0, border: line(rgb(0xd50000), 0.04), ..base.clone() }),
        look("hot", "Hot", "热卖", TitleGroup::Promo, rgb(0xff5722), TitleStyle { weight: 900, fill: Some(TitleFill { colors: vec![rgb(0xfff59d), rgb(0xff5722), rgb(0xd50000)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), outline: line(rgb(0x4a0000), 0.07), shadow: Some(Shadow { color: rgba(0xff9100, 0.9), offset: [0.0, 0.0], blur: 0.1 }), shadows: vec![Shadow { color: rgba(0xff3d00, 0.6), offset: [0.0, 0.0], blur: 0.3 }], ..base.clone() }),
        look("fresh", "New", "新品", TitleGroup::Promo, rgb(0xffffff), TitleStyle { weight: 900, background: Some(rgb(0x00c853)), radius: 0.6, border: line(rgb(0xffffff), 0.035), ..base.clone() }),
        look("luxe", "Luxe", "轻奢黑金", TitleGroup::Promo, rgb(0xf5c542), TitleStyle { fill: Some(TitleFill { colors: vec![rgb(0xfff6c2), rgb(0xf5c542), rgb(0xb8860b), rgb(0xffe27a)], angle: 90.0, lines: true, bands: false, repeat: 1.0 }), background: Some(rgb(0x111114)), radius: 0.1, border: line(rgb(0xd4a93a), 0.025), ..base.clone() }),
    ]
}

/// The template called `id`.
pub fn title_template(id: &str) -> Option<TitleTemplate> {
    title_templates().into_iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_its_own_id_a_sound_style_and_sits_with_its_group() {
        let all = title_templates();
        for (i, t) in all.iter().enumerate() {
            assert!(all[..i].iter().all(|o| o.id != t.id), "{} twice", t.id);
            assert_eq!(t.style.problem(), None, "{}", t.id);
            // a group's templates stand together
            let first = all.iter().position(|o| o.group == t.group).unwrap();
            assert!(all[first..=i].iter().all(|o| o.group == t.group), "{} is apart from its group", t.id);
        }
        for g in TitleGroup::ALL {
            assert!(all.iter().any(|t| t.group == g), "{} is empty", g.id());
        }
    }
}
