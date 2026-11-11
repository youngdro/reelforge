//! Clips taken together: groups, which are selected and moved as one, and compound clips, which
//! join several clips into one that plays a timeline of their own.

use std::collections::{BTreeMap, BTreeSet};

use reelforge_model::{Clip, ClipContent, Project, Timeline, Track, TrackKind};
use reelforge_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{fps, not_found, trim};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// A group of one is no group.
pub(crate) fn tidy_groups(proj: &mut Project) {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for c in proj.tracks.iter().flat_map(|t| &t.clips) {
        if let Some(g) = &c.group {
            *count.entry(g.clone()).or_default() += 1;
        }
    }
    // a script's group stays a group even of one clip: it is what the script is kept by
    let scripts = &proj.scripts;
    for c in proj.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        if c.group.as_ref().is_some_and(|g| count[g] < 2 && !scripts.contains_key(g)) {
            c.group = None;
        }
    }
}

pub(crate) fn clip_group(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    if ids.iter().collect::<BTreeSet<_>>().len() < 2 {
        return Err(EngineError::Params("a group takes two or more clips".into()));
    }
    s.edit("Group", |proj| {
        for id in &ids {
            proj.clip(id).ok_or_else(|| not_found("clip", id))?;
        }
        let group = proj.mint("g");
        for id in &ids {
            proj.clip_mut(id).expect("found above").group = Some(group.clone());
        }
        // what is left of the groups they were in
        tidy_groups(proj);
        Ok(json!({ "group": group, "clips": ids }))
    })
}

pub(crate) fn clip_ungroup(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    s.edit("Ungroup", |proj| {
        let mut groups = BTreeSet::new();
        for id in &ids {
            let (_, clip) = proj.clip(id).ok_or_else(|| not_found("clip", id))?;
            groups.extend(clip.group.clone());
        }
        if groups.is_empty() {
            return Err(EngineError::Refused("none of these clips is in a group".into()));
        }
        let mut freed = Vec::new();
        for c in proj.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if c.group.as_ref().is_some_and(|g| groups.contains(g)) {
                c.group = None;
                freed.push(c.id.clone());
            }
        }
        Ok(json!({ "clips": freed }))
    })
}

/// A clip with nothing set on it.
fn plain(id: String, start: Tick, duration: Tick, content: ClipContent) -> Clip {
    Clip {
        id,
        start,
        duration,
        content,
        transform: Default::default(),
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
        parent: None,
        path: None,
    }
}

/// Adds an empty track of `kind` at `at` among the tracks; returns its index.
fn new_track(proj: &mut Project, kind: TrackKind, at: usize) -> usize {
    let id = proj.mint("t");
    let n = proj.tracks.iter().filter(|t| t.kind == kind).count() + 1;
    let word = match kind {
        TrackKind::Video => "Video",
        TrackKind::Text => "Text",
        TrackKind::Audio => "Audio",
    };
    let track = Track {
        id,
        kind,
        name: format!("{word} {n}"),
        hidden: false,
        muted: false,
        magnetic: false,
        locked: false,
        clips: vec![],
    };
    let at = at.min(proj.tracks.len());
    proj.tracks.insert(at, track);
    at
}

pub(crate) fn clip_compound(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let ids = p.strings("clips")?;
    let name = p.opt_str("name")?.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    s.edit("Compound Clip", |proj| {
        for id in &ids {
            proj.clip(id).ok_or_else(|| not_found("clip", id))?;
        }
        let chosen: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        let picked = |c: &Clip| chosen.contains(c.id.as_str());
        if !proj.tracks.iter().any(|t| t.kind != TrackKind::Audio && t.clips.iter().any(picked)) {
            return Err(EngineError::Refused(
                "a compound clip needs something to show: include a video, picture or text clip".into(),
            ));
        }
        let all = || proj.tracks.iter().flat_map(|t| &t.clips).filter(|c| picked(c));
        let start = all().map(|c| c.start).min().expect("not empty");
        let end = all().map(Clip::end).max().expect("not empty");

        // their tracks, bottom first, become the tracks of a timeline of their own
        proj.name_first_timeline("Timeline 1");
        let mut inner = Vec::new();
        let mut from_video = Vec::new();
        for ti in 0..proj.tracks.len() {
            let (moved, kept): (Vec<Clip>, Vec<Clip>) =
                std::mem::take(&mut proj.tracks[ti].clips).into_iter().partition(|c| chosen.contains(c.id.as_str()));
            proj.tracks[ti].clips = kept;
            if moved.is_empty() {
                continue;
            }
            if proj.tracks[ti].kind == TrackKind::Video {
                from_video.push(ti);
            }
            let id = proj.mint("t");
            let t = &proj.tracks[ti];
            inner.push(Track {
                id,
                kind: t.kind,
                name: t.name.clone(),
                hidden: t.hidden,
                muted: t.muted,
                magnetic: false,
                locked: false,
                clips: moved
                    .into_iter()
                    .map(|mut c| {
                        c.start -= start;
                        c
                    })
                    .collect(),
            });
        }
        let timeline = proj.mint("s");
        let n = proj.timelines.iter().filter(|t| t.compound).count() + 1;
        proj.timelines.push(Timeline {
            id: timeline.clone(),
            name: name.unwrap_or_else(|| format!("Compound {n}")),
            tracks: inner,
            markers: vec![],
            cover: None,
            compound: true,
            made: None,
        });

        // the clip goes where the lowest of them was, if there is room; else on a track above them
        let range = TimeRange::new(start, end - start);
        let fits = |t: &Track| !t.locked && (t.magnetic || !t.clips.iter().any(|c| c.range().overlaps(range)));
        let index = match from_video.iter().copied().find(|&i| fits(&proj.tracks[i])) {
            Some(i) => i,
            None => {
                let above = from_video.last().map(|i| i + 1);
                let at = above
                    .or_else(|| proj.tracks.iter().rposition(|t| t.kind == TrackKind::Video).map(|i| i + 1))
                    .unwrap_or(0);
                new_track(proj, TrackKind::Video, at)
            }
        };
        let id = proj.mint("c");
        let content = ClipContent::Compound { timeline: timeline.clone(), source_in: Tick::ZERO };
        proj.tracks[index].clips.push(plain(id.clone(), start, end - start, content));
        tidy_groups(proj);
        Ok(json!({
            "clip": id,
            "timeline": timeline,
            "track": proj.tracks[index].id,
            "start": start,
            "duration": end - start,
        }))
    })
}

pub(crate) fn clip_uncompound(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let id = p.str("clip")?.to_string();
    s.edit("Take Apart Compound Clip", |proj| {
        let (ti, clip) = proj.clip(&id).ok_or_else(|| not_found("clip", &id))?;
        let Some((timeline, source_in)) = clip.compound() else {
            return Err(EngineError::Params(format!("clip {id:?} is not a compound clip")));
        };
        let (timeline, start, end) = (timeline.to_string(), clip.start, clip.end());
        // the part of its timeline the clip plays, and where that lies on this one
        let window = (source_in, source_in + (end - start));
        let shift = start - source_in;
        let inner: Vec<Track> = proj.tracks_of(&timeline).ok_or_else(|| not_found("timeline", &timeline))?.to_vec();
        proj.tracks[ti].clips.retain(|c| c.id != id);

        let slot = |k: TrackKind| match k {
            TrackKind::Video => 0,
            TrackKind::Text => 1,
            TrackKind::Audio => 2,
        };
        // the track the last inner track of each kind came out on: the next goes above it
        let mut last: [Option<usize>; 3] = [None; 3];
        let mut own = ti;
        let mut groups: BTreeMap<String, String> = BTreeMap::new();
        let mut placed = Vec::new();
        for track in &inner {
            let mut clips = Vec::new();
            for c in &track.clips {
                let (a, b) = (c.start.max(window.0), c.end().min(window.1));
                if a >= b {
                    continue;
                }
                let mut c = c.clone();
                trim(&mut c, a, b)?;
                c.start = a + shift;
                c.id = proj.mint("c");
                if let Some(g) = &c.group {
                    c.group = Some(groups.entry(g.clone()).or_insert_with(|| proj.mint("g")).clone());
                }
                clips.push(c);
            }
            if clips.is_empty() {
                continue;
            }
            let k = slot(track.kind);
            // a magnetic track packs what lands on it: only the clip's own track takes them so
            let fits = |i: usize, t: &Track| {
                t.kind == track.kind
                    && !t.locked
                    && (!t.magnetic || i == own)
                    && clips.iter().all(|c| !t.clips.iter().any(|o| o.range().overlaps(c.range())))
            };
            let from = match (last[k], track.kind) {
                (Some(i), _) => i + 1,
                (None, TrackKind::Video) => own,
                (None, _) => 0,
            };
            let index = match (from..proj.tracks.len()).find(|&i| fits(i, &proj.tracks[i])) {
                Some(i) => i,
                None => {
                    let end_of_kind = proj.tracks.iter().rposition(|t| t.kind == track.kind).map(|i| i + 1);
                    let at = last[k].map(|i| i + 1).or(end_of_kind).unwrap_or(match track.kind {
                        TrackKind::Video => 0,
                        _ => proj.tracks.len(),
                    });
                    let at = new_track(proj, track.kind, at);
                    // the tracks from there on moved up one
                    for i in last.iter_mut().flatten().filter(|i| **i >= at) {
                        *i += 1;
                    }
                    if own >= at {
                        own += 1;
                    }
                    at
                }
            };
            for c in clips {
                placed.push(json!({ "clip": c.id, "track": proj.tracks[index].id, "start": c.start }));
                proj.tracks[index].clips.push(c);
            }
            last[k] = Some(index);
        }
        // a timeline made for compound clips goes with the last of them
        if proj.compounds_of(&timeline).is_empty() {
            proj.timelines.retain(|t| !(t.id == timeline && t.compound));
        }
        tidy_groups(proj);
        Ok(json!({ "clips": placed }))
    })
}
