//! Taking time out of the timeline: stretches cut from every track at once so that what is left
//! stays in step, and the captions that are only a filler word, which are such stretches.

use reelforge_model::{ClipContent, Project};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::commands::{fps, not_found, split, trim};
use crate::params::{P, parse_time};
use crate::{CmdResult, EngineError, Session};

/// `ranges` in order, with the ones that touch or overlap joined; empty ones are dropped.
fn merged(ranges: &[(Tick, Tick)]) -> Vec<(Tick, Tick)> {
    let mut sorted: Vec<(Tick, Tick)> =
        ranges.iter().map(|(a, b)| ((*a).max(Tick::ZERO), *b)).filter(|(a, b)| a < b).collect();
    sorted.sort();
    let mut out: Vec<(Tick, Tick)> = Vec::new();
    for (a, b) in sorted {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Cuts `ranges` of time out of the tracks named (every track that is not locked by default):
/// what lies in a range goes, a clip reaching into one is cut there, and what comes after moves
/// up by its length. Returns the ranges cut, joined, and how much time went.
pub(crate) fn cut(
    proj: &mut Project,
    ranges: &[(Tick, Tick)],
    tracks: Option<&[String]>,
) -> Result<(Vec<(Tick, Tick)>, Tick), EngineError> {
    if let Some(ids) = tracks {
        for id in ids {
            proj.track(id).ok_or_else(|| not_found("track", id))?;
        }
    }
    let ranges = merged(ranges);
    let chosen: Vec<usize> = (0..proj.tracks.len())
        .filter(|&i| match tracks {
            Some(ids) => ids.contains(&proj.tracks[i].id),
            None => !proj.tracks[i].locked,
        })
        .collect();
    // the last first, so the ones before it are still where they were
    for &(a, b) in ranges.iter().rev() {
        let len = b - a;
        for &ti in &chosen {
            // a clip that runs through the whole range is two clips from here on
            let through: Vec<String> =
                proj.tracks[ti].clips.iter().filter(|c| c.start < a && c.end() > b).map(|c| c.id.clone()).collect();
            for id in through {
                split(proj, &id, a)?;
            }
            let mut kept = Vec::new();
            for mut c in std::mem::take(&mut proj.tracks[ti].clips) {
                let (start, end) = (c.start, c.end());
                if end <= a {
                    // before it
                } else if c.start >= b {
                    c.start -= len;
                } else if c.start >= a && end <= b {
                    continue;
                } else if c.start < a {
                    trim(&mut c, start, a)?;
                } else {
                    trim(&mut c, b, end)?;
                    c.start = a;
                }
                kept.push(c);
            }
            proj.tracks[ti].clips = kept;
        }
        // markers belong to the whole timeline: they move only when all of it does
        if tracks.is_none() {
            proj.markers.retain(|m| m.at <= a || m.at >= b);
            for m in proj.markers.iter_mut().filter(|m| m.at >= b) {
                m.at -= len;
            }
            // the cover stays on its frame, or goes to where its frame was cut out
            proj.cover = proj.cover.map(|c| if c >= b { c - len } else { c.min(a) });
        }
    }
    crate::compound::tidy_groups(proj);
    let removed = ranges.iter().fold(Tick::ZERO, |sum, (a, b)| sum + (*b - *a));
    Ok((ranges, removed))
}

fn ranges_param(v: &Value, fps: reelforge_time::FrameRate) -> Result<Vec<(Tick, Tick)>, EngineError> {
    let wrong = || EngineError::Params("`ranges` must be a list of {\"from\": time, \"to\": time}".into());
    let list = v.get("ranges").and_then(Value::as_array).ok_or_else(wrong)?;
    let mut out = Vec::new();
    for r in list {
        let end = |name: &str| r.get(name).ok_or_else(wrong).and_then(|t| parse_time(t, fps).map_err(|_| wrong()));
        let (from, to) = (end("from")?, end("to")?);
        if to <= from {
            return Err(EngineError::Params("a range ends after it starts".into()));
        }
        out.push((from, to));
    }
    if out.is_empty() {
        return Err(EngineError::Params("`ranges` is empty".into()));
    }
    Ok(out)
}

fn tracks_param(p: &P) -> Result<Option<Vec<String>>, EngineError> {
    p.opt("tracks").map(|_| p.strings("tracks")).transpose()
}

fn ranges_json(ranges: &[(Tick, Tick)]) -> Vec<Value> {
    ranges.iter().map(|(a, b)| json!({ "from": a, "to": b })).collect()
}

pub(crate) fn timeline_cut(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ranges = ranges_param(v, p.fps)?;
    let tracks = tracks_param(&p)?;
    s.edit("Cut Out Time", |proj| {
        let (ranges, removed) = cut(proj, &ranges, tracks.as_deref())?;
        Ok(json!({ "ranges": ranges_json(&ranges), "removed": removed, "duration": proj.duration() }))
    })
}

/// Words said while thinking of the next one, in Chinese, English and Japanese. A caption that is
/// made of nothing else says nothing.
const FILLERS: &[&str] = &[
    "嗯",
    "呃",
    "额",
    "啊",
    "哦",
    "噢",
    "唔",
    "呐",
    "哎",
    "诶",
    "欸",
    "那个",
    "这个",
    "就是",
    "就是说",
    "然后",
    "然后呢",
    "那么",
    "怎么说",
    "怎么说呢",
    "um",
    "uh",
    "er",
    "erm",
    "ah",
    "eh",
    "hm",
    "mm",
    "like",
    "you know",
    "i mean",
    "well",
    "so",
    "えーと",
    "えっと",
    "ええと",
    "えー",
    "えーっと",
    "あの",
    "あのー",
    "あのう",
    "その",
    "そのー",
    "まあ",
    "まぁ",
    "なんか",
    "うーん",
];

/// `text` as it is compared: lower case, anything that is not a letter or a digit a single space,
/// and a letter said long (`ummm`, `えーー`) said once.
fn plain(text: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = if c.is_alphanumeric() { c } else { ' ' };
        let long = c.is_ascii_alphabetic() || c == 'ー' || c == ' ';
        if out.last() == Some(&c) && long {
            continue;
        }
        out.push(c);
    }
    while out.last() == Some(&' ') {
        out.pop();
    }
    let lead = out.iter().take_while(|c| **c == ' ').count();
    out.drain(..lead);
    out
}

/// Whether `text` is filler words and nothing else: the built-in ones and `more`.
pub(crate) fn only_fillers(text: &str, more: &[String]) -> bool {
    let text = plain(text);
    if text.is_empty() {
        return false;
    }
    let words: Vec<Vec<char>> =
        FILLERS.iter().map(|w| plain(w)).chain(more.iter().map(|w| plain(w))).filter(|w| !w.is_empty()).collect();
    // from[i]: the text from i on is fillers and nothing else
    let mut from = vec![false; text.len() + 1];
    from[text.len()] = true;
    for i in (0..text.len()).rev() {
        if text[i] == ' ' {
            from[i] = from[i + 1];
            continue;
        }
        // a word in Latin letters starts and ends where a word does
        let starts = i == 0 || text[i - 1] == ' ' || !text[i].is_ascii_alphanumeric();
        from[i] = starts
            && words.iter().any(|w| {
                let end = i + w.len();
                text[i..].starts_with(w)
                    && from[end]
                    && (end == text.len() || text[end] == ' ' || !text[end - 1].is_ascii_alphanumeric())
            });
    }
    from[0]
}

/// The captions of `track` that are only filler words: `(clip, text, start, end)`.
fn fillers(proj: &Project, track: &str, more: &[String]) -> Vec<(String, String, Tick, Tick)> {
    let clips = proj.track(track).map(|t| t.clips.as_slice()).unwrap_or_default();
    clips
        .iter()
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } if only_fillers(text, more) => {
                Some((c.id.clone(), text.clone(), c.start, c.end()))
            }
            _ => None,
        })
        .collect()
}

fn fillers_json(found: &[(String, String, Tick, Tick)]) -> Vec<Value> {
    found.iter().map(|(clip, text, from, to)| json!({ "clip": clip, "text": text, "from": from, "to": to })).collect()
}

fn words_param(p: &P) -> Result<Vec<String>, EngineError> {
    Ok(p.opt("words").map(|_| p.strings("words")).transpose()?.unwrap_or_default())
}

pub(crate) fn captions_fillers(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let project = s.project();
    let track = crate::captions::target(&project, p.opt_str("track")?)?;
    let found = fillers(&project, &track, &words_param(&p)?);
    let time = found.iter().fold(Tick::ZERO, |sum, f| sum + (f.3 - f.2));
    Ok(json!({ "track": track, "fillers": fillers_json(&found), "time": time }))
}

pub(crate) fn captions_cut_fillers(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let track = p.opt_str("track")?.map(str::to_string);
    let words = words_param(&p)?;
    let tracks = tracks_param(&p)?;
    s.edit("Cut Filler Words", |proj| {
        let track = crate::captions::target(proj, track.as_deref())?;
        let found = fillers(proj, &track, &words);
        if found.is_empty() {
            return Err(EngineError::Refused("no caption on this track is only a filler word".into()));
        }
        let ranges: Vec<(Tick, Tick)> = found.iter().map(|f| (f.2, f.3)).collect();
        let (_, removed) = cut(proj, &ranges, tracks.as_deref())?;
        Ok(json!({ "track": track, "fillers": fillers_json(&found), "removed": removed, "duration": proj.duration() }))
    })
}

#[cfg(test)]
mod tests {
    use super::only_fillers;

    #[test]
    fn a_caption_of_filler_words_is_told_from_one_that_says_something() {
        let none: &[String] = &[];
        for text in [
            "嗯",
            "嗯……",
            "呃，那个",
            "那个那个，就是",
            "然后呢？",
            "Um",
            "Ummm...",
            "uh, you know",
            "I mean, like",
            "えーーと",
            "あのー、まあ",
        ] {
            assert!(only_fillers(text, none), "{text}");
        }
        for text in [
            "",
            "……",
            "嗯，好的",
            "那个人",
            "就是这样",
            "然后我们出发",
            "so far",
            "likely",
            "summer",
            "you know what",
            "あの人",
            "♪",
        ] {
            assert!(!only_fillers(text, none), "{text}");
        }
        // words of one's own count too
        assert!(only_fillers("对吧，嗯", &["对吧".to_string()]));
        assert!(only_fillers("Right, um", &["right".to_string()]));
    }
}
