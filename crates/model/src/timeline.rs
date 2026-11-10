//! More than one timeline in a project.
//!
//! A project edits one timeline at a time: its tracks are [`Project::tracks`], so every command,
//! the compiler and the clients keep working on "the" timeline. The others wait in
//! [`Project::timelines`] with their tracks; switching swaps the two. Media, the canvas and the
//! export settings are shared by all of them.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Clip, Project, Track, TrackKind};

/// A timeline of the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Timeline {
    /// Id (`s…`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Its tracks while another timeline is being edited; empty for the current one, whose tracks
    /// are the project's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
    /// Its marks while another timeline is being edited.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<crate::Marker>,
    /// Its cover while another timeline is being edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<Tick>,
    /// Made by joining clips into a compound clip, not as a cut of its own: taken away again
    /// when its last compound clip is taken apart.
    #[serde(default, skip_serializing_if = "crate::is_no")]
    pub compound: bool,
}

/// A timeline as listed: id, name, length and whether it is the one being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineInfo {
    /// Id; empty for a project that has only ever had one timeline.
    pub id: String,
    /// Display name.
    pub name: String,
    /// End of its last clip.
    pub duration: Tick,
    /// Whether it is the one in [`Project::tracks`].
    pub current: bool,
    /// Whether it was made for a compound clip ([`Timeline::compound`]).
    pub compound: bool,
}

/// The end of the last clip on these tracks.
pub fn tracks_duration(tracks: &[Track]) -> Tick {
    tracks.iter().flat_map(|t| t.clips.iter().map(Clip::end)).max().unwrap_or(Tick::ZERO)
}

/// The tracks of a new, empty timeline: one video, one text and one audio track.
pub fn empty_tracks(p: &mut Project) -> Vec<Track> {
    [(TrackKind::Video, "Video 1"), (TrackKind::Text, "Text 1"), (TrackKind::Audio, "Audio 1")]
        .into_iter()
        .map(|(kind, name)| Track {
            id: p.mint("t"),
            kind,
            name: name.into(),
            hidden: false,
            muted: false,
            magnetic: false,
            locked: false,
            clips: Vec::new(),
        })
        .collect()
}

impl Project {
    /// Every timeline in order. A project that never had a second one lists one unnamed timeline.
    pub fn timeline_list(&self) -> Vec<TimelineInfo> {
        if self.timelines.is_empty() {
            return vec![TimelineInfo {
                id: String::new(),
                name: String::new(),
                duration: self.duration(),
                current: true,
                compound: false,
            }];
        }
        self.timelines
            .iter()
            .map(|t| {
                let current = t.id == self.timeline;
                TimelineInfo {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    duration: if current { self.duration() } else { tracks_duration(&t.tracks) },
                    current,
                    compound: t.compound,
                }
            })
            .collect()
    }

    /// Makes the single implicit timeline an explicit one (named `name`), so that others can be
    /// added beside it. Does nothing once there is a list.
    pub fn name_first_timeline(&mut self, name: &str) {
        if self.timelines.is_empty() {
            let id = self.mint("s");
            self.timelines.push(Timeline {
                id: id.clone(),
                name: name.into(),
                tracks: Vec::new(),
                markers: Vec::new(),
                cover: None,
                compound: false,
            });
            self.timeline = id;
        }
    }

    /// Edits the timeline `id` from now on, parking the current one. False if there is no such
    /// timeline.
    pub fn switch_timeline(&mut self, id: &str) -> bool {
        if id == self.timeline {
            return self.timelines.iter().any(|t| t.id == id);
        }
        let Some(to) = self.timelines.iter().position(|t| t.id == id) else { return false };
        let incoming = std::mem::take(&mut self.timelines[to].tracks);
        let outgoing = std::mem::replace(&mut self.tracks, incoming);
        let marks_in = std::mem::take(&mut self.timelines[to].markers);
        let marks_out = std::mem::replace(&mut self.markers, marks_in);
        if let Some(from) = self.timelines.iter_mut().find(|t| t.id == self.timeline) {
            from.tracks = outgoing;
            from.markers = marks_out;
            from.cover = self.cover;
        }
        self.cover = self.timelines[to].cover.take();
        self.timeline = id.into();
        true
    }

    /// A copy of the current timeline's tracks under fresh ids.
    pub fn copy_tracks(&mut self) -> Vec<Track> {
        let mut tracks = self.tracks.clone();
        let mut groups = std::collections::BTreeMap::new();
        for t in &mut tracks {
            t.id = self.mint("t");
            for c in &mut t.clips {
                c.id = self.mint("c");
                if let Some(g) = &c.group {
                    c.group = Some(groups.entry(g.clone()).or_insert_with(|| self.mint("g")).clone());
                }
            }
        }
        tracks
    }

    /// The tracks of timeline `id`, wherever they are: the project's for the one being edited.
    /// (A project that never had a second timeline has one with the empty id.)
    pub fn tracks_of(&self, id: &str) -> Option<&[Track]> {
        if id == self.timeline {
            return Some(&self.tracks);
        }
        self.timelines.iter().find(|t| t.id == id).map(|t| t.tracks.as_slice())
    }

    /// The timelines the compound clips of timeline `id` play.
    fn played_by(&self, id: &str) -> impl Iterator<Item = &str> {
        self.tracks_of(id)
            .into_iter()
            .flatten()
            .flat_map(|t| &t.clips)
            .filter_map(|c| c.compound().map(|(timeline, _)| timeline))
    }

    /// Whether playing timeline `from` plays timeline `to`: one of its compound clips plays it,
    /// directly or through others.
    pub fn timeline_reaches(&self, from: &str, to: &str) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        let mut todo: Vec<&str> = self.played_by(from).collect();
        while let Some(id) = todo.pop() {
            if id == to {
                return true;
            }
            if seen.insert(id) {
                todo.extend(self.played_by(id));
            }
        }
        false
    }

    /// A timeline with a compound clip that plays timeline `id`, and that clip: the one being
    /// edited if it has one, else the first that does.
    pub fn player_of(&self, id: &str) -> Option<(&str, &Clip)> {
        let parked =
            self.timelines.iter().filter(|t| t.id != self.timeline).map(|t| (t.id.as_str(), t.tracks.as_slice()));
        std::iter::once((self.timeline.as_str(), self.tracks.as_slice())).chain(parked).find_map(
            |(timeline, tracks)| {
                let mut clips = tracks.iter().flat_map(|t| &t.clips);
                clips.find(|c| c.compound().is_some_and(|(played, _)| played == id)).map(|c| (timeline, c))
            },
        )
    }

    /// Whether timeline `id` has anything to hear: a clip of a video or a sound file, in it or in
    /// a timeline its compound clips play.
    pub fn timeline_sounds(&self, id: &str) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        let mut todo = vec![id];
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            for c in self.tracks_of(id).into_iter().flatten().flat_map(|t| &t.clips) {
                match &c.content {
                    crate::ClipContent::Media { media, silent: false, .. }
                        if self.media.get(media).is_some_and(|m| m.kind.sounds()) =>
                    {
                        return true;
                    }
                    crate::ClipContent::Compound { timeline, .. } => todo.push(timeline),
                    _ => {}
                }
            }
        }
        false
    }

    /// The compound clips anywhere in the project that play timeline `id`.
    pub fn compounds_of(&self, id: &str) -> Vec<&Clip> {
        let parked = self.timelines.iter().flat_map(|t| &t.tracks);
        self.tracks
            .iter()
            .chain(parked)
            .flat_map(|t| &t.clips)
            .filter(|c| c.compound().is_some_and(|(timeline, _)| timeline == id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::Project;

    #[test]
    fn switching_swaps_the_tracks_and_keeps_ids_unique() {
        let mut p = Project::default();
        assert_eq!(p.timeline_list().len(), 1);
        p.name_first_timeline("Timeline 1");
        let first = p.timeline.clone();
        let copy = p.copy_tracks();
        let id = p.mint("s");
        p.timelines.push(super::Timeline {
            id: id.clone(),
            name: "Timeline 2".into(),
            tracks: copy,
            markers: vec![],
            cover: None,
            compound: false,
        });
        p.check().unwrap();
        let before = p.tracks.clone();
        assert!(p.switch_timeline(&id));
        assert_ne!(p.tracks[0].id, before[0].id);
        assert_eq!(p.timelines[0].tracks, before);
        p.check().unwrap();
        assert!(p.switch_timeline(&first));
        assert_eq!(p.tracks, before);
        assert!(p.timelines[0].tracks.is_empty());
        assert!(!p.switch_timeline("nope"));
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
    }
}
