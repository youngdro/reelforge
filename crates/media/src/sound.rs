//! What is done to a clip's sound before its gain (`audio.fx/1`): noise taken out, a band kept, a
//! ring, an echo.
//!
//! Each effect is a function of the clip's own samples counted from its start, with silence
//! before it: a ring multiplies by a sine of the clip's time, a band is a fixed filter over a
//! fixed reach either side, an echo adds a fixed number of earlier copies, and noise is taken out
//! in frames on a grid that starts with the clip. So a stretch of the result does not depend on
//! where the mix that asked for it began, and a mix made in the preview's quarter-second slices is
//! the one made whole.

use reelforge_renderspec::AudioEffect;
use reelforge_time::TICKS_PER_SECOND;

use crate::analysis::{BANDS, FRAME, band_of, fft, window};

/// Sound, a list of samples for each channel.
pub type Planar = Vec<Vec<f32>>;

/// Reads `n` samples of a clip's sound from `from` samples after its start (which may lie before
/// it, where there is silence); `None` when the clip has no sound at all.
pub type Read<'a> = &'a dyn Fn(i64, usize) -> Result<Option<Planar>, String>;

/// `n` samples from `from` of what `read` gives, with `fx` done to it in order. At `rate`.
pub fn processed(read: Read, fx: &[AudioEffect], from: i64, n: usize, rate: u32) -> Result<Option<Planar>, String> {
    let Some((last, before)) = fx.split_last() else { return read(from, n) };
    match last {
        AudioEffect::Ring { hz, mix } => {
            let Some(mut x) = processed(read, before, from, n, rate)? else { return Ok(None) };
            let turn = std::f64::consts::TAU * f64::from(*hz) / f64::from(rate);
            for ch in &mut x {
                for (i, s) in ch.iter_mut().enumerate() {
                    let wave = (turn * (from + i as i64) as f64).sin() as f32;
                    *s *= 1.0 - mix + mix * wave;
                }
            }
            Ok(Some(x))
        }
        AudioEffect::Band { low, high } => {
            let taps = band_taps(*low, *high, rate);
            let reach = taps.len() / 2;
            let Some(x) = processed(read, before, from - reach as i64, n + 2 * reach, rate)? else {
                return Ok(None);
            };
            Ok(Some(
                x.iter()
                    .map(|ch| (0..n).map(|i| taps.iter().zip(&ch[i..]).map(|(t, s)| t * s).sum::<f32>()).collect())
                    .collect(),
            ))
        }
        AudioEffect::Echo { delay, decay, repeats } => {
            let gap = ((delay.0 as i128 * i128::from(rate) / TICKS_PER_SECOND as i128) as usize).max(1);
            let back = gap * *repeats as usize;
            let Some(x) = processed(read, before, from - back as i64, n + back, rate)? else { return Ok(None) };
            Ok(Some(
                x.iter()
                    .map(|ch| {
                        (0..n)
                            .map(|i| {
                                let mut loud = 1.0f32;
                                let mut sum = 0.0f32;
                                for k in 0..=*repeats as usize {
                                    sum += loud * ch[back + i - k * gap];
                                    loud *= decay;
                                }
                                sum
                            })
                            .collect()
                    })
                    .collect(),
            ))
        }
        AudioEffect::Denoise { floor, amount } => {
            let hop = (FRAME / 2) as i64;
            // frame `k` covers `(k - 1) · hop` to `(k + 1) · hop`; a sample is in two of them
            let (k0, k1) = (from.div_euclid(hop), (from + n as i64 - 1).div_euclid(hop) + 1);
            // and each frame is judged with the one before and the one after
            let lo = (k0 - 2) * hop;
            let Some(x) = processed(read, before, lo, ((k1 + 2) * hop - lo) as usize, rate)? else {
                return Ok(None);
            };
            Ok(Some(denoised(&x, floor, *amount, rate, (k1 - k0 + 1) as usize, (from - (k0 - 1) * hop) as usize, n)))
        }
    }
}

/// A band-pass filter's taps: a windowed sinc for `high` less one for `low`, symmetric, reaching
/// 128 samples either side at 48 kHz and as long in time at other rates.
fn band_taps(low: f32, high: f32, rate: u32) -> Vec<f32> {
    let reach = ((rate as usize * 128) / 48_000).max(8) as i64;
    let nyquist = rate as f32 / 2.0;
    let (lo, hi) = (low.clamp(0.0, nyquist) / rate as f32, high.clamp(0.0, nyquist) / rate as f32);
    let sinc = |cut: f32, j: i64| -> f32 {
        if j == 0 {
            2.0 * cut
        } else {
            (std::f32::consts::TAU * cut * j as f32).sin() / (std::f32::consts::PI * j as f32)
        }
    };
    (-reach..=reach)
        .map(|j| {
            let hamming = 0.54 + 0.46 * (std::f32::consts::PI * j as f32 / reach as f32).cos();
            (sinc(hi, j) - sinc(lo, j)) * hamming
        })
        .collect()
}

/// `n` samples of `x` with what does not rise above the noise `floor` turned down. `x` starts a
/// hop before the first of `frames + 2` frames half a frame apart; the first and last of those
/// are only looked at, the `frames` between are turned down and added up, and the result starts
/// `skip` samples into the first of them.
fn denoised(x: &Planar, floor: &[f32], amount: f32, rate: u32, frames: usize, skip: usize, n: usize) -> Planar {
    let hop = FRAME / 2;
    let window = window();
    let bins = FRAME / 2 + 1;
    // the noise in each bin, in the measure of `analysis::frame_density`
    let unit = rate as f32 * FRAME as f32 / 2.0;
    let noise: Vec<f32> = (0..bins)
        .map(|i| {
            let band = band_of(i as f32 * rate as f32 / FRAME as f32).min(BANDS - 1);
            10f32.powf(floor.get(band).copied().unwrap_or(-140.0) / 10.0) * unit
        })
        .collect();
    // every frame's spectrum, per channel, and its power over the channels
    let spectra: Vec<Vec<(Vec<f32>, Vec<f32>)>> = x
        .iter()
        .map(|ch| {
            (0..frames + 2)
                .map(|f| {
                    let mut re: Vec<f32> =
                        ch[f * hop..f * hop + FRAME].iter().zip(&window).map(|(s, w)| s * w).collect();
                    let mut im = vec![0.0; FRAME];
                    fft(&mut re, &mut im, false);
                    (re, im)
                })
                .collect()
        })
        .collect();
    let power: Vec<Vec<f32>> = (0..frames + 2)
        .map(|f| {
            (0..bins)
                .map(|i| {
                    spectra.iter().map(|ch| ch[f].0[i] * ch[f].0[i] + ch[f].1[i] * ch[f].1[i]).sum::<f32>()
                        / spectra.len().max(1) as f32
                })
                .collect()
        })
        .collect();
    // more is taken, and it is taken further down, the higher the amount: up to four times the
    // measured noise, down to 30 dB
    let over = 1.0 + 3.0 * amount;
    let least = 10f32.powf(-1.5 * amount);
    let mut out = vec![vec![0f32; n]; x.len()];
    let mut gain = vec![0f32; bins];
    for f in 1..=frames {
        for (i, g) in gain.iter_mut().enumerate() {
            // the power around: this frame and its neighbours in time, this bin and its neighbours
            let across = |f: usize| {
                let p = &power[f];
                (p[i.saturating_sub(1)] + 2.0 * p[i] + p[(i + 1).min(bins - 1)]) / 4.0
            };
            let around = (across(f - 1) + 2.0 * across(f) + across(f + 1)) / 4.0;
            *g = (1.0 - over * noise[i] / around.max(1e-20)).max(0.0).sqrt().max(least);
        }
        for (c, ch) in spectra.iter().enumerate() {
            let (mut re, mut im) = ch[f].clone();
            for i in 0..bins {
                re[i] *= gain[i];
                im[i] *= gain[i];
                if i > 0 && i < FRAME / 2 {
                    re[FRAME - i] *= gain[i];
                    im[FRAME - i] *= gain[i];
                }
            }
            fft(&mut re, &mut im, true);
            // frame `f` starts `(f - 1) · hop` after the first one that is kept
            let at = (f - 1) * hop;
            for j in 0..FRAME {
                if let Some(o) = (at + j).checked_sub(skip).filter(|o| *o < n) {
                    out[c][o] += re[j] * window[j];
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::noise_floor;
    use reelforge_time::Tick;

    const RATE: u32 = 48_000;

    fn noise(i: i64) -> f32 {
        let mut z = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }

    fn tone(hz: f32, i: i64) -> f32 {
        (std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(RATE)).sin() as f32
    }

    /// A clip whose sample `i` is `f(i)`, silent before its start.
    fn clip(f: impl Fn(i64) -> f32) -> impl Fn(i64, usize) -> Result<Option<Planar>, String> {
        move |from, n| {
            let ch: Vec<f32> = (from..from + n as i64).map(|i| if i < 0 { 0.0 } else { f(i) }).collect();
            Ok(Some(vec![ch.clone(), ch]))
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn every() -> Vec<AudioEffect> {
        vec![
            AudioEffect::Denoise { floor: vec![-80.0; BANDS], amount: 0.8 },
            AudioEffect::Band { low: 300.0, high: 3400.0 },
            AudioEffect::Ring { hz: 55.0, mix: 0.8 },
            AudioEffect::Echo { delay: Tick(TICKS_PER_SECOND / 10), decay: 0.5, repeats: 3 },
        ]
    }

    #[test]
    fn a_piece_is_the_same_whichever_mix_asked_for_it() {
        let read = clip(|i| 0.4 * tone(440.0, i) + 0.05 * noise(i));
        let fx = every();
        let whole = processed(&read, &fx, 0, 30_000, RATE).unwrap().unwrap();
        let mut at = 0usize;
        // pieces of awkward lengths, none on a frame's edge
        for len in [1usize, 777, 12_000, 5_001, 12_221] {
            let piece = processed(&read, &fx, at as i64, len, RATE).unwrap().unwrap();
            for c in 0..2 {
                for i in 0..len {
                    assert!((piece[c][i] - whole[c][at + i]).abs() < 1e-5, "{at}+{i}");
                }
            }
            at += len;
        }
        assert_eq!(at, 30_000);
        // nothing asked, nothing done; no sound, none made
        let plain = processed(&read, &[], 5, 100, RATE).unwrap().unwrap();
        assert_eq!(plain[0][0], read(5, 1).unwrap().unwrap()[0][0]);
        let none = |_: i64, _: usize| -> Result<Option<Planar>, String> { Ok(None) };
        assert_eq!(processed(&none, &fx, 0, 100, RATE).unwrap(), None);
    }

    #[test]
    fn a_band_keeps_what_is_inside_it() {
        let band = [AudioEffect::Band { low: 300.0, high: 3400.0 }];
        let through = |hz: f32| {
            let read = clip(move |i| tone(hz, i));
            rms(&processed(&read, &band, 4800, 9600, RATE).unwrap().unwrap()[0])
                / rms(&read(4800, 9600).unwrap().unwrap()[0])
        };
        assert!((through(1000.0) - 1.0).abs() < 0.05, "{}", through(1000.0));
        assert!((through(2500.0) - 1.0).abs() < 0.1, "{}", through(2500.0));
        assert!(through(50.0) < 0.15, "{}", through(50.0));
        assert!(through(6000.0) < 0.02, "{}", through(6000.0));
        assert!(through(12_000.0) < 0.02);
    }

    #[test]
    fn a_ring_moves_a_tone_to_either_side_and_an_echo_comes_back() {
        // a 1000 Hz tone ringed at 100 Hz is 900 Hz and 1100 Hz, and no 1000 Hz
        let read = clip(|i| tone(1000.0, i));
        let ringed = processed(&read, &[AudioEffect::Ring { hz: 100.0, mix: 1.0 }], 0, 48_000, RATE).unwrap().unwrap();
        let level = |hz: f32| {
            let (s, c) = ringed[0].iter().enumerate().fold((0.0f64, 0.0f64), |(s, c), (i, v)| {
                (
                    s + f64::from(*v * tone(hz, i as i64)),
                    c + f64::from(*v) * (std::f64::consts::TAU * f64::from(hz) * i as f64 / 48_000.0).cos(),
                )
            });
            (s * s + c * c).sqrt() / 24_000.0
        };
        assert!(level(1000.0) < 0.01 && (level(900.0) - 0.5).abs() < 0.01 && (level(1100.0) - 0.5).abs() < 0.01);
        // half mixed, the tone itself stays at half
        let half = processed(&read, &[AudioEffect::Ring { hz: 100.0, mix: 0.5 }], 0, 4, RATE).unwrap().unwrap();
        assert!((half[0][1] - 0.5 * tone(1000.0, 1) * (1.0 + tone(100.0, 1))).abs() < 1e-6);

        // one click comes back three times, each half as loud
        let click = clip(|i| if i == 100 { 1.0 } else { 0.0 });
        let echo = [AudioEffect::Echo { delay: Tick(TICKS_PER_SECOND / 100), decay: 0.5, repeats: 3 }];
        let out = processed(&click, &echo, 0, 2400, RATE).unwrap().unwrap();
        let heard: Vec<(usize, f32)> = out[1].iter().copied().enumerate().filter(|(_, v)| *v != 0.0).collect();
        assert_eq!(heard, [(100, 1.0), (580, 0.5), (1060, 0.25), (1540, 0.125)]);
    }

    #[test]
    fn noise_goes_and_the_voice_stays() {
        // hiss alone for a second, then a voice over it
        let hiss = |i: i64| 0.02 * noise(i);
        let voice = |i: i64| if i >= 48_000 { 0.4 * tone(300.0, i) + 0.2 * tone(1200.0, i) } else { 0.0 };
        let sound: Vec<f32> = (0..96_000).map(|i| hiss(i) + voice(i)).collect();
        let floor = noise_floor(&sound, RATE).unwrap().to_vec();
        let read = clip(move |i| hiss(i) + voice(i));
        let clean = |amount: f32| {
            let fx = [AudioEffect::Denoise { floor: floor.clone(), amount }];
            processed(&read, &fx, 0, 96_000, RATE).unwrap().unwrap()
        };
        let out = clean(0.8);
        let before = rms(&sound[4800..43_200]);
        let after = rms(&out[0][4800..43_200]);
        assert!(after < before * 0.12, "the hiss alone: {before} to {after}");
        // the voice is still there: what is left differs from the voice alone by less than the hiss did
        let left: Vec<f32> = (52_800..91_200).map(|i| out[0][i as usize] - voice(i)).collect();
        assert!(rms(&left) < before * 0.7, "{} / {before}", rms(&left));
        let kept = rms(&out[0][52_800..91_200]) / rms(&sound[52_800..91_200]);
        assert!((kept - 1.0).abs() < 0.05, "{kept}");
        // none taken, none changed
        let same = clean(0.0);
        for i in (0..96_000).step_by(997) {
            assert!((same[0][i] - sound[i]).abs() < 1e-4, "{i}");
        }
    }
}
