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

/// Source time for a media layer: `source = source_in + (t - at) * speed`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeMap {
    /// Source instant shown at the layer's start.
    #[serde(default)]
    pub source_in: Tick,
    /// Playback speed; positive. Speed curves arrive with a later feature.
    #[serde(default = "unit_speed")]
    pub speed: f64,
}

fn unit_speed() -> f64 {
    1.0
}

impl Default for TimeMap {
    fn default() -> TimeMap {
        TimeMap { source_in: Tick::ZERO, speed: 1.0 }
    }
}

impl TimeMap {
    /// The source instant for an offset into the layer.
    pub fn source_at(&self, offset: Tick) -> Tick {
        self.source_in + offset.scale(self.speed)
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
            if layer.mask.is_some() {
                out.insert("mask/1".to_string());
            }
            for fx in &layer.fx {
                out.insert(fx.id.clone());
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
