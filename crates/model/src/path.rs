//! A path a clip travels along: a smooth curve through a few points, at an even speed.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Clip, Ease};

/// A path a clip travels along, in place of its `x` and `y`: a smooth curve through `points`,
/// travelled at an even speed that `ease` shapes. Before it sets off the clip waits at the first
/// point, and after it arrives it stays at the last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MotionPath {
    /// The points it passes through, each counted as `x` and `y` are: offsets from the canvas
    /// centre (from the parent's anchor, when the clip has one) in fractions of the canvas. Two
    /// or more.
    pub points: Vec<[f32; 2]>,
    /// Whether it goes on from the last point round to the first again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    /// Whether the clip turns to face the way it goes: the path's heading (0 going right, 90
    /// going down) is added to its rotation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orient: bool,
    /// When it sets off, from the clip's start.
    #[serde(default, skip_serializing_if = "Tick::is_zero")]
    pub start: Tick,
    /// How long the journey takes; positive.
    pub duration: Tick,
    /// How it gets going and comes to rest.
    #[serde(default)]
    pub ease: Ease,
}

impl MotionPath {
    /// The most points a path may have.
    pub const MOST: usize = 64;
    /// How many straight pieces each stretch between two points is measured in.
    const PIECES: usize = 24;

    /// How many stretches between points it has.
    fn stretches(&self) -> usize {
        match self.points.len() {
            0 | 1 => 0,
            n if self.closed => n,
            n => n - 1,
        }
    }

    /// The curve `f` of the way along stretch `i`: a Catmull-Rom spline, which passes through
    /// every point and leaves each the way it came.
    fn point(&self, i: usize, f: f32) -> [f32; 2] {
        let n = self.points.len() as isize;
        let get = |k: isize| {
            let k = if self.closed { k.rem_euclid(n) } else { k.clamp(0, n - 1) };
            self.points[k as usize]
        };
        let i = i as isize;
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        let (f2, f3) = (f * f, f * f * f);
        [0, 1].map(|c| {
            0.5 * (2.0 * p1[c]
                + (p2[c] - p0[c]) * f
                + (2.0 * p0[c] - 5.0 * p1[c] + 4.0 * p2[c] - p3[c]) * f2
                + (3.0 * p1[c] - p0[c] - 3.0 * p2[c] + p3[c]) * f3)
        })
    }

    /// How far along the journey (0 to 1) the clip is at `t` from its start.
    pub fn progress(&self, t: Tick) -> f32 {
        if self.duration <= Tick::ZERO {
            return if t < self.start { 0.0 } else { 1.0 };
        }
        let u = ((t - self.start).0 as f64 / self.duration.0 as f64).clamp(0.0, 1.0) as f32;
        self.ease.apply(u).clamp(0.0, 1.0)
    }

    /// Where the path is `u` of its length along (0 to 1), and its heading there in degrees
    /// clockwise (0 going right), on a canvas `aspect` times as wide as it is high. The heading
    /// is counted on from the start, so a path that goes round keeps turning the same way.
    pub fn at(&self, u: f32, aspect: f32) -> ([f32; 2], f32) {
        let Some(first) = self.points.first().copied() else { return ([0.0, 0.0], 0.0) };
        let n = self.stretches();
        if n == 0 {
            return (first, 0.0);
        }
        // the curve as short straight pieces, measured as they are on the canvas
        let total = n * Self::PIECES;
        let at = |k: usize| {
            if k >= total && !self.closed {
                return self.points[self.points.len() - 1];
            }
            self.point(k / Self::PIECES % n, (k % Self::PIECES) as f32 / Self::PIECES as f32)
        };
        let pts: Vec<[f32; 2]> = (0..=total).map(at).collect();
        let far = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]) * aspect).hypot(b[1] - a[1]);
        let whole: f32 = pts.windows(2).map(|w| far(w[0], w[1])).sum();
        if whole <= 1e-9 {
            return (first, 0.0);
        }
        // each piece's heading, the short way round from the piece before; a piece with no
        // length keeps the heading it was reached with
        let short = |a: f32| (a + 180.0).rem_euclid(360.0) - 180.0;
        let mut headings = Vec::with_capacity(total);
        let mut last = None::<f32>;
        for w in pts.windows(2) {
            if far(w[0], w[1]) > 1e-9 {
                let way = (w[1][1] - w[0][1]).atan2((w[1][0] - w[0][0]) * aspect).to_degrees();
                last = Some(last.map_or(way, |h| h + short(way - h)));
            }
            headings.push(last);
        }
        let set_off = headings.iter().flatten().next().copied().unwrap_or(0.0);
        let headings: Vec<f32> = headings.into_iter().map(|h| h.unwrap_or(set_off)).collect();
        // past the ends of an open path the heading holds; a closed one goes on round
        let round = headings[total - 1] + short(headings[0] - headings[total - 1]) - headings[0];
        let heading = |k: isize| {
            let n = total as isize;
            if self.closed {
                headings[k.rem_euclid(n) as usize] + round * k.div_euclid(n) as f32
            } else {
                headings[k.clamp(0, n - 1) as usize]
            }
        };
        let want = u.clamp(0.0, 1.0) * whole;
        let mut gone = 0.0f32;
        for (k, w) in pts.windows(2).enumerate() {
            let d = far(w[0], w[1]);
            if gone + d >= want || k + 1 == total {
                let f = if d > 1e-9 { ((want - gone) / d).clamp(0.0, 1.0) } else { 0.0 };
                let here = [w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f];
                // it turns evenly from the middle of one piece to the middle of the next
                let k = k as isize;
                let (a, b, f) = if f < 0.5 { (k - 1, k, f + 0.5) } else { (k, k + 1, f - 0.5) };
                return (here, heading(a) + (heading(b) - heading(a)) * f);
            }
            gone += d;
        }
        (pts[total], headings[total - 1])
    }

    /// Moves it `by` in time, as a clip's keys move when its start is trimmed.
    pub fn shift(&mut self, by: Tick) {
        self.start += by;
    }

    /// Stretches it in time by `f`, as when the clip plays at another speed.
    pub fn stretch(&mut self, f: f64) {
        self.start = self.start.scale(f);
        self.duration = self.duration.scale(f).max(Tick(1));
    }
}

impl Clip {
    /// Where the clip's path has it at `t` from its start, on a canvas `aspect` times as wide as
    /// it is high: its `x` and `y`, and the degrees to add to its rotation (0 unless it turns to
    /// face the way it goes). `None` for a clip without a path.
    pub fn on_path(&self, t: Tick, aspect: f32) -> Option<([f32; 2], f32)> {
        let path = self.path.as_ref().filter(|p| p.points.len() >= 2)?;
        let (at, heading) = path.at(path.progress(t), aspect);
        Some((at, if path.orient { heading } else { 0.0 }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(points: &[[f32; 2]]) -> MotionPath {
        MotionPath {
            points: points.to_vec(),
            closed: false,
            orient: true,
            start: Tick::ZERO,
            duration: Tick::SECOND,
            ease: Ease::Linear,
        }
    }

    #[test]
    fn a_path_passes_through_its_points_at_an_even_speed() {
        // a right angle: along, then down
        let p = path(&[[-0.4, 0.0], [0.0, 0.0], [0.0, 0.4]]);
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 2e-3 && (a[1] - b[1]).abs() < 2e-3;
        assert!(near(p.at(0.0, 1.0).0, [-0.4, 0.0]) && near(p.at(1.0, 1.0).0, [0.0, 0.4]));
        // the corner is rounded, and the same way either side of the middle
        let (mid, heading) = p.at(0.5, 1.0);
        assert!((mid[0] + mid[1]).abs() < 2e-3 && mid[0] < 0.03 && mid[1] > -0.03, "{mid:?}");
        assert!((heading - 45.0).abs() < 3.0, "{heading}");
        // equal steps of the journey are equal steps along the curve
        let steps: Vec<f32> = (0..10)
            .map(|i| {
                let (a, b) = (p.at(i as f32 / 10.0, 1.0).0, p.at((i + 1) as f32 / 10.0, 1.0).0);
                (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .collect();
        let (lo, hi) = steps.iter().fold((f32::MAX, 0.0f32), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
        assert!(hi / lo < 1.05, "{steps:?}");
        // it sets off going right and arrives going down
        assert!(p.at(0.02, 1.0).1.abs() < 12.0 && (p.at(1.0, 1.0).1 - 90.0).abs() < 12.0);
        // on an upright canvas the same fractions down are farther than across
        let tall = p.at(0.5, 9.0 / 16.0).0;
        assert!(tall[1] < 0.2 && tall[0] > -0.01, "{tall:?}");
    }

    #[test]
    fn a_closed_path_comes_round_and_keeps_turning_the_same_way() {
        let mut p = path(&[[0.2, 0.0], [0.0, 0.2], [-0.2, 0.0], [0.0, -0.2]]);
        p.closed = true;
        let (a, b) = (p.at(0.0, 1.0), p.at(1.0, 1.0));
        assert!((a.0[0] - b.0[0]).abs() < 1e-3 && (a.0[1] - b.0[1]).abs() < 1e-3);
        // once round, clockwise: a whole turn on from where it set off
        assert!((b.1 - a.1 - 360.0).abs() < 20.0, "{} {}", a.1, b.1);
        let quarter = p.at(0.25, 1.0);
        assert!((quarter.0[0]).abs() < 0.01 && (quarter.0[1] - 0.2).abs() < 0.01, "{quarter:?}");
    }

    #[test]
    fn a_journey_waits_eases_and_stays() {
        let mut p = path(&[[0.0, 0.0], [0.4, 0.0]]);
        p.start = Tick::SECOND;
        p.ease = Ease::InOut;
        assert_eq!((p.progress(Tick::ZERO), p.progress(Tick(Tick::SECOND.0 * 3))), (0.0, 1.0));
        let early = p.progress(Tick(Tick::SECOND.0 * 5 / 4));
        assert!(early > 0.0 && early < 0.25, "{early}");
        assert_eq!(p.progress(Tick(Tick::SECOND.0 * 3 / 2)), 0.5);
    }
}
