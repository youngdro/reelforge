//! Local speech recognition: Whisper, run in pure Rust on [candle](https://github.com/huggingface/candle),
//! on the CPU of the machine that holds the media (the desktop app, or the server a web editor is
//! joined to). Nothing leaves that machine.
//!
//! A model is a folder of the three files Hugging Face publishes for every Whisper size
//! (`config.json`, `model.safetensors`, `tokenizer.json`), such as `openai/whisper-small`. The mel
//! filters are computed here, and the tokenizer is read without a tokenizer library: decoding only
//! needs each token's bytes.
//!
//! Decoding is greedy with Whisper's timestamp rules, 30 s at a time; text left open at the end of a
//! window is decoded again from its start in the next one. A window Whisper thinks holds no speech
//! is skipped.

mod mel;
#[doc(hidden)]
pub mod tiny;
mod tokens;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use candle_core::{D, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as w, Config, model::Whisper as Model};
use reelforge_media::Footage;
use reelforge_renderspec::RenderSpec;
use reelforge_time::{TICKS_PER_SECOND, Tick};

pub use mel::SAMPLE_RATE;
use tokens::{Vocab, apply_rules, split};

/// Said from `start` to `end`.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// From here.
    pub start: Tick,
    /// To here.
    pub end: Tick,
    /// What was said.
    pub text: String,
}

/// What a recording says.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    /// The language it was heard in (`zh`, `en`, …).
    pub language: String,
    /// What was said, in time order.
    pub segments: Vec<Segment>,
}

/// How to listen.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The language spoken (`zh`, `en`, …); `None` lets the model tell from the first 30 s.
    pub language: Option<String>,
    /// Words that come before the recording, to set the style (punctuation, simplified or
    /// traditional characters, names). `None` uses [`default_prompt`] for the language.
    pub prompt: Option<String>,
}

/// The prompt used when none is given: for Chinese, a sentence in simplified characters with
/// full-width punctuation, which keeps Whisper from answering in traditional ones.
pub fn default_prompt(language: &str) -> Option<&'static str> {
    (language == "zh").then_some("以下是普通话的句子，使用简体中文和标点符号。")
}

/// A loaded Whisper model.
pub struct Whisper {
    model: Model,
    vocab: Vocab,
    filters: Vec<f32>,
    device: Device,
}

const NO_SPEECH_THRESHOLD: f32 = 0.6;
const LOGPROB_THRESHOLD: f32 = -1.0;
/// Timestamps are 0.02 s apart, two mel frames.
const FRAMES_PER_STEP: usize = 2;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Whisper {
    /// Loads the model in `dir` (`config.json`, `model.safetensors`, `tokenizer.json`).
    pub fn load(dir: &Path) -> Result<Whisper, String> {
        let read = |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()));
        let config = String::from_utf8(read("config.json")?).map_err(err)?;
        let tokenizer = String::from_utf8(read("tokenizer.json")?).map_err(err)?;
        Whisper::from_bytes(&config, read("model.safetensors")?, &tokenizer)
    }

    /// A model from the contents of its three files.
    pub fn from_bytes(config: &str, weights: Vec<u8>, tokenizer: &str) -> Result<Whisper, String> {
        let config: Config = serde_json::from_str(config).map_err(|e| format!("config.json: {e}"))?;
        let device = Device::Cpu;
        let vb = VarBuilder::from_buffered_safetensors(weights, w::DTYPE, &device).map_err(err)?;
        Whisper::new(config, vb, tokenizer)
    }

    fn new(config: Config, vb: VarBuilder, tokenizer: &str) -> Result<Whisper, String> {
        let vocab = Vocab::from_json(tokenizer)?;
        let device = vb.device().clone();
        let filters = mel::filters(config.num_mel_bins);
        let model = Model::load(&vb, config).map_err(err)?;
        Ok(Whisper { model, vocab, filters, device })
    }

    /// What `samples` (mono, [`SAMPLE_RATE`] Hz) say. `progress` hears the share done, from 0 to
    /// 1, after every window, and stops the work by answering `false` (the result is then an error).
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        options: &Options,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<Transcript, String> {
        let cfg = self.model.config.clone();
        let n_mels = cfg.num_mel_bins;
        let mel = w::audio::pcm_to_mel(&cfg, samples, &self.filters);
        let mel_len = mel.len() / n_mels;
        let mel = Tensor::from_vec(mel, (1, n_mels, mel_len), &self.device).map_err(err)?;
        let content = samples.len() / w::HOP_LENGTH;
        let mut language = options.language.clone();
        let mut segments = Vec::new();
        let mut seek = 0;
        while seek < content {
            let size = w::N_FRAMES.min(mel_len - seek);
            let window = mel.narrow(2, seek, size).map_err(err)?;
            let audio = self.model.encoder.forward(&window, true).map_err(err)?;
            let lang = match &language {
                Some(l) => l.clone(),
                None => language.insert(self.detect_language(&audio)?).clone(),
            };
            let steps = (size.min(content - seek) / FRAMES_PER_STEP) as u32;
            let prompt = options.prompt.as_deref().or(default_prompt(&lang));
            let decoded = self.decode(&audio, &lang, prompt)?;
            let (pieces, advance) = split(&decoded.tokens, &self.vocab, steps.max(1));
            let silent = decoded.no_speech > NO_SPEECH_THRESHOLD && decoded.avg_logprob < LOGPROB_THRESHOLD;
            if !silent {
                let at = |step: u32| {
                    let frame = (seek + step as usize * FRAMES_PER_STEP).min(content);
                    Tick((frame * w::HOP_LENGTH) as i64 * TICKS_PER_SECOND / SAMPLE_RATE as i64)
                };
                for p in pieces {
                    let text = self.vocab.text(&p.tokens).trim().to_string();
                    // the model may stamp past the end of the sound; those pieces are dropped
                    let (start, end) = (at(p.start), at(p.end));
                    if !text.is_empty() && end > start {
                        segments.push(Segment { start, end, text });
                    }
                }
            }
            let advance = if silent { steps.max(1) } else { advance };
            seek += advance as usize * FRAMES_PER_STEP;
            if !progress((seek.min(content) as f32 / content as f32).min(1.0)) {
                return Err("stopped".into());
            }
        }
        Ok(Transcript { language: language.unwrap_or_else(|| "en".into()), segments })
    }

    fn logits(&mut self, tokens: &[u32], audio: &Tensor, first: bool) -> Result<Tensor, String> {
        let t = Tensor::new(tokens, &self.device).and_then(|t| t.unsqueeze(0)).map_err(err)?;
        let ys = self.model.decoder.forward(&t, audio, first).map_err(err)?;
        self.model.decoder.final_linear(&ys).map_err(err)
    }

    /// The likeliest language of the window `audio`.
    fn detect_language(&mut self, audio: &Tensor) -> Result<String, String> {
        let langs = self.vocab.languages();
        if langs.is_empty() {
            return Ok("en".into());
        }
        let logits = self.logits(&[self.vocab.sot], audio, true)?;
        let last: Vec<f32> = logits.i((0, 0)).and_then(|t| t.to_vec1()).map_err(err)?;
        let best = langs
            .iter()
            .filter(|(_, id)| (*id as usize) < last.len())
            .max_by(|a, b| last[a.1 as usize].total_cmp(&last[b.1 as usize]))
            .map_or("en".to_string(), |(code, _)| code.clone());
        Ok(best)
    }

    fn decode(&mut self, audio: &Tensor, language: &str, prompt: Option<&str>) -> Result<Decoded, String> {
        let v = &self.vocab;
        let cfg = &self.model.config;
        let mut tokens = Vec::new();
        if let (Some(prompt), Some(prev)) = (prompt, v.start_of_prev()) {
            let words = v.encode(prompt);
            let keep = cfg.max_target_positions / 2 - 1;
            tokens.push(prev);
            tokens.extend(&words[words.len().saturating_sub(keep)..]);
        }
        let sot_at = tokens.len();
        tokens.push(v.sot);
        if let Some(l) = v.language(language) {
            tokens.push(l);
        }
        tokens.push(v.transcribe);
        let prompt_len = tokens.len();
        let suppress = cfg.suppress_tokens.clone();
        let budget = (cfg.max_target_positions / 2).min(cfg.max_target_positions.saturating_sub(prompt_len));
        let (eot, no_speech_id) = (v.eot, v.no_speech);
        let mut no_speech = 0.0;
        let mut sum_logprob = 0.0;
        for i in 0..budget {
            let logits = self.logits(&tokens, audio, i == 0)?;
            let seq = logits.dim(1).map_err(err)?;
            if i == 0
                && let Some(ns) = no_speech_id
            {
                let probs = candle_nn::ops::softmax(&logits.i((0, sot_at)).map_err(err)?, D::Minus1).map_err(err)?;
                no_speech = probs.i(ns as usize).and_then(|p| p.to_scalar::<f32>()).map_err(err)?;
            }
            let mut last: Vec<f32> = logits.i((0, seq - 1)).and_then(|t| t.to_vec1()).map_err(err)?;
            apply_rules(&mut last, &tokens[prompt_len..], &self.vocab, &suppress, 50);
            let Some((next, _)) = last.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else { break };
            if !last[next].is_finite() {
                break;
            }
            sum_logprob += log_softmax_at(&last, next);
            let next = next as u32;
            if next == eot {
                break;
            }
            tokens.push(next);
        }
        let sampled = tokens.split_off(prompt_len);
        let avg_logprob = sum_logprob / (sampled.len() + 1) as f32;
        Ok(Decoded { tokens: sampled, no_speech, avg_logprob })
    }
}

struct Decoded {
    tokens: Vec<u32>,
    no_speech: f32,
    avg_logprob: f32,
}

fn log_softmax_at(logits: &[f32], i: usize) -> f32 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|&l| (l - max).exp()).sum();
    logits[i] - max - sum.ln()
}

/// The timeline's sound from `from` to `to`, mixed to mono at [`SAMPLE_RATE`] Hz, ready for
/// [`Whisper::transcribe`]. Offline media are silent.
pub fn timeline_audio(spec: &RenderSpec, media: &Footage, from: Tick, to: Tick) -> Result<Vec<f32>, String> {
    let at = |t: Tick| (t.0 as i128 * SAMPLE_RATE as i128 / TICKS_PER_SECOND as i128) as i64;
    let (start, end) = (at(from.max(Tick::ZERO)), at(to.min(spec.output.duration)));
    let mut out = Vec::with_capacity((end - start).max(0) as usize);
    let chunk = 10 * SAMPLE_RATE as i64;
    let mut s = start;
    while s < end {
        let n = chunk.min(end - s) as usize;
        let mix = reelforge_media::export::mix(spec, media, SAMPLE_RATE, s, n).map_err(err)?;
        let k = mix.len().max(1) as f32;
        out.extend((0..n).map(|i| mix.iter().map(|ch| ch[i]).sum::<f32>() / k));
        s += n as i64;
    }
    Ok(out)
}

/// Where the model is looked for: `$REELFORGE_WHISPER`, else `.reelforge/models/whisper` in the
/// home folder.
pub fn default_model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("REELFORGE_WHISPER") {
        return dir.into();
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".reelforge").join("models").join("whisper")
}

/// A model folder, loaded the first time it is needed and kept.
pub struct Recogniser {
    dir: PathBuf,
    model: Mutex<Option<Whisper>>,
}

impl Recogniser {
    /// Listens with the model in `dir`.
    pub fn new(dir: PathBuf) -> Recogniser {
        Recogniser { dir, model: Mutex::new(None) }
    }

    /// The model folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the model is there; the error says where to put it.
    pub fn check(&self) -> Result<(), String> {
        if is_model_dir(&self.dir) {
            return Ok(());
        }
        Err(format!(
            "no speech model in {}: put a Whisper model's config.json, model.safetensors and tokenizer.json there (for example from huggingface.co/openai/whisper-small), or set REELFORGE_WHISPER",
            self.dir.display()
        ))
    }

    /// What the timeline of `spec` says from `from` to `to` (the whole of it by default). One
    /// transcription runs at a time.
    pub fn transcribe(
        &self,
        spec: &RenderSpec,
        media: &Footage,
        range: (Option<Tick>, Option<Tick>),
        options: &Options,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<Transcript, String> {
        let from = range.0.unwrap_or(Tick::ZERO);
        let mut model = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if model.is_none() {
            self.check()?;
            *model = Some(Whisper::load(&self.dir)?);
        }
        let samples = timeline_audio(spec, media, from, range.1.unwrap_or(spec.output.duration))?;
        let mut out = model.as_mut().expect("loaded above").transcribe(&samples, options, progress)?;
        for s in &mut out.segments {
            s.start += from;
            s.end += from;
        }
        Ok(out)
    }
}

/// `captions.add` parameters placing `segments`, on `track` (a new captions track by default) in
/// `template`'s look.
pub fn captions_params(segments: &[Segment], track: Option<&str>, template: Option<&str>) -> serde_json::Value {
    let cues: Vec<serde_json::Value> =
        segments.iter().map(|s| serde_json::json!({"start": s.start.0, "end": s.end.0, "text": s.text})).collect();
    let mut out = serde_json::json!({ "cues": cues });
    if let Some(t) = track {
        out["track"] = t.into();
    }
    if let Some(t) = template {
        out["template"] = t.into();
    }
    out
}

/// Whether `dir` holds a model's three files.
pub fn is_model_dir(dir: &Path) -> bool {
    ["config.json", "model.safetensors", "tokenizer.json"].iter().all(|f| dir.join(f).is_file())
}

#[cfg(test)]
mod tests;
