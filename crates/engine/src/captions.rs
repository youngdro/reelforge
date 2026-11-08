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
/// Returns the track and the new clips.
pub(crate) fn place(
    proj: &mut Project,
    cues: &[Cue],
    track: Option<&str>,
    template: &str,
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
                name: CAPTIONS_TRACK.into(),
                hidden: false,
                muted: false,
                clips: vec![],
            };
            proj.tracks.insert(at, track);
            id
        }
    };
    let size = (proj.canvas.width.min(proj.canvas.height) as f32 * 0.06).round().max(12.0);
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
            transform: ClipTransform { y: 0.32, ..ClipTransform::default() },
            opacity: 1.0,
            volume_db: 0.0,
            effects: vec![],
            keys: Default::default(),
            anim: Default::default(),
            transition: None,
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
    s.edit("Import Captions", |proj| {
        let (track, clips) = place(proj, &cues, track.as_deref(), &template)?;
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
