//! Coverage of polygons, exact to the pixel's area, and painting through it.

use crate::geom::{Affine, P};

/// How overlapping contours fill.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    NonZero,
    EvenOdd,
}

/// A picture being drawn: premultiplied RGBA in the animation's own (sRGB) values, and the box
/// that has been drawn in so far.
pub(crate) struct Surface {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
    /// `[x0, y0, x1, y1]` of what may not be clear; empty when `x1 <= x0`.
    pub dirty: [usize; 4],
}

impl Surface {
    pub fn new(w: usize, h: usize) -> Surface {
        Surface { w, h, px: vec![[0.0; 4]; w * h], dirty: [w, h, 0, 0] }
    }

    pub fn clear(&mut self) {
        let [x0, y0, x1, y1] = self.dirty;
        for y in y0..y1.min(self.h) {
            self.px[y * self.w + x0..y * self.w + x1.min(self.w)].fill([0.0; 4]);
        }
        self.dirty = [self.w, self.h, 0, 0];
    }

    fn touch(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        self.dirty = [self.dirty[0].min(x0), self.dirty[1].min(y0), self.dirty[2].max(x1), self.dirty[3].max(y1)];
    }

    /// Lays `top` over this, `opacity` of it, where `clip` lets it show.
    pub fn draw(&mut self, top: &Surface, opacity: f32, clip: Option<&Coverage>) {
        let [mut x0, mut y0, mut x1, mut y1] = top.dirty;
        if let Some(c) = clip {
            (x0, y0, x1, y1) = (x0.max(c.x0), y0.max(c.y0), x1.min(c.x0 + c.w), y1.min(c.y0 + c.h));
        }
        if x1 <= x0 || y1 <= y0 || opacity <= 0.0 {
            return;
        }
        self.touch(x0, y0, x1, y1);
        for y in y0..y1 {
            let (from, to) = (&top.px[y * top.w + x0..y * top.w + x1], &mut self.px[y * self.w + x0..y * self.w + x1]);
            let through = clip.map(|c| &c.row(y)[x0 - c.x0..][..x1 - x0]);
            for (x, (s, d)) in from.iter().zip(to).enumerate() {
                let o = opacity * through.map_or(1.0, |t| t[x]);
                let a = s[3] * o;
                if a > 0.0 {
                    let k = 1.0 - a;
                    *d = [s[0] * o + d[0] * k, s[1] * o + d[1] * k, s[2] * o + d[2] * k, a + d[3] * k];
                }
            }
        }
    }

    /// Keeps of each pixel what `by` says, a number from 0 to 1 for each pixel of `matte`.
    pub fn keep(&mut self, matte: &Surface, by: impl Fn([f32; 4]) -> f32) {
        let [x0, y0, x1, y1] = self.dirty;
        for y in y0..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                let k = by(matte.px[y * self.w + x]).clamp(0.0, 1.0);
                let p = &mut self.px[y * self.w + x];
                *p = [p[0] * k, p[1] * k, p[2] * k, p[3] * k];
            }
        }
    }

    /// Straight 8-bit RGBA.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3].clamp(0.0, 1.0);
            if a <= 0.0 {
                out.extend([0, 0, 0, 0]);
                continue;
            }
            if a >= 1.0 {
                out.extend([
                    (p[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    (p[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    (p[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                    255,
                ]);
                continue;
            }
            for c in &p[..3] {
                out.push(((c / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            out.push((a * 255.0 + 0.5) as u8);
        }
        out
    }
}

/// How much of each pixel of a box polygons cover.
pub(crate) struct Coverage {
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    /// How far apart rows are in `data`, at least `w`.
    pub stride: usize,
    /// `h` rows of `w` values from 0 to 1, `stride` apart.
    pub data: Vec<f32>,
}

impl Coverage {
    /// What is left of it where `clip` lets it show; nothing when that is nowhere.
    pub fn clipped(self, clip: Option<&Coverage>) -> Option<Coverage> {
        let Some(c) = clip else { return Some(self) };
        let (x0, y0) = (self.x0.max(c.x0), self.y0.max(c.y0));
        let (x1, y1) = ((self.x0 + self.w).min(c.x0 + c.w), (self.y0 + self.h).min(c.y0 + c.h));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = Vec::with_capacity(w * h);
        for y in y0..y1 {
            let (a, b) = (&self.row(y)[x0 - self.x0..][..w], &c.row(y)[x0 - c.x0..][..w]);
            data.extend(a.iter().zip(b).map(|(p, q)| p * q));
        }
        Some(Coverage { x0, y0, w, h, stride: w, data })
    }

    /// The row of the picture's line `y`, which must be one of its own.
    pub fn row(&self, y: usize) -> &[f32] {
        &self.data[(y - self.y0) * self.stride..][..self.w]
    }

    #[cfg(test)]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.w || y >= self.y0 + self.h {
            0.0
        } else {
            self.row(y)[x - self.x0]
        }
    }
}

struct Accumulator {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Accumulator {
    /// One edge, in the box's own pixels; `x` already within `0..=w`.
    fn edge(&mut self, p0: P, p1: P) {
        if p0[1] == p1[1] {
            return;
        }
        let (dir, p0, p1) = if p0[1] < p1[1] { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1[0] - p0[0]) / (p1[1] - p0[1]);
        let mut x = p0[0];
        if p0[1] < 0.0 {
            x -= p0[1] * dxdy;
        }
        let (first, last) = (p0[1].max(0.0) as usize, (p1[1].ceil().max(0.0) as usize).min(self.h));
        let stride = self.w + 2;
        for y in first..last {
            let row = y * stride;
            let dy = ((y + 1) as f32).min(p1[1]) - (y as f32).max(p0[1]);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0 = x0.clamp(0.0, self.w as f32);
            let x1 = x1.clamp(0.0, self.w as f32);
            let x0floor = x0.floor();
            let x0i = x0floor as usize;
            let x1ceil = x1.ceil();
            let x1i = x1ceil as usize;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x0 + x1) - x0floor;
                self.a[row + x0i] += d - d * xmf;
                self.a[row + x0i + 1] += d * xmf;
            } else {
                let s = (x1 - x0).recip();
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.a[row + x0i] += d * a0;
                if x1i == x0i + 2 {
                    self.a[row + x0i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.a[row + x0i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        self.a[row + xi] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.a[row + x1i - 1] += d * (1.0 - a2 - am);
                }
                self.a[row + x1i] += d * am;
            }
            x = xnext;
        }
    }

    /// An edge anywhere: the parts left and right of the box are drawn along its sides, so
    /// what is inside is covered as if the box were endless.
    fn line(&mut self, p0: P, p1: P) {
        let w = self.w as f32;
        let (lo, hi) = (p0[0].min(p1[0]), p0[0].max(p1[0]));
        if lo >= 0.0 && hi <= w {
            return self.edge(p0, p1);
        }
        if hi <= 0.0 {
            return self.edge([0.0, p0[1]], [0.0, p1[1]]);
        }
        if lo >= w {
            return self.edge([w, p0[1]], [w, p1[1]]);
        }
        // cut where it crosses a side
        let at = |x: f32| {
            let t = (x - p0[0]) / (p1[0] - p0[0]);
            [x, p0[1] + (p1[1] - p0[1]) * t]
        };
        let mut cuts = vec![p0];
        let mut xs: Vec<f32> = [0.0, w].into_iter().filter(|x| *x > lo && *x < hi).collect();
        if p0[0] > p1[0] {
            xs.reverse();
        }
        cuts.extend(xs.into_iter().map(at));
        cuts.push(p1);
        for pair in cuts.windows(2) {
            let mid = (pair[0][0] + pair[1][0]) / 2.0;
            if mid <= 0.0 {
                self.edge([0.0, pair[0][1]], [0.0, pair[1][1]]);
            } else if mid >= w {
                self.edge([w, pair[0][1]], [w, pair[1][1]]);
            } else {
                self.edge([pair[0][0].clamp(0.0, w), pair[0][1]], [pair[1][0].clamp(0.0, w), pair[1][1]]);
            }
        }
    }
}

/// How much of each pixel `polys` cover, within a picture `w` by `h`. Each polygon is closed.
pub(crate) fn cover(polys: &[Vec<P>], rule: Rule, w: usize, h: usize) -> Option<Coverage> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in polys.iter().flatten() {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return None;
        }
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let (bx0, by0) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
    let (bx1, by1) = ((x1.ceil().max(0.0) as usize).min(w), (y1.ceil().max(0.0) as usize).min(h));
    if x1 <= x0 || y1 <= y0 || bx1 <= bx0 || by1 <= by0 {
        return None;
    }
    let (cw, ch) = (bx1 - bx0, by1 - by0);
    let mut acc = Accumulator { w: cw, h: ch, a: vec![0.0; (cw + 2) * ch] };
    let (ox, oy) = (bx0 as f32, by0 as f32);
    for poly in polys {
        let n = poly.len();
        if n < 3 {
            continue;
        }
        for k in 0..n {
            let (a, b) = (poly[k], poly[(k + 1) % n]);
            acc.line([a[0] - ox, a[1] - oy], [b[0] - ox, b[1] - oy]);
        }
    }
    // the running sum of each row is how much is covered, written where the edges were
    let mut data = acc.a;
    for row in data.chunks_exact_mut(cw + 2) {
        let mut sum = 0.0f32;
        for v in &mut row[..cw] {
            sum += *v;
            *v = match rule {
                Rule::NonZero => sum.abs().min(1.0),
                Rule::EvenOdd => {
                    let m = sum.abs().rem_euclid(2.0);
                    if m > 1.0 { 2.0 - m } else { m }
                }
            };
        }
    }
    Some(Coverage { x0: bx0, y0: by0, w: cw, h: ch, stride: cw + 2, data })
}

/// A picture to draw: straight RGBA.
pub struct Bitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, straight alpha.
    pub rgba: Vec<u8>,
}

/// What a covered pixel is painted with.
pub(crate) enum Paint<'a> {
    /// One straight colour.
    Solid([f32; 4]),
    /// A gradient: straight colours at offsets, between two points of the space `to_local` maps
    /// pixels into.
    Gradient { radial: bool, start: P, end: P, stops: &'a [(f32, [f32; 4])], to_local: Affine },
    /// A picture, for the pixels `to_local` maps into it.
    Image { bitmap: &'a Bitmap, to_local: Affine },
}

fn stop_colour(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    let Some(first) = stops.first() else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.0 {
            let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
            return std::array::from_fn(|c| a.1[c] + (b.1[c] - a.1[c]) * k);
        }
    }
    stops[stops.len() - 1].1
}

fn texel(b: &Bitmap, x: i64, y: i64) -> [f32; 4] {
    let (x, y) = (x.clamp(0, b.width as i64 - 1) as usize, y.clamp(0, b.height as i64 - 1) as usize);
    let p = &b.rgba[(y * b.width as usize + x) * 4..][..4];
    let a = p[3] as f32 / 255.0;
    [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
}

/// Paints `paint` over `surface` where `cov` covers it, `opacity` of it.
pub(crate) fn paint(surface: &mut Surface, cov: &Coverage, paint: &Paint, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    surface.touch(cov.x0, cov.y0, cov.x0 + cov.w, cov.y0 + cov.h);
    if let Paint::Solid(c) = paint {
        // the common case, without working the colour out again for each pixel
        let a = c[3] * opacity;
        let s = [c[0] * a, c[1] * a, c[2] * a, a];
        for y in 0..cov.h {
            let row = cov.row(cov.y0 + y);
            let out = &mut surface.px[(cov.y0 + y) * surface.w + cov.x0..][..cov.w];
            for (c, d) in row.iter().zip(out) {
                if *c >= 1.0 && a >= 1.0 {
                    *d = s;
                } else if *c > 0.0 {
                    let inv = 1.0 - a * c;
                    *d = [s[0] * c + d[0] * inv, s[1] * c + d[1] * inv, s[2] * c + d[2] * inv, a * c + d[3] * inv];
                }
            }
        }
        return;
    }
    for y in 0..cov.h {
        let row = cov.row(cov.y0 + y);
        let out = &mut surface.px[(cov.y0 + y) * surface.w + cov.x0..][..cov.w];
        let py = (cov.y0 + y) as f32 + 0.5;
        for (x, (c, d)) in row.iter().zip(out).enumerate() {
            if *c <= 0.0 {
                continue;
            }
            let px = (cov.x0 + x) as f32 + 0.5;
            // premultiplied source
            let s = match paint {
                Paint::Solid(c) => [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]],
                Paint::Gradient { radial, start, end, stops, to_local } => {
                    let p = to_local.apply([px, py]);
                    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
                    let t = if *radial {
                        let r = dx.hypot(dy);
                        if r > 0.0 { (p[0] - start[0]).hypot(p[1] - start[1]) / r } else { 1.0 }
                    } else {
                        let l2 = dx * dx + dy * dy;
                        if l2 > 0.0 { ((p[0] - start[0]) * dx + (p[1] - start[1]) * dy) / l2 } else { 1.0 }
                    };
                    let c = stop_colour(stops, t.clamp(0.0, 1.0));
                    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
                }
                Paint::Image { bitmap, to_local } => {
                    let p = to_local.apply([px, py]);
                    let (fx, fy) = (p[0] - 0.5, p[1] - 0.5);
                    let (ix, iy) = (fx.floor(), fy.floor());
                    let (tx, ty) = (fx - ix, fy - iy);
                    let (ix, iy) = (ix as i64, iy as i64);
                    let (a, b, c, e) = (
                        texel(bitmap, ix, iy),
                        texel(bitmap, ix + 1, iy),
                        texel(bitmap, ix, iy + 1),
                        texel(bitmap, ix + 1, iy + 1),
                    );
                    std::array::from_fn(|k| {
                        (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + e[k] * tx) * ty
                    })
                }
            };
            let k = c * opacity;
            let inv = 1.0 - s[3] * k;
            *d = [s[0] * k + d[0] * inv, s[1] * k + d[1] * inv, s[2] * k + d[2] * inv, s[3] * k + d[3] * inv];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_the_area_inside() {
        // a box from 1.5 to 4 across, 1 to 3 down
        let poly = vec![[1.5, 1.0], [4.0, 1.0], [4.0, 3.0], [1.5, 3.0]];
        let c = cover(std::slice::from_ref(&poly), Rule::NonZero, 8, 8).unwrap();
        assert_eq!((c.at(1, 1), c.at(2, 1), c.at(3, 2), c.at(4, 1), c.at(2, 3)), (0.5, 1.0, 1.0, 0.0, 0.0));
        // wound the other way it covers the same
        let back: Vec<P> = poly.iter().rev().copied().collect();
        assert_eq!(cover(&[back], Rule::NonZero, 8, 8).unwrap().at(1, 1), 0.5);
        // a box inside a box is a hole by even-odd, and filled by non-zero when wound the same way
        let outer = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]];
        let inner = vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]];
        let both = [outer, inner];
        assert_eq!(cover(&both, Rule::EvenOdd, 8, 8).unwrap().at(3, 3), 0.0);
        assert_eq!(cover(&both, Rule::NonZero, 8, 8).unwrap().at(3, 3), 1.0);
    }

    #[test]
    fn what_is_outside_the_picture_still_counts() {
        // a triangle much larger than the picture covers it wholly
        let big = vec![[-100.0, -100.0], [300.0, -100.0], [-100.0, 300.0]];
        let c = cover(&[big], Rule::NonZero, 4, 4).unwrap();
        assert!((0..4).all(|y| c.row(y).iter().all(|v| (*v - 1.0).abs() < 1e-4)), "{:?}", c.data);
        // an edge crossing the left side at a slant
        let slant = vec![[-2.0, 0.0], [2.0, 0.0], [2.0, 4.0], [-2.0, 4.0]];
        let c = cover(&[slant], Rule::NonZero, 4, 4).unwrap();
        assert_eq!((c.at(0, 0), c.at(1, 3), c.at(2, 0)), (1.0, 1.0, 0.0));
    }

    #[test]
    fn paint_lays_colour_over() {
        let mut s = Surface::new(4, 4);
        let c = cover(&[vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]], Rule::NonZero, 4, 4).unwrap();
        paint(&mut s, &c, &Paint::Solid([1.0, 0.0, 0.0, 1.0]), 0.5);
        assert_eq!(s.px[0], [0.5, 0.0, 0.0, 0.5]);
        // through a clip, only where both are
        let clip = cover(&[vec![[1.0, 0.0], [4.0, 0.0], [4.0, 4.0], [1.0, 4.0]]], Rule::NonZero, 4, 4);
        let both = cover(&[vec![[0.0, 2.0], [2.0, 2.0], [2.0, 4.0], [0.0, 4.0]]], Rule::NonZero, 4, 4).unwrap();
        let both = both.clipped(clip.as_ref()).unwrap();
        assert_eq!((both.x0, both.y0, both.w, both.h, both.at(1, 2), both.at(0, 2)), (1, 2, 1, 2, 1.0, 0.0));
        assert_eq!(s.dirty, [0, 0, 2, 2]);
        assert_eq!(&s.to_rgba8()[..4], &[255, 0, 0, 128]);
        s.clear();
        assert_eq!(s.px[0], [0.0; 4]);
    }
}
