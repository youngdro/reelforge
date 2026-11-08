//! How a media clip runs through its source: a speed, a speed curve over the clip, played
//! backwards, or held on one frame.
//!
//! A clip's speed and length fix how much source it covers (`duration × speed`). A curve reshapes
//! the speed across the clip without changing that span: its multipliers are scaled so their
//! average is one. Played backwards, the clip covers the same span from its end. A speed of 0 is a
//! freeze frame: the clip shows `source_in` throughout and has no sound.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One point of a speed curve: at fraction `x` of the clip (0 to 1), the speed is `m` times the
/// clip's average speed. Straight lines join the points.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CurvePoint {
    /// Where in the clip, 0 (start) to 1 (end).
    pub x: f64,
    /// Speed relative to the clip's average, 0.1 to 10.
    pub m: f64,
}

/// The lowest and highest multiplier a curve may use.
pub const CURVE_RANGE: (f64, f64) = (0.1, 10.0);

/// Ready-made speed curves.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CurvePreset {
    /// Fast, slow in the middle, fast again.
    Montage,
    /// A slow-motion moment in the middle of fast action.
    Hero,
    /// A long, very slow middle.
    Bullet,
    /// A sudden jump forward halfway.
    JumpCut,
    /// Rushes in, then settles to normal.
    FlashIn,
    /// Normal, then rushes out.
    FlashOut,
}

impl CurvePreset {
    /// Every preset, in menu order.
    pub const ALL: [CurvePreset; 6] = [
        CurvePreset::Montage,
        CurvePreset::Hero,
        CurvePreset::Bullet,
        CurvePreset::JumpCut,
        CurvePreset::FlashIn,
        CurvePreset::FlashOut,
    ];

    /// The id commands use.
    pub fn name(self) -> &'static str {
        match self {
            CurvePreset::Montage => "montage",
            CurvePreset::Hero => "hero",
            CurvePreset::Bullet => "bullet",
            CurvePreset::JumpCut => "jumpCut",
            CurvePreset::FlashIn => "flashIn",
            CurvePreset::FlashOut => "flashOut",
        }
    }

    /// The preset with this id.
    pub fn parse(s: &str) -> Option<CurvePreset> {
        CurvePreset::ALL.into_iter().find(|p| p.name() == s)
    }

    /// The preset's points.
    pub fn points(self) -> Vec<CurvePoint> {
        let pts: &[(f64, f64)] = match self {
            CurvePreset::Montage => &[(0.0, 2.0), (0.25, 2.0), (0.4, 0.5), (0.6, 0.5), (0.75, 2.0), (1.0, 2.0)],
            CurvePreset::Hero => &[(0.0, 1.5), (0.35, 1.5), (0.5, 0.25), (0.65, 1.5), (1.0, 1.5)],
            CurvePreset::Bullet => &[(0.0, 2.0), (0.25, 2.0), (0.35, 0.2), (0.65, 0.2), (0.75, 2.0), (1.0, 2.0)],
            CurvePreset::JumpCut => &[(0.0, 0.7), (0.45, 0.7), (0.5, 5.0), (0.55, 0.7), (1.0, 0.7)],
            CurvePreset::FlashIn => &[(0.0, 4.0), (0.3, 4.0), (0.5, 1.0), (1.0, 1.0)],
            CurvePreset::FlashOut => &[(0.0, 1.0), (0.5, 1.0), (0.7, 4.0), (1.0, 4.0)],
        };
        pts.iter().map(|&(x, m)| CurvePoint { x, m }).collect()
    }

    /// The preset whose points these are.
    pub fn of(points: &[CurvePoint]) -> Option<CurvePreset> {
        CurvePreset::ALL.into_iter().find(|p| p.points() == points)
    }
}

/// The multiplier at `x`: straight lines between points, held beyond the ends.
pub fn curve_at(points: &[CurvePoint], x: f64) -> f64 {
    let Some(first) = points.first() else { return 1.0 };
    if x <= first.x {
        return first.m;
    }
    for w in points.windows(2) {
        if x < w[1].x {
            let f = (x - w[0].x) / (w[1].x - w[0].x);
            return w[0].m + (w[1].m - w[0].m) * f;
        }
    }
    points[points.len() - 1].m
}

/// The curve's average over `a..b` (fractions of the clip).
pub fn curve_mean(points: &[CurvePoint], a: f64, b: f64) -> f64 {
    if points.is_empty() || b <= a {
        return curve_at(points, a);
    }
    let mut cuts = vec![a, b];
    cuts.extend(points.iter().map(|p| p.x).filter(|x| *x > a && *x < b));
    cuts.sort_by(f64::total_cmp);
    let area: f64 =
        cuts.windows(2).map(|w| (w[1] - w[0]) * (curve_at(points, w[0]) + curve_at(points, w[1])) / 2.0).sum();
    area / (b - a)
}

/// The part of a curve over `a..b`, stretched to cover a whole clip.
pub fn curve_part(points: &[CurvePoint], a: f64, b: f64) -> Vec<CurvePoint> {
    if points.is_empty() {
        return vec![];
    }
    let mut out = vec![CurvePoint { x: 0.0, m: curve_at(points, a) }];
    out.extend(points.iter().filter(|p| p.x > a && p.x < b).map(|p| CurvePoint { x: (p.x - a) / (b - a), m: p.m }));
    out.push(CurvePoint { x: 1.0, m: curve_at(points, b) });
    out
}

/// How source time runs against clip time: `source = source_in + ∫ speed`, from the clip's start.
/// Speeds are signed (below 0 plays backwards); `ramp` replaces `speed` when it has keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// The source instant at the clip's start.
    pub source_in: Tick,
    /// The speed, when there is no ramp.
    pub speed: f64,
    /// Speeds at instants from the clip's start, straight lines between them.
    pub ramp: Vec<(Tick, f64)>,
}

impl Timing {
    /// A media clip's timing: it covers source from `source_in` for `duration × speed`.
    pub fn of(source_in: Tick, speed: f64, reversed: bool, curve: &[CurvePoint], duration: Tick) -> Timing {
        if speed == 0.0 {
            return Timing { source_in, speed: 0.0, ramp: vec![] };
        }
        let sign = if reversed { -1.0 } else { 1.0 };
        let mean = curve_mean(curve, 0.0, 1.0);
        let ramp = curve.iter().map(|p| (duration.scale(p.x), sign * speed * p.m / mean)).collect();
        // backwards, the first frame shown is the last one of the span
        let start = if reversed { source_in + duration.scale(speed) - Tick(1) } else { source_in };
        Timing { source_in: start, speed: sign * speed, ramp }
    }

    /// Source ticks covered from the clip's start to `t`, signed.
    pub fn travel(&self, t: Tick) -> Tick {
        if self.ramp.is_empty() {
            return t.scale(self.speed);
        }
        let r = &self.ramp;
        let at = |t: Tick| -> f64 {
            if t <= r[0].0 {
                return r[0].1;
            }
            for w in r.windows(2) {
                if t < w[1].0 {
                    let f = (t - w[0].0).0 as f64 / (w[1].0 - w[0].0).0 as f64;
                    return w[0].1 + (w[1].1 - w[0].1) * f;
                }
            }
            r[r.len() - 1].1
        };
        let (lo, hi, sign) = if t >= Tick::ZERO { (Tick::ZERO, t, 1.0) } else { (t, Tick::ZERO, -1.0) };
        let mut cuts = vec![lo, hi];
        cuts.extend(r.iter().map(|k| k.0).filter(|k| *k > lo && *k < hi));
        cuts.sort();
        let area: f64 = cuts.windows(2).map(|w| (w[1] - w[0]).0 as f64 * (at(w[0]) + at(w[1])) / 2.0).sum();
        Tick((sign * area).round() as i64)
    }

    /// The source instant `t` after the clip's start.
    pub fn source_at(&self, t: Tick) -> Tick {
        self.source_in + self.travel(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_keep_the_span_and_parts_rejoin() {
        let d = Tick::SECOND * 4;
        for p in CurvePreset::ALL {
            let pts = p.points();
            assert_eq!(CurvePreset::of(&pts), Some(p));
            let t = Timing::of(Tick::SECOND, 2.0, false, &pts, d);
            let end = t.source_at(d);
            assert!((end - Tick::SECOND * 9).0.abs() < 1000, "{p:?}: {end:?}");
            // the two halves of a split, each with its own average, run exactly as the whole
            let (a, b) = (curve_part(&pts, 0.0, 0.4), curve_part(&pts, 0.4, 1.0));
            let whole = curve_mean(&pts, 0.0, 1.0);
            let left = Timing::of(Tick::SECOND, 2.0 * curve_mean(&pts, 0.0, 0.4) / whole, false, &a, d.scale(0.4));
            for f in [0.1, 0.25, 0.4] {
                assert!((left.source_at(d.scale(f)) - t.source_at(d.scale(f))).0.abs() < 1000);
            }
            let mid = t.source_at(d.scale(0.4));
            let right = Timing::of(mid, 2.0 * curve_mean(&pts, 0.4, 1.0) / whole, false, &b, d.scale(0.6));
            for f in [0.5, 0.8, 1.0] {
                assert!((right.source_at(d.scale(f - 0.4)) - t.source_at(d.scale(f))).0.abs() < 1000);
            }
        }
    }

    #[test]
    fn backwards_starts_at_the_end_and_freeze_holds() {
        let t = Timing::of(Tick::SECOND, 1.0, true, &[], Tick::SECOND * 2);
        assert_eq!(t.source_at(Tick::ZERO), Tick::SECOND * 3 - Tick(1));
        assert_eq!(t.source_at(Tick::SECOND * 2), Tick::SECOND - Tick(1));
        let f = Timing::of(Tick::SECOND, 0.0, false, &[], Tick::SECOND * 2);
        assert_eq!(f.source_at(Tick::SECOND), Tick::SECOND);
    }
}
