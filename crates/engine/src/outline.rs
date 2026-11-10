//! `project.outline`: the project in brief, with times in seconds, for an agent (or a person at a
//! terminal) to read before editing. `project.inspect` is the whole document, in ticks.

use reelforge_model::{Clip, ClipContent, Project};
use reelforge_time::Tick;
use serde_json::{Map, Value, json};

/// Seconds to the millisecond.
fn secs(t: Tick) -> f64 {
    (t.seconds() * 1000.0).round() / 1000.0
}

/// A stored `f32` as the number it was meant to be (0.3, not 0.30000001192092896).
fn num(x: f32) -> f64 {
    (x as f64 * 10_000.0).round() / 10_000.0
}

/// The names of an object's fields whose values are set (not null, false, empty or zero).
fn set_fields(v: Value) -> Vec<String> {
    let Value::Object(o) = v else { return vec![] };
    o.into_iter()
        .filter(|(_, v)| match v {
            Value::Null | Value::Bool(false) => false,
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            Value::Number(n) => n.as_f64() != Some(0.0),
            Value::String(s) => !s.is_empty(),
            Value::Bool(true) => true,
        })
        .map(|(k, _)| k)
        .collect()
}

fn clip(project: &Project, c: &Clip) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(c.id));
    o.insert("start".into(), json!(secs(c.start)));
    o.insert("end".into(), json!(secs(c.end())));
    match &c.content {
        ClipContent::Media { media, source_in, speed, reversed, curve, .. } => {
            let m = project.media.get(media);
            o.insert("media".into(), json!(media));
            if let Some(m) = m {
                o.insert("name".into(), json!(m.name));
            }
            if *source_in != Tick::ZERO {
                o.insert("sourceIn".into(), json!(secs(*source_in)));
            }
            if *speed != 1.0 {
                o.insert("speed".into(), json!(speed));
            }
            if *reversed {
                o.insert("reversed".into(), json!(true));
            }
            if !curve.is_empty() {
                o.insert("speedCurve".into(), json!(true));
            }
        }
        ClipContent::Text { text, .. } => {
            o.insert("text".into(), json!(text));
        }
        ClipContent::Solid { color } => {
            o.insert("color".into(), json!(color.map(num)));
        }
        ClipContent::Shape { shape } => {
            let v = serde_json::to_value(shape).unwrap_or_default();
            o.insert("shape".into(), v.get("kind").cloned().unwrap_or(json!(true)));
        }
        ClipContent::Adjustment => {
            o.insert("adjustment".into(), json!(true));
        }
        ClipContent::Compound { timeline, .. } => {
            o.insert("timeline".into(), json!(timeline));
        }
    }
    // where it is: what differs from a clip left as it lands
    let t = &c.transform;
    if t.x != 0.0 || t.y != 0.0 || t.scale != 1.0 || t.rotation != 0.0 {
        o.insert(
            "place".into(),
            json!({"x": num(t.x), "y": num(t.y), "scale": num(t.scale), "rotation": num(t.rotation)}),
        );
    }
    let mut space = Map::new();
    for (name, v, rest) in [
        ("scaleX", t.scale_x, 1.0),
        ("scaleY", t.scale_y, 1.0),
        ("rotationX", t.rotation_x, 0.0),
        ("rotationY", t.rotation_y, 0.0),
        ("perspective", t.distance, reelforge_model::ClipTransform::DISTANCE),
        ("motionBlur", t.motion_blur, 0.0),
    ] {
        if v != rest {
            space.insert(name.into(), json!(num(v)));
        }
    }
    if t.anchor != [0.5, 0.5] {
        space.insert("anchor".into(), json!(t.anchor.map(num)));
    }
    if !space.is_empty() {
        o.insert("space".into(), Value::Object(space));
    }
    if let Some(parent) = &c.parent {
        o.insert("parent".into(), json!(parent));
    }
    if c.path.is_some() {
        o.insert("path".into(), json!(true));
    }
    if c.steady.is_some() {
        o.insert("stabilized".into(), json!(true));
    }
    if c.opacity != 1.0 {
        o.insert("opacity".into(), json!(num(c.opacity)));
    }
    if c.volume_db != 0.0 {
        o.insert("volumeDb".into(), json!(num(c.volume_db)));
    }
    let keyed = set_fields(serde_json::to_value(&c.keys).unwrap_or_default());
    if !keyed.is_empty() {
        o.insert("keyframed".into(), json!(keyed));
    }
    let anim = serde_json::to_value(c.anim).unwrap_or_default();
    if let Value::Object(slots) = &anim {
        let a: Map<String, Value> =
            slots.iter().filter_map(|(slot, v)| Some((slot.clone(), v.get("kind")?.clone()))).collect();
        if !a.is_empty() {
            o.insert("anim".into(), Value::Object(a));
        }
    }
    if let Some(tr) = serde_json::to_value(c.transition).ok().filter(|v| !v.is_null()) {
        o.insert("transition".into(), tr.get("kind").cloned().unwrap_or(tr));
    }
    if !c.fx.is_empty() {
        let kinds: Vec<Value> =
            serde_json::to_value(&c.fx).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        o.insert("fx".into(), json!(kinds.iter().filter_map(|f| f.get("kind").cloned()).collect::<Vec<_>>()));
    }
    // what else is set on it, by name: colour, crop, mask, chroma key, voice…
    let mut more = vec![];
    for (name, v) in [
        ("color", serde_json::to_value(&c.color)),
        ("look", serde_json::to_value(&c.look)),
        ("voice", serde_json::to_value(&c.voice)),
    ] {
        let fields = set_fields(v.unwrap_or_default());
        if !fields.is_empty() {
            more.push(json!({name: fields}));
        }
    }
    if !more.is_empty() {
        o.insert("set".into(), json!(more));
    }
    if let Some(g) = &c.group {
        o.insert("group".into(), json!(g));
    }
    if let Some(n) = c.slot {
        o.insert("slot".into(), json!(n));
    }
    Value::Object(o)
}

/// The outline of the timeline being edited, its media, markers and cover.
pub fn outline(project: &Project) -> Value {
    let tracks: Vec<Value> = project
        .tracks
        .iter()
        .map(|t| {
            let mut o = json!({
                "id": t.id,
                "kind": t.kind,
                "name": t.name,
                "clips": t.clips.iter().map(|c| clip(project, c)).collect::<Vec<_>>(),
            });
            for (flag, on) in [("hidden", t.hidden), ("muted", t.muted), ("magnetic", t.magnetic), ("locked", t.locked)]
            {
                if on {
                    o[flag] = json!(true);
                }
            }
            o
        })
        .collect();
    let media: Vec<Value> = project
        .media
        .values()
        .map(|m| {
            let mut o = json!({"id": m.id, "name": m.name, "kind": m.kind});
            if let Some(d) = m.duration {
                o["seconds"] = json!(secs(d));
            }
            if let Some(s) = m.size {
                o["size"] = json!(s);
            }
            if let Some(f) = &m.folder {
                o["folder"] = json!(f);
            }
            o
        })
        .collect();
    let c = &project.canvas;
    json!({
        "canvas": {"width": c.width, "height": c.height, "fps": c.fps.num as f64 / c.fps.den as f64},
        "seconds": secs(project.duration()),
        "timeline": project.timeline,
        "tracks": tracks,
        "media": media,
        "markers": project.markers.iter().map(|m| json!({"id": m.id, "at": secs(m.at), "note": m.note})).collect::<Vec<_>>(),
        "cover": project.cover.map(secs),
        "note": "Times are in seconds; ids are what the other commands take. project.inspect has every detail, in ticks.",
    })
}
