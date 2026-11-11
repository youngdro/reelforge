//! What letters wear with `text/8`, as pixels: fills through several colours, a colour per
//! character, depth, more shadows, hollow letters, lit edges, letters out of line and round a
//! curve, and the bands, tiles, borders and leaning boxes behind them.

use reelforge_renderspec::*;

use crate::text_tests::{H, W, draw, ink, ink_box, spec_with, white};
use crate::*;

const RED: Color = Color([1.0, 0.0, 0.0, 1.0]);
const GREEN: Color = Color([0.0, 1.0, 0.0, 1.0]);

/// A pixel's red, green and blue as 0 or 255: the pure colours these tests use are 0 or 1.
fn px(img: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = img.pixels[(y * img.width + x) as usize];
    [p[0], p[1], p[2]].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// How many pixels are exactly this colour.
fn count(img: &Image, c: [u8; 3]) -> usize {
    (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| px(img, x, y) == c).count()
}

/// The box of the pixels that are exactly this colour.
fn box_of(img: &Image, c: [u8; 3]) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..img.height {
        for x in 0..img.width {
            if px(img, x, y) == c {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    b
}

/// The red and the green in a part of the picture.
fn red_green(img: &Image, [x0, y0, x1, y1]: [u32; 4]) -> (f32, f32) {
    let mut sum = (0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.pixels[(y * img.width + x) as usize];
            sum = (sum.0 + p[0], sum.1 + p[1]);
        }
    }
    sum
}

#[test]
fn a_fill_runs_through_its_colours_at_its_angle_smoothly_or_in_bands() {
    let mut t = white("田田田", 100.0);
    t.fill = Some(TextFill { colors: vec![RED, GREEN], angle: 0.0, lines: false, bands: false, repeat: 1.0 });
    let spec = spec_with(t.clone());
    assert!(spec.required_features().contains("text/8"));
    reelforge_renderspec::validate(&spec).expect("a fill is sound");
    let across = draw(&spec, 1.0);
    let b = ink_box(&draw(&spec_with(white("田田田", 100.0)), 1.0));
    let third = (b[2] - b[0]) / 3;
    // red at the left, green at the right, both in the middle
    let (lr, lg) = red_green(&across, [b[0], b[1], b[0] + third, b[3]]);
    let (rr, rg) = red_green(&across, [b[2] - third, b[1], b[2], b[3]]);
    assert!(lr > 3.0 * lg && rg > 3.0 * rr, "left {lr} {lg}, right {rr} {rg}");
    let mixed = |img: &Image| img.pixels.iter().filter(|p| p[0] > 0.15 && p[1] > 0.15).count();
    assert!(mixed(&across) > 500, "one colour runs into the next");

    // turned a quarter: red at the top, green at the bottom
    t.fill.as_mut().expect("set").angle = 90.0;
    let down = draw(&spec_with(t.clone()), 1.0);
    let quarter = (b[3] - b[1]) / 4;
    let (tr, tg) = red_green(&down, [b[0], b[1], b[2], b[1] + quarter]);
    let (br, bg) = red_green(&down, [b[0], b[3] - quarter, b[2], b[3]]);
    assert!(tr > 3.0 * tg && bg > 3.0 * br, "top {tr} {tg}, bottom {br} {bg}");

    // bands: every inked pixel is one colour or the other, and they go round as often as told
    let fill = t.fill.as_mut().expect("set");
    (fill.bands, fill.repeat) = (true, 3.0);
    let banded = draw(&spec_with(t), 1.0);
    assert_eq!(mixed(&banded), 0, "bands have hard edges");
    let x = (b[0]..b[2]).find(|&x| (b[1]..b[3]).filter(|&y| px(&banded, x, y) != [0, 0, 255]).count() > 60);
    // inside the stem, past its soft edge
    let x = x.expect("a stem that runs the height of the glyph") + 4;
    let mut turns = 0;
    let mut last = None;
    for y in b[1]..b[3] {
        let c = px(&banded, x, y);
        if (c == [255, 0, 0] || c == [0, 255, 0]) && last != Some(c) {
            turns += 1;
            last = Some(c);
        }
    }
    assert_eq!(turns, 6, "red, green, three times over");
}

#[test]
fn a_palette_gives_each_character_or_word_its_colour() {
    let mut t = white("田田田 田", 80.0);
    t.palette = Some(TextPalette { colors: vec![RED, GREEN], unit: TextUnit::Char });
    let by_char = draw(&spec_with(t.clone()), 1.0);
    let b = ink_box(&draw(&spec_with(white("田田田 田", 80.0)), 1.0));
    // the runs of ink along the line, each in one colour
    let runs = |img: &Image| {
        let mut out: Vec<[u8; 3]> = vec![];
        let mut run: Option<(f32, f32)> = None;
        for x in b[0].saturating_sub(2)..b[2] + 3 {
            let (r, g) = red_green(img, [x, b[1], x + 1, b[3]]);
            match (&mut run, r + g > 0.5) {
                (Some(sum), true) => *sum = (sum.0 + r, sum.1 + g),
                (None, true) => run = Some((r, g)),
                (Some((r, g)), false) => {
                    assert!(*r > 20.0 * *g || *g > 20.0 * *r, "one colour to a character: {r} {g}");
                    out.push(if r > g { [255, 0, 0] } else { [0, 255, 0] });
                    run = None;
                }
                (None, false) => {}
            }
        }
        out
    };
    assert_eq!(runs(&by_char), [[255, 0, 0], [0, 255, 0], [255, 0, 0], [0, 255, 0]]);
    // by word: a Chinese character is a word, so the same; by line, all one colour
    t.palette.as_mut().expect("set").unit = TextUnit::Line;
    assert_eq!(runs(&draw(&spec_with(t), 1.0)), [[255, 0, 0]; 4]);
}

#[test]
fn depth_and_more_shadows_reach_out_on_their_sides() {
    let plain = ink_box(&draw(&spec_with(white("田", 100.0)), 1.0));
    let mut t = white("田", 100.0);
    t.depth = Some(TextDepth { color: RED, offset: [0.2, 0.1], fade: None });
    let deep = draw(&spec_with(t.clone()), 1.0);
    let d = box_of(&deep, [255, 0, 0]);
    assert!(count(&deep, [255, 0, 0]) > 1500, "the block is drawn");
    // out to the right and down by its offset, and nowhere above or left of the glyph
    assert!((d[2] as i32 - plain[2] as i32 - 20).abs() <= 2 && (d[3] as i32 - plain[3] as i32 - 10).abs() <= 2);
    assert!(d[0] > plain[0] && d[1] > plain[1], "{d:?} {plain:?}");
    // the glyph itself is still whole and white on top
    assert_eq!(count(&deep, [255, 255, 255]), count(&draw(&spec_with(white("田", 100.0)), 1.0), [255, 255, 255]));
    // a block that changes colour on the way: green at its far end
    t.depth.as_mut().expect("set").fade = Some(GREEN);
    let faded = draw(&spec_with(t), 1.0);
    let far = px(&faded, d[2] - 2, d[3] - 30);
    assert!(far[1] > 200 && far[0] < 60, "{far:?}");

    // two more shadows beneath the first: one hard to the left, one hard above
    let mut t = white("田", 100.0);
    t.shadows = vec![
        TextShadow { color: RED, offset: [-0.2, 0.0], blur: 0.0 },
        TextShadow { color: GREEN, offset: [0.0, -0.2], blur: 0.0 },
    ];
    let img = draw(&spec_with(t), 1.0);
    let (r, g) = (box_of(&img, [255, 0, 0]), box_of(&img, [0, 255, 0]));
    assert!((plain[0] as i32 - r[0] as i32 - 20).abs() <= 1 && r[1] >= plain[1], "{r:?}");
    assert!((plain[1] as i32 - g[1] as i32 - 20).abs() <= 1 && g[0] >= plain[0], "{g:?}");
}

#[test]
fn hollow_letters_keep_their_edges_and_a_bevel_lights_one_side() {
    let plain = draw(&spec_with(white("工", 160.0)), 1.0);
    let mut t = white("工", 160.0);
    t.hollow = Some(0.02);
    let hollow = draw(&spec_with(t), 1.0);
    assert_eq!(ink_box(&hollow), ink_box(&plain), "the edges are where they were");
    assert!(ink(&hollow) < 0.6 * ink(&plain) && ink(&hollow) > 0.1 * ink(&plain));
    // the middle of the top bar is empty now
    let b = ink_box(&plain);
    let mid = (b[0] + b[2]) / 2;
    let top_bar = (b[1]..b[3]).filter(|&y| px(&plain, mid, y) == [255, 255, 255]).take(4).last().expect("ink");
    assert_eq!(px(&hollow, mid, top_bar + 3), [0, 0, 255]);

    // lit from the top left: the top edges turn red, the bottom edges green
    let mut t = white("工", 160.0);
    t.bevel = Some(TextBevel { light: RED, dark: GREEN, size: 0.03, angle: 315.0 });
    let lit = draw(&spec_with(t.clone()), 1.0);
    let whole = [b[0], b[1], b[2], b[3]];
    let (r, g) = red_green(&lit, whole);
    let (pr, pg) = red_green(&plain, whole);
    // red where the light falls means less green there, and the other way round
    assert!(r < pr - 100.0 && g < pg - 100.0, "both edges are tinted: {r} {pr} {g} {pg}");
    let (top, bottom) = ([b[0], b[1], b[2], b[1] + 5], [b[0], b[3] - 5, b[2], b[3]]);
    let ((tr, tg), (br, bg)) = (red_green(&lit, top), red_green(&lit, bottom));
    assert!(tr > 1.5 * tg && bg > 1.5 * br, "top {tr} {tg}, bottom {br} {bg}");
    // from the other side it is the other way round
    t.bevel.as_mut().expect("set").angle = 135.0;
    let lit = draw(&spec_with(t), 1.0);
    let ((tr, tg), (br, bg)) = (red_green(&lit, top), red_green(&lit, bottom));
    assert!(tg > 1.5 * tr && br > 1.5 * bg, "top {tr} {tg}, bottom {br} {bg}");
}

#[test]
fn scattered_letters_fall_the_same_way_each_time_and_an_arc_bends_the_line() {
    let plain = draw(&spec_with(white("限时特惠大促", 60.0)), 1.0);
    let mut t = white("限时特惠大促", 60.0);
    t.scatter = Some(TextScatter { rotate: 12.0, shift: 0.15, scale: 0.2, seed: 1 });
    let once = draw(&spec_with(t.clone()), 1.0);
    let again = draw(&spec_with(t.clone()), 1.0);
    assert!(once.pixels == again.pixels, "chance falls the same way every time");
    assert!(once.pixels != plain.pixels, "the letters are out of line");
    t.scatter.as_mut().expect("set").seed = 2;
    assert!(draw(&spec_with(t), 1.0).pixels != once.pixels, "another seed, another arrangement");
    // about as much ink, about where it was
    assert!((ink(&once) / ink(&plain) - 1.0).abs() < 0.25);

    // the topmost ink at the line's left end, its middle and its right end
    let tops = |img: &Image| {
        let b = ink_box(img);
        let w = (b[2] - b[0]) / 6;
        [b[0], (b[0] + b[2] - w) / 2, b[2] - w]
            .map(|from| (0..H).find(|&y| (from..from + w).any(|x| px(img, x, y) != [0, 0, 255])).expect("ink"))
    };
    let mut t = white("限时特惠大促", 60.0);
    t.arc = 90.0;
    let up = tops(&draw(&spec_with(t.clone()), 1.0));
    assert!(up[1] + 15 < up[0] && up[1] + 15 < up[2], "an arch: its middle is highest {up:?}");
    t.arc = -90.0;
    let down = tops(&draw(&spec_with(t), 1.0));
    assert!(down[1] > down[0] + 15 && down[1] > down[2] + 15, "a smile: its middle is lowest {down:?}");
    let flat = tops(&plain);
    assert!(flat[0].abs_diff(flat[1]) <= 3 && flat[1].abs_diff(flat[2]) <= 3);
}

#[test]
fn bands_tiles_borders_and_leaning_boxes_stand_behind_the_words() {
    let words = ink_box(&draw(&spec_with(white("田田田", 80.0)), 1.0));
    let with = |shape: TextBacking| {
        let mut t = white("田田田", 80.0);
        t.background = Some(TextBackground { shape, ..TextBackground::of(RED) });
        t
    };
    let boxed = box_of(&draw(&spec_with(with(TextBacking::Box)), 1.0), [255, 0, 0]);
    // a highlighter's band: the lower half of the line, as wide as the box
    let band = box_of(&draw(&spec_with(with(TextBacking::Marker)), 1.0), [255, 0, 0]);
    assert!(band[1] > words[1] + 20 && band[3] >= words[3], "{band:?} {words:?}");
    assert!(band[3] - band[1] < (boxed[3] - boxed[1]) * 2 / 3);
    assert_eq!((band[0], band[2]), (boxed[0], boxed[2]));

    // tiles: one behind each character, apart from the next
    let tiles = draw(&spec_with(with(TextBacking::Tiles)), 1.0);
    let t = box_of(&tiles, [255, 0, 0]);
    let y = t[1] + 2;
    let mut runs = 0;
    let mut on = false;
    for x in 0..W {
        let red = px(&tiles, x, y) == [255, 0, 0];
        runs += usize::from(red && !on);
        on = red;
    }
    assert_eq!(runs, 3);

    // a green border just inside the box's edge
    let mut t = with(TextBacking::Box);
    t.background.as_mut().expect("set").border = Some(TextStroke { color: GREEN, width: 4.0 });
    let framed = draw(&spec_with(t.clone()), 1.0);
    let mid = (boxed[1] + boxed[3]) / 2;
    assert_eq!(px(&framed, boxed[0] + 1, mid), [0, 255, 0]);
    assert_eq!(px(&framed, boxed[2] - 2, mid), [0, 255, 0]);
    assert_eq!(px(&framed, boxed[0] + 8, mid), [255, 0, 0], "and the box inside it");
    assert_eq!(box_of(&framed, [0, 255, 0]), boxed, "it does not make the box bigger");

    // leaning to the right: the top edge is further right than the bottom edge
    let mut t = with(TextBacking::Box);
    t.background.as_mut().expect("set").skew = 20.0;
    let leaning = draw(&spec_with(t), 1.0);
    let left_at = |y: u32| (0..W).find(|&x| px(&leaning, x, y) == [255, 0, 0]).expect("the box");
    assert!(left_at(boxed[1] + 3) > left_at(boxed[3] - 3) + 20);
    let spec = spec_with(with(TextBacking::Marker));
    assert!(spec.required_features().contains("text/8"));
}

#[test]
fn what_letters_wear_out_of_range_is_refused() {
    let refused = |change: fn(&mut TextContent)| {
        let mut t = white("花字", 60.0);
        change(&mut t);
        reelforge_renderspec::validate(&spec_with(t)).is_err()
    };
    assert!(refused(
        |t| t.fill = Some(TextFill { colors: vec![RED], angle: 0.0, lines: false, bands: false, repeat: 1.0 })
    ));
    assert!(refused(|t| t.palette = Some(TextPalette { colors: vec![], unit: TextUnit::Char })));
    assert!(refused(|t| t.hollow = Some(0.0)));
    assert!(refused(|t| t.arc = 400.0));
    assert!(refused(|t| t.depth = Some(TextDepth { color: RED, offset: [2.0, 0.0], fade: None })));
    assert!(refused(|t| t.shadows = vec![TextShadow { color: RED, offset: [0.0, 0.0], blur: 0.1 }; 5]));
    assert!(refused(|t| t.bevel = Some(TextBevel { light: RED, dark: GREEN, size: 0.0, angle: 0.0 })));
    assert!(refused(|t| t.scatter = Some(TextScatter { rotate: 200.0, shift: 0.0, scale: 0.0, seed: 0 })));
    assert!(!refused(|t| t.arc = 180.0));
}
