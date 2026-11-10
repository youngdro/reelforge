//! Listening to sound: how loud it is, where its beats fall, where something is heard at all, and
//! what its background noise is like. Pure functions of samples, for the tools that set a clip's
//! volume, mark a song's beats, lower music under a voice and clean up a recording.

/// The sample rate sound is listened to at (the loudness filter is defined for it).
pub const RATE: u32 = 48_000;

/// One second-order filter section, `a0` being 1.
#[derive(Copy, Clone)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn run(&self, x: &[f32]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&x0| {
                let x0 = f64::from(x0);
                let y0 = self.b[0] * x0 + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                (x2, x1, y2, y1) = (x1, x0, y1, y0);
                y0
            })
            .collect()
    }
}

/// The K-weighting of ITU-R BS.1770 at 48 kHz: a high shelf for the head, then a high-pass.
const K_WEIGHTING: [Biquad; 2] = [
    Biquad {
        b: [1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85],
        a: [-1.690_659_293_182_41, 0.732_480_774_215_85],
    },
    Biquad { b: [1.0, -2.0, 1.0], a: [-1.990_047_454_833_98, 0.990_072_250_366_21] },
];

/// How loud `channels` (at [`RATE`]) sound over their whole length, in LUFS: the integrated
/// loudness of ITU-R BS.1770, K-weighted, in 400 ms blocks a quarter apart, quiet blocks gated
/// out. `None` when there is nothing to hear.
pub fn loudness(channels: &[Vec<f32>]) -> Option<f32> {
    let n = channels.iter().map(Vec::len).min()?;
    if n == 0 {
        return None;
    }
    let weighted: Vec<Vec<f64>> = channels
        .iter()
        .map(|ch| {
            let once = K_WEIGHTING[0].run(&ch[..n]);
            let once: Vec<f32> = once.iter().map(|v| *v as f32).collect();
            K_WEIGHTING[1].run(&once)
        })
        .collect();
    let block = (RATE as usize * 2 / 5).min(n);
    let step = (block / 4).max(1);
    // each block's power, summed over the channels
    let mut powers = Vec::new();
    let mut at = 0;
    while at + block <= n {
        let p: f64 = weighted.iter().map(|ch| ch[at..at + block].iter().map(|v| v * v).sum::<f64>()).sum();
        powers.push(p / block as f64);
        at += step;
    }
    let lufs = |p: f64| -0.691 + 10.0 * p.log10();
    let mean = |ps: &[f64]| ps.iter().sum::<f64>() / ps.len() as f64;
    let heard: Vec<f64> = powers.into_iter().filter(|p| *p > 0.0 && lufs(*p) > -70.0).collect();
    if heard.is_empty() {
        return None;
    }
    let gate = lufs(mean(&heard)) - 10.0;
    let kept: Vec<f64> = heard.iter().copied().filter(|p| lufs(*p) > gate).collect();
    Some(lufs(mean(if kept.is_empty() { &heard } else { &kept })) as f32)
}

/// The largest sample, as an amplitude.
pub fn peak(channels: &[Vec<f32>]) -> f32 {
    channels.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// All channels as one.
pub fn mono(channels: &[Vec<f32>]) -> Vec<f32> {
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let k = 1.0 / channels.len().max(1) as f32;
    (0..n).map(|i| channels.iter().map(|ch| ch[i]).sum::<f32>() * k).collect()
}

/// A song's beats.
#[derive(Clone, Debug, PartialEq)]
pub struct Beats {
    /// Beats a minute.
    pub bpm: f32,
    /// Where each falls, in samples.
    pub at: Vec<i64>,
}

/// Finds the beat of `sound` (one channel at `rate`): how fast it is, between 60 and 180 a
/// minute, and where each beat falls. `None` when nothing in it pulses.
///
/// The sound's onsets are followed 100 times a second (rises in the bass and in the whole, on a
/// log scale); the tempo is the lag at which the onsets best repeat, with a mild preference for
/// two beats a second; the beats are the comb of that period that collects the most onset, each
/// tooth moved to the strongest onset close by.
pub fn beats(sound: &[f32], rate: u32) -> Option<Beats> {
    let hop = (rate / 100).max(1) as usize;
    let frames = sound.len() / hop;
    if frames < 200 {
        return None;
    }
    // energy per frame, of the bass (one-pole low-pass at about 150 Hz) and of everything
    let k = 1.0 - (-std::f32::consts::TAU * 150.0 / rate as f32).exp();
    let mut low = 0.0f32;
    let (mut bass, mut whole) = (vec![0f32; frames], vec![0f32; frames]);
    for (f, chunk) in sound.chunks_exact(hop).enumerate() {
        for &s in chunk {
            low += k * (s - low);
            bass[f] += low * low;
            whole[f] += s * s;
        }
    }
    let rise = |e: &[f32], f: usize| ((e[f] + 1e-7).ln() - (e[f - 1] + 1e-7).ln()).max(0.0);
    let mut flux: Vec<f32> =
        (0..frames).map(|f| if f == 0 { 0.0 } else { rise(&bass, f) + 0.5 * rise(&whole, f) }).collect();
    // what stands out of the half second around it
    let around = 50usize;
    let sums: Vec<f32> = std::iter::once(0.0)
        .chain(flux.iter().scan(0.0f32, |acc, v| {
            *acc += v;
            Some(*acc)
        }))
        .collect();
    let local: Vec<f32> = (0..frames)
        .map(|f| {
            let (a, b) = (f.saturating_sub(around), (f + around + 1).min(frames));
            (sums[b] - sums[a]) / (b - a) as f32
        })
        .collect();
    for (v, m) in flux.iter_mut().zip(&local) {
        *v = (*v - m).max(0.0);
    }
    // a pulse has onsets that stand out; a steady sound's ripple is not one
    if flux.iter().filter(|v| **v > 0.3).count() < 4 {
        return None;
    }
    // how well the onsets repeat at each lag, one and two beats on
    let score = |lag: usize| -> f32 {
        let n = frames.saturating_sub(2 * lag);
        if n == 0 {
            return 0.0;
        }
        let s: f32 = (0..n).map(|f| flux[f] * (flux[f + lag] + 0.5 * flux[f + 2 * lag])).sum();
        // songs are more often near 120 a minute than at either end
        let octaves = (lag as f32 / 50.0).log2();
        s / n as f32 * (-0.5 * (octaves / 0.9).powi(2)).exp()
    };
    let (lo, hi) = (33usize, 100usize);
    let scores: Vec<f32> = (lo - 1..=hi + 1).map(score).collect();
    let best = (1..scores.len() - 1).max_by(|a, b| scores[*a].total_cmp(&scores[*b]))?;
    // between frames: the top of the parabola through the best lag and its neighbours
    let (l, m, r) = (scores[best - 1], scores[best], scores[best + 1]);
    let bend = l - 2.0 * m + r;
    let period = (lo - 1 + best) as f32 + if bend < 0.0 { (0.5 * (l - r) / bend).clamp(-0.5, 0.5) } else { 0.0 };
    // the comb that collects the most
    let teeth = |phase: f32| (0..).map(move |k| phase + k as f32 * period).take_while(|t| (*t as usize) < frames);
    let phase = (0..period.ceil() as usize)
        .map(|p| p as f32)
        .max_by(|a, b| {
            let sum = |p: f32| teeth(p).map(|t| flux[t as usize]).sum::<f32>();
            sum(*a).total_cmp(&sum(*b))
        })
        .unwrap_or(0.0);
    let reach = (period / 10.0) as usize;
    let clear = 0.2 * flux.iter().copied().fold(0.0f32, f32::max);
    let mut on = 0;
    let at: Vec<i64> = teeth(phase)
        .map(|t| {
            let t = t.round() as usize;
            let (a, b) = (t.saturating_sub(reach), (t + reach + 1).min(frames));
            let strongest = (a..b).max_by(|x, y| flux[*x].total_cmp(&flux[*y])).unwrap_or(t);
            on += usize::from(flux[strongest] > clear);
            // a tooth with no onset near it stays where the comb put it
            let f = if flux[strongest] > 0.0 { strongest } else { t.min(frames - 1) };
            (f * hop) as i64
        })
        .collect();
    // a beat is kept by most of its teeth; a few sounds some way apart are not one
    if on < 6 || on * 2 < at.len() {
        return None;
    }
    Some(Beats { bpm: 6000.0 / period, at })
}

/// How many levels [`levels`] gives for a second of sound.
pub const LEVELS_PER_SECOND: u32 = 50;

/// How loud `sound` (one channel at `rate`) is, in dBFS, a fiftieth of a second at a time.
pub fn levels(sound: &[f32], rate: u32) -> Vec<f32> {
    let frame = (rate / LEVELS_PER_SECOND).max(1) as usize;
    sound
        .chunks(frame)
        .map(|c| 10.0 * (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32 + 1e-12).log10())
        .collect()
}

/// The level above which something is heard among `db` (see [`levels`]): 30 dB under its loudest
/// fifth of a second, and no lower than -50 dBFS, nor than a line drawn above its background
/// noise when it has one.
fn gate(db: &[f32]) -> f32 {
    // the loudest stretch, over ten frames
    let top = db.windows(10.min(db.len())).map(|w| w.iter().sum::<f32>() / w.len() as f32).fold(f32::MIN, f32::max);
    // and the quiet it comes out of: the level a tenth of the frames stay under
    let mut sorted = db.to_vec();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    // over a noisy background the line is drawn part of the way up from the noise; a sound with
    // no quiet in it is heard throughout
    let above_noise = if top - floor >= 10.0 { floor + 0.4 * (top - floor) } else { f32::MIN };
    (top - 30.0).max(-50.0).max(above_noise)
}

/// The stretches of `db` over `gate`, as `(first, one past the last)`: ones no more than `close`
/// levels apart are one, and ones shorter than `least` are dropped.
fn over(db: &[f32], gate: f32, close: usize, least: usize) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (i, v) in db.iter().enumerate() {
        if *v <= gate {
            continue;
        }
        match spans.last_mut() {
            Some(last) if i - last.1 <= close => last.1 = i + 1,
            _ => spans.push((i, i + 1)),
        }
    }
    spans.retain(|(a, b)| b - a >= least);
    spans
}

/// Where something is heard in `sound` (one channel at `rate`): spans of samples louder than
/// 30 dB under its loudest fifth of a second (and than -50 dBFS, and than a line drawn above its
/// background noise when it has one), short gaps closed, short blips dropped, each span opened a
/// little early and held a little late.
pub fn heard_spans(sound: &[f32], rate: u32) -> Vec<(i64, i64)> {
    let frame = (rate / LEVELS_PER_SECOND).max(1) as usize;
    let db = levels(sound, rate);
    if db.is_empty() {
        return vec![];
    }
    let lead = 5;
    // a gap under 0.4 s is a breath, not a pause
    over(&db, gate(&db), 20, 8)
        .into_iter()
        .map(|(a, b)| ((a.saturating_sub(lead) * frame) as i64, (((b + lead) * frame).min(sound.len())) as i64))
        .collect()
}

/// Where nothing is heard among `db` (see [`levels`]) for at least `least` levels: the pauses in
/// speech, with the quiet before the first sound and after the last. A click or a breath shorter
/// than 0.08 s does not end a pause, and a gap under 0.1 s inside a word does not start one.
/// `None` when nothing is heard at all.
pub fn quiet_spans(db: &[f32], least: usize) -> Option<Vec<(usize, usize)>> {
    if db.is_empty() {
        return None;
    }
    let heard = over(db, gate(db), 5, 4);
    if heard.is_empty() {
        return None;
    }
    let mut quiet = Vec::new();
    let mut from = 0;
    for (a, b) in heard.into_iter().chain([(db.len(), db.len())]) {
        if a - from >= least.max(1) {
            quiet.push((from, a));
        }
        from = b;
    }
    Some(quiet)
}

/// The length of the frames noise is measured and removed in.
pub const FRAME: usize = 1024;
/// How many bands a noise profile has.
pub const BANDS: usize = 24;

/// The band a frequency falls in: [`BANDS`] of them, evenly spaced on a log scale from 60 Hz to
/// 16 kHz, the first and last taking what lies beyond.
pub fn band_of(hz: f32) -> usize {
    let x = (hz.max(1.0) / 60.0).log2() / (16_000.0f32 / 60.0).log2();
    ((x * BANDS as f32).floor().max(0.0) as usize).min(BANDS - 1)
}

/// A Hann window's square root: used going in and coming out, frames half a frame apart add up
/// to the sound they were cut from.
pub fn window() -> Vec<f32> {
    (0..FRAME).map(|j| (0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / FRAME as f32).cos()).sqrt()).collect()
}

/// The Fourier transform of `re + i·im` in place (a power of two long); `inverse` goes back,
/// scaled so that there and back is the identity.
pub fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = std::f64::consts::TAU / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (angle.cos() as f32, angle.sin() as f32);
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                (re[b], im[b]) = (re[a] - xr, im[a] - xi);
                (re[a], im[a]) = (re[a] + xr, im[a] + xi);
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
    if inverse {
        let k = 1.0 / n as f32;
        for v in re.iter_mut().chain(im.iter_mut()) {
            *v *= k;
        }
    }
}

/// The power of each bin of a windowed frame, as a density: per hertz, whatever the sample rate.
pub fn frame_density(frame: &[f32], window: &[f32], rate: u32) -> Vec<f32> {
    let mut re: Vec<f32> = frame.iter().zip(window).map(|(s, w)| s * w).collect();
    let mut im = vec![0.0; FRAME];
    fft(&mut re, &mut im, false);
    // a Hann window's squares add up to half the frame
    let k = 1.0 / (rate as f32 * FRAME as f32 / 2.0);
    (0..=FRAME / 2).map(|i| (re[i] * re[i] + im[i] * im[i]) * k).collect()
}

/// What `sound`'s background is like (one channel at `rate`): the power density (in dB) of each
/// band over its quietest tenth of frames, which is the hiss or hum left when nothing else
/// sounds. Frames of plain silence (a fade, a gap in the file) are not counted. `None` when the
/// sound is too short to tell or silent throughout.
pub fn noise_floor(sound: &[f32], rate: u32) -> Option<[f32; BANDS]> {
    if sound.len() < FRAME * 8 {
        return None;
    }
    let window = window();
    // each frame's power in all and by band
    let mut frames: Vec<(f32, [f32; BANDS])> = Vec::new();
    let mut count = [0u32; BANDS];
    for frame in sound.windows(FRAME).step_by(FRAME / 2) {
        let density = frame_density(frame, &window, rate);
        let mut sum = [0.0f32; BANDS];
        count = [0; BANDS];
        for (i, d) in density.iter().enumerate().skip(1) {
            let b = band_of(i as f32 * rate as f32 / FRAME as f32);
            sum[b] += d;
            count[b] += 1;
        }
        let total: f32 = frame.iter().map(|s| s * s).sum::<f32>() / FRAME as f32;
        // under -90 dBFS nothing was recorded
        if total > 1e-9 {
            frames.push((total, sum));
        }
    }
    if frames.is_empty() {
        return None;
    }
    frames.sort_by(|a, b| a.0.total_cmp(&b.0));
    let quiet = &frames[..(frames.len() / 10).max(1)];
    let mut floor = [-140.0f32; BANDS];
    for b in 0..BANDS {
        if count[b] > 0 {
            let mean = quiet.iter().map(|(_, sum)| sum[b]).sum::<f32>() / (quiet.len() as f32 * count[b] as f32);
            floor[b] = (10.0 * (mean + 1e-14).log10()).max(-140.0);
        }
    }
    // a band with no bins of its own (below the first bin) takes its neighbour's
    for b in (0..BANDS - 1).rev() {
        if count[b] == 0 {
            floor[b] = floor[b + 1];
        }
    }
    Some(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amp: f32, secs: f32) -> Vec<f32> {
        (0..(RATE as f32 * secs) as usize)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// A number from -1 to 1, the same every run.
    fn noise(i: usize) -> f32 {
        let mut z = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }

    #[test]
    fn loudness_follows_the_standard() {
        // a full-scale 997 Hz sine in one channel reads -3.01 LUFS
        let full = tone(997.0, 1.0, 3.0);
        let l = loudness(std::slice::from_ref(&full)).unwrap();
        assert!((l - -3.01).abs() < 0.1, "{l}");
        // in both channels, 3 dB more; 20 dB down, 20 LU less
        let both = loudness(&[full.clone(), full.clone()]).unwrap();
        assert!((both - l - 3.01).abs() < 0.05, "{both}");
        let quiet = loudness(&[tone(997.0, 0.1, 3.0)]).unwrap();
        assert!((quiet - l + 20.0).abs() < 0.05, "{quiet}");
        // silence between does not pull the reading down: it is gated out
        let mut gapped = full.clone();
        gapped.extend(vec![0.0; RATE as usize * 3]);
        gapped.extend(full.clone());
        assert!((loudness(&[gapped]).unwrap() - l).abs() < 0.5);
        assert_eq!(loudness(&[vec![0.0; 48_000]]), None);
        assert_eq!(loudness(&[]), None);
        assert_eq!(peak(&[tone(100.0, 0.5, 0.1)]).max(0.49), peak(&[tone(100.0, 0.5, 0.1)]));
        assert_eq!(mono(&[vec![1.0, 0.0], vec![0.0, 1.0]]), [0.5, 0.5]);
    }

    #[test]
    fn beats_are_found_at_the_songs_tempo() {
        for bpm in [90.0f32, 120.0, 150.0] {
            let secs = 12.0;
            let gap = 60.0 / bpm;
            let mut song = vec![0f32; (RATE as f32 * secs) as usize];
            // a kick on every beat (the first a little in), a quieter tick on the off-beats, and hiss
            let mut t = 0.25;
            while t < secs - 0.3 {
                let at = (t * RATE as f32) as usize;
                for j in 0..4800 {
                    let env = (-(j as f32) / 900.0).exp();
                    song[at + j] += 0.8 * env * (std::f32::consts::TAU * 60.0 * j as f32 / RATE as f32).sin();
                }
                let off = at + (gap * 0.5 * RATE as f32) as usize;
                for j in 0..600 {
                    if off + j < song.len() {
                        song[off + j] += 0.1 * (-(j as f32) / 100.0).exp() * noise(j);
                    }
                }
                t += gap;
            }
            for (i, s) in song.iter_mut().enumerate() {
                *s += 0.002 * noise(i);
            }
            let found = beats(&song, RATE).expect("a beat");
            assert!((found.bpm - bpm).abs() < 2.0, "{bpm}: {}", found.bpm);
            // every beat found is on a kick, and nearly every kick is found
            let near = |sample: i64| {
                let x = (sample as f32 / RATE as f32 - 0.25) / gap;
                (x - x.round()).abs() * gap
            };
            let on: usize = found.at.iter().filter(|b| near(**b) < 0.03).count();
            assert!(on as f32 >= found.at.len() as f32 * 0.9, "{bpm}: {on} of {}", found.at.len());
            assert!(found.at.len() as f32 >= (secs - 0.6) / gap * 0.9, "{bpm}: {}", found.at.len());
        }
        // a steady tone has no beat, and a moment is too short to tell
        assert_eq!(beats(&tone(440.0, 0.5, 6.0), RATE), None);
        assert_eq!(beats(&tone(440.0, 0.5, 0.5), RATE), None);
        // nor do two words some seconds apart
        let mut words: Vec<f32> = (0..RATE as usize * 6).map(|i| 0.01 * noise(i)).collect();
        for (i, s) in words.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            if (1.0..2.0).contains(&t) || (4.0..5.0).contains(&t) {
                *s += 0.1 * (std::f32::consts::TAU * 220.0 * t).sin();
            }
        }
        assert_eq!(beats(&words, RATE), None);
    }

    #[test]
    fn what_is_heard_is_told_from_the_quiet_between() {
        let r = RATE as usize;
        let mut sound: Vec<f32> = (0..r * 6).map(|i| 0.001 * noise(i)).collect();
        // speech from 1 s to 2.5 s with a short breath in it, a blip at 4 s, speech from 5 s
        for (a, b) in [(1.0, 1.7), (1.9, 2.5), (4.0, 4.05), (5.0, 5.8)] {
            let (a, b) = ((a * r as f32) as usize, (b * r as f32) as usize);
            for (i, s) in sound.iter_mut().enumerate().take(b).skip(a) {
                *s += 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / r as f32).sin();
            }
        }
        let spans: Vec<(f32, f32)> =
            heard_spans(&sound, RATE).into_iter().map(|(a, b)| (a as f32 / r as f32, b as f32 / r as f32)).collect();
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!((spans[0].0 - 0.9).abs() < 0.05 && (spans[0].1 - 2.6).abs() < 0.05, "{spans:?}");
        assert!((spans[1].0 - 4.9).abs() < 0.05 && (spans[1].1 - 5.9).abs() < 0.05, "{spans:?}");
        // over a noisy room (16 dB under the voice) the words are still told apart
        let noisy: Vec<f32> = sound.iter().enumerate().map(|(i, s)| s + 0.08 * noise(i)).collect();
        let spans = heard_spans(&noisy, RATE);
        assert_eq!(spans.len(), 2, "{spans:?}");
        // and a sound with no quiet in it is heard from end to end
        let steady: Vec<f32> = (0..r * 2).map(|i| 0.2 * noise(i)).collect();
        assert_eq!(heard_spans(&steady, RATE), [(0, (r * 2) as i64)]);
        assert!(heard_spans(&[], RATE).is_empty());
        assert!(heard_spans(&vec![0.0; r], RATE).is_empty());
    }

    #[test]
    fn the_transform_goes_there_and_back_and_finds_a_tone() {
        let x: Vec<f32> = (0..FRAME).map(noise).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; FRAME]);
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in x.iter().zip(&re) {
            assert!((a - b).abs() < 1e-4);
        }
        // a tone is in its bin
        let hz = 3000.0;
        let d = frame_density(&tone(hz, 0.5, 0.1)[..FRAME], &window(), RATE);
        let top = (0..d.len()).max_by(|a, b| d[*a].total_cmp(&d[*b])).unwrap();
        assert_eq!(top, (hz / RATE as f32 * FRAME as f32).round() as usize);
        // windows half a frame apart, used twice, add up to one
        let w = window();
        for j in 0..FRAME / 2 {
            assert!((w[j] * w[j] + w[j + FRAME / 2] * w[j + FRAME / 2] - 1.0).abs() < 1e-5);
        }
        assert_eq!((band_of(10.0), band_of(60.0), band_of(20_000.0)), (0, 0, BANDS - 1));
        assert!(band_of(1000.0) > band_of(200.0));
    }

    #[test]
    fn the_noise_floor_is_the_hiss_not_the_voice() {
        let r = RATE as usize;
        // hiss throughout, a loud tone for half of it
        let hiss = |i: usize| 0.01 * noise(i);
        let mut sound: Vec<f32> = (0..r * 4).map(hiss).collect();
        let alone = noise_floor(&sound, RATE).unwrap();
        for (i, s) in sound.iter_mut().enumerate().take(r * 2) {
            *s += 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / r as f32).sin();
        }
        let with_voice = noise_floor(&sound, RATE).unwrap();
        for b in 0..BANDS {
            assert!((alone[b] - with_voice[b]).abs() < 3.0, "band {b}: {} / {}", alone[b], with_voice[b]);
        }
        // white noise of amplitude 0.01 (power a third of that squared) spread over the sample rate
        let expect = 10.0 * (0.01f32 * 0.01 / 3.0 / 48_000.0).log10();
        let mid = alone[band_of(2000.0)];
        assert!((mid - expect).abs() < 1.5, "{mid} / {expect}");
        // ten times quieter, 20 dB lower
        let quieter: Vec<f32> = (0..r * 4).map(|i| 0.001 * noise(i)).collect();
        let q = noise_floor(&quieter, RATE).unwrap();
        assert!((alone[10] - q[10] - 20.0).abs() < 1.0);
        assert_eq!(noise_floor(&[0.0; 100], RATE), None);
        assert_eq!(noise_floor(&vec![0.0; r], RATE), None);
    }
}
