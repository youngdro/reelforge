//! Sound for the preview: the export's own mix ([`reelforge_media::export::mix`]), made a little
//! ahead of the playhead and played on the default output device (WebAudio in the browser).
//!
//! While sound plays, the device sets the pace: the playhead is where the samples the device has
//! taken put it, so picture follows sound. Without a device (none, or it failed) the editor moves
//! the playhead by the frame clock, silently.
//!
//! Mixing runs on the editor's thread, a chunk at a time, when a frame is drawn (the browser has no
//! other thread to give it); the device's callback only copies from the queue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use reelforge_media::Footage;
use reelforge_renderspec::RenderSpec;
use reelforge_time::{TICKS_PER_SECOND, Tick};

/// How much sound to keep mixed ahead of the device, in seconds. Enough to ride out a slow frame.
const AHEAD_SECONDS: f64 = 0.25;
/// Samples mixed at a time.
const CHUNK: usize = 1024;
/// The rate used when there is no device.
const FALLBACK_RATE: u32 = 48_000;

/// Mixed samples on their way to the device, and how many it has taken.
#[derive(Default)]
struct Queue {
    frames: VecDeque<[f32; 2]>,
    /// Frames taken since the last start.
    played: u64,
    /// Whether the device should take frames (it plays silence otherwise).
    running: bool,
}

/// Preview sound.
pub struct Playback {
    rate: u32,
    queue: Arc<Mutex<Queue>>,
    /// The playhead when sound started.
    origin: Tick,
    /// The next output sample to mix.
    cursor: i64,
    device: Device,
    /// Why there is no sound, once said.
    problem: Option<String>,
}

enum Device {
    /// Not opened yet: the browser only allows sound after the person has clicked something.
    Closed,
    /// Playing (or ready to). Held, not read: dropping it closes the device.
    #[allow(dead_code)]
    Open(cpal::Stream),
    /// No device, or none that works; the playhead runs on the frame clock.
    None,
}

impl Default for Playback {
    fn default() -> Playback {
        Playback {
            rate: FALLBACK_RATE,
            queue: Arc::default(),
            origin: Tick::ZERO,
            cursor: 0,
            device: Device::Closed,
            problem: None,
        }
    }
}

impl Playback {
    /// Sound with no device, for tests: frames are taken with [`take`](Self::take).
    #[cfg(test)]
    pub fn silent(rate: u32) -> Playback {
        Playback { rate, device: Device::None, ..Playback::default() }
    }

    /// Opens the device, if that has not been tried. Opening can take a moment (a second or more
    /// through some sound servers), so the desktop does it before the first Play; the browser only
    /// allows it after the person has clicked, so there the first Play does it.
    pub fn prepare(&mut self) {
        if matches!(self.device, Device::Closed) {
            self.device = match self.open() {
                Ok(stream) => Device::Open(stream),
                Err(e) => {
                    self.problem = Some(format!("no sound: {e}"));
                    Device::None
                }
            };
        }
    }

    /// Starts sound at `at`, opening the device the first time.
    pub fn start(&mut self, at: Tick) {
        self.prepare();
        self.origin = at;
        self.cursor = sample_at(at, self.rate);
        let mut q = self.lock();
        q.frames.clear();
        q.played = 0;
        q.running = true;
    }

    /// Stops sound; what was mixed ahead is dropped.
    pub fn stop(&mut self) {
        let mut q = self.lock();
        q.running = false;
        q.frames.clear();
    }

    /// Whether sound is running.
    pub fn running(&self) -> bool {
        self.lock().running
    }

    /// Why there is no sound, once (for the status bar).
    pub fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
    }

    /// Mixes `spec` until the queue holds a little more than the device needs. A clip whose file
    /// cannot be read is left silent.
    pub fn fill(&mut self, spec: &RenderSpec, media: &Footage) {
        let want = (self.rate as f64 * AHEAD_SECONDS) as usize;
        if !self.running() {
            return;
        }
        while self.lock().frames.len() < want {
            let planar = match reelforge_media::export::mix(spec, media, self.rate, self.cursor, CHUNK) {
                Ok(p) => p,
                Err(e) => {
                    self.problem.get_or_insert_with(|| format!("sound: {e}"));
                    vec![vec![0.0; CHUNK]; 2]
                }
            };
            self.cursor += CHUNK as i64;
            let right = planar.len().min(2).saturating_sub(1);
            self.lock().frames.extend((0..CHUNK).map(|i| [planar[0][i], planar[right][i]]));
        }
    }

    /// Where the sound the device has taken puts the playhead, when a device sets the pace.
    pub fn clock(&self) -> Option<Tick> {
        if !matches!(self.device, Device::Open(_)) {
            return None;
        }
        Some(self.position())
    }

    fn position(&self) -> Tick {
        let played = self.lock().played as i128;
        self.origin + Tick((played * TICKS_PER_SECOND as i128 / self.rate as i128) as i64)
    }

    /// Takes `n` frames as a device would.
    #[cfg(test)]
    pub fn take(&self, n: usize) -> (Vec<[f32; 2]>, Tick) {
        let mut out = vec![[0.0; 2]; n];
        pull(&self.queue, &mut out);
        (out, self.position())
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    fn open(&mut self) -> Result<cpal::Stream, String> {
        Err("tests play no sound".into())
    }

    #[cfg(not(test))]
    fn open(&mut self) -> Result<cpal::Stream, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device().ok_or("no output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        self.rate = config.sample_rate as u32;
        let channels = config.channels as usize;
        let queue = self.queue.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, channels, queue),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, channels, queue),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, channels, queue),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, channels, queue),
            other => return Err(format!("the device wants {other} samples")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    }
}

#[cfg(not(test))]
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    queue: Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let mut frames: Vec<[f32; 2]> = Vec::new();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let n = data.len() / channels.max(1);
                frames.clear();
                frames.resize(n, [0.0; 2]);
                pull(&queue, &mut frames);
                for (out, [l, r]) in data.chunks_mut(channels.max(1)).zip(&frames) {
                    match out {
                        [mono] => *mono = T::from_sample((l + r) * 0.5),
                        [a, b, rest @ ..] => {
                            *a = T::from_sample(*l);
                            *b = T::from_sample(*r);
                            rest.iter_mut().for_each(|s| *s = T::from_sample(0.0));
                        }
                        [] => {}
                    }
                }
            },
            |e| eprintln!("reelforge: sound: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Fills `out` from the queue (silence past what is mixed, or when stopped) and counts what was
/// taken.
fn pull(queue: &Mutex<Queue>, out: &mut [[f32; 2]]) {
    let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
    if !q.running {
        out.fill([0.0; 2]);
        return;
    }
    let n = out.len().min(q.frames.len());
    for (o, f) in out.iter_mut().zip(q.frames.drain(..n)) {
        *o = f;
    }
    out[n..].fill([0.0; 2]);
    q.played += n as u64;
}

fn sample_at(t: Tick, rate: u32) -> i64 {
    (t.0 as i128 * rate as i128 / TICKS_PER_SECOND as i128) as i64
}
