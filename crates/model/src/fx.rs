//! Picture effects: presets that shake, pulse, flash or blur a picture for as long as they last.
//!
//! An effect sits on a clip ([`crate::Clip::fx`]) and plays over that clip, or on an adjustment
//! clip ([`crate::ClipContent::Adjustment`]) and plays on everything on the tracks below while
//! that clip lasts. Like in and out animations, an effect is a function of time
//! ([`Fx::sample`]): what it adds to the placement, and how it changes the exposure, the
//! saturation, the corners and the focus. The compiler lowers that to RenderSpec keys and to the
//! effects the renderer already has, so a renderer needs to know nothing about them.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Offset, Prop};

/// A picture effect preset.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FxKind {
    /// Jitters the picture, a little enlarged so its edges stay out of sight.
    Shake,
    /// Swells and settles on every beat, like a heartbeat.
    Pulse,
    /// Rocks left and right.
    Swing,
    /// Hops up and lands, again and again.
    Bounce,
    /// Pushes in slowly over the whole length.
    ZoomIn,
    /// Pulls back slowly over the whole length.
    ZoomOut,
    /// Flashes to white on every beat and falls back.
    Flash,
    /// Cuts to dark and back several times a second.
    Strobe,
    /// The brightness wavers like an old projector.
    Flicker,
    /// Starts out of focus and sharpens.
    BlurIn,
    /// Loses focus towards the end.
    BlurOut,
    /// Starts grey and takes on its colour.
    ColorIn,
    /// Dark corners that breathe.
    Vignette,
}

/// What kind of thing an effect does; the effects panel lists them by this.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FxGroup {
    /// Moves the picture.
    Motion,
    /// Changes its light.
    Light,
    /// Changes its focus or colour.
    Focus,
}

/// One thing an effect changes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FxChannel {
    /// Stops of exposure, added.
    Exposure,
    /// Saturation, added (-1 is grey).
    Saturation,
    /// Corner darkening, added (0 to 1).
    Vignette,
    /// Blur radius, as a share of the picture's shorter side.
    Blur,
}

impl FxKind {
    /// Every preset, in menu order.
    pub const ALL: [FxKind; 13] = [
        FxKind::Shake,
        FxKind::Pulse,
        FxKind::Swing,
        FxKind::Bounce,
        FxKind::ZoomIn,
        FxKind::ZoomOut,
        FxKind::Flash,
        FxKind::Strobe,
        FxKind::Flicker,
        FxKind::Vignette,
        FxKind::BlurIn,
        FxKind::BlurOut,
        FxKind::ColorIn,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            FxKind::Shake => "shake",
            FxKind::Pulse => "pulse",
            FxKind::Swing => "swing",
            FxKind::Bounce => "bounce",
            FxKind::ZoomIn => "zoomIn",
            FxKind::ZoomOut => "zoomOut",
            FxKind::Flash => "flash",
            FxKind::Strobe => "strobe",
            FxKind::Flicker => "flicker",
            FxKind::BlurIn => "blurIn",
            FxKind::BlurOut => "blurOut",
            FxKind::ColorIn => "colorIn",
            FxKind::Vignette => "vignette",
        }
    }

    /// The preset a command names.
    pub fn parse(name: &str) -> Option<FxKind> {
        FxKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// What kind of thing it does.
    pub fn group(self) -> FxGroup {
        match self {
            FxKind::Shake | FxKind::Pulse | FxKind::Swing | FxKind::Bounce | FxKind::ZoomIn | FxKind::ZoomOut => {
                FxGroup::Motion
            }
            FxKind::Flash | FxKind::Strobe | FxKind::Flicker | FxKind::Vignette => FxGroup::Light,
            FxKind::BlurIn | FxKind::BlurOut | FxKind::ColorIn => FxGroup::Focus,
        }
    }

    /// Whether it moves placement property `p` of the clip it sits on.
    pub fn moves(self, p: Prop) -> bool {
        match p {
            Prop::X => self == FxKind::Shake,
            Prop::Y => matches!(self, FxKind::Shake | FxKind::Bounce),
            Prop::Scale => matches!(self, FxKind::Shake | FxKind::Pulse | FxKind::ZoomIn | FxKind::ZoomOut),
            Prop::Rotation => matches!(self, FxKind::Shake | FxKind::Swing),
            _ => false,
        }
    }

    /// Whether it moves the picture at all.
    pub fn moves_any(self) -> bool {
        self.group() == FxGroup::Motion
    }

    /// Whether it changes `channel`.
    pub fn changes(self, channel: FxChannel) -> bool {
        match channel {
            FxChannel::Exposure => matches!(self, FxKind::Flash | FxKind::Strobe | FxKind::Flicker),
            FxChannel::Saturation => self == FxKind::ColorIn,
            FxChannel::Vignette => self == FxKind::Vignette,
            FxChannel::Blur => matches!(self, FxKind::BlurIn | FxKind::BlurOut),
        }
    }

    /// Whether `speed` changes it (the slow zooms take the whole length whatever it is).
    pub fn has_speed(self) -> bool {
        !matches!(self, FxKind::ZoomIn | FxKind::ZoomOut)
    }
}

/// An effect on a clip.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fx {
    /// Which preset.
    pub kind: FxKind,
    /// How strong, 0 to 1; 0.5 is the preset as designed.
    #[serde(default = "half")]
    pub strength: f32,
    /// How fast, 0.25 to 4 times the preset's own pace.
    #[serde(default = "one")]
    pub speed: f32,
}

fn half() -> f32 {
    0.5
}

fn one() -> f32 {
    1.0
}

/// What effects do to a picture at one instant.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FxSample {
    /// What is added to the placement.
    pub offset: Offset,
    /// Stops of exposure, added.
    pub exposure: f32,
    /// Saturation, added.
    pub saturation: f32,
    /// Corner darkening, added.
    pub vignette: f32,
    /// Blur radius, as a share of the picture's shorter side.
    pub blur: f32,
}

impl FxSample {
    /// No change.
    pub const REST: FxSample =
        FxSample { offset: Offset::REST, exposure: 0.0, saturation: 0.0, vignette: 0.0, blur: 0.0 };

    /// Both at once.
    pub fn and(self, o: FxSample) -> FxSample {
        FxSample {
            offset: self.offset.and(o.offset),
            exposure: self.exposure + o.exposure,
            saturation: self.saturation + o.saturation,
            vignette: self.vignette + o.vignette,
            blur: self.blur + o.blur,
        }
    }

    /// The value of `channel`.
    pub fn channel(&self, channel: FxChannel) -> f32 {
        match channel {
            FxChannel::Exposure => self.exposure.clamp(-5.0, 5.0),
            FxChannel::Saturation => self.saturation.clamp(-1.0, 1.0),
            FxChannel::Vignette => self.vignette.clamp(0.0, 1.0),
            FxChannel::Blur => self.blur.max(0.0),
        }
    }
}

/// A number from -1 to 1 for step `i` of stream `salt`: the same everywhere, every time.
fn hash(i: i64, salt: u64) -> f32 {
    let mut z = (i as u64).wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 40) as f32 / (1u64 << 23) as f32) - 1.0
}

/// Smooth noise from -1 to 1 along `x`: [`hash`] at whole numbers, eased between.
fn noise(x: f64, salt: u64) -> f32 {
    let i = x.floor();
    let f = (x - i) as f32;
    let (a, b) = (hash(i as i64, salt), hash(i as i64 + 1, salt));
    a + (b - a) * f * f * (3.0 - 2.0 * f)
}

impl Fx {
    /// The preset at its designed strength and pace.
    pub fn new(kind: FxKind) -> Fx {
        Fx { kind, strength: 0.5, speed: 1.0 }
    }

    /// Why this effect cannot be, if it cannot.
    pub fn problem(&self) -> Option<&'static str> {
        if !(0.0..=1.0).contains(&self.strength) {
            return Some("an effect's strength must be 0 to 1");
        }
        if !(0.25..=4.0).contains(&self.speed) {
            return Some("an effect's speed must be 0.25 to 4");
        }
        None
    }

    /// What it does `t` after it began, on something `len` long. Effects that repeat go on outside
    /// that span (a clip's layer is longer than the clip under a transition); the others hold
    /// their first and last values.
    pub fn sample(&self, t: Tick, len: Tick) -> FxSample {
        use std::f64::consts::{PI, TAU};
        let mut s = FxSample::REST;
        // twice the strength: 1 at the preset's own
        let a = self.strength * 2.0;
        let speed = f64::from(self.speed);
        // beats since it began, one a second at the preset's own pace
        let beats = t.0 as f64 * speed / Tick::SECOND.0 as f64;
        let part = beats - beats.floor();
        // how far through the whole length
        let u = if len > Tick::ZERO { (t.0 as f64 / len.0 as f64).clamp(0.0, 1.0) as f32 } else { 1.0 };
        // how far through a one-beat ramp from the start, and how far into one up to the end
        let ramp = Tick((Tick::SECOND.0 as f64 / speed) as i64).min(len).max(Tick(1));
        let begun = (t.0 as f64 / ramp.0 as f64).clamp(0.0, 1.0) as f32;
        let ending = ((t - (len - ramp)).0 as f64 / ramp.0 as f64).clamp(0.0, 1.0) as f32;
        match self.kind {
            FxKind::Shake => {
                let x = beats * 10.0;
                s.offset.dx = noise(x, 1) * 0.02 * a;
                s.offset.dy = noise(x, 2) * 0.02 * a;
                s.offset.rotation = noise(x, 3) * 1.5 * a;
                s.offset.scale = 1.0 + 0.05 * a;
            }
            FxKind::Pulse => {
                let p = part as f32;
                let e = if p < 0.15 { p / 0.15 } else { ((1.0 - p) / 0.85).powi(2) };
                s.offset.scale = 1.0 + 0.08 * a * e;
            }
            FxKind::Swing => s.offset.rotation = 5.0 * a * (TAU * beats * 0.5).sin() as f32,
            FxKind::Bounce => s.offset.dy = -0.04 * a * (PI * beats).sin().abs() as f32,
            FxKind::ZoomIn => s.offset.scale = 1.0 + 0.3 * a * u,
            FxKind::ZoomOut => s.offset.scale = 1.0 + 0.3 * a * (1.0 - u),
            FxKind::Flash => s.exposure = 2.5 * a * (1.0 - part as f32).powi(3),
            FxKind::Strobe => {
                if (beats * 12.0).floor() as i64 & 1 == 1 {
                    s.exposure = -(1.0 + 4.0 * self.strength);
                }
            }
            FxKind::Flicker => s.exposure = hash((beats * 24.0).floor() as i64, 4) * 0.6 * a,
            FxKind::BlurIn => s.blur = 0.05 * a * (1.0 - begun).powi(2),
            FxKind::BlurOut => s.blur = 0.05 * a * ending.powi(2),
            FxKind::ColorIn => s.saturation = -(a * (1.0 - begun)).min(1.0),
            FxKind::Vignette => s.vignette = (a * (0.6 + 0.4 * (TAU * beats * 0.5).sin() as f32)).min(1.0),
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_every_preset_does_something() {
        for k in FxKind::ALL {
            assert_eq!(FxKind::parse(k.name()), Some(k));
            let fx = Fx::new(k);
            assert_eq!(fx.problem(), None);
            let len = Tick::SECOND * 4;
            let changed = (0..120).any(|i| fx.sample(Tick(len.0 * i / 120), len) != FxSample::REST);
            assert!(changed, "{k:?}");
            // it says what it changes
            let moves = Prop::MOTION.iter().any(|p| k.moves(*p));
            let colours = [FxChannel::Exposure, FxChannel::Saturation, FxChannel::Vignette, FxChannel::Blur]
                .iter()
                .any(|c| k.changes(*c));
            assert!(moves != colours, "{k:?}");
            assert_eq!(moves, k.moves_any());
            for i in 0..120 {
                let s = fx.sample(Tick(len.0 * i / 120), len);
                let o = s.offset;
                if !k.moves(Prop::X) {
                    assert_eq!(o.dx, 0.0);
                }
                if !k.moves(Prop::Y) {
                    assert_eq!(o.dy, 0.0);
                }
                if !k.moves(Prop::Scale) {
                    assert_eq!(o.scale, 1.0);
                }
                if !k.moves(Prop::Rotation) {
                    assert_eq!(o.rotation, 0.0);
                }
                assert_eq!(o.opacity, 1.0);
                if !k.changes(FxChannel::Exposure) {
                    assert_eq!(s.exposure, 0.0);
                }
                if !k.changes(FxChannel::Blur) {
                    assert_eq!(s.blur, 0.0);
                }
            }
        }
        assert_eq!(FxKind::parse("nope"), None);
    }

    #[test]
    fn ramps_start_and_end_where_they_should() {
        let len = Tick::SECOND * 4;
        let at = |k: FxKind, t: Tick| Fx::new(k).sample(t, len);
        assert!((at(FxKind::BlurIn, Tick::ZERO).blur - 0.05).abs() < 1e-6);
        assert_eq!(at(FxKind::BlurIn, Tick::SECOND).blur, 0.0);
        assert_eq!(at(FxKind::BlurOut, Tick::SECOND * 3).blur, 0.0);
        assert!((at(FxKind::BlurOut, len).blur - 0.05).abs() < 1e-6);
        assert_eq!(at(FxKind::ColorIn, Tick::ZERO).saturation, -1.0);
        assert_eq!(at(FxKind::ColorIn, Tick::SECOND * 2).saturation, 0.0);
        assert_eq!(at(FxKind::ZoomIn, Tick::ZERO).offset.scale, 1.0);
        assert!((at(FxKind::ZoomIn, len).offset.scale - 1.3).abs() < 1e-6);
        assert!((at(FxKind::ZoomOut, Tick::ZERO).offset.scale - 1.3).abs() < 1e-6);
        // a flash is brightest on the beat
        assert!(at(FxKind::Flash, Tick::SECOND).exposure > at(FxKind::Flash, Tick(Tick::SECOND.0 * 3 / 2)).exposure);
        // twice as fast, twice as many beats
        let fast = Fx { speed: 2.0, ..Fx::new(FxKind::Flash) };
        assert_eq!(fast.sample(Tick(Tick::SECOND.0 / 2), len).exposure, at(FxKind::Flash, Tick::SECOND).exposure);
        // the shake never shows an edge: the picture is enlarged by more than it moves
        for i in 0..200 {
            let o = at(FxKind::Shake, Tick(len.0 * i / 200)).offset;
            assert!(o.dx.abs() <= 0.02 && o.dy.abs() <= 0.02 && (o.scale - 1.05).abs() < 1e-6);
        }
        assert!(Fx { strength: 1.5, ..Fx::new(FxKind::Shake) }.problem().is_some());
        assert!(Fx { speed: 0.1, ..Fx::new(FxKind::Shake) }.problem().is_some());
    }
}
