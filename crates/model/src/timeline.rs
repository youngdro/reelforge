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
        }
        self.timeline = id.into();
        true
    }

    /// A copy of the current timeline's tracks under fresh ids.
    pub fn copy_tracks(&mut self) -> Vec<Track> {
        let mut tracks = self.tracks.clone();
        for t in &mut tracks {
            t.id = self.mint("t");
            for c in &mut t.clips {
                c.id = self.mint("c");
            }
        }
        tracks
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
        p.timelines.push(super::Timeline { id: id.clone(), name: "Timeline 2".into(), tracks: copy, markers: vec![] });
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
