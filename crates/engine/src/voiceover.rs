//! Voice-over: lines read aloud (sound files a speech model wrote) imported and placed on
//! Voice-over audio tracks, each where its line starts. A line that would overlap a clip already
//! there goes on the next Voice-over track, made when there is none.

use reelforge_model::{ClipContent, Project, Track, TrackKind};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{not_found, one_step};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// The name voice-over tracks get.
pub const VOICEOVER_TRACK: &str = "Voice-over";

/// A title to be read aloud.
#[derive(Clone, Debug, PartialEq)]
pub struct Spoken {
    /// The title clip.
    pub clip: String,
    /// Where it starts, and where its voice-over goes.
    pub start: Tick,
    /// Its words, on one line.
    pub text: String,
}

/// What a voice-over reads: the titles `clips`, or with none, the captions. In timeline order;
/// titles with no words are left out.
pub fn spoken_lines(proj: &Project, clips: &[String]) -> Result<Vec<Spoken>, String> {
    let mut out = Vec::new();
    let mut take = |c: &reelforge_model::Clip| {
        if let ClipContent::Text { text, .. } = &c.content {
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                out.push(Spoken { clip: c.id.clone(), start: c.start, text });
            }
            true
        } else {
            false
        }
    };
    if clips.is_empty() {
        let track = proj
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Text && t.name == crate::CAPTIONS_TRACK)
            .ok_or("select the titles to read, or add captions first")?;
        track.clips.iter().for_each(|c| {
            take(c);
        });
    } else {
        for id in clips {
            let (_, c) = proj.clip(id).ok_or(format!("no clip {id}"))?;
            if !take(c) {
                return Err(format!("clip {id} is not a title"));
            }
        }
    }
    out.sort_by_key(|l| l.start);
    Ok(out)
}

struct Line {
    path: String,
    start: Tick,
    name: Option<String>,
    sha256: Option<String>,
    duration: Option<Tick>,
}

pub(crate) fn voiceover_add(s: &mut Session, v: &Value) -> CmdResult {
    let fps = crate::commands::fps(s);
    let p = P::new(v, fps);
    let list =
        p.opt("lines").and_then(Value::as_array).ok_or(EngineError::Params("`lines` must be an array".into()))?;
    let mut lines = Vec::with_capacity(list.len());
    for (i, l) in list.iter().enumerate() {
        let lp = P::new(l, fps);
        let field = |e: EngineError| {
            EngineError::Params(format!("lines[{i}]: {}", e.to_string().trim_start_matches("invalid parameters: ")))
        };
        lines.push(Line {
            path: lp.str("path").map_err(field)?.to_string(),
            start: lp.time("start").map_err(field)?,
            name: lp.opt_str("name").map_err(field)?.map(str::to_string),
            sha256: lp.opt_str("sha256").map_err(field)?.map(str::to_string),
            duration: lp.opt_time("duration").map_err(field)?,
        });
    }
    if lines.is_empty() {
        return Err(EngineError::Params("`lines` is empty".into()));
    }
    let track = p.opt_str("track")?.map(str::to_string);
    if let Some(t) = &track {
        let project = s.project();
        let tr = project.track(t).ok_or_else(|| not_found("track", t))?;
        if tr.kind != TrackKind::Audio {
            return Err(EngineError::Params(format!("track {t} is not an audio track")));
        }
    }
    one_step(s, "Add Voice-over", |s| {
        let mut clips = Vec::new();
        let mut tracks: Vec<String> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let mut import = json!({ "path": l.path, "kind": "audio" });
            for (key, value) in [("name", &l.name), ("sha256", &l.sha256)] {
                if let Some(value) = value {
                    import[key] = json!(value);
                }
            }
            if let Some(d) = l.duration {
                import["duration"] = json!(d.0);
            }
            let fail = |e: EngineError| EngineError::Refused(format!("lines[{i}]: {e}"));
            let imported = s.execute("media.import", &import).map_err(fail)?;
            let media = imported["media"]["id"].as_str().expect("an import names its media").to_string();
            let length = s.project().media[&media]
                .duration
                .ok_or_else(|| EngineError::Refused(format!("lines[{i}]: cannot tell how long {} is", l.path)))?;
            let on = match &track {
                Some(t) => t.clone(),
                None => match free_track(&s.project(), l.start, l.start + length) {
                    Some(t) => t,
                    None => s.edit("Add Track", |proj| Ok(new_track(proj)))?,
                },
            };
            let added =
                s.execute("clip.add", &json!({ "track": on, "media": media, "start": l.start.0 })).map_err(fail)?;
            clips.push(added["clip"].clone());
            if !tracks.contains(&on) {
                tracks.push(on);
            }
        }
        Ok(json!({ "clips": clips, "tracks": tracks }))
    })
}

/// The first Voice-over track with nothing between `start` and `end`.
fn free_track(proj: &Project, start: Tick, end: Tick) -> Option<String> {
    proj.tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && t.name.starts_with(VOICEOVER_TRACK) && !t.locked)
        .find(|t| t.clips.iter().all(|c| c.end() <= start || c.start >= end))
        .map(|t| t.id.clone())
}

/// A new Voice-over track below the other sound tracks; the second is "Voice-over 2", and so on.
fn new_track(proj: &mut Project) -> String {
    let n = proj.tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.name.starts_with(VOICEOVER_TRACK)).count();
    let name = if n == 0 { VOICEOVER_TRACK.to_string() } else { format!("{VOICEOVER_TRACK} {}", n + 1) };
    let id = proj.mint("t");
    let at = proj.tracks.iter().rposition(|t| t.kind == TrackKind::Audio).map_or(proj.tracks.len(), |i| i + 1);
    proj.tracks.insert(
        at,
        Track {
            id: id.clone(),
            kind: TrackKind::Audio,
            name,
            hidden: false,
            muted: false,
            magnetic: false,
            locked: false,
            clips: vec![],
        },
    );
    id
}
