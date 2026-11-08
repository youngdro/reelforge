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
}

impl Prop {
    /// Every property, in inspector order.
    pub const ALL: [Prop; 6] = [Prop::X, Prop::Y, Prop::Scale, Prop::Rotation, Prop::Opacity, Prop::VolumeDb];

    /// The name commands use (`x`, `volumeDb`…).
    pub fn name(self) -> &'static str {
        match self {
            Prop::X => "x",
            Prop::Y => "y",
            Prop::Scale => "scale",
            Prop::Rotation => "rotation",
            Prop::Opacity => "opacity",
            Prop::VolumeDb => "volumeDb",
        }
    }

    /// The property a command names.
    pub fn parse(name: &str) -> Option<Prop> {
        Prop::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// How a value travels from one key to the next.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Ease {
    /// Straight line.
    #[default]
    Linear,
    /// Holds until the next key.
    Hold,
    /// Slow out of this key and into the next.
    InOut,
}

impl Ease {
    /// The eased fraction for a straight fraction `f` (0 to 1).
    pub fn apply(self, f: f32) -> f32 {
        match self {
            Ease::Linear => f,
            Ease::Hold => 0.0,
            Ease::InOut => f * f * (3.0 - 2.0 * f),
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
}

impl AnimKind {
    /// Every preset, in menu order.
    pub const ALL: [AnimKind; 9] = [
        AnimKind::Fade,
        AnimKind::SlideUp,
        AnimKind::SlideDown,
        AnimKind::SlideLeft,
        AnimKind::SlideRight,
        AnimKind::ZoomIn,
        AnimKind::ZoomOut,
        AnimKind::Pop,
        AnimKind::Spin,
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
        }
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
            Prop::VolumeDb => base,
        }
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

/// A clip's in and out animations.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Animations {
    /// Plays from the clip's start.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub enter: Option<Animation>,
    /// Plays up to the clip's end.
    #[serde(default, rename = "out", skip_serializing_if = "Option::is_none")]
    pub exit: Option<Animation>,
}

impl Animations {
    /// Neither.
    pub fn is_empty(&self) -> bool {
        self.enter.is_none() && self.exit.is_none()
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
        let mut o = Offset::REST;
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
        }
    }

    /// Whether `p` has keyframes.
    pub fn keyed(&self, p: Prop) -> bool {
        self.keys.get(&p).is_some_and(|k| !k.is_empty())
    }

    /// The keyframed (or constant) value of `p` at `t` from the clip's start, before animations.
    pub fn value_at(&self, p: Prop, t: Tick) -> f32 {
        self.keys.get(&p).and_then(|k| sample(k, t)).unwrap_or_else(|| self.base(p))
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
}
