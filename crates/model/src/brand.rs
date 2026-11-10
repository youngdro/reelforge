//! A project's brand: the colours, the face and the mark its ready-made pieces are made in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the ready-made pieces (`promo.add`) take their look from, unless they are told otherwise.
/// Setting it changes pieces added afterwards, not the ones already on the timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Brand {
    /// The main colour, straight sRGB: buttons, tags, badges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<[f32; 4]>,
    /// The colour beside it: highlights, the better of two prices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<[f32; 4]>,
    /// The face titles are set in: a font among the project's media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The mark: an image among the project's media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

impl Brand {
    /// Nothing set.
    pub fn is_empty(&self) -> bool {
        *self == Brand::default()
    }

    /// What is wrong with its colours, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        let colours = self.primary.iter().chain(self.secondary.iter());
        colours.flatten().any(|c| !(0.0..=1.0).contains(c)).then_some("colours are 0 to 1")
    }
}

/// A ready-made piece as it was made: which one, and everything it was told or took from the
/// brand at the time. Kept by the timeline of a compound clip that is one, so that the piece can
/// be made again with other words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Made {
    /// Which piece (`promo.list`).
    pub kind: String,
    /// Its words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Its further lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// The number it counts to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    /// Its further numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f32>,
    /// Where its counting starts.
    #[serde(default)]
    pub from: f32,
    /// Its main colour and the one beside it.
    pub color: [f32; 4],
    /// The colour beside it.
    pub color2: [f32; 4],
    /// The way it leans, points or moves, if it was told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Where it sits in the frame, as shares of its size from the centre, and its size.
    pub x: f32,
    /// Vertical offset.
    pub y: f32,
    /// Size; 1 is as designed.
    pub scale: f32,
    /// How long it was made to show.
    pub duration: reelforge_time::Tick,
    /// The font its words are set in and the logo it shows: media of the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}
