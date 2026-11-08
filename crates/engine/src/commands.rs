//! The command registry. Every action in reelforge is one of these.

use reelforge_compile::{CompileOptions, compile};
use reelforge_model::{
    AnimKind, Animation, Clip, ClipContent, ClipEffect, ClipTransform, Ease, Key, Media, MediaKind, Outline, Project,
    Prop, Shadow, TitleStyle, Track, TrackKind, Transition, TransitionKind, anim,
};
use reelforge_renderspec::AssetKind;
use reelforge_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::params::P;
use crate::templates::{TitleTemplate, title_template, title_templates};
use crate::{CmdResult, EngineError, Session};

/// One command: identity, documentation for people and agents, an enablement check and the work.
pub struct CommandSpec {
    /// Stable id, `area.verb`.
    pub id: &'static str,
    /// Human label for menus and buttons.
    pub label: &'static str,
    /// Parameters, as agents see them. Times accept ticks, `{"seconds": s}` or `{"frame": n}`.
    pub params: &'static str,
    /// What it does, in one sentence.
    pub doc: &'static str,
    /// Whether it changes the project (false for queries).
    pub edits: bool,
    /// `Err(reason)` when the command cannot run now.
    pub enabled: fn(&Session) -> Result<(), String>,
    /// The work.
    pub run: fn(&mut Session, &Value) -> CmdResult,
}

fn always(_: &Session) -> Result<(), String> {
    Ok(())
}

fn can_undo(s: &Session) -> Result<(), String> {
    if s.undo.is_empty() { Err("nothing to undo".into()) } else { Ok(()) }
}

fn can_redo(s: &Session) -> Result<(), String> {
    if s.redo.is_empty() { Err("nothing to redo".into()) } else { Ok(()) }
}

static REGISTRY: &[CommandSpec] = &[
    CommandSpec {
        id: "commands.list",
        label: "List Commands",
        params: "{}",
        doc: "Every command with its parameters.",
        edits: false,
        enabled: always,
        run: commands_list,
    },
    CommandSpec {
        id: "captions.import",
        label: "Import Captions",
        params: r#"{"srt": string, "track"?: id, "template"?: id}"#,
        doc: "Places SubRip (.srt) captions as titles, low on the frame, on a new Captions track unless `track` names a text track; `template` sets their look (classic by default).",
        edits: true,
        enabled: always,
        run: crate::captions::captions_import,
    },
    CommandSpec {
        id: "captions.export",
        label: "Export Captions",
        params: r#"{"track"?: id}"#,
        doc: "The titles on a text track (the Captions track by default) as SubRip (.srt) text.",
        edits: false,
        enabled: always,
        run: crate::captions::captions_export,
    },
    CommandSpec {
        id: "title.templates",
        label: "Title Templates",
        params: "{}",
        doc: "The ready-made title looks (花字): pass an id as `template` to clip.add or clip.set.",
        edits: false,
        enabled: always,
        run: title_templates_list,
    },
    CommandSpec {
        id: "project.inspect",
        label: "Inspect Project",
        params: "{}",
        doc: "The whole project document, its length and the session revision.",
        edits: false,
        enabled: always,
        run: project_inspect,
    },
    CommandSpec {
        id: "project.setCanvas",
        label: "Canvas Settings",
        params: r#"{"width"?: u32, "height"?: u32, "fps"?: [num, den], "background"?: [r,g,b,a]}"#,
        doc: "Changes the output size, frame rate or background.",
        edits: true,
        enabled: always,
        run: project_set_canvas,
    },
    CommandSpec {
        id: "media.import",
        label: "Import Media",
        params: r#"{"path": string, "kind"?: "video"|"image"|"audio", "name"?: string, "duration"?: time, "size"?: [w, h], "sha256"?: hex}"#,
        doc: "Adds a file to the project's media. When the file is reachable, hashes it and measures its kind, picture size and length (any value passed wins).",
        edits: true,
        enabled: always,
        run: media_import,
    },
    CommandSpec {
        id: "track.add",
        label: "Add Track",
        params: r#"{"kind": "video"|"text"|"audio", "name"?: string}"#,
        doc: "Adds a track on top of the others of its kind.",
        edits: true,
        enabled: always,
        run: track_add,
    },
    CommandSpec {
        id: "track.set",
        label: "Track Settings",
        params: r#"{"track": id, "name"?: string, "hidden"?: bool, "muted"?: bool}"#,
        doc: "Renames a track, hides its picture or mutes its sound.",
        edits: true,
        enabled: always,
        run: track_set,
    },
    CommandSpec {
        id: "clip.add",
        label: "Add Clip",
        params: r#"{"track": id, "start"?: time, "duration"?: time, "media"?: id, "sourceIn"?: time, "text"?: string, "size"?: px, "color"?: [r,g,b,a], "weight"?: 100-900, "align"?: "left"|"center"|"right", "wrap"?: 0-1, "outline"?: {"color": [r,g,b,a], "width": em} | null, "background"?: [r,g,b,a] | null, "radius"?: em, "gradient"?: [r,g,b,a] | null, "outer"?: {"color", "width"} | null, "shadow"?: {"color", "offset": [x, y] em, "blur": em} | null, "template"?: id}"#,
        doc: "Places media, text or a colour on a track; starts at the track's end unless `start` is given. Titles default to bold white with a black outline, wrapping at 90 % of the canvas.",
        edits: true,
        enabled: always,
        run: clip_add,
    },
    CommandSpec {
        id: "clip.split",
        label: "Split",
        params: r#"{"clip": id, "at": time}"#,
        doc: "Cuts a clip in two at a timeline time inside it.",
        edits: true,
        enabled: always,
        run: clip_split,
    },
    CommandSpec {
        id: "clip.trim",
        label: "Trim",
        params: r#"{"clip": id, "start"?: time, "end"?: time}"#,
        doc: "Moves a clip's start or end edge on the timeline; media keeps its sync.",
        edits: true,
        enabled: always,
        run: clip_trim,
    },
    CommandSpec {
        id: "clip.move",
        label: "Move",
        params: r#"{"clip": id, "start": time, "track"?: id}"#,
        doc: "Moves a clip in time and optionally to another track.",
        edits: true,
        enabled: always,
        run: clip_move,
    },
    CommandSpec {
        id: "clip.remove",
        label: "Delete",
        params: r#"{"clips": id | [id]}"#,
        doc: "Removes clips, leaving gaps.",
        edits: true,
        enabled: always,
        run: clip_remove,
    },
    CommandSpec {
        id: "clip.setSpeed",
        label: "Speed",
        params: r#"{"clip": id, "speed"?: number, "keepPitch"?: bool}"#,
        doc: "Changes a media clip's speed; its length changes to keep the same source span. Sound keeps its pitch unless `keepPitch` is false.",
        edits: true,
        enabled: always,
        run: clip_set_speed,
    },
    CommandSpec {
        id: "clip.set",
        label: "Clip Properties",
        params: r#"{"clip": id, "x"?, "y"?, "scale"?, "rotation"?, "opacity"?, "volumeDb"?, "at"?: time, "fadeIn"?: time, "fadeOut"?: time, "text"?, "size"?, "color"?, "weight"?, "align"?, "wrap"?, "outline"?: {...} | null, "background"?: color | null, "radius"?: em, "gradient"?: [r,g,b,a] | null, "outer"?: {"color", "width"} | null, "shadow"?: {"color", "offset": [x, y] em, "blur": em} | null, "template"?: id}"#,
        doc: "Sets a clip's placement, opacity, volume, sound fades or title. A property with keyframes is set at `at` (a timeline time), adding or changing the keyframe there.",
        edits: true,
        enabled: always,
        run: clip_set,
    },
    CommandSpec {
        id: "clip.addEffect",
        label: "Add Effect",
        params: r#"{"clip": id, "effect": "name@version", "params"?: {name: number}}"#,
        doc: "Appends an effect to a picture clip.",
        edits: true,
        enabled: always,
        run: clip_add_effect,
    },
    CommandSpec {
        id: "clip.setKey",
        label: "Add Keyframe",
        params: r#"{"clip": id, "prop": "x"|"y"|"scale"|"rotation"|"opacity"|"volumeDb", "at": time, "value"?: number, "ease"?: "linear"|"hold"|"inOut"}"#,
        doc: "Sets a keyframe at a timeline time; without `value`, keeps the value the property has there. Once a property has keyframes, its value follows them.",
        edits: true,
        enabled: always,
        run: clip_set_key,
    },
    CommandSpec {
        id: "clip.removeKey",
        label: "Remove Keyframe",
        params: r#"{"clip": id, "prop": string, "at": time}"#,
        doc: "Removes the keyframe within half a frame of a timeline time; the last one leaves its value behind.",
        edits: true,
        enabled: always,
        run: clip_remove_key,
    },
    CommandSpec {
        id: "clip.clearKeys",
        label: "Clear Keyframes",
        params: r#"{"clip": id, "prop"?: string, "at"?: time}"#,
        doc: "Removes a property's keyframes (every property's without `prop`), keeping the value each had at `at` (the clip's start by default).",
        edits: true,
        enabled: always,
        run: clip_clear_keys,
    },
    CommandSpec {
        id: "clip.setAnimation",
        label: "Animation",
        params: r#"{"clip": id, "slot": "in"|"out", "kind": "fade"|"slideUp"|"slideDown"|"slideLeft"|"slideRight"|"zoomIn"|"zoomOut"|"pop"|"spin"|null, "duration"?: time}"#,
        doc: "Puts an in or out animation on a picture clip (null removes it); 0.5 s unless `duration` is given.",
        edits: true,
        enabled: always,
        run: clip_set_animation,
    },
    CommandSpec {
        id: "clip.setTransition",
        label: "Transition",
        params: r#"{"clip": id, "kind": "dissolve"|"fadeBlack"|"fadeWhite"|"pushLeft"|"pushRight"|"pushUp"|"pushDown"|"slideLeft"|"slideRight"|"slideUp"|"slideDown"|"zoom"|null, "duration"?: time}"#,
        doc: "Puts a transition from the clip before on a video track into this one, centred on the cut (null removes it); 0.5 s unless `duration` is given. It plays while the two clips touch.",
        edits: true,
        enabled: always,
        run: clip_set_transition,
    },
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        params: "{}",
        doc: "Undoes the last edit.",
        edits: true,
        enabled: can_undo,
        run: |s, _| s.swap(true),
    },
    CommandSpec {
        id: "edit.redo",
        label: "Redo",
        params: "{}",
        doc: "Redoes the last undone edit.",
        edits: true,
        enabled: can_redo,
        run: |s, _| s.swap(false),
    },
    CommandSpec {
        id: "edit.batch",
        label: "Batch",
        params: r#"{"steps": [{"id": string, "params"?: object}], "label"?: string}"#,
        doc: "Runs several commands as one undo step; if any fails, none applies.",
        edits: true,
        enabled: always,
        run: edit_batch,
    },
    CommandSpec {
        id: "render.compile",
        label: "Compile RenderSpec",
        params: r#"{"encode"?: bool}"#,
        doc: "The project as a RenderSpec, the document any reelforge renderer draws.",
        edits: false,
        enabled: always,
        run: render_compile,
    },
];

/// Every command.
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
}

pub(crate) fn fps(s: &Session) -> FrameRate {
    s.project.canvas.fps
}

pub(crate) fn not_found(what: &str, id: &str) -> EngineError {
    EngineError::Params(format!("no {what} {id:?}"))
}

fn commands_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(
        REGISTRY
            .iter()
            .map(|c| {
                json!({
                    "id": c.id, "label": c.label, "params": c.params, "doc": c.doc, "edits": c.edits,
                    "schema": crate::input_schema(c.id),
                    "enabled": (c.enabled)(s).err().map_or(json!(true), |why| json!(why)),
                })
            })
            .collect(),
    ))
}

fn project_inspect(s: &mut Session, _: &Value) -> CmdResult {
    let p = s.project();
    Ok(
        json!({ "revision": s.revision, "duration": p.duration(), "project": serde_json::to_value(&*p).expect("serialises") }),
    )
}

fn project_set_canvas(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let (w, h, bg) = (p.opt_u32("width")?, p.opt_u32("height")?, p.opt_color("background")?);
    let rate = match p.opt("fps") {
        None => None,
        Some(r) => {
            let pair: [u32; 2] = serde_json::from_value(r.clone())
                .map_err(|_| EngineError::Params("`fps` must be [num, den]".into()))?;
            let rate = FrameRate { num: pair[0], den: pair[1] };
            if !rate.is_valid() {
                return Err(EngineError::Params("`fps` is not a supported rate".into()));
            }
            Some(rate)
        }
    };
    for d in [w, h].into_iter().flatten() {
        if d == 0 || d > 8192 || d % 2 != 0 {
            return Err(EngineError::Params("width and height must be even and between 2 and 8192".into()));
        }
    }
    s.edit("Canvas Settings", |proj| {
        let c = &mut proj.canvas;
        if let Some(w) = w {
            c.width = w;
        }
        if let Some(h) = h {
            c.height = h;
        }
        if let Some(r) = rate {
            c.fps = r;
        }
        if let Some(bg) = bg {
            c.background = bg;
        }
        Ok(json!({ "canvas": serde_json::to_value(&*c).expect("serialises") }))
    })
}

fn kind_from_path(path: &str) -> Option<MediaKind> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mov" | "m4v" | "mkv" | "webm" => Some(MediaKind::Video),
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Some(MediaKind::Image),
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" => Some(MediaKind::Audio),
        _ => None,
    }
}

fn media_import(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let path = p.str("path")?.to_string();
    let asked: Option<MediaKind> = match p.opt_str("kind")? {
        Some(k) => Some(
            serde_json::from_value(json!(k))
                .map_err(|_| EngineError::Params("`kind` must be video, image or audio".into()))?,
        ),
        None => None,
    };
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut sha256 = p.opt_str("sha256")?.map(str::to_string);
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut size = match p.opt("size") {
        Some(v) => Some(
            serde_json::from_value::<[u32; 2]>(v.clone())
                .map_err(|_| EngineError::Params("`size` must be [w, h]".into()))?,
        ),
        None => None,
    };
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut duration = p.opt_time("duration")?;
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut probed: Option<reelforge_media::Probe> = None;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use sha2::{Digest, Sha256};
        let local = std::path::Path::new(path.strip_prefix("file://").unwrap_or(&path));
        if sha256.is_none()
            && let Ok(bytes) = std::fs::read(local)
        {
            sha256 = Some(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect());
        }
        if local.is_file() {
            probed = Some(
                reelforge_media::probe_path(local)
                    .map_err(|e| EngineError::Refused(format!("cannot read {path}: {e}")))?,
            );
        }
    }
    let kind = match (asked, &probed) {
        (Some(k), _) => k,
        (None, Some(pr)) => match pr.kind {
            AssetKind::Video => MediaKind::Video,
            AssetKind::Image => MediaKind::Image,
            _ => MediaKind::Audio,
        },
        (None, None) => kind_from_path(&path)
            .ok_or_else(|| EngineError::Params("cannot tell the kind from the extension; pass `kind`".into()))?,
    };
    if let Some(pr) = probed {
        size = size.or(pr.size);
        duration = duration.or(pr.duration);
    }
    if let Some(h) = &sha256
        && (h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(EngineError::Params("`sha256` must be 64 hex digits".into()));
    }
    let sha256 = sha256.map(|h| h.to_ascii_lowercase());
    let name = p
        .opt_str("name")?
        .map(str::to_string)
        .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string());
    s.edit("Import Media", |proj| {
        let id = proj.mint("m");
        let m = Media { id: id.clone(), name, uri: path, sha256, kind, duration, size };
        let out = json!({ "media": serde_json::to_value(&m).expect("serialises") });
        proj.media.insert(id, m);
        Ok(out)
    })
}

fn track_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("track")?.to_string();
    let (name, hidden, muted) = (p.opt_str("name")?.map(str::to_string), p.opt_bool("hidden")?, p.opt_bool("muted")?);
    if name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return Err(EngineError::Params("`name` must not be empty".into()));
    }
    s.edit("Track Settings", |proj| {
        let t = proj.track_mut(&id).ok_or_else(|| EngineError::Refused(format!("no track {id:?}")))?;
        if let Some(n) = name {
            t.name = n;
        }
        t.hidden = hidden.unwrap_or(t.hidden);
        t.muted = muted.unwrap_or(t.muted);
        Ok(json!({ "track": id, "name": t.name, "hidden": t.hidden, "muted": t.muted }))
    })
}

fn track_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let kind: TrackKind = serde_json::from_value(json!(p.str("kind")?))
        .map_err(|_| EngineError::Params("`kind` must be video, text or audio".into()))?;
    let name = p.opt_str("name")?.map(str::to_string);
    s.edit("Add Track", |proj| {
        let id = proj.mint("t");
        let n = proj.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let name = name.unwrap_or_else(|| {
            format!(
                "{} {n}",
                match kind {
                    TrackKind::Video => "Video",
                    TrackKind::Text => "Text",
                    TrackKind::Audio => "Audio",
                }
            )
        });
        let at = proj.tracks.iter().rposition(|t| t.kind == kind).map_or(proj.tracks.len(), |i| i + 1);
        proj.tracks.insert(at, Track { id: id.clone(), kind, name, hidden: false, muted: false, clips: vec![] });
        Ok(json!({ "track": id }))
    })
}

const STILL_LENGTH: Tick = Tick(5 * reelforge_time::TICKS_PER_SECOND);

fn clip_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let track_id = p.str("track")?.to_string();
    let start = p.opt_time("start")?;
    let duration = p.opt_time("duration")?;
    let source_in = p.opt_time("sourceIn")?.unwrap_or(Tick::ZERO);
    let media = p.opt_str("media")?.map(str::to_string);
    let text = p.opt_str("text")?.map(str::to_string);
    let color = p.opt_color("color")?.or(template(&p)?.map(|t| t.color));
    let size = p.opt_f64("size")?.unwrap_or(96.0) as f32;
    s.edit("Add Clip", |proj| {
        let track = proj.track(&track_id).ok_or_else(|| not_found("track", &track_id))?;
        let start = start.unwrap_or_else(|| track.clips.last().map_or(Tick::ZERO, Clip::end));
        let (content, natural) = match (&media, &text) {
            (Some(m), None) => {
                let item = proj.media.get(m).ok_or_else(|| not_found("media", m))?;
                let natural = item.duration.map(|d| d - source_in).filter(|d| *d > Tick::ZERO);
                (ClipContent::Media { media: m.clone(), source_in, speed: 1.0 }, natural)
            }
            (None, Some(t)) => (
                ClipContent::Text {
                    text: t.clone(),
                    size,
                    color: color.unwrap_or([1.0; 4]),
                    style: title_style(&p, TitleStyle::subtitle())?,
                },
                None,
            ),
            (None, None) => (
                ClipContent::Solid {
                    color: color.ok_or_else(|| EngineError::Params("give `media`, `text` or `color`".into()))?,
                },
                None,
            ),
            (Some(_), Some(_)) => return Err(EngineError::Params("give either `media` or `text`, not both".into())),
        };
        let duration = duration.or(natural).unwrap_or(STILL_LENGTH);
        let id = proj.mint("c");
        let clip = Clip {
            id: id.clone(),
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
        };
        proj.track_mut(&track_id).expect("found above").clips.push(clip);
        Ok(json!({ "clip": id, "start": start, "duration": duration }))
    })
}

fn clip_split(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let at = p.time("at")?;
    s.edit("Split", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if at <= clip.start || at >= clip.end() {
            return Err(EngineError::Params("`at` must be strictly inside the clip".into()));
        }
        let mut right = clip.clone();
        let left_len = at - clip.start;
        right.start = at;
        right.duration = clip.end() - at;
        right.id = proj.mint("c");
        if let ClipContent::Media { source_in, speed, .. } = &mut right.content {
            *source_in += left_len.scale(*speed);
        }
        // keys stay where they were on the timeline; the in animation stays on the left half and
        // the out animation on the right
        right.shift_keys(-left_len);
        right.anim.enter = None;
        right.transition = None;
        right.fade.fade_in = Tick::ZERO;
        let right_id = right.id.clone();
        proj.tracks[ti].clips.push(right);
        let left = proj.clip_mut(&id).expect("exists");
        left.duration = left_len;
        left.anim.exit = None;
        left.fade.fade_out = Tick::ZERO;
        Ok(json!({ "left": id, "right": right_id }))
    })
}

fn clip_trim(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let (new_start, new_end) = (p.opt_time("start")?, p.opt_time("end")?);
    s.edit("Trim", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let start = new_start.unwrap_or(clip.start);
        let end = new_end.unwrap_or(clip.end());
        if end <= start {
            return Err(EngineError::Params("the clip would have no length".into()));
        }
        let delta = start - clip.start;
        if let ClipContent::Media { source_in, speed, .. } = &mut clip.content {
            let moved = *source_in + delta.scale(*speed);
            if moved < Tick::ZERO {
                return Err(EngineError::Refused("the clip cannot start before its media does".into()));
            }
            *source_in = moved;
        }
        clip.shift_keys(-delta);
        clip.start = start;
        clip.duration = end - start;
        Ok(json!({ "clip": id, "start": start, "duration": end - start }))
    })
}

fn clip_move(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let start = p.time("start")?;
    let to = p.opt_str("track")?.map(str::to_string);
    s.edit("Move", |proj| {
        let (ti, _) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let pos = proj.tracks[ti].clips.iter().position(|c| c.id == id).expect("found");
        let mut clip = proj.tracks[ti].clips.remove(pos);
        clip.start = start;
        let dest = match &to {
            Some(t) => proj.tracks.iter().position(|x| &x.id == t).ok_or_else(|| not_found("track", t))?,
            None => ti,
        };
        proj.tracks[dest].clips.push(clip);
        Ok(json!({ "clip": id, "start": start, "track": proj.tracks[dest].id }))
    })
}

fn clip_remove(s: &mut Session, v: &Value) -> CmdResult {
    let ids = P::new(v, fps(s)).strings("clips")?;
    s.edit("Delete", |proj| {
        for id in &ids {
            let (ti, _) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            proj.tracks[ti].clips.retain(|c| &c.id != id);
        }
        Ok(json!({ "removed": ids }))
    })
}

fn clip_set_speed(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let new = p.opt_f64("speed")?;
    let keep = p.opt_bool("keepPitch")?;
    if new.is_none() && keep.is_none() {
        return Err(EngineError::Params("give `speed` or `keepPitch`".into()));
    }
    if new.is_some_and(|n| !(n > 0.0 && n <= 100.0)) {
        return Err(EngineError::Params("`speed` must be above 0 and at most 100".into()));
    }
    s.edit("Speed", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let ClipContent::Media { speed, .. } = &mut clip.content else {
            return Err(EngineError::Params("only media clips have a speed".into()));
        };
        if let Some(new) = new {
            let source_span = clip.duration.scale(*speed);
            let old = *speed;
            *speed = new;
            clip.duration = source_span.scale(1.0 / new).max(Tick(1));
            // keys follow the picture they were set on
            clip.scale_keys(old / new);
        }
        if let Some(k) = keep {
            clip.keep_pitch = k;
        }
        Ok(json!({ "clip": id, "duration": clip.duration, "keepPitch": clip.keep_pitch }))
    })
}

const STYLE_KEYS: [&str; 10] =
    ["weight", "align", "wrap", "outline", "background", "radius", "gradient", "outer", "shadow", "template"];

/// `base` with the title style parameters in `p` applied (`null` removes an outline or box).
fn title_style(p: &P, base: TitleStyle) -> Result<TitleStyle, EngineError> {
    let mut st = base;
    if let Some(t) = template(p)? {
        // a template brings its look; where the lines wrap and how they line up stay
        st = TitleStyle { align: st.align, wrap: st.wrap, ..t.style };
    }
    if let Some(w) = p.opt_u32("weight")? {
        st.weight = u16::try_from(w).map_err(|_| EngineError::Params("`weight` must be 100 to 900".into()))?;
    }
    if let Some(a) = p.opt_str("align")? {
        st.align = serde_json::from_value(json!(a))
            .map_err(|_| EngineError::Params("`align` must be left, center or right".into()))?;
    }
    if let Some(w) = p.opt_f64("wrap")? {
        st.wrap = w as f32;
    }
    match p.v.get("outline") {
        Some(Value::Null) => st.outline = None,
        Some(v) => {
            let o = P::new(v, p.fps);
            let current = st.outline.clone().unwrap_or(Outline { color: [0.0, 0.0, 0.0, 1.0], width: 0.08 });
            st.outline = Some(Outline {
                color: o.opt_color("color")?.unwrap_or(current.color),
                width: o.opt_f64("width")?.map_or(current.width, |w| w as f32),
            });
        }
        None => {}
    }
    match p.v.get("background") {
        Some(Value::Null) => st.background = None,
        Some(_) => st.background = p.opt_color("background")?,
        None => {}
    }
    if let Some(r) = p.opt_f64("radius")? {
        st.radius = (r as f32).clamp(0.0, 4.0);
    }
    match p.v.get("gradient") {
        Some(Value::Null) => st.gradient = None,
        Some(_) => st.gradient = p.opt_color("gradient")?,
        None => {}
    }
    match p.v.get("outer") {
        Some(Value::Null) => st.outer = None,
        Some(v) => {
            let o = P::new(v, p.fps);
            let current = st.outer.clone().unwrap_or(Outline { color: [1.0; 4], width: 0.08 });
            st.outer = Some(Outline {
                color: o.opt_color("color")?.unwrap_or(current.color),
                width: o.opt_f64("width")?.map_or(current.width, |w| w as f32),
            });
        }
        None => {}
    }
    match p.v.get("shadow") {
        Some(Value::Null) => st.shadow = None,
        Some(v) => {
            let o = P::new(v, p.fps);
            let current =
                st.shadow.clone().unwrap_or(Shadow { color: [0.0, 0.0, 0.0, 0.6], offset: [0.0, 0.05], blur: 0.08 });
            let offset = match o.opt("offset") {
                Some(v) => serde_json::from_value::<[f32; 2]>(v.clone())
                    .map_err(|_| EngineError::Params("`shadow.offset` must be [right, down] in ems".into()))?,
                None => current.offset,
            };
            if offset.iter().any(|v| v.abs() > 4.0) {
                return Err(EngineError::Params("`shadow.offset` must be within 4 ems".into()));
            }
            st.shadow = Some(Shadow {
                color: o.opt_color("color")?.unwrap_or(current.color),
                offset,
                blur: o.opt_f64("blur")?.map_or(current.blur, |b| (b as f32).clamp(0.0, 4.0)),
            });
        }
        None => {}
    }
    Ok(st)
}

/// The template the `template` parameter names, if any.
fn template(p: &P) -> Result<Option<TitleTemplate>, EngineError> {
    let Some(id) = p.opt_str("template")? else { return Ok(None) };
    let known = || title_templates().iter().map(|t| t.id).collect::<Vec<_>>().join(", ");
    title_template(id)
        .map(Some)
        .ok_or_else(|| EngineError::Params(format!("no title template {id:?}; there are {}", known())))
}

fn title_templates_list(_: &mut Session, _: &Value) -> CmdResult {
    let list: Vec<Value> = title_templates()
        .into_iter()
        .map(|t| json!({"id": t.id, "name": t.name, "nameZh": t.name_zh, "color": t.color, "style": t.style}))
        .collect();
    Ok(json!({ "templates": list }))
}

fn clip_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let f = |name: &str| p.opt_f64(name).map(|o| o.map(|x| x as f32));
    let mut values = vec![];
    for prop in Prop::ALL {
        if let Some(v) = f(prop.name())? {
            values.push((prop, v));
        }
    }
    let at = p.opt_time("at")?;
    let (fade_in, fade_out) = (p.opt_time("fadeIn")?, p.opt_time("fadeOut")?);
    if [fade_in, fade_out].iter().flatten().any(|f| *f < Tick::ZERO) {
        return Err(EngineError::Params("a fade cannot be negative".into()));
    }
    let (text, size) = (p.opt_str("text")?.map(str::to_string), f("size")?);
    // a template brings its colour unless one is given
    let color = p.opt_color("color")?.or(template(&p)?.map(|t| t.color));
    let styled = STYLE_KEYS.iter().any(|k| p.v.get(k).is_some());
    if values.iter().any(|&(p, v)| p == Prop::Opacity && !(0.0..=1.0).contains(&v)) {
        return Err(EngineError::Params("`opacity` must be between 0 and 1".into()));
    }
    s.edit("Clip Properties", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        for (prop, v) in values {
            if !clip.keyed(prop) {
                clip.set_base(prop, v);
                continue;
            }
            // a keyframed property is set where the playhead is: that key changes or a new one is made
            let Some(at) = at else {
                return Err(EngineError::Params(format!(
                    "`{}` has keyframes; pass `at` to set it at that time (clip.clearKeys drops them)",
                    prop.name()
                )));
            };
            let t = at - clip.start;
            if clip.value_at(prop, t) != v {
                let list = clip.keys.get_mut(&prop).expect("keyed");
                let ease = list.iter().find(|k| k.t == t).map_or(Ease::Linear, |k| k.ease);
                anim::insert(list, Key { t, v, ease });
            }
        }
        if let Some(f) = fade_in {
            clip.fade.fade_in = f;
        }
        if let Some(f) = fade_out {
            clip.fade.fade_out = f;
        }
        if styled && !matches!(clip.content, ClipContent::Text { .. }) {
            return Err(EngineError::Params("title style applies to text clips".into()));
        }
        match &mut clip.content {
            ClipContent::Text { text: tt, size: ts, color: tc, style } => {
                *style = title_style(&p, style.clone())?;
                if let Some(v) = text {
                    *tt = v;
                }
                if let Some(v) = size {
                    *ts = v;
                }
                if let Some(v) = color {
                    *tc = v;
                }
            }
            ClipContent::Solid { color: sc } => {
                if let Some(v) = color {
                    *sc = v;
                }
            }
            ClipContent::Media { .. } if text.is_some() || size.is_some() || color.is_some() => {
                return Err(EngineError::Params("`text`, `size` and `color` apply to text and colour clips".into()));
            }
            ClipContent::Media { .. } => {}
        }
        Ok(json!({ "clip": serde_json::to_value(&*clip).expect("serialises") }))
    })
}

fn clip_add_effect(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let effect = p.str("effect")?.to_string();
    let params: std::collections::BTreeMap<String, f32> = match p.opt("params") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|_| EngineError::Params("`params` must map names to numbers".into()))?,
        None => Default::default(),
    };
    if !effect
        .split_once('@')
        .is_some_and(|(n, ver)| !n.is_empty() && !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(EngineError::Params("`effect` must look like name@1".into()));
    }
    s.edit("Add Effect", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        clip.effects.push(ClipEffect { id: effect, params });
        Ok(json!({ "clip": id, "effects": clip.effects.len() }))
    })
}

fn prop(p: &P) -> Result<Prop, EngineError> {
    let name = p.str("prop")?;
    Prop::parse(name)
        .ok_or_else(|| EngineError::Params(format!("`prop` must be one of {}", Prop::ALL.map(Prop::name).join(", "))))
}

fn clip_set_key(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let prop = prop(&p)?;
    let at = p.time("at")?;
    let value = p.opt_f64("value")?.map(|x| x as f32);
    let ease: Option<Ease> = match p.opt_str("ease")? {
        Some(e) => Some(
            serde_json::from_value(json!(e))
                .map_err(|_| EngineError::Params("`ease` must be linear, hold or inOut".into()))?,
        ),
        None => None,
    };
    if value.is_some_and(|v| !v.is_finite() || (prop == Prop::Opacity && !(0.0..=1.0).contains(&v))) {
        return Err(EngineError::Params("`value` is out of range".into()));
    }
    s.edit("Add Keyframe", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let t = at - clip.start;
        let v = value.unwrap_or_else(|| clip.value_at(prop, t));
        let list = clip.keys.entry(prop).or_default();
        let ease = ease.or_else(|| list.iter().find(|k| k.t == t).map(|k| k.ease)).unwrap_or_default();
        anim::insert(list, Key { t, v, ease });
        Ok(json!({ "clip": id, "prop": prop.name(), "keys": list.len() }))
    })
}

fn clip_remove_key(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let prop = prop(&p)?;
    let at = p.time("at")?;
    let half = Tick(fps(s).frame_duration().0 / 2);
    s.edit("Remove Keyframe", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let t = at - clip.start;
        let list = clip
            .keys
            .get_mut(&prop)
            .filter(|l| !l.is_empty())
            .ok_or_else(|| EngineError::Refused(format!("`{}` has no keyframes", prop.name())))?;
        let i = list
            .iter()
            .enumerate()
            .filter(|(_, k)| (k.t - t).0.abs() <= half.0)
            .min_by_key(|(_, k)| (k.t - t).0.abs())
            .map(|(i, _)| i)
            .ok_or_else(|| EngineError::Refused(format!("no `{}` keyframe at that time", prop.name())))?;
        let gone = list.remove(i);
        let left = list.len();
        if left == 0 {
            clip.keys.remove(&prop);
            clip.set_base(prop, gone.v);
        }
        Ok(json!({ "clip": id, "prop": prop.name(), "keys": left }))
    })
}

fn clip_clear_keys(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let only = if p.opt("prop").is_some() { Some(prop(&p)?) } else { None };
    let at = p.opt_time("at")?;
    s.edit("Clear Keyframes", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let t = at.map_or(Tick::ZERO, |a| a - clip.start);
        let props: Vec<Prop> = clip.keys.keys().copied().filter(|k| only.is_none_or(|o| o == *k)).collect();
        for prop in &props {
            let v = clip.value_at(*prop, t);
            clip.keys.remove(prop);
            clip.set_base(*prop, v);
        }
        Ok(json!({ "clip": id, "cleared": props.iter().map(|p| p.name()).collect::<Vec<_>>() }))
    })
}

const ANIMATION_LENGTH: Tick = Tick(reelforge_time::TICKS_PER_SECOND / 2);

fn clip_set_animation(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let slot = p.str("slot")?.to_string();
    if slot != "in" && slot != "out" {
        return Err(EngineError::Params("`slot` must be in or out".into()));
    }
    let kind = match p.opt_str("kind")? {
        None => None,
        Some(k) => Some(AnimKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "`kind` must be null or one of {}",
                AnimKind::ALL.map(AnimKind::name).join(", ")
            ))
        })?),
    };
    let duration = p.opt_time("duration")?;
    if duration.is_some_and(|d| d <= Tick::ZERO) {
        return Err(EngineError::Params("`duration` must be positive".into()));
    }
    s.edit("Animation", |proj| {
        let (ti, _) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind == TrackKind::Audio {
            return Err(EngineError::Params("animations apply to picture and title clips".into()));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        let slot_ref = if slot == "in" { &mut clip.anim.enter } else { &mut clip.anim.exit };
        *slot_ref = kind.map(|kind| Animation {
            kind,
            duration: duration.or(slot_ref.map(|a| a.duration)).unwrap_or(ANIMATION_LENGTH),
        });
        Ok(json!({ "clip": id, "anim": serde_json::to_value(clip.anim).expect("serialises") }))
    })
}

fn clip_set_transition(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let kind = match p.opt_str("kind")? {
        None => None,
        Some(k) => Some(TransitionKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "`kind` must be null or one of {}",
                TransitionKind::ALL.map(TransitionKind::name).join(", ")
            ))
        })?),
    };
    let duration = p.opt_time("duration")?;
    if duration.is_some_and(|d| d <= Tick::ZERO) {
        return Err(EngineError::Params("`duration` must be positive".into()));
    }
    s.edit("Transition", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let track = &proj.tracks[ti];
        if kind.is_some() {
            if track.kind != TrackKind::Video {
                return Err(EngineError::Params("transitions go between clips on a video track".into()));
            }
            if !track.clips.iter().any(|c| c.end() == clip.start) {
                return Err(EngineError::Refused(
                    "no clip ends where this one starts; move them together first".into(),
                ));
            }
        }
        let clip = proj.clip_mut(&id).expect("found above");
        clip.transition = kind.map(|kind| Transition {
            kind,
            duration: duration.or(clip.transition.map(|t| t.duration)).unwrap_or(ANIMATION_LENGTH),
        });
        Ok(json!({ "clip": id, "transition": serde_json::to_value(clip.transition).expect("serialises") }))
    })
}

fn edit_batch(s: &mut Session, v: &Value) -> CmdResult {
    let steps = v
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::Params("`steps` must be a list".into()))?
        .clone();
    let label = v.get("label").and_then(Value::as_str).unwrap_or("Batch").to_string();
    let saved: (std::sync::Arc<Project>, usize, Vec<_>, u64) =
        (s.project.clone(), s.undo.len(), s.redo.clone(), s.revision);
    let mut results = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let id = step.get("id").and_then(Value::as_str).unwrap_or_default();
        let params = step.get("params").cloned().unwrap_or(Value::Null);
        let r = if id == "edit.undo" || id == "edit.redo" || id == "edit.batch" {
            Err(EngineError::Params(format!("{id} cannot run inside a batch")))
        } else {
            s.execute(id, &params)
        };
        match r {
            Ok(v) => results.push(v),
            Err(e) => {
                s.project = saved.0;
                s.undo.truncate(saved.1);
                s.redo = saved.2;
                s.revision = saved.3;
                return Err(EngineError::Refused(format!("step {i} ({id}) failed, nothing applied: {e}")));
            }
        }
    }
    if s.undo.len() > saved.1 {
        let before = s.undo[saved.1].1.clone();
        s.undo.truncate(saved.1);
        s.undo.push((label, before));
    }
    Ok(json!({ "results": results }))
}

fn render_compile(s: &mut Session, v: &Value) -> CmdResult {
    let encode = v.get("encode").and_then(Value::as_bool).unwrap_or(true);
    let spec = compile(&s.project, &CompileOptions { encode }).map_err(|e| EngineError::Refused(e.to_string()))?;
    Ok(serde_json::to_value(&spec).expect("serialises"))
}
