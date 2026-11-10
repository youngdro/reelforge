//! CapCut / 剪映 drafts: `draft_content.json` (or an unencrypted `draft_info.json`) read into a
//! timeline.
//!
//! A draft names its files by path, keeps times in microseconds and places pictures with a
//! transform centred on the canvas, x right and y up, in half-canvas units. What reelforge has an
//! equivalent for comes across: video, photo, sound and text segments with their source ranges,
//! speed, volume, opacity, position, scale, rotation, mirroring, sound fades and transitions (as
//! dissolves). Stickers, effects, filters, masks and keyframes are counted and skipped.

use std::collections::BTreeMap;

use reelforge_model::{
    Clip, ClipContent, ClipTransform, Fades, Media, MediaKind, Project, TitleStyle, Track, TrackKind, Transition,
    TransitionKind,
};
use reelforge_time::{FrameRate, TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

use crate::params::P;
use crate::{CmdResult, EngineError, Session};

const TICKS_PER_MICRO: i64 = TICKS_PER_SECOND / 1_000_000;

fn micros(v: &Value) -> Tick {
    Tick(v.as_f64().map_or(0, |us| us.round() as i64) * TICKS_PER_MICRO)
}

fn num(v: &Value, default: f64) -> f64 {
    v.as_f64().filter(|x| x.is_finite()).unwrap_or(default)
}

/// A file the draft uses.
struct Material {
    path: String,
    name: String,
    kind: MediaKind,
    duration: Option<Tick>,
    size: Option<[u32; 2]>,
}

/// What came across and what did not.
#[derive(Default)]
struct Report {
    clips: usize,
    skipped: BTreeMap<String, usize>,
}

impl Report {
    fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }
}

/// The draft's path with CapCut's folder placeholder replaced by `folder`.
fn resolve(path: &str, folder: Option<&str>) -> String {
    let rest = match path.strip_prefix("##_draftpath_placeholder_") {
        Some(r) => r.split_once("_##").map_or(r, |(_, after)| after).trim_start_matches(['/', '\\']),
        None => return path.to_string(),
    };
    match folder {
        Some(f) => format!("{}/{rest}", f.trim_end_matches(['/', '\\'])),
        None => rest.to_string(),
    }
}

/// The words of a text material. Newer drafts keep JSON (`{"text": …, "styles": …}`) in
/// `content`; older ones keep `<font …>[words]</font>` markup.
fn text_of(m: &Value) -> (String, Option<f64>, Option<[f32; 4]>) {
    let content = m["content"].as_str().unwrap_or_default();
    if let Ok(v) = serde_json::from_str::<Value>(content)
        && let Some(text) = v["text"].as_str()
    {
        let style = &v["styles"][0];
        let size = style["size"].as_f64();
        let color = style["fill"]["content"]["solid"]["color"].as_array().and_then(|c| {
            let c: Vec<f32> = c.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect();
            (c.len() >= 3).then(|| [c[0], c[1], c[2], 1.0])
        });
        return (text.to_string(), size, color);
    }
    let mut out = String::new();
    let mut in_tag = false;
    for c in content.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '[' | ']' if !in_tag => {}
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let size = m["font_size"].as_f64().or_else(|| m["text_size"].as_f64().map(|s| s / 2.0));
    let color = m["text_color"].as_str().and_then(hex_color);
    (out, size, color)
}

fn hex_color(s: &str) -> Option<[f32; 4]> {
    let h = s.trim_start_matches('#');
    let h = if h.len() == 8 { &h[2..] } else { h };
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|b| f32::from(b) / 255.0);
    Some([c(0)?, c(2)?, c(4)?, 1.0])
}

/// Reads the file at `path` when this machine has it: its hash, kind, length and size.
#[cfg(not(target_arch = "wasm32"))]
fn measure(path: &str, m: &mut Material) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    if let Ok(pr) = reelforge_media::probe_path(std::path::Path::new(path)) {
        m.duration = pr.duration.or(m.duration);
        m.size = pr.size.or(m.size);
    }
    Some(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(target_arch = "wasm32")]
fn measure(_: &str, _: &mut Material) -> Option<String> {
    None
}

fn blank_clip(id: String, start: Tick, duration: Tick, content: ClipContent) -> Clip {
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
    }
}

/// Reads a draft into `proj`'s current timeline, which the caller has emptied. Returns the report.
fn read_draft(proj: &mut Project, draft: &Value, folder: Option<&str>) -> Result<Report, EngineError> {
    let mats = &draft["materials"];
    let by_id = |list: &str| -> BTreeMap<String, &Value> {
        mats[list].as_array().into_iter().flatten().filter_map(|m| Some((m["id"].as_str()?.to_string(), m))).collect()
    };
    let videos = by_id("videos");
    let audios = by_id("audios");
    let texts = by_id("texts");
    let speeds = by_id("speeds");
    let fades = by_id("audio_fades");
    let transitions = by_id("transitions");

    let mut report = Report::default();
    // media, once per file
    let mut media_of: BTreeMap<String, String> = BTreeMap::new();
    let mut add_media = |proj: &mut Project, mat: &Value, kind: MediaKind| -> Option<String> {
        let raw = mat["path"].as_str().filter(|p| !p.is_empty())?;
        let path = resolve(raw, folder);
        if let Some(id) = media_of.get(&path) {
            return Some(id.clone());
        }
        // the same file, or one relinked from an earlier import of this draft
        let base = |u: &str| u.rsplit(['/', '\\']).next().unwrap_or(u).to_string();
        let file = base(&path);
        let same = |m: &&Media| m.uri == path || (m.kind == kind && base(&m.uri) == file);
        if let Some(m) = proj.media.values().find(same) {
            media_of.insert(path, m.id.clone());
            return Some(m.id.clone());
        }
        let name = mat["material_name"]
            .as_str()
            .or(mat["name"].as_str())
            .filter(|n| !n.is_empty())
            .map_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string(), str::to_string);
        let w = mat["width"].as_u64().unwrap_or(0) as u32;
        let h = mat["height"].as_u64().unwrap_or(0) as u32;
        let mut m = Material {
            path: path.clone(),
            name,
            kind,
            duration: Some(micros(&mat["duration"])).filter(|d| *d > Tick::ZERO),
            size: (w > 0 && h > 0).then_some([w, h]),
        };
        let sha256 = measure(&path, &mut m);
        let id = proj.mint("m");
        let duration = if m.kind == MediaKind::Image { None } else { m.duration };
        proj.media.insert(
            id.clone(),
            Media {
                id: id.clone(),
                name: m.name,
                uri: m.path,
                sha256,
                kind: m.kind,
                duration,
                size: m.size,
                folder: None,
            },
        );
        media_of.insert(path, id.clone());
        Some(id)
    };

    let canvas = proj.canvas.clone();
    let title_px = canvas.width.min(canvas.height) as f64 * 0.06;
    let mut video_tracks = Vec::new();
    let mut text_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    let mut counts = [0usize; 3];

    for tr in draft["tracks"].as_array().into_iter().flatten() {
        let kind = match tr["type"].as_str().unwrap_or_default() {
            "video" => TrackKind::Video,
            "audio" => TrackKind::Audio,
            "text" => TrackKind::Text,
            other => {
                for _ in tr["segments"].as_array().into_iter().flatten() {
                    report.skip(if other.is_empty() { "unknown" } else { other });
                }
                continue;
            }
        };
        let n = match kind {
            TrackKind::Video => &mut counts[0],
            TrackKind::Text => &mut counts[1],
            TrackKind::Audio => &mut counts[2],
        };
        *n += 1;
        let name = match kind {
            TrackKind::Video => format!("Video {n}"),
            TrackKind::Text => format!("Text {n}"),
            TrackKind::Audio => format!("Audio {n}"),
        };
        let flag = tr["attribute"].as_u64().unwrap_or(0);
        let mut track = Track {
            id: proj.mint("t"),
            kind,
            name,
            hidden: false,
            // bit 0 of `attribute` is CapCut's mute
            muted: flag & 1 == 1,
            magnetic: false,
            locked: false,
            clips: vec![],
        };
        let mut pending: Option<Transition> = None;
        let mut segs: Vec<&Value> = tr["segments"].as_array().into_iter().flatten().collect();
        segs.sort_by_key(|s| s["target_timerange"]["start"].as_i64().unwrap_or(0));
        for seg in segs {
            let start = micros(&seg["target_timerange"]["start"]);
            let duration = micros(&seg["target_timerange"]["duration"]);
            let mat_id = seg["material_id"].as_str().unwrap_or_default();
            if duration <= Tick::ZERO {
                continue;
            }
            let refs: Vec<&str> =
                seg["extra_material_refs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let speed = refs
                .iter()
                .find_map(|r| speeds.get(*r).map(|s| num(&s["speed"], 1.0)))
                .unwrap_or_else(|| num(&seg["speed"], 1.0))
                .clamp(0.01, 100.0);
            let content = match kind {
                TrackKind::Video | TrackKind::Audio => {
                    let (mat, mk) = match (videos.get(mat_id), audios.get(mat_id)) {
                        (Some(v), _) => {
                            (*v, if v["type"].as_str() == Some("photo") { MediaKind::Image } else { MediaKind::Video })
                        }
                        (None, Some(a)) => (*a, MediaKind::Audio),
                        _ => {
                            report.skip("missing material");
                            continue;
                        }
                    };
                    if kind == TrackKind::Video && mk == MediaKind::Audio {
                        report.skip("sound on a picture track");
                        continue;
                    }
                    let Some(media) = add_media(proj, mat, mk) else {
                        report.skip("material without a file");
                        continue;
                    };
                    let source_in =
                        if mk == MediaKind::Image { Tick::ZERO } else { micros(&seg["source_timerange"]["start"]) };
                    ClipContent::Media {
                        media,
                        source_in,
                        speed: if mk == MediaKind::Image { 1.0 } else { speed },
                        reversed: seg["reverse"].as_bool().unwrap_or(false),
                        curve: vec![],
                        silent: false,
                    }
                }
                TrackKind::Text => {
                    let Some(m) = texts.get(mat_id) else {
                        report.skip("missing material");
                        continue;
                    };
                    let (text, size, color) = text_of(m);
                    if text.trim().is_empty() {
                        continue;
                    }
                    // CapCut's default text size is 15; reelforge's default title is 6 % of the short side
                    let size = (size.unwrap_or(15.0) / 15.0 * title_px).clamp(8.0, 2000.0) as f32;
                    ClipContent::Text {
                        text,
                        size,
                        color: color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
                        style: TitleStyle::default(),
                    }
                }
            };
            let mut clip = blank_clip(proj.mint("c"), start, duration, content);
            let c = &seg["clip"];
            if !c.is_null() {
                let scale = num(&c["scale"]["x"], 1.0).max(num(&c["scale"]["y"], 1.0));
                clip.transform = ClipTransform {
                    x: (num(&c["transform"]["x"], 0.0) / 2.0) as f32,
                    y: (-num(&c["transform"]["y"], 0.0) / 2.0) as f32,
                    scale: scale.clamp(0.01, 100.0) as f32,
                    rotation: num(&c["rotation"], 0.0) as f32,
                };
                clip.opacity = num(&c["alpha"], 1.0).clamp(0.0, 1.0) as f32;
                if matches!(clip.content, ClipContent::Media { .. }) {
                    clip.look.flip_x = c["flip"]["horizontal"].as_bool().unwrap_or(false);
                    clip.look.flip_y = c["flip"]["vertical"].as_bool().unwrap_or(false);
                }
            }
            let volume = num(&seg["volume"], 1.0);
            clip.volume_db = if volume <= 0.0 { -60.0 } else { (20.0 * volume.log10()).clamp(-60.0, 24.0) as f32 };
            if let Some(f) = refs.iter().find_map(|r| fades.get(*r)) {
                let fade = |k: &str| micros(&f[k]).max(Tick::ZERO).min(duration);
                clip.fade = Fades { fade_in: fade("fade_in_duration"), fade_out: fade("fade_out_duration") };
            }
            // a transition on a segment leads into the next one
            if let Some(t) = pending.take()
                && track.clips.last().is_some_and(|prev| prev.end() == start)
            {
                clip.transition = Some(t);
            }
            if let Some(t) = refs.iter().find_map(|r| transitions.get(*r)) {
                let d = micros(&t["duration"]);
                if d > Tick::ZERO {
                    pending = Some(Transition { kind: TransitionKind::Dissolve, duration: d });
                }
            }
            if ["common_keyframes", "keyframe_refs"].iter().any(|k| seg[*k].as_array().is_some_and(|a| !a.is_empty())) {
                report.skip("keyframes");
            }
            // a clip that runs into the next is cut where the next begins
            if let Some(prev) = track.clips.last_mut()
                && prev.end() > start
            {
                if start <= prev.start {
                    report.skip("overlapping segment");
                    continue;
                }
                prev.duration = start - prev.start;
            }
            track.clips.push(clip);
            report.clips += 1;
        }
        match kind {
            TrackKind::Video => video_tracks.push(track),
            TrackKind::Text => text_tracks.push(track),
            TrackKind::Audio => audio_tracks.push(track),
        }
    }
    for list in ["stickers", "effects", "video_effects", "filters", "masks"] {
        let n = mats[list].as_array().map_or(0, Vec::len);
        if n > 0 {
            *report.skipped.entry(list.trim_start_matches("video_").to_string()).or_default() += n;
        }
    }
    let mut tracks = Vec::new();
    for (list, kind) in
        [(video_tracks, TrackKind::Video), (text_tracks, TrackKind::Text), (audio_tracks, TrackKind::Audio)]
    {
        if list.is_empty() {
            let name = match kind {
                TrackKind::Video => "Video 1",
                TrackKind::Text => "Text 1",
                TrackKind::Audio => "Audio 1",
            };
            tracks.push(Track {
                id: proj.mint("t"),
                kind,
                name: name.into(),
                hidden: false,
                muted: false,
                magnetic: false,
                locked: false,
                clips: vec![],
            });
        }
        tracks.extend(list);
    }
    proj.tracks = tracks;
    Ok(report)
}

/// `capcut.import`: the draft as a new timeline (or into an untouched project's only one).
pub(crate) fn capcut_import(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let text = p.str("draft")?;
    let folder = p.opt_str("folder")?.map(str::to_string);
    let draft: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| {
        EngineError::Params(
            "the draft is not JSON: 剪映 6 and later encrypt draft_info.json; use draft_content.json from CapCut or an earlier 剪映"
                .into(),
        )
    })?;
    if draft["tracks"].as_array().is_none() || !draft["materials"].is_object() {
        return Err(EngineError::Params("this is not a CapCut draft: it has no `tracks` and `materials`".into()));
    }
    let name = p
        .opt_str("name")?
        .map(str::to_string)
        .or_else(|| draft["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string))
        .unwrap_or_else(|| "CapCut draft".into());
    s.edit("Import CapCut Draft", |proj| {
        let fresh = proj.timelines.is_empty() && proj.tracks.iter().all(|t| t.clips.is_empty());
        if fresh {
            // an untouched project takes the draft's canvas too
            let w = draft["canvas_config"]["width"].as_u64().unwrap_or(0) as u32;
            let h = draft["canvas_config"]["height"].as_u64().unwrap_or(0) as u32;
            if w >= 16 && h >= 16 && w <= 8192 && h <= 8192 {
                proj.canvas.width = w - w % 2;
                proj.canvas.height = h - h % 2;
            }
            let fps = num(&draft["fps"], 0.0).round() as u32;
            if (1..=240).contains(&fps) {
                proj.canvas.fps = FrameRate { num: fps, den: 1 };
            }
            proj.name_first_timeline(&name);
        } else {
            proj.name_first_timeline("Timeline 1");
            let id = proj.mint("s");
            let at = proj.timelines.iter().position(|t| t.id == proj.timeline).map_or(proj.timelines.len(), |i| i + 1);
            proj.timelines.insert(
                at,
                reelforge_model::Timeline {
                    id: id.clone(),
                    name: name.clone(),
                    tracks: vec![],
                    markers: vec![],
                    cover: None,
                    compound: false,
                },
            );
            proj.switch_timeline(&id);
        }
        let report = read_draft(proj, &draft, folder.as_deref())?;
        let offline = proj
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| match &c.content {
                ClipContent::Media { media, .. } => proj.media.get(media).filter(|m| m.sha256.is_none()),
                _ => None,
            })
            .map(|m| m.name.clone())
            .collect::<std::collections::BTreeSet<_>>();
        Ok(json!({
            "timeline": proj.timeline,
            "clips": report.clips,
            "skipped": report.skipped,
            "offline": offline,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_paths_resolve_against_the_draft_folder() {
        let p = "##_draftpath_placeholder_0E685133-18CE-45ED-8CB8-2904A212EC80_##/assets/a.mp4";
        assert_eq!(resolve(p, Some("/drafts/one/")), "/drafts/one/assets/a.mp4");
        assert_eq!(resolve("/abs/b.mp4", Some("/x")), "/abs/b.mp4");
    }

    #[test]
    fn text_comes_from_json_or_markup() {
        let (t, size, color) = text_of(
            &json!({"content": r#"{"text":"你好","styles":[{"size":30,"fill":{"content":{"solid":{"color":[1,0,0]}}}}]}"#}),
        );
        assert_eq!((t.as_str(), size, color), ("你好", Some(30.0), Some([1.0, 0.0, 0.0, 1.0])));
        let (t, _, color) = text_of(
            &json!({"content": "<font id=\"\" path=\"x.ttf\"><color=(1,1,1,1)>[Hi there]</color></font>", "text_color": "#FF00FF00"}),
        );
        assert_eq!(t, "Hi there");
        assert_eq!(color, Some([0.0, 1.0, 0.0, 1.0]));
    }
}
