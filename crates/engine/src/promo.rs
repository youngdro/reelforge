//! Shapes, counters and the promotional pieces built from them: price tags, discount badges,
//! banners, countdowns, sales counters, struck-out prices and "new" stamps (`promo.add`).
//!
//! A piece is ordinary clips, a shape and a title (and a line), grouped so they are picked and
//! moved together, each on a track above the one before so they lie in order. Everything about
//! them can be changed afterwards with the usual commands.

use reelforge_model::{
    AnimKind, Animation, Clip, ClipContent, ClipTransform, Counter, Ease, Fill, Loop, LoopKind, Project, Shape,
    ShapeKind, ShapeShadow, Stroke, TitleStyle, Track, TrackKind,
};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{fps, not_found};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

fn wrong(name: &str, what: &str) -> EngineError {
    EngineError::Params(format!("`{name}` must be {what}"))
}

/// A fill as a command gives it: a colour `[r, g, b, a]`, or `{"type": "linear", "from", "to",
/// "angle"?}`, or `{"type": "radial", "from", "to"}`.
fn fill(v: &Value) -> Result<Fill, EngineError> {
    let what = "a colour [r, g, b, a], {\"type\": \"linear\", \"from\", \"to\", \"angle\"?} or {\"type\": \"radial\", \"from\", \"to\"}";
    if v.is_array() {
        let wrapped = json!({ "fill": v });
        let p = P::new(&wrapped, reelforge_time::FrameRate::FPS_30);
        return Ok(Fill::Solid { color: p.opt_color("fill")?.ok_or_else(|| wrong("fill", what))? });
    }
    let p = P::new(v, reelforge_time::FrameRate::FPS_30);
    let known = v.as_object().is_some_and(|o| o.keys().all(|k| ["type", "from", "to", "angle"].contains(&k.as_str())));
    let (Some(from), Some(to), true) = (p.opt_color("from")?, p.opt_color("to")?, known) else {
        return Err(wrong("fill", what));
    };
    match v.get("type").and_then(Value::as_str) {
        Some("linear") => Ok(Fill::Linear { from, to, angle: p.opt_f64("angle")?.unwrap_or(0.0) as f32 }),
        Some("radial") if v.get("angle").is_none() => Ok(Fill::Radial { from, to }),
        _ => Err(wrong("fill", what)),
    }
}

/// A colour inside `v`, an object a command gave.
fn colour_in(v: &Value, name: &str) -> Result<[f32; 4], EngineError> {
    P::new(v, reelforge_time::FrameRate::FPS_30)
        .opt_color("color")?
        .ok_or_else(|| wrong(name, "an object with a `color`"))
}

/// Changes `shape` as the parameters of `clip.add`'s `shape` or of `clip.setShape` say.
fn set_shape(shape: &mut Shape, v: &Value) -> Result<(), EngineError> {
    let p = P::new(v, reelforge_time::FrameRate::FPS_30);
    if let Some(k) = p.opt_str("kind")? {
        shape.kind = ShapeKind::parse(k)
            .ok_or_else(|| wrong("kind", &format!("one of {}", ShapeKind::ALL.map(ShapeKind::name).join(", "))))?;
    }
    if let Some(s) = p.opt("size") {
        shape.size = serde_json::from_value(s.clone()).map_err(|_| wrong("size", "[width, height] in pixels"))?;
    }
    if let Some(r) = p.opt_f64("radius")? {
        shape.radius = r as f32;
    }
    if let Some(n) = p.opt_u32("points")? {
        shape.points = n;
    }
    if let Some(i) = p.opt_f64("inner")? {
        shape.inner = i as f32;
    }
    if let Some(f) = v.get("fill") {
        shape.fill = if f.is_null() { None } else { Some(fill(f)?) };
    }
    if let Some(st) = v.get("stroke") {
        shape.stroke = if st.is_null() {
            None
        } else {
            let known = st.as_object().is_some_and(|o| o.keys().all(|k| ["color", "width"].contains(&k.as_str())));
            let width = st.get("width").and_then(Value::as_f64).filter(|_| known);
            let width = width.ok_or_else(|| wrong("stroke", "{\"color\", \"width\"} or null"))?;
            Some(Stroke { color: colour_in(st, "stroke")?, width: width as f32 })
        };
    }
    if let Some(sh) = v.get("shadow") {
        shape.shadow = if sh.is_null() {
            None
        } else {
            let what = "{\"color\", \"offset\": [x, y], \"blur\"?} or null";
            let known =
                sh.as_object().is_some_and(|o| o.keys().all(|k| ["color", "offset", "blur"].contains(&k.as_str())));
            let offset = sh.get("offset").and_then(|o| serde_json::from_value::<[f32; 2]>(o.clone()).ok());
            let offset = offset.filter(|_| known).ok_or_else(|| wrong("shadow", what))?;
            let blur = match sh.get("blur") {
                None => 0.0,
                Some(b) => b.as_f64().ok_or_else(|| wrong("shadow", what))? as f32,
            };
            Some(ShapeShadow { color: colour_in(sh, "shadow")?, offset, blur })
        };
    }
    if let Some(t) = p.opt("trim") {
        shape.trim = serde_json::from_value(t.clone()).map_err(|_| wrong("trim", "[start, end], each 0 to 1"))?;
    }
    match shape.problem() {
        Some(why) => Err(EngineError::Params(why.into())),
        None => Ok(()),
    }
}

/// The shape `clip.add` is given: its `shape` parameter, from a default box.
pub(crate) fn shape_param(v: &Value) -> Result<Shape, EngineError> {
    let mut shape = Shape::new(ShapeKind::Rectangle, [400.0, 400.0], [1.0, 1.0, 1.0, 1.0]);
    if let Some(k) = v.get("kind").and_then(Value::as_str).and_then(ShapeKind::parse) {
        shape = Shape::new(k, if k == ShapeKind::Line { [400.0, 0.0] } else { [400.0, 400.0] }, [1.0; 4]);
    }
    if !v.is_object() {
        return Err(wrong("shape", "an object such as {\"kind\": \"rectangle\", \"size\": [400, 200]}"));
    }
    set_shape(&mut shape, v)?;
    Ok(shape)
}

/// `clip.setShape`.
pub(crate) fn clip_set_shape(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    s.edit("Shape", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let ClipContent::Shape { shape } = &mut clip.content else {
            return Err(EngineError::Params(format!("clip {id:?} is not a shape")));
        };
        set_shape(shape, v)?;
        Ok(json!({ "clip": id, "shape": serde_json::to_value(&*shape).expect("serialises") }))
    })
}

/// A counter as a command gives it.
fn counter_param(v: &Value, fps: reelforge_time::FrameRate, base: Option<Counter>) -> Result<Counter, EngineError> {
    let p = P::new(v, fps);
    let mut c = base.unwrap_or(Counter {
        from: 0.0,
        to: 100.0,
        over: None,
        ease: Ease::OUT,
        decimals: 0,
        group: false,
        clock: false,
    });
    if let Some(f) = p.opt_f64("from")? {
        c.from = f as f32;
    }
    if let Some(t) = p.opt_f64("to")? {
        c.to = t as f32;
    }
    if v.get("over").is_some_and(Value::is_null) {
        c.over = None;
    } else if let Some(o) = p.opt_time("over")? {
        c.over = Some(o);
    }
    if let Some(e) = p.opt("ease") {
        c.ease = match e {
            Value::String(n) => Ease::named(n),
            Value::Array(_) => serde_json::from_value::<[f32; 4]>(e.clone()).ok().map(Ease::bezier),
            _ => None,
        }
        .ok_or_else(|| wrong("ease", "a curve's name or [x1, y1, x2, y2]"))?;
    }
    if let Some(d) = p.opt_u32("decimals")? {
        c.decimals = d.min(255) as u8;
    }
    if let Some(g) = p.opt_bool("group")? {
        c.group = g;
    }
    if let Some(k) = p.opt_bool("clock")? {
        c.clock = k;
    }
    match c.problem() {
        Some(why) => Err(EngineError::Params(why.into())),
        None => Ok(c),
    }
}

/// `clip.setCounter`.
pub(crate) fn clip_set_counter(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let off = v.get("counter").is_some_and(Value::is_null);
    let fps = fps(s);
    s.edit("Counter", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let ClipContent::Text { style, .. } = &mut clip.content else {
            return Err(EngineError::Params(format!("clip {id:?} is not a title")));
        };
        style.counter =
            if off { None } else { Some(counter_param(v.get("counter").unwrap_or(v), fps, style.counter)?) };
        Ok(json!({ "clip": id, "counter": style.counter }))
    })
}

/// A ready-made piece.
pub struct Promo {
    /// Its id.
    pub id: &'static str,
    /// What it is, in English.
    pub en: &'static str,
    /// In Chinese.
    pub zh: &'static str,
    /// The words it shows unless others are given; `{n}` is its number.
    pub text: &'static str,
    /// The number it counts to, if it counts.
    pub value: Option<f32>,
}

/// The pieces `promo.add` makes.
pub const PROMOS: &[Promo] = &[
    Promo { id: "priceTag", en: "Price tag", zh: "价格标签", text: "¥{n}", value: Some(99.0) },
    Promo { id: "discountBadge", en: "Discount badge", zh: "折扣角标", text: "-50%", value: None },
    Promo { id: "banner", en: "Sale banner", zh: "促销横幅", text: "限时特惠", value: None },
    Promo { id: "countdown", en: "Countdown", zh: "倒计时", text: "{n}", value: Some(10.0) },
    Promo { id: "salesCounter", en: "Sales counter", zh: "销量滚动", text: "已售 {n}+", value: Some(10000.0) },
    Promo { id: "strikePrice", en: "Struck-out price", zh: "划线原价", text: "¥199", value: None },
    Promo { id: "newArrival", en: "New arrival stamp", zh: "新品印章", text: "NEW", value: None },
];

/// `promo.list`.
pub(crate) fn promo_list(_: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({
        "promos": PROMOS
            .iter()
            .map(|p| json!({"id": p.id, "name": p.en, "zh": p.zh, "text": p.text, "value": p.value}))
            .collect::<Vec<_>>()
    }))
}

/// One clip of a piece, before it has a place.
struct Part {
    track: TrackKind,
    content: ClipContent,
    /// Offset from the piece's centre, in canvas pixels, and turn.
    at: [f32; 2],
    rotation: f32,
    enter: Option<AnimKind>,
    repeat: Option<LoopKind>,
}

fn title(text: String, size: f32, color: [f32; 4], counter: Option<Counter>) -> ClipContent {
    ClipContent::Text {
        text,
        size,
        color,
        style: TitleStyle { weight: 900, wrap: 0.0, counter, ..TitleStyle::default() },
    }
}

fn shape_part(shape: Shape, enter: Option<AnimKind>) -> Part {
    Part {
        track: TrackKind::Video,
        content: ClipContent::Shape { shape },
        at: [0.0; 2],
        rotation: 0.0,
        enter,
        repeat: None,
    }
}

fn text_part(content: ClipContent, at: [f32; 2], enter: Option<AnimKind>) -> Part {
    Part { track: TrackKind::Text, content, at, rotation: 0.0, enter, repeat: None }
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// The clips of piece `id`, bottom first, for a clip `dur` long. `accent` is its main colour.
fn parts(id: &str, text: String, from: f32, value: Option<f32>, accent: [f32; 4], dur: Tick) -> Option<Vec<Part>> {
    let shadow = Some(ShapeShadow { color: [0.0, 0.0, 0.0, 0.35], offset: [0.0, 10.0], blur: 14.0 });
    let lighter =
        |c: [f32; 4], k: f32| [c[0] + (1.0 - c[0]) * k, c[1] + (1.0 - c[1]) * k, c[2] + (1.0 - c[2]) * k, c[3]];
    let counter = |to: f32, group: bool, clock: bool, ease: Ease, over: Option<Tick>| Counter {
        from,
        to,
        over,
        ease,
        decimals: if to.fract() != 0.0 { 2 } else { 0 },
        group,
        clock,
    };
    let quick = Some(Tick::from_seconds(1.2).min(dur));
    Some(match id {
        "priceTag" => vec![
            shape_part(
                Shape {
                    radius: 28.0,
                    fill: Some(Fill::Linear { from: lighter(accent, 0.25), to: accent, angle: 90.0 }),
                    shadow,
                    ..Shape::new(ShapeKind::Tag, [560.0, 210.0], accent)
                },
                Some(AnimKind::Pop),
            ),
            text_part(
                title(text, 120.0, WHITE, value.map(|v| counter(v, false, false, Ease::OUT, quick))),
                [40.0, 0.0],
                Some(AnimKind::ZoomIn),
            ),
        ],
        "discountBadge" => {
            let mut star = Shape {
                points: 16,
                inner: 0.82,
                stroke: Some(Stroke { color: WHITE, width: 8.0 }),
                shadow,
                ..Shape::new(ShapeKind::Star, [380.0, 380.0], accent)
            };
            star.fill = Some(Fill::Radial { from: lighter(accent, 0.3), to: accent });
            let mut badge = shape_part(star, Some(AnimKind::Pop));
            badge.repeat = Some(LoopKind::Heartbeat);
            badge.rotation = -12.0;
            let mut words = text_part(title(text, 104.0, WHITE, None), [0.0, 0.0], Some(AnimKind::Pop));
            words.repeat = Some(LoopKind::Heartbeat);
            words.rotation = -12.0;
            vec![badge, words]
        }
        "banner" => vec![
            shape_part(
                Shape {
                    radius: 90.0,
                    fill: Some(Fill::Linear { from: lighter(accent, 0.35), to: accent, angle: 0.0 }),
                    shadow,
                    ..Shape::new(ShapeKind::Rectangle, [980.0, 180.0], accent)
                },
                Some(AnimKind::SlideRight),
            ),
            text_part(title(text, 92.0, WHITE, None), [0.0, 0.0], Some(AnimKind::CharPop)),
        ],
        "countdown" => vec![
            shape_part(
                Shape {
                    radius: 36.0,
                    stroke: Some(Stroke { color: accent, width: 8.0 }),
                    shadow,
                    ..Shape::new(ShapeKind::Rectangle, [520.0, 200.0], [0.08, 0.08, 0.1, 0.88])
                },
                Some(AnimKind::Draw),
            ),
            text_part(
                title(text, 112.0, WHITE, Some(counter(0.0, false, true, Ease::Linear, None))),
                [0.0, 0.0],
                Some(AnimKind::Fade),
            ),
        ],
        "salesCounter" => vec![
            shape_part(
                Shape { radius: 80.0, shadow, ..Shape::new(ShapeKind::Rectangle, [700.0, 160.0], WHITE) },
                Some(AnimKind::ZoomIn),
            ),
            text_part(
                title(
                    text,
                    78.0,
                    accent,
                    value.map(|v| counter(v, true, false, Ease::OUT, Some(Tick::from_seconds(2.0).min(dur)))),
                ),
                [0.0, 0.0],
                Some(AnimKind::Fade),
            ),
        ],
        "strikePrice" => {
            let width = text.chars().count() as f32 * 72.0 * 0.62 + 30.0;
            vec![
                text_part(title(text, 72.0, [0.6, 0.6, 0.62, 1.0], None), [0.0, 0.0], Some(AnimKind::Fade)),
                Part {
                    track: TrackKind::Video,
                    content: ClipContent::Shape {
                        shape: Shape {
                            stroke: Some(Stroke { color: accent, width: 7.0 }),
                            ..Shape::new(ShapeKind::Line, [width, 0.0], accent)
                        },
                    },
                    at: [0.0, 4.0],
                    rotation: -8.0,
                    enter: Some(AnimKind::Draw),
                    repeat: None,
                },
            ]
        }
        "newArrival" => vec![
            shape_part(
                Shape {
                    stroke: Some(Stroke { color: accent, width: 12.0 }),
                    fill: Some(Fill::Solid { color: [accent[0], accent[1], accent[2], 0.18] }),
                    ..Shape::new(ShapeKind::Ellipse, [320.0, 320.0], accent)
                },
                Some(AnimKind::Draw),
            ),
            text_part(title(text, 104.0, accent, None), [0.0, 0.0], Some(AnimKind::CharPop)),
        ],
        _ => return None,
    })
    .map(|mut parts: Vec<Part>| {
        // a countdown counts down from its value to nothing, at the speed of a clock
        if id == "countdown"
            && let Some(Part { content: ClipContent::Text { style, .. }, .. }) = parts.get_mut(1)
            && let Some(c) = &mut style.counter
        {
            let start = value.unwrap_or(10.0);
            *c = Counter { from: start, to: (start - dur.seconds() as f32).max(0.0), ..*c };
        }
        parts
    })
}

/// The first track at or above `from` of `kind` that is free from `start` for `dur`, or a new one
/// put in at the top.
fn free_track(proj: &mut Project, kind: TrackKind, from: usize, start: Tick, dur: Tick) -> usize {
    let span = reelforge_time::TimeRange::new(start, dur);
    if let Some(i) = (from..proj.tracks.len()).find(|&i| {
        let t = &proj.tracks[i];
        t.kind == kind && !t.locked && !t.magnetic && t.clips.iter().all(|c| !c.range().overlaps(span))
    }) {
        return i;
    }
    let n = proj.tracks.iter().filter(|t| t.kind == kind).count() + 1;
    let id = proj.mint("t");
    let name = match kind {
        TrackKind::Video => format!("Video {n}"),
        TrackKind::Text => format!("Text {n}"),
        TrackKind::Audio => format!("Audio {n}"),
    };
    proj.tracks.push(Track {
        id,
        kind,
        name,
        hidden: false,
        muted: false,
        magnetic: false,
        locked: false,
        clips: vec![],
    });
    proj.tracks.len() - 1
}

/// A plain clip.
pub(crate) fn plain_clip(id: String, start: Tick, duration: Tick, content: ClipContent) -> Clip {
    Clip {
        id,
        start,
        duration,
        content,
        transform: ClipTransform::default(),
        opacity: 1.0,
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
    }
}

/// `promo.add`.
pub(crate) fn promo_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("kind")?.to_string();
    let Some(promo) = PROMOS.iter().find(|x| x.id == id) else {
        return Err(wrong("kind", &format!("one of {}", PROMOS.iter().map(|x| x.id).collect::<Vec<_>>().join(", "))));
    };
    let start = p.opt_time("start")?;
    let dur = p.opt_time("duration")?.unwrap_or(Tick::from_seconds(3.0));
    if dur <= Tick::ZERO {
        return Err(wrong("duration", "positive"));
    }
    let text = p.opt_str("text")?.unwrap_or(promo.text).to_string();
    let value = p.opt_f64("value")?.map(|v| v as f32).or(promo.value);
    if value.is_some_and(|v| v.abs() >= 1e12) {
        return Err(wrong("value", "a number"));
    }
    let from = p.opt_f64("from")?.unwrap_or(0.0) as f32;
    let accent = p.opt_color("color")?.unwrap_or([0.93, 0.16, 0.2, 1.0]);
    let x = p.opt_f64("x")?.unwrap_or(0.0) as f32;
    let y = p.opt_f64("y")?.unwrap_or(0.0) as f32;
    let scale = p.opt_f64("scale")?.unwrap_or(1.0) as f32;
    if !(scale > 0.0 && scale <= 20.0) {
        return Err(wrong("scale", "above 0 and at most 20"));
    }
    s.edit("Add Promo", |proj| {
        let start = start.unwrap_or(Tick::ZERO);
        let parts = parts(&id, text.clone(), from, value, accent, dur).expect("a known piece");
        // sizes are for a canvas 1080 wide; a larger or smaller one gets them in proportion
        let k = proj.canvas.width.min(proj.canvas.height) as f32 / 1080.0 * scale;
        let group = proj.mint("g");
        let (cw, ch) = (proj.canvas.width as f32, proj.canvas.height as f32);
        let mut below = 0;
        let mut made = vec![];
        for part in parts {
            let ti = free_track(proj, part.track, below, start, dur);
            below = ti + 1;
            let cid = proj.mint("c");
            let mut clip = plain_clip(cid.clone(), start, dur, part.content);
            clip.transform = ClipTransform {
                x: x + part.at[0] * k / cw,
                y: y + part.at[1] * k / ch,
                scale: k,
                rotation: part.rotation,
            };
            clip.anim.enter =
                part.enter.map(|kind| Animation { kind, duration: Tick::from_seconds(0.6).min(Tick(dur.0 / 2)) });
            clip.anim.repeat = part.repeat.map(|kind| Loop { kind, period: kind.period() });
            clip.group = Some(group.clone());
            proj.tracks[ti].clips.push(clip);
            proj.tracks[ti].clips.sort_by_key(|c| c.start);
            made.push(json!({"clip": cid, "track": proj.tracks[ti].id}));
        }
        Ok(json!({ "group": group, "clips": made, "start": start, "duration": dur }))
    })
}
