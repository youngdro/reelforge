//! The wgpu compositor against the CPU reference, frame by frame.
//!
//! These need an adapter. Without one they say so and pass, unless `REELFORGE_REQUIRE_GPU` is set,
//! as CI sets it (it installs Mesa's software Vulkan, lavapipe). `REELFORGE_GPU_LIMITS=webgl2`
//! asks for WebGL2's limits (no compute, 2048-pixel textures), and `WGPU_BACKEND=gl` for the GL
//! backend, which together stand in for a browser without WebGPU.

use std::sync::{Arc, Mutex, OnceLock};

use reelforge_renderspec::*;
use reelforge_time::{FrameRate, Tick};

use super::GpuCompositor;
use crate::*;

fn gpu() -> Option<std::sync::MutexGuard<'static, GpuCompositor>> {
    static GPU: OnceLock<Option<Mutex<GpuCompositor>>> = OnceLock::new();
    let g = GPU.get_or_init(|| match webgl2().unwrap_or_else(GpuCompositor::headless) {
        Ok(g) => {
            eprintln!("GPU tests on {}", g.adapter());
            Some(Mutex::new(g))
        }
        Err(e) => {
            assert!(std::env::var_os("REELFORGE_REQUIRE_GPU").is_none(), "{e}");
            eprintln!("skipping GPU tests: {e}");
            None
        }
    });
    g.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A compositor on a device with WebGL2's limits, when `REELFORGE_GPU_LIMITS=webgl2` says so.
fn webgl2() -> Option<Result<GpuCompositor, super::GpuError>> {
    if std::env::var("REELFORGE_GPU_LIMITS").ok()? != "webgl2" {
        return None;
    }
    let unavailable = |e: String| super::GpuError::Unavailable(e);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    Some((|| {
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .map_err(|e| unavailable(e.to_string()))?;
        let limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() }),
        )
        .map_err(|e| unavailable(e.to_string()))?;
        assert!(GpuCompositor::supports(&adapter));
        Ok(GpuCompositor::new(device, queue, &adapter.get_info()))
    })())
}

/// Renders `spec` at `t` on both compositors and checks every 8-bit value is within one code.
fn same(spec: &RenderSpec, t: Tick, scale: f32, media: &dyn MediaSource, quality: Quality) {
    let Some(mut g) = gpu() else { return };
    let (cpu, cpu_report) = render_frame(spec, t, scale, media, quality).expect("cpu renders");
    let gpu_report = g.render(spec, t, scale, media, quality).expect("gpu renders");
    assert_eq!(cpu_report, gpu_report);
    let (w, h, bytes) = g.read_srgba8().expect("reads back");
    assert_eq!((w, h), (cpu.width, cpu.height));
    let want = cpu.to_srgba8();
    let mut worst = (0, 0);
    for (i, (a, b)) in want.iter().zip(&bytes).enumerate() {
        let d = a.abs_diff(*b);
        if d > worst.0 {
            worst = (d, i);
        }
    }
    let (d, i) = worst;
    let p = i / 4;
    assert!(
        d <= 1,
        "pixel ({}, {}) channel {}: cpu {:?} gpu {:?}",
        p as u32 % w,
        p as u32 / w,
        i % 4,
        &want[p * 4..p * 4 + 4],
        &bytes[p * 4..p * 4 + 4]
    );
}

fn base(w: u32, h: u32) -> RenderSpec {
    RenderSpec::new(Output {
        width: w,
        height: h,
        fps: FrameRate::FPS_30,
        duration: Tick::SECOND,
        background: Color([0.1, 0.2, 0.6, 1.0]),
        video: None,
        audio: None,
        format: Default::default(),
    })
}

fn layer(id: &str, content: LayerContent) -> Layer {
    Layer {
        id: id.into(),
        at: Tick::ZERO,
        dur: Tick::SECOND,
        content,
        transform: Transform::default(),
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
    }
}

/// A picture with detail everywhere: a gradient with a hard-edged checker and a soft alpha ramp.
fn picture(w: u32, h: u32) -> Arc<Image> {
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let check = ((x / 7 + y / 5) % 2) as u8 * 120;
            bytes.extend([(x * 255 / w) as u8, check, (y * 255 / h) as u8, (128 + (x + y) % 128) as u8]);
        }
    }
    Arc::new(Image::from_srgba8(w, h, &bytes))
}

struct Pictures(Arc<Image>);

impl MediaSource for Pictures {
    fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        Ok(Some(self.0.clone()))
    }
}

fn with_picture(spec: &mut RenderSpec, fit: Fit) -> usize {
    spec.assets.insert(
        "pic".into(),
        Asset { kind: AssetKind::Image, sha256: "1".repeat(64), uri: "file:///pic.png".into(), of: None, probe: None },
    );
    spec.layers.push(layer(
        "pic",
        LayerContent::Media { asset: "pic".into(), time: TimeMap::default(), fit, between: Default::default() },
    ));
    spec.layers.len() - 1
}

#[test]
fn solids_with_every_blend_and_opacity() {
    let mut s = base(64, 48);
    let mut l = layer("red", LayerContent::Solid { color: Color([0.9, 0.1, 0.1, 1.0]) });
    l.transform.scale = Animated::Constant(0.5);
    l.transform.position = Animated::Constant([-0.2, 0.1]);
    s.layers.push(l);
    for (i, blend) in [Blend::Add, Blend::Multiply, Blend::Screen, Blend::Normal].into_iter().enumerate() {
        let mut l = layer(&format!("b{i}"), LayerContent::Solid { color: Color([0.3, 0.8, 0.4, 0.7]) });
        l.blend = blend;
        l.opacity = Animated::Constant(0.6);
        l.transform.scale = Animated::Constant(0.4);
        l.transform.position = Animated::Constant([0.1 * i as f32 - 0.1, -0.05]);
        l.transform.rotation = Animated::Constant(15.0 * i as f32);
        s.layers.push(l);
    }
    same(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final);
}

#[test]
fn pictures_fitted_turned_and_scaled() {
    let pic = picture(50, 30);
    for fit in [Fit::Contain, Fit::Cover, Fit::Stretch] {
        let mut s = base(72, 128);
        let i = with_picture(&mut s, fit);
        s.layers[i].transform.rotation = Animated::Constant(23.0);
        s.layers[i].transform.scale = Animated::Constant(0.8);
        s.layers[i].opacity = Animated::Constant(0.9);
        same(&s, Tick::ZERO, 1.0, &Pictures(pic.clone()), Quality::Final);
        // previews are drawn smaller
        same(&s, Tick::ZERO, 0.5, &Pictures(pic.clone()), Quality::Final);
    }
}

#[test]
fn effects_apply_in_order() {
    let mut s = base(40, 40);
    let i = with_picture(&mut s, Fit::Cover);
    let fx = |id: &str, amount: f32| Effect {
        id: id.into(),
        params: [("amount".to_string(), Animated::Constant(amount))].into_iter().collect(),
        asset: None,
    };
    s.layers[i].fx = vec![fx("color.saturation@1", 0.2), fx("color.brightness@1", 0.15), fx("color.saturation@1", 1.8)];
    same(&s, Tick::ZERO, 1.0, &Pictures(picture(40, 40)), Quality::Final);
}

/// Pictures and one lookup table.
struct Graded(Arc<Image>, Arc<Lut>);

impl MediaSource for Graded {
    fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        Ok(Some(self.0.clone()))
    }

    fn lut(&self, _: &str, _: &Asset) -> Result<Option<Arc<Lut>>, String> {
        Ok(Some(self.1.clone()))
    }
}

#[test]
fn grades_and_lookup_tables_match() {
    let mut s = base(48, 48);
    let i = with_picture(&mut s, Fit::Cover);
    s.assets.insert(
        "look".into(),
        Asset { kind: AssetKind::Lut, sha256: "2".repeat(64), uri: "file:///look.cube".into(), of: None, probe: None },
    );
    let params = |pairs: &[(&str, f32)]| pairs.iter().map(|(k, v)| (k.to_string(), Animated::Constant(*v))).collect();
    s.layers[i].fx = vec![
        Effect {
            id: effects::GRADE.into(),
            params: params(&[
                ("exposure", 0.4),
                ("contrast", 0.3),
                ("saturation", -0.2),
                ("temperature", 0.5),
                ("tint", -0.2),
                ("highlights", -0.4),
                ("shadows", 0.3),
                ("fade", 0.3),
                ("vignette", 0.6),
            ]),
            asset: None,
        },
        Effect { id: effects::LUT.into(), params: params(&[("amount", 0.8)]), asset: Some("look".into()) },
    ];
    // a table that warms and crushes: not the identity anywhere
    let mut lut = Lut::identity(9);
    for c in &mut lut.data {
        *c = [c[0].sqrt(), c[1] * c[1], c[2] * 0.7 + 0.1];
    }
    let media = Graded(picture(48, 48), Arc::new(lut));
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    // and on a title
    let mut t = base(96, 48);
    let mut title =
        layer("t", LayerContent::Text(Box::new(TextContent::new("Hi".to_string(), 30.0, Color([0.9, 0.6, 0.3, 1.0])))));
    title.fx = s.layers[i].fx.clone();
    t.assets = s.assets.clone();
    t.layers.push(title);
    same(&t, Tick::ZERO, 1.0, &media, Quality::Final);
}

#[test]
fn text_with_outline_and_box_over_a_multiplied_picture() {
    let mut s = base(320, 180);
    let i = with_picture(&mut s, Fit::Contain);
    s.layers[i].blend = Blend::Multiply;
    let mut t = TextContent::new("字幕 Subtitle", 40.0, Color([1.0, 0.95, 0.2, 1.0]));
    t.weight = 700;
    t.stroke = Some(TextStroke { color: Color([0.0, 0.0, 0.0, 1.0]), width: 4.0 });
    t.background = Some(TextBackground {
        padding: 0.25,
        radius: 0.0,
        lines: false,
        ..TextBackground::of(Color([0.0, 0.0, 0.0, 0.5]))
    });
    let mut l = layer("title", LayerContent::Text(Box::new(t)));
    l.transform.rotation = Animated::Constant(-8.0);
    l.transform.position = Animated::Constant([0.0, 0.3]);
    // half off the bottom: clipped the same way
    s.layers.push(l);
    same(&s, Tick::ZERO, 1.0, &Pictures(picture(64, 36)), Quality::Final);
}

#[test]
fn drafts_draw_the_same_slate_and_report() {
    let mut s = base(48, 64);
    s.assets.insert(
        "v".into(),
        Asset {
            kind: AssetKind::Video,
            sha256: "0".repeat(64),
            uri: "file:///gone.mp4".into(),
            of: None,
            probe: Some(Probe { width: Some(30), height: Some(20), duration: None }),
        },
    );
    let mut l = layer(
        "v",
        LayerContent::Media {
            asset: "v".into(),
            time: TimeMap::default(),
            fit: Fit::Contain,
            between: Default::default(),
        },
    );
    l.transform.rotation = Animated::Constant(10.0);
    s.layers.push(l);
    same(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Draft);
    let Some(mut g) = gpu() else { return };
    assert!(matches!(g.render(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final), Err(RenderError::Missing { .. })));
}

#[test]
fn yuv_matches_the_cpu_conversion() {
    let Some(mut g) = gpu() else { return };
    if !g.converts_yuv() {
        // no compute shaders (WebGL2): exports there are drawn on the CPU
        return;
    }
    // odd sizes: the last chroma column and row average fewer pixels
    let mut s = base(37, 21);
    with_picture(&mut s, Fit::Stretch);
    let media = Pictures(picture(37, 21));
    g.render(&s, Tick::ZERO, 1.0, &media, Quality::Final).expect("renders");
    let (mut y, mut u, mut v) = (vec![], vec![], vec![]);
    g.read_yuv420(&mut y, &mut u, &mut v).expect("reads back");
    let (img, _) = render_frame(&s, Tick::ZERO, 1.0, &media, Quality::Final).expect("cpu renders");
    let want = cpu_yuv(&img);
    assert_eq!((y.len(), u.len(), v.len()), (37 * 21, 19 * 11, 19 * 11));
    for (plane, (got, want)) in [(&y, &want.0), (&u, &want.1), (&v, &want.2)].into_iter().enumerate() {
        for (i, (a, b)) in got.iter().zip(want).enumerate() {
            assert!(a.abs_diff(*b) <= 1, "plane {plane} sample {i}: gpu {a} cpu {b}");
        }
    }
}

/// `to_yuv420` from reelforge-media, restated (this crate cannot depend on it).
fn cpu_yuv(img: &Image) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use crate::image::linear_to_srgb;
    let (w, h) = (img.width as usize, img.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let rgb = |x: usize, y: usize| img.pixels[y * w + x].map(|c| linear_to_srgb(c.clamp(0.0, 1.0)));
    let luma = |p: [f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let y = (0..w * h).map(|i| (16.0 + 219.0 * luma(rgb(i % w, i / w))).round() as u8).collect();
    let (mut u, mut v) = (vec![], vec![]);
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut cb, mut cr, mut n) = (0.0, 0.0, 0.0);
            for yy in cy * 2..(cy * 2 + 2).min(h) {
                for xx in cx * 2..(cx * 2 + 2).min(w) {
                    let p = rgb(xx, yy);
                    let l = luma(p);
                    cb += (p[2] - l) / 1.8556;
                    cr += (p[0] - l) / 1.5748;
                    n += 1.0;
                }
            }
            u.push((128.0 + 224.0 * cb / n).round().clamp(16.0, 240.0) as u8);
            v.push((128.0 + 224.0 * cr / n).round().clamp(16.0, 240.0) as u8);
        }
    }
    (y, u, v)
}

#[test]
fn crops_flips_masks_and_every_blend_match() {
    let pic = picture(60, 40);
    for (k, shape) in
        [MaskShape::Linear, MaskShape::Mirror, MaskShape::Circle, MaskShape::Rectangle].into_iter().enumerate()
    {
        let mut s = base(64, 96);
        // a backdrop for the blends to work on
        s.layers.push(layer("under", LayerContent::Solid { color: Color([0.2, 0.5, 0.8, 1.0]) }));
        let i = with_picture(&mut s, Fit::Contain);
        s.layers[i].crop = Some(Crop { left: 0.1, top: 0.2, right: 0.25, bottom: 0.05 });
        s.layers[i].flip_x = k % 2 == 0;
        s.layers[i].flip_y = k >= 2;
        s.layers[i].transform.rotation = Animated::Constant(12.0);
        s.layers[i].shape_mask = Some(ShapeMask {
            shape,
            center: [0.45, 0.55],
            size: [0.5, 0.3],
            rotation: 20.0,
            feather: 0.08 * k as f32,
            invert: k == 1,
        });
        same(&s, Tick::ZERO, 1.0, &Pictures(pic.clone()), Quality::Final);
    }
    for blend in [
        Blend::Overlay,
        Blend::Darken,
        Blend::Lighten,
        Blend::SoftLight,
        Blend::HardLight,
        Blend::Difference,
        Blend::ColorDodge,
        Blend::ColorBurn,
    ] {
        let mut s = base(48, 48);
        s.layers.push(layer("under", LayerContent::Solid { color: Color([0.3, 0.6, 0.45, 0.9]) }));
        let i = with_picture(&mut s, Fit::Cover);
        s.layers[i].blend = blend;
        s.layers[i].opacity = Animated::Constant(0.8);
        // a solid masked by a circle on top, blended too
        let mut dot = layer("dot", LayerContent::Solid { color: Color([0.9, 0.4, 0.1, 1.0]) });
        dot.blend = blend;
        dot.shape_mask = Some(ShapeMask {
            shape: MaskShape::Circle,
            center: [0.5, 0.5],
            size: [0.6, 0.6],
            rotation: 0.0,
            feather: 0.1,
            invert: false,
        });
        s.layers.push(dot);
        same(&s, Tick::ZERO, 1.0, &Pictures(picture(48, 48)), Quality::Final);
    }
}

/// A video source that has its pictures as Y'CbCr: the GPU converts them, the CPU is handed
/// [`Yuv::to_image`].
struct Video(Arc<Yuv>);

impl MediaSource for Video {
    fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        Ok(Some(Arc::new(self.0.to_image())))
    }
    fn yuv(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Yuv>>, String> {
        Ok(Some(self.0.clone()))
    }
}

fn yuv(w: u32, h: u32, shift: (u32, u32), full_range: bool) -> Arc<Yuv> {
    let (cw, ch) = (w.div_ceil(1 << shift.0), h.div_ceil(1 << shift.1));
    let luma = (0..h).flat_map(|y| (0..w).map(move |x| (16 + (x * 7 + y * 3) % 220) as u8)).collect();
    let cb = (0..ch).flat_map(|y| (0..cw).map(move |x| (16 + (x * 13 + y * 29) % 224) as u8)).collect();
    let cr = (0..ch).flat_map(|y| (0..cw).map(move |x| (16 + (x * 31 + y * 11) % 224) as u8)).collect();
    Arc::new(Yuv {
        width: w,
        height: h,
        planes: [Arc::new(luma), Arc::new(cb), Arc::new(cr)],
        shift,
        kr_kb: (0.2126, 0.0722),
        full_range,
    })
}

#[test]
fn video_pictures_converted_on_the_gpu_match_the_cpu() {
    for (shift, full) in [((1, 1), false), ((1, 0), true), ((0, 0), false)] {
        let video = Video(yuv(51, 31, shift, full));
        let mut s = base(51, 31);
        s.assets.insert(
            "v".into(),
            Asset {
                kind: AssetKind::Video,
                sha256: "2".repeat(64),
                uri: "file:///v.mp4".into(),
                of: None,
                probe: None,
            },
        );
        s.layers.push(layer(
            "v",
            LayerContent::Media {
                asset: "v".into(),
                time: TimeMap::default(),
                fit: Fit::Stretch,
                between: Default::default(),
            },
        ));
        same(&s, Tick::ZERO, 1.0, &video, Quality::Final);
        // turned and smaller, over the background
        s.layers[0].transform.rotation = Animated::Constant(17.0);
        s.layers[0].transform.scale = Animated::Constant(0.7);
        same(&s, Tick::ZERO, 0.5, &video, Quality::Final);
    }
}

#[test]
fn pictures_larger_than_the_device_takes_are_fitted_and_frames_refused() {
    let Some(mut g) = gpu() else { return };
    let side = g.max_side();
    // a picture one pixel wider than a texture may be, with no probe to say so ahead
    let wide = Arc::new(Image::filled(side + 1, 2, [0.5, 0.25, 0.0, 1.0]));
    let mut s = base(64, 36);
    with_picture(&mut s, Fit::Stretch);
    g.render(&s, Tick::ZERO, 1.0, &Pictures(wide), Quality::Final).expect("drawn from a smaller copy");
    let (w, _, px) = g.read_srgba8().expect("reads back");
    let mid = &px[((18 * w + 32) * 4) as usize..][..4];
    assert!(mid[0] > 150 && mid[1] > 100 && mid[2] < 40, "{mid:?}");
    // a frame larger than that is refused, not a device error
    let huge = base(side + 1, 16);
    assert!(g.render(&huge, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).is_err());
}

fn fx_of(id: &str, pairs: &[(&str, f32)]) -> Effect {
    Effect {
        id: id.into(),
        params: pairs.iter().map(|(k, v)| (k.to_string(), Animated::Constant(*v))).collect(),
        asset: None,
    }
}

#[test]
fn a_keyed_colour_goes_the_same_way_on_both() {
    // green of several shades around a red box with soft edges, like a subject on a green screen
    let (w, h) = (48u32, 40u32);
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let inside = (12..36).contains(&x) && (8..32).contains(&y);
            let edge = x == 12 || x == 35 || y == 8 || y == 31;
            let shade = 150 + ((x * 3 + y * 5) % 80) as u8;
            bytes.extend(match (inside, edge) {
                (true, false) => [200, 60 + (x * 2) as u8, 50, 255],
                (true, true) => [120, 150, 50, 255],
                _ => [20 + (y % 30) as u8, shade, 30, 255],
            });
        }
    }
    let pic = Arc::new(Image::from_srgba8(w, h, &bytes));
    for (softness, spill) in [(0.1, 0.5), (0.0, 0.0), (0.4, 1.0)] {
        let mut s = base(64, 48);
        let i = with_picture(&mut s, Fit::Contain);
        s.layers[i].fx = vec![
            fx_of(
                effects::CHROMA_KEY,
                &[("r", 0.1), ("g", 0.75), ("b", 0.12), ("tolerance", 0.25), ("softness", softness), ("spill", spill)],
            ),
            fx_of(effects::GRADE, &[("exposure", 0.3)]),
        ];
        same(&s, Tick::ZERO, 1.0, &Pictures(pic.clone()), Quality::Final);
    }
}

#[test]
fn people_are_cut_out_the_same_way_on_both() {
    // a picture, with an effect after: both are handed the cut-out
    let portrait = crate::tests::portrait();
    let mut s = base(64, 48);
    let i = with_picture(&mut s, Fit::Contain);
    let key = fx_of(effects::PERSON_KEY, &[]);
    s.layers[i].fx = vec![key.clone(), fx_of(effects::GRADE, &[("exposure", 0.3)])];
    same(&s, Tick::ZERO, 1.0, &Pictures(portrait), Quality::Final);
    // a video the GPU would convert itself: here it is converted before, to be looked at whole
    let video = Video(yuv(64, 48, (1, 1), false));
    s.layers[i].fx = vec![key.clone()];
    same(&s, Tick::ZERO, 1.0, &video, Quality::Final);
    // a composition: drawn on the CPU for both, as a blurred one is
    let mut inner = Comp::default();
    inner.layers.push(layer("in", LayerContent::Solid { color: Color([0.9, 0.5, 0.3, 1.0]) }));
    s.comps.insert("k".into(), inner);
    s.layers.push(layer("c", LayerContent::Comp { comp: "k".into(), from: Tick::ZERO }));
    s.layers[i + 1].fx = vec![key];
    same(&s, Tick::ZERO, 1.0, &video, Quality::Final);
}

#[test]
fn backdrops_are_cut_and_shadows_cast_the_same_way_on_both() {
    let thing = crate::tests::boxed_on_a_sweep(96, 64);
    let mut s = base(64, 48);
    let i = with_picture(&mut s, Fit::Contain);
    s.layers[i].transform.scale = Animated::Constant(0.8);
    // cut out, with a soft shadow beside it and an effect after
    let key = fx_of(effects::BACKDROP_KEY, &[("tolerance", 0.5), ("holes", 0.5)]);
    let soft = fx_of(effects::SHADOW, &[("x", 0.06), ("y", 0.08), ("blur", 0.04), ("opacity", 0.7)]);
    s.layers[i].fx = vec![key.clone(), fx_of(effects::GRADE, &[("exposure", 0.3)]), soft.clone()];
    same(&s, Tick::ZERO, 1.0, &Pictures(thing.clone()), Quality::Final);
    // lying on the ground and coloured, under a layer that is turned, cropped and half there
    let lying = fx_of(
        effects::SHADOW,
        &[("x", 0.0), ("y", 0.02), ("blur", 0.02), ("flat", 0.7), ("lean", 0.4), ("r", 0.3), ("b", 0.5)],
    );
    s.layers[i].fx = vec![key.clone(), lying];
    s.layers[i].transform.rotation = Animated::Constant(20.0);
    s.layers[i].crop = Some(Crop { left: 0.1, top: 0.0, right: 0.0, bottom: 0.05 });
    s.layers[i].opacity = Animated::Constant(0.5);
    same(&s, Tick::ZERO, 1.0, &Pictures(thing.clone()), Quality::Final);
    // of a picture that is not cut out, behind a round mask: the mask's outline
    s.layers[i].fx = vec![soft.clone()];
    s.layers[i].crop = None;
    s.layers[i].shape_mask = Some(ShapeMask {
        shape: MaskShape::Circle,
        center: [0.5, 0.5],
        size: [0.7, 0.7],
        rotation: 0.0,
        feather: 0.0,
        invert: false,
    });
    same(&s, Tick::ZERO, 1.0, &Pictures(picture(96, 64)), Quality::Final);
    // a video the GPU would convert itself, and a composition: each casts its own
    s.layers[i].shape_mask = None;
    let video = Video(yuv(64, 48, (1, 1), false));
    same(&s, Tick::ZERO, 1.0, &video, Quality::Final);
    let mut inner = Comp::default();
    let mut small = layer("in", LayerContent::Solid { color: Color([0.9, 0.5, 0.3, 1.0]) });
    small.transform.scale = Animated::Constant(0.5);
    inner.layers.push(small);
    s.comps.insert("k".into(), inner);
    s.layers.push(layer("c", LayerContent::Comp { comp: "k".into(), from: Tick::ZERO }));
    s.layers[i + 1].fx = vec![soft];
    s.layers[i + 1].transform.position = Animated::Constant([0.1, -0.1]);
    same(&s, Tick::ZERO, 1.0, &video, Quality::Final);
}

#[test]
fn distortions_read_the_same_places_on_both() {
    let pic = picture(60, 44);
    let cases: Vec<Vec<Effect>> = vec![
        vec![fx_of(effects::SPLIT, &[("x", 0.031), ("y", -0.017)])],
        vec![fx_of(effects::MOSAIC, &[("size", 0.137)])],
        vec![fx_of(effects::WAVE, &[("amount", 0.043), ("waves", 3.3), ("phase", 0.21)])],
        vec![fx_of(effects::GLITCH, &[("amount", 0.13), ("seed", 7.0)])],
        vec![fx_of(effects::GLITCH, &[("amount", 0.2), ("seed", 41.6)])],
        // all at once, with a colour effect after them
        vec![
            fx_of(effects::SPLIT, &[("x", 0.02)]),
            fx_of(effects::GLITCH, &[("amount", 0.1), ("seed", 3.0)]),
            fx_of(effects::WAVE, &[("amount", 0.02)]),
            fx_of(effects::MOSAIC, &[("size", 0.071)]),
            fx_of(effects::GRADE, &[("contrast", 0.3)]),
        ],
    ];
    for fx in cases {
        let mut s = base(96, 72);
        let i = with_picture(&mut s, Fit::Contain);
        s.layers[i].fx = fx;
        s.layers[i].transform.scale = Animated::Constant(0.9);
        same(&s, Tick::ZERO, 1.0, &Pictures(pic.clone()), Quality::Final);
        // cropped, mirrored and turned: positions are of the visible part
        s.layers[i].crop = Some(Crop { left: 0.1, top: 0.05, right: 0.2, bottom: 0.15 });
        s.layers[i].flip_x = true;
        s.layers[i].transform.rotation = Animated::Constant(11.0);
        same(&s, Tick::ZERO, 1.0, &Pictures(pic.clone()), Quality::Final);
    }
    // text is split too
    let mut s = base(160, 80);
    let mut l = layer("t", LayerContent::Text(Box::new(TextContent::new("Glitch".to_string(), 40.0, Color([1.0; 4])))));
    l.fx = vec![fx_of(effects::SPLIT, &[("x", 0.03)]), fx_of(effects::GLITCH, &[("amount", 0.1), ("seed", 2.0)])];
    s.layers.push(l);
    same(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final);
}

#[test]
fn layers_smeared_along_their_way_match() {
    let frame = FrameRate::FPS_30.frame_duration();
    let moving = |from: [f32; 2], to: [f32; 2]| {
        Animated::Keys(vec![
            Key { t: Tick::ZERO, v: from, ease: Ease::Linear },
            Key { t: frame * 3, v: to, ease: Ease::Linear },
        ])
    };
    let pic = picture(50, 30);
    let mut s = base(96, 72);
    // a picture going across while it turns and grows, masked and graded
    let i = with_picture(&mut s, Fit::Contain);
    s.layers[i].transform.position = moving([-0.3, -0.1], [0.3, 0.1]);
    s.layers[i].transform.rotation = Animated::Keys(vec![
        Key { t: Tick::ZERO, v: -20.0, ease: Ease::Linear },
        Key { t: frame * 3, v: 25.0, ease: Ease::Linear },
    ]);
    s.layers[i].transform.scale = Animated::Keys(vec![
        Key { t: Tick::ZERO, v: 0.4, ease: Ease::Linear },
        Key { t: frame * 3, v: 0.9, ease: Ease::Linear },
    ]);
    s.layers[i].fx = vec![fx_of(effects::GRADE, &[("exposure", 0.4), ("saturation", 0.3)])];
    s.layers[i].motion_blur = Some(MotionBlur { shutter: 1.0 });
    // a see-through colour falling, multiplied over it
    let mut square = layer("sq", LayerContent::Solid { color: Color([0.9, 0.5, 0.2, 0.8]) });
    square.blend = Blend::Multiply;
    square.transform.scale = Animated::Constant(0.4);
    square.transform.position = moving([0.1, -0.4], [0.1, 0.4]);
    square.motion_blur = Some(MotionBlur { shutter: 0.5 });
    s.layers.push(square);
    // a title flying in, and a shape swung in space about its edge
    let mut title = layer("t", LayerContent::Text(Box::new(TextContent::new("Go".to_string(), 30.0, Color([1.0; 4])))));
    title.transform.position = moving([-0.6, 0.3], [0.2, 0.3]);
    title.motion_blur = Some(MotionBlur { shutter: 1.0 });
    s.layers.push(title);
    let mut door = layer("door", LayerContent::Solid { color: Color([0.2, 0.8, 0.4, 1.0]) });
    door.transform.scale = Animated::Constant(0.3);
    door.transform.position = Animated::Constant([-0.25, -0.25]);
    door.transform.anchor = Some([0.0, 0.5]);
    door.transform.turn = Some(Turn {
        y: Animated::Keys(vec![
            Key { t: Tick::ZERO, v: -80.0, ease: Ease::Linear },
            Key { t: frame * 3, v: 40.0, ease: Ease::Linear },
        ]),
        ..Turn::held(0.0, 0.0)
    });
    door.motion_blur = Some(MotionBlur { shutter: 1.0 });
    s.layers.push(door);
    reelforge_renderspec::validate(&s).expect("valid");
    let media = Pictures(pic);
    for f in [1, 2] {
        same(&s, frame * f, 1.0, &media, Quality::Final);
    }
    same(&s, frame, 0.5, &media, Quality::Final);
    // and at rest
    same(&s, frame * 6, 1.0, &media, Quality::Final);
}

#[test]
fn compositions_match_however_they_are_placed() {
    let pic = picture(50, 30);
    let mut s = base(96, 72);
    with_picture(&mut s, Fit::Contain);
    // inside: the picture turned, a see-through square multiplied over part of it, and a title
    let mut inside = s.layers.pop().unwrap();
    inside.transform.rotation = Animated::Constant(10.0);
    inside.transform.scale = Animated::Constant(0.8);
    let mut square = layer("sq", LayerContent::Solid { color: Color([0.9, 0.5, 0.2, 0.8]) });
    square.blend = Blend::Multiply;
    square.transform.scale = Animated::Constant(0.5);
    square.transform.position = Animated::Constant([0.15, -0.1]);
    let title = layer("t", LayerContent::Text(Box::new(TextContent::new("Ab".to_string(), 30.0, Color([1.0; 4])))));
    s.comps.insert("k".into(), Comp { layers: vec![inside, square, title], audio: vec![] });

    let mut behind = layer("behind", LayerContent::Solid { color: Color([0.2, 0.7, 0.3, 1.0]) });
    behind.transform.scale = Animated::Constant(0.7);
    s.layers.push(behind);
    s.layers.push(layer("c", LayerContent::Comp { comp: "k".into(), from: Tick::ZERO }));
    let media = Pictures(pic);
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    same(&s, Tick::ZERO, 0.5, &media, Quality::Final);

    // faded, moved, cropped, mirrored, masked, with effects of its own, blended by the shader
    let c = &mut s.layers[1];
    c.opacity = Animated::Constant(0.6);
    c.transform = Transform {
        position: Animated::Constant([0.1, 0.05]),
        scale: Animated::Constant(0.8),
        rotation: Animated::Constant(-15.0),
        ..Transform::default()
    };
    c.crop = Some(Crop { left: 0.1, top: 0.0, right: 0.05, bottom: 0.2 });
    c.flip_x = true;
    c.fx = vec![fx_of(effects::GRADE, &[("contrast", 0.2)]), fx_of(effects::WAVE, &[("amount", 0.03)])];
    c.blend = Blend::Multiply;
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    // blurred: its picture is made on the CPU for both
    s.layers[1].fx = vec![fx_of(effects::BLUR, &[("radius", 0.03)])];
    s.layers[1].blend = Blend::Normal;
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);

    // a composition that shows another, twice
    let mut twice = Comp::default();
    for (i, x) in [-0.25f32, 0.25].into_iter().enumerate() {
        let mut l = layer(&format!("k{i}"), LayerContent::Comp { comp: "k".into(), from: Tick::ZERO });
        l.transform.scale = Animated::Constant(0.5);
        l.transform.position = Animated::Constant([x, 0.0]);
        l.opacity = Animated::Constant(1.0 - 0.3 * i as f32);
        twice.layers.push(l);
    }
    s.comps.insert("two".into(), twice);
    s.layers[1].fx.clear();
    s.layers[1].content = LayerContent::Comp { comp: "two".into(), from: Tick::ZERO };
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
}

#[test]
fn the_glow_is_the_frame_far_out_of_focus() {
    let Some(mut g) = gpu() else { return };
    // one colour all over: the glow is that colour
    let mut s = base(64, 36);
    s.layers.push(layer("all", LayerContent::Solid { color: Color([0.5, 0.25, 0.0, 1.0]) }));
    g.render(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).expect("renders");
    let (_, _, frame) = g.read_srgba8().expect("reads back");
    let (w, h, glow) = g.read_rgba8(g.glow().expect("a frame was drawn")).expect("reads back");
    assert_eq!((w, h), (super::GLOW, super::GLOW));
    for px in glow.chunks(4) {
        assert!(px.iter().zip(&frame[..4]).all(|(a, b)| a.abs_diff(*b) <= 1), "{px:?} of {:?}", &frame[..4]);
    }
    // a small white square on black at the left: the glow is brighter there, and nowhere a square
    let mut s = base(64, 36);
    let mut l = layer("spot", LayerContent::Solid { color: Color([1.0, 1.0, 1.0, 1.0]) });
    l.transform.scale = Animated::Constant(0.2);
    l.transform.position = Animated::Constant([-0.3, 0.0]);
    s.layers.push(l);
    g.render(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).expect("renders");
    let (w, _, glow) = g.read_rgba8(g.glow().expect("a frame was drawn")).expect("reads back");
    let row = &glow[(8 * w * 4) as usize..][..(w * 4) as usize];
    let red = |x: usize| row[x * 4];
    assert!(red(3) > red(12) + 10, "{row:?}");
    assert!((0..w as usize).all(|x| red(x) < 200 && row[x * 4 + 3] == 255), "{row:?}");
    assert!((1..w as usize).all(|x| red(x).abs_diff(red(x - 1)) < 60), "{row:?}");
}

#[test]
fn layers_stretched_anchored_turned_in_space_and_carried_match_the_cpu() {
    let mut s = base(96, 64);
    let pic = picture(60, 40);
    // a picture swung like a door about its left edge, squeezed, cropped and masked
    let i = with_picture(&mut s, Fit::Contain);
    let p = &mut s.layers[i];
    p.transform = Transform {
        position: Animated::Constant([-0.2, 0.05]),
        scale: Animated::Constant(0.7),
        rotation: Animated::Constant(12.0),
        stretch: Some(Animated::Constant([1.0, 0.8])),
        anchor: Some([0.0, 0.5]),
        turn: Some(Turn::held(20.0, 48.0)),
        skew: None,
    };
    p.crop = Some(Crop { left: 0.1, top: 0.0, right: 0.0, bottom: 0.1 });
    p.flip_x = true;
    p.fx = vec![fx_of(effects::GRADE, &[("contrast", 0.2)])];
    // a title carried by it, tipped back on its own
    let mut title = layer("t", LayerContent::Text(Box::new(TextContent::new("Ab".to_string(), 26.0, Color([1.0; 4])))));
    title.parent = Some("pic".into());
    title.transform.position = Animated::Constant([0.3, -0.2]);
    title.transform.turn = Some(Turn { distance: 1.2, ..Turn::held(-35.0, 0.0) });
    s.layers.push(title);
    // a square carried by the title, stretched, blended by the shader
    let mut square = layer("sq", LayerContent::Solid { color: Color([0.9, 0.5, 0.2, 0.8]) });
    square.parent = Some("t".into());
    square.blend = Blend::Multiply;
    square.transform.scale = Animated::Constant(0.3);
    square.transform.stretch = Some(Animated::Constant([1.6, 0.7]));
    square.transform.position = Animated::Constant([0.1, 0.25]);
    s.layers.push(square);
    reelforge_renderspec::validate(&s).expect("valid");
    let media = Pictures(pic);
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    same(&s, Tick::ZERO, 0.5, &media, Quality::Final);
    // turned past edge on: its back, mirrored
    s.layers[i].transform.turn = Some(Turn::held(0.0, 140.0));
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    // leaning: the picture flat, the title and the square it carries each their own way
    s.layers[i].transform.turn = None;
    s.layers[i].transform.skew = Some(Animated::Constant([25.0, -10.0]));
    let title = s.layers.iter_mut().find(|l| l.id == "t").expect("the title");
    title.transform.skew = Some(Animated::Constant([-30.0, 0.0]));
    let square = s.layers.iter_mut().find(|l| l.id == "sq").expect("the square");
    square.transform.skew = Some(Animated::Constant([0.0, 40.0]));
    reelforge_renderspec::validate(&s).expect("valid");
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
}

#[test]
fn the_frost_is_the_frame_blurred_with_room_around_it() {
    let Some(mut g) = gpu() else { return };
    // one colour all over: the frost is that colour where the picture is, and fades out around it
    let mut s = base(64, 36);
    s.layers.push(layer("all", LayerContent::Solid { color: Color([0.5, 0.25, 0.0, 1.0]) }));
    g.render(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final).expect("renders");
    let (_, _, frame) = g.read_srgba8().expect("reads back");
    let (w, h, frost) = g.read_rgba8(g.frost().expect("a frame was drawn")).expect("reads back");
    assert_eq!((w, h), (super::FROST, super::FROST));
    let at = |x: u32, y: u32| &frost[((y * w + x) * 4) as usize..][..4];
    let middle = at(w / 2, h / 2);
    assert!(middle.iter().zip(&frame[..4]).all(|(a, b)| a.abs_diff(*b) <= 1), "{middle:?} of {:?}", &frame[..4]);
    // the picture's left edge is `FROST_ROOM` of its width in: half of the light there
    let edge = (super::FROST_ROOM / (1.0 + 2.0 * super::FROST_ROOM) * w as f32) as u32;
    let alpha = |x: u32| at(x, h / 2)[3];
    assert!((90..=165).contains(&alpha(edge)), "{}", alpha(edge));
    assert!((0..w / 2).all(|x| alpha(x) <= alpha(x + 1)), "{:?}", (0..w).map(alpha).collect::<Vec<_>>());
    // premultiplied, and nothing at the very edge
    assert!(at(edge, h / 2)[0] < middle[0]);
    for i in 0..w {
        for px in [at(i, 0), at(i, h - 1), at(0, i), at(w - 1, i)] {
            assert_eq!(px, [0; 4]);
        }
    }
}

#[test]
fn layers_out_of_focus_match_the_cpu() {
    use reelforge_renderspec::{Paint, ShapeContent, ShapeKind};
    let mut s = base(160, 96);
    let soft = |px: f32| vec![fx_of(effects::SOFT, &[("radius", px)])];
    // a picture, with a blur of its picture as well
    let i = with_picture(&mut s, Fit::Contain);
    s.layers[i].transform.scale = Animated::Constant(0.6);
    s.layers[i].fx = vec![fx_of(effects::BLUR, &[("radius", 0.02)]), fx_of(effects::SOFT, &[("radius", 6.0)])];
    // a title drawn straight onto the frame, and one turned and stretched
    let mut title = layer("t", LayerContent::Text(Box::new(TextContent::new("Ab".to_string(), 40.0, Color([1.0; 4])))));
    title.transform.position = Animated::Constant([-0.2, -0.2]);
    title.fx = soft(3.0);
    s.layers.push(title.clone());
    title.id = "t2".into();
    title.transform.position = Animated::Constant([0.25, 0.2]);
    title.transform.stretch = Some(Animated::Constant([1.4, 0.8]));
    title.transform.skew = Some(Animated::Constant([20.0, 0.0]));
    s.layers.push(title);
    // a shape near the frame's edge: its blur stops at the frame
    let mut dot = layer(
        "dot",
        LayerContent::Shape(Box::new(ShapeContent {
            kind: ShapeKind::Ellipse,
            size: [40.0, 40.0],
            radius: 0.0,
            points: 5,
            inner: 0.5,
            fill: Some(Paint::Solid { color: Color([1.0, 0.3, 0.1, 1.0]) }),
            stroke: None,
            shadow: None,
            trim: None,
            fill_opacity: None,
            fill_color: None,
            keys: None,
        })),
    );
    dot.transform.position = Animated::Constant([0.45, -0.4]);
    dot.fx = soft(8.0);
    s.layers.push(dot);
    reelforge_renderspec::validate(&s).expect("valid");
    assert!(crate::unsupported_features(&s).is_empty());
    let media = Pictures(picture(60, 40));
    same(&s, Tick::ZERO, 1.0, &media, Quality::Final);
    same(&s, Tick::ZERO, 0.5, &media, Quality::Final);
}
