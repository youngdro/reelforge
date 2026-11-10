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
//! Of this workspace the crate depends on `reelforge-renderspec`, `reelforge-time` and
//! `reelforge-matte` (the model behind `key.person@1`) only, none of which knows a project
//! (`cargo xtask layers` enforces it), so the same code renders inside the editor, in the browser
//! and on a render service.
//!
//! Pictures come from a [`MediaSource`]. A source that cannot produce a picture yet returns
//! `Ok(None)`; the frame then shows the offline slate where that layer would be, and the
//! [`FrameReport`] says so, so a draft preview keeps working while a final render can refuse.

pub mod cpu;
pub mod cutout;
pub mod distort;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod image;
pub mod lut;
pub mod media;
pub mod plan;
pub mod shape;
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
    "comp/1",
    "crop/1",
    "crop/2",
    "distort.glitch@1",
    "distort.mosaic@1",
    "distort.split@1",
    "distort.wave@1",
    "encode.audio/1",
    "encode.gif/1",
    "encode.size/1",
    "flip/1",
    "frames.blend/1",
    "frames.flow/1",
    "key.chroma@1",
    "key.person@1",
    "layer.parent/1",
    "lottie/1",
    "mask.shape/1",
    "mask.shape/2",
    "motion.blur/1",
    "shape/1",
    "text/1",
    "text/2",
    "text/3",
    "text/4",
    "text/5",
    "text/6",
    "text/7",
    "time.hold/1",
    "time.ramp/1",
    "time.reverse/1",
    "transform.3d/1",
    "transform.anchor/1",
    "transform.stretch/1",
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

/// The size, in output pixels, of the box a title's anchor is a point of, in the default face: as
/// tall as its lines together and as wide as its longest line reaches to either side of the
/// middle of its block, which is what the layer places.
pub fn text_block(text: &reelforge_renderspec::TextContent) -> [f32; 2] {
    text::block(text, text::DEFAULT_FAMILY)
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
    // a corner past the eye has no place on the frame: it is put where it would be seen flat
    let at = |m: &plan::Map, [x, y]: [f32; 2]| {
        plan::apply(m, x, y).unwrap_or([m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]])
    };
    let corners =
        |m: plan::Map, w: f32, h: f32| -> Outline { [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| at(&m, p)) };
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
            PlannedSource::Comp(_) => {
                // a composition's picture is a frame
                let (w, h) = layer.visible(target[0], target[1]);
                corners(layer.matrix(w, h, target), w, h)
            }
            PlannedSource::Shape(sh) if !layer.plain() => {
                // its box, counted from the anchor, then placed
                let k = plan.width as f32 / spec.output.width as f32 * layer.scale;
                let (w, h) = (sh.size[0] * k, sh.size[1] * k);
                let from = [1.0, 0.0, -layer.anchor[0] * w, 0.0, 1.0, -layer.anchor[1] * h, 0.0, 0.0, 1.0];
                corners(plan::then(&layer.place(), &from), w, h)
            }
            PlannedSource::Shape(sh) => {
                // its box, scaled and turned about the layer's centre
                let k = plan.width as f32 / spec.output.width as f32 * layer.scale;
                let (hw, hh) = (sh.size[0] * k / 2.0, sh.size[1] * k / 2.0);
                let (sin, cos) = layer.rotation.sin_cos();
                let [cx, cy] = layer.center;
                [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                    .map(|[x, y]| [cx + x * cos - y * sin, cy + x * sin + y * cos])
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
                if let Some(m) = &sp.map {
                    // drawn flat: the ink's box in the sprite, placed with it
                    let (lo, hi) = (lo.map(|v| v as f32), hi.map(|v| v as f32));
                    out.push((
                        layer.id.clone(),
                        [[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]].map(|p| at(m, p)),
                    ));
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
            format: Default::default(),
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
                ..Transform::default()
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
            parent: None,
            motion_blur: None,
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
        s.layers[0].content = LayerContent::Media {
            asset: "v".into(),
            time: TimeMap::default(),
            fit: Fit::Contain,
            between: Default::default(),
        };
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

    /// The portrait `reelforge-matte` is tested with: a person in front of a studio backdrop.
    pub(crate) fn portrait() -> Arc<Image> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../matte/tests/astronaut.png");
        let img = ::image::open(path).expect("the portrait").to_rgba8();
        Arc::new(Image::from_srgba8(img.width(), img.height(), img.as_raw()))
    }

    #[test]
    fn the_people_in_a_layer_stay_and_what_is_behind_them_shows_through() {
        struct Portrait(Arc<Image>);
        impl MediaSource for Portrait {
            fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
                Ok(Some(self.0.clone()))
            }
        }
        let mut s = spec();
        s.assets.insert(
            "p".into(),
            Asset {
                kind: AssetKind::Image,
                sha256: "4".repeat(64),
                uri: "file:///p.png".into(),
                of: None,
                probe: None,
            },
        );
        s.layers[0].content = LayerContent::Media {
            asset: "p".into(),
            time: TimeMap::default(),
            fit: Fit::Stretch,
            between: Default::default(),
        };
        s.layers[0].transform = Transform::default();
        let media = Portrait(portrait());
        // as it is: the studio backdrop covers the frame's blue
        let (whole, _) = render_frame(&s, Tick::ZERO, 4.0, &media, Quality::Final).unwrap();
        let (face, beside) = ((80, 19), (12, 9));
        assert_ne!(px(&whole, beside.0, beside.1), [0, 0, 255, 255]);
        // cut out: the face is as it was, and beside the head the frame's own blue shows
        s.layers[0].fx = vec![Effect { id: effects::PERSON_KEY.into(), params: Default::default(), asset: None }];
        assert!(s.required_features().contains("key.person@1") && SUPPORTED_FEATURES.contains(&"key.person@1"));
        let (cut, _) = render_frame(&s, Tick::ZERO, 4.0, &media, Quality::Final).unwrap();
        assert_eq!(px(&cut, face.0, face.1), px(&whole, face.0, face.1));
        assert_eq!(px(&cut, beside.0, beside.1), [0, 0, 255, 255]);
        // before a blur, which then softens the outline it made
        s.layers[0].fx.push(Effect {
            id: effects::BLUR.into(),
            params: [("radius".to_string(), Animated::Constant(0.05))].into(),
            asset: None,
        });
        let (soft, _) = render_frame(&s, Tick::ZERO, 4.0, &media, Quality::Final).unwrap();
        assert_eq!(px(&soft, beside.0, beside.1), [0, 0, 255, 255]);
        assert_ne!(px(&soft, face.0, face.1), px(&whole, face.0, face.1));
    }

    #[test]
    fn a_layer_that_wants_pictures_between_frames_asks_for_them_at_its_instant() {
        use reelforge_renderspec::Between;
        /// Answers a white picture for one made between frames and a black one otherwise, and
        /// remembers what was asked.
        struct Made(std::sync::Mutex<Vec<(Tick, Between, u32)>>);
        impl MediaSource for Made {
            fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
                Ok(Some(Arc::new(Image::filled(4, 2, [0.0, 0.0, 0.0, 1.0]))))
            }
            fn picture_between(
                &self,
                _: &str,
                _: &Asset,
                at: Tick,
                how: Between,
                shrink: u32,
            ) -> Result<Option<Arc<Image>>, String> {
                self.0.lock().unwrap().push((at, how, shrink));
                Ok(Some(Arc::new(Image::filled(4, 2, [1.0; 4]))))
            }
        }
        let mut s = spec();
        s.assets.insert(
            "v".into(),
            Asset {
                kind: AssetKind::Video,
                sha256: "3".repeat(64),
                uri: "file:///v.mp4".into(),
                of: None,
                probe: None,
            },
        );
        let quarter = TimeMap::new(Tick::SECOND, 0.25);
        s.layers[0].content =
            LayerContent::Media { asset: "v".into(), time: quarter.clone(), fit: Fit::Stretch, between: Between::Flow };
        s.layers[0].transform = Transform::default();
        assert!(s.required_features().contains("frames.flow/1") && SUPPORTED_FEATURES.contains(&"frames.flow/1"));
        let made = Made(Default::default());
        let t = Tick(Tick::SECOND.0 / 10);
        let (img, _) = render_frame(&s, t, 1.0, &made, Quality::Final).unwrap();
        assert_eq!(px(&img, 8, 10), [255, 255, 255, 255]);
        // the source instant the layer's time gives, a fortieth of a second in: between two frames
        assert_eq!(*made.0.lock().unwrap(), [(Tick::SECOND + Tick(t.0 / 4), Between::Flow, 1)]);
        // frames as they are: the picture of that instant, as before
        s.layers[0].content =
            LayerContent::Media { asset: "v".into(), time: quarter, fit: Fit::Stretch, between: Between::Nearest };
        assert!(!s.required_features().iter().any(|f| f.starts_with("frames.")));
        let (img, _) = render_frame(&s, t, 1.0, &made, Quality::Final).unwrap();
        assert_eq!(px(&img, 8, 10), [0, 0, 0, 255]);
        assert_eq!(made.0.lock().unwrap().len(), 1);
        // a source that makes none shows the frame of that instant
        let asked = Asked(Default::default());
        let between = asked.picture_between("v", &s.assets["v"], t, Between::Blend, 2).unwrap();
        assert!(between.is_some() && *asked.0.lock().unwrap() == [2]);
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

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn drawn(s: &RenderSpec) -> Image {
        render_frame(s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap().0
    }

    #[test]
    fn a_layer_is_stretched_one_way_and_placed_by_its_anchor() {
        // the box is 20 by 10 in the middle of the frame: columns 10 to 30, rows 5 to 15
        let mut s = spec();
        s.layers[0].transform.position = Animated::Constant([0.0, 0.0]);
        // half as wide, as tall as it was
        s.layers[0].transform.stretch = Some(Animated::Constant([0.5, 1.0]));
        let img = drawn(&s);
        assert_eq!([px(&img, 13, 10), px(&img, 16, 10), px(&img, 24, 10), px(&img, 26, 10)], [BLUE, RED, RED, BLUE]);
        assert_eq!([px(&img, 20, 4), px(&img, 20, 6), px(&img, 20, 14)], [BLUE, RED, RED]);
        assert_eq!(
            layer_outlines(&s, Tick::ZERO, &NoMedia)[0].1,
            [[15.0, 5.0], [25.0, 5.0], [25.0, 15.0], [15.0, 15.0]]
        );
        // its top left corner where its middle was: it reaches right and down from there
        s.layers[0].transform.stretch = None;
        s.layers[0].transform.anchor = Some([0.0, 0.0]);
        let img = drawn(&s);
        assert_eq!([px(&img, 18, 12), px(&img, 21, 12), px(&img, 38, 18), px(&img, 21, 8)], [BLUE, RED, RED, BLUE]);
        // and it turns about that corner: a quarter turn swings it down and to the left
        s.layers[0].transform.rotation = Animated::Constant(90.0);
        let img = drawn(&s);
        assert_eq!([px(&img, 12, 12), px(&img, 18, 18), px(&img, 21, 12), px(&img, 8, 12)], [RED, RED, BLUE, BLUE]);
        // a spec that asks for these says so
        let needs = s.required_features();
        assert!(needs.contains("transform.anchor/1") && !needs.contains("transform.stretch/1"));
        assert!(unsupported_features(&s).is_empty());
    }

    #[test]
    fn a_layer_turned_in_space_is_seen_in_perspective() {
        // 20 by 10 in the middle of the frame, the eye two frame heights (40 pixels) away
        let mut s = spec();
        s.layers[0].transform.position = Animated::Constant([0.0, 0.0]);
        // swung 60 degrees about the vertical: its right edge goes away and is smaller, its left
        // comes nearer and is larger; its middle stays where it was
        s.layers[0].transform.turn = Some(Turn::held(0.0, 60.0));
        let img = drawn(&s);
        assert_eq!([px(&img, 12, 10), px(&img, 14, 10), px(&img, 23, 10), px(&img, 25, 10)], [BLUE, RED, RED, BLUE]);
        // the near edge is taller than the box was, the far edge shorter
        assert_eq!([px(&img, 14, 4), px(&img, 14, 15), px(&img, 23, 5), px(&img, 23, 7)], [RED, RED, BLUE, RED]);
        let o = layer_outlines(&s, Tick::ZERO, &NoMedia)[0].1;
        assert!(o[3][1] - o[0][1] > 12.0 && o[2][1] - o[1][1] < 9.0, "{o:?}");
        // the other way round
        s.layers[0].transform.turn = Some(Turn::held(0.0, -60.0));
        let img = drawn(&s);
        assert_eq!([px(&img, 25, 4), px(&img, 16, 5), px(&img, 16, 7)], [RED, BLUE, RED]);
        // tipped about the horizontal: the top edge goes away
        s.layers[0].transform.turn = Some(Turn::held(60.0, 0.0));
        let img = drawn(&s);
        assert_eq!([px(&img, 10, 8), px(&img, 12, 8), px(&img, 9, 12), px(&img, 20, 6)], [BLUE, RED, RED, BLUE]);
        // edge on, nothing of it is seen
        s.layers[0].transform.turn = Some(Turn::held(0.0, 90.0));
        assert!(drawn(&s).to_srgba8().as_chunks::<4>().0.iter().all(|p| *p == BLUE));
        // from farther away the perspective is weaker: nearly a flat squeeze to half the width
        s.layers[0].transform.turn = Some(Turn { distance: 50.0, ..Turn::held(0.0, 60.0) });
        let img = drawn(&s);
        assert_eq!([px(&img, 14, 10), px(&img, 15, 6), px(&img, 24, 6), px(&img, 25, 10)], [BLUE, RED, RED, BLUE]);
        assert!(s.required_features().contains("transform.3d/1") && unsupported_features(&s).is_empty());
    }

    #[test]
    fn a_layer_on_the_move_is_smeared_along_its_way_and_sharp_at_rest() {
        // 20 by 10, going 20 pixels to the right over two frames and then standing still
        let mut s = spec();
        let frame = FrameRate::FPS_30.frame_duration();
        s.layers[0].transform.position = Animated::Keys(vec![
            Key { t: Tick::ZERO, v: [-0.25, 0.0], ease: Ease::Linear },
            Key { t: frame * 2, v: [0.25, 0.0], ease: Ease::Linear },
        ]);
        let at = |s: &RenderSpec, f: i64| render_frame(s, frame * f, 1.0, &NoMedia, Quality::Final).unwrap().0;
        // how red each column of the middle row is, in linear light
        let row = |img: &Image| -> Vec<f32> { (0..40).map(|x| img.pixels[(10 * 40 + x) as usize][0]).collect() };
        // without a shutter it is sharp half way: columns 10 to 30
        let sharp = row(&at(&s, 1));
        assert_eq!([sharp[9], sharp[10], sharp[29], sharp[30]], [0.0, 1.0, 1.0, 0.0]);
        // the shutter open the whole frame: it goes 10 pixels meanwhile, so its edges are ramps
        // 10 wide about where they were, and no more light comes from it than it gives
        s.layers[0].motion_blur = Some(MotionBlur { shutter: 1.0 });
        reelforge_renderspec::validate(&s).expect("valid");
        assert!(s.required_features().contains("motion.blur/1") && unsupported_features(&s).is_empty());
        let img = at(&s, 1);
        let soft = row(&img);
        assert!(soft[3] == 0.0 && soft[36] == 0.0 && soft[16] == 1.0 && soft[23] == 1.0, "{soft:?}");
        assert!(soft[6] > 0.0 && soft[6] < soft[10] && soft[10] < soft[13] && soft[13] <= 1.0, "{soft:?}");
        assert!((soft[10] - 0.5).abs() < 0.15 && (soft[29] - 0.5).abs() < 0.15, "{soft:?}");
        assert!((soft.iter().sum::<f32>() - 20.0).abs() < 0.5, "{}", soft.iter().sum::<f32>());
        // the frame under it shows through where it is thin
        assert!(img.pixels[10 * 40 + 6][2] > 0.5 && img.pixels[10 * 40 + 6][3] == 1.0);
        // up and down it does not move: its top and bottom edges are as sharp as ever
        assert_eq!([px(&img, 20, 4), px(&img, 20, 5), px(&img, 20, 14), px(&img, 20, 15)], [BLUE, RED, RED, BLUE]);
        // half the shutter, half the smear
        s.layers[0].motion_blur = Some(MotionBlur { shutter: 0.5 });
        let half = row(&at(&s, 1));
        assert!(half[6] == 0.0 && half[8] > 0.0 && half[13] == 1.0, "{half:?}");
        // at rest it is drawn once, as sharp as without: the plan has no other instant for it
        assert_eq!(
            row(&at(&s, 5)),
            row(&at(&RenderSpec { layers: vec![Layer { motion_blur: None, ..s.layers[0].clone() }], ..s.clone() }, 5))
        );
        assert!(plan::plan_frame(&s, frame * 5, 1.0).layers[0].smear.is_empty());
        assert!(plan::plan_frame(&s, frame, 1.0).layers[0].smear.len() >= 2);
        // growing smears too: it is larger at the end of the shutter's time than at its start
        s.layers[0].transform.position = Animated::Constant([0.0, 0.0]);
        s.layers[0].transform.scale = Animated::Keys(vec![
            Key { t: Tick::ZERO, v: 0.2, ease: Ease::Linear },
            Key { t: frame * 2, v: 1.0, ease: Ease::Linear },
        ]);
        s.layers[0].motion_blur = Some(MotionBlur { shutter: 1.0 });
        let grown = row(&at(&s, 1));
        assert!(
            grown[20] == 1.0 && grown[4] == 0.0 && grown[5] > 0.0 && grown[5] < grown[9] && grown[9] < grown[12],
            "{grown:?}"
        );
    }

    #[test]
    fn a_layer_is_carried_by_its_parent() {
        let mut s = spec();
        // what carries: unseen, as large as the frame, its middle a quarter to the right (30, 10)
        s.layers[0].opacity = Animated::Constant(0.0);
        s.layers[0].transform.scale = Animated::Constant(1.0);
        // what is carried: 8 by 4, placed in the frame's middle, which is the carrier's anchor
        let mut small = s.layers[0].clone();
        small.id = "small".into();
        small.opacity = Animated::Constant(1.0);
        small.transform = Transform { scale: Animated::Constant(0.2), ..Transform::default() };
        small.parent = Some("half".into());
        s.layers.push(small);
        reelforge_renderspec::validate(&s).expect("valid");
        let img = drawn(&s);
        assert_eq!([px(&img, 25, 10), px(&img, 27, 9), px(&img, 33, 11), px(&img, 34, 10)], [BLUE, RED, RED, BLUE]);
        assert_eq!(px(&img, 20, 10), BLUE);
        // moved 10 to its right, the carrier turned a quarter: it is 10 below, turned with it
        s.layers[1].transform.position = Animated::Constant([0.25, 0.0]);
        s.layers[0].transform.rotation = Animated::Constant(90.0);
        let img = drawn(&s);
        assert_eq!([px(&img, 27, 18), px(&img, 28, 17), px(&img, 31, 19), px(&img, 32, 18)], [BLUE, RED, RED, BLUE]);
        assert_eq!(px(&img, 30, 15), BLUE);
        // the carrier half the size: half as far from it, half as large
        s.layers[0].transform.scale = Animated::Constant(0.5);
        let img = drawn(&s);
        assert_eq!([px(&img, 29, 14), px(&img, 30, 12), px(&img, 30, 13), px(&img, 30, 16)], [RED, BLUE, RED, RED]);
        assert_eq!(px(&img, 31, 14), BLUE);
        // the carrier on the move carries it, on the carrier's own clock even before it starts
        s.layers[0].transform.rotation = Animated::Constant(0.0);
        s.layers[0].transform.scale = Animated::Constant(1.0);
        s.layers[1].transform.position = Animated::Constant([0.0, 0.0]);
        s.layers[0].at = Tick::from_seconds(0.5);
        s.layers[0].dur = Tick::from_seconds(0.5);
        s.layers[0].transform.position = Animated::Keys(vec![
            Key { t: Tick::ZERO, v: [-0.25, 0.0], ease: Ease::Linear },
            Key { t: Tick::from_seconds(0.5), v: [0.25, 0.0], ease: Ease::Linear },
        ]);
        let at = |t: f64| render_frame(&s, Tick::from_seconds(t), 1.0, &NoMedia, Quality::Final).unwrap().0;
        assert_eq!([px(&at(0.0), 10, 10), px(&at(0.75), 20, 10), px(&at(0.75), 10, 10)], [RED, RED, BLUE]);
        assert!(s.required_features().contains("layer.parent/1") && unsupported_features(&s).is_empty());
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
        s.layers[0].content = LayerContent::Media {
            asset: "p".into(),
            time: TimeMap::default(),
            fit: Fit::Stretch,
            between: Default::default(),
        };
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
        s.layers[0].content = LayerContent::Media {
            asset: "v".into(),
            time: TimeMap::default(),
            fit: Fit::Contain,
            between: Default::default(),
        };
        assert!(matches!(
            render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final),
            Err(RenderError::Missing { .. })
        ));
        let (_, report) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).unwrap();
        assert_eq!(report.placeholders.len(), 1);
    }

    #[test]
    fn a_composition_is_drawn_first_and_placed_as_one_picture() {
        // inside: red over the right half's middle (x 20..40), green beside and over it (x 10..30)
        let mut s = spec();
        let red = s.layers.remove(0);
        let mut green = red.clone();
        green.id = "green".into();
        green.content = LayerContent::Solid { color: Color([0.0, 1.0, 0.0, 1.0]) };
        green.transform.position = Animated::Constant([0.0, 0.0]);
        let mut shown = red.clone();
        shown.id = "c".into();
        shown.content = LayerContent::Comp { comp: "k".into(), from: Tick::ZERO };
        shown.transform = Transform::default();
        shown.opacity = Animated::Constant(0.5);
        s.comps.insert("k".into(), Comp { layers: vec![red, green], audio: vec![] });
        s.layers.push(shown);
        validate(&s).unwrap();
        assert!(s.required_features().contains("comp/1") && unsupported_features(&s).is_empty());
        let draw = |s: &RenderSpec| render_frame(s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).unwrap().0;
        let img = draw(&s);
        // half see-through as one picture: where green covers red, no red shows through it
        assert_eq!(px(&img, 25, 10), [0, 188, 188, 255]);
        assert_eq!(px(&img, 35, 10), [188, 0, 188, 255]);
        assert_eq!(px(&img, 15, 10), [0, 188, 188, 255]);
        // and see-through where it draws nothing
        assert_eq!(px(&img, 5, 10), [0, 0, 255, 255]);
        assert_eq!(px(&img, 25, 2), [0, 0, 255, 255]);

        // placed like a picture: half the size about the centre, mirrored
        s.layers[0].transform.scale = Animated::Constant(0.5);
        s.layers[0].flip_x = true;
        let img = draw(&s);
        assert_eq!(px(&img, 35, 10), [0, 0, 255, 255]);
        // red was at x 20..40 of 40: mirrored 0..20, halved about x 20: 10..20
        assert_eq!(px(&img, 12, 10), [188, 0, 188, 255]);
        assert_eq!(px(&img, 22, 10), [0, 188, 188, 255]);

        // it runs from `from`: green is gone after half a second of the composition
        s.layers[0].transform.scale = Animated::Constant(1.0);
        s.layers[0].flip_x = false;
        s.comps.get_mut("k").unwrap().layers[1].dur = Tick(Tick::SECOND.0 / 2);
        s.layers[0].content = LayerContent::Comp { comp: "k".into(), from: Tick(Tick::SECOND.0 / 2) };
        let img = draw(&s);
        assert_eq!(px(&img, 15, 10), [0, 0, 255, 255]);
        assert_eq!(px(&img, 25, 10), [188, 0, 188, 255]);
        // past the composition's last layer there is nothing to draw, and no layer is planned
        s.layers[0].content = LayerContent::Comp { comp: "k".into(), from: Tick::SECOND * 3 };
        assert!(plan_frame(&s, Tick::ZERO, 1.0).layers.is_empty());

        // a composition that shows itself is refused
        let mut looped = s.clone();
        let again = looped.layers[0].clone();
        looped.comps.get_mut("k").unwrap().layers.push(again);
        assert!(validate(&looped).is_err());
    }

    #[test]
    fn plan_is_scaled_for_previews() {
        let plan = plan_frame(&spec(), Tick::ZERO, 0.5);
        assert_eq!((plan.width, plan.height), (20, 10));
        assert_eq!(plan.layers[0].center, [15.0, 5.0]);
    }
}
