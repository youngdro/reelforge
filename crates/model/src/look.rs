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
    /// A blurred copy of the clip filling the frame behind it (剪映's 画布模糊), this blurry: 0
    /// a little to 1 very (media clips).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<f32>,
    /// A colour made see-through, as for a green screen (media clips).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chroma: Option<ChromaKey>,
    /// Only the people in it kept, the rest see-through, as a model finds them in each picture
    /// (剪映's 智能抠像; media clips).
    #[serde(skip_serializing_if = "is_false")]
    pub person: bool,
    /// What stands before a plain backdrop kept and the backdrop made see-through (a product
    /// photographed on a studio sweep; media and compound clips).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutout: Option<Cutout>,
    /// A shadow under the picture, of its outline as it shows (media and compound clips).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<CastShadow>,
}

/// How a plain backdrop is told from what stands before it.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Cutout {
    /// How far from the backdrop's colour still counts as backdrop, 0 to 1: more takes its
    /// soft shadows and creases too, less keeps more of what is pale.
    pub tolerance: f32,
    /// How readily backdrop-coloured places that the picture's edges do not reach go (the
    /// inside of a handle), 0 (never) to 1.
    pub holes: f32,
}

impl Default for Cutout {
    fn default() -> Self {
        Cutout { tolerance: 0.5, holes: 0.5 }
    }
}

/// A shadow under a picture.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CastShadow {
    /// Its colour: red, green and blue as on screen, 0 to 1.
    pub color: [f32; 3],
    /// How dark it is, 0 to 1.
    pub opacity: f32,
    /// How far right of the picture it lies, in shares of the picture's shorter side.
    pub x: f32,
    /// How far below.
    pub y: f32,
    /// How soft it is: the blur's deviation, in shares of the picture's shorter side.
    pub blur: f32,
    /// How much of its height is pressed away towards the outline's lowest line, 0 to 1: a
    /// shadow lying on the ground the thing stands on.
    pub flat: f32,
    /// How far its top is pushed sideways, in shares of the outline's height, -1 to 1.
    pub lean: f32,
}

impl Default for CastShadow {
    /// A soft shadow a little below and to the right: the thing floats before what is behind.
    fn default() -> Self {
        CastShadow { color: [0.0; 3], opacity: 0.45, x: 0.015, y: 0.03, blur: 0.03, flat: 0.0, lean: 0.0 }
    }
}

impl CastShadow {
    /// The kinds a shadow comes ready-made in.
    pub const KINDS: [&'static str; 4] = ["drop", "ground", "long", "glow"];

    /// A ready-made shadow: `drop` floats the thing before what is behind, `ground` stands it
    /// on a floor (a low, soft pool under its foot), `long` is the shadow of a low sun falling
    /// to the right, `glow` is a light all round it.
    pub fn kind(name: &str) -> Option<CastShadow> {
        Some(match name {
            "drop" => CastShadow::default(),
            "ground" => CastShadow { opacity: 0.5, x: 0.0, y: 0.012, blur: 0.022, flat: 0.86, ..CastShadow::default() },
            "long" => CastShadow {
                opacity: 0.35,
                x: 0.0,
                y: 0.0,
                blur: 0.012,
                flat: 0.55,
                lean: 0.45,
                ..CastShadow::default()
            },
            "glow" => CastShadow { color: [1.0; 3], opacity: 0.8, x: 0.0, y: 0.0, blur: 0.04, ..CastShadow::default() },
            _ => return None,
        })
    }
}

/// A colour of a picture made see-through (剪映's 色度抠图).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChromaKey {
    /// The colour that goes: red, green and blue as on screen, 0 to 1.
    pub color: [f32; 3],
    /// How far from it a colour still goes, 0 to 1.
    #[serde(default = "ChromaKey::tolerance")]
    pub tolerance: f32,
    /// How gradually what is further comes back, 0 (a hard edge) to 1.
    #[serde(default = "ChromaKey::softness")]
    pub softness: f32,
    /// How much of the colour's tint is taken off what stays near it, 0 to 1.
    #[serde(default = "ChromaKey::spill")]
    pub spill: f32,
}

impl ChromaKey {
    /// The green of a green screen as cameras see it.
    pub const GREEN: [f32; 3] = [0.1, 0.75, 0.2];

    /// `color` keyed out as most screens need. A dull or dark colour is nearer to grey than a
    /// bright one, so it is given less room: what has no colour at all (hair, a white shirt)
    /// always stays whole.
    pub fn new(color: [f32; 3]) -> ChromaKey {
        // how far the colour is from grey, as `key.chroma@1` measures it (BT.601 Cb and Cr)
        let [r, g, b] = color;
        let cb = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
        let cr = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        let far = cb.hypot(cr);
        let round = |v: f32| (v * 1000.0).round() / 1000.0;
        ChromaKey {
            color,
            tolerance: round((far * 0.6).clamp(0.02, Self::tolerance())),
            softness: round((far * 0.3).min(Self::softness())),
            spill: Self::spill(),
        }
    }

    fn tolerance() -> f32 {
        0.2
    }

    fn softness() -> f32 {
        0.1
    }

    fn spill() -> f32 {
        0.5
    }
}

/// How far a backdrop of blur `amount` (0 to 1) blurs, as a fraction of the picture's shorter side.
pub fn backdrop_radius(amount: f32) -> f32 {
    0.01 + amount.clamp(0.0, 1.0) * 0.07
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
        if self.backdrop.is_some_and(|b| !(0.0..=1.0).contains(&b)) {
            return Some("a backdrop's blur is from 0 to 1");
        }
        if let Some(k) = &self.chroma {
            let v = [k.color[0], k.color[1], k.color[2], k.tolerance, k.softness, k.spill];
            if v.iter().any(|x| !(0.0..=1.0).contains(x)) {
                return Some("a chroma key's colour, tolerance, softness and spill are from 0 to 1");
            }
        }
        if self.cutout.is_some_and(|c| !(0.0..=1.0).contains(&c.tolerance) || !(0.0..=1.0).contains(&c.holes)) {
            return Some("a cut-out's tolerance and holes are from 0 to 1");
        }
        if let Some(s) = &self.shadow {
            let shares = [s.color[0], s.color[1], s.color[2], s.opacity, s.flat];
            if shares.iter().any(|x| !(0.0..=1.0).contains(x))
                || [s.x, s.y, s.lean].iter().any(|x| !(-1.0..=1.0).contains(x))
                || !(0.0..=0.5).contains(&s.blur)
            {
                return Some(
                    "a shadow's colour, opacity and flat are from 0 to 1, its x, y and lean from -1 to 1 and its blur from 0 to 0.5",
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_never_reaches_grey() {
        // a bright screen gets the usual room
        let bright = ChromaKey::new([0.0, 1.0, 0.0]);
        assert_eq!((bright.tolerance, bright.softness), (0.2, 0.1));
        // a dim one, a blue one, a nearly grey one: all end before what has no colour
        for color in [ChromaKey::GREEN, [0.13, 0.7, 0.23], [0.1, 0.2, 0.8], [0.2, 0.35, 0.25], [0.5, 0.5, 0.5]] {
            let key = ChromaKey::new(color);
            let [r, g, b] = color;
            let cb = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
            let cr = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
            let grey = cb.hypot(cr);
            assert!(key.tolerance >= 0.02 && key.softness >= 0.0, "{key:?}");
            assert!(grey < 0.03 || key.tolerance + key.softness < grey, "{key:?} reaches grey at {grey}");
        }
    }
}
