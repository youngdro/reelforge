//! Lowering keyframes and in/out animations to RenderSpec keys.
//!
//! Keyframes pass through as they are, cut at the clip's edges where a trim left some outside. An animation is an eased curve laid over the clip's own
//! value (added to the position and rotation, multiplying the scale and opacity), which RenderSpec
//! keys cannot express exactly, so while an animation plays the result is sampled at every output
//! frame and joined with straight lines. Wherever no animation plays, the clip's own keys and
//! easing pass through untouched. Two properties lowered together (x and y, which RenderSpec keys
//! as one position) share key times; where their easings differ, the stretch is sampled per frame.

use reelforge_model::{Animations, Clip, Ease as MEase, Key as MKey, Prop};
use reelforge_renderspec::{Animated, Ease, Key};
use reelforge_time::Tick;

/// Whether an animation preset changes `p`.
fn moves(kind: reelforge_model::AnimKind, p: Prop) -> bool {
    use reelforge_model::AnimKind::*;
    match p {
        Prop::Opacity => true,
        Prop::X => matches!(kind, SlideLeft | SlideRight),
        Prop::Y => matches!(kind, SlideUp | SlideDown),
        Prop::Scale => matches!(kind, ZoomIn | ZoomOut | Pop | Spin),
        Prop::Rotation => matches!(kind, Spin),
        Prop::VolumeDb => false,
    }
}

/// One property of one clip, as a function of time from the clip's start.
struct Curve<'a> {
    prop: Prop,
    clip: &'a Clip,
    keys: &'a [MKey],
    anim: Animations,
    animated: bool,
}

/// How an interval of a curve can be drawn exactly from its end values.
#[derive(Copy, Clone, PartialEq, Debug)]
enum Fits {
    /// Flat: any easing reproduces it.
    Any,
    /// Exactly this easing.
    Only(Ease),
    /// No single easing: sample it.
    Sample,
}

impl Curve<'_> {
    fn value(&self, t: Tick) -> f32 {
        let base = self.clip.value_at(self.prop, t);
        if self.animated { self.anim.offset(t, self.clip.duration).apply(self.prop, base) } else { base }
    }

    /// The stretches where an animation reshapes this property.
    fn windows(&self) -> Vec<(Tick, Tick)> {
        if !self.animated {
            return vec![];
        }
        let len = self.clip.duration;
        let (a, b) = self.anim.spans(len);
        let mut w = vec![];
        if let Some(x) = self.anim.enter
            && a > Tick::ZERO
            && moves(x.kind, self.prop)
        {
            w.push((Tick::ZERO, a));
        }
        if let Some(x) = self.anim.exit
            && b > Tick::ZERO
            && moves(x.kind, self.prop)
        {
            w.push((len - b, len));
        }
        w
    }

    fn fits(&self, t0: Tick, t1: Tick) -> Fits {
        if self.windows().iter().any(|&(a, b)| t0 < b && t1 > a) {
            return Fits::Sample;
        }
        let k = self.keys;
        if k.len() < 2 || t1 <= k[0].t || t0 >= k[k.len() - 1].t {
            return Fits::Any;
        }
        let Some(j) = k.windows(2).position(|w| w[0].t <= t0 && t1 <= w[1].t) else {
            return Fits::Sample;
        };
        let (a, b) = (k[j], k[j + 1]);
        match a.ease {
            _ if a.v == b.v => Fits::Any,
            MEase::Linear => Fits::Only(Ease::Linear),
            MEase::Hold => Fits::Only(Ease::Hold),
            MEase::InOut if t0 == a.t && t1 == b.t => Fits::Only(Ease::InOut),
            MEase::InOut => Fits::Sample,
        }
    }
}

/// Key times and easings that reproduce every curve: their own key times, every frame where an
/// animation plays or easings disagree, and the clip's ends.
fn plan(curves: &[Curve], step: Tick, len: Tick) -> Vec<(Tick, Ease)> {
    let mut times: Vec<Tick> = curves.iter().flat_map(|c| c.keys.iter().map(|k| k.t)).collect();
    for c in curves {
        for (a, b) in c.windows() {
            times.extend([a, b]);
        }
    }
    // RenderSpec keys lie within the layer: keys a trim left outside are cut at its edges
    times.retain(|t| *t >= Tick::ZERO && *t <= len);
    times.extend([Tick::ZERO, len]);
    times.sort();
    times.dedup();
    let mut out = vec![];
    for (i, &t0) in times.iter().enumerate() {
        let Some(&t1) = times.get(i + 1) else {
            out.push((t0, Ease::Linear));
            break;
        };
        let mut agreed = Some(Fits::Any);
        for c in curves {
            agreed = match (agreed, c.fits(t0, t1)) {
                (Some(Fits::Any), f) | (Some(f), Fits::Any) => Some(f),
                (Some(Fits::Only(a)), Fits::Only(b)) if a == b => Some(Fits::Only(a)),
                _ => None,
            };
        }
        match agreed {
            Some(Fits::Any) => out.push((t0, Ease::Linear)),
            Some(Fits::Only(e)) => out.push((t0, e)),
            _ => {
                // every frame boundary between t0 and t1, on the clip's own frame grid
                let mut t = t0;
                while t < t1 {
                    out.push((t, Ease::Linear));
                    t = Tick((t.0 / step.0 + 1) * step.0);
                }
            }
        }
    }
    out
}

fn curve<'a>(clip: &'a Clip, prop: Prop) -> Curve<'a> {
    let anim = clip.anim;
    let animated = [anim.enter, anim.exit].iter().flatten().any(|a| moves(a.kind, prop));
    Curve { prop, clip, keys: clip.keys.get(&prop).map_or(&[], Vec::as_slice), anim, animated }
}

/// Property `prop` of `clip`, with time measured from the clip's start; `step` is a frame.
pub fn lower(clip: &Clip, prop: Prop, step: Tick) -> Animated<f32> {
    let c = curve(clip, prop);
    if c.keys.is_empty() && !c.animated {
        return Animated::Constant(clip.base(prop));
    }
    let keys = plan(std::slice::from_ref(&c), step, clip.duration);
    Animated::Keys(keys.into_iter().map(|(t, e)| Key { t, v: c.value(t), ease: e }).collect())
}

/// Position `[x, y]` of `clip`, keyed together.
pub fn lower_position(clip: &Clip, step: Tick) -> Animated<[f32; 2]> {
    let (x, y) = (curve(clip, Prop::X), curve(clip, Prop::Y));
    if x.keys.is_empty() && y.keys.is_empty() && !x.animated && !y.animated {
        return Animated::Constant([clip.base(Prop::X), clip.base(Prop::Y)]);
    }
    let keys = plan(&[x, y], step, clip.duration);
    let (x, y) = (curve(clip, Prop::X), curve(clip, Prop::Y));
    Animated::Keys(keys.into_iter().map(|(t, e)| Key { t, v: [x.value(t), y.value(t)], ease: e }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_model::{AnimKind, Animation, ClipContent, ClipTransform};

    fn clip(len: i64) -> Clip {
        Clip {
            id: "c1".into(),
            start: Tick(1000),
            duration: Tick(len),
            content: ClipContent::Solid { color: [1.0; 4] },
            transform: ClipTransform { x: 0.1, ..ClipTransform::default() },
            opacity: 0.8,
            volume_db: 0.0,
            effects: vec![],
            keys: Default::default(),
            anim: Default::default(),
        }
    }

    fn key(t: i64, v: f32, ease: MEase) -> MKey {
        MKey { t: Tick(t), v, ease }
    }

    #[test]
    fn plain_values_stay_constant_and_keys_pass_through() {
        let mut c = clip(1000);
        assert_eq!(lower(&c, Prop::Opacity, Tick(10)), Animated::Constant(0.8));
        c.keys.insert(Prop::Scale, vec![key(0, 1.0, MEase::InOut), key(500, 2.0, MEase::Linear)]);
        let Animated::Keys(k) = lower(&c, Prop::Scale, Tick(10)) else { panic!() };
        assert_eq!(
            k.iter().map(|k| (k.t.0, k.v, k.ease)).collect::<Vec<_>>(),
            [(0, 1.0, Ease::InOut), (500, 2.0, Ease::Linear), (1000, 2.0, Ease::Linear)]
        );
        // a trim that cuts keys off keeps the values on screen
        c.shift_keys(Tick(-300));
        let lowered = lower(&c, Prop::Scale, Tick(10));
        let Animated::Keys(k) = &lowered else { panic!() };
        assert!(k.iter().all(|k| k.t >= Tick::ZERO && k.t <= c.duration));
        for t in [0, 55, 120, 200, 600] {
            assert!((lowered.sample(Tick(t)) - c.value_at(Prop::Scale, Tick(t))).abs() < 0.01, "{t}");
        }
        assert_eq!(lower_position(&c, Tick(10)), Animated::Constant([0.1, 0.0]));
    }

    #[test]
    fn an_animation_is_sampled_only_while_it_plays() {
        let mut c = clip(1000);
        c.anim.enter = Some(Animation { kind: AnimKind::Fade, duration: Tick(100) });
        let lowered = lower(&c, Prop::Opacity, Tick(10));
        let Animated::Keys(k) = &lowered else { panic!() };
        // 0, 10, …, 100 and the end
        assert_eq!(k.len(), 12);
        assert_eq!(k[0].v, 0.0);
        assert!((lowered.sample(Tick(50)) - 0.4).abs() < 1e-6);
        assert_eq!(lowered.sample(Tick(600)), 0.8);
        // a fade does not move the clip
        assert_eq!(lower_position(&c, Tick(10)), Animated::Constant([0.1, 0.0]));
    }

    #[test]
    fn animations_ride_on_keyframes() {
        let mut c = clip(1000);
        c.keys.insert(Prop::Y, vec![key(200, 0.0, MEase::InOut), key(800, 0.5, MEase::Linear)]);
        c.keys.insert(Prop::X, vec![key(200, 0.0, MEase::InOut), key(800, 0.4, MEase::Linear)]);
        c.anim.exit = Some(Animation { kind: AnimKind::SlideUp, duration: Tick(100) });
        let lowered = lower_position(&c, Tick(10));
        let Animated::Keys(k) = &lowered else { panic!() };
        // matches the model everywhere it is keyed
        let model = |t: i64| {
            let o = c.anim.offset(Tick(t), c.duration);
            [o.apply(Prop::X, c.value_at(Prop::X, Tick(t))), o.apply(Prop::Y, c.value_at(Prop::Y, Tick(t)))]
        };
        for kk in k {
            assert_eq!(kk.v, model(kk.t.0));
        }
        // the shared ease-in-out runs through untouched; only the exit is sampled
        assert!(k.iter().any(|kk| kk.t == Tick(200) && kk.ease == Ease::InOut));
        let at = lowered.sample(Tick(500));
        assert!((at[0] - model(500)[0]).abs() < 1e-6 && (at[1] - model(500)[1]).abs() < 1e-6);
        assert!(k.len() < 20, "only the exit is sampled: {}", k.len());
    }
}
