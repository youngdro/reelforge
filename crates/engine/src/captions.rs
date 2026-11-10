//! Captions: SubRip (`.srt`) in and out, and cues placed as titles on a captions track. Speech
//! recognition hands its cues to the same placement.

use reelforge_model::{Clip, ClipContent, ClipTransform, Project, TitleStyle, Track, TrackKind};
use reelforge_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

use crate::params::P;
use crate::templates::title_template;
use crate::{CmdResult, EngineError, Session};

/// One caption: words on screen from `start` to `end`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// When it appears.
    pub start: Tick,
    /// When it goes.
    pub end: Tick,
    /// The words; `\n` breaks a line.
    pub text: String,
}

/// The name captions tracks get.
pub const CAPTIONS_TRACK: &str = "Captions";
/// The name the track of a second language's captions gets.
pub const CAPTIONS_TRACK_2: &str = "Captions 2";

const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

/// Reads SubRip text. Cue numbers are optional; blank lines separate cues; a cue without a time
/// line is an error naming its line.
pub fn parse_srt(srt: &str) -> Result<Vec<Cue>, String> {
    let srt = srt.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    let mut line_no = 0;
    for block in srt.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let first = line_no + 1;
        line_no += lines.len() + 1;
        let lines: Vec<&str> = lines.into_iter().skip_while(|l| l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        let at = lines.iter().position(|l| l.contains("-->")).ok_or(format!("line {first}: no `-->` time line"))?;
        if at > 1 {
            return Err(format!("line {first}: expected a cue number and a time line"));
        }
        let (a, b) = lines[at].split_once("-->").expect("found above");
        let start = srt_time(a.trim()).ok_or(format!("line {}: bad start time {:?}", first + at, a.trim()))?;
        // anything after the end time (position hints) is ignored
        let b = b.split_whitespace().next().unwrap_or("");
        let end = srt_time(b).ok_or(format!("line {}: bad end time {b:?}", first + at))?;
        if end <= start {
            return Err(format!("line {}: the cue ends before it starts", first + at));
        }
        let text = lines[at + 1..].iter().map(|l| strip_tags(l.trim())).collect::<Vec<_>>().join("\n");
        cues.push(Cue { start, end, text: text.trim().to_string() });
    }
    Ok(cues)
}

/// `hh:mm:ss,mmm` (a `.` works too, hours may be omitted).
fn srt_time(s: &str) -> Option<Tick> {
    let (hms, ms) = s.split_once([',', '.']).unwrap_or((s, "0"));
    let parts: Vec<i64> = hms.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let (h, m, sec) = match parts[..] {
        [h, m, s] => (h, m, s),
        [m, s] => (0, m, s),
        _ => return None,
    };
    let digits = ms.trim();
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ms: i64 = format!("{digits:0<3}").parse().ok()?;
    if m >= 60 || sec >= 60 || h < 0 || m < 0 || sec < 0 {
        return None;
    }
    Some(Tick(((h * 3600 + m * 60 + sec) * 1000 + ms) * TICKS_PER_MS))
}

/// Drops `<i>`-style tags and `{\an8}`-style overrides.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut skip: Option<char> = None;
    for c in s.chars() {
        match (skip, c) {
            (None, '<') => skip = Some('>'),
            (None, '{') => skip = Some('}'),
            (Some(end), c) if c == end => skip = None,
            (Some(_), _) => {}
            (None, c) => out.push(c),
        }
    }
    out
}

/// Writes cues as SubRip, numbered from 1, times rounded to the millisecond.
pub fn format_srt(cues: &[Cue]) -> String {
    let t = |t: Tick| {
        let ms = (t.0 + TICKS_PER_MS / 2).max(0) / TICKS_PER_MS;
        format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
    };
    cues.iter()
        .enumerate()
        .map(|(i, c)| format!("{}\n{} --> {}\n{}\n", i + 1, t(c.start), t(c.end), c.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Places `cues` as titles on `track` (a new captions track when `None`), in `template`'s look,
/// low on the frame. Cues that overlap are cut where the next begins; empty ones are skipped.
/// `second` places them as a second language: smaller, a line below where the first sits, on a
/// track of their own. Returns the track and the new clips.
pub(crate) fn place(
    proj: &mut Project,
    cues: &[Cue],
    track: Option<&str>,
    template: &str,
    second: bool,
) -> Result<(String, Vec<String>), EngineError> {
    let look =
        title_template(template).ok_or_else(|| EngineError::Params(format!("no title template {template:?}")))?;
    let track_id = match track {
        Some(t) => {
            let tr = proj.track(t).ok_or_else(|| crate::commands::not_found("track", t))?;
            if tr.kind != TrackKind::Text {
                return Err(EngineError::Params(format!("track {t} is not a text track")));
            }
            t.to_string()
        }
        None => {
            let id = proj.mint("t");
            let at = proj.tracks.iter().rposition(|t| t.kind == TrackKind::Text).map_or(proj.tracks.len(), |i| i + 1);
            let track = Track {
                id: id.clone(),
                kind: TrackKind::Text,
                name: if second { CAPTIONS_TRACK_2 } else { CAPTIONS_TRACK }.into(),
                hidden: false,
                muted: false,
                magnetic: false,
                locked: false,
                clips: vec![],
            };
            proj.tracks.insert(at, track);
            id
        }
    };
    let first = (proj.canvas.width.min(proj.canvas.height) as f32 * 0.06).round().max(12.0);
    // a second language is two thirds the size, its line right under the first's
    let (size, y) = if second {
        ((first * 0.68).round().max(10.0), 0.32 + first * 1.3 / proj.canvas.height.max(1) as f32)
    } else {
        (first, 0.32)
    };
    let mut sorted: Vec<&Cue> = cues.iter().filter(|c| !c.text.trim().is_empty()).collect();
    sorted.sort_by_key(|c| c.start);
    let mut ids = Vec::new();
    let mut clips = Vec::new();
    for (i, c) in sorted.iter().enumerate() {
        let end = sorted.get(i + 1).map_or(c.end, |n| c.end.min(n.start));
        if end <= c.start {
            continue;
        }
        let id = proj.mint("c");
        ids.push(id.clone());
        clips.push(Clip {
            id,
            start: c.start,
            duration: end - c.start,
            content: ClipContent::Text {
                text: c.text.clone(),
                size,
                color: look.color,
                style: TitleStyle { wrap: 0.86, ..look.style.clone() },
            },
            transform: ClipTransform { y, ..ClipTransform::default() },
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
        });
    }
    let track = proj.tracks.iter_mut().find(|t| t.id == track_id).expect("found or made above");
    track.clips.extend(clips);
    track.clips.sort_by_key(|c| c.start);
    Ok((track_id, ids))
}

/// The titles on `track` as cues, in time order.
pub(crate) fn cues_of(proj: &Project, track: &str) -> Result<Vec<Cue>, EngineError> {
    let tr = proj.track(track).ok_or_else(|| crate::commands::not_found("track", track))?;
    Ok(tr
        .clips
        .iter()
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } => Some(Cue { start: c.start, end: c.end(), text: text.clone() }),
            _ => None,
        })
        .collect())
}

/// The captions track: the last text track named so, else the last text track.
pub(crate) fn captions_track(proj: &Project) -> Option<String> {
    let text = || proj.tracks.iter().rev().filter(|t| t.kind == TrackKind::Text);
    text().find(|t| t.name == CAPTIONS_TRACK).or_else(|| text().next()).map(|t| t.id.clone())
}

pub(crate) fn captions_import(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let cues = parse_srt(p.str("srt")?).map_err(EngineError::Params)?;
    let track = p.opt_str("track")?.map(str::to_string);
    let template = p.opt_str("template")?.unwrap_or("classic").to_string();
    let second = match p.opt_u32("line")? {
        None | Some(1) => false,
        Some(2) => true,
        Some(n) => return Err(EngineError::Params(format!("`line` is 1 or 2, not {n}"))),
    };
    s.edit("Import Captions", |proj| {
        let (track, clips) = place(proj, &cues, track.as_deref(), &template, second)?;
        Ok(json!({ "track": track, "clips": clips }))
    })
}

pub(crate) fn captions_add(s: &mut Session, v: &Value) -> CmdResult {
    let fps = crate::commands::fps(s);
    let p = P::new(v, fps);
    let list = p.opt("cues").and_then(Value::as_array).ok_or(EngineError::Params("`cues` must be an array".into()))?;
    let mut cues = Vec::with_capacity(list.len());
    for (i, c) in list.iter().enumerate() {
        let cp = P::new(c, fps);
        let field = |e: EngineError| {
            EngineError::Params(format!("cues[{i}]: {}", e.to_string().trim_start_matches("invalid parameters: ")))
        };
        let (start, end) = (cp.time("start").map_err(field)?, cp.time("end").map_err(field)?);
        if end <= start {
            return Err(EngineError::Params(format!("cues[{i}]: ends before it starts")));
        }
        cues.push(Cue { start, end, text: cp.str("text").map_err(field)?.to_string() });
    }
    let track = p.opt_str("track")?.map(str::to_string);
    let template = p.opt_str("template")?.unwrap_or("classic").to_string();
    s.edit("Add Captions", |proj| {
        let (track, clips) = place(proj, &cues, track.as_deref(), &template, false)?;
        Ok(json!({ "track": track, "clips": clips }))
    })
}

pub(crate) fn captions_export(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let project = s.project();
    let track = match p.opt_str("track")? {
        Some(t) => t.to_string(),
        None => captions_track(&project).ok_or_else(|| EngineError::Params("there is no text track".into()))?,
    };
    let cues = cues_of(&project, &track)?;
    Ok(json!({ "track": track, "count": cues.len(), "srt": format_srt(&cues) }))
}

/// The track a captions command works on: the one named, which must hold titles, else the
/// captions track.
pub(crate) fn target(proj: &Project, track: Option<&str>) -> Result<String, EngineError> {
    match track {
        Some(t) => {
            let tr = proj.track(t).ok_or_else(|| crate::commands::not_found("track", t))?;
            if tr.kind != TrackKind::Text {
                return Err(EngineError::Params(format!("track {t} is not a text track")));
            }
            Ok(t.to_string())
        }
        None => captions_track(proj).ok_or_else(|| EngineError::Params("there is no text track".into())),
    }
}

pub(crate) fn captions_list(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let project = s.project();
    let track = target(&project, p.opt_str("track")?)?;
    let cues: Vec<Value> = project
        .track(&track)
        .expect("checked above")
        .clips
        .iter()
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } => {
                Some(json!({"clip": c.id, "start": c.start, "end": c.end(), "text": text}))
            }
            _ => None,
        })
        .collect();
    Ok(json!({ "track": track, "cues": cues }))
}

/// Where a caption of `chars` characters lasting `duration` is cut in time when its words are cut
/// after `at` characters: in proportion, on a frame, and at least a frame from either end.
fn split_time(start: Tick, duration: Tick, at: usize, chars: usize, fps: reelforge_time::FrameRate) -> Option<Tick> {
    let frame = fps.frame_duration();
    if duration < frame + frame {
        return None;
    }
    let share = Tick((duration.0 as i128 * at as i128 / chars.max(1) as i128) as i64);
    let t = fps.snap(start + share);
    Some(t.max(start + frame).min(start + duration - frame))
}

pub(crate) fn captions_split(s: &mut Session, v: &Value) -> CmdResult {
    let fps = crate::commands::fps(s);
    let p = P::new(v, fps);
    let id = p.str("clip")?.to_string();
    let at = p.opt_u32("at")?.map(|n| n as usize);
    let time = p.opt_time("time")?;
    s.edit("Split Caption", |proj| {
        let (_, clip) = proj.clip(&id).ok_or_else(|| crate::commands::not_found("clip", &id))?;
        let ClipContent::Text { text, .. } = &clip.content else {
            return Err(EngineError::Params(format!("clip {id} is not a title")));
        };
        let chars: Vec<char> = text.chars().collect();
        let (start, duration) = (clip.start, clip.duration);
        // the words are cut where asked, else where the time cut falls, else in the middle
        let inside = |n: usize| n.clamp(1, chars.len().saturating_sub(1).max(1));
        let at = match (at, time) {
            (Some(n), _) => n,
            (None, Some(t)) if t > start && t < start + duration => {
                inside(((t - start).0 as i128 * chars.len() as i128 / duration.0.max(1) as i128) as usize)
            }
            _ => inside(chars.len() / 2),
        };
        let left: String = chars[..at.min(chars.len())].iter().collect();
        let right: String = chars[at.min(chars.len())..].iter().collect();
        let (left, right) = (left.trim_end().to_string(), right.trim_start().to_string());
        if left.is_empty() || right.is_empty() {
            return Err(EngineError::Refused("a caption needs words on both sides of the cut".into()));
        }
        let cut = match time {
            Some(t) => t,
            None => split_time(start, duration, at, chars.len(), fps)
                .ok_or_else(|| EngineError::Refused("the caption is too short to split".into()))?,
        };
        let new = crate::commands::split(proj, &id, cut)?;
        for (clip, words) in [(&id, left), (&new, right)] {
            if let Some(ClipContent::Text { text, .. }) = proj.clip_mut(clip).map(|c| &mut c.content) {
                *text = words;
            }
        }
        Ok(json!({ "left": id, "right": new, "at": cut }))
    })
}

/// Joins two captions' words: with a space between Latin words, with nothing otherwise.
fn join(a: &str, b: &str) -> String {
    let (a, b) = (a.trim_end(), b.trim_start());
    let latin = |c: Option<char>| c.is_some_and(|c| c.is_ascii_graphic());
    let gap = if latin(a.chars().next_back()) && latin(b.chars().next()) { " " } else { "" };
    format!("{a}{gap}{b}")
}

pub(crate) fn captions_merge(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let ids = p.strings("clips")?;
    if ids.len() < 2 {
        return Err(EngineError::Params("`clips` needs two captions or more".into()));
    }
    s.edit("Merge Captions", |proj| {
        let (ti, _) = proj.clip(&ids[0]).ok_or_else(|| crate::commands::not_found("clip", &ids[0]))?;
        let track = &mut proj.tracks[ti];
        let mut parts: Vec<(Tick, Tick, String, String)> = Vec::new();
        for id in &ids {
            let c = track
                .clips
                .iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| EngineError::Params(format!("clip {id} is not on the same track as {}", ids[0])))?;
            let ClipContent::Text { text, .. } = &c.content else {
                return Err(EngineError::Params(format!("clip {id} is not a title")));
            };
            parts.push((c.start, c.end(), c.id.clone(), text.clone()));
        }
        parts.sort_by_key(|p| p.0);
        let (from, to) = (parts[0].0, parts.iter().map(|p| p.1).max().expect("two or more"));
        if track.clips.iter().any(|c| !ids.contains(&c.id) && c.start < to && c.end() > from) {
            return Err(EngineError::Refused("another clip lies between these captions".into()));
        }
        let words = parts.iter().skip(1).fold(parts[0].3.clone(), |all, p| join(&all, &p.3));
        let keep = parts[0].2.clone();
        track.clips.retain(|c| c.id == keep || !ids.contains(&c.id));
        let c = track.clips.iter_mut().find(|c| c.id == keep).expect("kept above");
        c.duration = to - from;
        if let ClipContent::Text { text, .. } = &mut c.content {
            *text = words;
        }
        Ok(json!({ "clip": keep, "removed": parts.iter().skip(1).map(|p| p.2.clone()).collect::<Vec<_>>() }))
    })
}

pub(crate) fn captions_replace(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let find = p.str("find")?.to_string();
    let with = p.str("replace")?.to_string();
    if find.is_empty() {
        return Err(EngineError::Params("`find` is empty".into()));
    }
    let track = target(&s.project(), p.opt_str("track")?)?;
    let hits = |proj: &Project| {
        proj.track(&track).map_or(0, |t| {
            t.clips
                .iter()
                .filter(|c| matches!(&c.content, ClipContent::Text { text, .. } if text.contains(&find)))
                .count()
        })
    };
    if hits(&s.project()) == 0 {
        // nothing to change is not an undo step
        return Ok(json!({ "track": track, "clips": [], "count": 0 }));
    }
    s.edit("Replace in Captions", |proj| {
        let t = proj.tracks.iter_mut().find(|t| t.id == track).expect("found above");
        let mut clips = Vec::new();
        let mut count = 0;
        for c in &mut t.clips {
            if let ClipContent::Text { text, .. } = &mut c.content {
                let n = text.matches(&find).count();
                if n > 0 {
                    *text = text.replace(&find, &with);
                    count += n;
                    clips.push(c.id.clone());
                }
            }
        }
        // a caption left with no words goes
        t.clips.retain(|c| !matches!(&c.content, ClipContent::Text { text, .. } if text.trim().is_empty()));
        Ok(json!({ "track": track, "clips": clips, "count": count }))
    })
}

pub(crate) fn captions_restyle(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let from = p.str("from")?.to_string();
    let track = p.opt_str("track")?.map(str::to_string);
    s.edit("Match Captions' Style", |proj| {
        let (ti, model) = proj.clip(&from).ok_or_else(|| crate::commands::not_found("clip", &from))?;
        let ClipContent::Text { size, color, style, .. } = &model.content else {
            return Err(EngineError::Params(format!("clip {from} is not a title")));
        };
        let (size, color, style) = (*size, *color, style.clone());
        let (transform, opacity, anim) = (model.transform, model.opacity, model.anim);
        let track = match &track {
            Some(t) => target(proj, Some(t))?,
            None => proj.tracks[ti].id.clone(),
        };
        let t = proj.tracks.iter_mut().find(|t| t.id == track).expect("found above");
        let mut clips = Vec::new();
        for c in t.clips.iter_mut().filter(|c| c.id != from) {
            if let ClipContent::Text { size: s, color: k, style: st, .. } = &mut c.content {
                (*s, *k, *st) = (size, color, style.clone());
                (c.transform, c.opacity, c.anim) = (transform, opacity, anim);
                clips.push(c.id.clone());
            }
        }
        Ok(json!({ "track": track, "clips": clips }))
    })
}

pub(crate) fn captions_shift(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, crate::commands::fps(s));
    let by = p.time("by")?;
    let track = target(&s.project(), p.opt_str("track")?)?;
    s.edit("Shift Captions", |proj| {
        let t = proj.tracks.iter_mut().find(|t| t.id == track).expect("found above");
        for c in &mut t.clips {
            c.start += by;
            if c.start < Tick::ZERO {
                return Err(EngineError::Refused("a caption would start before the timeline does".into()));
            }
        }
        Ok(json!({ "track": track, "count": t.clips.len(), "by": by }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>第一句</i>\r\n\r\n2\r\n00:00:02,000 --> 00:00:04,000 X1:0\r\n第二句\r\n两行\r\n\r\n";

    #[test]
    fn subrip_reads_and_writes() {
        let cues = parse_srt(SRT).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: Tick::SECOND, end: Tick::from_seconds(2.5), text: "第一句".into() });
        assert_eq!(cues[1].text, "第二句\n两行");
        let back = format_srt(&cues);
        assert_eq!(
            back,
            "1\n00:00:01,000 --> 00:00:02,500\n第一句\n\n2\n00:00:02,000 --> 00:00:04,000\n第二句\n两行\n"
        );
        assert_eq!(parse_srt(&back).unwrap(), cues);
        assert_eq!(srt_time("1:02.5"), Some(Tick::from_seconds(62.5)));
        assert_eq!(srt_time("01:00:00,000"), Some(Tick::SECOND * 3600));
    }

    #[test]
    fn broken_subrip_says_where() {
        assert!(parse_srt("1\nhello\n").unwrap_err().contains("line 1"));
        assert!(parse_srt("1\n00:00:01,000 --> 00:00:00,500\nx\n").unwrap_err().contains("ends before"));
        assert!(parse_srt("1\n00:00:01,000 --> 00:00:02,000\nok\n\n2\n00:61:00,000 --> 00:62:00,000\nx\n").is_err());
        assert_eq!(parse_srt("\n\n").unwrap(), vec![]);
    }
}
