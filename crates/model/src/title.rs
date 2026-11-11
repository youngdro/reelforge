//! What a title's letters can wear beyond one colour, an outline and a box: fills through several
//! colours, a colour per character, depth, lit edges, letters out of line or round a curve, and
//! other things to stand behind them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::is_no;

/// A fill through several colours, in place of the title's one colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TitleFill {
    /// The colours (straight sRGB), two to eight, evenly spaced along the way.
    pub colors: Vec<[f32; 4]>,
    /// Which way they run, in degrees clockwise: 0 from left to right, 90 from top to bottom.
    #[serde(default = "down")]
    pub angle: f32,
    /// They run across each line on its own, not once across the whole title.
    #[serde(default, skip_serializing_if = "is_no")]
    pub lines: bool,
    /// Bands with hard edges (stripes), not a smooth run from one colour to the next.
    #[serde(default, skip_serializing_if = "is_no")]
    pub bands: bool,
    /// How many times the colours go round along the way.
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub repeat: f32,
}

impl TitleFill {
    /// How many times the colours may go round.
    pub const REPEATS: (f32, f32) = (0.1, 32.0);

    /// A smooth run through `colors` from top to bottom.
    pub fn down(colors: &[[f32; 4]]) -> TitleFill {
        TitleFill { colors: colors.to_vec(), angle: 90.0, lines: true, bands: false, repeat: 1.0 }
    }

    /// A smooth run through `colors` from left to right across the whole title.
    pub fn across(colors: &[[f32; 4]]) -> TitleFill {
        TitleFill { colors: colors.to_vec(), angle: 0.0, lines: false, bands: false, repeat: 1.0 }
    }
}

fn down() -> f32 {
    90.0
}

fn once() -> f32 {
    1.0
}

fn is_once(v: &f32) -> bool {
    *v == 1.0
}

/// What takes one colour of a [`Palette`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PaletteUnit {
    /// Each character (white space left out).
    #[default]
    Char,
    /// Each word (a Chinese character is a word of its own).
    Word,
    /// Each line.
    Line,
}

impl PaletteUnit {
    /// Every unit, in menu order.
    pub const ALL: [PaletteUnit; 3] = [PaletteUnit::Char, PaletteUnit::Word, PaletteUnit::Line];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            PaletteUnit::Char => "char",
            PaletteUnit::Word => "word",
            PaletteUnit::Line => "line",
        }
    }

    /// The unit a command names.
    pub fn parse(name: &str) -> Option<PaletteUnit> {
        PaletteUnit::ALL.into_iter().find(|u| u.name() == name)
    }
}

/// Colours the title's characters, words or lines take in turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Palette {
    /// The colours (straight sRGB), one to eight: the first unit the first, the next the second,
    /// and round again.
    pub colors: Vec<[f32; 4]>,
    /// What takes one colour.
    #[serde(default)]
    pub unit: PaletteUnit,
}

/// The letters drawn out to one side as a solid block, as if they were thick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Depth {
    /// The block's colour (straight sRGB).
    pub color: [f32; 4],
    /// How far it reaches, `[right, down]` in ems.
    pub offset: [f32; 2],
    /// The colour at its far end, when it changes along the way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<[f32; 4]>,
}

/// Edges lit from one side and shaded on the other, as if the letters stood up from the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bevel {
    /// The lit edge's colour (straight sRGB).
    pub light: [f32; 4],
    /// The shaded edge's colour.
    pub dark: [f32; 4],
    /// How wide the edges are, in ems.
    pub size: f32,
    /// Where the light is, in degrees clockwise from straight above: 315 is the top left.
    #[serde(default = "top_left")]
    pub angle: f32,
}

fn top_left() -> f32 {
    315.0
}

/// Each character turned, moved and sized a little by chance: the same way every time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scatter {
    /// The most a character turns either way, in degrees.
    #[serde(default)]
    pub rotate: f32,
    /// The most it moves up or down, in ems.
    #[serde(default)]
    pub shift: f32,
    /// The most it grows or shrinks, as a share of its size.
    #[serde(default)]
    pub scale: f32,
    /// Which arrangement: another number, another one.
    #[serde(default)]
    pub seed: u32,
}

/// What a title's background colour is the colour of.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backing {
    /// A box behind the title, or behind each of its lines.
    #[default]
    Box,
    /// A band behind the lower half of each line, as a highlighter pen leaves.
    Marker,
    /// A tile behind each character; the characters stand [`Backing::TILE_ROOM`] further apart
    /// to make room for them.
    Tiles,
}

impl Backing {
    /// How much further apart characters with tiles stand, in ems.
    pub const TILE_ROOM: f32 = 0.3;

    /// Every kind, in menu order.
    pub const ALL: [Backing; 3] = [Backing::Box, Backing::Marker, Backing::Tiles];

    /// Whether it is the plain box.
    pub fn is_box(&self) -> bool {
        *self == Backing::Box
    }

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            Backing::Box => "box",
            Backing::Marker => "marker",
            Backing::Tiles => "tiles",
        }
    }

    /// The kind a command names.
    pub fn parse(name: &str) -> Option<Backing> {
        Backing::ALL.into_iter().find(|b| b.name() == name)
    }
}

fn colour(c: &[f32; 4]) -> bool {
    c.iter().all(|v| v.is_finite())
}

impl super::TitleStyle {
    /// The most extra shadows a title has.
    pub const SHADOWS: usize = 4;
    /// The furthest a title's lines bend, in degrees either way.
    pub const ARC: f32 = 360.0;
    /// The furthest a box's sides lean, in degrees either way.
    pub const SKEW: f32 = 60.0;

    /// What is wrong with what the letters wear, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if let Some(f) = &self.fill {
            if !(2..=8).contains(&f.colors.len()) || !f.colors.iter().all(colour) {
                return Some("a fill takes two to eight colours");
            }
            let (lo, hi) = TitleFill::REPEATS;
            if !f.angle.is_finite() || !(lo..=hi).contains(&f.repeat) {
                return Some("a fill's angle must be a number and its repeat 0.1 to 32");
            }
        }
        if let Some(p) = &self.palette
            && (!(1..=8).contains(&p.colors.len()) || !p.colors.iter().all(colour))
        {
            return Some("a palette takes one to eight colours");
        }
        if self.shadows.len() > Self::SHADOWS {
            return Some("a title has at most four more shadows");
        }
        for s in &self.shadows {
            if !colour(&s.color)
                || !s.offset.iter().all(|v| v.is_finite() && v.abs() <= 4.0)
                || !(0.0..=4.0).contains(&s.blur)
            {
                return Some("a shadow's offset and blur must be within 4 em");
            }
        }
        if let Some(d) = &self.depth
            && (!colour(&d.color)
                || d.fade.as_ref().is_some_and(|c| !colour(c))
                || !d.offset.iter().all(|v| v.is_finite() && v.abs() <= 1.0)
                || d.offset == [0.0, 0.0])
        {
            return Some("depth reaches up to 1 em to a side, and somewhere");
        }
        if self.hollow.is_some_and(|w| !(w > 0.0 && w <= 0.5)) {
            return Some("a hollow title's line must be above 0 and at most 0.5 em");
        }
        if let Some(b) = &self.bevel
            && (!colour(&b.light) || !colour(&b.dark) || !(b.size > 0.0 && b.size <= 0.5) || !b.angle.is_finite())
        {
            return Some("a bevel's edges must be above 0 and at most 0.5 em wide");
        }
        if let Some(s) = &self.scatter
            && !((0.0..=180.0).contains(&s.rotate) && (0.0..=2.0).contains(&s.shift) && (0.0..=0.9).contains(&s.scale))
        {
            return Some("scatter turns up to 180 degrees, moves up to 2 em and sizes up to 0.9");
        }
        if !(self.arc.is_finite() && self.arc.abs() <= Self::ARC) {
            return Some("an arc must be within 360 degrees either way");
        }
        if !(self.skew.is_finite() && self.skew.abs() <= Self::SKEW) {
            return Some("a box leans at most 60 degrees either way");
        }
        if self.border.as_ref().is_some_and(|o| !colour(&o.color) || !(0.0..=0.5).contains(&o.width)) {
            return Some("a box's border must be 0 to 0.5 em wide");
        }
        None
    }
}
