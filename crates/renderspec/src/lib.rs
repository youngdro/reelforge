//! RenderSpec: the contract between the editor and every renderer.
//!
//! A [`RenderSpec`] says everything a renderer needs to draw a finished edit and nothing else: the
//! output settings, the assets by content hash, and a flat list of layers and audio clips in exact
//! integer [`Tick`]s. All editing meaning has already been resolved by the editor's compiler
//! (`reelforge-compile`): transitions are overlaps with opacity keys, templates are filled, speed
//! is a time map, captions are text layers. Heavy analysis (cut-out masks, stabilisation, speech)
//! arrives as precomputed assets, so a renderer never runs a model and its output is reproducible.
//!
//! The renderer depends on this crate and on `reelforge-time` only. It never sees the project
//! model, the command registry or the interface, which is what lets a render run in the editor's
//! process by default, or on a separate render service from the same JSON.
//!
//! Compatibility rules:
//!
//! 1. `spec` names the format version ([`SPEC_VERSION`]). A renderer refuses a version it does not
//!    know instead of guessing.
//! 2. Effects are versioned by id (`color.brightness@1`). A renderer refuses an effect it does not
//!    implement ([`RenderSpec::required_features`] lists what a spec needs) instead of drawing
//!    something else.
//! 3. The JSON Schema in `schema/renderspec-1.schema.json` is generated from these types
//!    ([`json_schema`]) and checked in; a test fails when they drift.

use std::collections::{BTreeMap, BTreeSet};

use reelforge_time::{FrameRate, Tick};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod validate;

pub use validate::{ValidationError, validate};

/// The format version this crate reads and writes.
pub const SPEC_VERSION: &str = "renderspec/1";

/// A finished edit, ready to render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderSpec {
    /// Format version; must be [`SPEC_VERSION`].
    pub spec: String,
    /// What the renderer must support.
    #[serde(default)]
    pub engine: EngineRequirements,
    /// The picture and sound to produce.
    pub output: Output,
    /// Every file the edit reads, by id. Ids are local to this spec.
    #[serde(default)]
    pub assets: BTreeMap<String, Asset>,
    /// Picture layers, bottom first.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Sound, mixed together.
    #[serde(default)]
    pub audio: Vec<AudioClip>,
    /// Compositions by id (feature `comp/1`): each a picture and a sound made of layers and
    /// audio clips of its own, shown by [`LayerContent::Comp`] layers and played by audio clips
    /// that name it. Ids are local to this spec.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub comps: BTreeMap<String, Comp>,
}

/// A composition: an edit inside the edit (a compound clip's timeline). Its picture is as large
/// as the output and see-through where no layer draws; its time starts at 0.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Comp {
    /// Picture layers, bottom first.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Sound, mixed together.
    #[serde(default)]
    pub audio: Vec<AudioClip>,
}

/// What a renderer must provide to draw this spec.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineRequirements {
    /// The oldest renderer version that draws this spec correctly (semver), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    /// Feature ids the spec uses beyond the base format, such as `text/1`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

/// Output settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Output {
    /// Width in pixels; even, at most 8192.
    pub width: u32,
    /// Height in pixels; even, at most 8192.
    pub height: u32,
    /// Frame rate.
    pub fps: FrameRate,
    /// Length of the output.
    pub duration: Tick,
    /// Colour under every layer (straight sRGB, 0 to 1).
    #[serde(default = "black")]
    pub background: Color,
    /// How to encode the picture; absent when only frames are wanted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoEncoding>,
    /// How to encode the sound; absent for a silent output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioEncoding>,
    /// The kind of file to write.
    #[serde(default, skip_serializing_if = "OutputFormat::is_mp4")]
    pub format: OutputFormat,
}

/// The kind of file an output is written as.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// The picture and the sound, in an MP4.
    #[default]
    Mp4,
    /// The picture alone as an animated GIF that plays round and round (feature `encode.gif/1`),
    /// at `video.size` when one is given; a frame lasts `1 ÷ fps`, to the hundredth of a second.
    Gif,
    /// The sound alone, encoded as `audio` says, in an M4A (feature `encode.audio/1`).
    M4a,
    /// The sound alone as 16-bit PCM in a WAV, at `audio`'s sample rate (feature
    /// `encode.audio/1`).
    Wav,
}

impl OutputFormat {
    fn is_mp4(&self) -> bool {
        *self == OutputFormat::Mp4
    }

    /// The extension of such a file.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Mp4 => "mp4",
            OutputFormat::Gif => "gif",
            OutputFormat::M4a => "m4a",
            OutputFormat::Wav => "wav",
        }
    }
}

fn black() -> Color {
    Color([0.0, 0.0, 0.0, 1.0])
}

/// A straight (not premultiplied) sRGB colour with alpha, each 0 to 1.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Color(pub [f32; 4]);

/// Video encoding settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoEncoding {
    /// Codec.
    pub codec: VideoCodec,
    /// Constant quality (lower is better), when the encoder supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crf: Option<u8>,
    /// Target bitrate in kbit/s: used when no `crf` is given, or by encoders without a
    /// constant-quality mode (hardware ones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
    /// Encode at this size instead of the output's (feature `encode.size/1`): the frame is drawn
    /// `width ÷ output.width` times the output size. Both even, and the height must be
    /// `round(output.height × width ÷ output.width)`, so the picture is not stretched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
}

impl VideoEncoding {
    /// How much larger than the output the frames are drawn: 1 without a `size`.
    pub fn scale(&self, output_width: u32) -> f32 {
        self.size.map_or(1.0, |[w, _]| w as f32 / output_width as f32)
    }
}

/// The height a `width`-wide picture of a `ow` by `oh` output has, rounded as renderers round it.
pub fn scaled_height(ow: u32, oh: u32, width: u32) -> u32 {
    (oh as f32 * (width as f32 / ow as f32)).round() as u32
}

/// Video codecs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// AV1.
    Av1,
}

/// Audio encoding settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioEncoding {
    /// Codec.
    pub codec: AudioCodec,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Bitrate in kbit/s.
    pub bitrate_kbps: u32,
}

/// Audio codecs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    /// AAC-LC.
    Aac,
    /// Opus.
    Opus,
}

/// A file the edit reads, addressed by content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    /// What the file is.
    pub kind: AssetKind,
    /// SHA-256 of the file's bytes, lowercase hex. This, not the URI, is the asset's identity.
    pub sha256: String,
    /// Where to fetch it: `file:`, `https:`, `s3:`, `opfs:`… A renderer may resolve the hash
    /// through its own cache instead.
    pub uri: String,
    /// For a derived asset (a mask sequence, a stabilisation track), the asset it was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    /// What the editor measured when it imported the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
}

/// Asset kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// Video, possibly with sound.
    Video,
    /// A still image.
    Image,
    /// Sound only.
    Audio,
    /// A font file.
    Font,
    /// Per-frame alpha masks derived from another asset.
    MaskSequence,
    /// A colour lookup table.
    Lut,
    /// A Lottie animation (feature `lottie/1`): Bodymovin JSON or a dotLottie archive. It has
    /// no frames of its own: any instant is drawn, at the size the asset's `probe` gives, and
    /// past its length it goes round again. A media layer shows it as it shows a video.
    Lottie,
}

/// Measured properties of an asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// Picture width in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Picture height in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Length, for time-based media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Tick>,
}

/// One picture layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Id, unique in the spec; the editor uses its clip id so a render maps back to the edit.
    pub id: String,
    /// When the layer starts on the output timeline.
    pub at: Tick,
    /// How long it lasts; positive.
    pub dur: Tick,
    /// What it shows.
    pub content: LayerContent,
    /// Where it sits. Keyframe times are relative to the layer's start.
    #[serde(default)]
    pub transform: Transform,
    /// Opacity, 0 to 1.
    #[serde(default = "one")]
    pub opacity: Animated<f32>,
    /// How it combines with what is below.
    #[serde(default)]
    pub blend: Blend,
    /// A `mask_sequence` asset whose alpha multiplies this layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    /// Effects, applied in order to the layer's own picture before it is placed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx: Vec<Effect>,
    /// Edges cut off a media layer's picture before it is fitted (feature `crop/1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    /// A media layer's picture mirrored left to right (feature `flip/1`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    /// A media layer's picture mirrored top to bottom (feature `flip/1`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
    /// A shape that shows part of the layer, in the layer's own picture (feature `mask.shape/1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_mask: Option<ShapeMask>,
    /// The crop over the layer's time, in place of `crop` (feature `crop/2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_keys: Option<CropKeys>,
    /// Where and how big `shape_mask` is over the layer's time, in place of its own centre, size,
    /// rotation and feather (feature `mask.shape/2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_mask_keys: Option<ShapeMaskKeys>,
    /// The layer that carries this one, among the layers it is listed with (feature
    /// `layer.parent/1`). This layer is placed as it would be alone, and then moved, scaled and
    /// turned as the parent's transform moves the frame: its `position` is counted from the
    /// parent's (a layer at the centre sits on the parent's anchor), along the parent's own
    /// turned and scaled axes. A parent at the centre, at scale 1 and unturned changes nothing.
    /// The parent's transform is taken at the same output instant on the parent's own clock,
    /// held at its first value before the parent starts and at its last after it ends, so the
    /// parent need not be on screen; nothing else of the parent (its opacity, crop, effects) is
    /// passed on. A parent may have a parent, eight deep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// A crop that changes: each edge as in [`Crop`], keyed. A renderer keeps at least 1 % of the
/// picture, shrinking a pair of opposite edges in proportion where their keys would cut more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CropKeys {
    /// From the left edge.
    #[serde(default = "zero")]
    pub left: Animated<f32>,
    /// From the top edge.
    #[serde(default = "zero")]
    pub top: Animated<f32>,
    /// From the right edge.
    #[serde(default = "zero")]
    pub right: Animated<f32>,
    /// From the bottom edge.
    #[serde(default = "zero")]
    pub bottom: Animated<f32>,
}

impl CropKeys {
    /// The crop at `t` from the layer's start.
    pub fn sample(&self, t: Tick) -> Crop {
        let [l, tp, r, b] = [&self.left, &self.top, &self.right, &self.bottom].map(|e| e.sample(t).clamp(0.0, 0.98));
        let fit = |a: f32, b: f32| if a + b > 0.99 { (a * 0.99 / (a + b), b * 0.99 / (a + b)) } else { (a, b) };
        let ((left, right), (top, bottom)) = (fit(l, r), fit(tp, b));
        Crop { left, top, right, bottom }
    }
}

/// A shape mask that moves: the values of [`ShapeMask`] that can, keyed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeMaskKeys {
    /// The centre.
    pub center: Animated<[f32; 2]>,
    /// Width and height.
    pub size: Animated<[f32; 2]>,
    /// Clockwise turn, degrees.
    pub rotation: Animated<f32>,
    /// Width of the soft edge.
    pub feather: Animated<f32>,
}

impl ShapeMaskKeys {
    /// `mask` as it is at `t` from the layer's start.
    pub fn sample(&self, mask: ShapeMask, t: Tick) -> ShapeMask {
        let size = self.size.sample(t);
        ShapeMask {
            center: self.center.sample(t),
            size: [size[0].max(0.0), size[1].max(0.0)],
            rotation: self.rotation.sample(t),
            feather: self.feather.sample(t).max(0.0),
            ..mask
        }
    }
}

/// Edges cut off a picture, as fractions of its width (`left`, `right`) and height (`top`,
/// `bottom`); what is left must be at least 1 % of each.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    /// From the left edge.
    #[serde(default)]
    pub left: f32,
    /// From the top edge.
    #[serde(default)]
    pub top: f32,
    /// From the right edge.
    #[serde(default)]
    pub right: f32,
    /// From the bottom edge.
    #[serde(default)]
    pub bottom: f32,
}

/// A shape that shows part of a layer.
///
/// It is placed in the layer's own picture (after any crop), so it moves, turns and scales with
/// the layer. Lengths are in units of the picture's shorter side, so a circle stays round.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeMask {
    /// Which shape.
    pub shape: MaskShape,
    /// Its centre, as fractions of the picture's width and height (0.5, 0.5 is the middle).
    #[serde(default = "middle")]
    pub center: [f32; 2],
    /// Width and height (for `mirror`, the height is the band's thickness; `linear` ignores it).
    #[serde(default = "half")]
    pub size: [f32; 2],
    /// Clockwise turn, degrees.
    #[serde(default)]
    pub rotation: f32,
    /// Width of the soft edge, 0 for a hard one.
    #[serde(default)]
    pub feather: f32,
    /// Show the outside instead.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
}

fn middle() -> [f32; 2] {
    [0.5, 0.5]
}

fn half() -> [f32; 2] {
    [0.5, 0.5]
}

/// Mask shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    /// Everything above a line through the centre (turned by `rotation`).
    Linear,
    /// A band through the centre.
    Mirror,
    /// An ellipse.
    Circle,
    /// A rectangle.
    Rectangle,
}

impl ShapeMask {
    /// How much of the layer shows, 0 to 1, at `uv` (fractions of a `w` by `h` picture).
    pub fn coverage(&self, uv: [f32; 2], w: f32, h: f32) -> f32 {
        let unit = w.min(h).max(1e-6);
        let (dx, dy) = ((uv[0] - self.center[0]) * w / unit, (uv[1] - self.center[1]) * h / unit);
        let (sin, cos) = (-self.rotation.to_radians()).sin_cos();
        let (x, y) = (dx * cos - dy * sin, dx * sin + dy * cos);
        let (hw, hh) = (self.size[0].max(0.0) / 2.0, self.size[1].max(0.0) / 2.0);
        let d = match self.shape {
            MaskShape::Linear => y,
            MaskShape::Mirror => y.abs() - hh,
            MaskShape::Circle => {
                let (rx, ry) = (hw.max(1e-6), hh.max(1e-6));
                (((x / rx) * (x / rx) + (y / ry) * (y / ry)).sqrt() - 1.0) * rx.min(ry)
            }
            MaskShape::Rectangle => {
                let (qx, qy) = (x.abs() - hw, y.abs() - hh);
                (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt() + qx.max(qy).min(0.0)
            }
        };
        let m = if self.feather > 0.0 {
            (0.5 - d / self.feather).clamp(0.0, 1.0)
        } else if d <= 0.0 {
            1.0
        } else {
            0.0
        };
        if self.invert { 1.0 - m } else { m }
    }
}

fn one() -> Animated<f32> {
    Animated::Constant(1.0)
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerContent {
    /// Frames of a video or image asset.
    Media {
        /// The asset id.
        asset: String,
        /// Which source instant each output instant shows.
        #[serde(default)]
        time: TimeMap,
        /// How the picture is fitted to the frame before the transform applies.
        #[serde(default)]
        fit: Fit,
        /// What shows when an output instant falls between two frames of a video: what a clip
        /// slowed down, or a file of fewer frames a second than the output, is made smooth with.
        /// A still ignores it.
        #[serde(default, skip_serializing_if = "Between::is_nearest")]
        between: Between,
    },
    /// A flat colour filling the frame.
    Solid {
        /// The colour.
        color: Color,
    },
    /// Set text (feature `text/1`).
    Text(Box<TextContent>),
    /// A drawn shape (feature `shape/1`): a box, ellipse, line, polygon, star, price tag or
    /// arrow, filled and outlined, its outline drawn on over time. Its centre is placed by the
    /// layer's transform; its size is in output pixels, before the layer's scale.
    Shape(Box<ShapeContent>),
    /// The picture of a composition (feature `comp/1`): drawn first, on a see-through frame the
    /// size of the output, then placed, cropped, masked and given this layer's effects as one
    /// picture.
    Comp {
        /// The composition's id in [`RenderSpec::comps`].
        comp: String,
        /// The composition's instant shown at the layer's start; it runs at speed 1 from there.
        #[serde(default)]
        from: Tick,
    },
}

/// Source time for a media layer or audio clip: `source = source_in + ∫ speed dt` from the owner's
/// start. With no `ramp` the speed is constant, so `source = source_in + (t - at) * speed`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeMap {
    /// Source instant shown at the owner's start.
    #[serde(default)]
    pub source_in: Tick,
    /// Playback speed, -100 to 100. 0 holds `source_in` (a freeze frame, feature `time.hold/1`);
    /// below 0 plays backwards (feature `time.reverse/1`). Ignored when `ramp` is given.
    #[serde(default = "unit_speed")]
    pub speed: f64,
    /// A speed that changes over time (feature `time.ramp/1`): straight lines between keys, held
    /// before the first and after the last. Key times are from the owner's start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ramp: Vec<SpeedKey>,
}

/// One point of a speed ramp.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedKey {
    /// Time from the owner's start.
    pub t: Tick,
    /// Speed there, -100 to 100.
    pub speed: f64,
}

fn unit_speed() -> f64 {
    1.0
}

impl Default for TimeMap {
    fn default() -> TimeMap {
        TimeMap { source_in: Tick::ZERO, speed: 1.0, ramp: vec![] }
    }
}

impl TimeMap {
    /// A constant speed from `source_in`.
    pub fn new(source_in: Tick, speed: f64) -> TimeMap {
        TimeMap { source_in, speed, ramp: vec![] }
    }

    /// The source instant for an offset into the owner.
    pub fn source_at(&self, offset: Tick) -> Tick {
        if self.ramp.is_empty() {
            return self.source_in + offset.scale(self.speed);
        }
        self.source_in + Tick(self.travel(offset).round() as i64)
    }

    /// Source ticks covered from the owner's start to `offset`: the area under the ramp.
    fn travel(&self, offset: Tick) -> f64 {
        let r = &self.ramp;
        let at = |t: Tick| -> f64 {
            // the ramp's speed at t
            if t <= r[0].t {
                return r[0].speed;
            }
            for w in r.windows(2) {
                if t < w[1].t {
                    let f = (t - w[0].t).0 as f64 / (w[1].t - w[0].t).0 as f64;
                    return w[0].speed + (w[1].speed - w[0].speed) * f;
                }
            }
            r[r.len() - 1].speed
        };
        let (lo, hi, sign) = if offset >= Tick::ZERO { (Tick::ZERO, offset, 1.0) } else { (offset, Tick::ZERO, -1.0) };
        // the speed is a straight line between these instants, so each piece is a trapezoid
        let mut cuts = vec![lo, hi];
        cuts.extend(r.iter().map(|k| k.t).filter(|t| *t > lo && *t < hi));
        cuts.sort();
        let area: f64 = cuts.windows(2).map(|w| (w[1] - w[0]).0 as f64 * (at(w[0]) + at(w[1])) / 2.0).sum();
        sign * area
    }

    /// Whether any part of the map plays backwards.
    pub fn reverses(&self) -> bool {
        if self.ramp.is_empty() { self.speed < 0.0 } else { self.ramp.iter().any(|k| k.speed < 0.0) }
    }

    /// Whether the map holds one instant (a freeze frame).
    pub fn holds(&self) -> bool {
        self.ramp.is_empty() && self.speed == 0.0
    }

    /// The feature ids the map needs beyond a constant forward speed.
    pub fn features(&self) -> impl Iterator<Item = &'static str> {
        [
            self.holds().then_some("time.hold/1"),
            self.reverses().then_some("time.reverse/1"),
            (!self.ramp.is_empty()).then_some("time.ramp/1"),
        ]
        .into_iter()
        .flatten()
    }
}

/// What a video layer shows at an instant between two of its file's frames.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Between {
    /// The frame of that instant, held until the next.
    #[default]
    Nearest,
    /// The two frames mixed, by how far along from the one to the other the instant is (feature
    /// `frames.blend/1`).
    Blend,
    /// A frame made in between (feature `frames.flow/1`): what moved from the one frame to the
    /// other is followed, and drawn as far along its way as the instant is. Where nothing can
    /// be followed, the two are mixed as for `blend`.
    Flow,
}

impl Between {
    /// Whether frames are shown as they are.
    pub fn is_nearest(&self) -> bool {
        *self == Between::Nearest
    }

    /// The feature id a renderer needs for it, if any.
    pub fn feature(self) -> Option<&'static str> {
        match self {
            Between::Nearest => None,
            Between::Blend => Some("frames.blend/1"),
            Between::Flow => Some("frames.flow/1"),
        }
    }
}

/// How media is fitted to the output frame.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Whole picture visible, bars where the shapes differ.
    #[default]
    Contain,
    /// Frame covered, picture cropped where the shapes differ.
    Cover,
    /// Stretched to the frame.
    Stretch,
}

/// Text content (`text/1`).
///
/// The block is laid out, then its centre is placed by the layer's transform; `size` and `width`
/// are in output pixels, before the layer's scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    /// The words; `\n` breaks a line.
    pub text: String,
    /// A `font` asset; absent means the renderer's bundled default face (Noto Sans SC, which also
    /// covers Latin). Characters the face lacks fall back to the other faces the renderer has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Size of the em in output pixels.
    pub size: f32,
    /// Fill colour.
    pub color: Color,
    /// Weight, 100 (thin) to 900 (black); 400 is regular, 700 bold.
    #[serde(default = "regular")]
    pub weight: u16,
    /// Alignment of the lines within the block.
    #[serde(default)]
    pub align: TextAlign,
    /// Wrap width in output pixels; absent, lines break only at `\n`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// An outline around the glyphs, drawn beneath the fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<TextStroke>,
    /// A box behind the block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<TextBackground>,
    /// With `text/2`: the fill fades from `color` at the block's top to this at its bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Color>,
    /// With `text/2`: a second outline, outside `stroke` (its width counts from `stroke`'s edge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer: Option<TextStroke>,
    /// With `text/2`: a soft shadow of the glyphs and their outlines, beneath them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<TextShadow>,
    /// With `text/3`: how much of the text shows, 0 to 1, counted in characters from the start
    /// (a typewriter). The lines stay where the whole text puts them; a box behind each line
    /// grows with what shows of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<Animated<f32>>,
    /// With `text/4`: space added after every character, in ems (negative: closer together).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tracking: f32,
    /// With `text/4`: space added between lines, in ems (negative: closer together).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub leading: f32,
    /// With `text/4`: slanted; the face's own italic when it has one, otherwise sheared.
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    /// With `text/4`: a line under each line of text, outlined and shadowed with the glyphs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// With `text/5`: the characters move one after another, each about its own centre, from where
    /// the layout puts them. Several movements add up. Boxes and underlines stay where they are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chars: Vec<CharMotion>,
    /// With `text/6`: a number that changes over time (a price, a count, a clock), written into
    /// `text` where it says `{n}`, or as the whole text when it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<NumberText>,
}

/// A number shown by a text (`text/6`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NumberText {
    /// The value over the layer's time.
    pub value: Animated<f32>,
    /// Digits after the decimal point, 0 to 4.
    #[serde(default)]
    pub decimals: u8,
    /// Thousands separated by commas.
    #[serde(default, skip_serializing_if = "is_false")]
    pub group: bool,
    /// Written as a clock of whole seconds: `m:ss`, or `h:mm:ss` from an hour.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clock: bool,
}

impl NumberText {
    /// `value` written as this number is.
    pub fn format(&self, value: f32) -> String {
        if self.clock {
            let s = value.max(0.0).round() as u64;
            let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
            return if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") };
        }
        let d = self.decimals.min(4) as usize;
        let mut out = format!("{:.*}", d, value.abs() as f64);
        if self.group {
            let (whole, rest) = out.split_at(out.find('.').unwrap_or(out.len()));
            let digits: Vec<char> = whole.chars().collect();
            let mut g = String::new();
            for (i, c) in digits.iter().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(3) {
                    g.push(',');
                }
                g.push(*c);
            }
            out = g + rest;
        }
        // a value that rounds to zero is not negative
        if value < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
            out.insert(0, '-');
        }
        out
    }

    /// `template` with `{n}` replaced by `value`, or the number alone when it has none.
    pub fn fill(&self, template: &str, value: f32) -> String {
        let n = self.format(value);
        if template.contains("{n}") { template.replace("{n}", &n) } else { n }
    }
}

/// One way a text's characters move one after another (`text/5`). Characters are counted from
/// the start of the text, white space left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CharMotion {
    /// How far along the movement is. For `settle` and `spring`: 0, no character has started, to
    /// 1, every character is in place. For `wave` and `hop`: rounds, 1 a whole round.
    pub progress: Animated<f32>,
    /// How much later each character starts than the one before. For `settle` and `spring`, in
    /// units of one character's own time: 0 moves them all together, 1 starts each as the one
    /// before arrives. For `wave` and `hop`, in rounds.
    pub step: f32,
    /// How a character travels.
    pub curve: CharCurve,
    /// Where a character is when it is furthest from rest.
    pub pose: CharPose,
    /// Counted from the last character instead of the first.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
}

/// How a character travels between `pose` and rest.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CharCurve {
    /// From `pose` to rest once, fast at first and slowing into place; its opacity in a straight line.
    Settle,
    /// As `settle`, overshooting rest a little before it comes back.
    Spring,
    /// To `pose`, back through rest to its opposite and back, every round.
    Wave,
    /// To `pose` and back in the first half of every round, at rest in the second.
    Hop,
}

/// Where a character is, against where the layout puts it.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CharPose {
    /// Moved `[right, down]`, in ems.
    #[serde(default)]
    pub offset: [f32; 2],
    /// Scaled about its own centre.
    #[serde(default = "whole")]
    pub scale: f32,
    /// Turned about its own centre, degrees clockwise.
    #[serde(default)]
    pub rotation: f32,
    /// How much of it shows, 0 to 1.
    #[serde(default = "whole")]
    pub opacity: f32,
}

fn whole() -> f32 {
    1.0
}

impl CharPose {
    /// Where the layout puts it.
    pub const REST: CharPose = CharPose { offset: [0.0; 2], scale: 1.0, rotation: 0.0, opacity: 1.0 };

    /// Both at once.
    pub fn and(self, o: CharPose) -> CharPose {
        CharPose {
            offset: [self.offset[0] + o.offset[0], self.offset[1] + o.offset[1]],
            scale: self.scale * o.scale,
            rotation: self.rotation + o.rotation,
            opacity: self.opacity * o.opacity,
        }
    }

    /// This pose `w` of the way from rest (beyond 1, or the other way from rest below 0), showing
    /// as much as it does `shown` of the way from rest.
    fn weighed(self, w: f32, shown: f32) -> CharPose {
        CharPose {
            offset: [self.offset[0] * w, self.offset[1] * w],
            scale: (1.0 + (self.scale - 1.0) * w).max(0.0),
            rotation: self.rotation * w,
            opacity: (1.0 + (self.opacity - 1.0) * shown.clamp(0.0, 1.0)).clamp(0.0, 1.0),
        }
    }
}

impl CharMotion {
    /// Where character `i` of `n` is when the movement is at `progress`.
    pub fn at(&self, i: usize, n: usize, progress: f32) -> CharPose {
        let i = if self.reverse { n.saturating_sub(1 + i) } else { i } as f32;
        let step = self.step.max(0.0);
        match self.curve {
            CharCurve::Settle | CharCurve::Spring => {
                // one character's share of the whole, so that the last one ends at 1
                let own = 1.0 / (1.0 + step * n.saturating_sub(1) as f32);
                let u = ((progress - i * step * own) / own).clamp(0.0, 1.0);
                let away = if self.curve == CharCurve::Settle {
                    (1.0 - u).powi(3)
                } else {
                    // back-out: about a tenth past rest, then back
                    let (c, v) = (1.70158f32, u - 1.0);
                    -((c + 1.0) * v * v * v + c * v * v)
                };
                // a spring is all there a third of the way in, so its overshoot shows
                let shown = if self.curve == CharCurve::Settle { 1.0 - u } else { 1.0 - (u * 3.0).min(1.0) };
                self.pose.weighed(away, shown)
            }
            CharCurve::Wave | CharCurve::Hop => {
                let w = (std::f32::consts::TAU * (progress - i * step)).sin();
                let w = if self.curve == CharCurve::Hop { w.max(0.0) } else { w };
                self.pose.weighed(w, w.abs())
            }
        }
    }
}

fn regular() -> u16 {
    400
}

impl TextContent {
    /// Plain text: the default face, regular, centred, no wrapping, outline or box.
    pub fn new(text: impl Into<String>, size: f32, color: Color) -> TextContent {
        TextContent {
            text: text.into(),
            font: None,
            size,
            color,
            weight: regular(),
            align: TextAlign::Center,
            width: None,
            stroke: None,
            background: None,
            gradient: None,
            outer: None,
            shadow: None,
            reveal: None,
            tracking: 0.0,
            leading: 0.0,
            italic: false,
            underline: false,
            chars: vec![],
            number: None,
        }
    }
}

/// How lines line up within a text block.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Left edges line up.
    Left,
    /// Centred.
    #[default]
    Center,
    /// Right edges line up.
    Right,
}

/// An outline around text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStroke {
    /// Colour.
    pub color: Color,
    /// Width outside the glyph edge, in output pixels.
    pub width: f32,
}

/// A box behind text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextBackground {
    /// Colour.
    pub color: Color,
    /// Space between the text and the box edge, in ems.
    #[serde(default = "quarter")]
    pub padding: f32,
    /// With `text/2`: corner radius, in ems.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f32,
    /// With `text/3`: a box behind each line, hugging it, instead of one behind the block.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lines: bool,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// A shadow beneath text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextShadow {
    /// Colour.
    pub color: Color,
    /// Offset `[right, down]` in ems, on the frame (it does not turn with the text).
    pub offset: [f32; 2],
    /// Softness: the blur's standard deviation, in ems.
    #[serde(default)]
    pub blur: f32,
}

impl TextContent {
    /// Whether it uses anything beyond `text/1`.
    pub fn needs_v2(&self) -> bool {
        self.gradient.is_some()
            || self.outer.is_some()
            || self.shadow.is_some()
            || self.background.as_ref().is_some_and(|b| b.radius != 0.0)
    }

    /// Whether it uses anything beyond `text/2`.
    pub fn needs_v3(&self) -> bool {
        self.reveal.is_some() || self.background.as_ref().is_some_and(|b| b.lines)
    }

    /// Whether it uses anything beyond `text/3`.
    pub fn needs_v4(&self) -> bool {
        self.tracking != 0.0 || self.leading != 0.0 || self.italic || self.underline
    }

    /// Whether it uses anything beyond `text/4`.
    pub fn needs_v5(&self) -> bool {
        !self.chars.is_empty()
    }

    /// Whether it uses anything beyond `text/5`.
    pub fn needs_v6(&self) -> bool {
        self.number.is_some()
    }
}

/// A drawn shape (`shape/1`).
///
/// Lengths are in output pixels before the layer's scale. The shape fills a `size` box centred on
/// the layer's centre; its outline starts at the box's top-left corner (the top of an ellipse,
/// the left end of a line, the first point of a polygon or star, at the top) and runs clockwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeContent {
    /// Which shape.
    pub kind: ShapeKind,
    /// Width and height of its box.
    pub size: [f32; 2],
    /// Corner radius of a rectangle or a tag.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f32,
    /// Sides of a polygon, points of a star: 3 to 64.
    #[serde(default = "five")]
    pub points: u32,
    /// How far in a star's inner corners are, 0.05 to 1 of the way to its points.
    #[serde(default = "half_f")]
    pub inner: f32,
    /// What fills it; none leaves only the outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// The outline, centred on the shape's edge (a line's only ink).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ShapeStroke>,
    /// A soft shadow of the fill and outline, beneath them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShapeShadow>,
    /// The part of the outline drawn, as fractions of its length from its start: an outline that
    /// draws itself on (its end rising from 0 to 1) or a stroke that runs round it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<Trim>,
    /// How much of the fill shows, 0 to 1 (1 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_opacity: Option<Animated<f32>>,
}

fn five() -> u32 {
    5
}

fn half_f() -> f32 {
    0.5
}

/// Shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// A box, its corners rounded by `radius`.
    Rectangle,
    /// An ellipse filling the box.
    Ellipse,
    /// A line across the middle of the box, left to right; it shows by its stroke.
    Line,
    /// A regular polygon of `points` sides, a corner at the top, stretched to the box.
    Polygon,
    /// A star of `points` points, stretched to the box: a burst badge with many.
    Star,
    /// A price tag: a box pointed at its left end, with a hole near the point.
    Tag,
    /// An arrow pointing right.
    Arrow,
}

/// How a shape is filled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Paint {
    /// One colour.
    Solid {
        /// The colour.
        color: Color,
    },
    /// From one colour to another along a line across the box.
    Linear {
        /// The colour where the line starts.
        from: Color,
        /// The colour where it ends.
        to: Color,
        /// Which way the line runs, degrees clockwise from left-to-right (90: top to bottom).
        #[serde(default)]
        angle: f32,
    },
    /// From one colour at the box's centre to another at its edge.
    Radial {
        /// The colour at the centre.
        from: Color,
        /// The colour at the edge (and beyond, in the corners).
        to: Color,
    },
}

/// A shape's outline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeStroke {
    /// Colour.
    pub color: Color,
    /// Width, centred on the edge.
    pub width: f32,
}

/// A shape's shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeShadow {
    /// Colour.
    pub color: Color,
    /// Offset `[right, down]`, on the frame (it does not turn with the shape).
    pub offset: [f32; 2],
    /// Softness: the blur's standard deviation.
    #[serde(default)]
    pub blur: f32,
}

/// The part of an outline that is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trim {
    /// Where the drawn part starts, 0 to 1.
    #[serde(default = "zero")]
    pub start: Animated<f32>,
    /// Where it ends, 0 to 1; nothing is drawn where it is not past `start`.
    #[serde(default = "one")]
    pub end: Animated<f32>,
}

fn quarter() -> f32 {
    0.25
}

/// A layer's placement, about the frame's centre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// Offset of the layer's centre from the frame's centre, as fractions of the frame (x right,
    /// y down).
    #[serde(default = "zero2")]
    pub position: Animated<[f32; 2]>,
    /// Uniform scale; 1 is the fitted size.
    #[serde(default = "one")]
    pub scale: Animated<f32>,
    /// Clockwise rotation in degrees.
    #[serde(default = "zero")]
    pub rotation: Animated<f32>,
    /// Width and height scaled apart, `[across, down]`, on top of `scale` (feature
    /// `transform.stretch/1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stretch: Option<Animated<[f32; 2]>>,
    /// The point of the layer's picture that sits at `position`, and that the layer is scaled and
    /// turned about: fractions of the picture as it shows (after any crop) from its top left.
    /// Its centre, `[0.5, 0.5]`, when absent (feature `transform.anchor/1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f32; 2]>,
    /// The layer turned in space, seen in perspective (feature `transform.3d/1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
}

/// A layer turned out of the frame's plane about its anchor, after its `rotation` in the plane
/// is undone and before it is done again: first about the horizontal axis, then the vertical.
/// It is seen from straight in front of where its anchor is, so a layer turned on its own closes
/// like a door or a lid evenly, wherever in the frame it is. A layer carried by another
/// ([`Layer::parent`]) that is turned is seen from where the outermost turned layer carrying it
/// is, so the two stay one object. Past 90 degrees a layer shows its back, mirrored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// Degrees about the horizontal axis: positive tips the top edge away.
    #[serde(default = "zero")]
    pub x: Animated<f32>,
    /// Degrees about the vertical axis: positive swings the right edge away.
    #[serde(default = "zero")]
    pub y: Animated<f32>,
    /// How far the eye is, in frame heights: nearer is a stronger perspective. Positive.
    #[serde(default = "eye")]
    pub distance: f32,
}

fn eye() -> f32 {
    Turn::DISTANCE
}

impl Turn {
    /// How far the eye is unless told: two frame heights.
    pub const DISTANCE: f32 = 2.0;

    /// Held at `x` and `y` degrees, seen from the usual distance.
    pub fn held(x: f32, y: f32) -> Turn {
        Turn { x: Animated::Constant(x), y: Animated::Constant(y), distance: Turn::DISTANCE }
    }
}

/// How many layers deep one may carry another ([`Layer::parent`]).
pub const PARENTS: usize = 8;

fn zero() -> Animated<f32> {
    Animated::Constant(0.0)
}

fn zero2() -> Animated<[f32; 2]> {
    Animated::Constant([0.0, 0.0])
}

impl Default for Transform {
    fn default() -> Transform {
        Transform { position: zero2(), scale: one(), rotation: zero(), stretch: None, anchor: None, turn: None }
    }
}

/// Blend modes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    /// Over.
    #[default]
    Normal,
    /// Sum.
    Add,
    /// Product.
    Multiply,
    /// Inverse product.
    Screen,
    /// Multiply in the dark parts, screen in the bright (feature `blend/2`, as are those below).
    Overlay,
    /// The darker of the two.
    Darken,
    /// The lighter of the two.
    Lighten,
    /// A gentle overlay.
    SoftLight,
    /// Overlay, led by the layer.
    HardLight,
    /// The difference.
    Difference,
    /// Brightens what is below.
    ColorDodge,
    /// Darkens what is below.
    ColorBurn,
}

impl Blend {
    /// Whether the mode needs `blend/2`.
    pub fn is_v2(self) -> bool {
        !matches!(self, Blend::Normal | Blend::Add | Blend::Multiply | Blend::Screen)
    }

    /// The mode's function of straight colours (`B(Cs, Cd)` of the W3C compositing spec), for the
    /// modes a renderer works out itself.
    pub fn mix(self, s: f32, d: f32) -> f32 {
        let hard = |s: f32, d: f32| {
            if s <= 0.5 {
                2.0 * s * d
            } else {
                let t = 2.0 * s - 1.0;
                d + t - d * t
            }
        };
        match self {
            Blend::Normal => s,
            Blend::Add => s + d,
            Blend::Multiply => s * d,
            Blend::Screen => s + d - s * d,
            Blend::Overlay => hard(d, s),
            Blend::HardLight => hard(s, d),
            Blend::Darken => s.min(d),
            Blend::Lighten => s.max(d),
            Blend::Difference => (s - d).abs(),
            Blend::SoftLight => {
                if s <= 0.5 {
                    d - (1.0 - 2.0 * s) * d * (1.0 - d)
                } else {
                    let g = if d <= 0.25 { ((16.0 * d - 12.0) * d + 4.0) * d } else { d.sqrt() };
                    d + (2.0 * s - 1.0) * (g - d)
                }
            }
            Blend::ColorDodge => {
                if d <= 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (d / (1.0 - s)).min(1.0)
                }
            }
            Blend::ColorBurn => {
                if d >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - d) / s).min(1.0)
                }
            }
        }
    }
}

/// An effect applied to a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// `<name>@<version>`, such as `color.brightness@1`.
    pub id: String,
    /// Parameters by name.
    #[serde(default)]
    pub params: BTreeMap<String, Animated<f32>>,
    /// An asset the effect reads: the `lut` of `color.lut@1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
}

/// Effect ids, with their parameters.
///
/// - `color.brightness@1`: `amount` added to each channel, -1 to 1.
/// - `color.saturation@1`: `amount` times the colourfulness; 1 leaves it.
/// - `color.grade@1`: a colour grade. Every parameter is 0 when it changes nothing: `exposure`
///   (stops), `contrast` (-1 to 1, around mid grey), `saturation` (-1 grey to 1 twice as rich),
///   `temperature` (-1 cool to 1 warm), `tint` (-1 green to 1 magenta), `highlights` and
///   `shadows` (-1 to 1, darken or lift the bright or dark parts), `fade` (0 to 1, lifts the
///   blacks) and `vignette` (0 to 1, darkens towards the corners of the layer's picture).
/// - `color.lut@1`: a 3-D lookup table (`asset`, a `lut` in the `.cube` format, applied to
///   sRGB-encoded colour), mixed in by `amount` (0 to 1, default 1). One per layer.
/// - `blur.gaussian@1`: blurs a media layer's picture before it is placed; `radius` is the blur's
///   standard deviation as a fraction of the picture's shorter side (default 0.02). Text and colour
///   layers are left as they are. Editors use it for a blurred copy of a clip behind it (a
///   "canvas blur" background).
/// - `key.chroma@1`: makes one colour see-through, as for a green screen. `r`, `g`, `b` are the
///   key colour, sRGB-encoded (default green, 0 1 0). A pixel's distance from it is measured on
///   the chroma plane (BT.601 Cb and Cr of the encoded colour, so light and shade count little):
///   up to `tolerance` (0 to 1, default 0.3) it is gone, from there over `softness` (0 to 1,
///   default 0.1) it comes back, and `spill` (0 to 1, default 0.5) takes the key's tint off what
///   stays near it: that much of its chroma towards the key, its lightness kept.
/// - `key.person@1`: keeps the people in a media or composition layer's picture and makes the
///   rest see-through, as found by a segmentation model looking at the picture (it has no
///   parameters). It is done to the picture before it is placed and before any other effect.
/// - The `distort.*` effects change where a layer's picture is read, in this order whatever
///   their order in the list, each once per layer. Positions are fractions of the layer's visible
///   picture and are held at its edges.
///   - `distort.wave@1`: rows slide sideways along a sine: `amount` (of the width), `waves` down
///     the picture (default 4), `phase` in turns.
///   - `distort.glitch@1`: bands slide sideways and wrap around: up to `amount` of the width, in
///     a pattern picked by the whole part of `seed`.
///   - `distort.mosaic@1`: square blocks of one colour, `size` of the shorter side each.
///   - `distort.split@1`: red is read `x`, `y` away and blue as far the other way.
pub mod effects {
    /// A colour made see-through.
    pub const CHROMA_KEY: &str = "key.chroma@1";
    /// Everything but the people made see-through.
    pub const PERSON_KEY: &str = "key.person@1";
    /// Rows sliding along a sine.
    pub const WAVE: &str = "distort.wave@1";
    /// Bands sliding sideways.
    pub const GLITCH: &str = "distort.glitch@1";
    /// Blocks of one colour.
    pub const MOSAIC: &str = "distort.mosaic@1";
    /// Red and blue read apart.
    pub const SPLIT: &str = "distort.split@1";
    /// The `distort.*` effects.
    pub const DISTORT: [&str; 4] = [WAVE, GLITCH, MOSAIC, SPLIT];
    /// A blur of the layer's picture.
    pub const BLUR: &str = "blur.gaussian@1";
    /// Brightness.
    pub const BRIGHTNESS: &str = "color.brightness@1";
    /// Saturation.
    pub const SATURATION: &str = "color.saturation@1";
    /// A colour grade.
    pub const GRADE: &str = "color.grade@1";
    /// The parameters of a grade, in order.
    pub const GRADE_PARAMS: [&str; 9] =
        ["exposure", "contrast", "saturation", "temperature", "tint", "highlights", "shadows", "fade", "vignette"];
    /// A lookup table.
    pub const LUT: &str = "color.lut@1";
}

/// Sound from an asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    /// Id, unique among audio clips.
    pub id: String,
    /// A `video` or `audio` asset. Empty when the clip plays a composition.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asset: String,
    /// The composition whose mix it plays instead of an asset (feature `comp/1`), from
    /// `time.source_in` at speed 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp: Option<String>,
    /// When it starts on the output timeline.
    pub at: Tick,
    /// How long it plays; positive.
    pub dur: Tick,
    /// Which source instant plays when.
    #[serde(default)]
    pub time: TimeMap,
    /// Gain in dB; keyframe times are relative to the clip's start.
    #[serde(default = "zero")]
    pub gain_db: Animated<f32>,
    /// At a speed other than 1, stretch the sound in time keeping its pitch (feature
    /// `audio.stretch/1`) instead of resampling it, which moves the pitch with the speed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_pitch: bool,
    /// Semitones the sound is moved up (negative: down) without changing how long it lasts
    /// (feature `audio.pitch/1`). Sound played backwards ignores it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pitch: f32,
    /// What is done to the sound, in order, before its gain (feature `audio.fx/1`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx: Vec<AudioEffect>,
}

/// One thing done to a clip's sound. Each is a function of the clip's own samples, counted from
/// its start (what came before the clip is silence), so a mix made in pieces matches one made
/// whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioEffect {
    /// Steady background noise taken out. The sound is cut into frames of 1024 samples half a
    /// frame apart; in each, what does not rise above the noise is turned down.
    Denoise {
        /// The noise: its power density in dB (full scale squared per hertz) in each of
        /// [`AudioEffect::NOISE_BANDS`] bands evenly spaced on a log scale from 60 Hz to 16 kHz,
        /// the first and last taking what lies beyond.
        floor: Vec<f32>,
        /// How much is taken out, from 0 (nothing) to 1.
        amount: f32,
    },
    /// Only what lies between two frequencies is kept, as through a telephone.
    Band {
        /// The lowest frequency kept, in Hz.
        low: f32,
        /// The highest frequency kept, in Hz.
        high: f32,
    },
    /// The sound multiplied by a sine wave (ring modulation): a robot's voice.
    Ring {
        /// The sine's frequency in Hz.
        hz: f32,
        /// How much of the result is the modulated sound, from 0 to 1.
        mix: f32,
    },
    /// The sound again, later and quieter, a number of times.
    Echo {
        /// The time between repeats.
        delay: Tick,
        /// How loud each repeat is next to the one before, from 0 to 1.
        decay: f32,
        /// How many repeats, from 1 to 8.
        repeats: u32,
    },
}

impl AudioEffect {
    /// How many bands a noise profile has.
    pub const NOISE_BANDS: usize = 24;
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A value that is either constant or keyframed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Animated<T> {
    /// The same value throughout.
    Constant(T),
    /// Keyframes, sorted by time; before the first and after the last the value holds.
    Keys(Vec<Key<T>>),
}

/// One keyframe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Key<T> {
    /// Time relative to the owner's start.
    pub t: Tick,
    /// Value at that time.
    pub v: T,
    /// How the value moves from this key to the next.
    #[serde(default)]
    pub ease: Ease,
}

/// Interpolation from one key to the next.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    /// Straight line.
    #[default]
    Linear,
    /// Hold this value until the next key.
    Hold,
    /// Smoothstep: slow out of this key and into the next.
    InOut,
}

/// Values that can be interpolated.
pub trait Lerp: Copy {
    /// `a` at `f = 0`, `b` at `f = 1`.
    fn lerp(a: Self, b: Self, f: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(a: f32, b: f32, f: f32) -> f32 {
        a + (b - a) * f
    }
}

impl Lerp for [f32; 2] {
    fn lerp(a: [f32; 2], b: [f32; 2], f: f32) -> [f32; 2] {
        [f32::lerp(a[0], b[0], f), f32::lerp(a[1], b[1], f)]
    }
}

impl<T: Lerp> Animated<T> {
    /// The value at `t`, relative to the owner's start.
    pub fn sample(&self, t: Tick) -> T {
        match self {
            Animated::Constant(v) => *v,
            Animated::Keys(keys) => sample_keys(keys, t),
        }
    }
}

fn sample_keys<T: Lerp>(keys: &[Key<T>], t: Tick) -> T {
    let Some(first) = keys.first() else {
        panic!("validated specs never have an empty key list");
    };
    if t <= first.t {
        return first.v;
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if t < b.t {
            let f = ((t - a.t).0 as f64 / (b.t - a.t).0 as f64) as f32;
            return match a.ease {
                Ease::Hold => a.v,
                Ease::Linear => T::lerp(a.v, b.v, f),
                Ease::InOut => T::lerp(a.v, b.v, f * f * (3.0 - 2.0 * f)),
            };
        }
    }
    keys.last().map(|k| k.v).unwrap_or(first.v)
}

impl RenderSpec {
    /// An empty spec with these output settings.
    pub fn new(output: Output) -> RenderSpec {
        RenderSpec {
            spec: SPEC_VERSION.to_string(),
            engine: EngineRequirements::default(),
            output,
            assets: BTreeMap::new(),
            layers: Vec::new(),
            audio: Vec::new(),
            comps: BTreeMap::new(),
        }
    }

    /// Every feature id this spec needs: its declared features plus each effect id used.
    pub fn required_features(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.engine.features.iter().cloned().collect();
        if !self.comps.is_empty() {
            out.insert("comp/1".to_string());
        }
        if self.assets.values().any(|a| a.kind == AssetKind::Lottie) {
            out.insert("lottie/1".to_string());
        }
        for layer in self.layers.iter().chain(self.comps.values().flat_map(|c| &c.layers)) {
            if let LayerContent::Text(t) = &layer.content {
                out.insert("text/1".to_string());
                if t.needs_v2() {
                    out.insert("text/2".to_string());
                }
                if t.needs_v3() {
                    out.insert("text/3".to_string());
                }
                if t.needs_v4() {
                    out.insert("text/4".to_string());
                }
                if t.needs_v5() {
                    out.insert("text/5".to_string());
                }
                if t.needs_v6() {
                    out.insert("text/6".to_string());
                }
            }
            if let LayerContent::Shape(_) = &layer.content {
                out.insert("shape/1".to_string());
            }
            if let LayerContent::Media { time, between, .. } = &layer.content {
                out.extend(time.features().map(str::to_string));
                out.extend(between.feature().map(str::to_string));
            }
            if layer.crop.is_some() {
                out.insert("crop/1".to_string());
            }
            if layer.transform.stretch.is_some() {
                out.insert("transform.stretch/1".to_string());
            }
            if layer.transform.anchor.is_some() {
                out.insert("transform.anchor/1".to_string());
            }
            if layer.transform.turn.is_some() {
                out.insert("transform.3d/1".to_string());
            }
            if layer.parent.is_some() {
                out.insert("layer.parent/1".to_string());
            }
            if layer.flip_x || layer.flip_y {
                out.insert("flip/1".to_string());
            }
            if layer.shape_mask.is_some() {
                out.insert("mask.shape/1".to_string());
            }
            if layer.crop_keys.is_some() {
                out.insert("crop/2".to_string());
            }
            if layer.shape_mask_keys.is_some() {
                out.insert("mask.shape/2".to_string());
            }
            if layer.blend.is_v2() {
                out.insert("blend/2".to_string());
            }
            if layer.mask.is_some() {
                out.insert("mask/1".to_string());
            }
            for fx in &layer.fx {
                out.insert(fx.id.clone());
            }
        }
        if self.output.video.as_ref().is_some_and(|v| v.size.is_some()) {
            out.insert("encode.size/1".to_string());
        }
        match self.output.format {
            OutputFormat::Mp4 => {}
            OutputFormat::Gif => {
                out.insert("encode.gif/1".to_string());
            }
            OutputFormat::M4a | OutputFormat::Wav => {
                out.insert("encode.audio/1".to_string());
            }
        }
        for a in self.audio.iter().chain(self.comps.values().flat_map(|c| &c.audio)) {
            out.extend(a.time.features().map(str::to_string));
            if a.keep_pitch && (a.time.speed != 1.0 || !a.time.ramp.is_empty()) {
                out.insert("audio.stretch/1".to_string());
            }
            if a.pitch != 0.0 {
                out.insert("audio.pitch/1".to_string());
            }
            if !a.fx.is_empty() {
                out.insert("audio.fx/1".to_string());
            }
        }
        out
    }

    /// Parses a spec from JSON and validates it.
    pub fn from_json(json: &str) -> Result<RenderSpec, ValidationError> {
        let spec: RenderSpec = serde_json::from_str(json).map_err(|e| ValidationError::Parse(e.to_string()))?;
        validate(&spec)?;
        Ok(spec)
    }

    /// The spec as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a RenderSpec always serialises")
    }
}

/// The JSON Schema of [`RenderSpec`], as checked in under `schema/`.
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(RenderSpec)).expect("schema serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_interpolate_and_hold() {
        let a = Animated::Keys(vec![
            Key { t: Tick(0), v: 0.0f32, ease: Ease::Linear },
            Key { t: Tick(100), v: 1.0, ease: Ease::Hold },
            Key { t: Tick(200), v: 3.0, ease: Ease::Linear },
        ]);
        assert_eq!(a.sample(Tick(-5)), 0.0);
        assert_eq!(a.sample(Tick(50)), 0.5);
        assert_eq!(a.sample(Tick(150)), 1.0);
        assert_eq!(a.sample(Tick(250)), 3.0);
    }

    #[test]
    fn ramps_integrate_and_constant_speeds_scale() {
        assert_eq!(TimeMap::new(Tick(10), 2.0).source_at(Tick(5)), Tick(20));
        assert_eq!(TimeMap::new(Tick(100), -1.0).source_at(Tick(30)), Tick(70));
        assert_eq!(TimeMap::new(Tick(100), 0.0).source_at(Tick(30)), Tick(100));
        // 1 rising to 3 over 100 ticks, then held: 200 ticks of source, then 3 a tick
        let m = TimeMap {
            source_in: Tick(0),
            speed: 1.0,
            ramp: vec![SpeedKey { t: Tick(0), speed: 1.0 }, SpeedKey { t: Tick(100), speed: 3.0 }],
        };
        assert_eq!(m.source_at(Tick(50)), Tick(50 + 25));
        assert_eq!(m.source_at(Tick(100)), Tick(200));
        assert_eq!(m.source_at(Tick(110)), Tick(230));
        assert_eq!(m.features().collect::<Vec<_>>(), ["time.ramp/1"]);
        assert_eq!(TimeMap::new(Tick(0), -2.0).features().collect::<Vec<_>>(), ["time.reverse/1"]);
    }

    #[test]
    fn constant_and_keys_parse_from_the_same_field() {
        let c: Animated<[f32; 2]> = serde_json::from_str("[0.1, 0.2]").unwrap();
        assert_eq!(c, Animated::Constant([0.1, 0.2]));
        let k: Animated<[f32; 2]> = serde_json::from_str(r#"[{"t": 0, "v": [0, 0]}]"#).unwrap();
        assert!(matches!(k, Animated::Keys(_)));
    }

    #[test]
    fn numbers_are_written_as_prices_counts_and_clocks() {
        let n = NumberText { value: Animated::Constant(0.0), decimals: 0, group: false, clock: false };
        assert_eq!(n.format(99.6), "100");
        assert_eq!(NumberText { decimals: 2, ..n.clone() }.format(129.9), "129.90");
        let grouped = NumberText { group: true, ..n.clone() };
        assert_eq!(
            [grouped.format(999.0), grouped.format(1000.0), grouped.format(1234567.0)],
            ["999", "1,000", "1,234,567"]
        );
        assert_eq!(NumberText { group: true, decimals: 1, ..n.clone() }.format(-12345.67), "-12,345.7");
        assert_eq!(n.format(-0.2), "0");
        let clock = NumberText { clock: true, ..n.clone() };
        assert_eq!(
            [clock.format(90.0), clock.format(5.4), clock.format(3725.0), clock.format(-3.0)],
            ["01:30", "00:05", "1:02:05", "00:00"]
        );
        assert_eq!(n.fill("¥{n} 起", 59.0), "¥59 起");
        assert_eq!(n.fill("whatever", 59.0), "59");
    }

    #[test]
    fn checked_in_schema_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/renderspec-1.schema.json");
        let generated = serde_json::to_string_pretty(&json_schema()).unwrap() + "\n";
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == generated,
            "schema/renderspec-1.schema.json is stale: run `UPDATE_SCHEMA=1 cargo test -p reelforge-renderspec`"
        );
    }
}
