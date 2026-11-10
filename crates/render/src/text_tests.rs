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
        crop: None,
        flip_x: false,
        flip_y: false,
        shape_mask: None,
        crop_keys: None,
        shape_mask_keys: None,
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
    t.background =
        Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.25, radius: 0.0, lines: false });
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
    t.background =
        Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.25, radius: 0.0, lines: false });
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

    // what the font has no glyph for (it has no Chinese) is set in the bundled face, as it would
    // be without the font, not as empty boxes
    let set = |text: &str, font: bool| {
        let mut s = s.clone();
        let LayerContent::Text(t) = &mut s.layers[0].content else { unreachable!() };
        t.text = text.into();
        if !font {
            t.font = None;
        }
        render_frame(&s, Tick::ZERO, 1.0, &mono, Quality::Final).expect("renders").0
    };
    let (mixed, plain) = (set("中文", true), set("中文", false));
    let (m, p) = (ink_box(&mixed), ink_box(&plain));
    assert_eq!((m[0], m[2]), (p[0], p[2]), "the same glyphs, side to side");
    assert!((ink(&mixed) - ink(&plain)).abs() < ink(&plain) * 0.01, "{} {}", ink(&mixed), ink(&plain));
    // and beside the font's own letters, which stay in the font
    let both = ink_box(&set("Mo中文", true));
    assert!(both[2] - both[0] > m[2] - m[0] + 40, "{both:?} {m:?}");

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
    t.background = Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.3, radius: 0.5, lines: false });
    let rounded = draw(&spec_with(t.clone()), 1.0);
    t.background.as_mut().expect("set").radius = 0.0;
    let square = draw(&spec_with(t), 1.0);
    assert!(count(&rounded, [0, 255, 0]) + 200 < count(&square, [0, 255, 0]), "the corners are cut");
}

#[test]
fn a_typewriter_shows_the_first_characters_where_the_whole_text_puts_them() {
    let mut t = white("ABCDEFGH", 60.0);
    let whole = draw(&spec_with(t.clone()), 1.0);
    t.reveal = Some(Animated::Constant(0.5));
    let half = draw(&spec_with(t.clone()), 1.0);
    let (w, h) = (ink_box(&whole), ink_box(&half));
    // the first four letters, not moved: the same left edge, ending about half way
    assert_eq!(w[0], h[0]);
    assert!(h[2] < (w[0] + w[2]) / 2 + 10 && h[2] > w[0] + (w[2] - w[0]) / 3, "{w:?} {h:?}");
    assert!(ink(&half) < ink(&whole) * 0.6);
    // it moves with time: keys from nothing to all of it
    t.reveal = Some(Animated::Keys(vec![
        Key { t: Tick::ZERO, v: 0.0, ease: Ease::Linear },
        Key { t: Tick::from_seconds(0.5), v: 1.0, ease: Ease::Linear },
    ]));
    let s = spec_with(t);
    assert!(s.required_features().contains("text/3"));
    let at = |t: Tick| ink(&render_frame(&s, t, 1.0, &NoMedia, Quality::Final).unwrap().0);
    assert_eq!(at(Tick::ZERO), 0.0, "nothing typed yet");
    assert!(at(Tick::from_seconds(0.25)) > 0.0 && at(Tick::from_seconds(0.25)) < at(Tick::from_seconds(0.75)));
    assert_eq!(at(Tick::from_seconds(0.75)), ink(&whole));
}

#[test]
fn line_boxes_hug_each_line() {
    let green_at = |img: &[u8], x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        img[i] == 0 && img[i + 1] == 255
    };
    let mut t = white("WWWWWWWW\nW", 40.0);
    t.background =
        Some(TextBackground { color: Color([0.0, 1.0, 0.0, 1.0]), padding: 0.25, radius: 0.0, lines: false });
    let block = draw(&spec_with(t.clone()), 1.0);
    let b = ink_box(&block);
    t.background.as_mut().unwrap().lines = true;
    let s = spec_with(t.clone());
    assert!(s.required_features().contains("text/3"));
    let lines = draw(&s, 1.0);
    // beside the short second line: the block's box covers it, the line's does not
    let (x, y) = (b[0] + 8, b[3] - 10);
    assert!(green_at(&block.to_srgba8(), x, y));
    assert!(!green_at(&lines.to_srgba8(), x, y));
    // the long first line is boxed either way
    assert!(green_at(&lines.to_srgba8(), b[0] - 4, b[1] + 14));

    // typing in, a line's box grows with what shows of it
    t.reveal = Some(Animated::Constant(0.3));
    let typing = draw(&spec_with(t), 1.0).to_srgba8();
    assert!(green_at(&typing, b[0] - 4, b[1] + 14), "the first line has begun");
    assert!(!green_at(&typing, b[2] - 4, b[1] + 14), "but not reached its end");
}

#[test]
fn spacing_slant_and_underline_change_where_the_ink_lands() {
    let plain = white("ABCDEF\nABCDEF", 40.0);
    assert!(!spec_with(plain.clone()).required_features().contains("text/4"));
    let base = ink_box(&draw(&spec_with(plain.clone()), 1.0));
    let (bw, bh) = (base[2] - base[0], base[3] - base[1]);

    // tracking: 0.25 em (10 px) more after each letter, the block still centred
    let mut t = plain.clone();
    t.tracking = 0.25;
    let s = spec_with(t);
    assert!(s.required_features().contains("text/4"));
    let wide = ink_box(&draw(&s, 1.0));
    let grew = (wide[2] - wide[0]) as i32 - bw as i32;
    assert!((45..=65).contains(&grew), "five gaps of 10 px, got {grew}");
    assert_eq!(wide[3] - wide[1], bh, "lines stay where they were");
    // and negative tracking pulls them together
    let mut t = plain.clone();
    t.tracking = -0.05;
    let tight = ink_box(&draw(&spec_with(t), 1.0));
    assert!(tight[2] - tight[0] < bw);

    // leading: half an em (20 px) more between the two lines
    let mut t = plain.clone();
    t.leading = 0.5;
    let tall = ink_box(&draw(&spec_with(t), 1.0));
    let grew = (tall[3] - tall[1]) as i32 - bh as i32;
    assert!((18..=22).contains(&grew), "one gap of 20 px, got {grew}");
    assert_eq!(tall[2] - tall[0], bw);

    // italic: the bundled face has none, so the letters are slanted: the same height, wider ink
    let one = white("HHHH", 60.0);
    let upright = ink_box(&draw(&spec_with(one.clone()), 1.0));
    let mut t = one.clone();
    t.italic = true;
    let slanted = ink_box(&draw(&spec_with(t), 1.0));
    assert_eq!(slanted[3] - slanted[1], upright[3] - upright[1]);
    assert!(slanted[2] - slanted[0] > upright[2] - upright[0] + 4, "{upright:?} {slanted:?}");

    // underline: a rule below the letters, as wide as the line
    let mut t = one.clone();
    t.underline = true;
    let ruled = draw(&spec_with(t.clone()), 1.0);
    let r = ink_box(&ruled);
    assert!(r[3] > upright[3] + 2, "{upright:?} {r:?}");
    let row = r[3] - 2;
    let inked = (0..W).filter(|x| ruled.pixels[(row * W + x) as usize][0] > 0.5).count() as u32;
    assert!(inked + 6 >= upright[2] - upright[0], "{inked} of {}", upright[2] - upright[0]);
    // typed in half way, the rule stops with the letters
    t.reveal = Some(Animated::Constant(0.5));
    let typing = draw(&spec_with(t), 1.0);
    let inked = (0..W).filter(|x| typing.pixels[(row * W + x) as usize][0] > 0.5).count() as u32;
    assert!(inked > 20 && inked < (upright[2] - upright[0]) * 2 / 3, "{inked}");
}

#[test]
fn spacing_out_of_range_is_refused() {
    for (tracking, leading) in [(-0.6, 0.0), (4.5, 0.0), (0.0, -1.0), (0.0, f32::NAN)] {
        let mut t = white("A", 40.0);
        t.tracking = tracking;
        t.leading = leading;
        assert!(reelforge_renderspec::validate(&spec_with(t)).is_err(), "{tracking} {leading}");
    }
}
