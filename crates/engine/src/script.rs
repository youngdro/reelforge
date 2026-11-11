//! Motion scripts (`script.add` and the rest): JavaScript that makes nodes and tweens them, run
//! once by `reelforge-script` and turned here into ordinary clips with keyframes, grouped, each on
//! a track above the one before. The script and its parameters are kept by the group's id
//! (`Project::scripts`), so the group is built again when either changes; nothing of a script
//! reaches the compiler or the renderer.

use std::collections::BTreeMap;

use reelforge_model::{ClipContent, Ease, Key, Project, Prop, Script, TrackKind, anim};
use reelforge_script::{Built, Input, KeyOut};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{fps, not_found, one_step};
use crate::params::P;
use crate::promo::free_track;
use crate::{CmdResult, EngineError, Session};

fn refused(e: &reelforge_script::ScriptError) -> EngineError {
    EngineError::Refused(format!("script: {e}"))
}

/// The values a command gives a script: an object, or nothing.
fn values(v: &Value) -> Result<BTreeMap<String, Value>, EngineError> {
    match v.get("params") {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(o)) => Ok(o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        Some(_) => Err(EngineError::Params("`params` must be an object: {name: value}".into())),
    }
}

fn run(s: &Session, script: &Script) -> Result<Built, EngineError> {
    let canvas = &&s.project.canvas;
    let params = Value::Object(script.params.clone().into_iter().collect());
    reelforge_script::run(
        &script.source,
        &Input {
            params: &params,
            canvas: [canvas.width, canvas.height],
            fps: canvas.fps.fps(),
            seed: script.seed,
            duration: script.duration.map(Tick::seconds),
        },
    )
    .map_err(|e| refused(&e))
}

fn ease(v: &Value) -> Ease {
    match v {
        Value::String(s) if s == "hold" => Ease::Hold,
        Value::Array(_) => serde_json::from_value::<[f32; 4]>(v.clone()).map_or(Ease::Linear, Ease::bezier),
        _ => Ease::Linear,
    }
}

/// The keys of one property as a clip keeps them: on frames, within the clip, within what the
/// property may be (a curve that overshoots is held at the edge, as playing one back is).
fn keys(prop: Prop, list: &[KeyOut], fps: reelforge_time::FrameRate, length: Tick) -> Vec<Key> {
    let mut out = Vec::new();
    for k in list {
        let t = fps.tick_of((k.t * fps.fps()).round() as i64);
        if t < Tick::ZERO || t > length || !k.v.is_finite() {
            continue;
        }
        let mut v = k.v as f32;
        if let Some((lo, hi)) = prop.range() {
            v = v.clamp(lo, hi);
        }
        if prop == Prop::Scale {
            v = v.max(0.0);
        }
        anim::insert(&mut out, Key { t, v, ease: ease(&k.ease) });
    }
    out
}

/// Builds `script` into clips of group `group` from `start`, lowest on a free track at or above
/// track `from`. Runs inside a step of its own making; the caller makes it one undo step.
fn build(
    s: &mut Session,
    script: Script,
    group: Option<String>,
    start: Tick,
    from: usize,
) -> Result<Value, EngineError> {
    let built = run(s, &script)?;
    let fps = fps(s);
    let length = fps.tick_of((built.duration * fps.fps()).round() as i64).max(fps.frame_duration());
    let group = match group {
        Some(g) => g,
        None => s.edit("Script", |proj| Ok(proj.mint("g")))?,
    };
    let mut below = from;
    let mut made = Vec::new();
    for node in &built.nodes {
        let kind = if node.kind == "text" { TrackKind::Text } else { TrackKind::Video };
        let track = s.edit("Script", |proj| {
            let ti = free_track(proj, kind, below, start, length);
            below = ti + 1;
            Ok(proj.tracks[ti].id.clone())
        })?;
        let mut add = node.add.clone();
        add.insert("track".into(), json!(track));
        add.insert("start".into(), json!(start.0));
        add.insert("duration".into(), json!(length.0));
        let named = |e: EngineError| EngineError::Refused(format!("script: {}: {e}", node.name));
        let clip = s.execute("clip.add", &Value::Object(add)).map_err(named)?;
        let id = clip["clip"].as_str().unwrap_or_default().to_string();
        if let Some(counter) = &node.counter {
            s.execute("clip.setCounter", &json!({"clip": id, "counter": counter})).map_err(named)?;
        }
        for step in &node.after {
            if !["clip.setMask", "clip.setAnimation"].contains(&step.command.as_str()) {
                return Err(named(EngineError::Refused(format!("a script does not run {}", step.command))));
            }
            let mut params = step.params.clone();
            params.insert("clip".into(), json!(id));
            s.execute(&step.command, &Value::Object(params)).map_err(named)?;
        }
        made.push((id, track, node));
    }
    let declared: BTreeMap<String, Value> = built.params.clone().into_iter().collect();
    let reply: Vec<Value> = made
        .iter()
        .map(|(id, track, node)| json!({"clip": id, "track": track, "node": node.name, "kind": node.kind}))
        .collect();
    s.edit("Script", |proj| {
        for (id, _, node) in &made {
            let clip = proj.clip_mut(id).ok_or_else(|| not_found("clip", id))?;
            clip.group = Some(group.clone());
            for (name, list) in &node.keys {
                let prop = Prop::parse(name)
                    .ok_or_else(|| EngineError::Refused(format!("script: {}: no property `{name}`", node.name)))?;
                let list = keys(prop, list, fps, length);
                if !list.is_empty() {
                    clip.keys.insert(prop, list);
                }
            }
            // a picture is as long as the piece, however long its file
            if let ClipContent::Media { .. } = clip.content {
                clip.duration = length;
            }
        }
        proj.scripts.insert(group.clone(), Script { declared: declared.clone(), ..script.clone() });
        Ok(())
    })?;
    // what one node has of another, and what needs the clips as they now are
    for (id, _, node) in &made {
        let named = |e: EngineError| EngineError::Refused(format!("script: {}: {e}", node.name));
        if let Some(path) = &node.path {
            let at = |seconds: f64| fps.tick_of((seconds * fps.fps()).round() as i64);
            let (from, took) = (at(path.start).max(Tick::ZERO), at(path.duration).max(fps.frame_duration()));
            if from < length {
                s.execute(
                    "clip.setPath",
                    &json!({
                        "clip": id, "points": path.points, "closed": path.closed, "orient": path.orient,
                        "start": from.0, "duration": took.0, "ease": path.ease,
                    }),
                )
                .map_err(named)?;
            }
        }
        if let Some(parent) = node.parent.and_then(|i| made.get(i)) {
            s.execute("clip.setParent", &json!({"clip": id, "parent": parent.0, "keep": false})).map_err(named)?;
        }
    }
    if made.iter().any(|(_, _, node)| node.origin.is_some()) {
        s.edit("Script", |proj| {
            for (id, _, node) in &made {
                if let Some([x, y]) = node.origin {
                    crate::space::set_origin(proj, id, [x as f32, y as f32]);
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({
        "group": group,
        "clips": reply,
        "start": start,
        "duration": length,
        "seconds": length.seconds(),
        "params": built.params,
        "values": built.values,
        "log": built.log,
    }))
}

/// The clips of script `group` on the timeline being edited: their track indices and ids.
fn clips_of(proj: &Project, group: &str) -> Vec<(usize, String, Tick)> {
    let mut out = Vec::new();
    for (ti, t) in proj.tracks.iter().enumerate() {
        for c in &t.clips {
            if c.group.as_deref() == Some(group) {
                out.push((ti, c.id.clone(), c.start));
            }
        }
    }
    out
}

fn script_of<'a>(
    proj: &'a Project,
    v: &Value,
    fps: reelforge_time::FrameRate,
) -> Result<(String, &'a Script), EngineError> {
    let p = P::new(v, fps);
    let group = p.str("group")?.to_string();
    let script = proj.scripts.get(&group).ok_or_else(|| {
        let have: Vec<&str> = proj.scripts.keys().map(String::as_str).collect();
        EngineError::Params(format!(
            "no script {group:?}; {}",
            if have.is_empty() { "the project has none".to_string() } else { format!("there are {}", have.join(", ")) }
        ))
    })?;
    Ok((group, script))
}

fn seed(p: &P) -> Result<Option<u32>, EngineError> {
    p.opt_u32("seed")
}

fn source(p: &P) -> Result<String, EngineError> {
    let src = p.str("source")?;
    if src.trim().is_empty() {
        return Err(EngineError::Params("`source` is empty".into()));
    }
    Ok(src.to_string())
}

fn length(p: &P) -> Result<Option<Tick>, EngineError> {
    let d = p.opt_time("duration")?;
    if d.is_some_and(|d| d <= Tick::ZERO) {
        return Err(EngineError::Params("`duration` must be positive".into()));
    }
    Ok(d)
}

/// `script.add`.
pub(crate) fn script_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let script = Script {
        name: p.opt_str("name")?.unwrap_or_default().to_string(),
        source: source(&p)?,
        params: values(v)?,
        declared: BTreeMap::new(),
        seed: seed(&p)?.unwrap_or(1),
        duration: length(&p)?,
    };
    let start = p.opt_time("start")?.unwrap_or(Tick::ZERO);
    if start < Tick::ZERO {
        return Err(EngineError::Params("`start` must not be negative".into()));
    }
    one_step(s, "Add Script", |s| build(s, script, None, start, 0))
}

/// Builds script `group` again from `script`, where it is.
fn rebuild(s: &mut Session, group: String, script: Script, label: &str) -> CmdResult {
    let old = clips_of(&s.project, &group);
    if old.is_empty() {
        return Err(EngineError::Refused(format!(
            "script {group:?} has no clips on this timeline; switch to the timeline it is on"
        )));
    }
    let start = old.iter().map(|c| c.2).min().unwrap_or(Tick::ZERO);
    let from = old.iter().map(|c| c.0).min().unwrap_or(0);
    one_step(s, label, |s| {
        s.edit("Script", |proj| {
            for t in &mut proj.tracks {
                t.clips.retain(|c| c.group.as_deref() != Some(group.as_str()));
            }
            Ok(())
        })?;
        build(s, script, Some(group.clone()), start, from)
    })
}

/// `script.setParams`.
pub(crate) fn script_set_params(s: &mut Session, v: &Value) -> CmdResult {
    let (group, script) = script_of(&s.project, v, fps(s))?;
    let mut script = script.clone();
    let p = P::new(v, fps(s));
    if p.opt_bool("reset")?.unwrap_or(false) {
        script.params.clear();
    }
    for (k, value) in values(v)? {
        if value.is_null() {
            script.params.remove(&k);
        } else {
            script.params.insert(k, value);
        }
    }
    if let Some(n) = seed(&p)? {
        script.seed = n;
    }
    if v.get("duration").is_some_and(Value::is_null) {
        script.duration = None;
    } else if let Some(d) = length(&p)? {
        script.duration = Some(d);
    }
    rebuild(s, group, script, "Script Parameters")
}

/// `script.setSource`.
pub(crate) fn script_set_source(s: &mut Session, v: &Value) -> CmdResult {
    let (group, script) = script_of(&s.project, v, fps(s))?;
    let mut script = script.clone();
    let p = P::new(v, fps(s));
    script.source = source(&p)?;
    if let Some(n) = p.opt_str("name")? {
        script.name = n.to_string();
    }
    if v.get("params").is_some() {
        script.params = values(v)?;
    } else {
        // a value for a parameter the new source no longer has would refuse it
        let declared = reelforge_script::declared(&script.source).map_err(|e| refused(&e))?;
        script.params.retain(|k, _| declared.contains_key(k));
    }
    rebuild(s, group, script, "Script Source")
}

/// `script.check`: what a script would build, without building it.
pub(crate) fn script_check(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let script = Script {
        name: String::new(),
        source: source(&p)?,
        params: values(v)?,
        declared: BTreeMap::new(),
        seed: seed(&p)?.unwrap_or(1),
        duration: length(&p)?,
    };
    // built for real on the side, so that what the clips themselves refuse is found too
    let saved = (s.project.clone(), s.undo.len(), s.redo.clone(), s.revision);
    let out = one_step(s, "Check Script", |s| build(s, script, None, Tick::ZERO, 0));
    let summary = out.map(|built| {
        let nodes: Vec<Value> = built["clips"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                let id = c["clip"].as_str().unwrap_or_default();
                let keyed: BTreeMap<&str, usize> = s
                    .project()
                    .clip(id)
                    .map(|(_, clip)| clip.keys.iter().map(|(p, k)| (p.name(), k.len())).collect())
                    .unwrap_or_default();
                json!({"node": c["node"], "kind": c["kind"], "keys": keyed})
            })
            .collect();
        json!({
            "ok": true,
            "nodes": nodes,
            "seconds": built["seconds"],
            "params": built["params"],
            "values": built["values"],
            "log": built["log"],
        })
    });
    s.project = saved.0;
    s.undo.truncate(saved.1);
    s.redo = saved.2;
    s.revision = saved.3;
    summary
}

fn described(proj: &Project, group: &str, script: &Script, whole: bool) -> Value {
    let clips = clips_of(proj, group);
    let mut out = json!({
        "group": group,
        "name": script.name,
        "params": script.params,
        "declared": script.declared,
        "seed": script.seed,
        "clips": clips.iter().map(|c| c.1.clone()).collect::<Vec<_>>(),
        "start": clips.iter().map(|c| c.2).min(),
    });
    if whole {
        out["source"] = json!(script.source);
        out["duration"] = json!(script.duration);
    }
    out
}

/// `script.get`.
pub(crate) fn script_get(s: &mut Session, v: &Value) -> CmdResult {
    let (group, script) = script_of(&s.project, v, fps(s))?;
    Ok(described(&s.project, &group, script, true))
}

/// `script.list`.
pub(crate) fn script_list(s: &mut Session, _: &Value) -> CmdResult {
    let proj = &s.project;
    let all: Vec<Value> = proj.scripts.iter().map(|(g, sc)| described(proj, g, sc, false)).collect();
    Ok(json!({ "scripts": all }))
}

/// `script.detach`.
pub(crate) fn script_detach(s: &mut Session, v: &Value) -> CmdResult {
    let (group, _) = script_of(&s.project, v, fps(s))?;
    s.edit("Detach Script", |proj| {
        proj.scripts.remove(&group);
        let clips: Vec<String> = clips_of(proj, &group).into_iter().map(|c| c.1).collect();
        crate::compound::tidy_groups(proj);
        Ok(json!({ "group": group, "clips": clips }))
    })
}
