//! The project document.
//!
//! A [`Project`] is what the user edits and what a `.reel` file stores: the canvas, the media the
//! user imported, and tracks of clips. It is plain serde data with no behaviour beyond reading
//! itself and checking its invariants ([`Project::check`]); every change goes through the engine's
//! commands, and a render goes through `reelforge-compile`, which turns a project into a
//! RenderSpec. Nothing here knows about pixels.
//!
//! The model is shaped for short videos rather than for a broadcast NLE: one timeline, a vertical
//! canvas by default, tracks that are either picture, text or sound, and clips that carry their own
//! transform, opacity and volume.

use std::collections::BTreeMap;

use reelforge_time::{FrameRate, Tick, TimeRange};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The document version this build writes. Older documents keep loading.
pub const DOCUMENT_VERSION: u32 = 1;

/// A whole project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// Document version.
    pub version: u32,
    /// Output canvas.
    pub canvas: Canvas,
    /// Imported media by id.
    pub media: BTreeMap<String, Media>,
    /// Tracks, bottom first.
    pub tracks: Vec<Track>,
    /// Next id to mint.
    pub next_id: u64,
}

/// The output canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Canvas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frame rate.
    pub fps: FrameRate,
    /// Colour behind everything (straight sRGB).
    pub background: [f32; 4],
}

impl Default for Canvas {
    fn default() -> Canvas {
        Canvas { width: 1080, height: 1920, fps: FrameRate::FPS_30, background: [0.0, 0.0, 0.0, 1.0] }
    }
}

/// An imported file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Media {
    /// Id (`m…`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Where the file is: a path or a URI.
    pub uri: String,
    /// SHA-256 of the file, lowercase hex, when known. Renders need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// What it is.
    pub kind: MediaKind,
    /// Length, for video and sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Tick>,
    /// Picture size, for video and images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
}

/// Media kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Video, possibly with sound.
    Video,
    /// A still image.
    Image,
    /// Sound only.
    Audio,
}

/// A track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    /// Id (`t…`).
    pub id: String,
    /// What the track holds.
    pub kind: TrackKind,
    /// Display name.
    pub name: String,
    /// Picture hidden (picture and text tracks).
    #[serde(default)]
    pub hidden: bool,
    /// Sound muted.
    #[serde(default)]
    pub muted: bool,
    /// Clips in time order, never overlapping.
    pub clips: Vec<Clip>,
}

/// Track kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Video, images and colour.
    Video,
    /// Titles and captions.
    Text,
    /// Music, voice, effects.
    Audio,
}

/// A clip on a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Clip {
    /// Id (`c…`).
    pub id: String,
    /// Timeline start.
    pub start: Tick,
    /// Timeline length; positive.
    pub duration: Tick,
    /// What it shows or plays.
    pub content: ClipContent,
    /// Placement (picture clips).
    #[serde(default)]
    pub transform: ClipTransform,
    /// Opacity 0 to 1 (picture clips).
    #[serde(default = "one")]
    pub opacity: f32,
    /// Gain in dB (clips with sound).
    #[serde(default)]
    pub volume_db: f32,
    /// Effects, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ClipEffect>,
}

fn one() -> f32 {
    1.0
}

impl Clip {
    /// The clip's span on the timeline.
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// The first tick after the clip.
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
}

/// What a clip shows or plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    /// Part of an imported file.
    Media {
        /// The media id.
        media: String,
        /// Source instant at the clip's start.
        source_in: Tick,
        /// Playback speed; positive.
        speed: f64,
    },
    /// A colour filling the frame.
    Solid {
        /// Straight sRGB colour.
        color: [f32; 4],
    },
    /// A title.
    Text {
        /// The words.
        text: String,
        /// Em size in canvas pixels.
        size: f32,
        /// Straight sRGB colour.
        color: [f32; 4],
    },
}

/// A clip's placement: offset from the canvas centre as fractions of the canvas, scale and
/// clockwise rotation in degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClipTransform {
    /// Horizontal offset, fraction of the canvas width.
    pub x: f32,
    /// Vertical offset, fraction of the canvas height.
    pub y: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Degrees clockwise.
    pub rotation: f32,
}

impl Default for ClipTransform {
    fn default() -> ClipTransform {
        ClipTransform { x: 0.0, y: 0.0, scale: 1.0, rotation: 0.0 }
    }
}

/// An effect on a clip: a RenderSpec effect id and constant parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClipEffect {
    /// `<name>@<version>`.
    pub id: String,
    /// Parameters.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

/// A broken invariant.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ModelError {
    /// Two things share an id.
    #[error("duplicate id {0:?}")]
    DuplicateId(String),
    /// Two clips on one track overlap.
    #[error("clips {0:?} and {1:?} overlap on track {2:?}")]
    Overlap(String, String, String),
    /// A clip is in the wrong order on its track.
    #[error("clips on track {0:?} are not in time order")]
    Order(String),
    /// A clip has no length, or a negative start.
    #[error("clip {0:?} must start at or after 0 and have a positive duration")]
    BadSpan(String),
    /// A clip names media that is not in the project.
    #[error("clip {0:?} refers to unknown media {1:?}")]
    UnknownMedia(String, String),
    /// A clip's content does not belong on its track.
    #[error("clip {0:?} cannot sit on {1:?} track {2:?}")]
    WrongTrack(String, TrackKind, String),
    /// A speed outside (0, 100].
    #[error("clip {0:?} has an invalid speed")]
    BadSpeed(String),
}

impl Default for Project {
    fn default() -> Project {
        Project::new(Canvas::default())
    }
}

impl Project {
    /// An empty project with one video track and one text track.
    pub fn new(canvas: Canvas) -> Project {
        let mut p =
            Project { version: DOCUMENT_VERSION, canvas, media: BTreeMap::new(), tracks: Vec::new(), next_id: 1 };
        for (kind, name) in [(TrackKind::Video, "Video 1"), (TrackKind::Text, "Text 1"), (TrackKind::Audio, "Audio 1")]
        {
            let id = p.mint("t");
            p.tracks.push(Track { id, kind, name: name.into(), hidden: false, muted: false, clips: Vec::new() });
        }
        p
    }

    /// A fresh id with the given prefix (`m`, `t`, `c`).
    pub fn mint(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// The track with this id.
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// The track with this id, mutable.
    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// The clip with this id and the index of its track.
    pub fn clip(&self, id: &str) -> Option<(usize, &Clip)> {
        self.tracks.iter().enumerate().find_map(|(i, t)| t.clips.iter().find(|c| c.id == id).map(|c| (i, c)))
    }

    /// The clip with this id, mutable.
    pub fn clip_mut(&mut self, id: &str) -> Option<&mut Clip> {
        self.tracks.iter_mut().find_map(|t| t.clips.iter_mut().find(|c| c.id == id))
    }

    /// The end of the last clip: the length of the edit.
    pub fn duration(&self) -> Tick {
        self.tracks.iter().flat_map(|t| t.clips.iter().map(Clip::end)).max().unwrap_or(Tick::ZERO)
    }

    /// Sorts every track's clips by start time.
    pub fn sort_clips(&mut self) {
        for t in &mut self.tracks {
            t.clips.sort_by_key(|c| c.start);
        }
    }

    /// Checks every invariant. The engine runs this after each edit and refuses an edit that
    /// breaks one.
    pub fn check(&self) -> Result<(), ModelError> {
        let mut ids = std::collections::BTreeSet::new();
        for id in self.media.keys() {
            if !ids.insert(id.clone()) {
                return Err(ModelError::DuplicateId(id.clone()));
            }
        }
        for track in &self.tracks {
            if !ids.insert(track.id.clone()) {
                return Err(ModelError::DuplicateId(track.id.clone()));
            }
            for clip in &track.clips {
                if !ids.insert(clip.id.clone()) {
                    return Err(ModelError::DuplicateId(clip.id.clone()));
                }
                if clip.start < Tick::ZERO || clip.duration <= Tick::ZERO {
                    return Err(ModelError::BadSpan(clip.id.clone()));
                }
                let fits = match &clip.content {
                    ClipContent::Media { media, speed, .. } => {
                        let Some(m) = self.media.get(media) else {
                            return Err(ModelError::UnknownMedia(clip.id.clone(), media.clone()));
                        };
                        if !(*speed > 0.0 && *speed <= 100.0) {
                            return Err(ModelError::BadSpeed(clip.id.clone()));
                        }
                        match track.kind {
                            TrackKind::Video => m.kind != MediaKind::Audio,
                            TrackKind::Audio => m.kind != MediaKind::Image,
                            TrackKind::Text => false,
                        }
                    }
                    ClipContent::Solid { .. } => track.kind == TrackKind::Video,
                    ClipContent::Text { .. } => track.kind == TrackKind::Text,
                };
                if !fits {
                    return Err(ModelError::WrongTrack(clip.id.clone(), track.kind, track.id.clone()));
                }
            }
            for pair in track.clips.windows(2) {
                if pair[1].start < pair[0].start {
                    return Err(ModelError::Order(track.id.clone()));
                }
                if pair[0].range().overlaps(pair[1].range()) {
                    return Err(ModelError::Overlap(pair[0].id.clone(), pair[1].id.clone(), track.id.clone()));
                }
            }
        }
        Ok(())
    }

    /// Reads a project from JSON and checks it.
    pub fn from_json(json: &str) -> Result<Project, String> {
        let p: Project = serde_json::from_str(json).map_err(|e| e.to_string())?;
        p.check().map_err(|e| e.to_string())?;
        Ok(p)
    }

    /// The project as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a Project always serialises")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(id: &str, start: i64, dur: i64) -> Clip {
        Clip {
            id: id.into(),
            start: Tick(start),
            duration: Tick(dur),
            content: ClipContent::Solid { color: [1.0, 0.0, 0.0, 1.0] },
            transform: ClipTransform::default(),
            opacity: 1.0,
            volume_db: 0.0,
            effects: vec![],
        }
    }

    #[test]
    fn new_project_is_valid_and_round_trips() {
        let p = Project::default();
        p.check().unwrap();
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn overlaps_and_wrong_tracks_are_refused() {
        let mut p = Project::default();
        p.tracks[0].clips = vec![solid("c10", 0, 10), solid("c11", 5, 10)];
        assert!(matches!(p.check(), Err(ModelError::Overlap(..))));
        p.tracks[0].clips = vec![solid("c10", 0, 10), solid("c11", 10, 10)];
        p.check().unwrap();
        p.tracks[1].clips = vec![solid("c12", 0, 10)];
        assert!(matches!(p.check(), Err(ModelError::WrongTrack(..))));
    }
}
