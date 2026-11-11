//! Points, affine maps and paths: Bézier contours as Lottie writes them, flattened to polylines,
//! trimmed, dashed and widened into outlines.

/// A point or a vector.
pub(crate) type P = [f32; 2];

pub(crate) fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn mul(a: P, k: f32) -> P {
    [a[0] * k, a[1] * k]
}

pub(crate) fn lerp(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub(crate) fn len(a: P) -> f32 {
    a[0].hypot(a[1])
}

pub(crate) fn dist(a: P, b: P) -> f32 {
    len(sub(a, b))
}

/// An affine map: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(x: f32, y: f32) -> Affine {
        Affine { e: x, f: y, ..Affine::IDENTITY }
    }

    pub fn scale(x: f32, y: f32) -> Affine {
        Affine { a: x, d: y, ..Affine::IDENTITY }
    }

    /// A turn of `degrees` clockwise on a screen (y down).
    pub fn rotate(degrees: f32) -> Affine {
        let (s, c) = degrees.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// A shear along x by `tan`.
    pub fn shear(tan: f32) -> Affine {
        Affine { c: tan, ..Affine::IDENTITY }
    }

    /// `self` after `first`: the map that applies `first`, then `self`.
    pub fn then_after(self, first: Affine) -> Affine {
        Affine {
            a: self.a * first.a + self.c * first.b,
            b: self.b * first.a + self.d * first.b,
            c: self.a * first.c + self.c * first.d,
            d: self.b * first.c + self.d * first.d,
            e: self.a * first.e + self.c * first.f + self.e,
            f: self.b * first.e + self.d * first.f + self.f,
        }
    }

    pub fn apply(self, p: P) -> P {
        [self.a * p[0] + self.c * p[1] + self.e, self.b * p[0] + self.d * p[1] + self.f]
    }

    pub fn det(self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    pub fn inverse(self) -> Option<Affine> {
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) })
    }

    /// The most it stretches a length.
    pub fn stretch(self) -> f32 {
        let (p, q) = (self.a * self.a + self.b * self.b + self.c * self.c + self.d * self.d, self.det());
        (((p + (p * p - 4.0 * q * q).max(0.0).sqrt()) / 2.0).max(0.0)).sqrt()
    }
}

/// A Bézier contour as Lottie writes one: vertices, and for each the tangent coming in and the
/// tangent going out, relative to the vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Bez {
    pub v: Vec<P>,
    pub i: Vec<P>,
    pub o: Vec<P>,
    pub closed: bool,
}

impl Bez {
    /// From vertices with absolute tangents.
    pub fn absolute(points: &[(P, P, P)], closed: bool) -> Bez {
        Bez {
            v: points.iter().map(|p| p.0).collect(),
            i: points.iter().map(|p| sub(p.1, p.0)).collect(),
            o: points.iter().map(|p| sub(p.2, p.0)).collect(),
            closed,
        }
    }

    pub fn map(&self, m: Affine) -> Bez {
        let lin = Affine { e: 0.0, f: 0.0, ..m };
        Bez {
            v: self.v.iter().map(|p| m.apply(*p)).collect(),
            i: self.i.iter().map(|p| lin.apply(*p)).collect(),
            o: self.o.iter().map(|p| lin.apply(*p)).collect(),
            closed: self.closed,
        }
    }

    /// The contour as straight pieces no further than `tolerance` from it.
    pub fn flatten(&self, tolerance: f32) -> Line {
        let n = self.v.len().min(self.i.len()).min(self.o.len());
        let mut pts = Vec::with_capacity(n * 4);
        if n == 0 {
            return Line { pts, closed: self.closed };
        }
        pts.push(self.v[0]);
        let segments = if self.closed { n } else { n - 1 };
        for k in 0..segments {
            let j = (k + 1) % n;
            let (p0, p3) = (self.v[k], self.v[j]);
            let (p1, p2) = (add(p0, self.o[k]), add(p3, self.i[j]));
            if self.o[k] == [0.0, 0.0] && self.i[j] == [0.0, 0.0] {
                pts.push(p3);
                continue;
            }
            let bend = len(add(sub(p0, mul(p1, 2.0)), p2)).max(len(add(sub(p1, mul(p2, 2.0)), p3)));
            let steps = ((0.75 * bend / tolerance.max(1e-4)).sqrt().ceil() as usize).clamp(1, 96);
            for s in 1..=steps {
                pts.push(cubic(p0, p1, p2, p3, s as f32 / steps as f32));
            }
        }
        if self.closed && pts.len() > 1 && pts[0] == pts[pts.len() - 1] {
            pts.pop();
        }
        Line { pts, closed: self.closed }
    }
}

pub(crate) fn cubic(p0: P, p1: P, p2: P, p3: P, t: f32) -> P {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
}

/// A contour of straight pieces.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Line {
    pub pts: Vec<P>,
    pub closed: bool,
}

impl Line {
    pub fn map(&self, m: Affine) -> Line {
        Line { pts: self.pts.iter().map(|p| m.apply(*p)).collect(), closed: self.closed }
    }

    /// Its pieces, the closing one included.
    fn pieces(&self) -> impl Iterator<Item = (P, P)> + '_ {
        let n = self.pts.len();
        let count = if n < 2 {
            0
        } else if self.closed {
            n
        } else {
            n - 1
        };
        (0..count).map(move |k| (self.pts[k], self.pts[(k + 1) % n]))
    }

    pub fn length(&self) -> f32 {
        self.pieces().map(|(a, b)| dist(a, b)).sum()
    }

    /// The part from `from` to `to` along it, as lengths from its start (`from <= to`, both
    /// within its length).
    pub fn part(&self, from: f32, to: f32) -> Line {
        let mut pts: Vec<P> = Vec::new();
        let mut at = 0.0;
        for (a, b) in self.pieces() {
            let l = dist(a, b);
            let (start, end) = (at, at + l);
            at = end;
            if l <= 0.0 || end < from {
                continue;
            }
            if start > to {
                break;
            }
            let (t0, t1) = (((from - start) / l).clamp(0.0, 1.0), ((to - start) / l).clamp(0.0, 1.0));
            let (p, q) = (lerp(a, b, t0), lerp(a, b, t1));
            if pts.last() != Some(&p) {
                pts.push(p);
            }
            if pts.last() != Some(&q) {
                pts.push(q);
            }
        }
        Line { pts, closed: false }
    }

    /// The part between two fractions of its length, `start` to `start + span`, going round past
    /// its end when `start + span` is more than 1.
    pub fn trimmed(&self, start: f32, span: f32) -> Vec<Line> {
        let total = self.length();
        if span >= 1.0 - 1e-6 {
            return vec![self.clone()];
        }
        if span <= 1e-6 || total <= 0.0 {
            return Vec::new();
        }
        let start = start.rem_euclid(1.0);
        let end = start + span;
        if end <= 1.0 {
            return vec![self.part(start * total, end * total)];
        }
        let (mut first, second) = (self.part(start * total, total), self.part(0.0, (end - 1.0) * total));
        if self.closed {
            // one piece through the place the contour starts and ends
            for p in second.pts {
                if first.pts.last() != Some(&p) {
                    first.pts.push(p);
                }
            }
            return vec![first];
        }
        vec![first, second]
    }

    /// Cut into dashes: `pattern` is dash, gap, dash, gap… lengths, started `offset` in.
    pub fn dashed(&self, pattern: &[f32], offset: f32) -> Vec<Line> {
        let period: f32 = pattern.iter().sum();
        let total = self.length();
        if pattern.is_empty() || period <= 1e-3 || total <= 0.0 || pattern.iter().any(|v| *v < 0.0) {
            return vec![self.clone()];
        }
        let mut out = Vec::new();
        let mut at = -offset.rem_euclid(period);
        let mut k = 0;
        while at < total && out.len() < 4096 {
            let l = pattern[k % pattern.len()];
            if k % 2 == 0 && at + l > 0.0 && l > 0.0 {
                out.push(self.part(at.max(0.0), (at + l).min(total)));
            }
            at += l.max(1e-3);
            k += 1;
        }
        out
    }
}

/// How a line's ends are drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cap {
    Butt,
    Round,
    Square,
}

/// How a line's corners are drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Join {
    Miter,
    Round,
    Bevel,
}

fn area(poly: &[P]) -> f32 {
    let n = poly.len();
    (0..n).map(|k| poly[k][0] * poly[(k + 1) % n][1] - poly[(k + 1) % n][0] * poly[k][1]).sum::<f32>() / 2.0
}

/// Adds `poly` wound the positive way, so that outlines made of many overlapping polygons fill
/// as their union.
fn emit(out: &mut Vec<Vec<P>>, mut poly: Vec<P>) {
    let a = area(&poly);
    if a.abs() < 1e-9 {
        return;
    }
    if a < 0.0 {
        poly.reverse();
    }
    out.push(poly);
}

fn arc(out: &mut Vec<P>, centre: P, from: P, to: P, radius: f32, tolerance: f32) {
    let (a0, a1) = (from[1].atan2(from[0]), to[1].atan2(to[0]));
    let mut sweep = a1 - a0;
    while sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    }
    while sweep < -std::f32::consts::PI {
        sweep += std::f32::consts::TAU;
    }
    let step = 2.0 * (1.0 - (tolerance / radius.max(1e-3)).min(1.0)).acos().max(0.05);
    let n = ((sweep.abs() / step).ceil() as usize).clamp(1, 64);
    for k in 1..n {
        let a = a0 + sweep * k as f32 / n as f32;
        out.push([centre[0] + radius * a.cos(), centre[1] + radius * a.sin()]);
    }
}

fn disc(centre: P, radius: f32, tolerance: f32) -> Vec<P> {
    let step = 2.0 * (1.0 - (tolerance / radius.max(1e-3)).min(1.0)).acos().max(0.05);
    let n = ((std::f32::consts::TAU / step).ceil() as usize).clamp(8, 128);
    (0..n)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()]
        })
        .collect()
}

/// The outline of `line` drawn `width` wide, as polygons whose union it is (all wound one way:
/// fill them non-zero).
pub(crate) fn outline(line: &Line, width: f32, cap: Cap, join: Join, miter_limit: f32, tolerance: f32) -> Vec<Vec<P>> {
    let mut out = Vec::new();
    let hw = width / 2.0;
    if hw <= 0.0 {
        return out;
    }
    let mut pts: Vec<P> = Vec::with_capacity(line.pts.len());
    for p in &line.pts {
        if pts.last().is_none_or(|q| dist(*q, *p) > 1e-4) {
            pts.push(*p);
        }
    }
    let closed = line.closed && pts.len() > 2;
    if closed && dist(pts[0], pts[pts.len() - 1]) <= 1e-4 {
        pts.pop();
    }
    if pts.len() < 2 {
        if let (Some(p), true) = (pts.first(), !line.pts.is_empty()) {
            match cap {
                Cap::Round => emit(&mut out, disc(*p, hw, tolerance)),
                Cap::Square => emit(
                    &mut out,
                    vec![
                        [p[0] - hw, p[1] - hw],
                        [p[0] + hw, p[1] - hw],
                        [p[0] + hw, p[1] + hw],
                        [p[0] - hw, p[1] + hw],
                    ],
                ),
                Cap::Butt => {}
            }
        }
        return out;
    }
    let n = pts.len();
    let count = if closed { n } else { n - 1 };
    let dir = |k: usize| {
        let d = sub(pts[(k + 1) % n], pts[k]);
        mul(d, 1.0 / len(d).max(1e-9))
    };
    for k in 0..count {
        let (p, q, d) = (pts[k], pts[(k + 1) % n], dir(k));
        let nrm = [-d[1] * hw, d[0] * hw];
        emit(&mut out, vec![add(p, nrm), add(q, nrm), sub(q, nrm), sub(p, nrm)]);
    }
    // corners
    let corners: Vec<usize> = if closed { (0..n).collect() } else { (1..n - 1).collect() };
    for v in corners {
        let (d1, d2) = (dir((v + n - 1) % n), dir(v));
        let cross = d1[0] * d2[1] - d1[1] * d2[0];
        let dot = d1[0] * d2[0] + d1[1] * d2[1];
        if cross.abs() < 1e-6 && dot > 0.0 {
            continue;
        }
        // the outer side is away from where the line turns
        let side = if cross > 0.0 { -1.0 } else { 1.0 };
        let (n1, n2) = ([-d1[1] * hw * side, d1[0] * hw * side], [-d2[1] * hw * side, d2[0] * hw * side]);
        let c = pts[v];
        let sharp = dot < 0.96; // more than about 16 degrees
        match join {
            Join::Round if sharp => {
                let mut poly = vec![c, add(c, n1)];
                arc(&mut poly, c, n1, n2, hw, tolerance);
                poly.push(add(c, n2));
                emit(&mut out, poly);
            }
            Join::Miter if sharp => {
                // the miter's length over the line's half width is 1 / cos(half the turn)
                let half = ((1.0 + dot) / 2.0).max(0.0).sqrt();
                if half > 1e-3 && 1.0 / half <= miter_limit.max(1.0) {
                    let m = mul(add(n1, n2), 1.0 / (2.0 * half * half));
                    emit(&mut out, vec![c, add(c, n1), add(c, m), add(c, n2)]);
                } else {
                    emit(&mut out, vec![c, add(c, n1), add(c, n2)]);
                }
            }
            _ => emit(&mut out, vec![c, add(c, n1), add(c, n2)]),
        }
    }
    if !closed {
        for (p, d) in [(pts[0], mul(dir(0), -1.0)), (pts[n - 1], dir(n - 2))] {
            match cap {
                Cap::Butt => {}
                Cap::Round => emit(&mut out, disc(p, hw, tolerance)),
                Cap::Square => {
                    let nrm = [-d[1] * hw, d[0] * hw];
                    let far = add(p, mul(d, hw));
                    emit(&mut out, vec![add(p, nrm), add(far, nrm), sub(far, nrm), sub(p, nrm)]);
                }
            }
        }
    }
    out
}

/// Bézier contours with their sharp corners rounded by `radius`, as After Effects' Round
/// Corners does it.
pub(crate) fn rounded(path: &Bez, radius: f32) -> Bez {
    const K: f32 = 0.5519;
    let n = path.v.len();
    if radius <= 0.0 || n < 2 {
        return path.clone();
    }
    let mut pts: Vec<(P, P, P)> = Vec::with_capacity(n * 2);
    for k in 0..n {
        let v = path.v[k];
        if path.i[k] != [0.0, 0.0] || path.o[k] != [0.0, 0.0] || ((k == 0 || k == n - 1) && !path.closed) {
            pts.push((v, add(v, path.i[k]), add(v, path.o[k])));
            continue;
        }
        let towards = |other: P| {
            let d = dist(v, other);
            let part = if d > 0.0 { (d / 2.0).min(radius) / d } else { 0.0 };
            lerp(v, other, part)
        };
        let before = towards(path.v[(k + n - 1) % n]);
        pts.push((before, before, lerp(before, v, K)));
        let after = towards(path.v[(k + 1) % n]);
        pts.push((after, lerp(after, v, K), after));
    }
    Bez::absolute(&pts, path.closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_compose_and_invert() {
        let m = Affine::translate(10.0, 5.0).then_after(Affine::rotate(90.0)).then_after(Affine::scale(2.0, 1.0));
        let p = m.apply([1.0, 0.0]);
        assert!((p[0] - 10.0).abs() < 1e-5 && (p[1] - 7.0).abs() < 1e-5, "{p:?}");
        let back = m.inverse().unwrap().apply(p);
        assert!((back[0] - 1.0).abs() < 1e-5 && back[1].abs() < 1e-5);
        assert!((m.stretch() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_line_is_trimmed_dashed_and_widened() {
        let square = Line { pts: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], closed: true };
        assert_eq!(square.length(), 40.0);
        let half = square.trimmed(0.0, 0.5);
        assert_eq!(half[0].pts, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        // past the end it goes round as one piece
        let round = square.trimmed(0.875, 0.25);
        assert_eq!(round[0].pts, vec![[0.0, 5.0], [0.0, 0.0], [5.0, 0.0]]);
        let dashes = square.dashed(&[5.0, 5.0], 0.0);
        assert_eq!(dashes.len(), 4);
        let line = Line { pts: vec![[0.0, 0.0], [10.0, 0.0]], closed: false };
        let butt: f32 = outline(&line, 2.0, Cap::Butt, Join::Miter, 4.0, 0.1).iter().map(|p| area(p)).sum();
        let square: f32 = outline(&line, 2.0, Cap::Square, Join::Miter, 4.0, 0.1).iter().map(|p| area(p)).sum();
        assert_eq!((butt, square), (20.0, 24.0));
    }

    #[test]
    fn corners_are_rounded() {
        let corners: Vec<(P, P, P)> =
            [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]].into_iter().map(|p| (p, p, p)).collect();
        let b = Bez::absolute(&corners, true);
        let r = rounded(&b, 2.0);
        assert_eq!(r.v.len(), 8);
        assert_eq!(r.v[2], [8.0, 0.0]);
        assert_eq!(r.v[3], [10.0, 2.0]);
    }
}
