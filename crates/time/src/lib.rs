//! Exact time for reelforge.
//!
//! Every time value in the editor, the RenderSpec and the renderer is a [`Tick`]: an integer count of
//! 1 / [`TICKS_PER_SECOND`] of a second. That number divides evenly into the frame duration of every
//! common frame rate (23.976, 24, 25, 29.97, 30, 50, 59.94, 60, 120) and the sample duration of every
//! common audio rate (8 kHz to 192 kHz, including the 44.1 kHz family), so frame and sample
//! positions are exact integers and an edit never drifts. Seconds as `f64` appear only at the edges
//! (user input, display), converted once.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Ticks in one second (the same base FilmCraft uses); see the crate docs for why this number.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A position or a duration in ticks.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    /// Zero.
    pub const ZERO: Tick = Tick(0);
    /// One second.
    pub const SECOND: Tick = Tick(TICKS_PER_SECOND);

    /// The tick nearest to `seconds`.
    pub fn from_seconds(seconds: f64) -> Tick {
        Tick((seconds * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// Whether this is no time at all.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// This time in seconds, for display and user input only.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// The smaller of two times.
    pub fn min(self, other: Tick) -> Tick {
        Tick(self.0.min(other.0))
    }

    /// The larger of two times.
    pub fn max(self, other: Tick) -> Tick {
        Tick(self.0.max(other.0))
    }

    /// This duration scaled by a playback speed, rounded to the nearest tick.
    pub fn scale(self, factor: f64) -> Tick {
        Tick((self.0 as f64 * factor).round() as i64)
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, rhs: Tick) -> Tick {
        Tick(self.0 + rhs.0)
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, rhs: Tick) {
        self.0 += rhs.0;
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, rhs: Tick) -> Tick {
        Tick(self.0 - rhs.0)
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, rhs: Tick) {
        self.0 -= rhs.0;
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(-self.0)
    }
}

impl Mul<i64> for Tick {
    type Output = Tick;
    fn mul(self, rhs: i64) -> Tick {
        Tick(self.0 * rhs)
    }
}

/// A rational frame rate, `num / den` frames per second (30000 / 1001 is 29.97).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct FrameRate {
    /// Numerator.
    pub num: u32,
    /// Denominator; never zero.
    pub den: u32,
}

impl FrameRate {
    /// 24 fps.
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    /// 25 fps.
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    /// 30 fps, the default for short videos.
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    /// 29.97 fps.
    pub const FPS_29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    /// 60 fps.
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };

    /// Whether this rate can be used: both terms non-zero and one frame is a whole number of ticks.
    pub fn is_valid(self) -> bool {
        self.num != 0 && self.den != 0 && (TICKS_PER_SECOND as i128 * self.den as i128) % self.num as i128 == 0
    }

    /// The duration of one frame. Exact for every valid rate.
    pub fn frame_duration(self) -> Tick {
        self.tick_of(1)
    }

    /// The start of frame `frame`.
    pub fn tick_of(self, frame: i64) -> Tick {
        Tick((frame as i128 * TICKS_PER_SECOND as i128 * self.den as i128 / self.num as i128) as i64)
    }

    /// The frame that contains `t` (frames are half-open, so a frame's own start belongs to it).
    pub fn frame_at(self, t: Tick) -> i64 {
        (t.0 as i128 * self.num as i128).div_euclid(TICKS_PER_SECOND as i128 * self.den as i128) as i64
    }

    /// `t` moved back to the start of its frame.
    pub fn snap(self, t: Tick) -> Tick {
        self.tick_of(self.frame_at(t))
    }

    /// Frames per second as a float, for display.
    pub fn fps(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// A half-open range of time, `[start, start + duration)`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TimeRange {
    /// First tick inside the range.
    pub start: Tick,
    /// Length; never negative.
    pub duration: Tick,
}

impl TimeRange {
    /// A range from `start` lasting `duration`.
    pub fn new(start: Tick, duration: Tick) -> TimeRange {
        TimeRange { start, duration }
    }

    /// The first tick after the range.
    pub fn end(self) -> Tick {
        self.start + self.duration
    }

    /// Whether `t` lies inside the range.
    pub fn contains(self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }

    /// Whether two ranges share any tick.
    pub fn overlaps(self, other: TimeRange) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_rates_are_exact() {
        for rate in [
            FrameRate::FPS_24,
            FrameRate::FPS_25,
            FrameRate::FPS_29_97,
            FrameRate::FPS_30,
            FrameRate::FPS_60,
            FrameRate { num: 24000, den: 1001 },
            FrameRate { num: 60000, den: 1001 },
            FrameRate { num: 120, den: 1 },
        ] {
            assert!(rate.is_valid(), "{rate:?}");
            for frame in [0, 1, 7, 1799, 107_892] {
                assert_eq!(rate.frame_at(rate.tick_of(frame)), frame);
                assert_eq!(rate.frame_at(rate.tick_of(frame + 1) - Tick(1)), frame);
            }
        }
        for sample_rate in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            assert_eq!(TICKS_PER_SECOND % sample_rate, 0);
        }
    }

    #[test]
    fn ntsc_does_not_drift() {
        let rate = FrameRate::FPS_29_97;
        let hour = rate.tick_of(107_892);
        assert_eq!(hour, rate.frame_duration() * 107_892);
    }

    #[test]
    fn negative_times_floor() {
        assert_eq!(FrameRate::FPS_30.frame_at(Tick(-1)), -1);
    }

    #[test]
    fn ranges() {
        let a = TimeRange::new(Tick(0), Tick(10));
        assert!(a.contains(Tick(0)) && !a.contains(Tick(10)));
        assert!(!a.overlaps(TimeRange::new(Tick(10), Tick(5))));
        assert!(a.overlaps(TimeRange::new(Tick(9), Tick(5))));
    }

    #[test]
    fn serde_is_a_bare_integer() {
        assert_eq!(serde_json::to_string(&Tick(42)).unwrap(), "42");
    }
}
