//! Timeline templates (剪映's 剪同款): an edit made to be used again with other pictures.
//!
//! A template is an ordinary timeline in which some picture clips are *slots* ([`Clip::slot`]):
//! each stands for one of the person's own files, and keeps its place, its length and everything
//! set on it (effects, animations, transitions, colour) when that file takes its place. A slot
//! that is still a colour is empty. So any timeline can be made a template (`template.mark`), and
//! the built-in ones here are only timelines built by the same commands a person or an agent
//! would run, with a music bed from the sound library, titles, and colours where the pictures go.

use std::collections::BTreeMap;

use reelforge_model::{Clip, ClipContent, MediaKind, Project, Prop, TrackKind};
use reelforge_time::Tick;
use serde_json::{Value, json};

use crate::canvas::cover;
use crate::commands::{fps, not_found, one_step};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// One picture of a built-in template: how long it shows and what plays on it.
struct Shot {
    /// Seconds.
    len: f64,
    /// A picture effect (`fx.list`), or nothing.
    fx: &'static str,
    /// The transition into it from the shot before, and its seconds; nothing for a cut.
    cut: (&'static str, f64),
}

/// One title of a built-in template.
struct Line {
    text: &'static str,
    /// When it shows and for how long, in seconds.
    at: f64,
    len: f64,
    /// Em size in canvas pixels, and how far below the middle it sits (a fraction of the height).
    size: f32,
    y: f32,
    /// A title look (`title.templates`), or nothing for the plain one.
    look: &'static str,
    /// Letter spacing in em.
    tracking: f32,
    /// In and out animations, or nothing.
    enter: &'static str,
    exit: &'static str,
}

/// A sticker over the pictures.
struct Stuck {
    sticker: &'static str,
    at: f64,
    len: f64,
    place: [f32; 3],
    /// A looping animation, or nothing.
    repeat: &'static str,
}

/// A built-in template.
pub struct TimelineTemplate {
    /// Stable id, as commands take it.
    pub id: &'static str,
    /// English name.
    pub name: &'static str,
    /// Chinese name.
    pub name_zh: &'static str,
    /// What it is for, in English.
    pub about: &'static str,
    /// What it is for, in Chinese.
    pub about_zh: &'static str,
    /// The library music under it.
    pub music: &'static str,
    shots: &'static [Shot],
    lines: &'static [Line],
    stuck: &'static [Stuck],
    /// A filter over everything (`clip.setColor`), and effects over everything; nothing for none.
    filter: &'static str,
    over: &'static [&'static str],
}

impl TimelineTemplate {
    /// How many pictures it takes.
    pub fn slots(&self) -> usize {
        self.shots.len()
    }

    /// How long it runs, in seconds.
    pub fn seconds(&self) -> f64 {
        self.shots.iter().map(|s| s.len).sum()
    }

    /// How long each of its pictures shows, in seconds, in order.
    pub fn shots(&self) -> impl Iterator<Item = f64> + '_ {
        self.shots.iter().map(|s| s.len)
    }
}

const fn shot(len: f64, fx: &'static str, cut: &'static str, cut_len: f64) -> Shot {
    Shot { len, fx, cut: (cut, cut_len) }
}

/// The built-in templates, in the order a picker shows them.
pub fn timeline_templates() -> &'static [TimelineTemplate] {
    const NONE: (&str, f64) = ("", 0.0);
    static ALL: [TimelineTemplate; 4] = [
        TimelineTemplate {
            id: "beat",
            name: "Beat cuts",
            name_zh: "卡点快闪",
            about: "Ten pictures cut on the beat of an upbeat track, with a flash at every cut",
            about_zh: "十张画面踩着轻快的节拍切换，每次切换闪白",
            music: "upbeat",
            // the music is 120 beats a minute: every cut lands on a beat
            shots: &[
                Shot { len: 2.0, fx: "zoomIn", cut: NONE },
                shot(1.0, "zoomOut", "fadeWhite", 0.2),
                shot(1.0, "zoomIn", "fadeWhite", 0.2),
                shot(2.0, "zoomOut", "fadeWhite", 0.2),
                shot(1.0, "zoomIn", "fadeWhite", 0.2),
                shot(1.0, "zoomOut", "fadeWhite", 0.2),
                shot(2.0, "zoomIn", "fadeWhite", 0.2),
                shot(2.0, "zoomOut", "fadeWhite", 0.2),
                shot(2.0, "zoomIn", "fadeWhite", 0.2),
                shot(2.0, "zoomOut", "fadeWhite", 0.2),
            ],
            lines: &[
                Line {
                    text: "MY DAY",
                    at: 0.0,
                    len: 2.0,
                    size: 150.0,
                    y: 0.0,
                    look: "variety",
                    tracking: 0.0,
                    enter: "charPop",
                    exit: "fade",
                },
                Line {
                    text: "see you next time",
                    at: 14.0,
                    len: 2.0,
                    size: 72.0,
                    y: 0.3,
                    look: "classic",
                    tracking: 0.0,
                    enter: "charRise",
                    exit: "",
                },
            ],
            stuck: &[],
            filter: "vivid",
            over: &[],
        },
        TimelineTemplate {
            id: "vlog",
            name: "Vlog opener",
            name_zh: "Vlog 开场",
            about: "Six slow pictures that dissolve into each other over a lo-fi track, in warm colours",
            about_zh: "六张画面在低保真音乐里缓缓叠化，暖色调",
            music: "lofi",
            // one bar of the music each (85 beats a minute)
            shots: &[
                Shot { len: 2.824, fx: "zoomIn", cut: NONE },
                shot(2.824, "zoomOut", "dissolve", 0.6),
                shot(2.824, "zoomIn", "dissolve", 0.6),
                shot(2.824, "zoomOut", "dissolve", 0.6),
                shot(2.824, "zoomIn", "dissolve", 0.6),
                shot(2.824, "zoomOut", "dissolve", 0.6),
            ],
            lines: &[
                Line {
                    text: "VLOG",
                    at: 0.3,
                    len: 2.5,
                    size: 190.0,
                    y: -0.04,
                    look: "soft",
                    tracking: 0.12,
                    enter: "typewriter",
                    exit: "fade",
                },
                Line {
                    text: "a day in my life",
                    at: 1.0,
                    len: 1.8,
                    size: 60.0,
                    y: 0.07,
                    look: "",
                    tracking: 0.05,
                    enter: "fade",
                    exit: "fade",
                },
            ],
            stuck: &[],
            filter: "warm",
            over: &[],
        },
        TimelineTemplate {
            id: "cinematic",
            name: "Cinematic",
            name_zh: "电影感",
            about: "Four long pictures that fade through black, graded like film, with wide-set titles",
            about_zh: "四张长画面经黑场过渡，胶片色调，宽字距标题",
            music: "cinematic",
            shots: &[
                Shot { len: 4.0, fx: "zoomIn", cut: NONE },
                shot(4.0, "zoomIn", "fadeBlack", 0.8),
                shot(4.0, "zoomOut", "fadeBlack", 0.8),
                shot(4.0, "zoomIn", "fadeBlack", 0.8),
            ],
            lines: &[
                Line {
                    text: "A SHORT FILM",
                    at: 0.8,
                    len: 2.8,
                    size: 84.0,
                    y: 0.0,
                    look: "",
                    tracking: 0.4,
                    enter: "fade",
                    exit: "fade",
                },
                Line {
                    text: "THE END",
                    at: 13.5,
                    len: 2.5,
                    size: 72.0,
                    y: 0.0,
                    look: "",
                    tracking: 0.5,
                    enter: "fade",
                    exit: "",
                },
            ],
            stuck: &[],
            filter: "film",
            over: &["vignette"],
        },
        TimelineTemplate {
            id: "showcase",
            name: "Showcase",
            name_zh: "新品展示",
            about: "Six quick pictures that push each other out, with a boxed title and stickers that pop",
            about_zh: "六张画面互相推出，带底色标题和跳动的贴纸",
            music: "upbeat",
            shots: &[
                Shot { len: 2.0, fx: "zoomIn", cut: NONE },
                shot(2.0, "", "pushLeft", 0.3),
                shot(2.0, "pulse", "pushUp", 0.3),
                shot(2.0, "", "pushLeft", 0.3),
                shot(2.0, "pulse", "pushUp", 0.3),
                shot(2.0, "zoomOut", "zoom", 0.3),
            ],
            lines: &[
                Line {
                    text: "NEW IN",
                    at: 0.0,
                    len: 2.0,
                    size: 130.0,
                    y: -0.3,
                    look: "boxed",
                    tracking: 0.0,
                    enter: "pop",
                    exit: "slideUp",
                },
                Line {
                    text: "available now",
                    at: 10.0,
                    len: 2.0,
                    size: 80.0,
                    y: 0.3,
                    look: "label",
                    tracking: 0.0,
                    enter: "slideUp",
                    exit: "",
                },
            ],
            stuck: &[
                Stuck { sticker: "sparkle", at: 0.2, len: 1.8, place: [0.3, -0.38, 0.16], repeat: "breathe" },
                Stuck { sticker: "burst", at: 10.0, len: 2.0, place: [-0.3, 0.2, 0.2], repeat: "swing" },
            ],
            filter: "fresh",
            over: &[],
        },
    ];
    &ALL
}

/// The built-in template with this id.
pub fn timeline_template(id: &str) -> Option<&'static TimelineTemplate> {
    timeline_templates().iter().find(|t| t.id == id)
}

/// What a slot shows while it is empty: a different quiet colour for each.
pub fn slot_colour(n: u32) -> [f32; 4] {
    const TONES: [[f32; 3]; 6] = [
        [0.27, 0.33, 0.46],
        [0.44, 0.31, 0.42],
        [0.24, 0.40, 0.40],
        [0.46, 0.38, 0.27],
        [0.32, 0.30, 0.48],
        [0.28, 0.42, 0.31],
    ];
    let [r, g, b] = TONES[(n as usize + TONES.len() - 1) % TONES.len()];
    [r, g, b, 1.0]
}

/// One slot of the timeline being edited.
struct Slot {
    n: u32,
    clips: Vec<String>,
    start: Tick,
    /// The longest any of its clips shows.
    duration: Tick,
    /// The file in it; `None` while it is empty.
    media: Option<String>,
}

/// The slots of the timeline being edited, by number.
fn slots_of(proj: &Project) -> Vec<Slot> {
    let mut slots: BTreeMap<u32, Slot> = BTreeMap::new();
    for clip in proj.tracks.iter().flat_map(|t| &t.clips) {
        let Some(n) = clip.slot else { continue };
        let slot =
            slots.entry(n).or_insert(Slot { n, clips: vec![], start: clip.start, duration: Tick::ZERO, media: None });
        slot.clips.push(clip.id.clone());
        slot.start = slot.start.min(clip.start);
        slot.duration = slot.duration.max(clip.duration);
        if let ClipContent::Media { media, .. } = &clip.content {
            slot.media.get_or_insert_with(|| media.clone());
        }
    }
    slots.into_values().collect()
}

/// The slots of the timeline being edited, in order: each one's number, its first clip and
/// whether a file is in it.
pub fn slot_states(proj: &Project) -> Vec<(u32, String, bool)> {
    slots_of(proj)
        .into_iter()
        .map(|s| (s.n, s.clips.into_iter().next().unwrap_or_default(), s.media.is_some()))
        .collect()
}

pub(crate) fn slots_json(proj: &Project) -> Value {
    let slots = slots_of(proj);
    let texts: Vec<Value> = proj
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } => Some(json!({"clip": c.id, "text": text, "start": c.start})),
            _ => None,
        })
        .collect();
    json!({
        "slots": slots.iter().map(|s| json!({
            "slot": s.n,
            "clips": s.clips,
            "start": s.start,
            "duration": s.duration,
            "filled": s.media.is_some(),
            "media": s.media,
        })).collect::<Vec<_>>(),
        "empty": slots.iter().filter(|s| s.media.is_none()).count(),
        "texts": texts,
    })
}

/// Makes clip `id` show media `media` from `source_in`, in the same place for the same length
/// with everything set on it. A video too short for the clip is slowed down to fill it when
/// `slow`, and refused otherwise. What filled the frame goes on filling it: in place of a colour,
/// or of a picture scaled to cover the canvas, a picture of another shape is scaled to cover it.
pub(crate) fn replace(
    proj: &mut Project,
    id: &str,
    media: &str,
    source_in: Tick,
    slow: bool,
) -> Result<(), EngineError> {
    let item = proj.media.get(media).ok_or_else(|| not_found("media", media))?.clone();
    let (track, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
    let fits = matches!(
        (&clip.content, proj.tracks[track].kind, item.kind),
        (
            ClipContent::Media { .. } | ClipContent::Solid { .. },
            TrackKind::Video,
            MediaKind::Video | MediaKind::Image | MediaKind::Lottie
        ) | (ClipContent::Media { .. }, TrackKind::Audio, MediaKind::Audio | MediaKind::Video)
    );
    if !fits {
        return Err(EngineError::Refused(
            "a picture or a colour on a video track takes a video or an image, and a sound takes a sound".into(),
        ));
    }
    if source_in < Tick::ZERO {
        return Err(EngineError::Params("`sourceIn` must not be negative".into()));
    }
    let duration = clip.duration;
    let canvas = [proj.canvas.width, proj.canvas.height];
    // the scale at which what the clip shows now covers the canvas
    let covering = match &clip.content {
        ClipContent::Media { media, .. } => proj.media.get(media).and_then(|m| m.size).map(|size| cover(canvas, size)),
        _ => Some(1.0),
    };
    let grow = match (covering, item.size) {
        (Some(was), Some(size)) if (clip.transform.scale - was).abs() <= 0.01 * was => cover(canvas, size) / was,
        _ => 1.0,
    };
    // how the old file ran is part of the edit: the new one runs the same way
    let (mut speed, mut curve, silent) = match &clip.content {
        ClipContent::Media { speed, curve, silent, .. } if *speed > 0.0 => (*speed, curve.clone(), *silent),
        _ => (1.0, vec![], false),
    };
    match item.duration {
        Some(length) => {
            let room = length - source_in;
            if room <= Tick::ZERO {
                return Err(EngineError::Params("`sourceIn` is past the file's end".into()));
            }
            if source_in + duration.scale(speed) > length {
                if !slow {
                    return Err(EngineError::Refused(format!(
                        "{:?} is shorter than the clip needs ({:.2} s from there, {:.2} s needed): give `fit: \"slow\"` to slow it down to fill, or trim the clip first",
                        item.name,
                        room.seconds(),
                        duration.scale(speed).seconds()
                    )));
                }
                speed = room.0 as f64 / duration.0 as f64;
            }
        }
        // a still has no speed
        None => (speed, curve) = (1.0, vec![]),
    }
    let clip = proj.clip_mut(id).expect("found above");
    // what was asked of the old file's frames is asked of the new one's
    let frames = match &clip.content {
        ClipContent::Media { frames, .. } => *frames,
        _ => Default::default(),
    };
    clip.content =
        ClipContent::Media { media: media.to_string(), source_in, speed, reversed: false, curve, silent, frames };
    // another camera shook another way
    clip.steady = None;
    if grow != 1.0 {
        clip.transform.scale *= grow;
        for key in clip.keys.get_mut(&Prop::Scale).into_iter().flatten() {
            key.v *= grow;
        }
    }
    Ok(())
}

fn fit_param(p: &P) -> Result<Option<bool>, EngineError> {
    match p.opt_str("fit")? {
        None => Ok(None),
        Some("slow") => Ok(Some(true)),
        Some("refuse") => Ok(Some(false)),
        Some(_) => Err(EngineError::Params("`fit` must be \"slow\" or \"refuse\"".into())),
    }
}

pub(crate) fn clip_replace(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let (id, media) = (p.str("clip")?.to_string(), p.str("media")?.to_string());
    let source_in = p.opt_time("sourceIn")?.unwrap_or(Tick::ZERO);
    let slow = fit_param(&p)?.unwrap_or(false);
    s.edit("Replace", |proj| {
        replace(proj, &id, &media, source_in, slow)?;
        let speed = match &proj.clip(&id).expect("replaced").1.content {
            ClipContent::Media { speed, .. } => *speed,
            _ => 1.0,
        };
        Ok(json!({ "clip": id, "media": media, "speed": speed }))
    })
}

pub(crate) fn template_list(_: &mut Session, _: &Value) -> CmdResult {
    let all: Vec<Value> = timeline_templates()
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "name_zh": t.name_zh,
                "about": t.about,
                "about_zh": t.about_zh,
                "slots": t.slots(),
                "seconds": t.seconds(),
                "music": t.music,
            })
        })
        .collect();
    Ok(json!({ "templates": all }))
}

pub(crate) fn template_slots(s: &mut Session, _: &Value) -> CmdResult {
    Ok(slots_json(&s.project))
}

/// A list of ids under `name`, where an entry may be null; nothing given is an empty list.
fn picks(p: &P, name: &str) -> Result<Vec<Option<String>>, EngineError> {
    let wrong = || EngineError::Params(format!("`{name}` must be a list of ids (null skips a slot)"));
    match p.opt(name) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(list)) => list
            .iter()
            .map(|x| match x {
                Value::Null => Ok(None),
                Value::String(id) => Ok(Some(id.clone())),
                _ => Err(wrong()),
            })
            .collect(),
        Some(_) => Err(wrong()),
    }
}

/// Puts `media`, in order, into the slots from `from` on (each, filled or not), or, without
/// `from`, into the empty ones. Returns the slots filled and the files that found no slot.
fn fill(
    proj: &mut Project,
    media: &[Option<String>],
    from: Option<u32>,
    slow: bool,
) -> Result<(Vec<u32>, Vec<String>), EngineError> {
    let slots = slots_of(proj);
    if slots.is_empty() {
        return Err(EngineError::Refused("this timeline has no template slots (template.mark makes some)".into()));
    }
    let takers: Vec<&Slot> = match from {
        Some(n) => slots.iter().filter(|s| s.n >= n).collect(),
        None => slots.iter().filter(|s| s.media.is_none()).collect(),
    };
    let mut filled = vec![];
    for (slot, pick) in takers.iter().zip(media) {
        let Some(pick) = pick else { continue };
        for clip in &slot.clips {
            replace(proj, clip, pick, Tick::ZERO, slow)?;
        }
        filled.push(slot.n);
    }
    let left = media.iter().skip(takers.len()).flatten().cloned().collect();
    Ok((filled, left))
}

pub(crate) fn template_fill(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let media = picks(&p, "media")?;
    if media.iter().all(Option::is_none) {
        return Err(EngineError::Params("`media` must name the files to put in the slots".into()));
    }
    let from = p.opt_u32("from")?;
    if from == Some(0) {
        return Err(EngineError::Params("`from` is a slot's number, from 1".into()));
    }
    let slow = fit_param(&p)?.unwrap_or(true);
    s.edit("Fill Template", |proj| {
        let (filled, left) = fill(proj, &media, from, slow)?;
        let mut out = slots_json(proj);
        out["filled"] = json!(filled);
        out["left"] = json!(left);
        Ok(out)
    })
}

pub(crate) fn template_mark(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    // a number, null to make them ordinary clips again, or nothing for the next free numbers
    let wanted = match p.v.get("slot") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => match p.opt_u32("slot")? {
            Some(n) if n >= 1 => Some(Some(n)),
            _ => return Err(EngineError::Params("`slot` is a number from 1, or null".into())),
        },
    };
    s.edit("Template Slot", |proj| {
        // in the order they play, so the numbers follow the edit
        let mut clips: Vec<(Tick, String)> = Vec::new();
        for id in &ids {
            let (track, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            let picture = matches!(clip.content, ClipContent::Media { .. } | ClipContent::Solid { .. });
            if wanted != Some(None) && !(picture && proj.tracks[track].kind == TrackKind::Video) {
                return Err(EngineError::Refused(format!(
                    "{id} is not a picture on a video track: only those can be template slots"
                )));
            }
            clips.push((clip.start, id.clone()));
        }
        clips.sort();
        let mut next = slots_of(proj).last().map_or(1, |s| s.n + 1);
        let mut given = vec![];
        for (_, id) in &clips {
            let clip = proj.clip_mut(id).expect("found above");
            clip.slot = match wanted {
                Some(slot) => slot,
                // one that is a slot already keeps its number
                None => clip.slot.or_else(|| {
                    next += 1;
                    Some(next - 1)
                }),
            };
            given.push(json!({"clip": id, "slot": clip.slot}));
        }
        Ok(json!({ "clips": given }))
    })
}

fn secs(t: f64) -> Value {
    json!({ "seconds": t })
}

/// The id a command gave back under `key`.
fn made(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

/// Builds `t` on the timeline being edited, which is empty: the music, a colour for each slot,
/// the titles, and what lies over them. Returns the slots' clips.
fn build(s: &mut Session, t: &TimelineTemplate, texts: &[Option<String>]) -> Result<Vec<String>, EngineError> {
    let track = |s: &Session, kind: TrackKind| s.project.tracks.iter().find(|t| t.kind == kind).map(|t| t.id.clone());
    let need = |s: &mut Session, kind: TrackKind, name: &str| match track(s, kind) {
        Some(id) => Ok(id),
        None => Ok::<_, EngineError>(made(&s.execute("track.add", &json!({"kind": kind, "name": name}))?, "track")),
    };
    let video = need(s, TrackKind::Video, "Video 1")?;
    let text = need(s, TrackKind::Text, "Text 1")?;
    let total = t.seconds();

    // the pictures, each a slot
    let mut slots = vec![];
    let mut at = 0.0;
    for (i, shot) in t.shots.iter().enumerate() {
        let n = i as u32 + 1;
        let mut params =
            json!({"track": video, "color": slot_colour(n), "start": secs(at), "duration": secs(shot.len)});
        if !shot.fx.is_empty() {
            params["fx"] = json!([shot.fx]);
        }
        let clip = made(&s.execute("clip.add", &params)?, "clip");
        s.execute("template.mark", &json!({"clips": [clip], "slot": n}))?;
        if !shot.cut.0.is_empty() {
            let cut = json!({"clip": clip, "kind": shot.cut.0, "duration": secs(shot.cut.1)});
            s.execute("clip.setTransition", &cut)?;
        }
        slots.push(clip);
        at += shot.len;
    }

    // the music, cut to the pictures' length
    let sound = s.execute("sound.add", &json!({"sound": t.music, "at": 0}))?;
    let sound = made(&sound, "clip");
    let length = s.project.clip(&sound).map_or(0.0, |(_, c)| c.duration.seconds());
    if length > total {
        s.execute("clip.trim", &json!({"clip": sound, "end": secs(total)}))?;
    }
    s.execute("clip.set", &json!({"clip": sound, "fadeOut": secs(1.0), "volumeDb": -4.0}))?;

    // the titles; two that show at once sit on tracks of their own
    let mut rows: Vec<(String, f64)> = vec![(text, 0.0)];
    for (i, line) in t.lines.iter().enumerate() {
        let words = texts.get(i).cloned().flatten().unwrap_or_else(|| line.text.to_string());
        let row = match rows.iter().position(|(_, busy)| *busy <= line.at) {
            Some(row) => row,
            None => {
                let name = format!("Text {}", rows.len() + 1);
                let added = s.execute("track.add", &json!({"kind": "text", "name": name}))?;
                rows.push((made(&added, "track"), 0.0));
                rows.len() - 1
            }
        };
        rows[row].1 = line.at + line.len;
        let text = &rows[row].0;
        let mut params = json!({
            "track": text, "text": words, "size": line.size,
            "start": secs(line.at), "duration": secs(line.len), "tracking": line.tracking,
        });
        if !line.look.is_empty() {
            params["template"] = json!(line.look);
        }
        let clip = made(&s.execute("clip.add", &params)?, "clip");
        if line.y != 0.0 {
            s.execute("clip.set", &json!({"clip": clip, "y": line.y}))?;
        }
        for (slot, kind) in [("in", line.enter), ("out", line.exit)] {
            if !kind.is_empty() {
                s.execute("clip.setAnimation", &json!({"clip": clip, "slot": slot, "kind": kind}))?;
            }
        }
    }

    // stickers over the pictures
    if !t.stuck.is_empty() {
        let over = made(&s.execute("track.add", &json!({"kind": "video", "name": "Stickers"}))?, "track");
        for st in t.stuck {
            let params = json!({"track": over, "sticker": st.sticker, "start": secs(st.at), "duration": secs(st.len)});
            let clip = made(&s.execute("clip.add", &params)?, "clip");
            let [x, y, scale] = st.place;
            s.execute("clip.set", &json!({"clip": clip, "x": x, "y": y, "scale": scale}))?;
            s.execute("clip.setAnimation", &json!({"clip": clip, "slot": "in", "kind": "pop"}))?;
            if !st.repeat.is_empty() {
                s.execute("clip.setAnimation", &json!({"clip": clip, "slot": "loop", "kind": st.repeat}))?;
            }
        }
    }

    // one look over everything
    if !t.filter.is_empty() || !t.over.is_empty() {
        let look = made(&s.execute("track.add", &json!({"kind": "video", "name": "Look"}))?, "track");
        let params = json!({"track": look, "adjustment": true, "start": 0, "duration": secs(total), "fx": t.over});
        let clip = made(&s.execute("clip.add", &params)?, "clip");
        if !t.filter.is_empty() {
            s.execute("clip.setColor", &json!({"clip": clip, "filter": t.filter, "filterStrength": 0.7}))?;
        }
    }
    Ok(slots)
}

pub(crate) fn template_apply(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("template")?;
    let t = timeline_template(id).ok_or_else(|| {
        EngineError::Params(format!(
            "`template` must be one of {}",
            timeline_templates().iter().map(|t| t.id).collect::<Vec<_>>().join(", ")
        ))
    })?;
    let (media, texts) = (picks(&p, "media")?, picks(&p, "texts")?);
    let name = p.opt_str("name")?.unwrap_or(t.name).to_string();
    one_step(s, "Use Template", |s| {
        // on the timeline being edited when nothing is on it yet, else on one of its own
        if s.project.tracks.iter().any(|t| !t.clips.is_empty()) {
            s.execute("timeline.add", &json!({"name": name}))?;
        }
        let clips = build(s, t, &texts)?;
        if media.iter().any(Option::is_some) {
            s.execute("template.fill", &json!({"media": media, "from": 1}))?;
        }
        let mut out = slots_json(&s.project);
        out["template"] = json!(t.id);
        out["timeline"] = json!(s.project.timeline);
        out["clips"] = json!(clips);
        out["duration"] = json!(s.project.duration());
        Ok(out)
    })
}

/// Whether `clip` is a slot nothing was put in yet.
pub fn is_empty_slot(clip: &Clip) -> bool {
    clip.slot.is_some() && matches!(clip.content, ClipContent::Solid { .. })
}
