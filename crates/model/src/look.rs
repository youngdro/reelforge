//! How a picture clip's picture is cut, mirrored, masked and laid over what is below.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A picture clip's crop, mirror, mask and blend.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Look {
    /// Edges cut off a video or picture (media clips).
    #[serde(skip_serializing_if = "Crop::is_zero")]
    pub crop: Crop,
    /// Mirrored left to right (media clips).
    #[serde(skip_serializing_if = "is_false")]
    pub flip_x: bool,
    /// Mirrored top to bottom (media clips).
    #[serde(skip_serializing_if = "is_false")]
    pub flip_y: bool,
    /// How it combines with what is below.
    #[serde(skip_serializing_if = "BlendMode::is_normal")]
    pub blend: BlendMode,
    /// A shape that shows part of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<Mask>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Look {
    /// Changes nothing.
    pub fn is_empty(&self) -> bool {
        *self == Look::default()
    }

    /// Why the look cannot be drawn, if it cannot.
    pub fn problem(&self) -> Option<&'static str> {
        let c = &self.crop;
        let edges = [c.left, c.top, c.right, c.bottom];
        if edges.iter().any(|e| !(0.0..1.0).contains(e)) || c.left + c.right > 0.99 || c.top + c.bottom > 0.99 {
            return Some("a crop must leave at least 1 % of the picture");
        }
        if let Some(m) = &self.mask {
            let v = [m.center[0], m.center[1], m.size[0], m.size[1], m.rotation, m.feather];
            if v.iter().any(|x| !x.is_finite()) || m.size.iter().any(|x| *x < 0.0) || m.feather < 0.0 {
                return Some("a mask needs finite values and a size and feather of 0 or more");
            }
        }
        None
    }
}

/// Fractions cut off each edge.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Crop {
    /// Of the width, from the left.
    pub left: f32,
    /// Of the height, from the top.
    pub top: f32,
    /// Of the width, from the right.
    pub right: f32,
    /// Of the height, from the bottom.
    pub bottom: f32,
}

impl Crop {
    /// Cuts nothing.
    pub fn is_zero(&self) -> bool {
        *self == Crop::default()
    }
}

/// Blend modes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    /// Over what is below.
    #[default]
    Normal,
    /// Added: brighter.
    Add,
    /// Multiplied: darker.
    Multiply,
    /// Screened: lighter.
    Screen,
    /// More contrast.
    Overlay,
    /// The darker of the two.
    Darken,
    /// The lighter of the two.
    Lighten,
    /// A gentle overlay.
    SoftLight,
    /// A strong overlay.
    HardLight,
    /// The difference.
    Difference,
    /// Brightens what is below.
    ColorDodge,
    /// Darkens what is below.
    ColorBurn,
}

impl BlendMode {
    /// Every mode, in menu order.
    pub const ALL: [BlendMode; 12] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Difference,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
    ];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Add => "add",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::SoftLight => "softLight",
            BlendMode::HardLight => "hardLight",
            BlendMode::Difference => "difference",
            BlendMode::ColorDodge => "colorDodge",
            BlendMode::ColorBurn => "colorBurn",
        }
    }

    /// The mode with this id.
    pub fn parse(s: &str) -> Option<BlendMode> {
        BlendMode::ALL.into_iter().find(|m| m.name() == s)
    }

    /// Normal.
    pub fn is_normal(&self) -> bool {
        *self == BlendMode::Normal
    }
}

/// Mask shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MaskKind {
    /// Everything on one side of a line.
    Linear,
    /// A band.
    Mirror,
    /// An ellipse.
    Circle,
    /// A rectangle.
    Rectangle,
}

impl MaskKind {
    /// Every shape, in menu order.
    pub const ALL: [MaskKind; 4] = [MaskKind::Linear, MaskKind::Mirror, MaskKind::Circle, MaskKind::Rectangle];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            MaskKind::Linear => "linear",
            MaskKind::Mirror => "mirror",
            MaskKind::Circle => "circle",
            MaskKind::Rectangle => "rectangle",
        }
    }

    /// The shape with this id.
    pub fn parse(s: &str) -> Option<MaskKind> {
        MaskKind::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// A shape that shows part of a clip's picture, placed on the picture (so it moves with it).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mask {
    /// Which shape.
    pub kind: MaskKind,
    /// Centre, as fractions of the picture's width and height.
    pub center: [f32; 2],
    /// Width and height, in units of the picture's shorter side.
    pub size: [f32; 2],
    /// Clockwise turn, degrees.
    pub rotation: f32,
    /// Soft edge, in units of the shorter side.
    pub feather: f32,
    /// Show the outside instead.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
}

impl Mask {
    /// A shape of this kind in the middle, half the shorter side across, with a soft edge.
    pub fn new(kind: MaskKind) -> Mask {
        let size = match kind {
            MaskKind::Mirror => [1.0, 0.3],
            _ => [0.5, 0.5],
        };
        Mask { kind, center: [0.5, 0.5], size, rotation: 0.0, feather: 0.05, invert: false }
    }
}
