//! Drawing a RenderSpec.
//!
//! Rendering splits in two, and the split is the point of this crate:
//!
//! 1. [`plan::plan_frame`] answers "what is on screen at this instant, from where, how placed and
//!    how strongly". It touches no pixels and does no IO, so it is exact, fast and the same for
//!    every backend.
//! 2. A compositor draws a [`plan::FramePlan`] and nothing else. [`cpu`] is the reference
//!    implementation every other backend is tested against; `gpu` (feature `gpu`) draws the same
//!    plan with WGSL shaders, on the editor's wgpu device for previews or a headless one for
//!    exports. Both start from what [`cpu::resolve`] fetched and drew (pictures, text sprites), so
//!    they agree on placeholders and errors.
//!
//! This crate depends on `reelforge-renderspec` and `reelforge-time` only (`cargo xtask layers`
//! enforces it), so the same code renders inside the editor, in the browser and on a render
//! service.
//!
//! Pictures come from a [`MediaSource`]. A source that cannot produce a picture yet returns
//! `Ok(None)`; the frame then shows the offline slate where that layer would be, and the
//! [`FrameReport`] says so, so a draft preview keeps working while a final render can refuse.

pub mod cpu;
pub mod distort;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod image;
pub mod lut;
pub mod media;
pub mod plan;
pub mod text;
pub mod yuv;

#[cfg(test)]
mod text_tests;

use reelforge_renderspec::RenderSpec;
use reelforge_time::Tick;
use thiserror::Error;

pub use crate::image::Image;
pub use lut::Lut;
pub use media::{MediaSource, NoMedia, StillImages};
pub use plan::{FramePlan, PlannedLayer, PlannedSource, plan_frame};
pub use yuv::Yuv;

/// Feature ids this renderer implements; see `RenderSpec::required_features`. (The `audio.*` ones are
/// `reelforge-media`'s mix, which every reelforge renderer of sound uses.)
pub const SUPPORTED_FEATURES: &[&str] = &[
    "audio.fx/1",
    "audio.pitch/1",
    "audio.stretch/1",
    "blend/2",
    "blur.gaussian@1",
    "color.brightness@1",
    "color.grade@1",
    "color.lut@1",
    "color.saturation@1",
    "crop/1",
    "crop/2",
    "distort.glitch@1",
    "distort.mosaic@1",
    "distort.split@1",
    "distort.wave@1",
    "encode.size/1",
    "flip/1",
    "key.chroma@1",
    "mask.shape/1",
    "mask.shape/2",
    "text/1",
    "text/2",
    "text/3",
    "text/4",
    "time.hold/1",
    "time.ramp/1",
    "time.reverse/1",
];

/// Why a frame could not be rendered.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderError {
    /// The spec uses features this renderer does not have.
    #[error("this renderer does not support: {}", .0.join(", "))]
    Unsupported(Vec<String>),
    /// A final render met a layer it could only draw as a placeholder.
    #[error("layer {layer:?} could not be drawn: {reason}")]
    Missing {
        /// The layer id.
        layer: String,
        /// Why.
        reason: String,
    },
    /// A media source failed.
    #[error("media error: {0}")]
    Media(String),
}

/// How strict a render is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Preview: unsupported or offline layers draw as placeholders and are reported.
    Draft,
    /// Export: anything that cannot be drawn exactly is an error.
    Final,
}

/// What a draft frame could not draw exactly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameReport {
    /// `(layer id, reason)` for every placeholder drawn.
    pub placeholders: Vec<(String, String)>,
}

/// Features `spec` needs that this renderer lacks.
pub fn unsupported_features(spec: &RenderSpec) -> Vec<String> {
    spec.required_features().into_iter().filter(|f| !SUPPORTED_FEATURES.contains(&f.as_str())).collect()
}

/// Renders one frame at `t` on the CPU, `scale` times the output size (1.0 for full size).
pub fn render_frame(
    spec: &RenderSpec,
    t: Tick,
    scale: f32,
    media: &dyn MediaSource,
    quality: Quality,
) -> Result<(Image, FrameReport), RenderError> {
    if quality == Quality::Final {
        let missing = unsupported_features(spec);
        if !missing.is_empty() {
            return Err(RenderError::Unsupported(missing));
        }
    }
    let plan = plan_frame(spec, t, scale);
    cpu::composite(spec, &plan, media, quality)
}

/// Where a visible layer lands: its box's corners in output pixels (`[x, y]`, clockwise from the
/// content's top-left), turned with the layer.
pub type Outline = [[f32; 2]; 4];

/// Where each layer visible at `t` lands, bottom first: `(layer id, outline)`. For editors that
/// draw a selection box and let people drag what they see. Text is measured as drawn (clipped to
/// the frame); a picture by its probe, or by fetching it.
pub fn layer_outlines(spec: &RenderSpec, t: Tick, media: &dyn MediaSource) -> Vec<(String, Outline)> {
    let plan = plan_frame(spec, t, 1.0);
    let target = [plan.width as f32, plan.height as f32];
    let corners = |m: plan::Affine, w: f32, h: f32| -> Outline {
        [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|[x, y]| [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]])
    };
    let mut out = Vec::new();
    for layer in &plan.layers {
        let outline = match &layer.source {
            PlannedSource::Solid(_) => {
                let mut l = layer.clone();
                l.fit = reelforge_renderspec::Fit::Stretch;
                corners(l.matrix(1.0, 1.0, target), 1.0, 1.0)
            }
            PlannedSource::Media { asset, .. } => {
                let probed =
                    spec.assets.get(asset).and_then(|a| a.probe.as_ref()).and_then(|p| Some((p.width?, p.height?)));
                let size = match probed {
                    Some(s) => s,
                    None => match spec.assets.get(asset).map(|def| media.picture(asset, def, plan.t)) {
                        Some(Ok(Some(pic))) => (pic.width, pic.height),
                        _ => (plan.width, plan.height),
                    },
                };
                let (w, h) = layer.visible(size.0 as f32, size.1 as f32);
                corners(layer.matrix(w, h, target), w, h)
            }
            PlannedSource::Text(_) => {
                // measured upright, then turned about the layer's centre
                let mut upright = layer.clone();
                upright.rotation = 0.0;
                let mut report = FrameReport::default();
                let Ok(cpu::Resolved::Sprite(sp)) =
                    cpu::resolve(spec, &plan, &upright, media, Quality::Draft, cpu::Takes::CPU, &mut report)
                else {
                    continue;
                };
                // the ink, not the layout box (a wrapping title's box is as wide as its wrap)
                let w = sp.image.width as usize;
                let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
                for (i, px) in sp.image.pixels.iter().enumerate() {
                    if px[3] > 0.0 {
                        let (x, y) = (i % w, i / w);
                        lo = [lo[0].min(x), lo[1].min(y)];
                        hi = [hi[0].max(x + 1), hi[1].max(y + 1)];
                    }
                }
                if lo[0] == usize::MAX {
                    continue;
                }
                let (x0, y0) = (sp.x as f32 + lo[0] as f32, sp.y as f32 + lo[1] as f32);
                let (x1, y1) = (sp.x as f32 + hi[0] as f32, sp.y as f32 + hi[1] as f32);
                let (sin, cos) = layer.rotation.sin_cos();
                let [cx, cy] = layer.center;
                [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
                    .map(|[x, y]| [cx + (x - cx) * cos - (y - cy) * sin, cy + (x - cx) * sin + (y - cy) * cos])
            }
        };
        out.push((layer.id.clone(), outline));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_renderspec::*;
    use reelforge_time::FrameRate;
    use std::sync::Arc;

    pub(crate) fn spec() -> RenderSpec {
        let mut s = RenderSpec::new(Output {
            width: 40,
            height: 20,
            fps: FrameRate::FPS_30,
            duration: Tick::SECOND,
            background: Color([0.0, 0.0, 1.0, 1.0]),
            video: None,
            audio: None,
        });
        s.layers.push(Layer {
            id: "half".into(),
            at: Tick::ZERO,
            dur: Tick::SECOND,
            content: LayerContent::Solid { color: Color([1.0, 0.0, 0.0, 1.0]) },
            transform: Transform {
                position: Animated::Constant([0.25, 0.0]),
                scale: Animated::Constant(0.5),
                rotation: Animated::Constant(0.0),
            },
            opacity: Animated::Constant(1.0),
            blend: Blend::Normal,
            mask: None,
            fx: vec![],
            crop: None,
            flip_x: false,
            flip_y: false,
            shape_mask: None,
            crop_keys: None,
            shape_mask_keys: None,
        });
        s
    }

    fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let b = img.to_srgba8();
        let i = ((y * img.width + x) * 4) as usize;
        [b[i], b[i + 1], b[i + 2], b[i + 3]]
    }

    /// Answers every picture with a 1×1 picture and remembers how much smaller it was asked for.
    struct Asked(std::sync::Mutex<Vec<u32>>);

    impl MediaSource for Asked {
        fn picture(&self, id: &str, a: &Asset, at: Tick) -> Result<Option<Arc<Image>>, String> {
            self.picture_shrunk(id, a, at, 1)
        }
        fn picture_shrunk(&self, _: &str, _: &Asset, _: Tick, shrink: u32) -> Result<Option<Arc<Image>>, String> {
            self.0.lock().unwrap().push(shrink);
            Ok(Some(Arc::new(Image::filled(1, 1, [1.0; 4]))))
        }
    }

    #[test]
    fn drafts_ask_for_pictures_as_small_as_they_are_drawn() {
        let mut s = spec();
        s.assets.insert(
            "v".into(),
            Asset {
                kind: AssetKind::Video,
                sha256: "3".repeat(64),
                uri: "file:///v.mp4".into(),
                of: None,
                probe: Some(Probe { width: Some(400), height: Some(200), duration: None }),
            },
        );
        s.layers[0].content = LayerContent::Media { asset: "v".into(), time: TimeMap::default(), fit: Fit::Contain };
        s.layers[0].transform.scale = Animated::Constant(1.0);
        let asked = Asked(Default::default());
        let ask = |scale: f32, quality: Quality| {
            asked.0.lock().unwrap().clear();
            render_frame(&s, Tick::ZERO, scale, &asked, quality).unwrap();
            asked.0.lock().unwrap()[0]
        };
        // 400 wide, drawn 40 wide: ten times smaller, so 8 (the most)
        assert_eq!(ask(1.0, Quality::Draft), 8);
        // drawn 400 wide: full size
        assert_eq!(ask(10.0, Quality::Draft), 1);
        // drawn 160 wide: 2 (4 would leave too few pixels)
        assert_eq!(ask(4.0, Quality::Draft), 2);
        // a final frame always has the whole picture
        assert_eq!(ask(1.0, Quality::Final), 1);
    }

    #[test]
    fn outlines_say_where_layers_land() {
        let mut s = spec();
        let outlines = layer_outlines(&s, Tick::ZERO, &NoMedia);
        assert_eq!(outlines, vec![("half".to_string(), [[20.0, 5.0], [40.0, 5.0], [40.0, 15.0], [20.0, 15.0]])]);
        // turned a quarter: the box turns about its centre
        s.layers[0].transform.rotation = Animated::Constant(90.0);
        let (_, o) = &layer_outlines(&s, Tick::ZERO, &NoMedia)[0];
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
        assert!(near(o[0], [35.0, 0.0]) && near(o[2], [25.0, 20.0]), "{o:?}");
        // past its end nothing is there
        assert!(layer_outlines(&s, Tick::SECOND, &NoMedia).is_empty());
    }

    #[test]
    fn a_scaled_solid_covers_its_box_only() {
        let (img, report) = render_frame(&spec(), Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert!(report.placeholders.is_empty());
        // The box is 20x10 centred at (30, 10).
        assert_eq!(px(&img, 30, 10), [255, 0, 0, 255]);
        assert_eq!(px(&img, 21, 6), [255, 0, 0, 255]);
        assert_eq!(px(&img, 5, 10), [0, 0, 255, 255]);
        assert_eq!(px(&img, 30, 2), [0, 0, 255, 255]);
    }

    #[test]
    fn masks_crops_flips_and_blends_do_what_they_say() {
        // a circle in the middle of the 20x10 box shows; its corners do not
        let mut s = spec();
        s.layers[0].shape_mask = Some(ShapeMask {
            shape: MaskShape::Circle,
            center: [0.5, 0.5],
            size: [0.8, 0.8],
            rotation: 0.0,
            feather: 0.0,
            invert: false,
        });
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert_eq!(px(&img, 30, 10), [255, 0, 0, 255]);
        assert_eq!(px(&img, 21, 6), [0, 0, 255, 255]);
        // inverted, the other way round
        s.layers[0].shape_mask.as_mut().unwrap().invert = true;
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert_eq!(px(&img, 30, 10), [0, 0, 255, 255]);
        assert_eq!(px(&img, 21, 6), [255, 0, 0, 255]);
        // difference of red over blue is magenta; darken of red over blue is black
        let mut s = spec();
        s.layers[0].blend = Blend::Difference;
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert_eq!(px(&img, 30, 10), [255, 0, 255, 255]);
        s.layers[0].blend = Blend::Darken;
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert_eq!(px(&img, 30, 10), [0, 0, 0, 255]);
        assert!(s.required_features().contains("blend/2"));

        // a picture, left half white and right half black, stretched over the frame
        struct Halves;
        impl MediaSource for Halves {
            fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
                let mut img = Image::filled(4, 2, [1.0; 4]);
                for y in 0..2 {
                    for x in 2..4 {
                        img.pixels[y * 4 + x] = [0.0, 0.0, 0.0, 1.0];
                    }
                }
                Ok(Some(Arc::new(img)))
            }
        }
        let mut s = spec();
        s.assets.insert(
            "p".into(),
            Asset { kind: AssetKind::Image, sha256: "3".repeat(64), uri: "p.png".into(), of: None, probe: None },
        );
        s.layers[0].content = LayerContent::Media { asset: "p".into(), time: TimeMap::default(), fit: Fit::Stretch };
        s.layers[0].transform = Transform::default();
        let at = |s: &RenderSpec, x| px(&render_frame(s, Tick::ZERO, 1.0, &Halves, Quality::Final).unwrap().0, x, 10);
        assert_eq!(at(&s, 8), [255, 255, 255, 255]);
        s.layers[0].flip_x = true;
        assert_eq!(at(&s, 8), [0, 0, 0, 255]);
        // cropped to its right half, black fills the frame
        s.layers[0].flip_x = false;
        s.layers[0].crop = Some(Crop { left: 0.5, ..Default::default() });
        assert_eq!(at(&s, 20), [0, 0, 0, 255]);
        assert!(s.required_features().contains("crop/1"));
    }

    #[test]
    fn half_opacity_mixes_in_linear_light() {
        let mut s = spec();
        s.layers[0].opacity = Animated::Constant(0.5);
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        // Linear 0.5 red + 0.5 blue is sRGB 188.
        assert_eq!(px(&img, 30, 10), [188, 0, 188, 255]);
    }

    #[test]
    fn layers_outside_their_span_are_not_drawn() {
        let mut s = spec();
        s.layers[0].at = Tick::SECOND;
        s.output.duration = Tick::SECOND * 2;
        let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap();
        assert_eq!(px(&img, 30, 10), [0, 0, 255, 255]);
    }

    #[test]
    fn final_renders_refuse_what_drafts_mark() {
        let mut s = spec();
        s.layers[0].fx.push(Effect { id: "blur.motion@1".into(), params: Default::default(), asset: None });
        assert!(matches!(
            render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final),
            Err(RenderError::Unsupported(_))
        ));
        let mut s = spec();
        s.assets.insert(
            "v".into(),
            Asset {
                kind: AssetKind::Video,
                sha256: "0".repeat(64),
                uri: "file:///gone.mp4".into(),
                of: None,
                probe: None,
            },
        );
        s.layers[0].content = LayerContent::Media { asset: "v".into(), time: TimeMap::default(), fit: Fit::Contain };
        assert!(matches!(
            render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final),
            Err(RenderError::Missing { .. })
        ));
        let (_, report) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).unwrap();
        assert_eq!(report.placeholders.len(), 1);
    }

    #[test]
    fn plan_is_scaled_for_previews() {
        let plan = plan_frame(&spec(), Tick::ZERO, 0.5);
        assert_eq!((plan.width, plan.height), (20, 10));
        assert_eq!(plan.layers[0].center, [15.0, 5.0]);
    }
}
