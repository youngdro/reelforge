//! Recording a voice-over from the microphone: mono samples collected until stopped, then handed
//! over as a WAV file like any dropped file. The desktop records through cpal; the browser through
//! `getUserMedia` (it asks the person first).

/// What a finished recording is: mono samples and their rate.
pub(crate) struct Take {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Take {
    /// The take as a WAV file.
    pub fn wav(&self) -> Vec<u8> {
        reelforge_media::library::encode_wav(&self.samples, 1, self.rate)
    }

    /// Seconds recorded.
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.rate.max(1))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Recorder;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::Recorder;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::Take;

    /// A recording in progress.
    pub(crate) struct Recorder {
        _stream: cpal::Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        rate: u32,
    }

    impl Recorder {
        /// Starts recording from the default input.
        pub fn start() -> Result<Recorder, String> {
            let device = cpal::default_host().default_input_device().ok_or("no microphone")?;
            let config = device.default_input_config().map_err(|e| e.to_string())?;
            let format = config.sample_format();
            let config: cpal::StreamConfig = config.into();
            let channels = usize::from(config.channels).max(1);
            let samples: Arc<Mutex<Vec<f32>>> = Arc::default();
            let stream = match format {
                cpal::SampleFormat::F32 => build::<f32>(&device, &config, channels, samples.clone()),
                cpal::SampleFormat::I16 => build::<i16>(&device, &config, channels, samples.clone()),
                cpal::SampleFormat::U16 => build::<u16>(&device, &config, channels, samples.clone()),
                cpal::SampleFormat::I32 => build::<i32>(&device, &config, channels, samples.clone()),
                other => return Err(format!("the microphone gives {other} samples")),
            }?;
            stream.play().map_err(|e| e.to_string())?;
            Ok(Recorder { _stream: stream, samples, rate: config.sample_rate })
        }

        /// Seconds so far.
        pub fn seconds(&self) -> f64 {
            self.samples.lock().map_or(0, |s| s.len()) as f64 / f64::from(self.rate.max(1))
        }

        /// Stops and gives the take.
        pub fn stop(self) -> Result<Take, String> {
            let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
            Ok(Take { samples, rate: self.rate })
        }

        /// Why recording could not start (the desktop knows at once).
        pub fn failed(&self) -> Option<String> {
            None
        }
    }

    fn build<T: cpal::SizedSample>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        samples: Arc<Mutex<Vec<f32>>>,
    ) -> Result<cpal::Stream, String>
    where
        f32: cpal::FromSample<T>,
    {
        use cpal::Sample;
        device
            .build_input_stream(
                *config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    let mut out = samples.lock().unwrap_or_else(|e| e.into_inner());
                    // the channels mixed down to one
                    for frame in data.chunks(channels) {
                        let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                        out.push(sum / channels as f32);
                    }
                },
                |e| eprintln!("reelforge: microphone: {e}"),
                None,
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    use super::Take;

    #[wasm_bindgen(inline_js = r#"
let rec = null;
export async function rf_rec_start() {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
  const ctx = new AudioContext();
  const src = ctx.createMediaStreamSource(stream);
  const node = ctx.createScriptProcessor(4096, 1, 1);
  const chunks = [];
  node.onaudioprocess = (e) => { chunks.push(new Float32Array(e.inputBuffer.getChannelData(0))); };
  src.connect(node);
  node.connect(ctx.destination);
  rec = { stream, ctx, node, chunks };
  return ctx.sampleRate;
}
export function rf_rec_length() {
  return rec ? rec.chunks.reduce((n, c) => n + c.length, 0) : 0;
}
export function rf_rec_stop() {
  if (!rec) return new Float32Array(0);
  rec.node.disconnect();
  rec.stream.getTracks().forEach((t) => t.stop());
  rec.ctx.close();
  const n = rec.chunks.reduce((n, c) => n + c.length, 0);
  const out = new Float32Array(n);
  let at = 0;
  for (const c of rec.chunks) { out.set(c, at); at += c.length; }
  rec = null;
  return out;
}
"#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        fn rf_rec_start() -> Result<js_sys::Promise, JsValue>;
        fn rf_rec_length() -> f64;
        fn rf_rec_stop() -> js_sys::Float32Array;
    }

    /// A recording in progress: the browser may still be asking for the microphone.
    pub(crate) struct Recorder {
        /// The sample rate once recording, or why it could not start.
        state: Rc<RefCell<Option<Result<u32, String>>>>,
    }

    impl Recorder {
        /// Asks for the microphone and starts recording.
        pub fn start() -> Result<Recorder, String> {
            let promise = rf_rec_start().map_err(|e| format!("{e:?}"))?;
            let state: Rc<RefCell<Option<Result<u32, String>>>> = Rc::default();
            let set = state.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let r = JsFuture::from(promise)
                    .await
                    .map(|v| v.as_f64().unwrap_or(48_000.0) as u32)
                    .map_err(|e| e.as_string().unwrap_or_else(|| "the microphone was refused".into()));
                *set.borrow_mut() = Some(r);
            });
            Ok(Recorder { state })
        }

        /// Seconds so far.
        pub fn seconds(&self) -> f64 {
            match &*self.state.borrow() {
                Some(Ok(rate)) => rf_rec_length() / f64::from(*rate),
                _ => 0.0,
            }
        }

        /// Stops and gives the take, or why there is none.
        pub fn stop(self) -> Result<Take, String> {
            let rate = match self.state.borrow().clone() {
                Some(Ok(rate)) => rate,
                Some(Err(e)) => return Err(e),
                None => return Err("the microphone was not ready".into()),
            };
            Ok(Take { samples: rf_rec_stop().to_vec(), rate })
        }

        /// Why recording could not start, once known.
        pub fn failed(&self) -> Option<String> {
            match &*self.state.borrow() {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            }
        }
    }
}
