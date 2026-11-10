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
//! Picture effects (`reelforge_model::fx`) are laid over the same way and sampled wherever they
//! play: a clip's own turn it about its own centre, an adjustment clip's turn everything below
//! about the centre of the frame, as one picture. What effects do to light, colour and focus
//! comes out as keys for `color.grade@1` and `blur.gaussian@1` ([`fx_channel`]).
//!
//! A clip's layer can be longer than the clip: a transition centred on a cut starts the incoming
//! layer early (its `lead`) and runs the outgoing one on. All times here are from the layer's
//! start; the clip's keys are shifted by the lead, and animations play over the whole layer.

use reelforge_model::{
    Clip, Ease as MEase, Fades, Fx, FxChannel, FxSample, Key as MKey, Offset, Prop, Steadied, Timing, TransitionKind,
};
use reelforge_renderspec::{Animated, CharCurve, CharMotion, CharPose, Ease, Key, TextDraw, TextHighlight, TextUnit};
use reelforge_time::Tick;

/// Whether an animation preset changes `p`.
fn moves(kind: reelforge_model::AnimKind, p: Prop) -> bool {
    use reelforge_model::AnimKind::*;
    match p {
        Prop::Opacity => !kind.titles_only() && !kind.drawn_only(),
        Prop::X => matches!(kind, SlideLeft | SlideRight),
        Prop::Y => matches!(kind, SlideUp | SlideDown | Drop),
        Prop::Scale => matches!(kind, ZoomIn | ZoomOut | Pop | Spin | Elastic),
        Prop::Rotation => matches!(kind, Spin),
        Prop::RotationX => kind == FlipX,
        Prop::RotationY => kind == FlipY,
        _ => false,
    }
}

/// Whether a looping animation changes `p`.
fn loops(kind: reelforge_model::LoopKind, p: Prop) -> bool {
    use reelforge_model::LoopKind::*;
    match p {
        Prop::Opacity => kind == Blink,
        Prop::X => kind == Shake,
        Prop::Y => matches!(kind, Float | Bounce | Shake),
        Prop::Scale => matches!(kind, Breathe | Heartbeat),
        Prop::Rotation => matches!(kind, Swing | Spin),
        Prop::RotationX => kind == Tilt,
        Prop::RotationY => matches!(kind, Flip | Tilt),
        _ => false,
    }
}

/// How a title's characters move one after another over its layer (`chars`): coming in, going
/// out, and round and round in between. Empty when its animations move it only as a whole.
pub fn chars(clip: &Clip, shape: &Shape) -> Vec<CharMotion> {
    use reelforge_model::{AnimKind, LoopKind};
    let key = |t: Tick, v: f32| Key { t, v, ease: Ease::Linear };
    let len = shape.len;
    let (a, b) = clip.anim.spans(len);
    // where a character (or a word, or a line) waits before it comes in, and goes as it leaves;
    // how it travels, and how much later each starts than the one before
    let pose = |kind: AnimKind| -> Option<(CharCurve, CharPose, TextUnit, f32)> {
        let hidden = CharPose { opacity: 0.0, ..CharPose::REST };
        let (below, above) = (CharPose { offset: [0.0, 0.6], ..hidden }, CharPose { offset: [0.0, -0.6], ..hidden });
        let nothing = CharPose { scale: 0.0, ..hidden };
        match kind {
            AnimKind::CharFade => Some((CharCurve::Settle, hidden, TextUnit::Char, 0.35)),
            AnimKind::CharRise => Some((CharCurve::Settle, below, TextUnit::Char, 0.35)),
            AnimKind::CharDrop => Some((CharCurve::Settle, above, TextUnit::Char, 0.35)),
            AnimKind::CharPop => Some((CharCurve::Spring, nothing, TextUnit::Char, 0.35)),
            AnimKind::WordFade => Some((CharCurve::Settle, hidden, TextUnit::Word, 0.5)),
            AnimKind::WordRise => Some((CharCurve::Settle, below, TextUnit::Word, 0.5)),
            AnimKind::WordPop => Some((CharCurve::Spring, nothing, TextUnit::Word, 0.5)),
            AnimKind::LineRise => Some((CharCurve::Settle, below, TextUnit::Line, 0.6)),
            // a spring and a ball take their time: each starts well before the one before is still
            AnimKind::CharElastic => Some((CharCurve::Elastic, nothing, TextUnit::Char, 0.15)),
            AnimKind::CharBounce => {
                Some((CharCurve::Bounce, CharPose { offset: [0.0, -1.2], ..hidden }, TextUnit::Char, 0.15))
            }
            _ => None,
        }
    };
    let mut out = vec![];
    if let Some((curve, pose, unit, step)) = clip.anim.enter.and_then(|x| pose(x.kind))
        && a > Tick::ZERO
    {
        let mut keys = vec![key(Tick::ZERO, 0.0), key(a, 1.0)];
        if a < len {
            keys.push(key(len, 1.0));
        }
        out.push(CharMotion { progress: Animated::Keys(keys), step, curve, pose, reverse: false, unit });
    }
    if let Some((curve, pose, unit, step)) = clip.anim.exit.and_then(|x| pose(x.kind))
        && b > Tick::ZERO
    {
        // going out nothing springs or bounces: it leaves the way it would have settled
        let curve = match curve {
            CharCurve::Elastic | CharCurve::Bounce => CharCurve::Settle,
            c => c,
        };
        let mut keys = vec![key(len - b, 1.0), key(len, 0.0)];
        if b < len {
            keys.insert(0, key(Tick::ZERO, 1.0));
        }
        // the first character goes first
        out.push(CharMotion { progress: Animated::Keys(keys), step, curve, pose, reverse: true, unit });
    }
    if let Some(l) = clip.anim.repeat
        && l.kind.titles_only()
        && len > Tick::ZERO
    {
        let (curve, pose, step) = match l.kind {
            LoopKind::Jump => (CharCurve::Hop, CharPose { offset: [0.0, -0.35], ..CharPose::REST }, 0.12),
            _ => (CharCurve::Wave, CharPose { offset: [0.0, -0.18], ..CharPose::REST }, 0.15),
        };
        // so many rounds by the layer's end; each character is a little behind the one before,
        // so it runs from the first to the last
        let rounds = l.phase(len) as f32;
        let progress = Animated::Keys(vec![key(Tick::ZERO, 0.0), key(len, rounds)]);
        out.push(CharMotion { progress, step, curve, pose, reverse: false, unit: TextUnit::Char });
    }
    out
}

/// How a title is written on over its layer, when it comes in or goes out with `draw`: a pen
/// follows each character's outline and its ink comes in behind. `None` when it does neither.
pub fn written(clip: &Clip, shape: &Shape) -> Option<TextDraw> {
    use reelforge_model::AnimKind::Draw;
    let len = shape.len;
    let (a, b) = clip.anim.spans(len);
    let enter = clip.anim.enter.filter(|x| x.kind == Draw && a > Tick::ZERO).map(|_| a);
    let exit = clip.anim.exit.filter(|x| x.kind == Draw && b > Tick::ZERO).map(|_| b);
    if enter.is_none() && exit.is_none() {
        return None;
    }
    let key = |t: Tick, v: f32| Key { t, v, ease: Ease::Linear };
    let mut keys = vec![];
    if let Some(a) = enter {
        keys.extend([key(Tick::ZERO, 0.0), key(a, 1.0)]);
    }
    if let Some(b) = exit {
        keys.extend([key(len - b, 1.0), key(len, 0.0)]);
    }
    keys.dedup_by_key(|k| k.t);
    Some(TextDraw { progress: Animated::Keys(keys), width: 0.025, step: 0.4 })
}

/// What of a title takes its karaoke colour over its layer: nothing before the voice starts,
/// then more and more of it (or the word or line the voice is in), in a straight line over the
/// time the voice takes.
pub fn said(clip: &Clip, shape: &Shape) -> Option<TextHighlight> {
    use reelforge_model::KaraokeMode;
    let reelforge_model::ClipContent::Text { style, .. } = &clip.content else { return None };
    let k = style.karaoke?;
    let from = (shape.lead + k.start).max(Tick::ZERO);
    let takes = k.duration.unwrap_or(clip.duration - k.start).max(Tick(1));
    let color = reelforge_renderspec::Color(k.color);
    let unit = match k.mode {
        KaraokeMode::Sweep => TextUnit::Char,
        KaraokeMode::Word => TextUnit::Word,
        KaraokeMode::Line => TextUnit::Line,
    };
    if from >= shape.len {
        // the layer is over before the voice starts
        return Some(TextHighlight { progress: Animated::Constant(-1.0), color, unit });
    }
    // as far as the voice gets before the layer ends
    let to = (from + takes).min(shape.len);
    let reached = ((to - from).0 as f64 / takes.0 as f64) as f32;
    let mut keys = vec![Key { t: from, v: 0.0, ease: Ease::Linear }, Key { t: to, v: reached, ease: Ease::Linear }];
    // a word is not lit before the voice gets to it
    if from > Tick::ZERO && k.mode != KaraokeMode::Sweep {
        keys.insert(0, Key { t: Tick::ZERO, v: -1.0, ease: Ease::Hold });
    }
    Some(TextHighlight { progress: Animated::Keys(keys), color, unit })
}

/// How much of a title shows over its layer (`reveal`), when it types in or erases itself out: 0
/// to 1, in a straight line over each animation. `None` when neither does.
pub fn reveal(clip: &Clip, shape: &Shape) -> Option<Animated<f32>> {
    use reelforge_model::AnimKind::Typewriter;
    let enter = clip.anim.enter.filter(|a| a.kind == Typewriter).map(|a| a.duration.min(shape.len));
    let exit = clip.anim.exit.filter(|a| a.kind == Typewriter).map(|a| a.duration.min(shape.len));
    if enter.is_none() && exit.is_none() {
        return None;
    }
    let key = |t: Tick, v: f32| Key { t, v, ease: Ease::Linear };
    let mut keys = Vec::new();
    // where typing ends and erasing starts, when they would cross
    let (mut typed, mut erase) = (enter.unwrap_or(Tick::ZERO), shape.len - exit.unwrap_or(Tick::ZERO));
    if typed > erase {
        let mid = Tick((typed.0 + erase.0) / 2);
        (typed, erase) = (mid, mid);
    }
    if enter.is_some() {
        keys.push(key(Tick::ZERO, 0.0));
    }
    keys.push(key(typed, 1.0));
    if erase > typed {
        keys.push(key(erase, 1.0));
    }
    if exit.is_some() {
        keys.push(key(shape.len, 0.0));
    }
    keys.dedup_by_key(|k| k.t);
    Some(Animated::Keys(keys))
}

/// Whether a transition changes `p` on the incoming or outgoing clip.
fn shifts(kind: TransitionKind, incoming: bool, p: Prop) -> bool {
    use TransitionKind::*;
    match p {
        Prop::Opacity => incoming && matches!(kind, Dissolve | FadeBlack | FadeWhite | Zoom),
        Prop::X => matches!((kind, incoming), (PushLeft | PushRight, _) | (SlideLeft | SlideRight, true)),
        Prop::Y => matches!((kind, incoming), (PushUp | PushDown, _) | (SlideUp | SlideDown, true)),
        Prop::Scale => kind == Zoom,
        _ => false,
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

/// An effect of an adjustment clip above, over part of a layer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Frame {
    /// Where it starts on the layer, from the layer's start.
    pub from: Tick,
    /// Where it ends.
    pub to: Tick,
    /// Where the adjustment clip starts, from the layer's start (before it, when the clip began
    /// earlier).
    pub origin: Tick,
    /// The adjustment clip's length.
    pub len: Tick,
    /// The effect.
    pub fx: Fx,
}

impl Frame {
    fn sample(&self, t: Tick) -> FxSample {
        if t < self.from || t >= self.to { FxSample::REST } else { self.fx.sample(t - self.origin, self.len) }
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
    /// Effects of adjustment clips above it.
    pub frames: Vec<Frame>,
    /// The canvas, in pixels: effects from above turn the layer about its centre.
    pub canvas: [f32; 2],
    /// What holds the layer's picture steady, when it is steadied.
    pub steady: Option<Fix>,
    /// Whether another clip carries it: effects from above then move what carries it, and it
    /// goes along.
    pub carried: bool,
}

/// A steadied clip's picture, moved against the camera's shake (`reelforge_model::Steady`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The move at each instant of the source, and how much larger the picture is drawn.
    pub moves: Steadied,
    /// Which source instant shows when.
    pub timing: Timing,
    /// The whole picture's size in canvas pixels as it is drawn at scale 1.
    pub picture: [f32; 2],
    /// -1 where the picture is mirrored (across, down), else 1.
    pub flip: [f32; 2],
}

impl Fix {
    /// The fix for `clip`, whose file's picture is `size` pixels, on a `canvas`; nothing when the
    /// clip is not steadied.
    pub fn of(clip: &Clip, size: [u32; 2], canvas: [f32; 2]) -> Option<Fix> {
        let steady = clip.steady.as_ref()?;
        let timing = clip.timing()?;
        let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
        // the part the crop leaves is what is fitted inside the canvas
        let crop = clip.look.crop;
        let (vw, vh) = (w * (1.0 - crop.left - crop.right).max(1e-3), h * (1.0 - crop.top - crop.bottom).max(1e-3));
        let fit = (canvas[0] / vw).min(canvas[1] / vh);
        let flip = |on: bool| if on { -1.0 } else { 1.0 };
        Some(Fix {
            moves: steady.steadied(w / h),
            timing,
            picture: [w * fit, h * fit],
            flip: [flip(clip.look.flip_x), flip(clip.look.flip_y)],
        })
    }

    /// The move at `t` from the clip's start: across and down in the picture's own frame as
    /// fractions of its size, and the turn in degrees, as they show (a mirrored picture moves
    /// and turns the other way).
    fn at(&self, t: Tick) -> [f32; 3] {
        let [x, y, turn] = self.moves.at(self.timing.source_in + self.timing.travel(t));
        [x * self.flip[0], y * self.flip[1], turn * self.flip[0] * self.flip[1]]
    }
}

impl Shape {
    /// The layer is the clip.
    pub fn of(clip: &Clip) -> Shape {
        Shape {
            lead: Tick::ZERO,
            len: clip.duration,
            overlays: vec![],
            frames: vec![],
            canvas: [1.0, 1.0],
            steady: None,
            carried: false,
        }
    }

    /// What the clip's own effects and those from above do to `channel`, at `t` from the layer's
    /// start.
    fn channel(&self, clip: &Clip, channel: FxChannel, t: Tick) -> f32 {
        let mut s = FxSample::REST;
        for fx in clip.fx.iter().filter(|fx| fx.kind.changes(channel)) {
            s = s.and(fx.sample(t - self.lead, clip.duration));
        }
        for f in self.frames.iter().filter(|f| f.fx.kind.changes(channel)) {
            s = s.and(f.sample(t));
        }
        s.channel(channel)
    }
}

/// What the effects on `clip`'s layer do to `channel`, sampled at every frame while they play;
/// `None` when none touches it.
pub fn fx_channel(clip: &Clip, shape: &Shape, channel: FxChannel, step: Tick) -> Option<Animated<f32>> {
    let own = clip.fx.iter().any(|fx| fx.kind.changes(channel));
    let mut windows: Vec<(Tick, Tick)> =
        shape.frames.iter().filter(|f| f.fx.kind.changes(channel)).map(|f| (f.from, f.to)).collect();
    if own {
        windows = vec![(Tick::ZERO, shape.len)];
    }
    if windows.is_empty() {
        return None;
    }
    windows.sort();
    let mut times = vec![Tick::ZERO];
    for (a, b) in windows {
        let mut t = a;
        while t < b {
            times.push(t);
            t = Tick((t.0 / step.0 + 1) * step.0);
        }
        times.push(b);
    }
    times.push(shape.len);
    times.sort();
    times.dedup();
    // an effect from above starts at once: the value before it holds
    let starts = |t: Tick| !own && shape.frames.iter().any(|f| f.from == t && f.fx.kind.changes(channel));
    let keys = times.iter().enumerate().map(|(i, &t)| {
        let ease = if times.get(i + 1).is_some_and(|n| starts(*n)) { Ease::Hold } else { Ease::Linear };
        Key { t, v: shape.channel(clip, channel, t), ease }
    });
    Some(Animated::Keys(keys.collect()))
}

/// A value that steps: `spans` are `(from, to, value)` on a layer `len` long, the value 0 outside
/// them and added up where they overlap. `None` when it is 0 throughout.
pub fn stepped(spans: &[(Tick, Tick, f32)], len: Tick, range: (f32, f32)) -> Option<Animated<f32>> {
    if spans.iter().all(|s| s.2 == 0.0) {
        return None;
    }
    let mut times: Vec<Tick> = spans.iter().flat_map(|s| [s.0, s.1]).chain([Tick::ZERO, len]).collect();
    times.retain(|t| *t >= Tick::ZERO && *t <= len);
    times.sort();
    times.dedup();
    let at = |t: Tick| spans.iter().filter(|s| s.0 <= t && t < s.1).map(|s| s.2).sum::<f32>().clamp(range.0, range.1);
    Some(Animated::Keys(times.into_iter().map(|t| Key { t, v: at(t), ease: Ease::Hold }).collect()))
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
    /// The clip's own effects that move this property.
    own: Vec<Fx>,
    /// Effects from above that move it.
    frames: Vec<Frame>,
    /// The stretch of the layer over which the clip's path moves this property.
    journey: Option<(Tick, Tick)>,
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
        let animated = [anim.enter, anim.exit].iter().flatten().any(|a| moves(a.kind, prop))
            || anim.repeat.is_some_and(|l| loops(l.kind, prop));
        let keys =
            clip.keys.get(&prop).map_or(vec![], |k| k.iter().map(|k| MKey { t: k.t + shape.lead, ..*k }).collect());
        let overlays = shape.overlays.iter().copied().filter(|o| shifts(o.kind, o.incoming, prop)).collect();
        let fades = if prop == Prop::VolumeDb { clip.fade } else { Fades::default() };
        let own = clip.fx.iter().copied().filter(|fx| fx.kind.moves(prop)).collect();
        // turning or scaling the frame about its centre moves whatever is off-centre
        let frames = shape
            .frames
            .iter()
            .copied()
            .filter(|f| match prop {
                // what is carried is moved by what carries it
                Prop::X | Prop::Y | Prop::Scale | Prop::Rotation if shape.carried => false,
                Prop::X | Prop::Y => f.fx.kind.moves_any(),
                p => f.fx.kind.moves(p),
            })
            .collect();
        let journey = clip.path.as_ref().filter(|p| p.points.len() >= 2).and_then(|p| {
            let moved = matches!(prop, Prop::X | Prop::Y) || (prop == Prop::Rotation && p.orient);
            moved.then_some((shape.lead + p.start, shape.lead + p.start + p.duration))
        });
        Curve { prop, clip, shape, keys, animated, overlays, fades, own, frames, journey }
    }

    /// Property `p` at `t` as the clip's keys have it, or its path, before anything is laid over.
    fn base(&self, p: Prop, t: Tick) -> f32 {
        let at = t - self.shape.lead;
        let own = self.clip.value_at(p, at);
        let [w, h] = self.shape.canvas;
        match (p, self.clip.on_path(at, w / h)) {
            (Prop::X, Some((to, _))) => to[0],
            (Prop::Y, Some((to, _))) => to[1],
            (Prop::Rotation, Some((_, heading))) => own + heading,
            _ => own,
        }
    }

    fn still(&self) -> bool {
        self.keys.is_empty()
            && self.journey.is_none()
            && !self.steady()
            && !self.animated
            && self.overlays.is_empty()
            && self.fades.is_empty()
            && self.own.is_empty()
            && self.frames.is_empty()
    }

    /// Property `p` at `t` as the clip itself has it: its keys, animations, transitions and own
    /// effects, with its picture held steady.
    fn own_value(&self, p: Prop, t: Tick) -> f32 {
        self.steadied(p, t).apply(p, self.shaky(p, t))
    }

    /// [`own_value`](Self::own_value) before the picture is steadied.
    fn shaky(&self, p: Prop, t: Tick) -> f32 {
        let base = self.base(p, t);
        let mut o = self.clip.anim.offset(t, self.shape.len);
        for ov in self.shape.overlays.iter().filter(|o| shifts(o.kind, o.incoming, p)) {
            o = o.and(ov.offset(t));
        }
        for fx in &self.clip.fx {
            o = o.and(fx.sample(t - self.shape.lead, self.clip.duration).offset);
        }
        o.apply(p, base)
    }

    /// What holding the picture steady does to property `p` at `t`: the picture is drawn larger
    /// and turned against the shake, and moved against it in its own frame, so as far as it is
    /// drawn large and the way it is turned.
    fn steadied(&self, p: Prop, t: Tick) -> Offset {
        let Some(fix) = &self.shape.steady else { return Offset::REST };
        let [x, y, turn] = fix.at((t - self.shape.lead).max(Tick::ZERO).min(self.clip.duration));
        match p {
            Prop::Scale => Offset { scale: fix.moves.zoom, ..Offset::REST },
            Prop::Rotation => Offset { rotation: turn, ..Offset::REST },
            Prop::X | Prop::Y => {
                let scale = self.shaky(Prop::Scale, t) * fix.moves.zoom;
                let (sin, cos) = (self.shaky(Prop::Rotation, t) + turn).to_radians().sin_cos();
                let (a, b) = (x * fix.picture[0] * scale, y * fix.picture[1] * scale);
                let [w, h] = self.shape.canvas;
                Offset { dx: (cos * a - sin * b) / w, dy: (sin * a + cos * b) / h, ..Offset::REST }
            }
            _ => Offset::REST,
        }
    }

    /// Whether the picture is held steady in a way that changes this property.
    fn steady(&self) -> bool {
        self.shape.steady.is_some() && matches!(self.prop, Prop::X | Prop::Y | Prop::Scale | Prop::Rotation)
    }

    /// The value at `t` under the effects from above: the frame turned, scaled and moved as one
    /// picture, about the canvas centre.
    fn framed(&self, t: Tick) -> f32 {
        let mut f = Offset::REST;
        for fr in &self.frames {
            f = f.and(fr.sample(t).offset);
        }
        match self.prop {
            Prop::X | Prop::Y => {
                let [w, h] = self.shape.canvas;
                let (x, y) = (self.own_value(Prop::X, t) * w, self.own_value(Prop::Y, t) * h);
                let (sin, cos) = f.rotation.to_radians().sin_cos();
                if self.prop == Prop::X {
                    (cos * x - sin * y) * f.scale / w + f.dx
                } else {
                    (sin * x + cos * y) * f.scale / h + f.dy
                }
            }
            p => f.apply(p, self.own_value(p, t)),
        }
    }

    fn value(&self, t: Tick) -> f32 {
        if !self.frames.is_empty() {
            return self.framed(t);
        }
        let base = self.base(self.prop, t);
        let mut o = if self.animated { self.clip.anim.offset(t, self.shape.len) } else { Offset::REST };
        for ov in &self.overlays {
            o = o.and(ov.offset(t));
        }
        for fx in &self.own {
            o = o.and(fx.sample(t - self.shape.lead, self.clip.duration).offset);
        }
        let v = self.steadied(self.prop, t).apply(self.prop, o.apply(self.prop, base));
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
        w.extend(self.frames.iter().map(|f| (f.from, f.to)));
        // along a curve, frame by frame
        w.extend(self.journey);
        if !self.own.is_empty() || (self.steady() && self.prop != Prop::Scale) {
            // an effect or a steadied picture moves all the while (steadied, its size alone does
            // not: it is as much larger from start to end)
            w.push((Tick::ZERO, self.shape.len));
        }
        if !self.animated {
            return w;
        }
        let len = self.shape.len;
        let anim = self.clip.anim;
        if anim.repeat.is_some_and(|l| loops(l.kind, self.prop)) {
            w.push((Tick::ZERO, len));
        }
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
            // a curve drawn by hand is followed frame by frame
            MEase::Bezier(_) => Fits::Sample,
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
        // an effect from above starts at once: the value jumps at `t1`
        let jump = curves.iter().any(|c| c.frames.iter().any(|f| f.from == t1));
        match agreed {
            Some(Fits::Any) if jump => out.push((t0, Ease::Hold)),
            Some(Fits::Any) => out.push((t0, Ease::Linear)),
            Some(Fits::Only(e)) if !jump => out.push((t0, e)),
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
    lower_pair(clip, [Prop::X, Prop::Y], shape, step)
}

/// Two properties of `clip`'s layer as one value, keyed together.
pub fn lower_pair(clip: &Clip, props: [Prop; 2], shape: &Shape, step: Tick) -> Animated<[f32; 2]> {
    let curves = props.map(|p| Curve::new(clip, p, shape));
    if curves.iter().all(Curve::still) {
        return Animated::Constant(props.map(|p| clip.base(p)));
    }
    let keys = plan(&curves, step, shape.len);
    let [x, y] = &curves;
    Animated::Keys(keys.into_iter().map(|(t, e)| Key { t, v: [x.value(t), y.value(t)], ease: e }).collect())
}

/// `f` at every frame of a layer `len` long from its start up to `until`, and held from there to
/// its end; `step` is a frame.
fn framed_keys(len: Tick, step: Tick, until: Tick, f: impl Fn(Tick) -> f32) -> Animated<f32> {
    let until = until.clamp(Tick::ZERO, len);
    let mut keys = vec![];
    let mut t = Tick::ZERO;
    while t < until {
        keys.push(Key { t, v: f(t), ease: Ease::Linear });
        t = Tick((t.0 / step.0 + 1) * step.0);
    }
    keys.push(Key { t: until, v: f(until), ease: Ease::Linear });
    if until < len {
        keys.push(Key { t: len, v: f(len), ease: Ease::Linear });
    }
    keys.dedup_by_key(|k| k.t);
    Animated::Keys(keys)
}

/// How a shape's outline is drawn over its layer (its trim, start and end) and how much of its
/// fill shows: its `drawStart` and `drawEnd` and its `draw` animations. `None` for each where
/// nothing changes it.
pub fn draw(
    clip: &Clip,
    shape: &Shape,
    step: Tick,
    stroked: bool,
) -> (Option<[Animated<f32>; 2]>, Option<Animated<f32>>) {
    use reelforge_model::AnimKind::Draw;
    let len = shape.len;
    let (a, b) = clip.anim.spans(len);
    let enter = clip.anim.enter.filter(|x| x.kind == Draw && a > Tick::ZERO).map(|_| a);
    let exit = clip.anim.exit.filter(|x| x.kind == Draw && b > Tick::ZERO).map(|_| b);
    let keyed = Prop::DRAW.iter().any(|p| clip.keyed(*p));
    let base = Prop::DRAW.map(|p| clip.base(p));
    // how far in (1 at rest) the clip is at `t`, coming in and going out
    let into = |t: Tick| enter.map_or(1.0, |a| (t.0 as f64 / a.0 as f64).clamp(0.0, 1.0) as f32);
    let away = |t: Tick| exit.map_or(1.0, |b| ((len - t).0 as f64 / b.0 as f64).clamp(0.0, 1.0) as f32);
    let ease = |u: f32| 1.0 - (1.0 - u.clamp(0.0, 1.0)).powi(3);
    let trim = (stroked && (keyed || base != [0.0, 1.0] || enter.is_some() || exit.is_some())).then(|| {
        let start = |t: Tick| {
            let s = clip.value_at(Prop::DrawStart, t - shape.lead);
            let e = clip.value_at(Prop::DrawEnd, t - shape.lead);
            // going out, the outline is wiped from its start
            s + (e - s).max(0.0) * (1.0 - ease(away(t) / 0.7))
        };
        let end = |t: Tick| clip.value_at(Prop::DrawEnd, t - shape.lead) * ease(into(t) / 0.7);
        if !keyed && enter.is_none() && exit.is_none() {
            return [Animated::Constant(base[0]), Animated::Constant(base[1])];
        }
        [framed_keys(len, step, len, start), framed_keys(len, step, len, end)]
    });
    let fill = (enter.is_some() || exit.is_some()).then(|| {
        // with an outline the fill follows it in and leaves before it; without, it fades
        let k = |u: f32| match stroked {
            _ if u >= 1.0 => 1.0,
            true => ((u - 0.6) / 0.4).clamp(0.0, 1.0),
            false => u,
        };
        framed_keys(len, step, len, |t| k(into(t)) * k(away(t)))
    });
    (trim, fill)
}

/// The number a title's counter shows over its layer, or `None` when it has none.
pub fn count(clip: &Clip, shape: &Shape, step: Tick) -> Option<Animated<f32>> {
    let reelforge_model::ClipContent::Text { style, .. } = &clip.content else { return None };
    let c = style.counter?;
    if clip.keyed(Prop::Count) {
        return Some(framed_keys(shape.len, step, shape.len, |t| clip.value_at(Prop::Count, t - shape.lead)));
    }
    let until = shape.lead + c.over.unwrap_or(clip.duration);
    Some(framed_keys(shape.len, step, until, |t| c.at((t - shape.lead).max(Tick::ZERO), clip.duration)))
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
            look: Default::default(),
            fx: vec![],
            voice: Default::default(),
            group: None,
            slot: None,
            steady: None,
            parent: None,
            path: None,
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
    fn a_loop_is_sampled_all_the_way_and_only_where_it_moves() {
        use reelforge_model::{Loop, LoopKind};
        let mut c = clip(1000);
        c.anim.repeat = Some(Loop { kind: LoopKind::Swing, period: Tick(200) });
        let turned = lower(&c, Prop::Rotation, &Shape::of(&c), Tick(10));
        let Animated::Keys(k) = &turned else { panic!() };
        assert_eq!(k.len(), 101);
        assert!((turned.sample(Tick(50)) - 8.0).abs() < 1e-3 && (turned.sample(Tick(150)) + 8.0).abs() < 1e-3);
        assert!(turned.sample(Tick(400)).abs() < 1e-3);
        // a swing neither fades nor moves the clip
        assert_eq!(lower(&c, Prop::Opacity, &Shape::of(&c), Tick(10)), Animated::Constant(0.8));
        assert_eq!(lower_position(&c, &Shape::of(&c), Tick(10)), Animated::Constant([0.1, 0.0]));
        assert!(chars(&c, &Shape::of(&c)).is_empty());
    }

    #[test]
    fn characters_come_in_go_out_and_wave_in_between() {
        use reelforge_model::{Loop, LoopKind};
        let mut c = clip(1000);
        c.anim.enter = Some(Animation { kind: AnimKind::CharRise, duration: Tick(300) });
        c.anim.exit = Some(Animation { kind: AnimKind::CharPop, duration: Tick(200) });
        c.anim.repeat = Some(Loop { kind: LoopKind::Wave, period: Tick(250) });
        // the title itself stays where it is, whole
        assert_eq!(lower(&c, Prop::Opacity, &Shape::of(&c), Tick(10)), Animated::Constant(0.8));
        assert_eq!(lower(&c, Prop::Scale, &Shape::of(&c), Tick(10)), Animated::Constant(1.0));
        let m = chars(&c, &Shape::of(&c));
        assert_eq!(m.len(), 3);
        let at = |i: usize, t: i64| m[i].progress.sample(Tick(t));
        assert_eq!([at(0, 0), at(0, 150), at(0, 300), at(0, 900)], [0.0, 0.5, 1.0, 1.0]);
        assert_eq!([at(1, 0), at(1, 800), at(1, 900), at(1, 1000)], [1.0, 1.0, 0.5, 0.0]);
        assert_eq!([at(2, 0), at(2, 500), at(2, 1000)], [0.0, 2.0, 4.0]);
        assert_eq!((m[0].curve, m[1].curve, m[2].curve), (CharCurve::Settle, CharCurve::Spring, CharCurve::Wave));
        assert!(!m[0].reverse && m[1].reverse);
        // of four characters coming in, the first is there before the last has started
        let first = m[0].at(0, 4, 0.5);
        assert_eq!((first.opacity, first.offset), (1.0, [0.0, 0.0]));
        let last = m[0].at(3, 4, 0.5);
        assert_eq!((last.opacity, last.offset), (0.0, [0.0, 0.6]));
        assert_eq!(m[0].at(3, 4, 1.0), CharPose::REST);
        // going out, the first leaves first
        assert_eq!(m[1].at(3, 4, 0.6), CharPose::REST);
        assert_eq!(m[1].at(0, 4, 0.0).opacity, 0.0);
        assert!(m[1].at(0, 4, 0.4).scale < 1.0);
        // the wave: a quarter of a round in the first character is at its top, the next on its way
        let top = m[2].at(0, 4, 0.25);
        assert!((top.offset[1] + 0.18).abs() < 1e-6 && top.opacity == 1.0);
        let next = m[2].at(1, 4, 0.25).offset[1];
        assert!(next < 0.0 && next > -0.18, "{next}");
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
