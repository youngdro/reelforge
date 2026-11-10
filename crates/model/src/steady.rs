//! A shaky shot steadied (剪映's 防抖).
//!
//! What is kept with the clip is the camera's path as it was measured, frame by frame, and how
//! much of the shake should go. What that does to the picture is worked out from the two each
//! time ([`Steady::steadied`]): the path is smoothed, the picture is moved from where it was to
//! where the smooth path has it, and it is enlarged just enough that its edges stay out of sight.
//! So how much is taken out can be changed without watching the file again.

use reelforge_time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A clip's picture held steady.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Steady {
    /// How much of the shake goes, above 0 and up to 1: a little leaves the camera's own
    /// movement and takes the tremble out, 1 holds the picture nearly still.
    pub amount: f32,
    /// The source instant of the path's first sample.
    pub from: Tick,
    /// The source time from one sample to the next: a frame of the file.
    pub step: Tick,
    /// Where the picture was at each sample against the first: across and down as fractions of
    /// its width and height (right and down are positive), and its turn in degrees (clockwise).
    pub path: Vec<[f32; 3]>,
}

/// What steadies a picture: how it is moved at each sample of its path, and how much larger it
/// is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Steadied {
    from: Tick,
    step: Tick,
    /// The move at each sample, as the path is given: fractions of the picture, and degrees.
    pub moves: Vec<[f32; 3]>,
    /// How much larger the picture is drawn, so that moved it still fills its frame.
    pub zoom: f32,
}

impl Steady {
    /// How much is taken out when nothing else is asked.
    pub const AMOUNT: f32 = 0.5;
    /// The most the picture is enlarged; where steadying would need more, less is taken out.
    pub const ZOOM: f32 = 1.3;
    /// The most samples a path has.
    pub const SAMPLES: usize = 36_000;

    /// Over how many seconds of the source the path is smoothed (the deviation of the bell the
    /// path is averaged under).
    fn smoothing(&self) -> f64 {
        0.25 + 1.75 * f64::from(self.amount.clamp(0.0, 1.0))
    }

    /// What steadies the picture, of `aspect` (its width over its height).
    pub fn steadied(&self, aspect: f32) -> Steadied {
        let n = self.path.len();
        let step = self.step.seconds().max(1e-6);
        let sigma = (self.smoothing() / step).max(0.5);
        let reach = (3.0 * sigma).ceil() as usize;
        let bell: Vec<f64> = (0..=reach).map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp()).collect();
        let mut moves = Vec::with_capacity(n);
        for i in 0..n {
            // a straight line laid through the path around this sample, nearer samples counting
            // for more: where the line is, here. In the middle of the path that is its average;
            // near the ends, where there is path on one side only, a steady pan stays a pan.
            let (mut s0, mut s1, mut s2) = (0.0f64, 0.0f64, 0.0f64);
            let (mut t0, mut t1) = ([0.0f64; 3], [0.0f64; 3]);
            for j in i.saturating_sub(reach)..(i + reach + 1).min(n) {
                let (w, d) = (bell[i.abs_diff(j)], j as f64 - i as f64);
                s0 += w;
                s1 += w * d;
                s2 += w * d * d;
                for k in 0..3 {
                    let v = f64::from(self.path[j][k]);
                    t0[k] += w * v;
                    t1[k] += w * d * v;
                }
            }
            let spread = s0 * s2 - s1 * s1;
            let at = self.path[i];
            moves.push([0, 1, 2].map(|k| {
                let smooth = if spread > 1e-9 { (s2 * t0[k] - s1 * t1[k]) / spread } else { t0[k] / s0 };
                smooth as f32 - at[k]
            }));
        }
        // how much larger a picture must be to fill its frame when moved like this
        let wide = aspect.max(1.0 / aspect.max(1e-6));
        let needs = |m: &[f32; 3]| {
            let (sin, cos) = m[2].to_radians().abs().sin_cos();
            (1.0 + 2.0 * m[0].abs().max(m[1].abs())) * (cos + sin * wide)
        };
        let zoom = moves.iter().map(needs).fold(1.0f32, f32::max).min(Self::ZOOM);
        // where even that is not enough, the picture is moved less
        for m in &mut moves {
            let need = needs(m);
            if need > zoom {
                let less = (zoom - 1.0) / (need - 1.0);
                *m = m.map(|v| v * less);
            }
        }
        Steadied { from: self.from, step: self.step, moves, zoom }
    }
}

impl Steadied {
    /// The move at source instant `at`: between samples, between their moves; before the first
    /// and after the last, theirs.
    pub fn at(&self, at: Tick) -> [f32; 3] {
        let (Some(first), Some(last)) = (self.moves.first(), self.moves.last()) else { return [0.0; 3] };
        if at <= self.from || self.step <= Tick::ZERO {
            return *first;
        }
        let along = (at - self.from).0 as f64 / self.step.0 as f64;
        let i = along.floor() as usize;
        if i + 1 >= self.moves.len() {
            return *last;
        }
        let f = (along - i as f64) as f32;
        let (a, b) = (self.moves[i], self.moves[i + 1]);
        [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shaky(n: usize) -> Vec<[f32; 3]> {
        // a slow pan to the right with a tremble on it
        (0..n)
            .map(|i| {
                let t = i as f32;
                [0.002 * t + 0.01 * (t * 1.9).sin(), 0.008 * (t * 2.3).cos(), 0.4 * (t * 1.3).sin()]
            })
            .collect()
    }

    #[test]
    fn the_tremble_goes_and_the_pan_stays() {
        let step = Tick::from_seconds(1.0 / 30.0);
        let steady = Steady { amount: 0.5, from: Tick::from_seconds(2.0), step, path: shaky(120) };
        let fixed = steady.steadied(16.0 / 9.0);
        assert_eq!(fixed.moves.len(), 120);
        // where the picture ends up: its path plus the move. From frame to frame it now goes
        // evenly, about as far as the pan takes it
        let ends: Vec<f32> = steady.path.iter().zip(&fixed.moves).map(|(p, m)| p[0] + m[0]).collect();
        let jumps = |v: &[f32]| v.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0.0f32, f32::max);
        let before: Vec<f32> = steady.path.iter().map(|p| p[0]).collect();
        assert!(jumps(&ends[10..110]) < 0.05 * jumps(&before[10..110]), "{} {}", jumps(&ends), jumps(&before));
        assert!((ends[100] - ends[40] - 0.002 * 60.0).abs() < 0.01, "{}", ends[100] - ends[40]);
        // enlarged enough to hide the edges, and no more than it may be
        let most = fixed.moves.iter().map(|m| m[0].abs().max(m[1].abs())).fold(0.0f32, f32::max);
        assert!(fixed.zoom >= 1.0 + 2.0 * most && fixed.zoom <= Steady::ZOOM, "{} {most}", fixed.zoom);
        // between samples, between their moves; outside the path, its ends
        let half = fixed.at(Tick::from_seconds(2.0) + step * 10 + Tick(step.0 / 2));
        assert!((half[0] - (fixed.moves[10][0] + fixed.moves[11][0]) / 2.0).abs() < 1e-5);
        assert_eq!(fixed.at(Tick::ZERO), fixed.moves[0]);
        assert_eq!(fixed.at(Tick::from_seconds(60.0)), fixed.moves[119]);
        // a slow sway is left mostly alone when little is asked, and taken out when much is
        let sway: Vec<[f32; 3]> = (0..240).map(|i| [0.02 * (i as f32 * 0.105).sin(), 0.0, 0.0]).collect();
        let moved = |amount: f32| {
            let fixed = Steady { amount, from: Tick::ZERO, step, path: sway.clone() }.steadied(16.0 / 9.0);
            fixed.moves[60..180].iter().map(|m| m[0].abs()).sum::<f32>()
        };
        assert!(moved(1.0) > 2.0 * moved(0.05), "{} {}", moved(1.0), moved(0.05));
    }

    #[test]
    fn a_jolt_too_large_to_hide_is_followed_part_of_the_way() {
        let step = Tick::from_seconds(1.0 / 30.0);
        let mut path = vec![[0.0f32; 3]; 90];
        for p in &mut path[40..50] {
            p[0] = 0.4;
        }
        let fixed = Steady { amount: 1.0, from: Tick::ZERO, step, path }.steadied(1.0);
        assert_eq!(fixed.zoom, Steady::ZOOM);
        let most = fixed.moves.iter().map(|m| m[0].abs()).fold(0.0f32, f32::max);
        assert!(most <= (Steady::ZOOM - 1.0) / 2.0 + 1e-4, "{most}");
        // nothing to steady: nothing moves and nothing is enlarged
        let calm = Steady { amount: 0.5, from: Tick::ZERO, step, path: vec![[0.0; 3]; 30] }.steadied(1.0);
        assert_eq!((calm.zoom, calm.at(Tick::from_seconds(0.5))), (1.0, [0.0; 3]));
    }
}
