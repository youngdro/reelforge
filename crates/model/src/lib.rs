//! The project document.
//!
//! A [`Project`] is what the user edits and what a `.reel` file stores: the canvas, the media the
//! user imported, and tracks of clips. It is plain serde data with no behaviour beyond reading
//! itself and checking its invariants ([`Project::check`]); every change goes through the engine's
//! commands, and a render goes through `reelforge-compile`, which turns a project into a
//! RenderSpec. Nothing here knows about pixels.
//!
//! The model is shaped for short videos rather than for a broadcast NLE: one timeline edited at a
//! time (others wait in [`Project::timelines`]), a vertical canvas by default, tracks that are either picture, text or sound, and clips that carry their own
//! transform, opacity and volume.

use std::collections::BTreeMap;

use reelforge_time::{FrameRate, Tick, TimeRange};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod anim;
pub mod brand;
pub mod color;
pub mod export;
pub mod fx;
pub mod look;
pub mod shape;
pub mod steady;
pub mod timeline;
pub mod timing;
pub mod transition;
pub mod voice;

pub use anim::{AnimKind, Animation, Animations, Ease, Key, Keys, Loop, LoopKind, Offset, Prop};
pub use brand::{Brand, CopyField, CopyLine, Forms, Made, fields_in, is_field_name, written};
pub use color::{Adjust, ColorGrade, Filter, FilterKind, LutUse};
pub use export::{Codec, ExportSettings, Quality, Resolution};
pub use fx::{Fx, FxChannel, FxGroup, FxKind, FxSample};
pub use look::{BlendMode, ChromaKey, Crop, Look, Mask, MaskKind};
pub use shape::{Counter, Fill, Shape, ShapeKind, ShapeShadow, Stroke};
pub use steady::{Steadied, Steady};
pub use timeline::{Timeline, TimelineInfo};
pub use timing::{CurvePoint, CurvePreset, Inbetween, Timing};
pub use transition::{Transition, TransitionKind};
pub use voice::{Denoise, Voice, VoiceKind, VoicePart};

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
    /// How the project is exported.
    #[serde(default, skip_serializing_if = "ExportSettings::is_default")]
    pub export: ExportSettings,
    /// Every timeline, in order, once there is more than one; empty while the project has only
    /// ever had the one in `tracks`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timelines: Vec<Timeline>,
    /// Id of the timeline being edited (its tracks are `tracks`); empty while `timelines` is.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timeline: String,
    /// Marks on the timeline being edited, in time order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
    /// The instant of the timeline being edited whose frame stands for it: the cover, written
    /// beside an exported video. Past the end, the last frame stands in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<Tick>,
    /// The colours, face and mark the ready-made pieces are made in.
    #[serde(default, skip_serializing_if = "Brand::is_empty")]
    pub brand: Brand,
    /// The project's copy by name (a storyboard's fields: the hook, the product's name, the
    /// price), which ready-made pieces are written from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy: Vec<CopyField>,
    /// The captions written from the copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_lines: Vec<CopyLine>,
}

/// A mark on a timeline: a place to come back to, and to snap to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Marker {
    /// Id (`m…`).
    pub id: String,
    /// Where it is.
    pub at: Tick,
    /// What it is for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
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
    /// The folder of the media panel it is filed under, if any. A folder is nothing but this name:
    /// it is there while some media has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl Media {
    /// The longest a folder's name can be, in characters.
    pub const FOLDER_NAME_MAX: usize = 40;
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
    /// A colour lookup table (`.cube`), used by clips' colour rather than placed.
    Lut,
    /// A font file (TrueType or OpenType), set on titles rather than placed.
    Font,
    /// A Lottie animation (Bodymovin JSON or a dotLottie archive): placed like a video, drawn
    /// as sharp as its `size` says, see-through where it draws nothing, and going round again
    /// when a clip of it is longer than it is.
    Lottie,
}

impl MediaKind {
    /// Whether it has a picture, and so goes on a video track.
    pub fn pictured(self) -> bool {
        matches!(self, MediaKind::Video | MediaKind::Image | MediaKind::Lottie)
    }

    /// Whether its picture changes over time: it can be sped up, slowed, reversed and frozen.
    pub fn moves(self) -> bool {
        matches!(self, MediaKind::Video | MediaKind::Lottie)
    }

    /// Whether it may have sound.
    pub fn sounds(self) -> bool {
        matches!(self, MediaKind::Video | MediaKind::Audio)
    }

    /// Whether it goes round again past its end, so a clip of it can be any length.
    pub fn loops(self) -> bool {
        self == MediaKind::Lottie
    }
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
    /// Magnetic: clips stay end to end from the timeline's start, so a removal or trim closes its
    /// gap and a move reorders.
    #[serde(default, skip_serializing_if = "is_no")]
    pub magnetic: bool,
    /// Locked: its clips cannot be changed until it is unlocked.
    #[serde(default, skip_serializing_if = "is_no")]
    pub locked: bool,
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
    /// Keyframes; a keyed property ignores its constant value above.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: Keys,
    /// In and out animations.
    #[serde(default, skip_serializing_if = "Animations::is_empty")]
    pub anim: Animations,
    /// A transition from the clip before it, when that clip ends where this one starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Sound fading in from the clip's start and out to its end.
    #[serde(default, skip_serializing_if = "Fades::is_empty")]
    pub fade: Fades,
    /// Whether sound at another speed keeps its pitch (rather than rising and falling like tape).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub keep_pitch: bool,
    /// Filter, lookup table and colour adjustments (picture clips).
    #[serde(default, skip_serializing_if = "ColorGrade::is_empty")]
    pub color: ColorGrade,
    /// Crop, mirror, mask and blend (picture clips).
    #[serde(default, skip_serializing_if = "Look::is_empty")]
    pub look: Look,
    /// Picture effects that play while the clip lasts: on the clip itself, or, on an adjustment
    /// clip, on everything on the tracks below.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx: Vec<Fx>,
    /// A changed voice, a moved pitch, noise taken out (clips with sound).
    #[serde(default, skip_serializing_if = "Voice::is_empty")]
    pub voice: Voice,
    /// The group it belongs to (`g…`): clips of one group are selected and moved together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The slot of a template this clip is, from 1 (剪映's 剪同款): a picture made to be replaced
    /// by one of the person's own, keeping its place, its length and everything set on it. A slot
    /// that is still a colour is empty. Clips with the same number take the same file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// The picture held steady (a video's): the camera's path as measured, and how much of its
    /// shake is taken out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steady: Option<Steady>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

pub(crate) fn is_no(b: &bool) -> bool {
    !*b
}

/// A clip's sound fades.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fades {
    /// Fade-in length from the clip's start.
    #[serde(default, rename = "in", skip_serializing_if = "Tick::is_zero")]
    pub fade_in: Tick,
    /// Fade-out length up to the clip's end.
    #[serde(default, rename = "out", skip_serializing_if = "Tick::is_zero")]
    pub fade_out: Tick,
}

impl Fades {
    /// No fades.
    pub fn is_empty(&self) -> bool {
        self.fade_in == Tick::ZERO && self.fade_out == Tick::ZERO
    }

    /// The fades on a clip `len` long, shortened in proportion when together they would not fit.
    pub fn spans(&self, len: Tick) -> (Tick, Tick) {
        let (a, b) = (self.fade_in, self.fade_out);
        if a + b <= len || a + b <= Tick::ZERO {
            return (a, b);
        }
        let a2 = Tick((a.0 as i128 * len.0 as i128 / (a + b).0 as i128) as i64);
        (a2, len - a2)
    }

    /// The amplitude the fades leave at `t` from the start of a clip `len` long, 0 to 1.
    pub fn gain(&self, t: Tick, len: Tick) -> f32 {
        let (a, b) = self.spans(len);
        let mut g = 1.0f32;
        if a > Tick::ZERO && t < a {
            g *= (t.0.max(0) as f64 / a.0 as f64) as f32;
        }
        if b > Tick::ZERO && t > len - b {
            g *= ((len - t).0.max(0) as f64 / b.0 as f64) as f32;
        }
        g
    }
}

fn one() -> f32 {
    1.0
}

impl Clip {
    /// Whether it is an adjustment layer: nothing to see, effects and colour for what is below.
    pub fn is_adjustment(&self) -> bool {
        matches!(self.content, ClipContent::Adjustment)
    }

    /// The timeline a compound clip plays, and from when.
    pub fn compound(&self) -> Option<(&str, Tick)> {
        match &self.content {
            ClipContent::Compound { timeline, source_in } => Some((timeline, *source_in)),
            _ => None,
        }
    }

    /// Whether it shows a picture of its own that can be cropped, mirrored and coloured: media or
    /// a compound clip.
    pub fn is_picture(&self) -> bool {
        matches!(self.content, ClipContent::Media { .. } | ClipContent::Compound { .. })
    }

    /// The clip's span on the timeline.
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// The first tick after the clip.
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    /// How a media clip runs through its source; `None` for other clips.
    pub fn timing(&self) -> Option<Timing> {
        match &self.content {
            ClipContent::Media { source_in, speed, reversed, curve, .. } => {
                Some(Timing::of(*source_in, *speed, *reversed, curve, self.duration))
            }
            _ => None,
        }
    }
}

impl ClipContent {
    /// A media clip from `source_in` at normal speed.
    pub fn media(media: impl Into<String>, source_in: Tick) -> ClipContent {
        ClipContent::Media {
            media: media.into(),
            source_in,
            speed: 1.0,
            reversed: false,
            curve: vec![],
            silent: false,
            frames: Inbetween::Repeat,
        }
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
        /// Playback speed, up to 100; 0 holds `source_in` (a freeze frame, silent).
        speed: f64,
        /// Played backwards: the clip covers the same source span from its end.
        #[serde(default, skip_serializing_if = "is_no")]
        reversed: bool,
        /// A speed curve across the clip ([`timing`]); empty is a steady speed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        curve: Vec<CurvePoint>,
        /// Its sound is not played: it was detached to a clip of its own.
        #[serde(default, skip_serializing_if = "is_no")]
        silent: bool,
        /// What a video shows between two frames of its file.
        #[serde(default, skip_serializing_if = "Inbetween::is_repeat")]
        frames: Inbetween,
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
        /// Weight, outline, box, wrapping.
        #[serde(default)]
        style: TitleStyle,
    },
    /// A drawn shape: a box, ellipse, line, polygon, star, price tag or arrow (picture tracks).
    Shape {
        /// What it draws.
        shape: Shape,
    },
    /// An adjustment layer: it shows nothing itself; its effects ([`Clip::fx`]) and its colour
    /// (filter and adjustments) apply to the clips on the tracks below it while it lasts.
    Adjustment,
    /// Another timeline of the project played as one clip (剪映's 复合片段): its picture is drawn
    /// first and then placed, faded, masked and given effects as one; its sound plays with it.
    Compound {
        /// The timeline's id.
        timeline: String,
        /// The instant of that timeline at the clip's start.
        source_in: Tick,
    },
}

/// How a title looks beyond its words, size and colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TitleStyle {
    /// 100 (thin) to 900 (black); 700 is bold.
    #[serde(default = "bold")]
    pub weight: u16,
    /// Line alignment.
    #[serde(default)]
    pub align: TitleAlign,
    /// Lines wrap at this fraction of the canvas width; 0 wraps only at line breaks.
    #[serde(default = "wrap")]
    pub wrap: f32,
    /// An outline around the glyphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    /// A box behind the title, straight sRGB colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[f32; 4]>,
    /// The box's corner radius, in ems.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f32,
    /// The fill fades from the title's colour at the top to this at the bottom (straight sRGB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<[f32; 4]>,
    /// A second outline outside the first (its width counts from the first's edge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer: Option<Outline>,
    /// A shadow beneath the glyphs and outlines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// The box goes behind each line, hugging it (as subtitles have it), not the whole title.
    #[serde(default, skip_serializing_if = "is_no")]
    pub line_boxes: bool,
    /// The face it is set in: font media; absent, the built-in face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Space added after every character, in ems.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tracking: f32,
    /// Space added between lines, in ems.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub leading: f32,
    /// Slanted.
    #[serde(default, skip_serializing_if = "is_no")]
    pub italic: bool,
    /// Underlined.
    #[serde(default, skip_serializing_if = "is_no")]
    pub underline: bool,
    /// A number it shows, running from one value to another: where its words say `{n}`, or as
    /// all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<Counter>,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// A title's shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Shadow {
    /// Straight sRGB colour.
    pub color: [f32; 4],
    /// Offset `[right, down]` in ems.
    pub offset: [f32; 2],
    /// Softness in ems; 0 is a hard edge.
    #[serde(default)]
    pub blur: f32,
}

fn bold() -> u16 {
    700
}

fn wrap() -> f32 {
    0.9
}

impl Default for TitleStyle {
    /// Bold, centred, wrapping at 90 % of the canvas, no outline or box.
    fn default() -> TitleStyle {
        TitleStyle {
            weight: bold(),
            align: TitleAlign::Center,
            wrap: wrap(),
            outline: None,
            background: None,
            radius: 0.0,
            gradient: None,
            outer: None,
            shadow: None,
            line_boxes: false,
            font: None,
            tracking: 0.0,
            leading: 0.0,
            italic: false,
            underline: false,
            counter: None,
        }
    }
}

impl TitleStyle {
    /// The furthest characters and lines can be moved together or apart, in ems.
    pub const SPACING: (f32, f32) = (-0.5, 4.0);

    /// This title wearing `look` (a template's): its colours, outlines, box and weight, while how
    /// its lines wrap and line up, its face, its spacing, slant and underline stay its own.
    pub fn wearing(&self, look: &TitleStyle) -> TitleStyle {
        TitleStyle {
            align: self.align,
            wrap: self.wrap,
            font: self.font.clone(),
            tracking: self.tracking,
            leading: self.leading,
            italic: self.italic,
            underline: self.underline,
            counter: self.counter,
            ..look.clone()
        }
    }

    /// The style new titles get: bold white-on-anything, with a black outline, as subtitles are.
    pub fn subtitle() -> TitleStyle {
        TitleStyle { outline: Some(Outline { color: [0.0, 0.0, 0.0, 1.0], width: 0.08 }), ..TitleStyle::default() }
    }
}

/// Line alignment of a title.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TitleAlign {
    /// Left edges line up.
    Left,
    /// Centred.
    #[default]
    Center,
    /// Right edges line up.
    Right,
}

/// An outline around a title's glyphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outline {
    /// Straight sRGB colour.
    pub color: [f32; 4],
    /// Width in ems (0.08 is 8 % of the size).
    pub width: f32,
}

/// A clip's placement: offset from the canvas centre as fractions of the canvas, scale and
/// clockwise rotation in degrees.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// A speed outside [0, 100].
    #[error("clip {0:?} has an invalid speed")]
    BadSpeed(String),
    /// A title style value out of range.
    #[error("title {0:?}: {1}")]
    BadStyle(String, &'static str),
    /// Keyframes or animations that cannot be played.
    #[error("clip {0:?}: {1}")]
    BadMotion(String, &'static str),
    /// The list of timelines does not hold together.
    #[error("timelines: {0}")]
    BadTimeline(&'static str),
    /// The brand names a file that is not there, or a colour that is none.
    #[error("brand: {0}")]
    BadBrand(String),
}

impl Default for Project {
    fn default() -> Project {
        Project::new(Canvas::default())
    }
}

impl Project {
    /// An empty project with one video track and one text track.
    pub fn new(canvas: Canvas) -> Project {
        let mut p = Project {
            version: DOCUMENT_VERSION,
            canvas,
            media: BTreeMap::new(),
            tracks: Vec::new(),
            next_id: 1,
            export: ExportSettings::default(),
            timelines: Vec::new(),
            timeline: String::new(),
            markers: Vec::new(),
            cover: None,
            brand: Brand::default(),
            copy: Vec::new(),
            copy_lines: Vec::new(),
        };
        p.tracks = timeline::empty_tracks(&mut p);
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
        self.markers.sort_by_key(|m| m.at);
    }

    /// The first locked track whose clips differ between `before` and this project, if any.
    pub fn changed_locked_track(&self, before: &Project) -> Option<&Track> {
        before.tracks.iter().filter(|t| t.locked).find_map(|old| match self.tracks.iter().find(|t| t.id == old.id) {
            Some(now) if now.locked && now.clips != old.clips => Some(now),
            _ => None,
        })
    }

    /// Lays the clips of each magnetic track end to end from the start, in their current order.
    pub fn pack_magnetic(&mut self) {
        for t in self.tracks.iter_mut().filter(|t| t.magnetic) {
            let mut at = Tick::ZERO;
            // keys are kept from each clip's start, so they move with it
            for c in &mut t.clips {
                c.start = at;
                at = c.end();
            }
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
        if self.timelines.is_empty() != self.timeline.is_empty() {
            return Err(ModelError::BadTimeline("the current timeline is not in the list"));
        }
        for (i, field) in self.copy.iter().enumerate() {
            if !is_field_name(&field.name) || self.copy[..i].iter().any(|f| f.name == field.name) {
                return Err(ModelError::BadBrand(format!(
                    "the copy's field {:?} is misnamed or named twice",
                    field.name
                )));
            }
        }
        for t in &self.timelines {
            if !ids.insert(t.id.clone()) {
                return Err(ModelError::DuplicateId(t.id.clone()));
            }
            if t.id == self.timeline && !t.tracks.is_empty() {
                return Err(ModelError::BadTimeline("the current timeline's tracks are the project's"));
            }
            if let Some(made) = &t.made {
                let mut colours = made.color.iter().chain(&made.color2);
                let sized = made.scale.is_finite() && made.scale > 0.0;
                if !t.compound || colours.any(|c| !(0.0..=1.0).contains(c)) || !sized {
                    return Err(ModelError::BadTimeline("a ready-made piece is a compound clip's, its colours 0 to 1"));
                }
            }
        }
        if !self.timelines.is_empty() && !self.timelines.iter().any(|t| t.id == self.timeline) {
            return Err(ModelError::BadTimeline("the current timeline is not in the list"));
        }
        let parked = self.timelines.iter().flat_map(|t| &t.tracks);
        for track in self.tracks.iter().chain(parked) {
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
                let motion = |why| Err(ModelError::BadMotion(clip.id.clone(), why));
                for list in clip.keys.values() {
                    if list.iter().any(|k| !k.v.is_finite()) {
                        return motion("a keyframe value is not a number");
                    }
                    if list.windows(2).any(|w| w[1].t <= w[0].t) {
                        return motion("keyframes must be in time order, one per instant");
                    }
                    if list.iter().any(|k| !k.ease.is_valid()) {
                        return motion("a keyframe's curve must go forward in time and stay within reach");
                    }
                }
                if [clip.anim.enter, clip.anim.exit].iter().flatten().any(|a| a.duration <= Tick::ZERO) {
                    return motion("an animation must have a positive length");
                }
                if clip.anim.repeat.is_some_and(|l| l.period <= Tick::ZERO) {
                    return motion("a looping animation must take some time a round");
                }
                if clip.fade.fade_in < Tick::ZERO || clip.fade.fade_out < Tick::ZERO {
                    return motion("a fade cannot be negative");
                }
                if let Some(why) = clip.look.problem() {
                    return motion(why);
                }
                if !clip.color.adjust.in_range() {
                    return motion("a colour adjustment is out of range");
                }
                if clip.color.filter.is_some_and(|f| !(0.0..=1.0).contains(&f.strength)) {
                    return motion("a filter's strength must be 0 to 1");
                }
                if let Some(l) = &clip.color.lut {
                    if !(0.0..=1.0).contains(&l.strength) {
                        return motion("a lookup table's strength must be 0 to 1");
                    }
                    if self.media.get(&l.media).is_none_or(|m| m.kind != MediaKind::Lut) {
                        return Err(ModelError::UnknownMedia(clip.id.clone(), l.media.clone()));
                    }
                }
                if let Some(why) = clip.voice.problem() {
                    return motion(why);
                }
                if !clip.voice.is_empty() && !matches!(clip.content, ClipContent::Media { .. }) {
                    return motion("only a clip with sound has a voice to change");
                }
                if let Some(why) = clip.fx.iter().find_map(Fx::problem) {
                    return motion(why);
                }
                if clip.transition.is_some_and(|t| t.duration <= Tick::ZERO) {
                    return motion("a transition must have a positive length");
                }
                if clip.slot == Some(0) {
                    return motion("template slots count from 1");
                }
                if let Some(steady) = &clip.steady {
                    if !(track.kind == TrackKind::Video && matches!(clip.content, ClipContent::Media { .. })) {
                        return motion("only a video on a video track can be held steady");
                    }
                    if !(steady.amount > 0.0 && steady.amount <= 1.0) {
                        return motion("how much a picture is steadied must be above 0 and at most 1");
                    }
                    if steady.step <= Tick::ZERO || steady.path.is_empty() || steady.path.len() > Steady::SAMPLES {
                        return motion("a steadied picture's path needs samples (not too many) a positive time apart");
                    }
                    if steady.path.iter().flatten().any(|v| !v.is_finite()) {
                        return motion("a steadied picture's path must be finite numbers");
                    }
                }
                if clip.slot.is_some()
                    && !(track.kind == TrackKind::Video
                        && matches!(clip.content, ClipContent::Media { .. } | ClipContent::Solid { .. }))
                {
                    return motion("only a picture on a video track can be a template slot");
                }
                let fits = match &clip.content {
                    ClipContent::Media { media, speed, .. } => {
                        let Some(m) = self.media.get(media) else {
                            return Err(ModelError::UnknownMedia(clip.id.clone(), media.clone()));
                        };
                        if !(*speed >= 0.0 && *speed <= 100.0) {
                            return Err(ModelError::BadSpeed(clip.id.clone()));
                        }
                        if let ClipContent::Media { reversed, curve, .. } = &clip.content {
                            if *speed == 0.0 && (*reversed || !curve.is_empty()) {
                                return motion("a freeze frame cannot play backwards or follow a curve");
                            }
                            let (lo, hi) = timing::CURVE_RANGE;
                            if !curve.is_empty()
                                && (curve.len() < 2
                                    || curve.iter().any(|p| !(0.0..=1.0).contains(&p.x) || !(lo..=hi).contains(&p.m))
                                    || curve.windows(2).any(|w| w[1].x <= w[0].x))
                            {
                                return motion(
                                    "a speed curve needs two or more points in order, 0 ≤ x ≤ 1, 0.1 ≤ m ≤ 10",
                                );
                            }
                        }
                        match track.kind {
                            TrackKind::Video => m.kind.pictured(),
                            TrackKind::Audio => m.kind.sounds(),
                            TrackKind::Text => false,
                        }
                    }
                    ClipContent::Solid { .. } => track.kind == TrackKind::Video,
                    ClipContent::Shape { shape } => {
                        if let Some(why) = shape.problem() {
                            return motion(why);
                        }
                        track.kind == TrackKind::Video
                    }
                    ClipContent::Compound { timeline, source_in } => {
                        if !self.timelines.iter().any(|t| t.id == *timeline) {
                            return Err(ModelError::BadTimeline("a compound clip plays a timeline that is not there"));
                        }
                        if *source_in < Tick::ZERO {
                            return motion("a compound clip cannot start before its timeline does");
                        }
                        track.kind == TrackKind::Video
                    }
                    ClipContent::Adjustment => {
                        if clip.color.lut.is_some() {
                            return motion("an adjustment clip takes a filter and adjustments, not a lookup table");
                        }
                        track.kind == TrackKind::Video
                    }
                    ClipContent::Text { size, style, .. } => {
                        let bad = |why| Err(ModelError::BadStyle(clip.id.clone(), why));
                        if !(*size > 0.0 && size.is_finite()) {
                            return bad("size must be positive");
                        }
                        if !(100..=900).contains(&style.weight) {
                            return bad("weight must be 100 to 900");
                        }
                        if !(0.0..=1.0).contains(&style.wrap) {
                            return bad("wrap must be 0 to 1");
                        }
                        if style.outline.as_ref().is_some_and(|o| !(0.0..=0.5).contains(&o.width)) {
                            return bad("outline width must be 0 to 0.5 em");
                        }
                        let (lo, hi) = TitleStyle::SPACING;
                        if ![style.tracking, style.leading].iter().all(|v| v.is_finite() && (lo..=hi).contains(v)) {
                            return bad("letter and line spacing must be -0.5 to 4 em");
                        }
                        if let Some(why) = style.counter.as_ref().and_then(Counter::problem) {
                            return bad(why);
                        }
                        if let Some(font) = &style.font
                            && self.media.get(font).is_none_or(|m| m.kind != MediaKind::Font)
                        {
                            return Err(ModelError::UnknownMedia(clip.id.clone(), font.clone()));
                        }
                        track.kind == TrackKind::Text
                    }
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
        // a timeline cannot play inside itself, however many compound clips lie between
        for t in &self.timelines {
            if self.timeline_reaches(&t.id, &t.id) {
                return Err(ModelError::BadTimeline("a compound clip plays the timeline it is in"));
            }
        }
        if let Some(why) = self.brand.problem() {
            return Err(ModelError::BadBrand(why.into()));
        }
        for (id, kind) in [(&self.brand.font, MediaKind::Font), (&self.brand.logo, MediaKind::Image)] {
            if let Some(id) = id
                && self.media.get(id).is_none_or(|m| m.kind != kind)
            {
                return Err(ModelError::BadBrand(format!("{id:?} is not a file of the kind it must be")));
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

    #[test]
    fn a_title_wearing_a_look_keeps_its_face_and_spacing() {
        let own = TitleStyle {
            align: TitleAlign::Left,
            wrap: 0.5,
            font: Some("m1".into()),
            tracking: 0.2,
            leading: -0.1,
            italic: true,
            underline: true,
            ..TitleStyle::subtitle()
        };
        let look =
            TitleStyle { weight: 900, outline: None, gradient: Some([1.0, 0.0, 0.0, 1.0]), ..TitleStyle::default() };
        let worn = own.wearing(&look);
        assert_eq!((worn.weight, worn.outline.clone(), worn.gradient), (900, None, look.gradient));
        assert_eq!(
            (worn.align, worn.wrap, worn.font.as_deref(), worn.tracking, worn.leading, worn.italic, worn.underline),
            (TitleAlign::Left, 0.5, Some("m1"), 0.2, -0.1, true, true)
        );
        // styles written before there were fonts and spacing read as the built-in face, unspaced
        let old: TitleStyle = serde_json::from_str(r#"{"weight": 700, "align": "center", "wrap": 0.9}"#).unwrap();
        assert_eq!((old.font, old.tracking, old.leading, old.italic, old.underline), (None, 0.0, 0.0, false, false));
    }

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
