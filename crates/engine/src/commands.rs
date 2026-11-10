//! The command registry. Every action in reelforge is one of these.

use reelforge_compile::{CompileOptions, compile};
use reelforge_model::timeline as model_timeline;
use reelforge_model::{
    Adjust, AnimKind, Animation, BlendMode, ChromaKey, Clip, ClipContent, ClipEffect, ClipTransform, Codec, CurvePoint,
    CurvePreset, Denoise, Ease, ExportSettings, Filter, FilterKind, Fx, FxGroup, FxKind, Inbetween, Key, Loop,
    LoopKind, LutUse, Mask, MaskKind, Media, MediaKind, Outline, Project, Prop, Quality, Resolution, Shadow, Steady,
    Timeline, TitleStyle, Track, TrackKind, Transition, TransitionKind, Voice, VoiceKind, anim, export, timing,
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
        params: r#"{"srt": string, "track"?: id, "template"?: id, "line"?: 1 | 2}"#,
        doc: "Places SubRip (.srt) captions as titles, low on the frame, on a new Captions track unless `track` names a text track; `template` sets their look (classic by default). `line: 2` places a second language: smaller, under the first, on a track of its own.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_import,
    },
    CommandSpec {
        id: "capcut.import",
        label: "Import CapCut Draft",
        params: r#"{"draft": string, "folder"?: string, "name"?: string}"#,
        doc: "Reads a CapCut / 剪映 draft (the text of draft_content.json) into a new timeline, or into an untouched project (taking its canvas). `folder` is the draft's folder, for its relative paths. Video, photo, sound and text segments come across with speed, volume, opacity, placement, mirroring, sound fades and transitions (as dissolves); the reply lists what was skipped and which files are not on this machine.",
        edits: true,
        enabled: always,
        run: crate::capcut::capcut_import,
    },
    CommandSpec {
        id: "captions.add",
        label: "Add Captions",
        params: r#"{"cues": [{"start": time, "end": time, "text": string}], "track"?: id, "template"?: id}"#,
        doc: "Places timed lines (from speech recognition, a script, a translation) as captions, like captions.import does with SubRip.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_add,
    },
    CommandSpec {
        id: "voiceover.add",
        label: "Add Voice-over",
        params: r#"{"lines": [{"path": string, "start": time, "name"?: string, "sha256"?: string, "duration"?: time}], "track"?: id}"#,
        doc: "Imports spoken lines (sound files, from text to speech) and places each where its line starts, on a Voice-over audio track; one that would overlap goes on the next Voice-over track. One undo step.",
        edits: true,
        enabled: always,
        run: crate::voiceover::voiceover_add,
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
        id: "captions.list",
        label: "List Captions",
        params: r#"{"track"?: id}"#,
        doc: "The titles on a text track (the Captions track by default) in time order: each one's clip, start, end and words.",
        edits: false,
        enabled: always,
        run: crate::captions::captions_list,
    },
    CommandSpec {
        id: "captions.split",
        label: "Split Caption",
        params: r#"{"clip": id, "at"?: u32, "time"?: time}"#,
        doc: "Cuts a caption in two: its words after `at` characters (else where `time` falls, else in the middle) and its time at `time` (else in proportion to the words).",
        edits: true,
        enabled: always,
        run: crate::captions::captions_split,
    },
    CommandSpec {
        id: "captions.merge",
        label: "Merge Captions",
        params: r#"{"clips": [id]}"#,
        doc: "Joins captions on one track, with no other clip between them, into the first: it lasts until the last one ends and says all their words in order.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_merge,
    },
    CommandSpec {
        id: "captions.replace",
        label: "Replace in Captions",
        params: r#"{"find": string, "replace": string, "track"?: id}"#,
        doc: "Replaces `find` with `replace` in every title on a text track (the Captions track by default); a caption left without words is removed. Replies with how many were replaced.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_replace,
    },
    CommandSpec {
        id: "captions.restyle",
        label: "Match Captions' Style",
        params: r#"{"from": id, "track"?: id}"#,
        doc: "Gives every other title on the track (the one `from` is on by default) the look of title `from`: size, colour, style, placement, opacity and in/out animations.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_restyle,
    },
    CommandSpec {
        id: "captions.shift",
        label: "Shift Captions",
        params: r#"{"by": time, "track"?: id}"#,
        doc: "Moves every clip on a text track (the Captions track by default) later by `by` (earlier when negative), to line captions up with the sound.",
        edits: true,
        enabled: always,
        run: crate::captions::captions_shift,
    },
    CommandSpec {
        id: "captions.fillers",
        label: "Find Filler Captions",
        params: r#"{"track"?: id, "words"?: [string]}"#,
        doc: "Lists the captions of a text track that are nothing but filler words (嗯, 那个, 就是, um, you know, えーと, …, and `words` if given), with the time each takes: ranges for timeline.cut.",
        edits: false,
        enabled: always,
        run: crate::cut::captions_fillers,
    },
    CommandSpec {
        id: "captions.cutFillers",
        label: "Cut Filler Words",
        params: r#"{"track"?: id, "words"?: [string], "tracks"?: [id]}"#,
        doc: "Cuts the time of every caption that is nothing but filler words out of the timeline (see captions.fillers and timeline.cut; `tracks` as there), in one step.",
        edits: true,
        enabled: always,
        run: crate::cut::captions_cut_fillers,
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
        id: "sound.library",
        label: "Sound Library",
        params: "{}",
        doc: "The built-in sounds: effects (whoosh, pop, ding...) and music beds, made by synthesis so they are free to use. Add one with sound.add.",
        edits: false,
        enabled: always,
        run: sound_library,
    },
    CommandSpec {
        id: "sound.add",
        label: "Add Sound",
        params: r#"{"sound": id, "at"?: time, "track"?: id, "name"?: string}"#,
        doc: "Puts a library sound on an audio track (the given one, else the first, else a new one) at `at` (default: after the track's last clip).",
        edits: true,
        enabled: always,
        run: sound_add,
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
        id: "project.outline",
        label: "Outline",
        params: "{}",
        doc: "The timeline being edited in brief, with times in seconds: each track's clips (start, end, what they show, and what is set on them: placement, keyframed properties, animations, transition, effects, colour, crop or mask, voice, group), the media with their lengths, markers and the cover. Read this first; project.inspect is the whole document in ticks.",
        edits: false,
        enabled: always,
        run: |s, _| Ok(crate::outline::outline(&s.project())),
    },
    CommandSpec {
        id: "project.new",
        label: "New Project",
        params: "{}",
        doc: "Empties the project: no media, one timeline with an empty video, text and audio track, no markers. The canvas and export settings stay. One undo step brings the old project back.",
        edits: true,
        enabled: always,
        run: project_new,
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
        id: "project.setExport",
        label: "Export Settings",
        params: r#"{"preset"?: "draft"|"standard"|"smooth"|"4k", "format"?: "mp4"|"gif"|"m4a"|"wav", "resolution"?: "canvas"|"720p"|"1080p"|"2k"|"4k", "fps"?: 0|24|25|30|50|60, "quality"?: "low"|"recommended"|"high", "codec"?: "h264"|"hevc"|"av1"}"#,
        doc: "Sets how the project exports: a preset (which is an MP4), then any of the kind of file, picture size (by the shorter side; `canvas` keeps the canvas's), frame rate (0 keeps the canvas's), quality and codec. `gif` writes a silent animated GIF whose size and frame rate follow the quality alone (240 px at 10 fps, 360 at 12.5, 480 at 20); `m4a` (AAC) and `wav` write the sound alone. HEVC and AV1 are encoded by the browser's (hardware) encoder; the desktop and CLI write H.264.",
        edits: true,
        enabled: always,
        run: project_set_export,
    },
    CommandSpec {
        id: "project.setCover",
        label: "Cover",
        params: r#"{"at": time | null}"#,
        doc: "Makes the frame at `at` the cover of the timeline being edited (null: none). An export of a video writes the cover beside it as a JPEG; `render.frame` draws any frame.",
        edits: true,
        enabled: always,
        run: project_set_cover,
    },
    CommandSpec {
        id: "media.import",
        label: "Import Media",
        params: r#"{"path": string, "kind"?: "video"|"image"|"audio"|"lut"|"font", "name"?: string, "duration"?: time, "size"?: [w, h], "sha256"?: hex}"#,
        doc: "Adds a file to the project's media. When the file is reachable, hashes it and measures its kind, picture size and length (any value passed wins). A font (.ttf, .otf, .ttc) is named after its family and style, and is set on titles with clip.set `font`.",
        edits: true,
        enabled: always,
        run: media_import,
    },
    CommandSpec {
        id: "media.set",
        label: "Media Name and Folder",
        params: r#"{"media": id | [id], "name"?: string, "folder"?: string | null}"#,
        doc: "Renames a file in the project's media (one at a time), or files one or several under a folder of the media panel; `folder` null takes them out of theirs. A folder is only its name: it is there while some media is filed under it. Clips that use the media are not changed.",
        edits: true,
        enabled: always,
        run: media_set,
    },
    CommandSpec {
        id: "media.remove",
        label: "Remove Media",
        params: r#"{"media": id}"#,
        doc: "Takes a file out of the project's media. Refused while a clip on any timeline uses it (as its picture or sound, as its LUT, or as a title's font).",
        edits: true,
        enabled: always,
        run: media_remove,
    },
    CommandSpec {
        id: "timeline.list",
        label: "Timelines",
        params: "{}",
        doc: "The project's timelines in order: id, name, length and which one is being edited (the one every other command works on).",
        edits: false,
        enabled: always,
        run: timeline_list,
    },
    CommandSpec {
        id: "timeline.add",
        label: "New Timeline",
        params: r#"{"name"?: string, "copy"?: bool}"#,
        doc: "Adds a timeline after the current one, empty or (`copy`) a copy of the current one, and switches to it. Media, canvas and export settings are shared.",
        edits: true,
        enabled: always,
        run: timeline_add,
    },
    CommandSpec {
        id: "timeline.switch",
        label: "Switch Timeline",
        params: r#"{"timeline": id}"#,
        doc: "Edits another timeline from now on; the current one keeps its tracks.",
        edits: true,
        enabled: always,
        run: timeline_switch,
    },
    CommandSpec {
        id: "timeline.rename",
        label: "Rename Timeline",
        params: r#"{"timeline"?: id, "name": string}"#,
        doc: "Renames a timeline (the current one by default).",
        edits: true,
        enabled: always,
        run: timeline_rename,
    },
    CommandSpec {
        id: "timeline.remove",
        label: "Delete Timeline",
        params: r#"{"timeline"?: id}"#,
        doc: "Deletes a timeline (the current one by default) and its clips; the last one cannot go. Deleting the current one switches to its neighbour.",
        edits: true,
        enabled: always,
        run: timeline_remove,
    },
    CommandSpec {
        id: "timeline.cut",
        label: "Cut Out Time",
        params: r#"{"ranges": [{"from": time, "to": time}], "tracks"?: [id]}"#,
        doc: "Takes stretches of time out of the timeline, on every track that is not locked at once (or only on `tracks`): what lies in a range goes, a clip reaching into one is cut there, and everything after moves up, so picture, sound and captions stay in step. With every track cut, markers and the cover move too. For pauses and filler words (audio.cutPauses and captions.fillers find them).",
        edits: true,
        enabled: always,
        run: crate::cut::timeline_cut,
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
        params: r#"{"track": id, "name"?: string, "hidden"?: bool, "muted"?: bool, "magnetic"?: bool, "locked"?: bool}"#,
        doc: "Renames a track, hides its picture, mutes its sound, makes it magnetic (clips kept end to end from the start, so removing or trimming closes the gap and moving reorders) or locks it (its clips cannot change until it is unlocked).",
        edits: true,
        enabled: always,
        run: track_set,
    },
    CommandSpec {
        id: "clip.add",
        label: "Add Clip",
        params: r#"{"track": id, "start"?: time, "duration"?: time, "media"?: id, "sourceIn"?: time, "sticker"?: id, "x"?, "y"?, "scale"?, "rotation"?, "opacity"?, "volumeDb"?, "name"?: string, "adjustment"?: bool, "timeline"?: id, "fx"?: [{"kind": name, "strength"?: 0-1, "speed"?: 0.25-4} | name], "shape"?: {"kind"?, "size"?: [w, h], "radius"?, "points"?, "inner"?, "fill"?, "stroke"?, "shadow"?, "trim"?}, "text"?: string, "size"?: px, "color"?: [r,g,b,a], "weight"?: 100-900, "align"?: "left"|"center"|"right", "wrap"?: 0-1, "outline"?: {"color": [r,g,b,a], "width": em} | null, "background"?: [r,g,b,a] | null, "radius"?: em, "gradient"?: [r,g,b,a] | null, "outer"?: {"color", "width"} | null, "shadow"?: {"color", "offset": [x, y] em, "blur": em} | null, "lineBoxes"?: bool, "font"?: id | null, "tracking"?: em, "leading"?: em, "italic"?: bool, "underline"?: bool, "template"?: id}"#,
        doc: "Places media, a library sticker, text, a drawn shape (`shape`, on a video track: see clip.setShape), a colour or an adjustment clip on a track; starts at the track's end unless `start` is given. Titles default to bold white with a black outline, wrapping at 90 % of the canvas. A sticker (sticker.library) is an image a third of the frame wide; `name` is what the project's media calls it (its English name by default). An adjustment clip shows nothing: its effects (`fx`) and colour work on the tracks below it while it lasts. `timeline` places another timeline of the project as a compound clip on a video track, from `sourceIn`.",
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
        params: r#"{"clips": id | [id], "ripple"?: bool}"#,
        doc: "Removes clips, leaving gaps; with `ripple`, later clips on the same track move up to close them.",
        edits: true,
        enabled: always,
        run: clip_remove,
    },
    CommandSpec {
        id: "clip.roll",
        label: "Roll",
        params: r#"{"clip": id, "by": time}"#,
        doc: "Moves the cut between a clip and the clip that starts where it ends: one gets longer by `by`, the other shorter, and nothing else moves.",
        edits: true,
        enabled: always,
        run: clip_roll,
    },
    CommandSpec {
        id: "clip.slip",
        label: "Slip",
        params: r#"{"clip": id, "by": time}"#,
        doc: "Shows a later (or, negative, earlier) part of a clip's media in the same place and length on the timeline.",
        edits: true,
        enabled: always,
        run: clip_slip,
    },
    CommandSpec {
        id: "clip.detachAudio",
        label: "Detach Audio",
        params: r#"{"clip": id}"#,
        doc: "Moves a video clip's sound to a clip of its own on an audio track (same source, timing, volume and fades); the video clip plays silent.",
        edits: true,
        enabled: always,
        run: clip_detach_audio,
    },
    CommandSpec {
        id: "track.move",
        label: "Move Track",
        params: r#"{"track": id, "index": number}"#,
        doc: "Moves a track among the tracks of its kind: `index` 0 is the bottom one (drawn first), and so on up.",
        edits: true,
        enabled: always,
        run: track_move,
    },
    CommandSpec {
        id: "track.remove",
        label: "Delete Track",
        params: r#"{"track": id}"#,
        doc: "Removes a track and its clips. The last track of a kind stays.",
        edits: true,
        enabled: always,
        run: track_remove,
    },
    CommandSpec {
        id: "marker.add",
        label: "Add Marker",
        params: r#"{"at": time, "note"?: string}"#,
        doc: "Marks a time on the timeline being edited, to come back to and to snap to.",
        edits: true,
        enabled: always,
        run: marker_add,
    },
    CommandSpec {
        id: "marker.set",
        label: "Marker",
        params: r#"{"marker": id, "at"?: time, "note"?: string}"#,
        doc: "Moves a marker or changes its note.",
        edits: true,
        enabled: always,
        run: marker_set,
    },
    CommandSpec {
        id: "marker.remove",
        label: "Delete Marker",
        params: r#"{"marker": id}"#,
        doc: "Removes a marker.",
        edits: true,
        enabled: always,
        run: marker_remove,
    },
    CommandSpec {
        id: "clip.shift",
        label: "Move",
        params: r#"{"clips": id | [id], "by": time}"#,
        doc: "Moves several clips together by the same time, each on its own track (negative `by` moves them earlier). Refused if any would start before 0 or overlap another clip.",
        edits: true,
        enabled: always,
        run: clip_shift,
    },
    CommandSpec {
        id: "clip.copy",
        label: "Copy",
        params: r#"{"clips": id | [id]}"#,
        doc: "Returns the clips as a clipboard (`{\"clips\": [{\"track\", \"kind\", \"clip\"}]}`) for `clip.paste`. Changes nothing.",
        edits: false,
        enabled: always,
        run: clip_copy,
    },
    CommandSpec {
        id: "clip.paste",
        label: "Paste",
        params: r#"{"clipboard": object, "at": time}"#,
        doc: "Places copies of a clipboard's clips (from `clip.copy`) with new ids, the earliest at `at` and the rest at the same distances. Each goes on its own track when there is room there, else on another track of its kind, else on a new one.",
        edits: true,
        enabled: always,
        run: clip_paste,
    },
    CommandSpec {
        id: "clip.duplicate",
        label: "Duplicate",
        params: r#"{"clips": id | [id]}"#,
        doc: "Copies clips and places the copies right after the last of them, as `clip.paste` would.",
        edits: true,
        enabled: always,
        run: clip_duplicate,
    },
    CommandSpec {
        id: "clip.group",
        label: "Group",
        params: r#"{"clips": [id]}"#,
        doc: "Makes two or more clips a group (剪映's 编组): editors select and move them together. Each clip stays an ordinary clip on its track; a clip is in one group at a time.",
        edits: true,
        enabled: always,
        run: crate::compound::clip_group,
    },
    CommandSpec {
        id: "clip.ungroup",
        label: "Ungroup",
        params: r#"{"clips": id | [id]}"#,
        doc: "Dissolves the groups these clips are in: every clip of those groups is on its own again.",
        edits: true,
        enabled: always,
        run: crate::compound::clip_ungroup,
    },
    CommandSpec {
        id: "clip.compound",
        label: "Compound Clip",
        params: r#"{"clips": id | [id], "name"?: string}"#,
        doc: "Joins clips into one compound clip (剪映's 复合片段): they move to a timeline of their own (listed by timeline.list with `compound`), and one clip that plays it takes their place. Its picture is drawn first and then placed, faded, masked, given effects, animations and transitions as one; its sound plays with it. timeline.switch to that timeline edits what is inside; clip.uncompound puts the clips back.",
        edits: true,
        enabled: always,
        run: crate::compound::clip_compound,
    },
    CommandSpec {
        id: "clip.uncompound",
        label: "Take Apart Compound Clip",
        params: r#"{"clip": id}"#,
        doc: "Puts the clips of a compound clip back on the timeline where it was (the part of them the clip played), on tracks with room or new ones, and removes the compound clip; what was set on the compound clip itself goes with it.",
        edits: true,
        enabled: always,
        run: crate::compound::clip_uncompound,
    },
    CommandSpec {
        id: "clip.setSpeed",
        label: "Speed",
        params: r#"{"clip": id, "speed"?: number, "keepPitch"?: bool, "frames"?: "repeat"|"blend"|"flow"}"#,
        doc: "Changes a media clip's speed; its length changes to keep the same source span. Sound keeps its pitch unless `keepPitch` is false. `frames` is what a video shows between two frames of its file, which a slowed clip has to: the frame before held (`repeat`), the two mixed (`blend`), or a frame made in between by following what moved (`flow`).",
        edits: true,
        enabled: always,
        run: clip_set_speed,
    },
    CommandSpec {
        id: "clip.setReverse",
        label: "Reverse",
        params: r#"{"clip": id, "reversed": bool}"#,
        doc: "Plays a video or sound clip backwards (or forwards again) over the same source span.",
        edits: true,
        enabled: always,
        run: clip_set_reverse,
    },
    CommandSpec {
        id: "clip.setSpeedCurve",
        label: "Speed Curve",
        params: r#"{"clip": id, "curve": "montage"|"hero"|"bullet"|"jumpCut"|"flashIn"|"flashOut"|[[x, m], ...]|null}"#,
        doc: "Shapes a media clip's speed across its length: a preset or points of (fraction of the clip 0-1, multiple of the average speed 0.1-10). The clip keeps its length and source span. null makes the speed steady again.",
        edits: true,
        enabled: always,
        run: clip_set_speed_curve,
    },
    CommandSpec {
        id: "clip.freeze",
        label: "Freeze Frame",
        params: r#"{"clip": id, "at": time, "duration"?: time}"#,
        doc: "Holds the frame a video clip shows at a timeline time for `duration` (3 s by default): the clip is split there, the still goes between, and later clips on the track move along.",
        edits: true,
        enabled: always,
        run: clip_freeze,
    },
    CommandSpec {
        id: "clip.set",
        label: "Clip Properties",
        params: r#"{"clip": id, "x"?, "y"?, "scale"?, "rotation"?, "opacity"?, "volumeDb"?, "at"?: time, "fadeIn"?: time, "fadeOut"?: time, "text"?, "size"?, "color"?, "weight"?, "align"?, "wrap"?, "outline"?: {...} | null, "background"?: color | null, "radius"?: em, "gradient"?: [r,g,b,a] | null, "outer"?: {"color", "width"} | null, "shadow"?: {"color", "offset": [x, y] em, "blur": em} | null, "lineBoxes"?: bool, "font"?: id | null, "tracking"?: em, "leading"?: em, "italic"?: bool, "underline"?: bool, "template"?: id}"#,
        doc: "Sets a clip's placement, opacity, volume, sound fades or title. A property with keyframes is set at `at` (a timeline time), adding or changing the keyframe there.",
        edits: true,
        enabled: always,
        run: clip_set,
    },
    CommandSpec {
        id: "clip.setColor",
        label: "Colour",
        params: r#"{"clip": id, "filter"?: "fresh"|"warm"|"cool"|"mono"|"vintage"|"film"|"vivid"|"dusk"|null, "filterStrength"?: 0-1, "lut"?: media id | null, "lutStrength"?: 0-1, "exposure"?: -3..3, "contrast"?, "saturation"?, "temperature"?, "tint"?, "highlights"?, "shadows"?: -1..1, "fade"?, "vignette"?: 0..1, "reset"?: bool, "at"?: time}"#,
        doc: "Sets a picture clip's colour: a filter at a strength, a lookup table (a `lut` media, from a .cube file) at a strength, and adjustments, applied in that order. `reset` clears all three first.",
        edits: true,
        enabled: always,
        run: clip_set_color,
    },
    CommandSpec {
        id: "clip.setCrop",
        label: "Crop",
        params: r#"{"clip": id, "left"?: 0-1, "top"?: 0-1, "right"?: 0-1, "bottom"?: 0-1, "flipX"?: bool, "flipY"?: bool, "at"?: time}"#,
        doc: "Cuts edges off a video or picture clip (fractions of its width or height) and mirrors it. At least 1 % must stay.",
        edits: true,
        enabled: always,
        run: clip_set_crop,
    },
    CommandSpec {
        id: "clip.setBackdrop",
        label: "Canvas Blur",
        params: r#"{"clip": id, "blur": number | null, "all"?: bool}"#,
        doc: "Fills the frame behind a video or picture clip with a blurred copy of it (剪映's canvas blur): `blur` from 0 (a little) to 1 (very); null removes it. With `all`, every video and picture clip on the clip's track gets the same.",
        edits: true,
        enabled: always,
        run: clip_set_backdrop,
    },
    CommandSpec {
        id: "clip.setChromaKey",
        label: "Chroma Key",
        params: r#"{"clip": id, "color"?: [r, g, b] | null, "tolerance"?: 0-1, "softness"?: 0-1, "spill"?: 0-1}"#,
        doc: "Makes one colour of a video or picture clip see-through, as for a green screen (剪映's 色度抠图), so the tracks below show there. `color` is the colour that goes, as on screen (clip.pickChromaKey finds it from the picture); `tolerance` is how far from it still goes (0.2 at first, less for a dull colour so that greys stay whole), `softness` how gradually the rest comes back (0.1 or less), `spill` how much of the colour's tint comes off what stays near it (0.5). `color` null removes the key.",
        edits: true,
        enabled: always,
        run: clip_set_chroma_key,
    },
    CommandSpec {
        id: "clip.setPersonKey",
        label: "Cut Out People",
        params: r#"{"clip": id, "on"?: bool}"#,
        doc: "Keeps only the people in a video or picture clip and makes the rest see-through (剪映's 智能抠像), so the tracks below show around them; no green screen is needed. A small model bundled with the editor looks at every picture as it is drawn, so nothing is uploaded and nothing has to be analysed first. It is made for a person facing the camera, as in a talking-head shot: outlines are soft, hair and fingers are rough, and in a picture with nobody in it it keeps whatever stands out. `on` false (true when left out) removes it.",
        edits: true,
        enabled: always,
        run: clip_set_person_key,
    },
    CommandSpec {
        id: "clip.setBlend",
        label: "Blend Mode",
        params: r#"{"clip": id, "mode": "normal"|"add"|"multiply"|"screen"|"overlay"|"darken"|"lighten"|"softLight"|"hardLight"|"difference"|"colorDodge"|"colorBurn"}"#,
        doc: "Sets how a picture clip combines with the tracks below it.",
        edits: true,
        enabled: always,
        run: clip_set_blend,
    },
    CommandSpec {
        id: "clip.setMask",
        label: "Mask",
        params: r#"{"clip": id, "shape"?: "linear"|"mirror"|"circle"|"rectangle"|null, "x"?: 0-1, "y"?: 0-1, "width"?, "height"?, "rotation"?: degrees, "feather"?, "invert"?: bool, "at"?: time}"#,
        doc: "Shows only part of a picture clip: inside a shape placed on the picture (centre `x`, `y` as fractions of it; `width`, `height` and `feather` in units of its shorter side). A new shape starts in the middle; null removes the mask.",
        edits: true,
        enabled: always,
        run: clip_set_mask,
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
        id: "clip.setFx",
        label: "Effects",
        params: r#"{"clip": id, "fx": [{"kind": name, "strength"?: 0-1, "speed"?: 0.25-4} | name]}"#,
        doc: "Sets a clip's picture effects (fx.list names them), replacing the ones it has; an empty list removes them. On a picture or title they play on that clip; on an adjustment clip (clip.add with `adjustment`) they play on everything on the tracks below while it lasts, as does its colour (clip.setColor).",
        edits: true,
        enabled: always,
        run: clip_set_fx,
    },
    CommandSpec {
        id: "fx.list",
        label: "Effects",
        params: "{}",
        doc: "Lists the picture effects: what moves the picture (shake, pulse, swing, bounce, slow zooms), what changes its light (flash, strobe, flicker, breathing corners) and what changes its focus or colour (blur in and out, grey to colour).",
        edits: false,
        enabled: always,
        run: fx_list,
    },
    CommandSpec {
        id: "sticker.library",
        label: "Stickers",
        params: "{}",
        doc: "Lists the built-in stickers. clip.add with `sticker` places one.",
        edits: false,
        enabled: always,
        run: sticker_library,
    },
    CommandSpec {
        id: "clip.setVoice",
        label: "Voice",
        params: r#"{"clip": id, "voice"?: "high"|"low"|"chipmunk"|"monster"|"robot"|"phone"|"megaphone"|"echo" | null, "pitch"?: semitones}"#,
        doc: "Changes how a clip's sound sounds: a ready-made voice (voice.list names them; null takes it off) and a pitch moved up to 12 semitones either way without changing its length.",
        edits: true,
        enabled: always,
        run: clip_set_voice,
    },
    CommandSpec {
        id: "clip.setDenoise",
        label: "Reduce Noise",
        params: r#"{"clip": id, "amount": 0-1, "floor"?: [24 numbers]}"#,
        doc: "Takes steady background noise out of a clip's sound: `amount` says how much (0 turns it off), `floor` is the noise, its level in dB in 24 bands. audio.denoise measures the floor from the clip and calls this; without `floor` the one the clip has is kept.",
        edits: true,
        enabled: always,
        run: clip_set_denoise,
    },
    CommandSpec {
        id: "clip.setStabilize",
        label: "Stabilize",
        params: r#"{"clip": id, "amount": 0-1, "path"?: [[across, down, turn]], "from"?: time, "step"?: time}"#,
        doc: "Holds a shaky video clip's picture steady (剪映's 防抖). `amount` says how much of the shake goes: a little keeps the camera's own movement and takes the tremble out, 1 holds the picture nearly still, 0 turns it off. `path` is the camera's path as measured: where the picture is at each sample against the first, as [across, down, turn] in fractions of its width and height and in degrees, the first at source instant `from`, one every `step`. clip.stabilize measures it from the file and calls this; without `path` the clip's own is kept, so `amount` alone changes how much is taken out. The path is smoothed, the picture is moved from where it was to where the smooth path has it, and it is drawn up to 1.3 times larger so that its edges stay out of sight.",
        edits: true,
        enabled: always,
        run: clip_set_stabilize,
    },
    CommandSpec {
        id: "voice.list",
        label: "Voices",
        params: "{}",
        doc: "Lists the ready-made voices for clip.setVoice and how far each moves the pitch.",
        edits: false,
        enabled: always,
        run: voice_list,
    },
    CommandSpec {
        id: "clip.setKey",
        label: "Add Keyframe",
        params: r#"{"clip": id, "prop": "x"|"y"|"scale"|"rotation"|"opacity"|"volumeDb"|"cropLeft"|"cropTop"|"cropRight"|"cropBottom"|"maskX"|"maskY"|"maskWidth"|"maskHeight"|"maskRotation"|"maskFeather"|"exposure"|"contrast"|"saturation"|"temperature"|"tint"|"highlights"|"shadows"|"fade"|"vignette", "at": time, "value"?: number, "ease"?: "linear"|"hold"|"inOut"|"in"|"out"|"overshoot"|"anticipate" | [x1, y1, x2, y2]}"#,
        doc: "Sets a keyframe at a timeline time; without `value`, keeps the value the property has there. Once a property has keyframes, its value follows them: placement, opacity and volume, the crop's edges, the mask's centre, size, turn and soft edge, and the colour adjustments can all move. `ease` is how the value travels on to the next keyframe: a named curve, or one drawn by hand (see clip.setEase).",
        edits: true,
        enabled: always,
        run: clip_set_key,
    },
    CommandSpec {
        id: "clip.setEase",
        label: "Keyframe Curve",
        params: r#"{"clip": id, "at": time, "ease": "linear"|"hold"|"inOut"|"in"|"out"|"overshoot"|"anticipate" | [x1, y1, x2, y2], "props"?: [string]}"#,
        doc: "Sets how values travel from the keyframes at a timeline time (to within half a frame) on to the next ones: every property keyed there, or only `props`. A curve is a name (linear, hold: a jump at the next key, inOut, in: slow away, out: slow to arrive, overshoot: past the next key and back, anticipate: back a little first) or drawn by hand as a cubic Bézier `[x1, y1, x2, y2]` like CSS's cubic-bezier: across is time (each x from 0 to 1), up is how far the value has got (each y from -1 to 2).",
        edits: true,
        enabled: always,
        run: clip_set_ease,
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
        params: r#"{"clip": id, "slot": "in"|"out"|"loop", "kind": "fade"|"slideUp"|"slideDown"|"slideLeft"|"slideRight"|"zoomIn"|"zoomOut"|"pop"|"spin"|"typewriter"|"charFade"|"charRise"|"charDrop"|"charPop" | "breathe"|"heartbeat"|"swing"|"float"|"bounce"|"shake"|"spin"|"blink"|"wave"|"jump" | null, "duration"?: time}"#,
        doc: "Puts an in or out animation on a picture or title clip, or one that repeats for as long as it is on screen (slot `loop`, with the kinds from `breathe` on); null removes it. In and out take 0.5 s unless `duration` is given; for a loop `duration` is one round (1 s or so by default, by kind). For titles only: `typewriter` types the words in or erases them, `charFade`, `charRise`, `charDrop` and `charPop` bring the characters in (or take them out) one after another, and the loops `wave` and `jump` move them one after another.",
        edits: true,
        enabled: always,
        run: clip_set_animation,
    },
    CommandSpec {
        id: "clip.setShape",
        label: "Shape",
        params: r#"{"clip": id, "kind"?: "rectangle"|"ellipse"|"line"|"polygon"|"star"|"tag"|"arrow", "size"?: [w, h] px, "radius"?: px, "points"?: 3-64, "inner"?: 0.05-1, "fill"?: [r,g,b,a] | {"type": "linear", "from", "to", "angle"?} | {"type": "radial", "from", "to"} | null, "stroke"?: {"color", "width": px} | null, "shadow"?: {"color", "offset": [x, y] px, "blur"?: px} | null, "trim"?: [start, end]}"#,
        doc: "Changes a shape clip: its kind, size in canvas pixels (before the clip's scale), corner radius, points, fill (a colour or a gradient), outline, shadow, and the part of its outline drawn (`trim`, also keyable as drawStart and drawEnd; the `draw` animation draws it on).",
        edits: true,
        enabled: always,
        run: crate::promo::clip_set_shape,
    },
    CommandSpec {
        id: "clip.setCounter",
        label: "Counter",
        params: r#"{"clip": id, "counter": {"from"?: number, "to"?: number, "over"?: time | null, "ease"?: name | [x1,y1,x2,y2], "decimals"?: 0-4, "group"?: bool, "clock"?: bool} | null}"#,
        doc: "Makes a title show a number that runs from `from` to `to` over `over` (the whole clip by default; slowing into place unless `ease` says otherwise): written where its words say {n}, or as all of them. `group` puts commas between thousands; `clock` writes whole seconds as 01:30, so from 90 to 0 is a countdown. Keys on the property `count` say the value instead. null removes it.",
        edits: true,
        enabled: always,
        run: crate::promo::clip_set_counter,
    },
    CommandSpec {
        id: "promo.list",
        label: "Promo Pieces",
        params: "{}",
        doc: "The ready-made promotional pieces promo.add makes: their ids, names, default words and numbers.",
        edits: false,
        enabled: always,
        run: crate::promo::promo_list,
    },
    CommandSpec {
        id: "promo.add",
        label: "Add Promo Piece",
        params: r#"{"kind": "priceTag"|"discountBadge"|"banner"|"countdown"|"salesCounter"|"strikePrice"|"newArrival", "start"?: time, "duration"?: time, "text"?: string, "value"?: number, "from"?: number, "color"?: [r,g,b,a], "x"?, "y"?, "scale"?}"#,
        doc: "Adds a ready-made promotional piece (a price tag, a discount badge, a sale banner, a countdown, a sales counter, a struck-out price, a \"new\" stamp) as grouped shape and title clips on free tracks above the others, animated in. `text` replaces its words ({n} is its number), `value` the number it counts to (a countdown counts down from it, a second a second), `from` where counting starts, `color` its main colour; `x`, `y` and `scale` place it. 3 s long from 0 unless `start` and `duration` say otherwise.",
        edits: true,
        enabled: always,
        run: crate::promo::promo_add,
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
        id: "clip.replace",
        label: "Replace",
        params: r#"{"clip": id, "media": id, "sourceIn"?: time, "fit"?: "refuse"|"slow"}"#,
        doc: "Makes a clip show (or play) another file of the project, in the same place for the same length, with everything set on it: effects, animations, keyframes, transition, colour, crop, speed. A picture or a colour on a video track takes a video or an image; a sound takes a sound. A video plays from `sourceIn` (its start by default); one too short for the clip is refused, or with `fit: \"slow\"` slowed down to fill it. What filled the frame goes on filling it: in place of a colour, or of a picture scaled to cover the canvas, a picture of another shape is scaled to cover it.",
        edits: true,
        enabled: always,
        run: crate::remix::clip_replace,
    },
    CommandSpec {
        id: "template.list",
        label: "Templates",
        params: "{}",
        doc: "Lists the built-in timeline templates (剪同款): for each its id, names, what it is for, how many pictures it takes (`slots`), its length in seconds and its music.",
        edits: false,
        enabled: always,
        run: crate::remix::template_list,
    },
    CommandSpec {
        id: "template.apply",
        label: "Use Template",
        params: r#"{"template": id, "media"?: [id | null], "texts"?: [string | null], "name"?: string}"#,
        doc: "Builds a built-in template (template.list): music from the sound library, titles, transitions and effects, and a colour where each picture goes, a slot. It is built on the timeline being edited when that is empty, else on a new timeline called `name` (the template's name by default). `media` fills the slots in order as template.fill does; `texts` are the titles' words in order (null keeps a title's own). One undo step. Returns the slots as template.slots does, with `timeline` and the slots' `clips`.",
        edits: true,
        enabled: always,
        run: crate::remix::template_apply,
    },
    CommandSpec {
        id: "template.slots",
        label: "Template Slots",
        params: "{}",
        doc: "The template slots of the timeline being edited, by number: for each its `clips`, where it starts, the longest it shows, whether it is `filled` and with which `media`; how many are `empty`; and the timeline's titles (`texts`, with clip ids for clip.set).",
        edits: false,
        enabled: always,
        run: crate::remix::template_slots,
    },
    CommandSpec {
        id: "template.fill",
        label: "Fill Template",
        params: r#"{"media": [id | null], "from"?: slot, "fit"?: "slow"|"refuse"}"#,
        doc: "Puts files of the project into the timeline's template slots, in order: into the empty slots, or, with `from`, into every slot from that number on, replacing what is there (null skips a slot). Each slot's clips keep their place, length and everything set on them (clip.replace). A video shorter than its slot is slowed down to fill it unless `fit` is \"refuse\". Returns the slots as template.slots does, the numbers `filled`, and the files `left` over when there were more than slots.",
        edits: true,
        enabled: always,
        run: crate::remix::template_fill,
    },
    CommandSpec {
        id: "template.mark",
        label: "Template Slot",
        params: r#"{"clips": id | [id], "slot"?: number | null}"#,
        doc: "Makes picture clips on video tracks template slots, so the timeline can be used again with other files (template.fill): each gets the next free number, in the order they play, or all get `slot` (clips with one number take the same file). null makes them ordinary clips again. What a clip shows does not change.",
        edits: true,
        enabled: always,
        run: crate::remix::template_mark,
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
        doc: "Runs several commands as one undo step; if any fails, none applies. A parameter written `${n.field}` is what step n (from 0) replied there, such as `${0.clip}` for the clip step 0 added, or `${1.clips.0.clip}`.",
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

fn sound_library(_: &mut Session, _: &Value) -> CmdResult {
    use reelforge_media::library::{Category, SOUNDS};
    let list: Vec<Value> = SOUNDS
        .iter()
        .map(|snd| {
            json!({
                "id": snd.id,
                "kind": match snd.category { Category::Effect => "effect", Category::Music => "music" },
                "name": snd.en,
                "nameZh": snd.zh,
            })
        })
        .collect();
    Ok(json!({ "sounds": list }))
}

fn sound_add(s: &mut Session, v: &Value) -> CmdResult {
    use reelforge_media::library;
    let p = P::new(v, fps(s));
    let id = p.str("sound")?;
    let snd = library::find(id).ok_or_else(|| {
        EngineError::Params(format!(
            "`sound` must be one of {}",
            library::SOUNDS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")
        ))
    })?;
    let (at, track, name) = (p.opt_time("at")?, p.opt_str("track")?.map(str::to_string), p.opt_str("name")?);
    if at.is_some_and(|t| t < Tick::ZERO) {
        return Err(EngineError::Params("`at` must not be negative".into()));
    }
    let uri = format!("{}{}", library::SCHEME, snd.id);
    let name = name.unwrap_or(snd.en).to_string();
    // made once here for its hash and length; the same bytes wherever it plays
    let known = s.project().media.values().find(|m| m.uri == uri).map(|m| m.id.clone());
    let made = match known {
        Some(_) => None,
        None => {
            use sha2::{Digest, Sha256};
            let bytes = library::wav(snd.id).expect("a library sound");
            let frames = (bytes.len() - 44) / 4;
            let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            Some((sha, Tick::from_seconds(frames as f64 / f64::from(library::RATE))))
        }
    };
    s.edit("Add Sound", |proj| {
        let media = match (known, made) {
            (Some(m), _) => m,
            (None, Some((sha, duration))) => {
                let m = proj.mint("m");
                let item = Media {
                    id: m.clone(),
                    name,
                    uri,
                    sha256: Some(sha),
                    kind: MediaKind::Audio,
                    duration: Some(duration),
                    size: None,
                    folder: None,
                };
                proj.media.insert(m.clone(), item);
                m
            }
            (None, None) => unreachable!("made when not known"),
        };
        let duration = proj.media[&media].duration.expect("library sounds have a length");
        let free = |t: &Track, start: Tick| t.clips.iter().all(|c| c.end() <= start || c.start >= start + duration);
        let (track, start) = match track {
            Some(t) => {
                let tr = proj.track(&t).ok_or_else(|| not_found("track", &t))?;
                if tr.kind != TrackKind::Audio {
                    return Err(EngineError::Params(format!("{t} is not an audio track")));
                }
                let start = at.unwrap_or_else(|| tr.clips.last().map_or(Tick::ZERO, Clip::end));
                (t, start)
            }
            // the first audio track with room, else a new one
            None => {
                let room = proj.tracks.iter().filter(|t| t.kind == TrackKind::Audio).find_map(|t| {
                    let start = at.unwrap_or_else(|| t.clips.last().map_or(Tick::ZERO, Clip::end));
                    free(t, start).then(|| (t.id.clone(), start))
                });
                match room {
                    Some(found) => found,
                    None => {
                        let t = proj.mint("t");
                        let n = proj.tracks.iter().filter(|t| t.kind == TrackKind::Audio).count() + 1;
                        proj.tracks.push(Track {
                            id: t.clone(),
                            kind: TrackKind::Audio,
                            name: format!("Audio {n}"),
                            hidden: false,
                            muted: false,
                            magnetic: false,
                            locked: false,
                            clips: vec![],
                        });
                        (t, at.unwrap_or(Tick::ZERO))
                    }
                }
            }
        };
        let clip = proj.mint("c");
        let c = Clip {
            id: clip.clone(),
            start,
            duration,
            content: ClipContent::media(media.clone(), Tick::ZERO),
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
        };
        proj.track_mut(&track).expect("found above").clips.push(c);
        Ok(json!({ "media": media, "track": track, "clip": clip, "start": start, "duration": duration }))
    })
}

fn project_inspect(s: &mut Session, _: &Value) -> CmdResult {
    let p = s.project();
    Ok(
        json!({ "revision": s.revision, "duration": p.duration(), "project": serde_json::to_value(&*p).expect("serialises") }),
    )
}

fn project_new(s: &mut Session, _: &Value) -> CmdResult {
    s.edit("New Project", |p| {
        p.media.clear();
        p.timelines.clear();
        p.timeline.clear();
        p.markers.clear();
        p.cover = None;
        // ids keep counting up, so nothing still pointing at an old track finds a new one
        p.tracks = model_timeline::empty_tracks(p);
        Ok(())
    })?;
    Ok(json!({ "tracks": s.project().tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>() }))
}

fn project_set_cover(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    if v.get("at").is_none() {
        return Err(EngineError::Params("`at` is required: a time, or null for no cover".into()));
    }
    let at = p.opt_time("at")?;
    if at.is_some_and(|t| t < Tick::ZERO) {
        return Err(EngineError::Params("`at` cannot be before the start".into()));
    }
    s.edit("Cover", |proj| {
        if at.is_some_and(|t| t >= proj.duration().max(Tick(1))) {
            return Err(EngineError::Refused("the cover must be a frame of the timeline: `at` is past its end".into()));
        }
        // a cover is a frame: the one `at` is in
        proj.cover = at.map(|t| proj.canvas.fps.snap(t));
        Ok(json!({ "cover": proj.cover }))
    })
}

fn project_set_export(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    fn pick<T>(p: &P, name: &str, parse: fn(&str) -> Option<T>, ids: &[&str]) -> Result<Option<T>, EngineError> {
        match p.opt_str(name)? {
            None => Ok(None),
            Some(x) => parse(x)
                .map(Some)
                .ok_or_else(|| EngineError::Params(format!("`{name}` must be one of {}", ids.join(", ")))),
        }
    }
    let presets: Vec<&str> = export::PRESETS.iter().map(|(id, _)| *id).collect();
    let preset = pick(&p, "preset", ExportSettings::of_preset, &presets)?;
    let resolution = pick(&p, "resolution", Resolution::parse, &Resolution::ALL.map(Resolution::name))?;
    let quality = pick(&p, "quality", Quality::parse, &Quality::ALL.map(Quality::name))?;
    let codec = pick(&p, "codec", Codec::parse, &Codec::ALL.map(Codec::name))?;
    let format = pick(&p, "format", export::Format::parse, &export::Format::ALL.map(export::Format::name))?;
    let rate = match p.opt_u32("fps")? {
        Some(r) if r != 0 && !export::FRAME_RATES.contains(&r) => {
            return Err(EngineError::Params(format!("`fps` must be 0 or one of {:?}", export::FRAME_RATES)));
        }
        r => r,
    };
    s.edit("Export Settings", |proj| {
        let e = &mut proj.export;
        if let Some(p) = preset {
            *e = p;
        }
        e.resolution = resolution.unwrap_or(e.resolution);
        e.quality = quality.unwrap_or(e.quality);
        e.codec = codec.unwrap_or(e.codec);
        e.fps = rate.unwrap_or(e.fps);
        e.format = format.unwrap_or(e.format);
        let c = &proj.canvas;
        let [w, h] = e.size(c);
        Ok(json!({
            "export": serde_json::to_value(*e).expect("serialises"),
            "preset": e.preset(),
            "format": e.format.name(),
            "width": w,
            "height": h,
            "fps": e.frame_rate(c).fps(),
            "bitrateKbps": e.bitrate_kbps(c),
        }))
    })
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
        "cube" => Some(MediaKind::Lut),
        "ttf" | "otf" | "ttc" => Some(MediaKind::Font),
        _ => None,
    }
}

fn media_import(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let path = p.str("path")?.to_string();
    let asked: Option<MediaKind> = match p.opt_str("kind")? {
        Some(k) => Some(
            serde_json::from_value(json!(k))
                .map_err(|_| EngineError::Params("`kind` must be video, image, audio, lut or font".into()))?,
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
    // a font is called by its family's name unless it is given another
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut family: Option<String> = None;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use sha2::{Digest, Sha256};
        let local = std::path::Path::new(path.strip_prefix("file://").unwrap_or(&path));
        let wanted = asked.or(kind_from_path(&path));
        if (sha256.is_none() || wanted == Some(MediaKind::Font))
            && let Ok(bytes) = std::fs::read(local)
        {
            let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            if wanted == Some(MediaKind::Font) {
                family = Some(
                    reelforge_media::font_name(&hash, &bytes)
                        .map_err(|e| EngineError::Refused(format!("cannot read {path}: {e}")))?,
                );
            }
            sha256 = sha256.or(Some(hash));
        }
        // a lookup table is text, read when a frame needs it; a font was read just now
        if local.is_file() && !matches!(wanted, Some(MediaKind::Lut | MediaKind::Font)) {
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
            AssetKind::Lut => MediaKind::Lut,
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
        .or(family)
        .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string());
    let base = |u: &str| u.rsplit(['/', '\\']).next().unwrap_or(u).to_ascii_lowercase();
    s.edit("Import Media", |proj| {
        // a file that was offline (from a CapCut draft, say) is relinked rather than added again
        let file = base(&path);
        if sha256.is_some()
            && let Some(m) = proj
                .media
                .values_mut()
                .find(|m| m.sha256.is_none() && m.kind == kind && (base(&m.uri) == file || base(&m.name) == file))
        {
            m.uri = path;
            m.sha256 = sha256;
            m.duration = duration.or(m.duration);
            m.size = size.or(m.size);
            return Ok(json!({ "media": serde_json::to_value(&*m).expect("serialises"), "relinked": true }));
        }
        // a font the project has is not added twice: titles pick it by name
        if kind == MediaKind::Font
            && sha256.is_some()
            && let Some(m) = proj.media.values().find(|m| m.kind == kind && m.sha256 == sha256)
        {
            return Ok(json!({ "media": serde_json::to_value(m).expect("serialises") }));
        }
        let id = proj.mint("m");
        let m = Media { id: id.clone(), name, uri: path, sha256, kind, duration, size, folder: None };
        let out = json!({ "media": serde_json::to_value(&m).expect("serialises") });
        proj.media.insert(id, m);
        Ok(out)
    })
}

/// How many clips, on every timeline, use media `id` (as their source or their LUT).
pub fn media_uses(proj: &Project, id: &str) -> usize {
    let parked = proj.timelines.iter().flat_map(|t| &t.tracks);
    proj.tracks
        .iter()
        .chain(parked)
        .flat_map(|t| &t.clips)
        .filter(|c| {
            matches!(&c.content, ClipContent::Media { media, .. } if media == id)
                || matches!(&c.content, ClipContent::Text { style, .. } if style.font.as_deref() == Some(id))
                || c.color.lut.as_ref().is_some_and(|l| l.media == id)
        })
        .count()
}

fn media_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids: Vec<String> = match p.opt("media") {
        Some(Value::String(id)) => vec![id.clone()],
        Some(list) => serde_json::from_value(list.clone())
            .map_err(|_| EngineError::Params("`media` must be an id or a list of ids".into()))?,
        None => return Err(EngineError::Params("`media` is required".into())),
    };
    if ids.is_empty() {
        return Err(EngineError::Params("`media` must name at least one file".into()));
    }
    let name = p.opt_str("name")?.map(|n| n.trim().to_string());
    if name.as_deref().is_some_and(str::is_empty) {
        return Err(EngineError::Params("`name` must not be empty".into()));
    }
    if name.is_some() && ids.len() > 1 {
        return Err(EngineError::Params("`name` renames one file: give one id".into()));
    }
    // null takes the files out of their folder; leaving `folder` out keeps it
    let folder: Option<Option<String>> = match v.get("folder") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(f)) => {
            let f = f.trim();
            if f.is_empty() || f.chars().count() > Media::FOLDER_NAME_MAX {
                return Err(EngineError::Params(format!(
                    "`folder` is a name of 1 to {} characters, or null",
                    Media::FOLDER_NAME_MAX
                )));
            }
            Some(Some(f.to_string()))
        }
        Some(_) => return Err(EngineError::Params("`folder` must be a name or null".into())),
    };
    if name.is_none() && folder.is_none() {
        return Err(EngineError::Params("give a `name`, a `folder`, or both".into()));
    }
    s.edit("Media Name and Folder", |proj| {
        for id in &ids {
            let m = proj.media.get_mut(id).ok_or_else(|| EngineError::Refused(format!("no media {id:?}")))?;
            if let Some(n) = &name {
                m.name = n.clone();
            }
            if let Some(f) = &folder {
                m.folder = f.clone();
            }
        }
        let mut folders: Vec<&str> = proj.media.values().filter_map(|m| m.folder.as_deref()).collect();
        folders.sort();
        folders.dedup();
        Ok(json!({ "media": ids, "folders": folders }))
    })
}

fn media_remove(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("media")?.to_string();
    s.edit("Remove Media", |proj| {
        if !proj.media.contains_key(&id) {
            return Err(EngineError::Refused(format!("no media {id:?}")));
        }
        match media_uses(proj, &id) {
            0 => {}
            n => return Err(EngineError::Refused(format!("{n} clip(s) use {id:?}; delete them first"))),
        }
        proj.media.remove(&id);
        Ok(json!({ "media": id }))
    })
}

fn track_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("track")?.to_string();
    let (name, hidden, muted) = (p.opt_str("name")?.map(str::to_string), p.opt_bool("hidden")?, p.opt_bool("muted")?);
    let magnetic = p.opt_bool("magnetic")?;
    let locked = p.opt_bool("locked")?;
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
        t.magnetic = magnetic.unwrap_or(t.magnetic);
        t.locked = locked.unwrap_or(t.locked);
        Ok(json!({ "track": id, "name": t.name, "hidden": t.hidden, "muted": t.muted, "magnetic": t.magnetic, "locked": t.locked }))
    })
}

fn timeline_json(p: &Project) -> Value {
    let list: Vec<Value> = p
        .timeline_list()
        .into_iter()
        .map(|t| {
            let mut v = json!({ "id": t.id, "name": t.name, "duration": t.duration, "current": t.current });
            if t.compound {
                v["compound"] = json!(true);
            }
            v
        })
        .collect();
    json!({ "current": p.timeline, "timelines": list })
}

fn timeline_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(timeline_json(&s.project()))
}

/// The timeline a command names, or the current one.
fn which_timeline(proj: &Project, p: &P) -> Result<String, EngineError> {
    let id = p.opt_str("timeline")?.map_or_else(|| proj.timeline.clone(), str::to_string);
    if proj.timelines.iter().any(|t| t.id == id) || (proj.timelines.is_empty() && id.is_empty()) {
        Ok(id)
    } else {
        Err(EngineError::Params(format!("no timeline {id:?}")))
    }
}

fn timeline_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let name = p.opt_str("name")?.map(str::to_string);
    let copy = p.opt_bool("copy")?.unwrap_or(false);
    s.edit("New Timeline", |proj| {
        proj.name_first_timeline("Timeline 1");
        let tracks = if copy { proj.copy_tracks() } else { model_timeline::empty_tracks(proj) };
        let id = proj.mint("s");
        let name = name.unwrap_or_else(|| {
            let current = proj.timelines.iter().find(|t| t.id == proj.timeline).map(|t| t.name.clone());
            match (copy, current) {
                (true, Some(n)) => format!("{n} copy"),
                _ => format!("Timeline {}", proj.timelines.len() + 1),
            }
        });
        let at = proj.timelines.iter().position(|t| t.id == proj.timeline).map_or(proj.timelines.len(), |i| i + 1);
        let markers = if copy {
            let mut m = proj.markers.clone();
            for k in &mut m {
                k.id = proj.mint("mk");
            }
            m
        } else {
            vec![]
        };
        let cover = proj.cover.filter(|_| copy);
        proj.timelines.insert(at, Timeline { id: id.clone(), name, tracks, markers, cover, compound: false });
        proj.switch_timeline(&id);
        Ok(json!({ "timeline": id }))
    })
}

fn timeline_switch(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("timeline")?.to_string();
    s.edit("Switch Timeline", |proj| {
        if proj.switch_timeline(&id) {
            Ok(json!({ "timeline": id }))
        } else {
            Err(EngineError::Params(format!("no timeline {id:?}")))
        }
    })
}

fn timeline_rename(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let name = p.str("name")?.trim().to_string();
    if name.is_empty() {
        return Err(EngineError::Params("`name` cannot be empty".into()));
    }
    s.edit("Rename Timeline", |proj| {
        let mut id = which_timeline(proj, &p)?;
        if id.is_empty() {
            proj.name_first_timeline(&name);
            id = proj.timeline.clone();
        }
        if let Some(t) = proj.timelines.iter_mut().find(|t| t.id == id) {
            t.name = name;
        }
        Ok(json!({ "timeline": id }))
    })
}

fn timeline_remove(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    s.edit("Delete Timeline", |proj| {
        let id = which_timeline(proj, &p)?;
        if proj.timelines.len() < 2 {
            return Err(EngineError::Refused("a project keeps at least one timeline".into()));
        }
        if let Some(clip) = proj.compounds_of(&id).first() {
            return Err(EngineError::Refused(format!(
                "compound clip {:?} plays this timeline; take it apart (clip.uncompound) or remove it first",
                clip.id
            )));
        }
        let at = proj.timelines.iter().position(|t| t.id == id).expect("checked above");
        if id == proj.timeline {
            let next = proj.timelines[if at + 1 < proj.timelines.len() { at + 1 } else { at - 1 }].id.clone();
            proj.switch_timeline(&next);
        }
        proj.timelines.remove(at);
        Ok(json!({ "timeline": proj.timeline }))
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
        proj.tracks.insert(
            at,
            Track {
                id: id.clone(),
                kind,
                name,
                hidden: false,
                muted: false,
                magnetic: false,
                locked: false,
                clips: vec![],
            },
        );
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
    let fx = fx_param(&p)?.unwrap_or_default();
    let adjustment = p.opt_bool("adjustment")?.unwrap_or(false);
    let timeline = p.opt_str("timeline")?.map(str::to_string);
    // placed as it lands, so a title or sticker needs no clip.set after
    let place = |name: &str| p.opt_f64(name).map(|o| o.map(|x| x as f32));
    let (x, y, scale, rotation) = (place("x")?, place("y")?, place("scale")?, place("rotation")?);
    let (opacity, volume_db) = (place("opacity")?, place("volumeDb")?);
    if opacity.is_some_and(|o| !(0.0..=1.0).contains(&o)) {
        return Err(EngineError::Params("`opacity` must be between 0 and 1".into()));
    }
    let shape = p.opt("shape").map(crate::promo::shape_param).transpose()?;
    let sticker = match p.opt_str("sticker")? {
        None => None,
        Some(id) => {
            use reelforge_media::stickers;
            let st = stickers::find(id).ok_or_else(|| {
                EngineError::Params(format!(
                    "`sticker` must be one of {}",
                    stickers::STICKERS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")
                ))
            })?;
            let uri = format!("{}{}", stickers::PREFIX, st.id);
            // drawn once here for its hash; the same picture wherever it shows
            let sha = match s.project().media.values().any(|m| m.uri == uri) {
                true => None,
                false => {
                    use sha2::{Digest, Sha256};
                    let px = stickers::render(st.id, stickers::SIZE).expect("a library sticker");
                    Some(Sha256::digest(&px).iter().map(|b| format!("{b:02x}")).collect::<String>())
                }
            };
            Some((uri, p.opt_str("name")?.unwrap_or(st.en).to_string(), sha))
        }
    };
    if [media.is_some(), text.is_some(), sticker.is_some(), adjustment, timeline.is_some(), shape.is_some()]
        .iter()
        .filter(|x| **x)
        .count()
        > 1
    {
        return Err(EngineError::Params(
            "give one of `media`, `sticker`, `text`, `shape`, `adjustment` or `timeline`".into(),
        ));
    }
    s.edit("Add Clip", |proj| {
        let track = proj.track(&track_id).ok_or_else(|| not_found("track", &track_id))?;
        let start = start.unwrap_or_else(|| track.clips.last().map_or(Tick::ZERO, Clip::end));
        let media = match &sticker {
            None => media.clone(),
            Some((uri, name, sha)) => Some(match proj.media.values().find(|m| m.uri == *uri) {
                Some(m) => m.id.clone(),
                None => {
                    let m = proj.mint("m");
                    let side = reelforge_media::stickers::SIZE;
                    let item = Media {
                        id: m.clone(),
                        name: name.clone(),
                        uri: uri.clone(),
                        sha256: sha.clone(),
                        kind: MediaKind::Image,
                        duration: None,
                        size: Some([side, side]),
                        folder: None,
                    };
                    proj.media.insert(m.clone(), item);
                    m
                }
            }),
        };
        let (content, natural) = match (&media, &text) {
            _ if adjustment => (ClipContent::Adjustment, None),
            _ if shape.is_some() => (ClipContent::Shape { shape: shape.clone().expect("checked") }, None),
            _ if timeline.is_some() => {
                let id = timeline.clone().expect("checked");
                let inner = proj.tracks_of(&id).filter(|_| !id.is_empty()).ok_or_else(|| not_found("timeline", &id))?;
                if id == proj.timeline || proj.timeline_reaches(&id, &proj.timeline) {
                    return Err(EngineError::Refused("a timeline cannot play inside itself".into()));
                }
                let natural = Some(model_timeline::tracks_duration(inner) - source_in).filter(|d| *d > Tick::ZERO);
                (ClipContent::Compound { timeline: id, source_in }, natural)
            }
            (Some(m), None) => {
                let item = proj.media.get(m).ok_or_else(|| not_found("media", m))?;
                let natural = item.duration.map(|d| d - source_in).filter(|d| *d > Tick::ZERO);
                (ClipContent::media(m.clone(), source_in), natural)
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
            // a sticker starts a third of the frame wide
            transform: ClipTransform {
                x: x.unwrap_or(0.0),
                y: y.unwrap_or(0.0),
                scale: scale.unwrap_or(if sticker.is_some() { 0.35 } else { 1.0 }),
                rotation: rotation.unwrap_or(0.0),
            },
            opacity: opacity.unwrap_or(1.0),
            volume_db: volume_db.unwrap_or(0.0),
            effects: vec![],
            keys: Default::default(),
            anim: Default::default(),
            transition: None,
            fade: Default::default(),
            keep_pitch: true,
            color: Default::default(),
            look: Default::default(),
            fx,
            voice: Default::default(),
            group: None,
            slot: None,
            steady: None,
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
        let right = split(proj, &id, at)?;
        Ok(json!({ "left": id, "right": right }))
    })
}

/// Cuts clip `id` at timeline time `at`, strictly inside it; returns the right half's id.
pub(crate) fn split(proj: &mut Project, id: &str, at: Tick) -> Result<String, EngineError> {
    let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    if at <= clip.start || at >= clip.end() {
        return Err(EngineError::Params("`at` must be strictly inside the clip".into()));
    }
    let mut right = clip.clone();
    let left_len = at - clip.start;
    // where each half starts in the source, and how fast each runs so that together they play
    // exactly as the whole did
    let halves = match (&clip.content, clip.timing()) {
        (ClipContent::Media { source_in, speed, reversed, curve, .. }, Some(t)) => {
            let f = left_len.0 as f64 / clip.duration.0 as f64;
            let travel = Tick(t.travel(left_len).0.abs());
            let span = clip.duration.scale(*speed);
            let whole = timing::curve_mean(curve, 0.0, 1.0);
            let part = |a: f64, b: f64| {
                if curve.is_empty() {
                    (*speed, vec![])
                } else {
                    (*speed * timing::curve_mean(curve, a, b) / whole, timing::curve_part(curve, a, b))
                }
            };
            let (left, right) = (part(0.0, f), part(f, 1.0));
            // backwards, the left half plays the end of the span
            let (lin, rin) =
                if *reversed { (*source_in + span - travel, *source_in) } else { (*source_in, *source_in + travel) };
            Some(((lin, left), (rin, right)))
        }
        _ => None,
    };
    // a compound clip's right half goes on from where the left stops in its timeline
    if let ClipContent::Compound { source_in, .. } = &mut right.content {
        *source_in += left_len;
    }
    right.start = at;
    right.duration = clip.end() - at;
    right.id = proj.mint("c");
    // keys stay where they were on the timeline; the in animation stays on the left half and
    // the out animation on the right
    right.shift_keys(-left_len);
    right.anim.enter = None;
    right.transition = None;
    right.fade.fade_in = Tick::ZERO;
    let set = |c: &mut Clip, (from, (v, pts)): (Tick, (f64, Vec<CurvePoint>))| {
        if let ClipContent::Media { source_in, speed, curve, .. } = &mut c.content {
            (*source_in, *speed, *curve) = (from, v, pts);
        }
    };
    let halves = halves.map(|(l, r)| {
        set(&mut right, r);
        l
    });
    let right_id = right.id.clone();
    proj.tracks[ti].clips.push(right);
    let left = proj.clip_mut(id).expect("exists");
    left.duration = left_len;
    left.anim.exit = None;
    left.fade.fade_out = Tick::ZERO;
    if let Some(l) = halves {
        set(left, l);
    }
    Ok(right_id)
}

fn clip_trim(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let (new_start, new_end) = (p.opt_time("start")?, p.opt_time("end")?);
    s.edit("Trim", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let start = new_start.unwrap_or(clip.start);
        let end = new_end.unwrap_or(clip.end());
        trim(clip, start, end)?;
        Ok(json!({ "clip": id, "start": start, "duration": end - start }))
    })
}

/// Moves a clip's edges to `start` and `end`, keeping its media in sync.
pub(crate) fn trim(clip: &mut Clip, start: Tick, end: Tick) -> Result<(), EngineError> {
    if end <= start {
        return Err(EngineError::Params("the clip would have no length".into()));
    }
    let delta = start - clip.start;
    let old_end = clip.end();
    if let ClipContent::Media { source_in, speed, reversed, .. } = &mut clip.content {
        // the source runs from the clip's start, or backwards from its end
        let moved =
            if *reversed { *source_in - (end - old_end).scale(*speed) } else { *source_in + delta.scale(*speed) };
        if moved < Tick::ZERO {
            return Err(EngineError::Refused("the clip cannot start before its media does".into()));
        }
        *source_in = moved;
    }
    if let ClipContent::Compound { source_in, .. } = &mut clip.content {
        if *source_in + delta < Tick::ZERO {
            return Err(EngineError::Refused("the clip cannot start before its timeline does".into()));
        }
        *source_in += delta;
    }
    clip.shift_keys(-delta);
    clip.start = start;
    clip.duration = end - start;
    Ok(())
}

fn clip_roll(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let by = p.time("by")?;
    s.edit("Roll", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let end = clip.end();
        let next = proj.tracks[ti]
            .clips
            .iter()
            .find(|c| c.start == end)
            .map(|c| c.id.clone())
            .ok_or_else(|| EngineError::Refused("no clip starts where this one ends".into()))?;
        let cut = end + by;
        let a = proj.clip_mut(&id).expect("found");
        trim(a, a.start, cut)?;
        let b = proj.clip_mut(&next).expect("found");
        trim(b, cut, b.end())?;
        Ok(json!({ "clip": id, "next": next, "cut": cut }))
    })
}

fn clip_slip(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let by = p.time("by")?;
    s.edit("Slip", |proj| {
        let (_, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let ClipContent::Media { media, .. } = &clip.content else {
            return Err(EngineError::Params("only video and sound clips slip".into()));
        };
        let length = proj.media.get(media).and_then(|m| m.duration);
        let span = clip.timing().map(|t| Tick(t.travel(clip.duration).0.abs())).unwrap_or(Tick::ZERO);
        let clip = proj.clip_mut(&id).expect("found");
        let ClipContent::Media { source_in, speed, .. } = &mut clip.content else { unreachable!() };
        if *speed == 0.0 {
            return Err(EngineError::Refused("a freeze frame has nothing to slip".into()));
        }
        let moved = *source_in + by.scale(*speed);
        if moved < Tick::ZERO || length.is_some_and(|l| moved + span > l) {
            return Err(EngineError::Refused("the clip would run past its media".into()));
        }
        *source_in = moved;
        Ok(json!({ "clip": id, "sourceIn": moved }))
    })
}

fn clip_detach_audio(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    s.edit("Detach Audio", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let video = matches!(&clip.content, ClipContent::Media { media, silent: false, .. }
            if proj.media.get(media).is_some_and(|m| m.kind == MediaKind::Video));
        if proj.tracks[ti].kind != TrackKind::Video || !video {
            return Err(EngineError::Params("only a video clip with its sound can have it detached".into()));
        }
        // the sound as a clip of its own: same source, timing, volume and fades
        let mut sound = clip.clone();
        sound.id = proj.mint("c");
        sound.transform = ClipTransform::default();
        sound.opacity = 1.0;
        sound.effects.clear();
        sound.keys.retain(|prop, _| *prop == Prop::VolumeDb);
        sound.anim = Default::default();
        sound.transition = None;
        sound.color = Default::default();
        sound.look = Default::default();
        sound.fx.clear();
        let range = sound.range();
        let track = match proj
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Audio && !t.clips.iter().any(|c| c.range().overlaps(range)))
        {
            Some(i) => i,
            None => {
                let tid = proj.mint("t");
                let n = proj.tracks.iter().filter(|t| t.kind == TrackKind::Audio).count() + 1;
                proj.tracks.push(Track {
                    id: tid,
                    kind: TrackKind::Audio,
                    name: format!("Audio {n}"),
                    hidden: false,
                    muted: false,
                    magnetic: false,
                    locked: false,
                    clips: vec![],
                });
                proj.tracks.len() - 1
            }
        };
        let (sid, tid) = (sound.id.clone(), proj.tracks[track].id.clone());
        proj.tracks[track].clips.push(sound);
        let picture = proj.clip_mut(&id).expect("found");
        if let ClipContent::Media { silent, .. } = &mut picture.content {
            *silent = true;
        }
        // its voice went with its sound
        picture.voice = Voice::default();
        Ok(json!({ "clip": id, "audio": sid, "track": tid }))
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
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    let ripple = p.opt_bool("ripple")?.unwrap_or(false);
    s.edit("Delete", |proj| {
        let mut gone = Vec::new();
        for id in &ids {
            let (ti, c) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            gone.push((ti, c.start, c.duration));
            proj.tracks[ti].clips.retain(|c| &c.id != id);
        }
        if ripple {
            // latest first, so each gap closes from where it was
            gone.sort_by_key(|g| std::cmp::Reverse(g.1));
            for (ti, start, len) in gone {
                for c in proj.tracks[ti].clips.iter_mut().filter(|c| c.start >= start) {
                    c.start -= len;
                }
            }
        }
        crate::compound::tidy_groups(proj);
        Ok(json!({ "removed": ids }))
    })
}

fn track_move(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("track")?.to_string();
    let index = p.opt_u32("index")?.ok_or_else(|| EngineError::Params("missing `index`".into()))? as usize;
    s.edit("Move Track", |proj| {
        let from = proj.tracks.iter().position(|t| t.id == id).ok_or_else(|| not_found("track", &id))?;
        let kind = proj.tracks[from].kind;
        let track = proj.tracks.remove(from);
        // the places of its kind, bottom first, without it
        let slots: Vec<usize> = (0..proj.tracks.len()).filter(|&i| proj.tracks[i].kind == kind).collect();
        let at = match slots.get(index) {
            Some(&i) => i,
            None => slots.last().map_or(from.min(proj.tracks.len()), |&i| i + 1),
        };
        proj.tracks.insert(at, track);
        let index = (0..=at).filter(|&i| proj.tracks[i].kind == kind).count() - 1;
        Ok(json!({ "track": id, "index": index }))
    })
}

fn track_remove(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("track")?.to_string();
    s.edit("Delete Track", |proj| {
        let at = proj.tracks.iter().position(|t| t.id == id).ok_or_else(|| not_found("track", &id))?;
        let kind = proj.tracks[at].kind;
        if proj.tracks.iter().filter(|t| t.kind == kind).count() == 1 {
            return Err(EngineError::Refused("the last track of a kind stays".into()));
        }
        if proj.tracks[at].locked && !proj.tracks[at].clips.is_empty() {
            return Err(EngineError::Refused(format!("track {:?} is locked", proj.tracks[at].name)));
        }
        proj.tracks.remove(at);
        Ok(json!({ "removed": id }))
    })
}

fn marker_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let at = p.time("at")?;
    let note = p.opt_str("note")?.unwrap_or_default().to_string();
    if at < Tick::ZERO {
        return Err(EngineError::Params("`at` must not be negative".into()));
    }
    let at = fps(s).snap(at);
    s.edit("Add Marker", |proj| {
        let id = proj.mint("mk");
        proj.markers.push(reelforge_model::Marker { id: id.clone(), at, note });
        Ok(json!({ "marker": id, "at": at }))
    })
}

fn marker_set(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("marker")?.to_string();
    let at = p.opt_time("at")?.map(|t| fps(s).snap(t.max(Tick::ZERO)));
    let note = p.opt_str("note")?.map(str::to_string);
    s.edit("Marker", |proj| {
        let m = proj.markers.iter_mut().find(|m| m.id == id).ok_or_else(|| not_found("marker", &id))?;
        m.at = at.unwrap_or(m.at);
        if let Some(n) = note {
            m.note = n;
        }
        Ok(json!({ "marker": id, "at": m.at, "note": m.note }))
    })
}

fn marker_remove(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("marker")?.to_string();
    s.edit("Delete Marker", |proj| {
        let before = proj.markers.len();
        proj.markers.retain(|m| m.id != id);
        if proj.markers.len() == before {
            return Err(not_found("marker", &id));
        }
        Ok(json!({ "removed": id }))
    })
}

fn clip_shift(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    let by = p.time("by")?;
    s.edit("Move", |proj| {
        for id in &ids {
            let clip = proj.clip_mut(id).ok_or_else(|| not_found("clip", id))?;
            clip.start += by;
            if clip.start < Tick::ZERO {
                return Err(EngineError::Refused("a clip would start before the timeline does".into()));
            }
        }
        Ok(json!({ "clips": ids, "by": by }))
    })
}

/// The clipboard of `clip.copy`: each clip with the track it was on and that track's kind.
fn clipboard(proj: &Project, ids: &[String]) -> Result<Value, EngineError> {
    let mut clips = Vec::new();
    for id in ids {
        let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
        let t = &proj.tracks[ti];
        clips.push(json!({ "track": t.id, "kind": t.kind, "clip": clip }));
    }
    Ok(json!({ "clips": clips }))
}

fn clip_copy(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    clipboard(&s.project, &ids)
}

fn clip_paste(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let board = p.opt("clipboard").ok_or_else(|| EngineError::Params("missing `clipboard`".into()))?.clone();
    let at = p.time("at")?;
    if at < Tick::ZERO {
        return Err(EngineError::Params("`at` must not be negative".into()));
    }
    s.edit("Paste", |proj| paste(proj, &board, at))
}

fn clip_duplicate(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    s.edit("Duplicate", |proj| {
        let board = clipboard(proj, &ids)?;
        let end = ids.iter().filter_map(|id| proj.clip(id)).map(|(_, c)| c.end()).max().unwrap_or(Tick::ZERO);
        paste(proj, &board, end)
    })
}

/// Places a clipboard's clips from `at`; returns their new ids, in the clipboard's order.
fn paste(proj: &mut Project, board: &Value, at: Tick) -> Result<Value, EngineError> {
    let bad = || EngineError::Params("`clipboard` must be what `clip.copy` returned".into());
    let entries = board.get("clips").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(bad)?;
    let mut parsed = Vec::new();
    for e in entries {
        let clip: Clip = serde_json::from_value(e.get("clip").cloned().ok_or_else(bad)?).map_err(|_| bad())?;
        let kind: TrackKind = serde_json::from_value(e.get("kind").cloned().ok_or_else(bad)?).map_err(|_| bad())?;
        let track = e.get("track").and_then(Value::as_str).unwrap_or_default().to_string();
        if let ClipContent::Media { media, .. } = &clip.content
            && !proj.media.contains_key(media)
        {
            return Err(not_found("media", media));
        }
        if let Some((timeline, _)) = clip.compound()
            && (timeline == proj.timeline || proj.tracks_of(timeline).is_none())
        {
            return Err(EngineError::Refused(format!("compound clip {:?} cannot be placed in this timeline", clip.id)));
        }
        parsed.push((track, kind, clip));
    }
    // copies of grouped clips are a group of their own
    let mut groups: std::collections::BTreeMap<String, String> = Default::default();
    for (_, _, clip) in &mut parsed {
        if let Some(g) = &clip.group {
            clip.group = Some(groups.entry(g.clone()).or_insert_with(|| proj.mint("g")).clone());
        }
    }
    let base = parsed.iter().map(|(_, _, c)| c.start).min().expect("not empty");
    let mut placed = Vec::new();
    for (track, kind, mut clip) in parsed {
        clip.start = at + (clip.start - base);
        clip.id = proj.mint("c");
        let range = clip.range();
        let fits = |t: &Track| t.kind == kind && (t.magnetic || !t.clips.iter().any(|c| c.range().overlaps(range)));
        // its own track first, then the others of its kind, then a new one
        let index =
            proj.tracks.iter().position(|t| t.id == track && fits(t)).or_else(|| proj.tracks.iter().position(fits));
        let index = match index {
            Some(i) => i,
            None => {
                let id = proj.mint("t");
                let n = proj.tracks.iter().filter(|t| t.kind == kind).count() + 1;
                let word = match kind {
                    TrackKind::Video => "Video",
                    TrackKind::Text => "Text",
                    TrackKind::Audio => "Audio",
                };
                let i = proj.tracks.iter().rposition(|t| t.kind == kind).map_or(proj.tracks.len(), |i| i + 1);
                proj.tracks.insert(
                    i,
                    Track {
                        id,
                        kind,
                        name: format!("{word} {n}"),
                        hidden: false,
                        muted: false,
                        magnetic: false,
                        locked: false,
                        clips: vec![],
                    },
                );
                i
            }
        };
        placed.push(json!({ "clip": clip.id, "track": proj.tracks[index].id, "start": clip.start }));
        proj.tracks[index].clips.push(clip);
    }
    Ok(json!({ "clips": placed }))
}

fn clip_set_speed(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let new = p.opt_f64("speed")?;
    let keep = p.opt_bool("keepPitch")?;
    let between = match p.opt_str("frames")? {
        None => None,
        Some(name) => Some(Inbetween::parse(name).ok_or_else(|| {
            EngineError::Params(format!("`frames` must be one of {}", Inbetween::ALL.map(Inbetween::name).join(", ")))
        })?),
    };
    if new.is_none() && keep.is_none() && between.is_none() {
        return Err(EngineError::Params("give `speed`, `keepPitch` or `frames`".into()));
    }
    if new.is_some_and(|n| !(n > 0.0 && n <= 100.0)) {
        return Err(EngineError::Params("`speed` must be above 0 and at most 100".into()));
    }
    s.edit("Speed", |proj| {
        let (_, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let video = matches!(&clip.content, ClipContent::Media { media, .. }
            if proj.media.get(media).is_some_and(|m| m.kind == MediaKind::Video));
        if between.is_some() && !video {
            return Err(EngineError::Params("only a video has frames to make pictures between".into()));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        let ClipContent::Media { speed, frames, .. } = &mut clip.content else {
            return Err(EngineError::Params("only media clips have a speed".into()));
        };
        if let Some(between) = between {
            *frames = between;
        }
        let frames = *frames;
        if new.is_some() && *speed == 0.0 {
            return Err(EngineError::Refused("a freeze frame has no speed; change its length instead".into()));
        }
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
        Ok(json!({ "clip": id, "duration": clip.duration, "keepPitch": clip.keep_pitch, "frames": frames.name() }))
    })
}

const STYLE_KEYS: [&str; 16] = [
    "font",
    "tracking",
    "leading",
    "italic",
    "underline",
    "weight",
    "align",
    "wrap",
    "outline",
    "background",
    "radius",
    "gradient",
    "outer",
    "shadow",
    "lineBoxes",
    "template",
];

/// `base` with the title style parameters in `p` applied (`null` removes an outline or box).
fn title_style(p: &P, base: TitleStyle) -> Result<TitleStyle, EngineError> {
    let mut st = base;
    if let Some(t) = template(p)? {
        // a template brings its look; where the lines wrap and how they line up stay
        st = st.wearing(&t.style);
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
    if let Some(b) = p.opt_bool("lineBoxes")? {
        st.line_boxes = b;
    }
    match p.v.get("font") {
        Some(Value::Null) => st.font = None,
        Some(v) => {
            st.font =
                Some(v.as_str().ok_or_else(|| EngineError::Params("`font` must be a media id or null".into()))?.into());
        }
        None => {}
    }
    let (lo, hi) = TitleStyle::SPACING;
    for (name, value) in [("tracking", &mut st.tracking), ("leading", &mut st.leading)] {
        if let Some(v) = p.opt_f64(name)? {
            if !(f64::from(lo)..=f64::from(hi)).contains(&v) {
                return Err(EngineError::Params(format!("`{name}` must be from {lo} to {hi} em")));
            }
            *value = v as f32;
        }
    }
    if let Some(b) = p.opt_bool("italic")? {
        st.italic = b;
    }
    if let Some(b) = p.opt_bool("underline")? {
        st.underline = b;
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
    for prop in Prop::MOTION {
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
            set_at(clip, prop, v, at)?;
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
            // a shape's colour is its fill, made one colour
            ClipContent::Shape { shape } if text.is_none() && size.is_none() => {
                if let Some(v) = color {
                    shape.fill = Some(reelforge_model::Fill::Solid { color: v });
                }
            }
            ClipContent::Media { .. }
            | ClipContent::Adjustment
            | ClipContent::Compound { .. }
            | ClipContent::Shape { .. }
                if text.is_some() || size.is_some() || color.is_some() =>
            {
                return Err(EngineError::Params(
                    "`text`, `size` and `color` apply to text and colour clips; `color` to shapes".into(),
                ));
            }
            ClipContent::Media { .. }
            | ClipContent::Adjustment
            | ClipContent::Compound { .. }
            | ClipContent::Shape { .. } => {}
        }
        Ok(json!({ "clip": serde_json::to_value(&*clip).expect("serialises") }))
    })
}

/// Sets `prop` of `clip` to `v`: its constant value, or, where it has keyframes, its value at
/// timeline time `at` (the key there changes, or a new one is made).
fn set_at(clip: &mut Clip, prop: Prop, v: f32, at: Option<Tick>) -> Result<(), EngineError> {
    if !clip.keyed(prop) {
        clip.set_base(prop, v);
        return Ok(());
    }
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
    Ok(())
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

/// The `fx` parameter: a list of effects, each a name or `{kind, strength, speed}`.
fn fx_param(p: &P) -> Result<Option<Vec<Fx>>, EngineError> {
    let Some(v) = p.opt("fx") else { return Ok(None) };
    let bad = || {
        EngineError::Params(format!(
            "`fx` must list effects, each a name or {{kind, strength 0-1, speed 0.25-4}}; the names are {}",
            FxKind::ALL.map(FxKind::name).join(", ")
        ))
    };
    let list = v.as_array().ok_or_else(bad)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let kind = item.as_str().or_else(|| item.get("kind").and_then(Value::as_str)).ok_or_else(bad)?;
        let mut fx = Fx::new(FxKind::parse(kind).ok_or_else(bad)?);
        let number = |name: &str| match item.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(x) => x.as_f64().map(|x| Some(x as f32)).ok_or_else(bad),
        };
        if let Some(x) = number("strength")? {
            fx.strength = x;
        }
        if let Some(x) = number("speed")? {
            fx.speed = x;
        }
        if let Some(why) = fx.problem() {
            return Err(EngineError::Params(why.into()));
        }
        out.push(fx);
    }
    Ok(Some(out))
}

fn clip_set_fx(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let fx = fx_param(&p)?.ok_or_else(|| EngineError::Params("missing `fx`".into()))?;
    s.edit("Effects", |proj| {
        let (ti, _) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind == TrackKind::Audio {
            return Err(EngineError::Params("effects apply to picture and title clips".into()));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        clip.fx = fx;
        Ok(json!({ "clip": id, "fx": serde_json::to_value(&clip.fx).expect("serialises") }))
    })
}

fn fx_list(_: &mut Session, _: &Value) -> CmdResult {
    let list: Vec<Value> = FxKind::ALL
        .iter()
        .map(|k| {
            json!({
                "id": k.name(),
                "group": match k.group() {
                    FxGroup::Motion => "motion",
                    FxGroup::Light => "light",
                    FxGroup::Focus => "focus",
                    FxGroup::Distort => "distort",
                },
                "hasSpeed": k.has_speed(),
            })
        })
        .collect();
    Ok(json!({ "effects": list }))
}

fn sticker_library(_: &mut Session, _: &Value) -> CmdResult {
    let list: Vec<Value> = reelforge_media::stickers::STICKERS
        .iter()
        .map(|st| json!({ "id": st.id, "name": st.en, "nameZh": st.zh }))
        .collect();
    Ok(json!({ "stickers": list }))
}

/// The clip `id` when it has sound of its own: a video with its sound, or audio.
fn sounding<'a>(proj: &'a mut Project, id: &str) -> Result<&'a mut Clip, EngineError> {
    let (_, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    let sound = matches!(&clip.content, ClipContent::Media { media, silent: false, .. }
        if proj.media.get(media).is_some_and(|m| m.kind != MediaKind::Image));
    if !sound {
        return Err(EngineError::Params("this clip has no sound".into()));
    }
    Ok(proj.clip_mut(id).expect("found above"))
}

fn clip_set_voice(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let kind = match v.get("voice") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(name) => Some(Some(name.as_str().and_then(VoiceKind::parse).ok_or_else(|| {
            EngineError::Params(format!(
                "`voice` must be null or one of {}",
                VoiceKind::ALL.map(VoiceKind::name).join(", ")
            ))
        })?)),
    };
    let pitch = p.opt_f64("pitch")?.map(|x| x as f32);
    if pitch.is_some_and(|x| x.abs() > Voice::PITCH_RANGE) {
        return Err(EngineError::Params("`pitch` must be from -12 to 12 semitones".into()));
    }
    s.edit("Voice", |proj| {
        let clip = sounding(proj, &id)?;
        if let Some(k) = kind {
            clip.voice.kind = k;
        }
        if let Some(x) = pitch {
            clip.voice.pitch = x;
        }
        Ok(json!({ "clip": id, "voice": clip.voice.kind.map(VoiceKind::name), "pitch": clip.voice.pitch, "semitones": clip.voice.semitones() }))
    })
}

fn clip_set_denoise(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let amount = p.opt_f64("amount")?.ok_or_else(|| EngineError::Params("`amount` is required".into()))? as f32;
    if !(0.0..=1.0).contains(&amount) {
        return Err(EngineError::Params("`amount` must be from 0 to 1".into()));
    }
    let floor: Option<Vec<f32>> = match p.opt("floor") {
        None => None,
        Some(list) => Some(
            list.as_array()
                .filter(|l| l.len() == Denoise::BANDS)
                .and_then(|l| l.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).map(|x| x as f32)).collect())
                .ok_or_else(|| EngineError::Params(format!("`floor` must be {} numbers", Denoise::BANDS)))?,
        ),
    };
    s.edit("Reduce Noise", |proj| {
        let clip = sounding(proj, &id)?;
        if amount == 0.0 {
            clip.voice.denoise = None;
            return Ok(json!({ "clip": id, "amount": 0.0 }));
        }
        let floor = floor.or_else(|| clip.voice.denoise.take().map(|d| d.floor)).ok_or_else(|| {
            EngineError::Params("the clip's noise has not been measured: pass `floor`, or use audio.denoise".into())
        })?;
        clip.voice.denoise = Some(Denoise { amount, floor });
        Ok(json!({ "clip": id, "amount": amount }))
    })
}

fn clip_set_stabilize(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let amount = p.opt_f64("amount")?.ok_or_else(|| EngineError::Params("`amount` is required".into()))? as f32;
    if !(0.0..=1.0).contains(&amount) {
        return Err(EngineError::Params("`amount` must be from 0 to 1".into()));
    }
    let wrong = || EngineError::Params("`path` must be a list of [across, down, turn], each three numbers".into());
    let path: Option<Vec<[f32; 3]>> = match p.opt("path") {
        None => None,
        Some(list) => Some(
            list.as_array()
                .ok_or_else(wrong)?
                .iter()
                .map(|at| {
                    let at = at.as_array().filter(|a| a.len() == 3)?;
                    let n = |i: usize| at[i].as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
                    Some([n(0)?, n(1)?, n(2)?])
                })
                .collect::<Option<_>>()
                .ok_or_else(wrong)?,
        ),
    };
    let (from, step) = (p.opt_time("from")?, p.opt_time("step")?);
    s.edit("Stabilize", |proj| {
        let (track, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let video = proj.tracks[track].kind == TrackKind::Video
            && matches!(&clip.content, ClipContent::Media { media, .. }
                if proj.media.get(media).is_some_and(|m| m.kind == MediaKind::Video));
        if !video {
            return Err(EngineError::Params("only a video clip on a video track can be stabilized".into()));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        if amount == 0.0 {
            clip.steady = None;
            return Ok(json!({ "clip": id, "amount": 0.0 }));
        }
        let steady = match path {
            Some(path) => {
                let step = step.ok_or_else(|| {
                    EngineError::Params("`step` is required with `path`: the time between samples".into())
                })?;
                Steady { amount, from: from.unwrap_or(Tick::ZERO), step, path }
            }
            None => Steady {
                amount,
                ..clip.steady.take().ok_or_else(|| {
                    EngineError::Params(
                        "the camera's path has not been measured for this clip: pass `path`, or use clip.stabilize"
                            .into(),
                    )
                })?
            },
        };
        let samples = steady.path.len();
        clip.steady = Some(steady);
        Ok(json!({ "clip": id, "amount": amount, "samples": samples }))
    })
}

fn voice_list(_: &mut Session, _: &Value) -> CmdResult {
    let list: Vec<Value> = VoiceKind::ALL.iter().map(|k| json!({ "id": k.name(), "semitones": k.pitch() })).collect();
    Ok(json!({ "voices": list }))
}

fn prop(p: &P) -> Result<Prop, EngineError> {
    let name = p.str("prop")?;
    Prop::parse(name)
        .ok_or_else(|| EngineError::Params(format!("`prop` must be one of {}", Prop::ALL.map(Prop::name).join(", "))))
}

/// The `ease` parameter: a curve by name, or drawn by hand as `[x1, y1, x2, y2]`.
fn ease_of(p: &P) -> Result<Option<Ease>, EngineError> {
    let Some(v) = p.opt("ease") else { return Ok(None) };
    let names = || Ease::NAMED.map(|(n, _)| n).join(", ");
    if let Some(name) = v.as_str() {
        return Ease::named(name)
            .map(Some)
            .ok_or_else(|| EngineError::Params(format!("`ease` must be one of {}, or [x1, y1, x2, y2]", names())));
    }
    let points: [f32; 4] = serde_json::from_value(v.clone())
        .map_err(|_| EngineError::Params(format!("`ease` must be one of {}, or [x1, y1, x2, y2]", names())))?;
    let ease = Ease::Bezier(points);
    if !ease.is_valid() {
        let (lo, hi) = Ease::REACH;
        return Err(EngineError::Params(format!(
            "`ease` [x1, y1, x2, y2] goes forward in time: each x from 0 to 1, each y from {lo} to {hi}"
        )));
    }
    Ok(Some(ease))
}

fn clip_set_ease(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let at = p.time("at")?;
    let ease = ease_of(&p)?.ok_or_else(|| EngineError::Params("`ease` is required".into()))?;
    let props: Option<Vec<Prop>> = match p.opt("props") {
        None => None,
        Some(list) => {
            let names: Vec<String> = serde_json::from_value(list.clone())
                .map_err(|_| EngineError::Params("`props` must be a list of property names".into()))?;
            let props: Option<Vec<Prop>> = names.iter().map(|n| Prop::parse(n)).collect();
            Some(props.filter(|l| !l.is_empty()).ok_or_else(|| {
                EngineError::Params(format!("`props` must name some of {}", Prop::ALL.map(Prop::name).join(", ")))
            })?)
        }
    };
    let half = Tick(fps(s).frame_duration().0 / 2);
    s.edit("Keyframe Curve", |proj| {
        let clip = proj.clip_mut(&id).ok_or_else(|| not_found("clip", &id))?;
        let t = at - clip.start;
        let mut set = Vec::new();
        for (prop, list) in clip.keys.iter_mut().filter(|(prop, _)| props.as_ref().is_none_or(|l| l.contains(prop))) {
            let near = list.iter_mut().filter(|k| (k.t - t).0.abs() <= half.0).min_by_key(|k| (k.t - t).0.abs());
            if let Some(k) = near {
                k.ease = ease;
                set.push(prop.name());
            }
        }
        if set.is_empty() {
            return Err(EngineError::Refused("no keyframe at that time to shape the curve from".into()));
        }
        Ok(json!({ "clip": id, "props": set, "ease": serde_json::to_value(ease).expect("serialises") }))
    })
}

fn clip_set_key(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let prop = prop(&p)?;
    let at = p.time("at")?;
    let value = p.opt_f64("value")?.map(|x| x as f32);
    let ease = ease_of(&p)?;
    if value.is_some_and(|v| !v.is_finite() || prop.range().is_some_and(|(lo, hi)| !(lo..=hi).contains(&v))) {
        return Err(EngineError::Params("`value` is out of range".into()));
    }
    s.edit("Add Keyframe", |proj| {
        if prop.plain() {
            // what moves must be there: a picture to crop or colour, a mask to move
            let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
            let media = clip.is_picture();
            let ok = if Prop::MASK.contains(&prop) {
                clip.look.mask.is_some()
            } else if Prop::CROP.contains(&prop) {
                media && proj.tracks[ti].kind != TrackKind::Audio
            } else if Prop::DRAW.contains(&prop) {
                matches!(&clip.content, ClipContent::Shape { shape } if shape.stroke.is_some())
            } else if prop == Prop::Count {
                matches!(&clip.content, ClipContent::Text { style, .. } if style.counter.is_some())
            } else {
                proj.tracks[ti].kind != TrackKind::Audio
            };
            if !ok {
                return Err(EngineError::Params(format!("this clip has no `{}` to key", prop.name())));
            }
        }
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
    if slot != "in" && slot != "out" && slot != "loop" {
        return Err(EngineError::Params("`slot` must be in, out or loop".into()));
    }
    let name = p.opt_str("kind")?;
    let duration = p.opt_time("duration")?;
    if duration.is_some_and(|d| d <= Tick::ZERO) {
        return Err(EngineError::Params("`duration` must be positive".into()));
    }
    let looped = match name {
        Some(k) if slot == "loop" => Some(LoopKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "a loop's `kind` must be null or one of {}",
                LoopKind::ALL.map(LoopKind::name).join(", ")
            ))
        })?),
        _ => None,
    };
    // a round shorter than this is a blur
    if slot == "loop" && duration.is_some_and(|d| d < Tick(reelforge_time::TICKS_PER_SECOND / 20)) {
        return Err(EngineError::Params("a loop's `duration` is one round: at least 0.05 s".into()));
    }
    let kind = match name {
        Some(k) if slot != "loop" => Some(AnimKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "`kind` must be null or one of {}",
                AnimKind::ALL.map(AnimKind::name).join(", ")
            ))
        })?),
        _ => None,
    };
    s.edit("Animation", |proj| {
        let (ti, _) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind == TrackKind::Audio {
            return Err(EngineError::Params("animations apply to picture and title clips".into()));
        }
        if (kind.is_some_and(AnimKind::titles_only) || looped.is_some_and(LoopKind::titles_only))
            && !matches!(proj.clip(&id).map(|(_, c)| &c.content), Some(ClipContent::Text { .. }))
        {
            return Err(EngineError::Params(format!("{} applies to titles", name.unwrap_or_default())));
        }
        if kind.is_some_and(AnimKind::shapes_only)
            && !matches!(proj.clip(&id).map(|(_, c)| &c.content), Some(ClipContent::Shape { .. }))
        {
            return Err(EngineError::Params(format!("{} applies to shapes", name.unwrap_or_default())));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        if slot == "loop" {
            let before = clip.anim.repeat;
            clip.anim.repeat = looped.map(|kind| Loop {
                kind,
                // another kind keeps the round it had only when one was asked for
                period: duration.or(before.filter(|l| l.kind == kind).map(|l| l.period)).unwrap_or(kind.period()),
            });
            return Ok(json!({ "clip": id, "anim": serde_json::to_value(clip.anim).expect("serialises") }));
        }
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
            if clip.is_adjustment() {
                return Err(EngineError::Params("an adjustment clip has no picture to cut to".into()));
            }
            if !track.clips.iter().any(|c| c.end() == clip.start && !c.is_adjustment()) {
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

fn clip_set_color(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let reset = p.opt_bool("reset")?.unwrap_or(false);
    let filter = match p.v.get("filter") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(k)) => Some(Some(FilterKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "`filter` must be null or one of {}",
                FilterKind::ALL.map(FilterKind::name).join(", ")
            ))
        })?)),
        Some(_) => return Err(EngineError::Params("`filter` must be a name or null".into())),
    };
    let lut = match p.v.get("lut") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(m)) => Some(Some(m.clone())),
        Some(_) => return Err(EngineError::Params("`lut` must be a media id or null".into())),
    };
    let unit = |name: &str| -> Result<Option<f32>, EngineError> {
        match p.opt_f64(name)? {
            Some(x) if !(0.0..=1.0).contains(&x) => Err(EngineError::Params(format!("`{name}` must be 0 to 1"))),
            x => Ok(x.map(|x| x as f32)),
        }
    };
    let (filter_strength, lut_strength) = (unit("filterStrength")?, unit("lutStrength")?);
    let at = p.opt_time("at")?;
    let mut changes = Vec::new();
    for (name, lo, hi) in Adjust::RANGES {
        if let Some(x) = p.opt_f64(name)? {
            if !(lo as f64..=hi as f64).contains(&x) {
                return Err(EngineError::Params(format!("`{name}` must be {lo} to {hi}")));
            }
            changes.push((name, x as f32));
        }
    }
    s.edit("Colour", |proj| {
        let (ti, _) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind == TrackKind::Audio {
            return Err(EngineError::Params("only picture clips have colour".into()));
        }
        if let Some(Some(m)) = &lut {
            match proj.media.get(m) {
                Some(media) if media.kind == MediaKind::Lut => {}
                Some(_) => return Err(EngineError::Params(format!("media {m} is not a lookup table"))),
                None => return Err(not_found("media", m)),
            }
        }
        let clip = proj.clip_mut(&id).expect("found above");
        let c = &mut clip.color;
        if reset {
            *c = Default::default();
        }
        match filter {
            Some(None) => c.filter = None,
            Some(Some(kind)) => {
                c.filter = Some(Filter { kind, strength: c.filter.map_or(1.0, |f| f.strength) });
            }
            None => {}
        }
        if let Some(st) = filter_strength {
            let f = c.filter.as_mut().ok_or_else(|| EngineError::Params("there is no filter to strengthen".into()))?;
            f.strength = st;
        }
        match lut {
            Some(None) => c.lut = None,
            Some(Some(media)) => {
                let strength = c.lut.as_ref().map_or(1.0, |l| l.strength);
                c.lut = Some(LutUse { media, strength });
            }
            None => {}
        }
        if let Some(st) = lut_strength {
            let l =
                c.lut.as_mut().ok_or_else(|| EngineError::Params("there is no lookup table to strengthen".into()))?;
            l.strength = st;
        }
        if reset {
            clip.keys.retain(|p, _| !Prop::ADJUST.contains(p));
        }
        for (name, x) in &changes {
            let i = Adjust::RANGES.iter().position(|(n, ..)| n == name).expect("a known name");
            set_at(clip, Prop::ADJUST[i], *x, at)?;
        }
        Ok(json!({ "clip": id, "color": serde_json::to_value(&clip.color).expect("serialises") }))
    })
}

fn clip_set_crop(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let mut edges = [None; 4];
    for (i, name) in ["left", "top", "right", "bottom"].into_iter().enumerate() {
        if let Some(x) = p.opt_f64(name)? {
            if !(0.0..1.0).contains(&x) {
                return Err(EngineError::Params(format!("`{name}` must be 0 to below 1")));
            }
            edges[i] = Some(x as f32);
        }
    }
    let (flip_x, flip_y) = (p.opt_bool("flipX")?, p.opt_bool("flipY")?);
    let at = p.opt_time("at")?;
    s.edit("Crop", |proj| {
        let (_, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let picture = match &clip.content {
            ClipContent::Media { media, .. } => proj.media.get(media).is_some_and(|m| m.kind != MediaKind::Audio),
            ClipContent::Compound { .. } => true,
            _ => false,
        };
        if !picture {
            return Err(EngineError::Params(
                "only video, picture and compound clips can be cropped or mirrored".into(),
            ));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        for (prop, x) in Prop::CROP.into_iter().zip(edges) {
            if let Some(x) = x {
                set_at(clip, prop, x, at)?;
            }
        }
        let look = &mut clip.look;
        look.flip_x = flip_x.unwrap_or(look.flip_x);
        look.flip_y = flip_y.unwrap_or(look.flip_y);
        if let Some(why) = look.problem() {
            return Err(EngineError::Params(why.into()));
        }
        Ok(json!({ "clip": id, "look": serde_json::to_value(&clip.look).expect("serialises") }))
    })
}

fn clip_set_person_key(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let on = p.opt_bool("on")?.unwrap_or(true);
    s.edit("Cut Out People", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind != TrackKind::Video || !matches!(clip.content, ClipContent::Media { .. }) {
            return Err(EngineError::Params("only video and picture clips can have their people cut out".into()));
        }
        proj.clip_mut(&id).expect("found above").look.person = on;
        Ok(json!({ "clip": id, "person": on }))
    })
}

fn clip_set_chroma_key(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let off = matches!(v.get("color"), Some(Value::Null));
    let color = if off { None } else { p.opt_color("color")?.map(|c| [c[0], c[1], c[2]]) };
    let mut tweaks = [None; 3];
    for (slot, name) in tweaks.iter_mut().zip(["tolerance", "softness", "spill"]) {
        *slot = match p.opt_f64(name)? {
            Some(x) if (0.0..=1.0).contains(&x) => Some(x as f32),
            Some(_) => return Err(EngineError::Params(format!("`{name}` must be from 0 to 1"))),
            None => None,
        };
    }
    s.edit("Chroma Key", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        if proj.tracks[ti].kind != TrackKind::Video || !matches!(clip.content, ClipContent::Media { .. }) {
            return Err(EngineError::Params("only video and picture clips can have a colour keyed out".into()));
        }
        let clip = proj.clip_mut(&id).expect("found above");
        if off {
            clip.look.chroma = None;
        } else {
            let mut key = match (clip.look.chroma, color) {
                (Some(k), Some(c)) => ChromaKey { color: c, ..k },
                (Some(k), None) => k,
                (None, Some(c)) => ChromaKey::new(c),
                (None, None) => {
                    return Err(EngineError::Params("the clip has no chroma key; give a `color`".into()));
                }
            };
            let [tolerance, softness, spill] = tweaks;
            key.tolerance = tolerance.unwrap_or(key.tolerance);
            key.softness = softness.unwrap_or(key.softness);
            key.spill = spill.unwrap_or(key.spill);
            clip.look.chroma = Some(key);
        }
        Ok(json!({ "clip": id, "chroma": serde_json::to_value(clip.look.chroma).expect("serialises") }))
    })
}

fn clip_set_blend(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let mode = BlendMode::parse(p.str("mode")?).ok_or_else(|| {
        EngineError::Params(format!("`mode` must be one of {}", BlendMode::ALL.map(BlendMode::name).join(", ")))
    })?;
    s.edit("Blend Mode", |proj| {
        let clip = picture_clip(proj, &id, "blend")?;
        clip.look.blend = mode;
        Ok(json!({ "clip": id, "blend": mode.name() }))
    })
}

fn clip_set_mask(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let shape = match p.v.get("shape") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(k)) => Some(Some(MaskKind::parse(k).ok_or_else(|| {
            EngineError::Params(format!(
                "`shape` must be null or one of {}",
                MaskKind::ALL.map(MaskKind::name).join(", ")
            ))
        })?)),
        Some(_) => return Err(EngineError::Params("`shape` must be a name or null".into())),
    };
    let num = |name: &str| -> Result<Option<f32>, EngineError> {
        match p.opt_f64(name)? {
            Some(x) if !x.is_finite() => Err(EngineError::Params(format!("`{name}` must be a number"))),
            x => Ok(x.map(|x| x as f32)),
        }
    };
    let (x, y, w, h, rotation, feather) =
        (num("x")?, num("y")?, num("width")?, num("height")?, num("rotation")?, num("feather")?);
    let invert = p.opt_bool("invert")?;
    let at = p.opt_time("at")?;
    s.edit("Mask", |proj| {
        let clip = picture_clip(proj, &id, "have a mask")?;
        match shape {
            Some(None) => {
                clip.look.mask = None;
                clip.keys.retain(|p, _| !Prop::MASK.contains(p));
            }
            Some(Some(kind)) => match &mut clip.look.mask {
                Some(m) => m.kind = kind,
                None => clip.look.mask = Some(Mask::new(kind)),
            },
            None => {}
        }
        let tweaks = [x, y, w, h, rotation, feather].iter().any(Option::is_some) || invert.is_some();
        match &mut clip.look.mask {
            Some(m) => {
                m.invert = invert.unwrap_or(m.invert);
                for (prop, v) in Prop::MASK.into_iter().zip([x, y, w, h, rotation, feather]) {
                    if let Some(v) = v {
                        set_at(clip, prop, v, at)?;
                    }
                }
            }
            None if tweaks => return Err(EngineError::Params("the clip has no mask; give a `shape`".into())),
            None => {}
        }
        if let Some(why) = clip.look.problem() {
            return Err(EngineError::Params(why.into()));
        }
        Ok(json!({ "clip": id, "mask": serde_json::to_value(clip.look.mask).expect("serialises") }))
    })
}

/// The clip `id` on a picture track, for what `what` needs.
fn clip_set_backdrop(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let blur = match v.get("blur") {
        None => return Err(EngineError::Params("missing `blur` (a number from 0 to 1, or null)".into())),
        Some(Value::Null) => None,
        Some(_) => match p.opt_f64("blur")? {
            Some(b) if (0.0..=1.0).contains(&b) => Some(b as f32),
            _ => return Err(EngineError::Params("`blur` must be from 0 to 1, or null".into())),
        },
    };
    let all = p.opt_bool("all")?.unwrap_or(false);
    s.edit("Canvas Blur", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let is_picture = |c: &Clip| matches!(c.content, ClipContent::Media { .. });
        if proj.tracks[ti].kind != TrackKind::Video || !is_picture(clip) {
            return Err(EngineError::Params("only video and picture clips have a canvas blur".into()));
        }
        let ids: Vec<String> = if all {
            proj.tracks[ti].clips.iter().filter(|c| is_picture(c)).map(|c| c.id.clone()).collect()
        } else {
            vec![id.clone()]
        };
        for c in &ids {
            proj.clip_mut(c).expect("on the track").look.backdrop = blur;
        }
        Ok(json!({ "clips": ids, "blur": blur }))
    })
}

fn picture_clip<'a>(proj: &'a mut Project, id: &str, what: &str) -> Result<&'a mut Clip, EngineError> {
    let (ti, _) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    if proj.tracks[ti].kind == TrackKind::Audio {
        return Err(EngineError::Params(format!("only picture clips can {what}")));
    }
    Ok(proj.clip_mut(id).expect("found above"))
}

/// The media clip `id`, refusing freeze frames and stills for what `what` needs.
fn moving_media<'a>(proj: &'a mut Project, id: &str, what: &str) -> Result<&'a mut Clip, EngineError> {
    let (_, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    let kind = match &clip.content {
        ClipContent::Media { media, .. } => proj.media.get(media).map(|m| m.kind),
        _ => None,
    };
    match kind {
        None => return Err(EngineError::Params(format!("only video and sound clips can {what}"))),
        Some(MediaKind::Image) => return Err(EngineError::Params(format!("a picture cannot {what}"))),
        Some(_) => {}
    }
    let clip = proj.clip_mut(id).expect("found above");
    if matches!(clip.content, ClipContent::Media { speed, .. } if speed == 0.0) {
        return Err(EngineError::Refused(format!("a freeze frame cannot {what}")));
    }
    Ok(clip)
}

fn clip_set_reverse(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let on = p.opt_bool("reversed")?.ok_or_else(|| EngineError::Params("give `reversed`".into()))?;
    s.edit("Reverse", |proj| {
        let clip = moving_media(proj, &id, "play backwards")?;
        if let ClipContent::Media { reversed, .. } = &mut clip.content {
            *reversed = on;
        }
        Ok(json!({ "clip": id, "reversed": on }))
    })
}

fn clip_set_speed_curve(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let bad = || {
        EngineError::Params(format!(
            "`curve` must be null, one of {}, or [[x, m], ...] with x from 0 to 1 in order and m from 0.1 to 10",
            CurvePreset::ALL.map(CurvePreset::name).join(", ")
        ))
    };
    let points = match p.v.get("curve") {
        None => return Err(EngineError::Params("give `curve`".into())),
        Some(Value::Null) => vec![],
        Some(Value::String(name)) => CurvePreset::parse(name).ok_or_else(bad)?.points(),
        Some(Value::Array(list)) => {
            let pts = list
                .iter()
                .map(|pt| match pt.as_array().map(Vec::as_slice) {
                    Some([x, m]) => Some(CurvePoint { x: x.as_f64()?, m: m.as_f64()? }),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(bad)?;
            let (lo, hi) = timing::CURVE_RANGE;
            if pts.len() < 2
                || pts.iter().any(|p| !(0.0..=1.0).contains(&p.x) || !(lo..=hi).contains(&p.m))
                || pts.windows(2).any(|w| w[1].x <= w[0].x)
            {
                return Err(bad());
            }
            pts
        }
        Some(_) => return Err(bad()),
    };
    s.edit("Speed Curve", |proj| {
        let clip = moving_media(proj, &id, "follow a speed curve")?;
        if let ClipContent::Media { curve, .. } = &mut clip.content {
            *curve = points;
        }
        let preset = match &clip.content {
            ClipContent::Media { curve, .. } => CurvePreset::of(curve).map(CurvePreset::name),
            _ => None,
        };
        Ok(json!({ "clip": id, "preset": preset }))
    })
}

/// How long a freeze frame holds unless told.
pub const FREEZE_LENGTH: Tick = Tick(3 * reelforge_time::TICKS_PER_SECOND);

fn clip_freeze(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    let at = p.time("at")?;
    let duration = p.opt_time("duration")?.unwrap_or(FREEZE_LENGTH);
    if duration <= Tick::ZERO {
        return Err(EngineError::Params("`duration` must be positive".into()));
    }
    s.edit("Freeze Frame", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let clip = clip.clone();
        if proj.tracks[ti].kind != TrackKind::Video {
            return Err(EngineError::Params("freeze frames are made from clips on a video track".into()));
        }
        let is_video = matches!(&clip.content, ClipContent::Media { media, .. }
            if proj.media.get(media).is_some_and(|m| m.kind == MediaKind::Video));
        let Some(timing) = clip.timing().filter(|_| is_video) else {
            return Err(EngineError::Params("only video clips can freeze".into()));
        };
        if timing.speed == 0.0 {
            return Err(EngineError::Refused("this clip is already a freeze frame".into()));
        }
        if at < clip.start || at > clip.end() {
            return Err(EngineError::Params("`at` must be within the clip".into()));
        }
        // the frame on screen at `at`: the clip's last when `at` is its end
        let into = (at - clip.start).min(clip.duration - Tick(1));
        let held = timing.source_at(into).max(Tick::ZERO);
        let mut still = clip.clone();
        still.id = proj.mint("c");
        still.start = at;
        still.duration = duration;
        if let ClipContent::Media { source_in, speed, reversed, curve, .. } = &mut still.content {
            (*source_in, *speed, *reversed) = (held, 0.0, false);
            curve.clear();
        }
        // the placement it had at that instant, held
        for prop in Prop::ALL {
            let v = clip.value_at(prop, into);
            still.set_base(prop, v);
        }
        still.keys.clear();
        still.anim = Default::default();
        still.transition = None;
        still.fade = Default::default();
        let right = if at > clip.start && at < clip.end() { Some(split(proj, &id, at)?) } else { None };
        for c in proj.tracks[ti].clips.iter_mut().filter(|c| c.start >= at) {
            c.start += duration;
        }
        let still_id = still.id.clone();
        proj.tracks[ti].clips.push(still);
        Ok(json!({ "clip": still_id, "right": right, "duration": duration }))
    })
}

fn edit_batch(s: &mut Session, v: &Value) -> CmdResult {
    let steps = v
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::Params("`steps` must be a list".into()))?
        .clone();
    let label = v.get("label").and_then(Value::as_str).unwrap_or("Batch").to_string();
    let results = one_step(s, &label, |s| {
        let mut results = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("id").and_then(Value::as_str).unwrap_or_default();
            let mut params = step.get("params").cloned().unwrap_or(Value::Null);
            fill_refs(&mut params, &results)
                .map_err(|e| EngineError::Params(format!("step {i} ({id}), nothing applied: {e}")))?;
            let r = if id == "edit.undo" || id == "edit.redo" || id == "edit.batch" {
                Err(EngineError::Params(format!("{id} cannot run inside a batch")))
            } else {
                s.execute(id, &params)
            };
            results.push(r.map_err(|e| EngineError::Refused(format!("step {i} ({id}) failed, nothing applied: {e}")))?);
        }
        Ok(results)
    })?;
    Ok(json!({ "results": results }))
}

/// Replaces every string of the form `${n.path}` in a batch step's parameters with what an
/// earlier step `n` (from 0) replied there: `${0.clip}` is the id of the clip step 0 added,
/// `${1.clips.0.clip}` the first of a list, so a batch can add a clip and go on editing it.
fn fill_refs(v: &mut Value, results: &[Value]) -> Result<(), String> {
    match v {
        Value::String(s) => {
            let Some(inner) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}')) else { return Ok(()) };
            let mut parts = inner.split('.');
            let n: usize = parts
                .next()
                .and_then(|n| n.trim().parse().ok())
                .ok_or_else(|| format!("{s:?} must name an earlier step by number, as ${{0.clip}}"))?;
            let mut at = results.get(n).ok_or_else(|| format!("{s:?} names step {n}, which has not run"))?;
            for key in parts {
                at = match at {
                    Value::Array(a) => key.parse::<usize>().ok().and_then(|i| a.get(i)),
                    Value::Object(o) => o.get(key),
                    _ => None,
                }
                .ok_or_else(|| format!("{s:?}: step {n} replied with no `{key}` there"))?;
            }
            *v = at.clone();
        }
        Value::Array(a) => a.iter_mut().try_for_each(|x| fill_refs(x, results))?,
        Value::Object(o) => o.values_mut().try_for_each(|x| fill_refs(x, results))?,
        _ => {}
    }
    Ok(())
}

/// Runs `f`, which may run several commands, as one undo step called `label`; if it fails,
/// nothing it did applies.
pub(crate) fn one_step<T>(
    s: &mut Session,
    label: &str,
    f: impl FnOnce(&mut Session) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let saved: (std::sync::Arc<Project>, usize, Vec<_>, u64) =
        (s.project.clone(), s.undo.len(), s.redo.clone(), s.revision);
    match f(s) {
        Ok(out) => {
            if s.undo.len() > saved.1 {
                let before = s.undo[saved.1].1.clone();
                s.undo.truncate(saved.1);
                s.undo.push((label.to_string(), before));
            }
            Ok(out)
        }
        Err(e) => {
            s.project = saved.0;
            s.undo.truncate(saved.1);
            s.redo = saved.2;
            s.revision = saved.3;
            Err(e)
        }
    }
}

fn render_compile(s: &mut Session, v: &Value) -> CmdResult {
    let encode = v.get("encode").and_then(Value::as_bool).unwrap_or(true);
    let spec = compile(&s.project, &CompileOptions { encode }).map_err(|e| EngineError::Refused(e.to_string()))?;
    Ok(serde_json::to_value(&spec).expect("serialises"))
}
