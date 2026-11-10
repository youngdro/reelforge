//! Motion on the beat (`beat.list`, `beat.apply`, `beat.cut`).
//!
//! The beat is a list of times: the timeline's beat markers (the ones `audio.beats` leaves), a
//! tempo, or the times themselves. A move is keys written at each of them: a clip jumps to a
//! peak on the beat and settles before the next, so a move is ordinary keys that can be edited or
//! cleared afterwards. Cutting on the beat lays clips end to end so that each ends on one.

use reelforge_model::{Clip, ClipContent, Ease, Key, Project, Prop, TrackKind};
use reelforge_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::commands::{fps, not_found, trim};
use crate::listen::BEAT;
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// Something a clip does on every beat.
pub struct Move {
    /// Its id.
    pub id: &'static str,
    /// What it is, in English.
    pub en: &'static str,
    /// In Chinese.
    pub zh: &'static str,
    /// The properties it keys.
    pub props: &'static [Prop],
    /// What it looks like.
    pub about: &'static str,
}

/// The moves `beat.apply` knows.
pub const MOVES: [Move; 8] = [
    Move {
        id: "punch",
        en: "Punch in",
        zh: "缩放冲击",
        props: &[Prop::Scale],
        about: "Jumps larger on the beat and settles back: the classic zoom hit.",
    },
    Move {
        id: "pulse",
        en: "Pulse",
        zh: "呼吸律动",
        props: &[Prop::Scale],
        about: "Swells up to each beat and down between them, without a jump.",
    },
    Move {
        id: "shake",
        en: "Shake",
        zh: "抖动",
        props: &[Prop::X, Prop::Y],
        about: "Rattles about its place for a few frames after the beat.",
    },
    Move {
        id: "flash",
        en: "Flash",
        zh: "闪白",
        props: &[Prop::Exposure],
        about: "Goes bright on the beat and comes back. For video, picture and compound clips.",
    },
    Move {
        id: "strobe",
        en: "Strobe",
        zh: "闪黑",
        props: &[Prop::Opacity],
        about: "Goes faint on the beat and comes back, showing what is behind it.",
    },
    Move {
        id: "tilt",
        en: "Tilt",
        zh: "摇摆",
        props: &[Prop::Rotation],
        about: "Kicks to one side on a beat and to the other on the next, settling upright.",
    },
    Move {
        id: "bounce", en: "Bounce", zh: "弹跳", props: &[Prop::Y], about: "Hops up from the beat and lands again."
    },
    Move {
        id: "squash",
        en: "Squash",
        zh: "压弹",
        props: &[Prop::ScaleX, Prop::ScaleY],
        about: "Goes wide and flat on the beat and springs back past its shape.",
    },
];

fn wrong(name: &str, what: &str) -> EngineError {
    EngineError::Params(format!("`{name}` must be {what}"))
}

/// `beat.list`.
pub(crate) fn beat_list(_: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({
        "moves": MOVES
            .iter()
            .map(|m| json!({
                "id": m.id,
                "name": m.en,
                "zh": m.zh,
                "props": m.props.iter().map(|p| p.name()).collect::<Vec<_>>(),
                "about": m.about,
            }))
            .collect::<Vec<_>>()
    }))
}

/// Where the beat is said to be.
#[derive(Clone)]
enum Source {
    /// The timeline's beat markers.
    Markers,
    /// So many beats a minute, the first one here.
    Tempo(f64, Tick),
    /// These times.
    Times(Vec<Tick>),
}

/// The beat a command is given, and which of its beats count.
struct Ask {
    source: Source,
    every: usize,
    offset: usize,
}

impl Ask {
    fn read(p: &P) -> Result<Ask, EngineError> {
        let small = |name: &str, range: (u32, u32), unless: u32| -> Result<usize, EngineError> {
            match p.opt_u32(name)? {
                Some(n) if n < range.0 || n > range.1 => Err(wrong(name, &format!("from {} to {}", range.0, range.1))),
                n => Ok(n.unwrap_or(unless) as usize),
            }
        };
        let (every, offset) = (small("every", (1, 16), 1)?, small("offset", (0, 15), 0)?);
        let first = p.opt_time("first")?;
        let source = match (p.opt("at"), p.opt_f64("bpm")?) {
            (Some(_), Some(_)) => return Err(EngineError::Params("give `at` or `bpm`, not both".into())),
            (Some(at), None) => {
                let what = "a list of times";
                let list = at.as_array().filter(|a| !a.is_empty()).ok_or_else(|| wrong("at", what))?;
                let times: Result<Vec<Tick>, _> = list.iter().map(|t| crate::params::parse_time(t, p.fps)).collect();
                Source::Times(times.map_err(|_| wrong("at", what))?)
            }
            (None, Some(bpm)) if !(20.0..=400.0).contains(&bpm) => return Err(wrong("bpm", "from 20 to 400")),
            (None, Some(bpm)) => Source::Tempo(bpm, first.unwrap_or(Tick::ZERO)),
            (None, None) => Source::Markers,
        };
        if first.is_some() && !matches!(source, Source::Tempo(..)) {
            return Err(EngineError::Params("`first` goes with `bpm`".into()));
        }
        Ok(Ask { source, every, offset })
    }

    /// The beats that count from `from` up to `until`, each at the start of a frame, in order.
    fn beats(&self, proj: &Project, from: Tick, until: Tick) -> Result<Vec<Tick>, EngineError> {
        let fps = proj.canvas.fps;
        let mut all: Vec<Tick> = match &self.source {
            Source::Markers => proj.markers.iter().filter(|m| m.note == BEAT).map(|m| m.at).collect(),
            Source::Times(times) => times.clone(),
            Source::Tempo(bpm, first) => {
                // counted in beats from the first, so that no rounding adds up
                let beat = Tick::SECOND.0 as f64 * 60.0 / bpm;
                let k0 = (((from - *first).0 as f64) / beat).ceil().max(0.0) as i64;
                (k0..)
                    .map(|k| *first + Tick((k as f64 * beat).round() as i64))
                    .take_while(|t| *t <= until)
                    .take(20_000)
                    .collect()
            }
        };
        if all.is_empty() && matches!(self.source, Source::Markers) {
            return Err(EngineError::Refused(
                "the timeline has no beat markers: mark the music's beats with audio.beats first, or give `bpm` (with `first`) or `at`".into(),
            ));
        }
        for t in all.iter_mut() {
            *t = fps.snap(*t);
        }
        all.sort();
        all.dedup();
        Ok(all.into_iter().filter(|t| *t >= from && *t <= until).skip(self.offset).step_by(self.every).collect())
    }
}

/// One beat as a clip meets it.
#[derive(Copy, Clone)]
struct Hit {
    /// When, from the clip's start.
    t: Tick,
    /// How long until the next beat, or the clip's end.
    gap: Tick,
    /// How long the clip takes to settle after it.
    settle: Tick,
    /// Which beat it is, from 0.
    n: usize,
}

/// The longest a clip takes to settle after a beat.
const SETTLE: Tick = Tick(Tick::SECOND.0 * 2 / 5);

/// The beats (from the clip's start, in order, within it) as hits. Beats too close together for a
/// move to fit between them are left out.
fn hits(beats: &[Tick], length: Tick, fps: FrameRate) -> Vec<Hit> {
    let frame = fps.frame_duration();
    let mut kept: Vec<Tick> = vec![];
    for t in beats {
        if *t + frame * 2 <= length && kept.last().is_none_or(|k| *t - *k >= frame * 3) {
            kept.push(*t);
        }
    }
    kept.iter()
        .enumerate()
        .map(|(n, t)| {
            let gap = kept.get(n + 1).copied().unwrap_or(length) - *t;
            // settled a frame at least before the next beat
            let settle = fps.snap(Tick((gap.0 as f64 * 0.6) as i64)).min(SETTLE).min(gap - frame);
            Hit { t: *t, gap, settle: settle.max(frame), n }
        })
        .collect()
}

/// A number from 0 to 1 that depends only on what it is given, for a shake that is the same
/// every time.
fn chance(a: usize, b: usize) -> f32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B1) ^ (b as u32).wrapping_mul(0x85EB_CA6B) ^ 0x2545_F491;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

fn key(t: Tick, v: f32, ease: Ease) -> Key {
    Key { t, v, ease }
}

/// A jump to `peak` on each beat that eases back to `rest`.
fn kicks(hits: &[Hit], rest: f32, ease: Ease, peak: impl Fn(&Hit) -> f32) -> Vec<Key> {
    let mut keys = vec![];
    if hits.first().is_some_and(|h| h.t > Tick::ZERO) {
        keys.push(key(Tick::ZERO, rest, Ease::Hold));
    }
    for h in hits {
        keys.push(key(h.t, peak(h), ease));
        keys.push(key(h.t + h.settle, rest, Ease::Hold));
    }
    keys
}

/// The keys of `mv` for a clip that meets `hits`; `k` is how strong, 1 as designed.
fn keys_of(mv: &Move, clip: &Clip, hits: &[Hit], k: f32, fps: FrameRate) -> Vec<(Prop, Vec<Key>)> {
    let frame = fps.frame_duration();
    let base = |p: Prop| clip.base(p);
    match mv.id {
        "punch" => {
            let rest = base(Prop::Scale);
            vec![(Prop::Scale, kicks(hits, rest, Ease::OUT, |_| rest * (1.0 + 0.14 * k)))]
        }
        "pulse" => {
            let rest = base(Prop::Scale);
            let peak = rest * (1.0 + 0.07 * k);
            let mut keys = vec![];
            for (i, h) in hits.iter().enumerate() {
                if i == 0 {
                    let rise = fps.snap(Tick(h.gap.0 / 2)).min(h.t);
                    if rise > Tick::ZERO {
                        keys.push(key(h.t - rise, rest, Ease::InOut));
                    }
                }
                keys.push(key(h.t, peak, Ease::InOut));
                // down halfway to the next beat, or as far past the last as a beat settles
                let fall = if i + 1 < hits.len() { fps.snap(Tick(h.gap.0 / 2)).max(frame) } else { h.settle };
                keys.push(key(h.t + fall, rest, Ease::InOut));
            }
            vec![(Prop::Scale, keys)]
        }
        "shake" => {
            let (x0, y0) = (base(Prop::X), base(Prop::Y));
            let (mut xs, mut ys) = (vec![], vec![]);
            if hits.first().is_some_and(|h| h.t > Tick::ZERO) {
                xs.push(key(Tick::ZERO, x0, Ease::Hold));
                ys.push(key(Tick::ZERO, y0, Ease::Hold));
            }
            for h in hits {
                let steps = (h.settle.0 / frame.0).clamp(1, 6) as usize;
                for i in 0..steps {
                    let far = 0.014 * k * (1.0 - i as f32 / steps as f32);
                    let turn = chance(h.n, i) * std::f32::consts::TAU;
                    let t = h.t + frame * i as i64;
                    xs.push(key(t, x0 + far * turn.cos(), Ease::Linear));
                    ys.push(key(t, y0 + far * turn.sin(), Ease::Linear));
                }
                let t = h.t + frame * steps as i64;
                xs.push(key(t, x0, Ease::Hold));
                ys.push(key(t, y0, Ease::Hold));
            }
            vec![(Prop::X, xs), (Prop::Y, ys)]
        }
        "flash" => {
            let rest = base(Prop::Exposure);
            vec![(Prop::Exposure, kicks(hits, rest, Ease::OUT, |_| (rest + 0.9 * k).min(3.0)))]
        }
        "strobe" => {
            let rest = base(Prop::Opacity);
            vec![(Prop::Opacity, kicks(hits, rest, Ease::OUT, |_| rest * (1.0 - 0.45 * k).max(0.05)))]
        }
        "tilt" => {
            let rest = base(Prop::Rotation);
            let side = |h: &Hit| if h.n.is_multiple_of(2) { 1.0 } else { -1.0 };
            vec![(Prop::Rotation, kicks(hits, rest, Ease::OUT, |h| rest + 3.5 * k * side(h)))]
        }
        "bounce" => {
            let rest = base(Prop::Y);
            let mut keys = vec![];
            for h in hits {
                keys.push(key(h.t, rest, Ease::OUT));
                if h.settle >= frame * 2 {
                    let up = fps.snap(Tick(h.settle.0 * 2 / 5)).max(frame);
                    keys.push(key(h.t + up, rest - 0.03 * k, Ease::IN));
                }
                keys.push(key(h.t + h.settle, rest, Ease::Hold));
            }
            vec![(Prop::Y, keys)]
        }
        "squash" => {
            let (w, h) = (base(Prop::ScaleX), base(Prop::ScaleY));
            let by = (0.1 * k).min(0.6);
            vec![
                (Prop::ScaleX, kicks(hits, w, Ease::OVERSHOOT, |_| w * (1.0 + by))),
                (Prop::ScaleY, kicks(hits, h, Ease::OVERSHOOT, |_| h * (1.0 - by))),
            ]
        }
        _ => vec![],
    }
}

fn move_of(name: &str) -> Result<&'static Move, EngineError> {
    MOVES
        .iter()
        .find(|m| m.id == name)
        .ok_or_else(|| wrong("move", &format!("one of {}", MOVES.map(|m| m.id).join(", "))))
}

fn strength(p: &P) -> Result<f32, EngineError> {
    match p.opt_f64("strength")? {
        Some(x) if !(0.0..=1.0).contains(&x) => Err(wrong("strength", "0 to 1")),
        x => Ok(x.unwrap_or(0.5) as f32 * 2.0),
    }
}

/// Whether `clip`, on track `ti`, can make `mv`; why not otherwise.
fn takes(proj: &Project, ti: usize, clip: &Clip, mv: &Move) -> Result<(), EngineError> {
    if proj.tracks[ti].kind == TrackKind::Audio {
        return Err(EngineError::Params(format!("{:?} is a sound clip: nothing of it is seen to move", clip.id)));
    }
    if mv.id == "flash" && !clip.is_picture() {
        return Err(EngineError::Params(format!(
            "{:?} is not a video, picture or compound clip: `flash` brightens a picture (use `strobe` or `punch`)",
            clip.id
        )));
    }
    if clip.path.as_ref().is_some_and(|p| p.points.len() >= 2)
        && mv.props.iter().any(|p| matches!(p, Prop::X | Prop::Y))
    {
        return Err(EngineError::Refused(format!(
            "{:?} travels along a path, which decides where it is: `{}` would not be seen (remove the path with clip.setPath, or use another move)",
            clip.id, mv.id
        )));
    }
    Ok(())
}

/// Writes `mv` on a clip for the beats it meets, in place of the keys the move's properties had.
/// How many beats it met.
fn write(clip: &mut Clip, mv: &Move, beats: &[Tick], k: f32, fps: FrameRate) -> usize {
    let hits = hits(beats, clip.duration, fps);
    if hits.is_empty() {
        return 0;
    }
    for (prop, keys) in keys_of(mv, clip, &hits, k, fps) {
        let mut list: Vec<Key> = vec![];
        for key in keys {
            reelforge_model::anim::insert(&mut list, key);
        }
        clip.keys.insert(prop, list);
    }
    hits.len()
}

/// `beat.apply`.
pub(crate) fn beat_apply(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let mv = move_of(p.str("move")?)?;
    let clips = p.strings("clips")?;
    if clips.is_empty() || clips.iter().enumerate().any(|(i, id)| clips[..i].contains(id)) {
        return Err(wrong("clips", "one or more different clips"));
    }
    let ask = Ask::read(&p)?;
    let k = strength(&p)?;
    s.edit("Beat Move", |proj| {
        let fps = proj.canvas.fps;
        let mut done = vec![];
        let mut total = 0;
        for id in &clips {
            let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            takes(proj, ti, clip, mv)?;
            let (start, end) = (clip.start, clip.end());
            let beats: Vec<Tick> = ask.beats(proj, start, end)?.into_iter().map(|t| t - start).collect();
            let clip = proj.clip_mut(id).expect("found");
            let met = write(clip, mv, &beats, k, fps);
            total += met;
            done.push(json!({ "clip": id, "beats": met }));
        }
        if total == 0 {
            return Err(EngineError::Refused("no beat falls within these clips".into()));
        }
        Ok(json!({
            "move": mv.id,
            "props": mv.props.iter().map(|p| p.name()).collect::<Vec<_>>(),
            "beats": total,
            "clips": done,
        }))
    })
}

/// The longest `clip` can be made from where it starts, when there is an end to what it shows.
fn longest(proj: &Project, clip: &Clip) -> Option<Tick> {
    match &clip.content {
        ClipContent::Media { media, source_in, speed, reversed, curve, .. } => {
            let length = proj.media.get(media).filter(|m| !m.kind.loops()).and_then(|m| m.duration)?;
            if *speed == 0.0 {
                // a held frame lasts as long as it is asked to
                return None;
            }
            if *reversed || !curve.is_empty() {
                // where these end up when made longer is not worked out here: only shorter
                return Some(clip.duration);
            }
            Some(Tick(((length - *source_in).0 as f64 / *speed).floor() as i64).max(Tick::ZERO))
        }
        ClipContent::Compound { .. } => Some(clip.duration),
        _ => None,
    }
}

/// `beat.cut`.
pub(crate) fn beat_cut(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let clips = p.strings("clips")?;
    if clips.is_empty() || clips.iter().enumerate().any(|(i, id)| clips[..i].contains(id)) {
        return Err(wrong("clips", "one or more different clips"));
    }
    let ask = Ask::read(&p)?;
    let from = p.opt_time("from")?;
    if from.is_some_and(|t| t < Tick::ZERO) {
        return Err(wrong("from", "not negative"));
    }
    let mv = p.opt_str("move")?.map(move_of).transpose()?;
    let k = strength(&p)?;
    s.edit("Cut On Beat", |proj| {
        let fps = proj.canvas.fps;
        let frame = fps.frame_duration();
        let mut track = None;
        let mut first = None;
        for id in &clips {
            let (ti, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            if *track.get_or_insert(ti) != ti {
                return Err(EngineError::Refused(
                    "the clips must be on one track, to follow one another: move them there with clip.move".into(),
                ));
            }
            if let Some(mv) = mv {
                takes(proj, ti, clip, mv)?;
            }
            first = Some(first.map_or(clip.start, |f: Tick| f.min(clip.start)));
        }
        let ti = track.expect("there are clips");
        let start = fps.snap(from.unwrap_or_else(|| first.expect("there are clips")));
        // a tempo goes on for ever: as many of its beats as these clips can take
        let until = match ask.source {
            Source::Tempo(bpm, first) => {
                let beats = ask.every * (clips.len() + 1) + ask.offset + 2;
                start.max(first) + Tick((Tick::SECOND.0 as f64 * 60.0 / bpm * beats as f64) as i64)
            }
            _ => Tick(i64::MAX / 4),
        };
        // counted from the start, on a beat or not: the first clip ends on the `every`-th after it
        let each = Ask { source: ask.source.clone(), every: 1, offset: 0 };
        let beats: Vec<Tick> = each
            .beats(proj, start + frame * 2, until)?
            .into_iter()
            .skip(ask.offset + ask.every - 1)
            .step_by(ask.every)
            .collect();
        let mut at = start;
        let mut next = 0;
        let mut cuts = vec![];
        let mut spans: Vec<(Tick, Tick)> = vec![];
        let mut on_beat = 0;
        for id in &clips {
            let (_, clip) = proj.clip(id).expect("found");
            let most = longest(proj, clip);
            // the next beat that counts, when the clip reaches it; else the last beat it reaches
            while beats.get(next).is_some_and(|b| *b < at + frame * 2) {
                next += 1;
            }
            let reach = |b: &Tick| most.is_none_or(|m| *b - at <= m);
            let beat = match beats.get(next) {
                Some(b) if reach(b) => {
                    next += 1;
                    Some(*b)
                }
                Some(b) => {
                    // every beat there is, not only the ones that count, is better than none
                    let nearer = Ask { source: ask.source.clone(), every: 1, offset: 0 };
                    let all = nearer.beats(proj, at + frame * 2, *b)?;
                    let found = all.iter().copied().rfind(reach);
                    if found.is_none() {
                        let m = most.unwrap_or(clip.duration);
                        return Err(EngineError::Refused(format!(
                            "{id:?} has {:.2} s to show from where it starts, and the next beat is {:.2} s on: use a longer clip, slow it with clip.setSpeed, or leave it out",
                            m.seconds(),
                            (all.first().copied().unwrap_or(*b) - at).seconds()
                        )));
                    }
                    found
                }
                None => None,
            };
            // past the last beat, clips follow at their own length
            let end = beat.unwrap_or(at + fps.snap(clip.duration).max(frame));
            on_beat += beat.is_some() as usize;
            let clip = proj.clip_mut(id).expect("found");
            clip.start = at;
            trim(clip, at, end)?;
            if let Some(mv) = mv {
                write(clip, mv, &[Tick::ZERO], k, fps);
            }
            cuts.push(json!({ "clip": id, "start": at, "duration": end - at }));
            spans.push((at, end));
            at = end;
        }
        // nothing else on the track may be where these now are
        if let Some(other) = proj.tracks[ti]
            .clips
            .iter()
            .find(|c| !clips.contains(&c.id) && spans.iter().any(|(a, b)| c.start < *b && c.end() > *a))
        {
            return Err(EngineError::Refused(format!(
                "{:?} is in the way on that track ({:.2} s to {:.2} s): move it, or give it among `clips`",
                other.id,
                other.start.seconds(),
                other.end().seconds()
            )));
        }
        if on_beat == 0 {
            return Err(EngineError::Refused("no beat comes after where these clips start".into()));
        }
        // a magnetic track packs its clips from the start, which would take the cuts off the beat
        proj.sort_clips();
        proj.pack_magnetic();
        if clips.iter().zip(&spans).any(|(id, (a, _))| proj.clip(id).is_some_and(|(_, c)| c.start != *a)) {
            return Err(EngineError::Refused(
                "the track is magnetic and would pack these clips away from the beat: start them where the track's clips before them end, or turn that off with track.set".into(),
            ));
        }
        Ok(json!({ "cuts": cuts, "onBeat": on_beat, "end": at, "move": mv.map(|m| m.id) }))
    })
}
