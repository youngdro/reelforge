//! Lowering keyframes, in/out animations and transitions to RenderSpec keys.
//!
//! Keyframes pass through as they are, cut at the layer's edges where a trim left some outside.
//! Animations and transitions are eased curves laid over the clip's own value (added to the
//! position and rotation, multiplying the scale and opacity), which RenderSpec keys cannot express
//! exactly, so while one plays the result is sampled at every output frame and joined with
//! straight lines. Wherever none plays, the clip's own keys and easing pass through untouched. Two
//! properties lowered together (x and y, which RenderSpec keys as one position) share key times;
//! where their easings differ, the stretch is sampled per frame.
//!
//! A clip's layer can be longer than the clip: a transition centred on a cut starts the incoming
//! layer early (its `lead`) and runs the outgoing one on. All times here are from the layer's
//! start; the clip's keys are shifted by the lead, and animations play over the whole layer.

use reelforge_model::{Clip, Ease as MEase, Fades, Key as MKey, Offset, Prop, TransitionKind};
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

/// Whether a transition changes `p` on the incoming or outgoing clip.
fn shifts(kind: TransitionKind, incoming: bool, p: Prop) -> bool {
    use TransitionKind::*;
    match p {
        Prop::Opacity => incoming && matches!(kind, Dissolve | FadeBlack | FadeWhite | Zoom),
        Prop::X => matches!((kind, incoming), (PushLeft | PushRight, _) | (SlideLeft | SlideRight, true)),
        Prop::Y => matches!((kind, incoming), (PushUp | PushDown, _) | (SlideUp | SlideDown, true)),
        Prop::Scale => kind == Zoom,
        Prop::Rotation | Prop::VolumeDb => false,
    }
}

/// A transition over part of a layer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Overlay {
    /// Where it starts, from the layer's start.
    pub from: Tick,
    /// Where it ends.
    pub to: Tick,
    /// Which transition.
    pub kind: TransitionKind,
    /// Whether this layer is the clip coming in (or the one going out).
    pub incoming: bool,
}

impl Overlay {
    fn offset(&self, t: Tick) -> Offset {
        if t <= self.from {
            return self.kind.offset(0.0, self.incoming);
        }
        if t >= self.to {
            return self.kind.offset(1.0, self.incoming);
        }
        self.kind.offset(((t - self.from).0 as f64 / (self.to - self.from).0 as f64) as f32, self.incoming)
    }
}

/// How a clip's layer lies against the clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    /// How long the layer starts before the clip.
    pub lead: Tick,
    /// The layer's length.
    pub len: Tick,
    /// Transitions over it.
    pub overlays: Vec<Overlay>,
}

impl Shape {
    /// The layer is the clip.
    pub fn of(clip: &Clip) -> Shape {
        Shape { lead: Tick::ZERO, len: clip.duration, overlays: vec![] }
    }
}

/// One property of one clip, as a function of time from its layer's start.
struct Curve<'a> {
    prop: Prop,
    clip: &'a Clip,
    shape: &'a Shape,
    /// The clip's keys, moved to layer time.
    keys: Vec<MKey>,
    animated: bool,
    overlays: Vec<Overlay>,
    /// Sound fades (volume only).
    fades: Fades,
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
    fn new<'a>(clip: &'a Clip, prop: Prop, shape: &'a Shape) -> Curve<'a> {
        let anim = clip.anim;
        let animated = [anim.enter, anim.exit].iter().flatten().any(|a| moves(a.kind, prop));
        let keys =
            clip.keys.get(&prop).map_or(vec![], |k| k.iter().map(|k| MKey { t: k.t + shape.lead, ..*k }).collect());
        let overlays = shape.overlays.iter().copied().filter(|o| shifts(o.kind, o.incoming, prop)).collect();
        let fades = if prop == Prop::VolumeDb { clip.fade } else { Fades::default() };
        Curve { prop, clip, shape, keys, animated, overlays, fades }
    }

    fn still(&self) -> bool {
        self.keys.is_empty() && !self.animated && self.overlays.is_empty() && self.fades.is_empty()
    }

    fn value(&self, t: Tick) -> f32 {
        let base = self.clip.value_at(self.prop, t - self.shape.lead);
        let mut o = if self.animated { self.clip.anim.offset(t, self.shape.len) } else { Offset::REST };
        for ov in &self.overlays {
            o = o.and(ov.offset(t));
        }
        let v = o.apply(self.prop, base);
        if self.fades.is_empty() {
            return v;
        }
        // a fade multiplies the amplitude; silence is held at -80 dB
        let g = self.fades.gain(t - self.shape.lead, self.clip.duration).max(1e-4);
        v + 20.0 * g.log10()
    }

    /// The stretches where an animation or transition reshapes this property.
    fn windows(&self) -> Vec<(Tick, Tick)> {
        let mut w: Vec<(Tick, Tick)> = self.overlays.iter().map(|o| (o.from, o.to)).collect();
        let (fi, fo) = self.fades.spans(self.clip.duration);
        let lead = self.shape.lead;
        if fi > Tick::ZERO {
            w.push((lead, lead + fi));
        }
        if fo > Tick::ZERO {
            w.push((lead + self.clip.duration - fo, lead + self.clip.duration));
        }
        if !self.animated {
            return w;
        }
        let len = self.shape.len;
        let anim = self.clip.anim;
        let (a, b) = anim.spans(len);
        if let Some(x) = anim.enter
            && a > Tick::ZERO
            && moves(x.kind, self.prop)
        {
            w.push((Tick::ZERO, a));
        }
        if let Some(x) = anim.exit
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
        let k = &self.keys;
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
/// animation or transition plays or easings disagree, and the layer's ends.
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
                // every frame boundary between t0 and t1, on the layer's own frame grid
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

/// Property `prop` of `clip`'s layer, laid out as `shape`; `step` is a frame.
pub fn lower(clip: &Clip, prop: Prop, shape: &Shape, step: Tick) -> Animated<f32> {
    let c = Curve::new(clip, prop, shape);
    if c.still() {
        return Animated::Constant(clip.base(prop));
    }
    let keys = plan(std::slice::from_ref(&c), step, shape.len);
    Animated::Keys(keys.into_iter().map(|(t, e)| Key { t, v: c.value(t), ease: e }).collect())
}

/// Position `[x, y]` of `clip`'s layer, keyed together.
pub fn lower_position(clip: &Clip, shape: &Shape, step: Tick) -> Animated<[f32; 2]> {
    let curves = [Curve::new(clip, Prop::X, shape), Curve::new(clip, Prop::Y, shape)];
    if curves.iter().all(Curve::still) {
        return Animated::Constant([clip.base(Prop::X), clip.base(Prop::Y)]);
    }
    let keys = plan(&curves, step, shape.len);
    let [x, y] = &curves;
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
            transition: None,
            fade: Default::default(),
            keep_pitch: true,
            color: Default::default(),
        }
    }

    fn key(t: i64, v: f32, ease: MEase) -> MKey {
        MKey { t: Tick(t), v, ease }
    }

    #[test]
    fn plain_values_stay_constant_and_keys_pass_through() {
        let mut c = clip(1000);
        assert_eq!(lower(&c, Prop::Opacity, &Shape::of(&c), Tick(10)), Animated::Constant(0.8));
        c.keys.insert(Prop::Scale, vec![key(0, 1.0, MEase::InOut), key(500, 2.0, MEase::Linear)]);
        let Animated::Keys(k) = lower(&c, Prop::Scale, &Shape::of(&c), Tick(10)) else { panic!() };
        assert_eq!(
            k.iter().map(|k| (k.t.0, k.v, k.ease)).collect::<Vec<_>>(),
            [(0, 1.0, Ease::InOut), (500, 2.0, Ease::Linear), (1000, 2.0, Ease::Linear)]
        );
        // a trim that cuts keys off keeps the values on screen
        c.shift_keys(Tick(-300));
        let lowered = lower(&c, Prop::Scale, &Shape::of(&c), Tick(10));
        let Animated::Keys(k) = &lowered else { panic!() };
        assert!(k.iter().all(|k| k.t >= Tick::ZERO && k.t <= c.duration));
        for t in [0, 55, 120, 200, 600] {
            assert!((lowered.sample(Tick(t)) - c.value_at(Prop::Scale, Tick(t))).abs() < 0.01, "{t}");
        }
        assert_eq!(lower_position(&c, &Shape::of(&c), Tick(10)), Animated::Constant([0.1, 0.0]));
    }

    #[test]
    fn an_animation_is_sampled_only_while_it_plays() {
        let mut c = clip(1000);
        c.anim.enter = Some(Animation { kind: AnimKind::Fade, duration: Tick(100) });
        let lowered = lower(&c, Prop::Opacity, &Shape::of(&c), Tick(10));
        let Animated::Keys(k) = &lowered else { panic!() };
        // 0, 10, …, 100 and the end
        assert_eq!(k.len(), 12);
        assert_eq!(k[0].v, 0.0);
        assert!((lowered.sample(Tick(50)) - 0.4).abs() < 1e-6);
        assert_eq!(lowered.sample(Tick(600)), 0.8);
        // a fade does not move the clip
        assert_eq!(lower_position(&c, &Shape::of(&c), Tick(10)), Animated::Constant([0.1, 0.0]));
    }

    #[test]
    fn animations_ride_on_keyframes() {
        let mut c = clip(1000);
        c.keys.insert(Prop::Y, vec![key(200, 0.0, MEase::InOut), key(800, 0.5, MEase::Linear)]);
        c.keys.insert(Prop::X, vec![key(200, 0.0, MEase::InOut), key(800, 0.4, MEase::Linear)]);
        c.anim.exit = Some(Animation { kind: AnimKind::SlideUp, duration: Tick(100) });
        let lowered = lower_position(&c, &Shape::of(&c), Tick(10));
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

    #[test]
    fn fades_shape_the_volume_in_decibels() {
        let mut c = clip(1000);
        c.volume_db = -6.0;
        c.fade = Fades { fade_in: Tick(100), fade_out: Tick(200) };
        let lowered = lower(&c, Prop::VolumeDb, &Shape::of(&c), Tick(10));
        assert!((lowered.sample(Tick(0)) - -86.0).abs() < 1e-3);
        assert_eq!(lowered.sample(Tick(500)), -6.0);
        let half = lowered.sample(Tick(50));
        assert!((half - (-6.0 + 20.0 * 0.5f32.log10())).abs() < 1e-3, "{half}");
        assert!((lowered.sample(Tick(1000)) - -86.0).abs() < 1e-3);
        // the picture is untouched
        assert_eq!(lower(&c, Prop::Opacity, &Shape::of(&c), Tick(10)), Animated::Constant(0.8));
    }
}
