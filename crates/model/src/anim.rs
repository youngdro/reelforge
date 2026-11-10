//! Movement over time: keyframes on a clip's properties, and the in and out animations short
//! videos put on nearly every title and sticker.
//!
//! Keyframe times are relative to the clip's start, so moving a clip keeps its motion; trimming
//! and splitting shift them so they stay where they were on the timeline. The compiler lowers
//! both keyframes and animations to RenderSpec keys (`reelforge-compile`).

use std::collections::BTreeMap;

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Clip;

/// A clip property that can move.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Prop {
    /// Horizontal offset, share of the canvas width.
    X,
    /// Vertical offset, share of the canvas height.
    Y,
    /// Uniform scale.
    Scale,
    /// Degrees clockwise.
    Rotation,
    /// 0 to 1.
    Opacity,
    /// Gain in dB.
    VolumeDb,
    /// The crop: the share cut off the picture's left edge.
    CropLeft,
    /// The share cut off the top.
    CropTop,
    /// The share cut off the right.
    CropRight,
    /// The share cut off the bottom.
    CropBottom,
    /// The mask's centre across the picture, 0 to 1.
    MaskX,
    /// The mask's centre down the picture, 0 to 1.
    MaskY,
    /// The mask's width, in units of the picture's shorter side.
    MaskWidth,
    /// The mask's height.
    MaskHeight,
    /// The mask's turn, degrees clockwise.
    MaskRotation,
    /// The mask's soft edge.
    MaskFeather,
    /// The colour adjustments, as [`crate::Adjust`] names them.
    Exposure,
    /// See [`crate::Adjust`].
    Contrast,
    /// See [`crate::Adjust`].
    Saturation,
    /// See [`crate::Adjust`].
    Temperature,
    /// See [`crate::Adjust`].
    Tint,
    /// See [`crate::Adjust`].
    Highlights,
    /// See [`crate::Adjust`].
    Shadows,
    /// See [`crate::Adjust`].
    Fade,
    /// See [`crate::Adjust`].
    Vignette,
}

impl Prop {
    /// Where and how loud: what `clip.set` sets, in inspector order.
    pub const MOTION: [Prop; 6] = [Prop::X, Prop::Y, Prop::Scale, Prop::Rotation, Prop::Opacity, Prop::VolumeDb];
    /// The crop's edges: left, top, right, bottom.
    pub const CROP: [Prop; 4] = [Prop::CropLeft, Prop::CropTop, Prop::CropRight, Prop::CropBottom];
    /// The mask: centre, size, turn, soft edge.
    pub const MASK: [Prop; 6] =
        [Prop::MaskX, Prop::MaskY, Prop::MaskWidth, Prop::MaskHeight, Prop::MaskRotation, Prop::MaskFeather];
    /// The colour adjustments, in [`crate::Adjust::RANGES`] order.
    pub const ADJUST: [Prop; 9] = [
        Prop::Exposure,
        Prop::Contrast,
        Prop::Saturation,
        Prop::Temperature,
        Prop::Tint,
        Prop::Highlights,
        Prop::Shadows,
        Prop::Fade,
        Prop::Vignette,
    ];
    /// Every property.
    pub const ALL: [Prop; 25] = [
        Prop::X,
        Prop::Y,
        Prop::Scale,
        Prop::Rotation,
        Prop::Opacity,
        Prop::VolumeDb,
        Prop::CropLeft,
        Prop::CropTop,
        Prop::CropRight,
        Prop::CropBottom,
        Prop::MaskX,
        Prop::MaskY,
        Prop::MaskWidth,
        Prop::MaskHeight,
        Prop::MaskRotation,
        Prop::MaskFeather,
        Prop::Exposure,
        Prop::Contrast,
        Prop::Saturation,
        Prop::Temperature,
        Prop::Tint,
        Prop::Highlights,
        Prop::Shadows,
        Prop::Fade,
        Prop::Vignette,
    ];

    /// The name commands use (`x`, `volumeDb`, `cropLeft`, `maskX`, `exposure`…).
    pub fn name(self) -> &'static str {
        match self {
            Prop::X => "x",
            Prop::Y => "y",
            Prop::Scale => "scale",
            Prop::Rotation => "rotation",
            Prop::Opacity => "opacity",
            Prop::VolumeDb => "volumeDb",
            Prop::CropLeft => "cropLeft",
            Prop::CropTop => "cropTop",
            Prop::CropRight => "cropRight",
            Prop::CropBottom => "cropBottom",
            Prop::MaskX => "maskX",
            Prop::MaskY => "maskY",
            Prop::MaskWidth => "maskWidth",
            Prop::MaskHeight => "maskHeight",
            Prop::MaskRotation => "maskRotation",
            Prop::MaskFeather => "maskFeather",
            Prop::Exposure => "exposure",
            Prop::Contrast => "contrast",
            Prop::Saturation => "saturation",
            Prop::Temperature => "temperature",
            Prop::Tint => "tint",
            Prop::Highlights => "highlights",
            Prop::Shadows => "shadows",
            Prop::Fade => "fade",
            Prop::Vignette => "vignette",
        }
    }

    /// The property a command names.
    pub fn parse(name: &str) -> Option<Prop> {
        Prop::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The values a key of this property may take, where they are bounded.
    pub fn range(self) -> Option<(f32, f32)> {
        if let Some(i) = Prop::ADJUST.iter().position(|p| *p == self) {
            let (_, lo, hi) = crate::Adjust::RANGES[i];
            return Some((lo, hi));
        }
        match self {
            Prop::Opacity => Some((0.0, 1.0)),
            p if Prop::CROP.contains(&p) => Some((0.0, 0.98)),
            Prop::MaskWidth | Prop::MaskHeight | Prop::MaskFeather => Some((0.0, f32::MAX)),
            _ => None,
        }
    }

    /// Whether in and out animations and transitions never touch it: it is lowered key for key.
    pub fn plain(self) -> bool {
        !Prop::MOTION.contains(&self)
    }
}

/// How a value travels from one key to the next.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Ease {
    /// Straight line.
    #[default]
    Linear,
    /// Holds until the next key.
    Hold,
    /// Slow out of this key and into the next.
    InOut,
    /// A curve drawn by hand: a cubic Bézier from (0, 0) to (1, 1) with the control points
    /// `[x1, y1, x2, y2]`, as CSS writes `cubic-bezier`. Across is time (each `x` within 0 to 1),
    /// up is how far the value has got, which may pass the next key and come back.
    Bezier([f32; 4]),
}

impl Ease {
    /// Slow away from this key, arriving at speed.
    pub const IN: Ease = Ease::Bezier([0.42, 0.0, 1.0, 1.0]);
    /// Leaving at speed, slow into the next key.
    pub const OUT: Ease = Ease::Bezier([0.0, 0.0, 0.58, 1.0]);
    /// Past the next key and back to it.
    pub const OVERSHOOT: Ease = Ease::Bezier([0.34, 1.56, 0.64, 1.0]);
    /// Back a little first, then on to the next key.
    pub const ANTICIPATE: Ease = Ease::Bezier([0.36, 0.0, 0.66, -0.56]);

    /// The curves with a name, by the name commands use.
    pub const NAMED: [(&'static str, Ease); 7] = [
        ("linear", Ease::Linear),
        ("hold", Ease::Hold),
        ("inOut", Ease::InOut),
        ("in", Ease::IN),
        ("out", Ease::OUT),
        ("overshoot", Ease::OVERSHOOT),
        ("anticipate", Ease::ANTICIPATE),
    ];

    /// How far up a hand-drawn curve may go, below the start and above the end.
    pub const REACH: (f32, f32) = (-1.0, 2.0);

    /// The curve called `name`.
    pub fn named(name: &str) -> Option<Ease> {
        Ease::NAMED.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
    }

    /// The name of this curve, when it has one.
    pub fn name(self) -> Option<&'static str> {
        Ease::NAMED.iter().find(|(_, e)| *e == self).map(|(n, _)| *n)
    }

    /// A hand-drawn curve through these control points, each pulled to where a curve can go.
    pub fn bezier(points: [f32; 4]) -> Ease {
        let [x1, y1, x2, y2] = points;
        let (lo, hi) = Ease::REACH;
        Ease::Bezier([x1.clamp(0.0, 1.0), y1.clamp(lo, hi), x2.clamp(0.0, 1.0), y2.clamp(lo, hi)])
    }

    /// This curve's control points `[x1, y1, x2, y2]`, as a curve editor draws them; a hold has
    /// none.
    pub fn points(self) -> Option<[f32; 4]> {
        match self {
            Ease::Linear => Some([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]),
            Ease::Hold => None,
            // the smooth step is itself a cubic
            Ease::InOut => Some([1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0]),
            Ease::Bezier(p) => Some(p),
        }
    }

    /// Whether the curve is one a project can hold: finite, and going forward in time.
    pub fn is_valid(self) -> bool {
        match self {
            Ease::Bezier([x1, y1, x2, y2]) => {
                let (lo, hi) = Ease::REACH;
                [x1, x2].iter().all(|x| (0.0..=1.0).contains(x)) && [y1, y2].iter().all(|y| (lo..=hi).contains(y))
            }
            _ => true,
        }
    }

    /// The eased fraction for a straight fraction `f` (0 to 1).
    pub fn apply(self, f: f32) -> f32 {
        match self {
            Ease::Linear => f,
            Ease::Hold => 0.0,
            Ease::InOut => f * f * (3.0 - 2.0 * f),
            Ease::Bezier([x1, y1, x2, y2]) => {
                let f = f.clamp(0.0, 1.0);
                // one coordinate of the curve at parameter `u`
                let at = |a: f32, b: f32, u: f32| {
                    let v = 1.0 - u;
                    3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
                };
                // `x` only goes forward (its control points lie within 0 to 1): halve to where
                // it is `f`
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..24 {
                    let mid = (lo + hi) / 2.0;
                    if at(x1, x2, mid) < f {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                at(y1, y2, (lo + hi) / 2.0)
            }
        }
    }
}

/// One keyframe.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Key {
    /// Time from the clip's start.
    pub t: Tick,
    /// The value there.
    pub v: f32,
    /// How the value moves on to the next key.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: Ease,
}

fn is_linear(e: &Ease) -> bool {
    *e == Ease::Linear
}

/// Keyframes by property; each list sorted by time, no two keys at one time.
pub type Keys = BTreeMap<Prop, Vec<Key>>;

/// The value of sorted `keys` at `t`; before the first and after the last the value holds.
pub fn sample(keys: &[Key], t: Tick) -> Option<f32> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.v);
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t < b.t {
            let f = ((t - a.t).0 as f64 / (b.t - a.t).0 as f64) as f32;
            return Some(a.v + (b.v - a.v) * a.ease.apply(f));
        }
    }
    keys.last().map(|k| k.v)
}

/// Puts `key` in `keys`, replacing a key at the same time.
pub fn insert(keys: &mut Vec<Key>, key: Key) {
    match keys.binary_search_by_key(&key.t, |k| k.t) {
        Ok(i) => keys[i] = key,
        Err(i) => keys.insert(i, key),
    }
}

/// A preset that brings a clip in or takes it out.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AnimKind {
    /// Opacity from nothing.
    Fade,
    /// Rises into place while fading in.
    SlideUp,
    /// Drops into place while fading in.
    SlideDown,
    /// Moves in leftwards from the right while fading in.
    SlideLeft,
    /// Moves in rightwards from the left while fading in.
    SlideRight,
    /// Grows from smaller while fading in.
    ZoomIn,
    /// Shrinks from larger while fading in.
    ZoomOut,
    /// Grows from nothing, overshoots a little and settles.
    Pop,
    /// Turns a quarter turn into place while growing.
    Spin,
    /// Types a title in a character at a time (or erases it as it leaves); titles only.
    Typewriter,
    /// A title's characters fade in one after another; titles only.
    CharFade,
    /// A title's characters rise into place one after another; titles only.
    CharRise,
    /// A title's characters drop into place one after another; titles only.
    CharDrop,
    /// A title's characters pop up one after another, each overshooting a little; titles only.
    CharPop,
}

impl AnimKind {
    /// Every preset, in menu order.
    pub const ALL: [AnimKind; 14] = [
        AnimKind::Fade,
        AnimKind::SlideUp,
        AnimKind::SlideDown,
        AnimKind::SlideLeft,
        AnimKind::SlideRight,
        AnimKind::ZoomIn,
        AnimKind::ZoomOut,
        AnimKind::Pop,
        AnimKind::Spin,
        AnimKind::Typewriter,
        AnimKind::CharFade,
        AnimKind::CharRise,
        AnimKind::CharDrop,
        AnimKind::CharPop,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            AnimKind::Fade => "fade",
            AnimKind::SlideUp => "slideUp",
            AnimKind::SlideDown => "slideDown",
            AnimKind::SlideLeft => "slideLeft",
            AnimKind::SlideRight => "slideRight",
            AnimKind::ZoomIn => "zoomIn",
            AnimKind::ZoomOut => "zoomOut",
            AnimKind::Pop => "pop",
            AnimKind::Spin => "spin",
            AnimKind::Typewriter => "typewriter",
            AnimKind::CharFade => "charFade",
            AnimKind::CharRise => "charRise",
            AnimKind::CharDrop => "charDrop",
            AnimKind::CharPop => "charPop",
        }
    }

    /// Whether only titles take it.
    pub fn titles_only(self) -> bool {
        self == AnimKind::Typewriter || self.by_character()
    }

    /// Whether it moves a title's characters one after another instead of the title as a whole.
    pub fn by_character(self) -> bool {
        matches!(self, AnimKind::CharFade | AnimKind::CharRise | AnimKind::CharDrop | AnimKind::CharPop)
    }

    /// The preset a command names.
    pub fn parse(name: &str) -> Option<AnimKind> {
        AnimKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// How far from rest the clip is when the animation is `u` of the way in (0 at the far end, 1
    /// at rest): what it adds to x and y and to the rotation, and what it multiplies the scale and
    /// opacity by.
    pub fn offset(self, u: f32) -> Offset {
        let u = u.clamp(0.0, 1.0);
        // ease out: fast at first, settling into place
        let e = 1.0 - (1.0 - u).powi(3);
        let away = 1.0 - e;
        let slide = 0.12;
        let mut o = Offset { opacity: u.min(1.0), ..Offset::REST };
        match self {
            AnimKind::Fade => {}
            AnimKind::SlideUp => o.dy = slide * away,
            AnimKind::SlideDown => o.dy = -slide * away,
            AnimKind::SlideLeft => o.dx = slide * away,
            AnimKind::SlideRight => o.dx = -slide * away,
            AnimKind::ZoomIn => o.scale = 0.6 + 0.4 * e,
            AnimKind::ZoomOut => o.scale = 1.4 - 0.4 * e,
            AnimKind::Pop => {
                // back-out: overshoots by about 10 % and settles
                let c = 1.70158f32;
                let v = u - 1.0;
                o.scale = 1.0 + (c + 1.0) * v * v * v + c * v * v;
                o.opacity = (u * 3.0).min(1.0);
            }
            AnimKind::Spin => {
                o.rotation = -90.0 * away;
                o.scale = 0.5 + 0.5 * e;
            }
            // the text itself changes (`reveal`, or its characters move), not its placement
            AnimKind::Typewriter | AnimKind::CharFade | AnimKind::CharRise | AnimKind::CharDrop | AnimKind::CharPop => {
                o = Offset::REST
            }
        }
        o
    }
}

/// What an animation does to a clip at one instant; see [`AnimKind::offset`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Offset {
    /// Added to x.
    pub dx: f32,
    /// Added to y.
    pub dy: f32,
    /// Multiplies the scale.
    pub scale: f32,
    /// Added to the rotation, degrees.
    pub rotation: f32,
    /// Multiplies the opacity.
    pub opacity: f32,
}

impl Offset {
    /// No change.
    pub const REST: Offset = Offset { dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, opacity: 1.0 };

    /// Both offsets at once.
    pub fn and(self, o: Offset) -> Offset {
        Offset {
            dx: self.dx + o.dx,
            dy: self.dy + o.dy,
            scale: self.scale * o.scale,
            rotation: self.rotation + o.rotation,
            opacity: self.opacity * o.opacity,
        }
    }

    /// `base` with this offset applied, for property `p`.
    pub fn apply(self, p: Prop, base: f32) -> f32 {
        match p {
            Prop::X => base + self.dx,
            Prop::Y => base + self.dy,
            Prop::Scale => base * self.scale,
            Prop::Rotation => base + self.rotation,
            Prop::Opacity => base * self.opacity,
            _ => base,
        }
    }
}

/// A movement that repeats for as long as the clip is on screen.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoopKind {
    /// Grows and shrinks a little, slowly.
    Breathe,
    /// Two quick beats and a rest.
    Heartbeat,
    /// Turns a little to one side and the other.
    Swing,
    /// Drifts up and down.
    Float,
    /// Jumps up and lands.
    Bounce,
    /// Trembles.
    Shake,
    /// Turns round and round.
    Spin,
    /// Fades nearly out and back.
    Blink,
    /// A title's characters rise and fall in a wave that runs along it; titles only.
    Wave,
    /// A title's characters hop one after another; titles only.
    Jump,
}

impl LoopKind {
    /// Every one, in menu order.
    pub const ALL: [LoopKind; 10] = [
        LoopKind::Breathe,
        LoopKind::Heartbeat,
        LoopKind::Swing,
        LoopKind::Float,
        LoopKind::Bounce,
        LoopKind::Shake,
        LoopKind::Spin,
        LoopKind::Blink,
        LoopKind::Wave,
        LoopKind::Jump,
    ];

    /// The name commands use.
    pub fn name(self) -> &'static str {
        match self {
            LoopKind::Breathe => "breathe",
            LoopKind::Heartbeat => "heartbeat",
            LoopKind::Swing => "swing",
            LoopKind::Float => "float",
            LoopKind::Bounce => "bounce",
            LoopKind::Shake => "shake",
            LoopKind::Spin => "spin",
            LoopKind::Blink => "blink",
            LoopKind::Wave => "wave",
            LoopKind::Jump => "jump",
        }
    }

    /// The one a command names.
    pub fn parse(name: &str) -> Option<LoopKind> {
        LoopKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Whether only titles take it: it moves their characters.
    pub fn titles_only(self) -> bool {
        matches!(self, LoopKind::Wave | LoopKind::Jump)
    }

    /// How long one round takes when none is asked for.
    pub fn period(self) -> Tick {
        let ms = match self {
            LoopKind::Breathe | LoopKind::Float | LoopKind::Spin => 2000,
            LoopKind::Shake => 200,
            LoopKind::Bounce => 800,
            _ => 1000,
        };
        Tick(reelforge_time::TICKS_PER_SECOND / 1000 * ms)
    }

    /// Where the clip is `phase` rounds in (only the fraction counts; every round starts and ends
    /// at rest, a spin a whole turn on).
    pub fn offset(self, phase: f64) -> Offset {
        let u = phase.rem_euclid(1.0) as f32;
        let turn = std::f32::consts::TAU * u;
        let mut o = Offset::REST;
        match self {
            LoopKind::Breathe => o.scale = 1.0 + 0.06 * turn.sin(),
            LoopKind::Heartbeat => {
                let beat = |x: f32| (std::f32::consts::PI * x).sin().powi(2);
                let b = if u < 0.2 {
                    beat(u / 0.2)
                } else if u < 0.45 {
                    0.6 * beat((u - 0.2) / 0.25)
                } else {
                    0.0
                };
                o.scale = 1.0 + 0.14 * b;
            }
            LoopKind::Swing => o.rotation = 8.0 * turn.sin(),
            LoopKind::Float => o.dy = -0.02 * turn.sin(),
            LoopKind::Bounce => o.dy = -0.06 * (std::f32::consts::PI * u).sin(),
            LoopKind::Shake => {
                o.dx = 0.008 * turn.sin();
                o.dy = 0.005 * (2.0 * turn).sin();
            }
            LoopKind::Spin => o.rotation = 360.0 * u,
            LoopKind::Blink => o.opacity = 0.575 + 0.425 * turn.cos(),
            // the characters move, not the title
            LoopKind::Wave | LoopKind::Jump => {}
        }
        o
    }
}

/// A repeating movement on a clip.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Loop {
    /// Which movement.
    pub kind: LoopKind,
    /// How long one round takes; positive.
    pub period: Tick,
}

impl Loop {
    /// How many rounds in the clip is at `t` from its start.
    pub fn phase(&self, t: Tick) -> f64 {
        if self.period <= Tick::ZERO { 0.0 } else { t.0 as f64 / self.period.0 as f64 }
    }
}

/// An in or out animation on a clip.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Animation {
    /// Which preset.
    pub kind: AnimKind,
    /// How long it takes; positive.
    pub duration: Tick,
}

/// A clip's in and out animations, and the one that repeats in between.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Animations {
    /// Plays from the clip's start.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub enter: Option<Animation>,
    /// Plays up to the clip's end.
    #[serde(default, rename = "out", skip_serializing_if = "Option::is_none")]
    pub exit: Option<Animation>,
    /// Repeats from the clip's start to its end, under the other two.
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Loop>,
}

impl Animations {
    /// None of them.
    pub fn is_empty(&self) -> bool {
        self.enter.is_none() && self.exit.is_none() && self.repeat.is_none()
    }

    /// How long the in and out animations play on a clip `len` long: each as asked, shortened in
    /// proportion when together they would not fit.
    pub fn spans(&self, len: Tick) -> (Tick, Tick) {
        let a = self.enter.map_or(Tick::ZERO, |x| x.duration);
        let b = self.exit.map_or(Tick::ZERO, |x| x.duration);
        if a + b <= len || a + b <= Tick::ZERO {
            return (a, b);
        }
        let a2 = Tick((a.0 as i128 * len.0 as i128 / (a + b).0 as i128) as i64);
        (a2, len - a2)
    }

    /// The combined offset at `t` from the start of a clip `len` long.
    pub fn offset(&self, t: Tick, len: Tick) -> Offset {
        let (a, b) = self.spans(len);
        let mut o = self.repeat.map_or(Offset::REST, |l| l.kind.offset(l.phase(t)));
        if let Some(x) = self.enter
            && a > Tick::ZERO
            && t < a
        {
            o = o.and(x.kind.offset(((t.0 as f64) / a.0 as f64) as f32));
        }
        if let Some(x) = self.exit
            && b > Tick::ZERO
            && t > len - b
        {
            o = o.and(x.kind.offset((((len - t).0 as f64) / b.0 as f64) as f32));
        }
        o
    }
}

impl Clip {
    /// The constant value of `p`, used where it has no keys.
    pub fn base(&self, p: Prop) -> f32 {
        match p {
            Prop::X => self.transform.x,
            Prop::Y => self.transform.y,
            Prop::Scale => self.transform.scale,
            Prop::Rotation => self.transform.rotation,
            Prop::Opacity => self.opacity,
            Prop::VolumeDb => self.volume_db,
            Prop::CropLeft => self.look.crop.left,
            Prop::CropTop => self.look.crop.top,
            Prop::CropRight => self.look.crop.right,
            Prop::CropBottom => self.look.crop.bottom,
            Prop::MaskX => self.look.mask.map_or(0.5, |m| m.center[0]),
            Prop::MaskY => self.look.mask.map_or(0.5, |m| m.center[1]),
            Prop::MaskWidth => self.look.mask.map_or(0.5, |m| m.size[0]),
            Prop::MaskHeight => self.look.mask.map_or(0.5, |m| m.size[1]),
            Prop::MaskRotation => self.look.mask.map_or(0.0, |m| m.rotation),
            Prop::MaskFeather => self.look.mask.map_or(0.0, |m| m.feather),
            p => {
                let i = Prop::ADJUST.iter().position(|a| *a == p).expect("every other property is an adjustment");
                self.color.adjust.values()[i]
            }
        }
    }

    /// Sets the constant value of `p`.
    pub fn set_base(&mut self, p: Prop, v: f32) {
        match p {
            Prop::X => self.transform.x = v,
            Prop::Y => self.transform.y = v,
            Prop::Scale => self.transform.scale = v,
            Prop::Rotation => self.transform.rotation = v,
            Prop::Opacity => self.opacity = v,
            Prop::VolumeDb => self.volume_db = v,
            Prop::CropLeft => self.look.crop.left = v,
            Prop::CropTop => self.look.crop.top = v,
            Prop::CropRight => self.look.crop.right = v,
            Prop::CropBottom => self.look.crop.bottom = v,
            // a clip without a mask has none of its values to set
            Prop::MaskX | Prop::MaskY | Prop::MaskWidth | Prop::MaskHeight | Prop::MaskRotation | Prop::MaskFeather => {
                if let Some(m) = &mut self.look.mask {
                    match p {
                        Prop::MaskX => m.center[0] = v,
                        Prop::MaskY => m.center[1] = v,
                        Prop::MaskWidth => m.size[0] = v,
                        Prop::MaskHeight => m.size[1] = v,
                        Prop::MaskRotation => m.rotation = v,
                        _ => m.feather = v,
                    }
                }
            }
            p => {
                let i = Prop::ADJUST.iter().position(|a| *a == p).expect("every other property is an adjustment");
                let mut values = self.color.adjust.values();
                values[i] = v;
                self.color.adjust = crate::Adjust::from_values(values);
            }
        }
    }

    /// The crop at `t` from the clip's start: left, top, right, bottom, each side's pair kept
    /// to at most 99 % of the picture however the keys run.
    pub fn crop_at(&self, t: Tick) -> [f32; 4] {
        let [l, tp, r, b] = Prop::CROP.map(|p| self.value_at(p, t).clamp(0.0, 0.98));
        let fit = |a: f32, b: f32| if a + b > 0.99 { (a * 0.99 / (a + b), b * 0.99 / (a + b)) } else { (a, b) };
        let ((l, r), (tp, b)) = (fit(l, r), fit(tp, b));
        [l, tp, r, b]
    }

    /// The mask at `t` from the clip's start, if the clip has one.
    pub fn mask_at(&self, t: Tick) -> Option<crate::Mask> {
        let m = self.look.mask?;
        let [x, y, w, h, rotation, feather] = Prop::MASK.map(|p| self.value_at(p, t));
        Some(crate::Mask { center: [x, y], size: [w.max(0.0), h.max(0.0)], rotation, feather: feather.max(0.0), ..m })
    }

    /// The colour adjustments at `t` from the clip's start.
    pub fn adjust_at(&self, t: Tick) -> crate::Adjust {
        crate::Adjust::from_values(Prop::ADJUST.map(|p| self.value_at(p, t)))
    }

    /// Whether `p` has keyframes.
    pub fn keyed(&self, p: Prop) -> bool {
        self.keys.get(&p).is_some_and(|k| !k.is_empty())
    }

    /// The keyframed (or constant) value of `p` at `t` from the clip's start, before animations.
    pub fn value_at(&self, p: Prop, t: Tick) -> f32 {
        let Some(v) = self.keys.get(&p).and_then(|k| sample(k, t)) else { return self.base(p) };
        // a curve that passes its key stays within what the property can be
        match p.range() {
            Some((lo, hi)) => v.clamp(lo, hi),
            None if p == Prop::Scale => v.max(0.0),
            None => v,
        }
    }

    /// Moves every key `by` (negative moves them earlier), as when the clip's start moves the other
    /// way and its keys must stay put on the timeline.
    pub fn shift_keys(&mut self, by: Tick) {
        for list in self.keys.values_mut() {
            for k in list.iter_mut() {
                k.t += by;
            }
        }
    }

    /// Stretches every key time by `f`, as when the clip plays at another speed.
    pub fn scale_keys(&mut self, f: f64) {
        for list in self.keys.values_mut() {
            for k in list.iter_mut() {
                k.t = k.t.scale(f);
            }
            list.dedup_by_key(|k| k.t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_interpolate_hold_and_ease() {
        let keys = vec![
            Key { t: Tick(0), v: 0.0, ease: Ease::Linear },
            Key { t: Tick(100), v: 1.0, ease: Ease::Hold },
            Key { t: Tick(200), v: 3.0, ease: Ease::InOut },
            Key { t: Tick(300), v: 5.0, ease: Ease::Linear },
        ];
        assert_eq!(sample(&keys, Tick(-5)), Some(0.0));
        assert_eq!(sample(&keys, Tick(50)), Some(0.5));
        assert_eq!(sample(&keys, Tick(150)), Some(1.0));
        assert_eq!(sample(&keys, Tick(250)), Some(4.0));
        assert_eq!(sample(&keys, Tick(400)), Some(5.0));
        assert_eq!(sample(&[], Tick(0)), None);
    }

    #[test]
    fn a_hand_drawn_curve_starts_and_ends_on_its_keys_and_may_pass_them() {
        // the named curves are what they say
        assert_eq!(Ease::named("inOut"), Some(Ease::InOut));
        assert_eq!((Ease::named("in"), Ease::IN.name()), (Some(Ease::IN), Some("in")));
        assert_eq!((Ease::named("spring"), Ease::Bezier([0.1, 0.2, 0.3, 0.4]).name()), (None, None));
        for (_, e) in Ease::NAMED {
            assert!(e.is_valid());
            if e != Ease::Hold {
                assert!(e.apply(0.0).abs() < 1e-4 && (e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
            }
        }
        // slow away, fast in; and the other way round
        assert!(Ease::IN.apply(0.25) < 0.15 && Ease::OUT.apply(0.25) > 0.35);
        assert!((Ease::IN.apply(0.3) + Ease::OUT.apply(0.7) - 1.0).abs() < 1e-3, "mirror images");
        // the smooth step and the straight line as curves an editor can draw
        for e in [Ease::Linear, Ease::InOut] {
            let drawn = Ease::Bezier(e.points().unwrap());
            for f in [0.1, 0.35, 0.5, 0.8] {
                assert!((drawn.apply(f) - e.apply(f)).abs() < 1e-3, "{e:?} at {f}");
            }
        }
        assert_eq!(Ease::Hold.points(), None);
        // past the key and back; before it and on
        let top = (0..=20).map(|i| Ease::OVERSHOOT.apply(i as f32 / 20.0)).fold(0.0, f32::max);
        let low = (0..=20).map(|i| Ease::ANTICIPATE.apply(i as f32 / 20.0)).fold(0.0, f32::min);
        assert!(top > 1.05 && low < -0.05, "{top} {low}");
        // out of reach is pulled in, and what a file says is checked
        assert_eq!(Ease::bezier([-1.0, 9.0, 2.0, -9.0]), Ease::Bezier([0.0, 2.0, 1.0, -1.0]));
        assert!(!Ease::Bezier([1.2, 0.0, 0.5, 1.0]).is_valid() && !Ease::Bezier([0.2, f32::NAN, 0.5, 1.0]).is_valid());
        assert_eq!(serde_json::to_string(&Ease::IN).unwrap(), r#"{"bezier":[0.42,0.0,1.0,1.0]}"#);

        // on a clip, a curve that passes its key stays within what the property can be
        let keys =
            vec![Key { t: Tick(0), v: 0.5, ease: Ease::OVERSHOOT }, Key { t: Tick(100), v: 1.0, ease: Ease::Linear }];
        assert!(sample(&keys, Tick(70)).unwrap() > 1.0);
    }

    #[test]
    fn insert_replaces_at_the_same_time() {
        let mut keys = vec![];
        insert(&mut keys, Key { t: Tick(10), v: 1.0, ease: Ease::Linear });
        insert(&mut keys, Key { t: Tick(0), v: 2.0, ease: Ease::Linear });
        insert(&mut keys, Key { t: Tick(10), v: 3.0, ease: Ease::Hold });
        assert_eq!(keys.iter().map(|k| (k.t.0, k.v)).collect::<Vec<_>>(), [(0, 2.0), (10, 3.0)]);
    }

    #[test]
    fn animations_rest_in_the_middle_and_shrink_to_fit() {
        let a = Animations {
            enter: Some(Animation { kind: AnimKind::SlideUp, duration: Tick(40) }),
            exit: Some(Animation { kind: AnimKind::Fade, duration: Tick(60) }),
            repeat: None,
        };
        assert_eq!(a.spans(Tick(200)), (Tick(40), Tick(60)));
        assert_eq!(a.spans(Tick(50)), (Tick(20), Tick(30)));
        assert_eq!(a.offset(Tick(100), Tick(200)), Offset::REST);
        let start = a.offset(Tick(0), Tick(200));
        assert_eq!(start.opacity, 0.0);
        assert!(start.dy > 0.1);
        assert_eq!(a.offset(Tick(200), Tick(200)).opacity, 0.0);
        for k in AnimKind::ALL {
            let o = k.offset(1.0);
            assert!((o.scale - 1.0).abs() < 1e-5 && o.dx == 0.0 && o.dy == 0.0 && o.rotation == 0.0, "{k:?}");
            assert_eq!(AnimKind::parse(k.name()), Some(k));
        }
    }

    #[test]
    fn a_loop_comes_back_to_rest_every_round_under_the_in_and_out() {
        for k in LoopKind::ALL {
            assert_eq!(LoopKind::parse(k.name()), Some(k));
            assert!(k.period() > Tick::ZERO);
            let (start, round) = (k.offset(0.0), k.offset(3.0));
            assert_eq!(start, round, "{k:?}");
            assert!(start.dx == 0.0 && start.dy == 0.0 && (start.scale - 1.0).abs() < 1e-6, "{k:?}");
            assert!((start.opacity - 1.0).abs() < 1e-6 && start.rotation == 0.0, "{k:?}");
            // somewhere in the round it is elsewhere, unless it is the characters that move
            let moved = (1..20).any(|i| k.offset(i as f64 / 20.0) != Offset::REST);
            assert_eq!(moved, !k.titles_only(), "{k:?}");
        }
        let a = Animations {
            enter: Some(Animation { kind: AnimKind::Fade, duration: Tick(40) }),
            exit: None,
            repeat: Some(Loop { kind: LoopKind::Swing, period: Tick(100) }),
        };
        assert!(!a.is_empty());
        // a quarter of a round in: as far over as it goes, and half faded in
        let o = a.offset(Tick(25), Tick(1000));
        assert!((o.rotation - 8.0).abs() < 1e-4 && (o.opacity - 0.625).abs() < 1e-6, "{o:?}");
        assert!((a.offset(Tick(575), Tick(1000)).rotation + 8.0).abs() < 1e-3);
        assert!(a.offset(Tick(600), Tick(1000)).rotation.abs() < 1e-3);
    }
}
