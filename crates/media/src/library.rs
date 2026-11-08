//! The built-in sound library: sound effects and music beds made by synthesis, so they need no
//! files, no downloads and no licences. A project refers to one as `library:<id>`; [`Footage`]
//! makes it when it is first played, the same bytes every time (so its hash is stable).
//!
//! [`Footage`]: crate::Footage

use std::f32::consts::{PI, TAU};

/// The URI scheme of library sounds.
pub const SCHEME: &str = "library:";

/// The sample rate sounds are made at.
pub const RATE: u32 = 48_000;

/// What a sound is for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Category {
    /// A short effect.
    Effect,
    /// A music bed to put under a video.
    Music,
}

/// A library sound.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LibrarySound {
    /// The id commands use.
    pub id: &'static str,
    /// Effect or music.
    pub category: Category,
    /// English name.
    pub en: &'static str,
    /// Chinese name.
    pub zh: &'static str,
}

const fn effect(id: &'static str, en: &'static str, zh: &'static str) -> LibrarySound {
    LibrarySound { id, category: Category::Effect, en, zh }
}

const fn music(id: &'static str, en: &'static str, zh: &'static str) -> LibrarySound {
    LibrarySound { id, category: Category::Music, en, zh }
}

/// Every sound, effects first.
pub const SOUNDS: [LibrarySound; 16] = [
    effect("whoosh", "Whoosh", "嗖"),
    effect("swipe", "Swipe", "划过"),
    effect("riser", "Riser", "上升"),
    effect("boom", "Boom", "轰"),
    effect("pop", "Pop", "啵"),
    effect("click", "Click", "咔哒"),
    effect("ding", "Ding", "叮"),
    effect("chime", "Chime", "风铃"),
    effect("shutter", "Camera shutter", "快门"),
    effect("typing", "Typing", "打字"),
    effect("beep", "Beep", "哔哔"),
    effect("coin", "Coin", "金币"),
    music("lofi", "Lo-fi", "低保真"),
    music("upbeat", "Upbeat", "轻快"),
    music("ambient", "Ambient", "氛围"),
    music("cinematic", "Cinematic", "电影感"),
];

/// The sound with this id.
pub fn find(id: &str) -> Option<&'static LibrarySound> {
    SOUNDS.iter().find(|s| s.id == id)
}

/// The id in a `library:` URI.
pub fn id_of(uri: &str) -> Option<&str> {
    uri.strip_prefix(SCHEME).filter(|id| find(id).is_some())
}

/// A sound as a 16-bit stereo WAV file.
pub fn wav(id: &str) -> Option<Vec<u8>> {
    let [l, r] = render(id)?;
    let interleaved: Vec<f32> = l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect();
    Some(encode_wav(&interleaved, 2, RATE))
}

/// Interleaved samples (-1 to 1) as a 16-bit PCM WAV file.
pub fn encode_wav(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    out
}

/// A sound's samples, left and right, at [`RATE`], peaking at -1 dBFS.
pub fn render(id: &str) -> Option<[Vec<f32>; 2]> {
    let mut s = Synth::new(match id {
        "whoosh" => 0.9,
        "swipe" => 0.4,
        "riser" => 2.2,
        "boom" => 1.8,
        "pop" => 0.2,
        "click" => 0.06,
        "ding" => 1.6,
        "chime" => 2.2,
        "shutter" => 0.35,
        "typing" => 2.0,
        "beep" => 0.45,
        "coin" => 0.6,
        "lofi" => bars(85.0, 8),
        "upbeat" => bars(120.0, 8),
        "ambient" => 16.0,
        "cinematic" => 16.0,
        _ => return None,
    });
    match id {
        "whoosh" => whoosh(&mut s, 0.0, 0.9, 300.0, 3500.0),
        "swipe" => whoosh(&mut s, 0.0, 0.4, 1500.0, 7000.0),
        "riser" => riser(&mut s),
        "boom" => boom(&mut s, 0.0, 1.0),
        "pop" => pop(&mut s, 0.0),
        "click" => click(&mut s, 0.0, 1.0),
        "ding" => bell(&mut s, 0.0, 1320.0, 1.0),
        "chime" => {
            for (i, f) in [1046.5, 1318.5, 1568.0, 2093.0].into_iter().enumerate() {
                bell(&mut s, i as f32 * 0.12, f, 0.7);
            }
        }
        "shutter" => {
            click(&mut s, 0.0, 1.0);
            noise_burst(&mut s, 0.02, 0.12, 0.5, 2500.0);
            click(&mut s, 0.16, 0.8);
        }
        "typing" => {
            let mut t = 0.0;
            while t < 1.9 {
                let g = 0.5 + 0.5 * s.rand();
                click(&mut s, t, g);
                t += 0.06 + 0.1 * s.rand();
            }
        }
        "beep" => {
            tone(&mut s, 0.0, 0.15, 1000.0, 0.5);
            tone(&mut s, 0.25, 0.15, 1000.0, 0.5);
        }
        "coin" => {
            tone(&mut s, 0.0, 0.08, 987.8, 0.4);
            tone(&mut s, 0.08, 0.45, 1318.5, 0.4);
        }
        "lofi" => lofi(&mut s),
        "upbeat" => upbeat(&mut s),
        "ambient" => ambient(&mut s),
        "cinematic" => cinematic(&mut s),
        _ => unreachable!("lengths name every sound"),
    }
    Some(s.finish())
}

/// Seconds in `n` bars of 4/4 at `bpm`.
fn bars(bpm: f32, n: u32) -> f32 {
    n as f32 * 4.0 * 60.0 / bpm
}

/// A stereo buffer being written, with a fixed-seed noise source.
struct Synth {
    l: Vec<f32>,
    r: Vec<f32>,
    seed: u32,
}

impl Synth {
    fn new(seconds: f32) -> Synth {
        let n = (seconds * RATE as f32) as usize;
        Synth { l: vec![0.0; n], r: vec![0.0; n], seed: 0x2545_f491 }
    }

    /// 0 to 1.
    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// -1 to 1.
    fn noise(&mut self) -> f32 {
        self.rand() * 2.0 - 1.0
    }

    /// Adds `v` at sample `i`, panned (-1 left to 1 right).
    fn add(&mut self, i: usize, v: f32, pan: f32) {
        if i < self.l.len() {
            let a = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
            self.l[i] += v * a.cos();
            self.r[i] += v * a.sin();
        }
    }

    fn at(t: f32) -> usize {
        (t.max(0.0) * RATE as f32) as usize
    }

    /// Ends at -1 dBFS, with short fades (1 ms in, 5 ms out) so nothing clicks.
    fn finish(mut self) -> [Vec<f32>; 2] {
        let n = self.l.len();
        let (fin, fout) = ((RATE as usize / 1000).min(n / 2).max(1), (RATE as usize / 200).min(n / 2).max(1));
        for i in 0..n {
            let edge = (i as f32 / fin as f32).min((n - 1 - i) as f32 / fout as f32).min(1.0);
            self.l[i] *= edge;
            self.r[i] *= edge;
        }
        let peak = self.l.iter().chain(&self.r).fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        let k = 10f32.powf(-1.0 / 20.0) / peak;
        for v in self.l.iter_mut().chain(self.r.iter_mut()) {
            *v *= k;
        }
        [self.l, self.r]
    }
}

/// A two-pole resonant low-pass, its cutoff set per sample.
#[derive(Default)]
struct LowPass {
    a: f32,
    b: f32,
}

impl LowPass {
    fn run(&mut self, x: f32, cutoff: f32, q: f32) -> f32 {
        let f = (2.0 * (PI * cutoff / RATE as f32).sin()).min(1.0);
        let fb = q + q / (1.0 - f).max(0.01);
        self.a += f * (x - self.a + fb * (self.a - self.b));
        self.b += f * (self.a - self.b);
        self.b
    }
}

/// Noise swept from `lo` to `hi` and back down, moving left to right.
fn whoosh(s: &mut Synth, at: f32, len: f32, lo: f32, hi: f32) {
    let (mut lp, mut hp) = (LowPass::default(), LowPass::default());
    let n = Synth::at(len);
    for i in 0..n {
        let x = i as f32 / n as f32;
        let env = (PI * x).sin().powi(2);
        let cutoff = lo + (hi - lo) * (PI * x).sin();
        let v = s.noise();
        let band = lp.run(v, cutoff, 0.6) - hp.run(v, cutoff * 0.3, 0.0);
        s.add(Synth::at(at) + i, band * env, x * 1.6 - 0.8);
    }
}

fn riser(s: &mut Synth) {
    let n = s.l.len();
    let mut lp = LowPass::default();
    let mut phase = 0.0f32;
    for i in 0..n {
        let x = i as f32 / n as f32;
        let f = 120.0 * 2f32.powf(x * 4.0);
        phase = (phase + f / RATE as f32).fract();
        let saw = 2.0 * phase - 1.0;
        let v = lp.run(s.noise() * 0.6 + saw * 0.5, 400.0 + 8000.0 * x * x, 0.5);
        let env = x * x * if x > 0.97 { (1.0 - x) / 0.03 } else { 1.0 };
        s.add(i, v * env, 0.0);
    }
}

fn boom(s: &mut Synth, at: f32, gain: f32) {
    let n = Synth::at(1.6);
    let mut phase = 0.0f32;
    let mut lp = LowPass::default();
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let f = 35.0 + 50.0 * (-t * 8.0).exp();
        phase += TAU * f / RATE as f32;
        let body = phase.sin() * (-t * 2.5).exp();
        let hit = lp.run(s.noise(), 900.0, 0.2) * (-t * 18.0).exp();
        s.add(Synth::at(at) + i, gain * (body + 0.6 * hit), 0.0);
    }
}

fn pop(s: &mut Synth, at: f32) {
    let n = Synth::at(0.18);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        phase += TAU * (300.0 + 700.0 * (-t * 40.0).exp()) / RATE as f32;
        s.add(Synth::at(at) + i, phase.sin() * (-t * 30.0).exp(), 0.0);
    }
}

fn click(s: &mut Synth, at: f32, gain: f32) {
    let n = Synth::at(0.03);
    let mut lp = LowPass::default();
    let pan = s.noise() * 0.3;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let v = s.noise();
        let high = v - lp.run(v, 1500.0, 0.0);
        s.add(Synth::at(at) + i, gain * high * (-t * 220.0).exp(), pan);
    }
}

fn noise_burst(s: &mut Synth, at: f32, len: f32, gain: f32, cutoff: f32) {
    let n = Synth::at(len);
    let mut lp = LowPass::default();
    for i in 0..n {
        let x = i as f32 / n as f32;
        let v = lp.run(s.noise(), cutoff, 0.3);
        s.add(Synth::at(at) + i, gain * v * (1.0 - x), 0.0);
    }
}

/// An inharmonic struck bell.
fn bell(s: &mut Synth, at: f32, f: f32, gain: f32) {
    let n = Synth::at(1.5);
    for (ratio, amp, decay) in [(1.0, 1.0, 3.0), (2.76, 0.45, 6.0), (5.4, 0.25, 10.0), (8.93, 0.1, 16.0)] {
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let v = (TAU * f * ratio * t).sin() * amp * (-t * decay).exp() * (t * 2000.0).min(1.0);
            s.add(Synth::at(at) + i, gain * v * 0.5, 0.0);
        }
    }
}

/// A soft square tone.
fn tone(s: &mut Synth, at: f32, len: f32, f: f32, gain: f32) {
    let n = Synth::at(len);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let w = TAU * f * t;
        let v = w.sin() + (w * 3.0).sin() / 3.0 + (w * 5.0).sin() / 5.0;
        let env = (t * 400.0).min(1.0) * ((len - t) * 200.0).clamp(0.0, 1.0);
        s.add(Synth::at(at) + i, gain * v * env, 0.0);
    }
}

/// Note `m` (MIDI) in Hz.
fn hz(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

/// An electric-piano-like note.
fn keys(s: &mut Synth, at: f32, len: f32, m: f32, gain: f32, pan: f32) {
    let f = hz(m);
    let n = Synth::at(len + 0.6);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let env = (t * 300.0).min(1.0) * (-t * 1.6).exp() * if t > len { (-(t - len) * 8.0).exp() } else { 1.0 };
        let v = (TAU * f * t).sin()
            + 0.3 * (TAU * 2.0 * f * t).sin() * (-t * 6.0).exp()
            + 0.08 * (TAU * 3.0 * f * t + 0.3 * (TAU * 0.2 * t).sin()).sin();
        s.add(Synth::at(at) + i, gain * v * env, pan);
    }
}

fn kick(s: &mut Synth, at: f32, gain: f32) {
    let n = Synth::at(0.4);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        phase += TAU * (48.0 + 110.0 * (-t * 30.0).exp()) / RATE as f32;
        s.add(Synth::at(at) + i, gain * phase.sin() * (-t * 7.0).exp(), 0.0);
    }
}

fn hat(s: &mut Synth, at: f32, gain: f32, len: f32) {
    let n = Synth::at(len);
    let mut lp = LowPass::default();
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let v = s.noise();
        let high = v - lp.run(v, 6000.0, 0.0);
        s.add(Synth::at(at) + i, gain * high * (-t * 60.0 / len.max(0.02) * 0.05).exp(), 0.25);
    }
}

fn clap(s: &mut Synth, at: f32, gain: f32) {
    for k in 0..3 {
        noise_burst(s, at + k as f32 * 0.012, 0.12, gain * 0.6, 2200.0);
    }
}

/// A plucked, low-passed saw.
fn pluck(s: &mut Synth, at: f32, len: f32, m: f32, gain: f32, pan: f32, cutoff: f32) {
    let f = hz(m);
    let n = Synth::at(len);
    let mut lp = LowPass::default();
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        phase = (phase + f / RATE as f32).fract();
        let v = lp.run(2.0 * phase - 1.0, cutoff * (0.3 + (-t * 12.0).exp()), 0.3);
        let env = (t * 500.0).min(1.0) * (-t * 4.0).exp() * ((len - t) * 100.0).clamp(0.0, 1.0);
        s.add(Synth::at(at) + i, gain * v * env, pan);
    }
}

/// A slow pad: detuned sines swelling in and out.
fn pad(s: &mut Synth, at: f32, len: f32, notes: &[f32], gain: f32) {
    let n = Synth::at(len);
    for (j, &m) in notes.iter().enumerate() {
        let pan = (j as f32 / notes.len().max(1) as f32) * 1.2 - 0.6;
        for detune in [-0.06f32, 0.06] {
            let f = hz(m + detune);
            for i in 0..n {
                let t = i as f32 / RATE as f32;
                let x = t / len;
                let env = (PI * x).sin().powf(0.7);
                s.add(Synth::at(at) + i, gain * (TAU * f * t).sin() * env, pan + detune * 3.0);
            }
        }
    }
}

fn lofi(s: &mut Synth) {
    let beat = 60.0 / 85.0;
    // Cmaj7, Am7, Fmaj7, G7, twice
    let chords: [[f32; 4]; 4] =
        [[60.0, 64.0, 67.0, 71.0], [57.0, 60.0, 64.0, 67.0], [53.0, 57.0, 60.0, 64.0], [55.0, 59.0, 62.0, 65.0]];
    for bar in 0..8 {
        let t0 = bar as f32 * 4.0 * beat;
        for (k, &m) in chords[bar % 4].iter().enumerate() {
            keys(s, t0 + k as f32 * 0.02, 3.6 * beat, m, 0.12, k as f32 * 0.3 - 0.45);
        }
        keys(s, t0, 3.8 * beat, chords[bar % 4][0] - 24.0, 0.18, 0.0);
        for b in 0..4 {
            let t = t0 + b as f32 * beat;
            if b % 2 == 0 {
                kick(s, t, 0.5);
            } else {
                noise_burst(s, t, 0.15, 0.25, 1800.0);
            }
            // lazy swung hats
            hat(s, t, 0.08, 0.05);
            hat(s, t + beat * 0.62, 0.05, 0.05);
        }
    }
    // vinyl crackle
    let n = s.l.len();
    for i in 0..n {
        if s.rand() < 0.0001 {
            let v = s.noise() * 0.03;
            s.add(i, v, 0.0);
        }
    }
}

fn upbeat(s: &mut Synth) {
    let beat = 0.5;
    // C, G, Am, F
    let roots = [48.0, 55.0, 45.0, 53.0];
    let arps: [[f32; 3]; 4] = [[72.0, 76.0, 79.0], [71.0, 74.0, 79.0], [69.0, 72.0, 76.0], [69.0, 72.0, 77.0]];
    for bar in 0..8 {
        let t0 = bar as f32 * 4.0 * beat;
        for b in 0..4 {
            let t = t0 + b as f32 * beat;
            kick(s, t, 0.6);
            if b % 2 == 1 {
                clap(s, t, 0.5);
            }
            hat(s, t + beat / 2.0, 0.12, 0.04);
        }
        for e in 0..8 {
            let t = t0 + e as f32 * beat / 2.0;
            pluck(s, t, beat / 2.0 * 0.9, roots[bar % 4], 0.35, 0.0, 600.0);
            let note = arps[bar % 4][e % 3];
            pluck(s, t, beat / 2.0 * 0.8, note, 0.12, if e % 2 == 0 { -0.4 } else { 0.4 }, 3000.0);
        }
    }
}

fn ambient(s: &mut Synth) {
    // Cmaj9, Am9, Fmaj9, Gsus
    let chords: [&[f32]; 4] = [
        &[48.0, 55.0, 62.0, 64.0, 71.0],
        &[45.0, 52.0, 59.0, 60.0, 67.0],
        &[41.0, 48.0, 55.0, 57.0, 64.0],
        &[43.0, 50.0, 57.0, 60.0, 62.0],
    ];
    for (i, c) in chords.iter().enumerate() {
        pad(s, i as f32 * 4.0 - 0.5, 5.5, c, 0.05);
    }
    for k in 0..12 {
        let t = 0.7 + k as f32 * 1.3;
        let m = [79.0, 76.0, 74.0, 72.0, 71.0, 74.0][k % 6];
        bell(s, t, hz(m), 0.08);
    }
}

fn cinematic(s: &mut Synth) {
    pad(s, 0.0, 16.0, &[36.0, 43.0], 0.12);
    pad(s, 2.0, 14.0, &[60.0, 63.0, 67.0], 0.04);
    pad(s, 8.0, 8.0, &[58.0, 62.0, 65.0, 70.0], 0.04);
    for k in 0..4 {
        boom(s, k as f32 * 4.0, 0.8);
    }
    let n = s.l.len();
    let mut lp = LowPass::default();
    for i in (n * 3 / 4)..n {
        let x = (i - n * 3 / 4) as f32 / (n / 4) as f32;
        let v = lp.run(s.noise(), 300.0 + 3000.0 * x, 0.4) * x * 0.3;
        s.add(i, v, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_made_the_same_each_time_and_is_audible() {
        for snd in SOUNDS {
            let [l, r] = render(snd.id).unwrap();
            assert_eq!(l.len(), r.len());
            let peak = l.iter().chain(&r).fold(0f32, |m, v| m.max(v.abs()));
            assert!((0.88..=0.9).contains(&peak), "{}: peak {peak}", snd.id);
            assert!(l.iter().all(|v| v.is_finite()), "{}", snd.id);
            let rms = (l.iter().map(|v| v * v).sum::<f32>() / l.len() as f32).sqrt();
            assert!(rms > 0.005, "{}: rms {rms}", snd.id);
            assert_eq!(wav(snd.id), wav(snd.id), "{} is deterministic", snd.id);
        }
        assert_eq!(id_of("library:pop"), Some("pop"));
        assert_eq!(id_of("library:nope"), None);
        assert!(render("nope").is_none());
    }
}
