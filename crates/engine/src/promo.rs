//! Shapes, counters, the brand, and the ready-made pieces built from them (`promo.add`; what
//! there is and how each is built is in [`crate::pieces`]).
//!
//! A piece is ordinary clips, shapes and titles, grouped so they are picked and moved together,
//! each on a track above the one before so they lie in order. Everything about them can be
//! changed afterwards with the usual commands. Asked to, the piece is folded into a compound clip:
//! one clip on the timeline, its parts inside.

use reelforge_model::{
    Animation, Clip, ClipContent, ClipTransform, Counter, Ease, Fill, Key, Loop, Made, MediaKind, Project, Prop, Shape,
    ShapeKind, ShapeShadow, Stroke, Timeline, Track, TrackKind,
};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{fps, not_found};
use crate::params::P;
use crate::pieces::{self, Ask, PROMOS, Promo, Side};
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

/// `promo.list`.
pub(crate) fn promo_list(_: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({
        "promos": PROMOS
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "category": p.category.name(),
                    "name": p.en,
                    "zh": p.zh,
                    "text": p.text,
                    "lines": p.lines,
                    "value": p.value,
                    "values": p.values,
                    "from": p.from,
                    "x": p.place[0],
                    "y": p.place[1],
                    "duration": Tick::from_seconds(p.millis as f64 / 1000.0),
                    "clips": p.clips(),
                    "about": p.about,
                })
            })
            .collect::<Vec<_>>()
    }))
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

/// The numbers `name` lists, at most `most` of them.
fn numbers(v: &Value, name: &str, most: usize) -> Result<Option<Vec<f32>>, EngineError> {
    let Some(list) = v.get(name) else { return Ok(None) };
    let what = format!("a list of at most {most} numbers");
    let list = list.as_array().filter(|a| a.len() <= most).ok_or_else(|| wrong(name, &what))?;
    let read = |x: &Value| x.as_f64().filter(|f| f.abs() < 1e12).map(|f| f as f32).ok_or_else(|| wrong(name, &what));
    list.iter().map(read).collect::<Result<Vec<_>, _>>().map(Some)
}

fn known(id: &str) -> Result<&'static Promo, EngineError> {
    PROMOS
        .iter()
        .find(|x| x.id == id)
        .ok_or_else(|| wrong("kind", &format!("one of {}", PROMOS.iter().map(|x| x.id).collect::<Vec<_>>().join(", "))))
}

/// What a command says of a piece, over what it was (`promo.set`) or over what the piece comes
/// with and the brand gives (`promo.add`).
fn told(v: &Value, fps: reelforge_time::FrameRate, was: Made) -> Result<Made, EngineError> {
    let p = P::new(v, fps);
    let promo = known(&was.kind)?;
    let mut made = was;
    // what is told here is no longer written from the project's copy
    if let Some(text) = p.opt_str("text")? {
        made.text = text.to_string();
        made.forms.text = None;
    }
    if v.get("lines").is_some() {
        made.lines = p.strings("lines")?;
        made.forms.lines = None;
    }
    if made.lines.len() > 6 || made.lines.iter().any(|l| l.chars().count() > 200) {
        return Err(wrong("lines", "at most 6 lines of at most 200 characters"));
    }
    // a piece that is all lines needs one
    if made.lines.is_empty() && promo.text.is_empty() && !promo.lines.is_empty() {
        return Err(wrong("lines", "at least one line"));
    }
    if let Some(value) = p.opt_f64("value")? {
        made.value = Some(value as f32);
        made.forms.value = None;
    }
    if made.value.is_some_and(|v| v.abs() >= 1e12) {
        return Err(wrong("value", "a number"));
    }
    if let Some(values) = numbers(v, "values", 4)? {
        made.values = values;
        made.forms.values = None;
    }
    if let Some(from) = p.opt_f64("from")? {
        made.from = from as f32;
    }
    made.color = p.opt_color("color")?.unwrap_or(made.color);
    made.color2 = p.opt_color("color2")?.unwrap_or(made.color2);
    if let Some(name) = p.opt_str("side")? {
        Side::parse(name).ok_or_else(|| wrong("side", &format!("one of {}", Side::NAMES.join(", "))))?;
        made.side = Some(name.to_string());
    }
    made.x = p.opt_f64("x")?.map_or(made.x, |x| x as f32);
    made.y = p.opt_f64("y")?.map_or(made.y, |y| y as f32);
    made.scale = p.opt_f64("scale")?.map_or(made.scale, |s| s as f32);
    if !(made.scale > 0.0 && made.scale <= 20.0) {
        return Err(wrong("scale", "above 0 and at most 20"));
    }
    Ok(made)
}

/// The clips of a piece as `made` has it, bottom first, each with the kind of track it goes on
/// and when it starts after the piece does. Their ids are still to be given.
fn clips_of(proj: &Project, made: &Made) -> Vec<(TrackKind, Tick, Clip)> {
    let (x, y, scale, dur) = (made.x, made.y, made.scale, made.duration);
    let (cw, ch) = (proj.canvas.width as f32, proj.canvas.height as f32);
    // sizes are for a canvas whose shorter side is 1080; a larger or smaller one gets them in
    // proportion
    let fit = cw.min(ch) / 1080.0;
    let k = fit * scale;
    // media the piece was made with that has since gone is left out
    let held = |id: &Option<String>, kind: MediaKind| {
        id.clone().filter(|id| proj.media.get(id).is_some_and(|m| m.kind == kind))
    };
    let font = held(&made.font, MediaKind::Font);
    let ask = Ask {
        text: made.text.clone(),
        lines: made.lines.clone(),
        from: made.from,
        value: made.value,
        values: made.values.clone(),
        accent: made.color,
        second: made.color2,
        side: made.side.as_deref().and_then(Side::parse),
        dur,
        canvas: [cw / fit, ch / fit],
        place: [x, y],
        scale,
        logo: held(&made.logo, MediaKind::Image),
    };
    let parts = pieces::parts(&made.kind, &ask).expect("a known piece");
    let mut out = vec![];
    for mut part in parts {
        // a part that comes in late still shows for a quarter of the piece
        let wait = part.delay.min(Tick(dur.0 / 4 * 3));
        let long = dur - wait;
        let mut size = k;
        match &mut part.content {
            ClipContent::Text { style, size: letters, .. } => {
                if style.font.is_none() {
                    style.font = font.clone();
                }
                // a title that wraps does so at a share of the frame, whatever its clip's
                // scale: the frame's size goes into its letters instead
                if let Some(width) = part.boxed.or(part.wrapped) {
                    pieces::boxed(style, width, ask.canvas[0], part.boxed.is_some());
                    *letters *= fit;
                    size = scale;
                }
            }
            ClipContent::Media { .. } => size = part.fit.unwrap_or(1.0) * scale,
            _ => {}
        }
        let mut clip = plain_clip(String::new(), wait, long, part.content);
        clip.transform = ClipTransform {
            x: x + part.at[0] * k / cw,
            y: y + part.at[1] * k / ch,
            scale: size,
            rotation: part.rotation,
        };
        let enter = part.enter_for.unwrap_or(Tick::from_seconds(0.6)).min(Tick(long.0 / 2));
        clip.anim.enter = part.enter.map(|kind| Animation { kind, duration: enter });
        clip.anim.repeat = part.repeat.map(|kind| Loop { kind, period: kind.period() });
        for (prop, keys) in part.keys {
            let mut list: Vec<Key> = Vec::new();
            for (share, value, ease) in keys {
                let t = Tick(long.0 * share / 1000);
                let v = match prop {
                    Prop::X => clip.transform.x + value * k / cw,
                    Prop::Y => clip.transform.y + value * k / ch,
                    _ => value,
                };
                match list.last_mut() {
                    Some(last) if last.t == t => *last = Key { t, v, ease },
                    _ => list.push(Key { t, v, ease }),
                }
            }
            clip.keys.insert(prop, list);
        }
        out.push((part.track, wait, clip));
    }
    out
}

/// The clips of a piece on tracks of their own, a track a clip, bottom first: the inside of a
/// compound clip. Returns the clips' ids too.
fn inside(proj: &mut Project, made: &Made) -> (Vec<Track>, Vec<String>) {
    let group = proj.mint("g");
    let mut tracks: Vec<Track> = vec![];
    let mut ids = vec![];
    for (kind, _, mut clip) in clips_of(proj, made) {
        clip.id = proj.mint("c");
        clip.group = Some(group.clone());
        ids.push(clip.id.clone());
        let n = tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let word = match kind {
            TrackKind::Video => "Video",
            TrackKind::Text => "Text",
            TrackKind::Audio => "Audio",
        };
        tracks.push(Track {
            id: proj.mint("t"),
            kind,
            name: format!("{word} {n}"),
            hidden: false,
            muted: false,
            magnetic: false,
            locked: false,
            clips: vec![clip],
        });
    }
    (tracks, ids)
}

/// `promo.add`.
pub(crate) fn promo_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let promo = known(p.str("kind")?)?;
    let start = p.opt_time("start")?.unwrap_or(Tick::ZERO);
    let dur = p.opt_time("duration")?.unwrap_or(Tick::from_seconds(promo.millis as f64 / 1000.0));
    if dur <= Tick::ZERO {
        return Err(wrong("duration", "positive"));
    }
    let folded = p.opt_bool("compound")? == Some(true);
    // what the piece comes with and the brand gives, under what the command says
    let brand = s.project.brand.clone();
    let comes = Made {
        kind: promo.id.to_string(),
        text: promo.text.to_string(),
        lines: promo.lines.iter().map(|l| l.to_string()).collect(),
        value: promo.value,
        values: promo.values.to_vec(),
        from: promo.from,
        color: brand.primary.unwrap_or(pieces::RED),
        color2: brand.secondary.unwrap_or(pieces::YELLOW),
        side: None,
        x: promo.place[0],
        y: promo.place[1],
        scale: 1.0,
        duration: dur,
        font: brand.font,
        logo: brand.logo,
        forms: Default::default(),
    };
    let made = told(v, fps(s), comes)?;
    s.edit("Add Promo", |proj| {
        if folded {
            // one compound clip, the piece inside it on a timeline of its own
            proj.name_first_timeline("Timeline 1");
            let (tracks, parts) = inside(proj, &made);
            let timeline = proj.mint("s");
            proj.timelines.push(Timeline {
                id: timeline.clone(),
                name: promo.en.to_string(),
                tracks,
                markers: vec![],
                cover: None,
                compound: true,
                made: Some(made.clone()),
            });
            let ti = free_track(proj, TrackKind::Video, 0, start, dur);
            let id = proj.mint("c");
            let content = ClipContent::Compound { timeline: timeline.clone(), source_in: Tick::ZERO };
            proj.tracks[ti].clips.push(plain_clip(id.clone(), start, dur, content));
            proj.tracks[ti].clips.sort_by_key(|c| c.start);
            return Ok(json!({
                "clip": id,
                "timeline": timeline,
                "track": proj.tracks[ti].id,
                "start": start,
                "duration": dur,
                "parts": parts,
            }));
        }
        let group = proj.mint("g");
        let mut below = 0;
        let mut clips = vec![];
        for (kind, wait, mut clip) in clips_of(proj, &made) {
            clip.start = start + wait;
            let ti = free_track(proj, kind, below, clip.start, clip.duration);
            below = ti + 1;
            clip.id = proj.mint("c");
            clip.group = Some(group.clone());
            clips.push(json!({"clip": clip.id, "track": proj.tracks[ti].id}));
            proj.tracks[ti].clips.push(clip);
            proj.tracks[ti].clips.sort_by_key(|c| c.start);
        }
        Ok(json!({ "group": group, "clips": clips, "start": start, "duration": dur }))
    })
}

/// The timeline `clip` plays, if it is a ready-made piece folded into a compound clip.
pub fn made_of<'a>(proj: &'a Project, clip: &str) -> Option<(&'a str, &'a Made)> {
    let (timeline, _) = proj.clip(clip)?.1.compound()?;
    let made = proj.timelines.iter().find(|t| t.id == timeline)?.made.as_ref()?;
    Some((timeline, made))
}

/// `promo.get`.
pub(crate) fn promo_get(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?;
    s.project.clip(id).ok_or_else(|| not_found("clip", id))?;
    match made_of(&s.project, id) {
        Some((timeline, made)) => Ok(json!({ "clip": id, "timeline": timeline, "piece": made })),
        None => Ok(json!({ "clip": id, "piece": null })),
    }
}

/// `promo.set`.
pub(crate) fn promo_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let fps = fps(s);
    s.edit("Rewrite Promo", |proj| {
        proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let Some((timeline, was)) = made_of(proj, &id).map(|(t, m)| (t.to_string(), m.clone())) else {
            return Err(EngineError::Refused(format!(
                "clip {id:?} is not a ready-made piece folded into one clip (promo.add with compound: true makes one); change its clips with the usual commands"
            )));
        };
        let made = told(v, fps, was)?;
        remake(proj, &timeline, made.clone());
        Ok(json!({ "clip": id, "timeline": timeline, "piece": made }))
    })
}

/// Makes the inside of the piece on `timeline` again, as `made` has it. What was changed inside
/// it by hand is lost; the compound clips that play it stay as they are.
pub(crate) fn remake(proj: &mut Project, timeline: &str, made: Made) {
    let (tracks, _) = inside(proj, &made);
    if let Some(slot) = proj.tracks_of_mut(timeline) {
        *slot = tracks;
    }
    if let Some(t) = proj.timelines.iter_mut().find(|t| t.id == timeline) {
        t.made = Some(made);
    }
}

/// `brand.get`.
pub(crate) fn brand_get(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({ "brand": s.project.brand }))
}

/// `brand.set`.
pub(crate) fn brand_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let (primary, secondary) = (p.opt_color_or_null("primary")?, p.opt_color_or_null("secondary")?);
    let media = |name: &str| match v.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(id)) => Ok(Some(Some(id.clone()))),
        Some(_) => Err(wrong(name, "a media id or null")),
    };
    let (font, logo) = (media("font")?, media("logo")?);
    s.edit("Brand", |proj| {
        let wanted = |id: &Option<Option<String>>, kind: MediaKind, what: &str| match id {
            Some(Some(id)) => match proj.media.get(id) {
                None => Err(not_found("media", id)),
                Some(m) if m.kind != kind => Err(EngineError::Params(format!("media {id:?} is not {what}"))),
                Some(_) => Ok(()),
            },
            _ => Ok(()),
        };
        wanted(&font, MediaKind::Font, "a font")?;
        wanted(&logo, MediaKind::Image, "a picture")?;
        let b = &mut proj.brand;
        b.primary = primary.unwrap_or(b.primary);
        b.secondary = secondary.unwrap_or(b.secondary);
        b.font = font.unwrap_or(b.font.take());
        b.logo = logo.unwrap_or(b.logo.take());
        Ok(json!({ "brand": proj.brand }))
    })
}
