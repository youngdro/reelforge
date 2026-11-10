//! Shapes (`shape/1`): boxes, ellipses, lines, polygons, stars, price tags and arrows, filled with
//! a colour or a gradient, outlined, shadowed, their outline drawn on over time.
//!
//! A shape is drawn like text: into a sprite in target pixels, already placed, scaled and turned
//! by its layer, which both compositors then lay over the frame with the layer's effects, opacity
//! and blend. The outline is a polyline; the part `trim` keeps is inked by its distance from each
//! pixel, so its ends and corners come out round.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use filmcraft_text::mask::{blur, offset};
use filmcraft_text::raster::{Mask, Path, fill_into};
use reelforge_renderspec::{Animated, Paint, ShapeContent, ShapeKind};

use crate::Image;
use crate::image::premultiply_srgb;
use crate::plan::PlannedLayer;
use crate::text::Sprite;

/// One contour of a shape, in its own space (output pixels from its box's centre, y down).
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    /// Its points, in the order the outline runs.
    pub points: Vec<[f32; 2]>,
    /// Whether it returns to its first point.
    pub closed: bool,
}

/// The contours of `sh`, its points no further apart than about `detail` output pixels on its
/// curves. The first runs clockwise from where the outline starts; a hole runs the other way.
pub fn contours(sh: &ShapeContent, detail: f32) -> Vec<Contour> {
    let [w, h] = sh.size.map(|v| v.max(0.0));
    let (hw, hh) = (w / 2.0, h / 2.0);
    let detail = detail.max(0.05);
    let closed = |points| Contour { points, closed: true };
    // every corner of a polygon rounded by `r`
    let rounded = |corners: &[[f32; 2]], r: f32| -> Vec<[f32; 2]> {
        if r <= 0.0 {
            return corners.to_vec();
        }
        let n = corners.len();
        let mut out = Vec::new();
        for i in 0..n {
            let (a, p, b) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
            let len = |q: [f32; 2]| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt().max(1e-6);
            let d = r.min(len(a) / 2.0).min(len(b) / 2.0);
            let s = [p[0] + (a[0] - p[0]) / len(a) * d, p[1] + (a[1] - p[1]) / len(a) * d];
            let e = [p[0] + (b[0] - p[0]) / len(b) * d, p[1] + (b[1] - p[1]) / len(b) * d];
            // a cubic close to a quarter circle on a square corner
            const K: f32 = 0.5523;
            let c1 = [s[0] + (p[0] - s[0]) * K, s[1] + (p[1] - s[1]) * K];
            let c2 = [e[0] + (p[0] - e[0]) * K, e[1] + (p[1] - e[1]) * K];
            let steps = ((d * 1.6 / detail).ceil() as usize).clamp(2, 48);
            for j in 0..=steps {
                let t = j as f32 / steps as f32;
                let u = 1.0 - t;
                let (k0, k1, k2, k3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                out.push([
                    k0 * s[0] + k1 * c1[0] + k2 * c2[0] + k3 * e[0],
                    k0 * s[1] + k1 * c1[1] + k2 * c2[1] + k3 * e[1],
                ]);
            }
        }
        out
    };
    let ring = |cx: f32, cy: f32, rx: f32, ry: f32, clockwise: bool| -> Vec<[f32; 2]> {
        let around = std::f32::consts::TAU * ((rx * rx + ry * ry) / 2.0).sqrt();
        let n = ((around / detail).ceil() as usize).clamp(16, 512);
        (0..n)
            .map(|i| {
                let f = i as f32 / n as f32;
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * if clockwise { f } else { -f };
                [cx + rx * a.cos(), cy + ry * a.sin()]
            })
            .collect()
    };
    let spokes = |n: usize, inner: Option<f32>| -> Vec<[f32; 2]> {
        let count = if inner.is_some() { n * 2 } else { n };
        (0..count)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / count as f32;
                let k = if i % 2 == 1 { inner.unwrap_or(1.0) } else { 1.0 };
                [hw * k * a.cos(), hh * k * a.sin()]
            })
            .collect()
    };
    let r = sh.radius.max(0.0);
    match sh.kind {
        ShapeKind::Rectangle => {
            // from the top-left corner, along the top first
            let mut pts = rounded(&[[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]], r.min(hw).min(hh));
            if r > 0.0 {
                // start where the top edge does, not halfway round the first corner
                let first = pts.len() / 4 - 1;
                pts.rotate_left(first);
            }
            vec![closed(pts)]
        }
        ShapeKind::Ellipse => vec![closed(ring(0.0, 0.0, hw, hh, true))],
        ShapeKind::Line => vec![Contour { points: vec![[-hw, 0.0], [hw, 0.0]], closed: false }],
        ShapeKind::Polygon => vec![closed(spokes(sh.points.clamp(3, 64) as usize, None))],
        ShapeKind::Star => vec![closed(spokes(sh.points.clamp(3, 64) as usize, Some(sh.inner.clamp(0.05, 1.0))))],
        ShapeKind::Tag => {
            let nose = hh.min(w * 0.3);
            let corners = [[-hw + nose, -hh], [hw, -hh], [hw, hh], [-hw + nose, hh], [-hw, 0.0]];
            let outer = rounded(&corners, r.min(hh));
            let hole = ring(-hw + nose * 0.85, 0.0, hh * 0.16, hh * 0.16, false);
            vec![closed(outer), closed(hole)]
        }
        ShapeKind::Arrow => {
            let head = (w * 0.45).min(h);
            let shaft = hh * 0.42;
            vec![closed(rounded(
                &[
                    [-hw, -shaft],
                    [hw - head, -shaft],
                    [hw - head, -hh],
                    [hw, 0.0],
                    [hw - head, hh],
                    [hw - head, shaft],
                    [-hw, shaft],
                ],
                r.min(shaft),
            ))]
        }
    }
}

/// The parts of `contours` between `start` and `end`, fractions of their whole length taken in
/// order, as open polylines.
pub fn trimmed(contours: &[Contour], start: f32, end: f32) -> Vec<Vec<[f32; 2]>> {
    let runs: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .map(|c| {
            let mut p = c.points.clone();
            if c.closed
                && let Some(&first) = p.first()
            {
                p.push(first);
            }
            p
        })
        .collect();
    let seg = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let total: f32 = runs.iter().flat_map(|r| r.windows(2).map(|w| seg(w[0], w[1]))).sum();
    let (from, to) = (start.clamp(0.0, 1.0) * total, end.clamp(0.0, 1.0) * total);
    if total <= 0.0 || to <= from {
        return vec![];
    }
    if from <= 0.0 && to >= total {
        return runs;
    }
    let mut out = vec![];
    let mut at = 0.0;
    for run in &runs {
        let mut cur: Vec<[f32; 2]> = vec![];
        for w in run.windows(2) {
            let (a, b, l) = (w[0], w[1], seg(w[0], w[1]));
            let (s, e) = (at, at + l);
            at = e;
            if e <= from || s >= to || l <= 0.0 {
                continue;
            }
            let point = |d: f32| {
                let f = ((d - s) / l).clamp(0.0, 1.0);
                [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
            };
            if cur.is_empty() {
                cur.push(point(from.max(s)));
            }
            cur.push(point(to.min(e)));
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    out
}

/// Draws `sh` for `layer` on a `target`-sized frame; `k` is target pixels per output pixel.
/// `None` when nothing lands in the frame. The last few sprites are kept, as for text.
pub fn rasterize(layer: &PlannedLayer, sh: &ShapeContent, k: f32, target: [u32; 2]) -> Option<Sprite> {
    type Recent = Mutex<VecDeque<(String, Option<Sprite>)>>;
    static RECENT: OnceLock<Recent> = OnceLock::new();
    const KEEP: usize = 16;
    let key = format!("{sh:?}|{k}|{:?}|{}|{}|{target:?}", layer.center, layer.scale, layer.rotation);
    let recent = RECENT.get_or_init(Mutex::default);
    if let Some((_, sp)) = recent.lock().expect("sprite lock").iter().find(|(k, _)| *k == key) {
        return sp.clone();
    }
    let sp = draw(layer, sh, k, target);
    let mut r = recent.lock().expect("sprite lock");
    if r.len() == KEEP {
        r.pop_front();
    }
    r.push_back((key, sp.clone()));
    sp
}

fn value(a: &Option<Animated<f32>>, default: f32) -> f32 {
    a.as_ref().map_or(default, |v| v.sample(reelforge_time::Tick::ZERO))
}

fn draw(layer: &PlannedLayer, sh: &ShapeContent, k: f32, target: [u32; 2]) -> Option<Sprite> {
    let s = k * layer.scale;
    if s <= 0.0 || !s.is_finite() {
        return None;
    }
    let (sin, cos) = layer.rotation.sin_cos();
    let [cx, cy] = layer.center;
    let place = |p: [f32; 2]| [cx + s * (p[0] * cos - p[1] * sin), cy + s * (p[0] * sin + p[1] * cos)];
    // about half a target pixel between points on curves
    let shapes = contours(sh, 0.5 / s);

    let fill_op = value(&sh.fill_opacity, 1.0).clamp(0.0, 1.0);
    let filled = sh.fill.is_some() && sh.kind != ShapeKind::Line && fill_op > 0.0;
    let (t0, t1) = sh
        .trim
        .as_ref()
        .map_or((0.0, 1.0), |t| (t.start.sample(reelforge_time::Tick::ZERO), t.end.sample(reelforge_time::Tick::ZERO)));
    let stroke_w = sh.stroke.as_ref().map_or(0.0, |st| st.width * s);
    let lines: Vec<Vec<[f32; 2]>> = if stroke_w > 0.0 {
        trimmed(&shapes, t0, t1).into_iter().map(|l| l.into_iter().map(place).collect()).collect()
    } else {
        vec![]
    };
    let mut path = Path::new();
    if filled {
        for c in shapes.iter().filter(|c| c.closed && c.points.len() >= 3) {
            let pts: Vec<[f32; 2]> = c.points.iter().map(|p| place(*p)).collect();
            path.move_to(pts[0][0], pts[0][1]);
            for p in &pts[1..] {
                path.line_to(p[0], p[1]);
            }
            path.close();
        }
    }
    if path.is_empty() && lines.is_empty() {
        return None;
    }
    // where ink can be: the fill, the stroke's reach, the shadow's
    let reach = stroke_w / 2.0 + 1.0;
    let mut d = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut take = |x: f32, y: f32, grow: f32| {
        d = [d[0].min(x - grow), d[1].min(y - grow), d[2].max(x + grow), d[3].max(y + grow)];
    };
    if let Some((x0, y0, x1, y1)) = path.bounds() {
        take(x0, y0, 1.0);
        take(x1, y1, 1.0);
    }
    for p in lines.iter().flatten() {
        take(p[0], p[1], reach);
    }
    let shadow = sh
        .shadow
        .as_ref()
        .map(|sd| ((sd.offset[0] * k).round() as i32, (sd.offset[1] * k).round() as i32, sd.blur * k));
    let spread = shadow.map_or(0.0, |(dx, dy, sigma)| dx.abs().max(dy.abs()) as f32 + 3.0 * sigma) + 2.0;
    let x0 = ((d[0] - spread).floor() as i32).max(-(spread as i32));
    let y0 = ((d[1] - spread).floor() as i32).max(-(spread as i32));
    let x1 = ((d[2] + spread).ceil() as i32).min(target[0] as i32 + spread as i32);
    let y1 = ((d[3] + spread).ceil() as i32).min(target[1] as i32 + spread as i32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);

    let mut fill = Mask::new(w, h);
    if !path.is_empty() {
        fill_into(&path, &mut fill, -x0 as f32, -y0 as f32);
    }
    // the stroke: each pixel's distance from the nearest drawn segment, segment by segment
    let mut line = Mask::new(w, h);
    if !lines.is_empty() {
        let half = stroke_w / 2.0;
        let mut near = vec![f32::MAX; w * h];
        for run in &lines {
            for seg in run.windows(2) {
                let (a, b) =
                    ([seg[0][0] - x0 as f32, seg[0][1] - y0 as f32], [seg[1][0] - x0 as f32, seg[1][1] - y0 as f32]);
                let lo = [
                    (a[0].min(b[0]) - half - 1.0).floor().max(0.0) as usize,
                    (a[1].min(b[1]) - half - 1.0).floor().max(0.0) as usize,
                ];
                let hi = [
                    ((a[0].max(b[0]) + half + 1.0).ceil().max(0.0) as usize).min(w),
                    ((a[1].max(b[1]) + half + 1.0).ceil().max(0.0) as usize).min(h),
                ];
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let len2 = ex * ex + ey * ey;
                for y in lo[1]..hi[1] {
                    for x in lo[0]..hi[0] {
                        let (px, py) = (x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]);
                        let t = if len2 > 0.0 { ((px * ex + py * ey) / len2).clamp(0.0, 1.0) } else { 0.0 };
                        let (qx, qy) = (px - ex * t, py - ey * t);
                        let dist = (qx * qx + qy * qy).sqrt();
                        let i = y * w + x;
                        if dist < near[i] {
                            near[i] = dist;
                        }
                    }
                }
            }
        }
        for (c, d) in line.a.iter_mut().zip(near) {
            *c = (half - d + 0.5).clamp(0.0, 1.0);
        }
    }
    for a in &mut fill.a {
        *a *= fill_op;
    }
    let shade = shadow.map(|(dx, dy, sigma)| {
        let mut both = fill.clone();
        both.max_with(&line);
        let moved = offset(&both, dx, dy);
        if sigma > 0.0 { blur(&moved, sigma) } else { moved }
    });

    // the fill's colour where a target pixel is, from its place in the shape's own box
    let [hw, hh] = [sh.size[0].max(1e-3) / 2.0, sh.size[1].max(1e-3) / 2.0];
    let local = |x: f32, y: f32| {
        let (dx, dy) = (x - cx, y - cy);
        [(dx * cos + dy * sin) / s, (-dx * sin + dy * cos) / s]
    };
    let paint = sh.fill.as_ref().map(|p| match p {
        Paint::Solid { color } => (premultiply_srgb(color.0), premultiply_srgb(color.0), None),
        Paint::Linear { from, to, angle } => {
            (premultiply_srgb(from.0), premultiply_srgb(to.0), Some(Some(angle.to_radians())))
        }
        Paint::Radial { from, to } => (premultiply_srgb(from.0), premultiply_srgb(to.0), Some(None)),
    });
    let fill_at = |x: f32, y: f32| -> [f32; 4] {
        let Some((a, b, how)) = paint else { return [0.0; 4] };
        let t = match how {
            None => 0.0,
            Some(Some(angle)) => {
                let p = local(x, y);
                let (sn, cs) = angle.sin_cos();
                let extent = (hw * cs.abs() + hh * sn.abs()).max(1e-3);
                ((p[0] * cs + p[1] * sn) / extent * 0.5 + 0.5).clamp(0.0, 1.0)
            }
            Some(None) => {
                let p = local(x, y);
                ((p[0] / hw).powi(2) + (p[1] / hh).powi(2)).sqrt().clamp(0.0, 1.0)
            }
        };
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    };
    let stroke_c = sh.stroke.as_ref().map(|st| premultiply_srgb(st.color.0)).unwrap_or([0.0; 4]);
    let shadow_c = sh.shadow.as_ref().map(|sd| premultiply_srgb(sd.color.0)).unwrap_or([0.0; 4]);
    let over = |top: [f32; 4], cov: f32, under: [f32; 4]| -> [f32; 4] {
        let a = top[3] * cov;
        std::array::from_fn(|i| top[i] * cov + under[i] * (1.0 - a))
    };
    let mut image = Image::filled(w as u32, h as u32, [0.0; 4]);
    for (i, px) in image.pixels.iter_mut().enumerate() {
        let (x, y) = ((i % w) as f32 + 0.5 + x0 as f32, (i / w) as f32 + 0.5 + y0 as f32);
        if let Some(m) = &shade {
            *px = over(shadow_c, m.a[i].clamp(0.0, 1.0), *px);
        }
        let f = fill.a[i];
        if f > 0.0 {
            *px = over(fill_at(x, y), f.clamp(0.0, 1.0), *px);
        }
        let l = line.a[i];
        if l > 0.0 {
            *px = over(stroke_c, l, *px);
        }
    }
    Some(Sprite { x: x0, y: y0, image: Arc::new(image), map: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_renderspec::{Color, ShapeStroke, Trim};

    fn shape(kind: ShapeKind) -> ShapeContent {
        ShapeContent {
            kind,
            size: [200.0, 100.0],
            radius: 0.0,
            points: 5,
            inner: 0.5,
            fill: Some(Paint::Solid { color: Color([1.0, 0.0, 0.0, 1.0]) }),
            stroke: None,
            shadow: None,
            trim: None,
            fill_opacity: None,
        }
    }

    #[test]
    fn outlines_start_top_left_and_run_clockwise() {
        let r = contours(&shape(ShapeKind::Rectangle), 1.0);
        assert_eq!(r[0].points[..2], [[-100.0, -50.0], [100.0, -50.0]]);
        let mut rounded = shape(ShapeKind::Rectangle);
        rounded.radius = 20.0;
        let r = contours(&rounded, 1.0);
        // it starts on the top edge, after the first corner, heading right
        let (a, b) = (r[0].points[0], r[0].points[1]);
        assert!((a[1] + 50.0).abs() < 1e-3 && b[0] > a[0], "{a:?} {b:?}");
        // a tag has a hole, running the other way
        let t = contours(&shape(ShapeKind::Tag), 1.0);
        assert_eq!(t.len(), 2);
        let area = |p: &[[f32; 2]]| {
            (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f32>()
        };
        assert!(area(&t[0].points) > 0.0 && area(&t[1].points) < 0.0);
        // a star of five has ten corners
        assert_eq!(contours(&shape(ShapeKind::Star), 1.0)[0].points.len(), 10);
    }

    #[test]
    fn a_trim_keeps_part_of_the_outline_by_length() {
        let c = contours(&shape(ShapeKind::Rectangle), 1.0);
        // 600 pixels round; the first half is 300: the top edge and the right one
        let part = trimmed(&c, 0.0, 0.5);
        assert_eq!(part.len(), 1);
        assert_eq!(part[0].first(), Some(&[-100.0, -50.0]));
        let end = part[0].last().unwrap();
        assert!((end[0] - 100.0).abs() < 1e-3 && (end[1] - 50.0).abs() < 1e-3, "{end:?}");
        let mid = trimmed(&c, 0.25, 0.5);
        assert!((mid[0][0][0] - 50.0).abs() < 1e-3 && (mid[0][0][1] + 50.0).abs() < 1e-3);
        assert!(trimmed(&c, 0.5, 0.5).is_empty());
        assert_eq!(trimmed(&c, 0.0, 1.0)[0].len(), 5);
    }

    #[test]
    fn a_shape_is_filled_stroked_and_drawn_on() {
        use reelforge_renderspec::{Blend, Fit};
        let layer = PlannedLayer {
            id: "s".into(),
            source: crate::plan::PlannedSource::Solid([0.0; 4]),
            fit: Fit::Contain,
            center: [150.0, 100.0],
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            blend: Blend::Normal,
            fx: vec![],
            lut: None,
            crop: [0.0; 4],
            flip: [false; 2],
            shape: None,
            stretch: [1.0, 1.0],
            anchor: [0.5, 0.5],
            turn: None,
            carried: vec![],
            frame: [300.0, 200.0],
        };
        let mut sh = shape(ShapeKind::Rectangle);
        sh.stroke = Some(ShapeStroke { color: Color([0.0, 0.0, 1.0, 1.0]), width: 10.0 });
        let sp = rasterize(&layer, &sh, 1.0, [300, 200]).unwrap();
        let at = |sp: &Sprite, x: i32, y: i32| {
            let (u, v) = (x - sp.x, y - sp.y);
            if u < 0 || v < 0 || u >= sp.image.width as i32 || v >= sp.image.height as i32 {
                return [0.0; 4];
            }
            sp.image.pixels[(v * sp.image.width as i32 + u) as usize]
        };
        // red inside, blue on the edge, nothing outside it
        assert_eq!(at(&sp, 150, 100), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&sp, 150, 50), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(at(&sp, 150, 40)[3], 0.0);
        // half drawn on: the top and right edges have their outline, the bottom not yet
        sh.trim = Some(Trim { start: Animated::Constant(0.0), end: Animated::Constant(0.5) });
        sh.fill = None;
        let sp = rasterize(&layer, &sh, 1.0, [300, 200]).unwrap();
        assert_eq!(at(&sp, 150, 50)[3], 1.0);
        assert_eq!(at(&sp, 250, 100)[3], 1.0);
        assert_eq!(at(&sp, 150, 150)[3], 0.0);
        assert_eq!(at(&sp, 150, 100)[3], 0.0);
    }
}
