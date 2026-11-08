//! The wgpu compositor against the CPU reference, frame by frame.
//!
//! These need an adapter. Without one they say so and pass, unless `REELFORGE_REQUIRE_GPU` is set,
//! as CI sets it (it installs Mesa's software Vulkan, lavapipe).

use std::sync::{Arc, Mutex, OnceLock};

use reelforge_renderspec::*;
use reelforge_time::{FrameRate, Tick};

use super::GpuCompositor;
use crate::*;

fn gpu() -> Option<std::sync::MutexGuard<'static, GpuCompositor>> {
    static GPU: OnceLock<Option<Mutex<GpuCompositor>>> = OnceLock::new();
    let g = GPU.get_or_init(|| match GpuCompositor::headless() {
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
    spec.layers.push(layer("pic", LayerContent::Media { asset: "pic".into(), time: TimeMap::default(), fit }));
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
        layer("t", LayerContent::Text(TextContent::new("Hi".to_string(), 30.0, Color([0.9, 0.6, 0.3, 1.0]))));
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
    t.background = Some(TextBackground { color: Color([0.0, 0.0, 0.0, 0.5]), padding: 0.25, radius: 0.0 });
    let mut l = layer("title", LayerContent::Text(t));
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
    let mut l = layer("v", LayerContent::Media { asset: "v".into(), time: TimeMap::default(), fit: Fit::Contain });
    l.transform.rotation = Animated::Constant(10.0);
    s.layers.push(l);
    same(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Draft);
    let Some(mut g) = gpu() else { return };
    assert!(matches!(g.render(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final), Err(RenderError::Missing { .. })));
}

#[test]
fn yuv_matches_the_cpu_conversion() {
    let Some(mut g) = gpu() else { return };
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
