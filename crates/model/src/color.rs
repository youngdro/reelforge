//! A picture clip's colour: a filter, a lookup table and the user's own adjustments, applied in
//! that order. The compiler lowers each to RenderSpec effects (`color.grade@1`, `color.lut@1`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Colour adjustments; each is 0 when it changes nothing.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Adjust {
    /// Stops, -3 to 3.
    #[serde(skip_serializing_if = "is_zero")]
    pub exposure: f32,
    /// -1 to 1, around mid grey.
    #[serde(skip_serializing_if = "is_zero")]
    pub contrast: f32,
    /// -1 (grey) to 1.
    #[serde(skip_serializing_if = "is_zero")]
    pub saturation: f32,
    /// -1 (cool) to 1 (warm).
    #[serde(skip_serializing_if = "is_zero")]
    pub temperature: f32,
    /// -1 (green) to 1 (magenta).
    #[serde(skip_serializing_if = "is_zero")]
    pub tint: f32,
    /// -1 (darker) to 1 (brighter) in the bright parts.
    #[serde(skip_serializing_if = "is_zero")]
    pub highlights: f32,
    /// -1 (darker) to 1 (brighter) in the dark parts.
    #[serde(skip_serializing_if = "is_zero")]
    pub shadows: f32,
    /// 0 to 1: lifted blacks, a washed-out look.
    #[serde(skip_serializing_if = "is_zero")]
    pub fade: f32,
    /// 0 to 1: darker corners.
    #[serde(skip_serializing_if = "is_zero")]
    pub vignette: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl Adjust {
    /// Every adjustment's name (as commands and RenderSpec spell it) with its range.
    pub const RANGES: [(&'static str, f32, f32); 9] = [
        ("exposure", -3.0, 3.0),
        ("contrast", -1.0, 1.0),
        ("saturation", -1.0, 1.0),
        ("temperature", -1.0, 1.0),
        ("tint", -1.0, 1.0),
        ("highlights", -1.0, 1.0),
        ("shadows", -1.0, 1.0),
        ("fade", 0.0, 1.0),
        ("vignette", 0.0, 1.0),
    ];

    /// The values, in [`Adjust::RANGES`] order.
    pub fn values(&self) -> [f32; 9] {
        [
            self.exposure,
            self.contrast,
            self.saturation,
            self.temperature,
            self.tint,
            self.highlights,
            self.shadows,
            self.fade,
            self.vignette,
        ]
    }

    /// The adjustment with these values, in [`Adjust::RANGES`] order.
    pub fn from_values(v: [f32; 9]) -> Adjust {
        let [exposure, contrast, saturation, temperature, tint, highlights, shadows, fade, vignette] = v;
        Adjust { exposure, contrast, saturation, temperature, tint, highlights, shadows, fade, vignette }
    }

    /// Changes nothing.
    pub fn is_zero(&self) -> bool {
        self.values().iter().all(|v| *v == 0.0)
    }

    /// Every value times `k`.
    pub fn scaled(&self, k: f32) -> Adjust {
        Adjust::from_values(self.values().map(|v| v * k))
    }

    /// Whether every value is finite and in range.
    pub fn in_range(&self) -> bool {
        self.values().iter().zip(Adjust::RANGES).all(|(v, (_, lo, hi))| (lo..=hi).contains(v))
    }
}

/// Ready-made looks.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FilterKind {
    /// Bright, airy, slightly cool.
    Fresh,
    /// Golden hour.
    Warm,
    /// Blue and crisp.
    Cool,
    /// Black and white.
    Mono,
    /// Faded, warm, darker corners.
    Vintage,
    /// Soft highlights and lifted blacks.
    Film,
    /// Rich colour and punch.
    Vivid,
    /// Pink and orange evening light.
    Dusk,
}

impl FilterKind {
    /// Every filter, in menu order.
    pub const ALL: [FilterKind; 8] = [
        FilterKind::Fresh,
        FilterKind::Warm,
        FilterKind::Cool,
        FilterKind::Mono,
        FilterKind::Vintage,
        FilterKind::Film,
        FilterKind::Vivid,
        FilterKind::Dusk,
    ];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            FilterKind::Fresh => "fresh",
            FilterKind::Warm => "warm",
            FilterKind::Cool => "cool",
            FilterKind::Mono => "mono",
            FilterKind::Vintage => "vintage",
            FilterKind::Film => "film",
            FilterKind::Vivid => "vivid",
            FilterKind::Dusk => "dusk",
        }
    }

    /// The filter with this id.
    pub fn parse(s: &str) -> Option<FilterKind> {
        FilterKind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The filter's adjustments at full strength.
    pub fn adjust(self) -> Adjust {
        let z = Adjust::default();
        match self {
            FilterKind::Fresh => Adjust {
                exposure: 0.15,
                contrast: -0.1,
                saturation: 0.1,
                temperature: -0.15,
                highlights: -0.2,
                shadows: 0.2,
                ..z
            },
            FilterKind::Warm => {
                Adjust { temperature: 0.45, tint: 0.05, saturation: 0.1, highlights: -0.1, fade: 0.1, ..z }
            }
            FilterKind::Cool => Adjust { temperature: -0.45, tint: -0.05, contrast: 0.1, saturation: -0.05, ..z },
            FilterKind::Mono => Adjust { saturation: -1.0, contrast: 0.25, ..z },
            FilterKind::Vintage => {
                Adjust { temperature: 0.3, saturation: -0.35, fade: 0.6, contrast: -0.15, vignette: 0.5, ..z }
            }
            FilterKind::Film => Adjust {
                contrast: 0.2,
                saturation: -0.15,
                fade: 0.35,
                highlights: -0.25,
                temperature: 0.1,
                vignette: 0.25,
                ..z
            },
            FilterKind::Vivid => Adjust { saturation: 0.5, contrast: 0.2, shadows: 0.1, ..z },
            FilterKind::Dusk => {
                Adjust { temperature: 0.35, tint: 0.25, exposure: -0.2, contrast: 0.15, vignette: 0.35, ..z }
            }
        }
    }
}

/// A filter at a strength.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Filter {
    /// Which.
    pub kind: FilterKind,
    /// 0 to 1.
    pub strength: f32,
}

/// A lookup table at a strength.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LutUse {
    /// A `lut` media id.
    pub media: String,
    /// 0 to 1.
    pub strength: f32,
}

/// A picture clip's colour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ColorGrade {
    /// A ready-made look, applied first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    /// A lookup table, applied next.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lut: Option<LutUse>,
    /// The user's adjustments, applied last.
    #[serde(skip_serializing_if = "Adjust::is_zero")]
    pub adjust: Adjust,
}

impl ColorGrade {
    /// Changes nothing.
    pub fn is_empty(&self) -> bool {
        self.filter.is_none() && self.lut.is_none() && self.adjust.is_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_in_range_and_values_round_trip() {
        for k in FilterKind::ALL {
            assert!(k.adjust().in_range(), "{k:?}");
            assert_eq!(FilterKind::parse(k.name()), Some(k));
            let a = k.adjust();
            assert_eq!(Adjust::from_values(a.values()), a);
        }
        assert!(!Adjust { exposure: 4.0, ..Default::default() }.in_range());
        assert!(!Adjust { fade: f32::NAN, ..Default::default() }.in_range());
        let g = ColorGrade::default();
        assert!(g.is_empty());
        assert_eq!(serde_json::to_string(&g).unwrap(), "{}");
    }
}
