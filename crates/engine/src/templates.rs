//! Title templates ("花字"): ready-made looks a title takes in one step, by the UI's picker or an
//! agent's `"template"` parameter. A template sets the title's colour and style; the words, size
//! and placement stay.

use reelforge_model::{Outline, Shadow, TitleStyle};

/// A ready-made title look.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleTemplate {
    /// Stable id, as commands take it.
    pub id: &'static str,
    /// English name.
    pub name: &'static str,
    /// Chinese name.
    pub name_zh: &'static str,
    /// Text colour, straight sRGB.
    pub color: [f32; 4],
    /// Everything else.
    pub style: TitleStyle,
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn rgb(hex: u32) -> [f32; 4] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0, 1.0]
}

fn line(color: [f32; 4], width: f32) -> Option<Outline> {
    Some(Outline { color, width })
}

fn shadow(color: [f32; 4], offset: [f32; 2], blur: f32) -> Option<Shadow> {
    Some(Shadow { color, offset, blur })
}

/// The templates, in the order a picker shows them.
pub fn title_templates() -> Vec<TitleTemplate> {
    let base = TitleStyle::default();
    vec![
        TitleTemplate {
            id: "classic",
            name: "Classic",
            name_zh: "经典字幕",
            color: WHITE,
            style: TitleStyle { outline: line(BLACK, 0.08), ..base.clone() },
        },
        TitleTemplate {
            id: "variety",
            name: "Variety",
            name_zh: "综艺黄字",
            color: rgb(0xffe14d),
            style: TitleStyle {
                outline: line(BLACK, 0.1),
                shadow: shadow(BLACK, [0.04, 0.06], 0.0),
                weight: 900,
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "sticker",
            name: "Sticker",
            name_zh: "贴纸",
            color: WHITE,
            style: TitleStyle {
                outline: line(rgb(0x1d1d2b), 0.07),
                outer: line(WHITE, 0.09),
                shadow: shadow([0.0, 0.0, 0.0, 0.45], [0.0, 0.05], 0.06),
                weight: 900,
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "candy",
            name: "Candy",
            name_zh: "糖果渐变",
            color: rgb(0xfff27a),
            style: TitleStyle {
                gradient: Some(rgb(0xff5fa2)),
                outline: line(WHITE, 0.07),
                outer: line(rgb(0xc2185b), 0.06),
                weight: 900,
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "fire",
            name: "Fire",
            name_zh: "火焰",
            color: rgb(0xfff3a0),
            style: TitleStyle {
                gradient: Some(rgb(0xff3d00)),
                outline: line(rgb(0x5a0a00), 0.08),
                shadow: shadow([1.0, 0.35, 0.0, 0.6], [0.0, 0.0], 0.18),
                weight: 900,
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "neon",
            name: "Neon",
            name_zh: "霓虹",
            color: rgb(0xe9fffd),
            style: TitleStyle {
                outline: line(rgb(0x2bd4c4), 0.04),
                shadow: shadow([0.17, 0.83, 0.77, 0.9], [0.0, 0.0], 0.22),
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "retro",
            name: "Retro",
            name_zh: "复古",
            color: rgb(0xff9f1c),
            style: TitleStyle {
                outline: line(rgb(0x1b1f3b), 0.06),
                shadow: shadow(rgb(0x2ec4b6), [0.07, 0.07], 0.0),
                weight: 900,
                ..base.clone()
            },
        },
        TitleTemplate {
            id: "soft",
            name: "Soft shadow",
            name_zh: "柔和投影",
            color: WHITE,
            style: TitleStyle { shadow: shadow([0.0, 0.0, 0.0, 0.7], [0.0, 0.04], 0.1), ..base.clone() },
        },
        TitleTemplate {
            id: "boxed",
            name: "Boxed",
            name_zh: "黑底白字",
            color: WHITE,
            style: TitleStyle { background: Some([0.0, 0.0, 0.0, 0.6]), radius: 0.2, ..base.clone() },
        },
        TitleTemplate {
            id: "label",
            name: "Label",
            name_zh: "黄色标签",
            color: rgb(0x1a1a1a),
            style: TitleStyle { background: Some(rgb(0xffd400)), radius: 0.3, weight: 900, ..base },
        },
    ]
}

/// The template called `id`.
pub fn title_template(id: &str) -> Option<TitleTemplate> {
    title_templates().into_iter().find(|t| t.id == id)
}
