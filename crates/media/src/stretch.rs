//! Changing sound's speed without changing its pitch (`audio.stretch/1`).
//!
//! Waveform-similarity overlap-add (WSOLA): the output is built from Hann-windowed grains a hop
//! apart; grain `k` is read from the source near `k · hop · speed`, nudged to where it best
//! continues the grain before it, so periodic sound joins without the beating plain overlap-add
//! leaves. Grains are chained in fixed blocks of [`BLOCK`]: the first grain of a block fits
//! against the un-nudged grain before it, each later one against the grain actually placed before
//! it. A stretch of output is made by running the chain from its block's start, so the result is
//! the same whether a mix is made in one piece or in the preview's quarter-second slices.

use reelforge_renderspec::Asset;

use crate::Footage;

/// Output samples between grains.
const HOP: i64 = 512;
/// Grain length: two hops, so Hann windows a hop apart sum to one.
const GRAIN: i64 = HOP * 2;
/// How far a grain may move to fit, either way.
const SEARCH: i64 = 256;
/// How much of a grain is compared when fitting it.
const FIT: usize = 256;
/// Grains per chain.
const BLOCK: i64 = 32;

/// `frames` output samples of `asset` played at `speed` with its pitch kept, starting `from`
/// samples after the clip's start, whose first sample is source sample `source_in`; all at `rate`.
pub fn stretched(
    media: &Footage,
    asset: &Asset,
    source_in: i64,
    speed: f64,
    from: i64,
    frames: usize,
    rate: u32,
) -> Result<Option<Vec<Vec<f32>>>, String> {
    let to = from + frames as i64;
    let (k0, k1) = (from.div_euclid(HOP), (to - 1).div_euclid(HOP) + 1);
    // the chain starts at its block's first grain
    let first = k0.div_euclid(BLOCK) * BLOCK;
    let natural = |k: i64| source_in + (k as f64 * HOP as f64 * speed).round() as i64 - HOP;
    // the source this stretch reads: every grain with room to move, and the grain before the first
    let lo = natural(first - 1) - SEARCH;
    let hi = natural(k1) + GRAIN + SEARCH;
    let lo_read = lo.max(0);
    if hi <= lo_read {
        return Ok(Some(vec![vec![0.0; frames]; 2]));
    }
    let Some(src) = media.audio(asset, lo_read, (hi - lo_read) as usize, rate)? else { return Ok(None) };
    if src.is_empty() {
        return Ok(None);
    }
    let at = |c: usize, i: i64| -> f32 {
        if i < lo_read {
            return 0.0;
        }
        src[c.min(src.len() - 1)].get((i - lo_read) as usize).copied().unwrap_or(0.0)
    };
    let mono = |i: i64| -> f32 { (0..src.len()).map(|c| at(c, i)).sum::<f32>() };
    let window: Vec<f32> =
        (0..GRAIN).map(|j| 0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / GRAIN as f32).cos()).collect();
    let fit = |target: i64, around: i64| -> i64 {
        let score = |d: i64| -> f32 { (0..FIT as i64).map(|j| mono(target + j) * mono(around + d + j)).sum() };
        // coarse, then fine around the best
        let mut best = (0i64, score(0));
        let mut d = -SEARCH;
        while d <= SEARCH {
            let s = score(d);
            if s > best.1 {
                best = (d, s);
            }
            d += 8;
        }
        let centre = best.0;
        for d in (centre - 7).max(-SEARCH)..=(centre + 7).min(SEARCH) {
            let s = score(d);
            if s > best.1 {
                best = (d, s);
            }
        }
        best.0
    };
    let channels = src.len().max(2);
    let mut out = vec![vec![0f32; frames]; channels];
    let mut before = natural(first - 1);
    for k in first..=k1 {
        // where the grain before runs on to, and where this one naturally starts
        let start = natural(k);
        let s = start + fit(before + HOP, start);
        before = if (k + 1) % BLOCK == 0 { natural(k) } else { s };
        if k < k0 {
            continue;
        }
        let o = k * HOP - HOP;
        for j in 0..GRAIN {
            let oi = o + j;
            if oi < from || oi >= to {
                continue;
            }
            let w = window[j as usize];
            for (c, ch) in out.iter_mut().enumerate() {
                ch[(oi - from) as usize] += w * at(c, s + j);
            }
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    /// The stretch's own maths on a tone, without a decoder: the pitch must hold.
    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn hann_windows_a_hop_apart_sum_to_one() {
        use super::*;
        let w = |j: i64| 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / GRAIN as f64).cos();
        for j in 0..HOP {
            assert!((w(j) + w(j + HOP) - 1.0).abs() < 1e-9);
        }
        assert_eq!(zero_crossings(&[-1.0, 1.0, -1.0, 1.0]), 2);
    }
}
