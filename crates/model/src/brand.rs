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
