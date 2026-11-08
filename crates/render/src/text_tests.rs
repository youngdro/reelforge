//! Text as pixels: where the ink lands, how much of it, in which colours.

use std::sync::Arc;

use reelforge_renderspec::*;
use reelforge_time::{FrameRate, Tick};

use crate::*;

const W: u32 = 480;
const H: u32 = 240;

fn spec_with(text: TextContent) -> RenderSpec {
    let mut s = RenderSpec::new(Output {
        width: W,
        height: H,
        fps: FrameRate::FPS_30,
        duration: Tick::SECOND,
        background: Color([0.0, 0.0, 1.0, 1.0]),
        video: None,
        audio: None,
    });
    s.layers.push(Layer {
        id: "title".into(),
        at: Tick::ZERO,
        dur: Tick::SECOND,
        content: LayerContent::Text(text),
        transform: Transform::default(),
        opacity: Animated::Constant(1.0),
        blend: Blend::Normal,
        mask: None,
        fx: vec![],
    });
    s
}

fn white(text: &str, size: f32) -> TextContent {
    TextContent::new(text, size, Color([1.0; 4]))
}

fn draw(s: &RenderSpec, scale: f32) -> Image {
    let (img, report) = render_frame(s, Tick::ZERO, scale, &NoMedia, Quality::Final).expect("renders");
    assert!(report.placeholders.is_empty());
    img
}

/// Total red (the background has none, white text has it all), in pixels' worth.
fn ink(img: &Image) -> f32 {
    img.pixels.iter().map(|p| p[0]).sum()
}

/// The box `[x0, y0, x1, y1]` of pixels with any red.
fn ink_box(img: &Image) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..img.height {
        for x in 0..img.width {
            if img.pixels[(y * img.width + x) as usize][0] > 0.05 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    b
}

#[test]
fn chinese_and_latin_text_is_drawn_centred() {
    let img = draw(&spec_with(white("短视频 Hi", 64.0)), 1.0);
    let b = ink_box(&img);
    assert!(b[2] > b[0], "nothing drawn");
    let (cx, cy) = ((b[0] + b[2]) as f32 / 2.0, (b[1] + b[3]) as f32 / 2.0);
    assert!((cx - W as f32 / 2.0).abs() < 4.0, "centre x {cx}");
    assert!((cy - H as f32 / 2.0).abs() < 12.0, "centre y {cy}");
    // three ideographs and two latin letters at 64 px: roughly 3 × 64 + 2 × 35 wide
    let w = b[2] - b[0];
    assert!((230..300).contains(&w), "width {w}");
    // solid white strokes exist: the fill colour arrives unchanged
    assert!(img.to_srgba8().chunks(4).any(|p| p == [255, 255, 255, 255]));
}

#[test]
fn a_titles_outline_is_its_ink() {
    let s = spec_with(white("短视频 Hi", 64.0));
    let b = ink_box(&draw(&s, 1.0)).map(|v| v as f32);
    let outlines = layer_outlines(&s, Tick::ZERO, &NoMedia);
    let [tl, _, br, _] = outlines[0].1;
    for (got, want) in [(tl[0], b[0]), (tl[1], b[1]), (br[0], b[2]), (br[1], b[3])] {
        assert!((got - want).abs() <= 2.0, "outline {:?} ink {b:?}", outlines[0].1);
    }
}

#[test]
fn bold_lays_down_more_ink_than_regular() {
    let mut t = white("剪辑", 80.0);
    let regular = ink(&draw(&spec_with(t.clone()), 1.0));
    t.weight = 700;
    let bold = ink(&draw(&spec_with(t), 1.0));
    assert!(bold > regular * 1.2, "bold {bold} vs regular {regular}");
}

#[test]
fn preview_scale_scales_the_text() {
    let s = spec_with(white("预览 Preview", 48.0));
    let full = ink(&draw(&s, 1.0));
    let half = ink(&draw(&s, 0.5));
    let ratio = half / full;
    assert!((0.2..0.3).contains(&ratio), "a quarter of the area: {ratio}");
}

#[test]
fn outline_and_box_draw_behind_the_fill() {
    let mut t = white("字幕", 72.0);
    t.stroke = Some(TextStroke { color: Color([1.0, 0.0, 0.0, 1.0]), width: 6.0 });
    let outlined = draw(&spec_with(t.clone()), 1.0);
    let plain = draw(&spec_with(white("字幕", 72.0)), 1.0);
    let pure_red = |img: &Image| img.to_srgba8().chunks(4).filter(|p| p == &[255, 0, 0, 255]).count();
    assert!(pure_red(&outlined) > 500, "the outline is red");
    assert_eq!(pure_red(&plain), 0);
    let (o, p) = (ink_box(&outlined), ink_box(&plain));
    assert!(o[0] + 4 <= p[0] && o[2] >= p[2] + 4, "the outline reaches beyond the glyphs");

    t.stroke = None;
    t.background = Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.25, radius: 0.0 });
    let boxed = draw(&spec_with(t), 1.0).to_srgba8();
    let at = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [boxed[i], boxed[i + 1], boxed[i + 2]]
    };
    // just inside the padded box but outside the glyphs: green; far away: the blue background
    assert_eq!(at(p[0] - 6, (p[1] + p[3]) / 2), [0, 255, 0]);
    assert_eq!(at(4, 4), [0, 0, 255]);

    // with a wrap width far wider than the words, the box still hugs them
    let mut t = white("字幕", 72.0);
    t.width = Some(460.0);
    t.background = Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.25, radius: 0.0 });
    let wide = draw(&spec_with(t), 1.0).to_srgba8();
    let green = |x: u32| wide[((H / 2 * W + x) * 4) as usize + 1] == 255 && wide[((H / 2 * W + x) * 4) as usize] == 0;
    assert!(!green(30) && !green(W - 30), "no box far from the words");
}

#[test]
fn wrapping_and_rotation_follow_the_layout() {
    let mut t = white("一二三四五六七八九十", 40.0);
    let one_line = ink_box(&draw(&spec_with(t.clone()), 1.0));
    t.width = Some(220.0);
    let wrapped = ink_box(&draw(&spec_with(t.clone()), 1.0));
    assert!(wrapped[2] - wrapped[0] <= 222 && wrapped[3] - wrapped[1] > (one_line[3] - one_line[1]) * 3 / 2);

    let mut s = spec_with(white("旋转", 80.0));
    let level = ink_box(&draw(&s, 1.0));
    s.layers[0].transform.rotation = Animated::Constant(90.0);
    let turned = ink_box(&draw(&s, 1.0));
    // a quarter turn swaps the box's sides (within anti-aliasing)
    let (lw, lh) = ((level[2] - level[0]) as i32, (level[3] - level[1]) as i32);
    let (tw, th) = ((turned[2] - turned[0]) as i32, (turned[3] - turned[1]) as i32);
    assert!((lw - th).abs() <= 3 && (lh - tw).abs() <= 3, "{lw}x{lh} turned {tw}x{th}");
}

struct Fonts(Arc<[u8]>);

impl MediaSource for Fonts {
    fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        Ok(None)
    }
    fn font(&self, _: &str, _: &Asset) -> Result<Option<Arc<[u8]>>, String> {
        Ok(Some(self.0.clone()))
    }
}

#[test]
fn a_font_asset_replaces_the_default_face() {
    let mut t = white("Mono", 60.0);
    t.font = Some("mono".into());
    let mut s = spec_with(t);
    s.assets.insert(
        "mono".into(),
        Asset {
            kind: AssetKind::Font,
            sha256: "ab".repeat(32),
            uri: "file:///fonts/mono.ttf".into(),
            of: None,
            probe: None,
        },
    );
    let mono = Fonts(filmcraft_text::fonts::JETBRAINS_MONO_REGULAR.into());
    let (with_font, _) = render_frame(&s, Tick::ZERO, 1.0, &mono, Quality::Final).expect("renders");
    let default = draw(&spec_with(white("Mono", 60.0)), 1.0);
    assert_ne!(ink_box(&with_font), ink_box(&default), "a different face sets different glyphs");

    // the font is not reachable: a draft says so and uses the default face, a final refuses
    let (_, report) = render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).expect("draft");
    assert_eq!(report.placeholders.len(), 1);
    assert!(matches!(render_frame(&s, Tick::ZERO, 1.0, &NoMedia, Quality::Final), Err(RenderError::Missing { .. })));
}

#[test]
fn second_outlines_shadows_gradients_and_round_boxes() {
    let px = |img: &Image, x: u32, y: u32| {
        let p = img.pixels[(y * img.width + x) as usize];
        // linear values of the pure colours these tests use are 0 or 1: enough to tell them apart
        p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)[..3].try_into().unwrap_or([0; 3])
    };
    let count = |img: &Image, c: [u8; 3]| img.to_srgba8().chunks(4).filter(|p| p[..3] == c).count();
    let mut t = white("花字", 80.0);
    t.stroke = Some(TextStroke { color: Color([1.0, 0.0, 0.0, 1.0]), width: 4.0 });
    let single = draw(&spec_with(t.clone()), 1.0);
    // a green outline beyond the red one
    t.outer = Some(TextStroke { color: Color([0.0, 1.0, 0.0, 1.0]), width: 6.0 });
    let spec = spec_with(t.clone());
    assert_eq!(spec.required_features().into_iter().collect::<Vec<_>>(), ["text/1", "text/2"]);
    let double = draw(&spec, 1.0);
    assert!(count(&double, [0, 255, 0]) > 500, "the second outline is drawn");
    assert!(count(&double, [255, 0, 0]) > 300, "the first one is still there");
    // everything not the blue background
    let drawn = |img: &Image| {
        let mut b = [u32::MAX, u32::MAX, 0, 0];
        for y in 0..img.height {
            for x in 0..img.width {
                if px(img, x, y) != [0, 0, 255] {
                    b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
                }
            }
        }
        b
    };
    let (s, d) = (drawn(&single), drawn(&double));
    assert!(d[0] + 4 <= s[0] && d[2] >= s[2] + 4, "it reaches beyond the first");

    // a hard black shadow down and right: dark pixels appear below-right of the glyphs only
    let mut t = white("花字", 80.0);
    t.shadow = Some(TextShadow { color: Color([0.0, 0.0, 0.0, 1.0]), offset: [0.1, 0.1], blur: 0.0 });
    let shadowed = draw(&spec_with(t), 1.0);
    let b = ink_box(&shadowed);
    assert!(count(&shadowed, [0, 0, 0]) > 300, "the shadow is drawn");
    assert_eq!(px(&shadowed, b[0] - 2, b[1] - 2), [0, 0, 255], "nothing above-left");

    // a vertical gradient: red at the top of the glyphs, green at the bottom
    let mut t = TextContent::new("一", 120.0, Color([1.0, 0.0, 0.0, 1.0]));
    t.text = "工".into();
    t.gradient = Some(Color([0.0, 1.0, 0.0, 1.0]));
    let faded = draw(&spec_with(t), 1.0);
    let rows: Vec<(u32, [u8; 3])> = (0..H)
        .filter_map(|y| {
            let x = W / 2;
            let c = px(&faded, x, y);
            (c != [0, 0, 255]).then_some((y, c))
        })
        .collect();
    let (top, bottom) = (rows.first().expect("ink").1, rows.last().expect("ink").1);
    assert!(top[0] > top[1] && bottom[1] > bottom[0], "top {top:?} bottom {bottom:?}");

    // a round box leaves its corners to the background
    let mut t = white("花字", 80.0);
    t.background = Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.3, radius: 0.5 });
    let rounded = draw(&spec_with(t.clone()), 1.0);
    t.background.as_mut().expect("set").radius = 0.0;
    let square = draw(&spec_with(t), 1.0);
    assert!(count(&rounded, [0, 255, 0]) + 200 < count(&square, [0, 255, 0]), "the corners are cut");
}
