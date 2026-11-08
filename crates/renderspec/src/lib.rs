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
    /// Target bitrate in kbit/s, when no `crf` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
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
    },
    /// A flat colour filling the frame.
    Solid {
        /// The colour.
        color: Color,
    },
    /// Set text (feature `text/1`).
    Text(TextContent),
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
}

fn zero() -> Animated<f32> {
    Animated::Constant(0.0)
}

fn zero2() -> Animated<[f32; 2]> {
    Animated::Constant([0.0, 0.0])
}

impl Default for Transform {
    fn default() -> Transform {
        Transform { position: zero2(), scale: one(), rotation: zero() }
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
pub mod effects {
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
    /// A `video` or `audio` asset.
    pub asset: String,
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
        }
    }

    /// Every feature id this spec needs: its declared features plus each effect id used.
    pub fn required_features(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.engine.features.iter().cloned().collect();
        for layer in &self.layers {
            if let LayerContent::Text(t) = &layer.content {
                out.insert("text/1".to_string());
                if t.needs_v2() {
                    out.insert("text/2".to_string());
                }
            }
            if let LayerContent::Media { time, .. } = &layer.content {
                out.extend(time.features().map(str::to_string));
            }
            if layer.crop.is_some() {
                out.insert("crop/1".to_string());
            }
            if layer.flip_x || layer.flip_y {
                out.insert("flip/1".to_string());
            }
            if layer.shape_mask.is_some() {
                out.insert("mask.shape/1".to_string());
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
        for a in &self.audio {
            out.extend(a.time.features().map(str::to_string));
            if a.keep_pitch && (a.time.speed != 1.0 || !a.time.ramp.is_empty()) {
                out.insert("audio.stretch/1".to_string());
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
