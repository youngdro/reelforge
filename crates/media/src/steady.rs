//! Watching how the camera moved, to steady a shot.
//!
//! Each frame is looked at small and grey. Between one frame and the next, patches of the first
//! are found again in the second (block matching, coarse then fine, then to a fraction of a
//! pixel), and one movement of the whole picture is fitted to where the patches went: a shift
//! across and down and a turn about the centre. Patches that went elsewhere (someone walking
//! through the shot) are outvoted. Added up frame after frame, the movements are the camera's
//! path; what smooths it and moves the picture back is `reelforge_model::Steady`.

/// A small grey picture: `width × height` values from 0 (black) to 1 (white), row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Gray {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height` values.
    pub px: Vec<f32>,
}

/// The longer side of the pictures the camera's movement is measured on.
pub const SIDE: u32 = 192;
/// A patch's side, in pixels.
const BLOCK: usize = 16;
/// How far a patch is looked for at half size, in pixels each way.
const REACH: isize = 7;
/// The room a patch needs around it: the farthest it can be found, and one more for the fraction.
const MARGIN: usize = 2 * REACH as usize + 2;
/// Patches flatter than this (the deviation of their grey) cannot be followed.
const FLAT: f32 = 0.012;
/// A patch is found again when what it is matched to differs from it by less than this much of
/// its own deviation; a worse match says nothing of where it went (it left the picture, or the
/// scene changed).
const FOUND: f32 = 0.5;
/// A patch within this many pixels of where the whole picture went has gone with it.
const AGREE: f32 = 1.5;

impl Gray {
    /// The grey of sRGB `[r, g, b, a]` bytes.
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Gray {
        let px = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0)
            .collect();
        Gray { width, height, px }
    }

    /// Half as large each way: every pixel the mean of four.
    fn half(&self) -> Gray {
        let (w, h) = (self.width / 2, self.height / 2);
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let (a, b) = (&self.px[2 * y * self.width..], &self.px[(2 * y + 1) * self.width..]);
            for x in 0..w {
                px.push((a[2 * x] + a[2 * x + 1] + b[2 * x] + b[2 * x + 1]) / 4.0);
            }
        }
        Gray { width: w, height: h, px }
    }

    /// The mean difference between the `n × n` patch of `self` at `(x, y)` and the one of `other`
    /// moved by `(dx, dy)`.
    fn differs(&self, other: &Gray, (x, y): (usize, usize), (dx, dy): (isize, isize), n: usize) -> f32 {
        let (ox, oy) = ((x as isize + dx) as usize, (y as isize + dy) as usize);
        let mut sum = 0.0;
        for row in 0..n {
            let a = &self.px[(y + row) * self.width + x..][..n];
            let b = &other.px[(oy + row) * other.width + ox..][..n];
            sum += a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum::<f32>();
        }
        sum / (n * n) as f32
    }

    /// How much the patch at `(x, y)` varies: the deviation of its grey.
    fn texture(&self, (x, y): (usize, usize), n: usize) -> f32 {
        let (mut sum, mut squares) = (0.0f32, 0.0f32);
        for row in 0..n {
            for v in &self.px[(y + row) * self.width + x..][..n] {
                sum += v;
                squares += v * v;
            }
        }
        let count = (n * n) as f32;
        (squares / count - (sum / count).powi(2)).max(0.0).sqrt()
    }
}

/// Where the lowest of three evenly spaced values of a valley lies, from the middle one: -0.5 to 0.5.
fn valley(before: f32, at: f32, after: f32) -> f32 {
    let bend = before - 2.0 * at + after;
    if bend <= 1e-9 { 0.0 } else { (0.5 * (before - after) / bend).clamp(-0.5, 0.5) }
}

/// One patch followed from a frame to the next: where it was (from the centre of the picture),
/// how far it went, and how well it could be told from its surroundings.
struct Followed {
    at: [f32; 2],
    went: [f32; 2],
    weight: f32,
}

/// Follows the patches of `a` into `b`.
fn follow(a: &Gray, b: &Gray) -> Vec<Followed> {
    let (w, h) = (a.width, a.height);
    if w < BLOCK + 2 * MARGIN || h < BLOCK + 2 * MARGIN || (b.width, b.height) != (w, h) {
        return vec![];
    }
    let (ah, bh) = (a.half(), b.half());
    // as many patches as fit, spread over the middle of the picture
    let count = |side: usize| (side - 2 * MARGIN) / BLOCK;
    let first = |side: usize| (side - count(side) * BLOCK) / 2;
    let mut out = vec![];
    for row in 0..count(h) {
        for col in 0..count(w) {
            let at = (first(w) + col * BLOCK, first(h) + row * BLOCK);
            let texture = a.texture(at, BLOCK);
            if texture < FLAT {
                continue;
            }
            // roughly, at half size
            let half = (at.0 / 2, at.1 / 2);
            let mut best = (f32::MAX, (0isize, 0isize));
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let d = ah.differs(&bh, half, (dx, dy), BLOCK / 2);
                    if d < best.0 {
                        best = (d, (dx, dy));
                    }
                }
            }
            // to the pixel
            let rough = (best.1.0 * 2, best.1.1 * 2);
            let mut best = (f32::MAX, rough);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let d = (rough.0 + dx, rough.1 + dy);
                    let s = a.differs(b, at, d, BLOCK);
                    if s < best.0 {
                        best = (s, d);
                    }
                }
            }
            let (s, (dx, dy)) = best;
            if s > FOUND * texture {
                continue;
            }
            // and to a fraction of one
            let fx = valley(a.differs(b, at, (dx - 1, dy), BLOCK), s, a.differs(b, at, (dx + 1, dy), BLOCK));
            let fy = valley(a.differs(b, at, (dx, dy - 1), BLOCK), s, a.differs(b, at, (dx, dy + 1), BLOCK));
            out.push(Followed {
                at: [
                    at.0 as f32 + BLOCK as f32 / 2.0 - w as f32 / 2.0,
                    at.1 as f32 + BLOCK as f32 / 2.0 - h as f32 / 2.0,
                ],
                went: [dx as f32 + fx, dy as f32 + fy],
                weight: texture.min(0.1),
            });
        }
    }
    out
}

/// The one movement of the whole picture that best says where the patches went: a shift (pixels)
/// and a turn about the centre (radians, clockwise as seen), each patch counting as `weights` say.
fn fit(patches: &[Followed], weights: &[f32]) -> Option<[f32; 3]> {
    // least squares of  went = shift + turn × (-y, x)
    let (mut n, mut sx, mut sy, mut rr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut su, mut sv, mut cross) = (0.0f64, 0.0f64, 0.0f64);
    for (p, w) in patches.iter().zip(weights) {
        let (w, x, y) = (f64::from(*w), f64::from(p.at[0]), f64::from(p.at[1]));
        let (u, v) = (f64::from(p.went[0]), f64::from(p.went[1]));
        n += w;
        sx += w * x;
        sy += w * y;
        rr += w * (x * x + y * y);
        su += w * u;
        sv += w * v;
        cross += w * (v * x - u * y);
    }
    if n <= 0.0 {
        return None;
    }
    // shift = (su + turn·sy, sv − turn·sx) / n, put into the turn's own equation
    let spread = rr - (sx * sx + sy * sy) / n;
    let turn = if spread > 1e-6 { (cross - (sv * sx - su * sy) / n) / spread } else { 0.0 };
    Some([((su + turn * sy) / n) as f32, ((sv - turn * sx) / n) as f32, turn as f32])
}

/// How the picture moved from frame `a` to frame `b`: across and down as fractions of its width
/// and height (to the right and down are positive), and its turn about its centre in degrees
/// (clockwise as seen is positive). Nothing when the two are not of one scene, or too flat or too
/// small to follow anything in.
pub fn motion(a: &Gray, b: &Gray) -> Option<[f32; 3]> {
    let patches = follow(a, b);
    if patches.len() < 4 {
        return None;
    }
    let mut weights: Vec<f32> = patches.iter().map(|p| p.weight).collect();
    let mut moved = fit(&patches, &weights)?;
    // patches far from where the whole went count for less, then for nothing
    for _ in 0..4 {
        let off: Vec<f32> = patches
            .iter()
            .map(|p| {
                let (u, v) = (moved[0] - moved[2] * p.at[1], moved[1] + moved[2] * p.at[0]);
                (p.went[0] - u).hypot(p.went[1] - v)
            })
            .collect();
        let mut sorted = off.clone();
        sorted.sort_by(f32::total_cmp);
        let far = 3.0 * (1.4826 * sorted[sorted.len() / 2] + 0.05);
        for ((w, p), r) in weights.iter_mut().zip(&patches).zip(&off) {
            *w = if *r < far { p.weight * (1.0 - (r / far).powi(2)).powi(2) } else { 0.0 };
        }
        moved = fit(&patches, &weights).unwrap_or(moved);
    }
    // a picture that moved takes most of its patches with it; when few went where the whole is
    // said to have gone, they went anywhere: another scene
    let with = patches
        .iter()
        .filter(|p| {
            let (u, v) = (moved[0] - moved[2] * p.at[1], moved[1] + moved[2] * p.at[0]);
            (p.went[0] - u).hypot(p.went[1] - v) < AGREE
        })
        .count();
    if with * 3 < patches.len() {
        return None;
    }
    Some([moved[0] / a.width as f32, moved[1] / a.height as f32, moved[2].to_degrees()])
}

/// The camera's path over `frames`: where the picture is at each against the first (the
/// movements from frame to frame, added up; see [`motion`]). Across a cut, and wherever nothing
/// can be followed, the picture counts as not having moved.
pub fn path(frames: impl IntoIterator<Item = Gray>) -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = vec![];
    let mut last: Option<Gray> = None;
    for frame in frames {
        let at = match (&last, out.last()) {
            (Some(before), Some(at)) => {
                let m = motion(before, &frame).unwrap_or([0.0; 3]);
                [at[0] + m[0], at[1] + m[1], at[2] + m[2]]
            }
            _ => [0.0; 3],
        };
        out.push(at);
        last = Some(frame);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with something to follow everywhere, seen moved by `(dx, dy)` pixels and turned
    /// by `turn` radians about its centre.
    fn scene(w: usize, h: usize, dx: f32, dy: f32, turn: f32) -> Gray {
        let (sin, cos) = turn.sin_cos();
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                // where this pixel was before the picture moved
                let (cx, cy) = (x as f32 - w as f32 / 2.0 - dx, y as f32 - h as f32 / 2.0 - dy);
                let (u, v) = (cos * cx + sin * cy, -sin * cx + cos * cy);
                let grey = 0.5
                    + 0.2 * (u * 0.21).sin() * (v * 0.17).cos()
                    + 0.15 * (u * 0.071 + v * 0.093).sin()
                    + 0.1 * (u * 0.33 - v * 0.29).cos();
                px.push(grey.clamp(0.0, 1.0));
            }
        }
        Gray { width: w, height: h, px }
    }

    #[test]
    fn a_shift_and_a_turn_are_measured_to_a_fraction_of_a_pixel() {
        let (w, h) = (192, 108);
        let still = scene(w, h, 0.0, 0.0, 0.0);
        for (dx, dy, turn) in [(3.4f32, -2.2f32, 0.0f32), (-6.0, 5.5, 0.0), (0.0, 0.0, 0.02), (2.5, 1.0, -0.015)] {
            let m = motion(&still, &scene(w, h, dx, dy, turn)).expect("something to follow");
            assert!((m[0] * w as f32 - dx).abs() < 0.15, "{dx} {dy} {turn}: {m:?}");
            assert!((m[1] * h as f32 - dy).abs() < 0.15, "{dx} {dy} {turn}: {m:?}");
            assert!((m[2].to_radians() - turn).abs() < 0.002, "{dx} {dy} {turn}: {m:?}");
        }
    }

    #[test]
    fn something_crossing_the_shot_is_outvoted_and_a_flat_picture_says_nothing() {
        let (w, h) = (192, 108);
        let a = scene(w, h, 0.0, 0.0, 0.0);
        let mut b = scene(w, h, 2.0, 1.0, 0.0);
        // a quarter of the picture goes its own way
        let other = scene(w, h, -9.0, 4.0, 0.0);
        for y in 20..80 {
            for x in 30..90 {
                b.px[y * w + x] = other.px[y * w + x];
            }
        }
        let m = motion(&a, &b).unwrap();
        assert!((m[0] * w as f32 - 2.0).abs() < 0.2 && (m[1] * h as f32 - 1.0).abs() < 0.2, "{m:?}");

        let flat = Gray { width: w, height: h, px: vec![0.5; w * h] };
        assert_eq!(motion(&flat, &flat), None);
        // a cut: nothing of the one scene is in the other, and its patches turn up anywhere
        let blotch = |x: usize, y: usize| {
            let n = (x as u32 / 6).wrapping_mul(0x9E37_79B1) ^ (y as u32 / 6).wrapping_mul(0x85EB_CA77);
            ((n ^ (n >> 13)).wrapping_mul(0xC2B2_AE35) >> 24) as f32 / 255.0
        };
        let elsewhere = Gray { width: w, height: h, px: (0..w * h).map(|i| blotch(i % w, i / w)).collect() };
        assert_eq!(motion(&a, &elsewhere), None);
        // the path adds the movements up and holds still where it cannot tell
        let walked = path([a.clone(), scene(w, h, 2.0, 0.0, 0.0), scene(w, h, 5.0, -1.0, 0.0), flat]);
        assert_eq!(walked.len(), 4);
        assert!((walked[2][0] * w as f32 - 5.0).abs() < 0.3 && (walked[2][1] * h as f32 + 1.0).abs() < 0.3);
        assert_eq!(walked[3], walked[2]);
    }
}
