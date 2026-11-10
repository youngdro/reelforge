//! Tools that listen before they edit: level a clip's loudness, mark a song's beats, lower music
//! while a voice speaks, measure a clip's background noise.
//!
//! A session has no sound to listen to, only a project; whoever holds the media does ([`plan`]
//! takes it). Each tool reads the project, listens to the clips it is about through the export's
//! own mix, and answers with ordinary commands to run as one `edit.batch`, so what it changes is
//! undone in one step and reaches every client like any other edit.

use reelforge_compile::{CompileOptions, compile};
use reelforge_media::Footage;
use reelforge_media::analysis::{self, RATE};
use reelforge_media::export::mix;
use reelforge_model::{Clip, Denoise, Project, Prop};
use reelforge_renderspec::Animated;
use reelforge_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

use crate::input_schema;

/// One listening tool.
pub struct Tool {
    /// Dotted id, `audio.beats`.
    pub name: &'static str,
    /// Short human title.
    pub title: &'static str,
    /// What it does.
    pub doc: &'static str,
}

/// Every listening tool.
pub const TOOLS: [Tool; 4] = [
    Tool {
        name: "audio.normalize",
        title: "Level Loudness",
        doc: "Measures how loud each clip sounds (integrated LUFS, as heard with its volume, fades and voice) and changes its volume so that it reaches `target`, -14 LUFS by default, without clipping. For clips recorded at different levels.",
    },
    Tool {
        name: "audio.beats",
        title: "Mark Beats",
        doc: "Finds the beat of a clip's music and puts a marker (note ♪) on every beat, or every `every`-th; clips and the playhead snap to markers, so cuts can be made on the beat. Beat markers already on the clip are replaced; `clear` only removes them.",
    },
    Tool {
        name: "audio.duck",
        title: "Duck Under Voice",
        doc: "Lowers a music clip by `depthDb` (12 by default) wherever the clips on other tracks are heard (or only the ones in `under`), with volume keyframes that ease down before and back up after. The clip's own volume keyframes are replaced.",
    },
    Tool {
        name: "audio.denoise",
        title: "Reduce Noise",
        doc: "Measures a clip's steady background noise (hiss, hum, a fan) from its quietest moments and takes out `amount` of it, 0.7 by default (clip.setDenoise holds the result; amount 0 turns it off).",
    },
];

/// The note a beat's marker carries.
pub const BEAT: &str = "♪";

/// The longest stretch a tool listens to.
const LIMIT: Tick = Tick(300 * TICKS_PER_SECOND);

/// The JSON Schema of tool `name`'s parameters.
pub fn tool_schema(name: &str) -> Option<Value> {
    let clip = json!({"type": "string", "description": "Clip id, such as c4."});
    let clips = |doc: &str| json!({"type": "array", "items": {"type": "string"}, "description": doc});
    let object = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required, "additionalProperties": false});
    Some(match name {
        "audio.normalize" => object(
            json!({
                "clips": clips("The clips to level; every clip with sound by default."),
                "target": {"type": "number", "minimum": -40, "maximum": -5, "description": "The loudness to reach, in LUFS; -14 by default."}
            }),
            &[],
        ),
        "audio.beats" => object(
            json!({
                "clip": clip,
                "every": {"type": "integer", "minimum": 1, "maximum": 8, "description": "Mark every n-th beat; 1 by default."},
                "clear": {"type": "boolean", "description": "Only remove the clip's beat markers."}
            }),
            &["clip"],
        ),
        "audio.duck" => object(
            json!({
                "clip": clip,
                "under": clips("The clips to make room for; every sounding clip on another track by default."),
                "depthDb": {"type": "number", "minimum": 1, "maximum": 40, "description": "How far the clip is lowered, in dB; 12 by default."}
            }),
            &["clip"],
        ),
        "audio.denoise" => object(
            json!({
                "clip": clip,
                "amount": input_schema("clip.setDenoise")?["properties"]["amount"].clone()
            }),
            &["clip"],
        ),
        _ => return None,
    })
}

/// What a tool found and what to do about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The name the undo step gets.
    pub label: &'static str,
    /// `edit.batch` steps; may be empty when nothing needs changing.
    pub steps: Vec<Value>,
    /// What was measured, for whoever asked.
    pub found: Value,
}

impl Plan {
    /// The parameters of the `edit.batch` that carries it out.
    pub fn batch(&self) -> Value {
        json!({ "steps": self.steps, "label": self.label })
    }
}

fn step(id: &str, params: Value) -> Value {
    json!({ "id": id, "params": params })
}

fn samples(t: Tick) -> i64 {
    (t.0 as i128 * i128::from(RATE) / TICKS_PER_SECOND as i128) as i64
}

fn ticks(samples: i64) -> Tick {
    Tick((samples as i128 * TICKS_PER_SECOND as i128 / i128::from(RATE)) as i64)
}

fn tenth(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

/// The sound of clips `ids` from `from` for `len` (at most [`LIMIT`]) at [`RATE`]: `as_heard` with
/// each clip's volume and effects, otherwise plain (its speed and pitch only).
fn sound_of(
    project: &Project,
    media: &Footage,
    ids: &[String],
    as_heard: bool,
    from: Tick,
    len: Tick,
) -> Result<Vec<Vec<f32>>, String> {
    let mut spec = compile(project, &CompileOptions { encode: false }).map_err(|e| e.to_string())?;
    spec.audio.retain(|a| ids.contains(&a.id));
    if spec.audio.is_empty() {
        return Err(match ids {
            [one] => format!("clip {one:?} has no sound (it is silent, muted or not a sound)"),
            _ => "none of these clips has sound".into(),
        });
    }
    if !as_heard {
        for a in &mut spec.audio {
            a.gain_db = Animated::Constant(0.0);
            a.fx.clear();
        }
    }
    let (a, b) = (samples(from), samples(from + len.min(LIMIT)));
    mix(&spec, media, RATE, a, (b - a).max(0) as usize).map_err(|e| e.to_string())
}

fn clip_of<'a>(project: &'a Project, id: &str) -> Result<(usize, &'a Clip), String> {
    project.clip(id).ok_or_else(|| format!("no clip {id:?}"))
}

fn clip_param(params: &Value) -> Result<String, String> {
    params.get("clip").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "`clip` is required".into())
}

fn ids_param(params: &Value, name: &str) -> Result<Option<Vec<String>>, String> {
    match params.get(name).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v
            .as_array()
            .and_then(|l| l.iter().map(|x| x.as_str().map(str::to_string)).collect::<Option<Vec<_>>>())
            .map(Some)
            .ok_or_else(|| format!("`{name}` must be a list of clip ids")),
    }
}

fn number(params: &Value, name: &str, default: f32, range: (f32, f32)) -> Result<f32, String> {
    match params.get(name).filter(|v| !v.is_null()) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .map(|x| x as f32)
            .filter(|x| (range.0..=range.1).contains(x))
            .ok_or_else(|| format!("`{name}` must be from {} to {}", range.0, range.1)),
    }
}

/// Listens for tool `name` (one of [`TOOLS`]) with `params` and plans its edit. The error says
/// what could not be heard or which parameter is wrong.
pub fn plan(name: &str, project: &Project, media: &Footage, params: &Value) -> Result<Plan, String> {
    match name {
        "audio.normalize" => normalize(project, media, params),
        "audio.beats" => beats(project, media, params),
        "audio.duck" => duck(project, media, params),
        "audio.denoise" => denoise(project, media, params),
        other => Err(format!("unknown tool {other:?}")),
    }
}

fn normalize(project: &Project, media: &Footage, params: &Value) -> Result<Plan, String> {
    let target = number(params, "target", -14.0, (-40.0, -5.0))?;
    let ids = match ids_param(params, "clips")? {
        Some(ids) => ids,
        // every clip that sounds
        None => compile(project, &CompileOptions { encode: false })
            .map_err(|e| e.to_string())?
            .audio
            .into_iter()
            .map(|a| a.id)
            .collect(),
    };
    let (mut steps, mut found) = (vec![], vec![]);
    for id in &ids {
        let (_, clip) = clip_of(project, id)?;
        let sound = sound_of(project, media, std::slice::from_ref(id), true, clip.start, clip.duration)?;
        let Some(lufs) = analysis::loudness(&sound) else {
            found.push(json!({ "clip": id, "lufs": null, "changeDb": 0.0 }));
            continue;
        };
        // as far as the target, and no further than its peaks have room for
        let room = -0.5 - 20.0 * analysis::peak(&sound).max(1e-6).log10();
        let change = (target - lufs).min(room);
        found.push(
            json!({ "clip": id, "lufs": tenth(lufs), "changeDb": tenth(change), "limited": change < target - lufs }),
        );
        if change.abs() < 0.1 {
            continue;
        }
        match clip.keys.get(&Prop::VolumeDb).filter(|k| !k.is_empty()) {
            Some(keys) => {
                for k in keys {
                    steps.push(step(
                        "clip.setKey",
                        json!({ "clip": id, "prop": "volumeDb", "at": (clip.start + k.t).0, "value": tenth(k.v + change) }),
                    ));
                }
            }
            None => steps.push(step("clip.set", json!({ "clip": id, "volumeDb": tenth(clip.volume_db + change) }))),
        }
    }
    if found.is_empty() {
        return Err("no clip has sound".into());
    }
    Ok(Plan { label: "Level Loudness", steps, found: json!({ "target": target, "clips": found }) })
}

fn beats(project: &Project, media: &Footage, params: &Value) -> Result<Plan, String> {
    let id = clip_param(params)?;
    let every = match params.get("every").filter(|v| !v.is_null()) {
        None => 1,
        Some(v) => v.as_u64().filter(|n| (1..=8).contains(n)).ok_or("`every` must be from 1 to 8")? as usize,
    };
    let clear = params.get("clear").and_then(Value::as_bool).unwrap_or(false);
    let (_, clip) = clip_of(project, &id)?;
    // the beats this clip was given before go either way
    let mut steps: Vec<Value> = project
        .markers
        .iter()
        .filter(|m| m.note == BEAT && m.at >= clip.start && m.at <= clip.end())
        .map(|m| step("marker.remove", json!({ "marker": m.id })))
        .collect();
    if clear {
        let removed = steps.len();
        return Ok(Plan { label: "Clear Beats", steps, found: json!({ "clip": id, "removed": removed }) });
    }
    let sound = sound_of(project, media, std::slice::from_ref(&id), false, clip.start, clip.duration)?;
    let found = analysis::beats(&analysis::mono(&sound), RATE).ok_or("no steady beat was found in this clip")?;
    let fps = project.canvas.fps;
    let mut last = None;
    let mut marked = 0;
    for at in found.at.iter().step_by(every) {
        let t = fps.snap(clip.start + ticks(*at));
        // two beats in one frame are one marker
        if last == Some(t) || t > clip.end() {
            continue;
        }
        last = Some(t);
        marked += 1;
        steps.push(step("marker.add", json!({ "at": t.0, "note": BEAT })));
    }
    Ok(Plan { label: "Mark Beats", steps, found: json!({ "clip": id, "bpm": tenth(found.bpm), "beats": marked }) })
}

/// How long the music takes to go down before a voice, and to come back after it.
const ATTACK: Tick = Tick(TICKS_PER_SECOND / 5);
const RELEASE: Tick = Tick(TICKS_PER_SECOND / 2);

fn duck(project: &Project, media: &Footage, params: &Value) -> Result<Plan, String> {
    let id = clip_param(params)?;
    let depth = number(params, "depthDb", 12.0, (1.0, 40.0))?;
    let (track, clip) = clip_of(project, &id)?;
    let over = match ids_param(params, "under")? {
        Some(ids) => ids.into_iter().filter(|other| *other != id).collect(),
        None => project
            .tracks
            .iter()
            .enumerate()
            .filter(|(ti, _)| *ti != track)
            .flat_map(|(_, t)| t.clips.iter())
            .filter(|c| c.range().overlaps(clip.range()))
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
    };
    if over.is_empty() {
        return Err("nothing else plays while this clip does".into());
    }
    // the clip itself must have sound to lower
    sound_of(project, media, std::slice::from_ref(&id), false, clip.start, Tick(1))?;
    let voice = sound_of(project, media, &over, true, clip.start, clip.duration)
        .map_err(|_| "nothing else sounds while this clip plays".to_string())?;
    let len = clip.duration;
    // spans close enough that the music would not get back up between them are one
    let mut spans: Vec<(Tick, Tick)> = Vec::new();
    for (a, b) in analysis::heard_spans(&analysis::mono(&voice), RATE) {
        let (a, b) = (ticks(a), ticks(b).min(len));
        match spans.last_mut() {
            Some(last) if a - last.1 < ATTACK + RELEASE => last.1 = b,
            _ => spans.push((a, b)),
        }
    }
    if spans.is_empty() {
        return Err("nothing is heard over this clip".into());
    }
    let keys = clip.keys.get(&Prop::VolumeDb).filter(|k| !k.is_empty());
    // the level it comes back up to: where it was, the top of any ducking it had
    let base = keys.map_or(clip.volume_db, |k| k.iter().map(|k| k.v).fold(f32::MIN, f32::max));
    let mut steps = vec![];
    if keys.is_some() {
        steps.push(step("clip.clearKeys", json!({ "clip": id, "prop": "volumeDb" })));
        steps.push(step("clip.set", json!({ "clip": id, "volumeDb": tenth(base) })));
    }
    let mut key = |t: Tick, v: f32| {
        steps.push(step(
            "clip.setKey",
            json!({ "clip": id, "prop": "volumeDb", "at": (clip.start + t).0, "value": tenth(v), "ease": "linear" }),
        ));
    };
    for (a, b) in &spans {
        if *a - ATTACK > Tick::ZERO {
            key(*a - ATTACK, base);
        }
        key((*a).max(Tick::ZERO), base - depth);
        key(*b, base - depth);
        if *b + RELEASE < len {
            key(*b + RELEASE, base);
        }
    }
    Ok(Plan {
        label: "Duck Under Voice",
        steps,
        found: json!({ "clip": id, "under": over, "spans": spans.len(), "depthDb": depth }),
    })
}

fn denoise(project: &Project, media: &Footage, params: &Value) -> Result<Plan, String> {
    let id = clip_param(params)?;
    let amount = number(params, "amount", Denoise::AMOUNT, (0.0, 1.0))?;
    let (_, clip) = clip_of(project, &id)?;
    if amount == 0.0 {
        let steps = vec![step("clip.setDenoise", json!({ "clip": id, "amount": 0.0 }))];
        return Ok(Plan { label: "Reduce Noise", steps, found: json!({ "clip": id, "amount": 0.0 }) });
    }
    let sound = sound_of(project, media, std::slice::from_ref(&id), false, clip.start, clip.duration)?;
    let floor = analysis::noise_floor(&analysis::mono(&sound), RATE)
        .ok_or("this clip is too short or too quiet to tell its noise from its sound")?;
    let floor: Vec<f64> = floor.iter().map(|v| tenth(*v)).collect();
    // the noise's level over the whole band, for whoever asked
    let level = floor.iter().sum::<f64>() / floor.len() as f64 + 10.0 * (f64::from(RATE) / 2.0).log10();
    Ok(Plan {
        label: "Reduce Noise",
        steps: vec![step("clip.setDenoise", json!({ "clip": id, "amount": amount, "floor": floor }))],
        found: json!({ "clip": id, "amount": amount, "noiseDb": (level * 10.0).round() / 10.0 }),
    })
}
