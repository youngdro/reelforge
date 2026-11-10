//! Pictures made between a video's own frames (剪映's 补帧): what a clip slowed down, or a file of
//! fewer frames a second than the output, is made smooth with.
//!
//! [`blend`] mixes the two frames. [`flow`] finds how the picture moved from the one to the other,
//! cell by cell (block matching on small grey pictures, coarse to fine, each cell starting from
//! what the cells around it found one size up, then to a fraction of a pixel), and [`between`]
//! draws both frames moved along that way to the instant asked for and mixes them. Where nothing
//! moved it is the frame itself, and where nothing could be followed the movement found is that
//! of the cells around, so the worst is a mix like [`blend`]'s.

use rayon::prelude::*;
use reelforge_render::Image;

use crate::steady::Gray;

/// The longer side of the pictures movement is looked for on.
const SIDE: usize = 320;
/// A cell's side on those pictures, in pixels.
const CELL: usize = 8;
/// How far a cell is looked for at the smallest size, in pixels each way.
const REACH: i32 = 4;
/// How many sizes the pictures are looked at: each half the one before.
const SIZES: usize = 3;
/// What a pixel of movement away from what was expected costs, as a difference in grey: where the
/// picture is flat, or the same all along an edge, the expected movement wins.
const STIFF: f32 = 0.0015;
/// What the two frames show at a point agrees when it differs by less than the first (in linear
/// light, the channel that differs most), and does not from the second on.
const AGREE: (f32, f32) = (0.04, 0.12);
/// Movement shorter than this many pixels is none.
const STILL: f32 = 0.05;

/// The two frames mixed: `f` of the way from `a` to `b`.
pub fn blend(a: &Image, b: &Image, f: f32) -> Image {
    let pixels = a.pixels.par_iter().zip(&b.pixels).map(|(a, b)| mix(*a, *b, f)).collect();
    Image { width: a.width, height: a.height, pixels }
}

fn mix(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * f)
}

/// How a picture moved from one frame to the next: a movement for each cell of a grid over it.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    cols: usize,
    rows: usize,
    /// A cell's side, in the picture's pixels.
    cell: f32,
    /// Where each cell's part of the first frame is in the second, from where it was: across and
    /// down in the picture's pixels, row by row.
    went: Vec<[f32; 2]>,
}

impl Flow {
    /// The movement at a point of the picture (in its pixels): between the cells' centres,
    /// between their movements.
    pub fn at(&self, x: f32, y: f32) -> [f32; 2] {
        let (cx, cy) = (x / self.cell - 0.5, y / self.cell - 0.5);
        let (cx, cy) = (cx.clamp(0.0, (self.cols - 1) as f32), cy.clamp(0.0, (self.rows - 1) as f32));
        let (x0, y0) = (cx.floor() as usize, cy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.cols - 1), (y0 + 1).min(self.rows - 1));
        let (tx, ty) = (cx - x0 as f32, cy - y0 as f32);
        let v = |x: usize, y: usize| self.went[y * self.cols + x];
        let (a, b, c, d) = (v(x0, y0), v(x1, y0), v(x0, y1), v(x1, y1));
        [0, 1].map(|i| {
            let (top, bottom) = (a[i] + (b[i] - a[i]) * tx, c[i] + (d[i] - c[i]) * tx);
            top + (bottom - top) * ty
        })
    }

    /// Whether nothing moved anywhere.
    pub fn is_still(&self) -> bool {
        self.went.iter().all(|v| v[0].abs() < STILL && v[1].abs() < STILL)
    }
}

/// `img` grey and `s` times smaller each way: every pixel the mean of the block it stands for,
/// its light as the eye takes it (the root of its linear light).
fn gray(img: &Image, s: usize) -> Gray {
    let (w, h) = (img.width as usize, img.height as usize);
    let (ow, oh) = ((w / s).max(1), (h / s).max(1));
    let mut px = vec![0.0f32; ow * oh];
    px.par_chunks_mut(ow).enumerate().for_each(|(oy, row)| {
        for (ox, out) in row.iter_mut().enumerate() {
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for y in oy * s..((oy + 1) * s).min(h) {
                for p in &img.pixels[y * w + ox * s..y * w + ((ox + 1) * s).min(w)] {
                    sum += 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                    n += 1.0;
                }
            }
            *out = (sum / n.max(1.0)).max(0.0).sqrt();
        }
    });
    Gray { width: ow, height: oh, px }
}

/// The movements found at one size: whole pixels of that size, a cell of [`CELL`] each.
struct Found {
    cols: usize,
    rows: usize,
    went: Vec<[i32; 2]>,
}

impl Found {
    fn at(&self, col: usize, row: usize) -> [i32; 2] {
        self.went[row.min(self.rows - 1) * self.cols + col.min(self.cols - 1)]
    }
}

/// The mean difference between `a`'s cell at `(x, y)` and `b`'s moved by `(dx, dy)`; nothing when
/// that is not all inside `b`.
fn differs(a: &Gray, b: &Gray, (x, y): (usize, usize), (dx, dy): (i32, i32)) -> Option<f32> {
    let (ox, oy) = (x as i64 + i64::from(dx), y as i64 + i64::from(dy));
    if ox < 0 || oy < 0 || ox as usize + CELL > b.width || oy as usize + CELL > b.height {
        return None;
    }
    let (ox, oy) = (ox as usize, oy as usize);
    let mut sum = 0.0;
    for row in 0..CELL {
        let p = &a.px[(y + row) * a.width + x..][..CELL];
        let q = &b.px[(oy + row) * b.width + ox..][..CELL];
        sum += p.iter().zip(q).map(|(p, q)| (p - q).abs()).sum::<f32>();
    }
    Some(sum / (CELL * CELL) as f32)
}

/// Where each cell of `a` is in `b`. `above` is what was found at half this size: a cell starts
/// from what the cell over it and that one's neighbours found, and looks a pixel around each.
/// Without it every movement up to [`REACH`] is tried.
fn find(a: &Gray, b: &Gray, above: Option<&Found>) -> Found {
    let (cols, rows) = (a.width / CELL, a.height / CELL);
    let cell = |i: usize| -> [i32; 2] {
        let (col, row) = (i % cols, i / cols);
        let at = (col * CELL, row * CELL);
        // (what it costs, where it went)
        let mut best = (f32::MAX, [0, 0]);
        let try_around = |expected: [i32; 2], reach: i32, best: &mut (f32, [i32; 2])| {
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let v = [expected[0] + dx, expected[1] + dy];
                    let Some(d) = differs(a, b, at, (v[0], v[1])) else { continue };
                    let cost = d + STIFF * ((dx.abs() + dy.abs()) as f32);
                    if cost < best.0 {
                        *best = (cost, v);
                    }
                }
            }
        };
        match above {
            None => try_around([0, 0], REACH, &mut best),
            Some(above) => {
                let (pc, pr) = (col / 2, row / 2);
                let around = [(0i64, 0i64), (-1, 0), (1, 0), (0, -1), (0, 1)];
                let mut tried: Vec<[i32; 2]> = Vec::with_capacity(5);
                for (dc, dr) in around {
                    let (c, r) = ((pc as i64 + dc).max(0) as usize, (pr as i64 + dr).max(0) as usize);
                    let expected = above.at(c, r).map(|v| v * 2);
                    if !tried.contains(&expected) {
                        tried.push(expected);
                        try_around(expected, 1, &mut best);
                    }
                }
            }
        }
        best.1
    };
    let went = (0..cols * rows).into_par_iter().map(cell).collect();
    Found { cols, rows, went }
}

/// Where the lowest of three evenly spaced values of a valley lies, from the middle one.
fn valley(before: f32, at: f32, after: f32) -> f32 {
    let bend = before - 2.0 * at + after;
    if bend <= 1e-9 { 0.0 } else { (0.5 * (before - after) / bend).clamp(-0.5, 0.5) }
}

/// The middle of nine values.
fn middle(mut v: [f32; 9]) -> f32 {
    v.sort_by(f32::total_cmp);
    v[4]
}

/// How the picture moved from `a` to `b`, two frames of one size. Pictures too small to tell
/// (under two cells either way at the size they are looked at) are taken not to have moved.
pub fn flow(a: &Image, b: &Image) -> Flow {
    let still = |cell: f32| Flow { cols: 1, rows: 1, cell, went: vec![[0.0, 0.0]] };
    let longer = a.width.max(a.height) as usize;
    if (a.width, a.height) != (b.width, b.height) || longer == 0 {
        return still(longer.max(1) as f32);
    }
    let s = longer.div_ceil(SIDE).max(1);
    // largest first
    let mut sizes = vec![(gray(a, s), gray(b, s))];
    while sizes.len() < SIZES {
        let (pa, pb) = sizes.last().expect("one at least");
        if pa.width.min(pa.height) / 2 < 3 * CELL {
            break;
        }
        sizes.push((pa.half(), pb.half()));
    }
    let (ga, gb) = &sizes[0];
    if ga.width < 2 * CELL || ga.height < 2 * CELL {
        return still(longer as f32);
    }
    let mut found: Option<Found> = None;
    for (pa, pb) in sizes.iter().rev() {
        found = Some(find(pa, pb, found.as_ref()));
    }
    let found = found.expect("one size at least");
    // to a fraction of a pixel, by how much worse it is a pixel to either side
    let (cols, rows) = (found.cols, found.rows);
    let fine: Vec<[f32; 2]> = (0..cols * rows)
        .map(|i| {
            let at = ((i % cols) * CELL, (i / cols) * CELL);
            let [x, y] = found.went[i];
            let d = |dx: i32, dy: i32| differs(ga, gb, at, (x + dx, y + dy));
            let Some(here) = d(0, 0) else { return [x as f32, y as f32] };
            let part = |before: Option<f32>, after: Option<f32>| match (before, after) {
                (Some(before), Some(after)) => valley(before, here, after),
                _ => 0.0,
            };
            [x as f32 + part(d(-1, 0), d(1, 0)), y as f32 + part(d(0, -1), d(0, 1))]
        })
        .collect();
    // a cell that went its own way among cells that agree goes with them
    let near = |col: usize, row: usize, k: usize| -> [f32; 9] {
        let mut out = [0.0; 9];
        for (n, slot) in out.iter_mut().enumerate() {
            let c = (col + n % 3).saturating_sub(1).min(cols - 1);
            let r = (row + n / 3).saturating_sub(1).min(rows - 1);
            *slot = fine[r * cols + c][k];
        }
        out
    };
    let scale = s as f32;
    let went = (0..cols * rows)
        .map(|i| {
            let v = [0, 1].map(|k| middle(near(i % cols, i / cols, k)));
            if v[0].abs() < STILL && v[1].abs() < STILL { [0.0, 0.0] } else { [v[0] * scale, v[1] * scale] }
        })
        .collect();
    Flow { cols, rows, cell: (CELL * s) as f32, went }
}

/// The pixel of `img` at a continuous position (pixel centres at `n + 0.5`), the edge's beyond it.
fn sample(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width as usize, img.height as usize);
    let (fx, fy) = ((x - 0.5).clamp(0.0, (w - 1) as f32), (y - 0.5).clamp(0.0, (h - 1) as f32));
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let p = |x: usize, y: usize| img.pixels[y * w + x];
    mix(mix(p(x0, y0), p(x1, y0), tx), mix(p(x0, y1), p(x1, y1), tx), ty)
}

/// The picture `f` of the way from frame `a` to frame `b`, whose movement `flow` is
/// ([`flow`]`(a, b)`): each point shows what `a` had where it came from and `b` has where it is
/// going, mixed by how far along it is. Where the two differ, the nearer frame's is shown.
pub fn between(a: &Image, b: &Image, flow: &Flow, f: f32) -> Image {
    if (a.width, a.height) != (b.width, b.height) || flow.is_still() {
        return if (a.width, a.height) == (b.width, b.height) { blend(a, b, f) } else { a.clone() };
    }
    let w = a.width as usize;
    let mut pixels = vec![[0.0f32; 4]; a.pixels.len()];
    pixels.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let py = y as f32 + 0.5;
        for (x, out) in row.iter_mut().enumerate() {
            let px = x as f32 + 0.5;
            // what passes here now set out from further back: the movement there, not here
            let v = flow.at(px, py);
            let v = flow.at(px - f * v[0], py - f * v[1]);
            let (from, to) = if v == [0.0, 0.0] {
                (a.pixels[y * w + x], b.pixels[y * w + x])
            } else {
                (sample(a, px - f * v[0], py - f * v[1]), sample(b, px + (1.0 - f) * v[0], py + (1.0 - f) * v[1]))
            };
            // where the two do not show the same thing (something was not followed, or came out
            // from behind something else), mixing them would show it twice: the nearer frame's
            let apart = (0..3).map(|i| (from[i] - to[i]).abs()).fold(0.0f32, f32::max);
            let lean = ((apart - AGREE.0) / (AGREE.1 - AGREE.0)).clamp(0.0, 1.0);
            let nearer = if f < 0.5 { 0.0 } else { 1.0 };
            *out = mix(from, to, f + (nearer - f) * lean);
        }
    });
    Image { width: a.width, height: a.height, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backdrop with something to follow everywhere, and a square of another pattern over it
    /// whose corner is at `square`.
    fn scene(shift: f32, square: f32) -> Image {
        let (w, h) = (320u32, 192u32);
        let hash = |x: i64, y: i64| {
            let n = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
            (n.wrapping_mul(0xC2B2_AE35) >> 24) as f32 / 255.0
        };
        // blobs a few pixels wide, so that half a pixel of movement still shows
        let soft = |x: f32, y: f32, cell: f32, seed: i64| {
            let (cx, cy) = (x / cell, y / cell);
            let (x0, y0) = (cx.floor(), cy.floor());
            let (tx, ty) = (cx - x0, cy - y0);
            let v = |dx: i64, dy: i64| hash(x0 as i64 + dx + seed, y0 as i64 + dy + 7 * seed);
            let (top, bottom) = (v(0, 0) + (v(1, 0) - v(0, 0)) * tx, v(0, 1) + (v(1, 1) - v(0, 1)) * tx);
            top + (bottom - top) * ty
        };
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let inside = (square..square + 64.0).contains(&fx) && (64.0..128.0).contains(&fy);
                let v = if inside { soft(fx - square, fy, 5.0, 900) } else { soft(fx - shift, fy, 6.0, 0) };
                pixels.push(if inside { [v, 0.2, 1.0 - v, 1.0] } else { [v * 0.8, v, 0.3, 1.0] });
            }
        }
        Image { width: w, height: h, pixels }
    }

    fn apart(a: &Image, b: &Image) -> f32 {
        let sum: f32 = a.pixels.iter().zip(&b.pixels).map(|(p, q)| (p[0] - q[0]).abs() + (p[1] - q[1]).abs()).sum();
        sum / a.pixels.len() as f32
    }

    #[test]
    fn a_frame_in_between_shows_things_part_of_the_way_where_a_mix_shows_them_twice() {
        // the backdrop goes 6 pixels right and the square 14 the other way from one frame to the next
        let (a, b, half) = (scene(0.0, 200.0), scene(6.0, 186.0), scene(3.0, 193.0));
        let found = flow(&a, &b);
        // the backdrop's movement is found away from the square, and the square's inside it
        let (back, front) = (found.at(40.0, 30.0), found.at(225.0, 96.0));
        assert!((back[0] - 6.0).abs() < 0.3 && back[1].abs() < 0.3, "{back:?}");
        assert!((front[0] + 14.0).abs() < 0.5 && front[1].abs() < 0.5, "{front:?}");
        let made = between(&a, &b, &found, 0.5);
        let mixed = blend(&a, &b, 0.5);
        let (made, mixed) = (apart(&made, &half), apart(&mixed, &half));
        assert!(made < mixed / 4.0, "{made} against {mixed}");
        // at either end it is the frame itself, or as good as
        assert!(apart(&between(&a, &b, &found, 0.0), &a) < 1e-6);
        assert!(apart(&between(&a, &b, &found, 1.0), &b) < 1e-6);
        // a third of the way, a third of the way
        let third = apart(&between(&a, &b, &found, 1.0 / 3.0), &scene(2.0, 200.0 - 14.0 / 3.0));
        assert!(third < mixed / 4.0, "{third}");
    }

    #[test]
    fn nothing_moving_is_left_as_it_is_and_a_mix_is_a_mix() {
        let a = scene(0.0, 200.0);
        let found = flow(&a, &a);
        assert!(found.is_still());
        assert_eq!(between(&a, &a, &found, 0.4), a);
        // a flat picture growing lighter: nothing to follow, so the two are mixed
        let flat = |v: f32| Image::filled(64, 48, [v, v, v, 1.0]);
        let found = flow(&flat(0.2), &flat(0.6));
        assert!(found.is_still());
        let made = between(&flat(0.2), &flat(0.6), &found, 0.25);
        assert!((made.pixels[100][0] - 0.3).abs() < 1e-6);
        assert_eq!(blend(&flat(0.2), &flat(0.6), 0.25).pixels[7], made.pixels[7]);
        // frames of two sizes, or too small to look at: not followed, and nothing breaks
        assert!(flow(&flat(0.2), &Image::filled(32, 48, [0.0; 4])).is_still());
        let tiny = Image::filled(6, 4, [0.5; 4]);
        assert_eq!(between(&tiny, &tiny, &flow(&tiny, &tiny), 0.5), tiny);
    }
}
