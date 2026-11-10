//! Local voice-over: text read aloud by Kokoro-82M, run in pure Rust on
//! [candle](https://github.com/huggingface/candle) on the CPU of the machine that holds the
//! project (the desktop app, or the server a browser editor is joined to). Nothing leaves it.
//!
//! A model is the folder Hugging Face publishes as `hexgrad/Kokoro-82M`: `config.json`, the
//! checkpoint `kokoro-v1_0.pth`, and `voices/*.pt`. Chinese is read by an in-tree Mandarin frontend
//! (numbers in Chinese, pinyin with tone sandhi; [`zh`]) into the phonemes Kokoro was trained on, so
//! the Chinese voices (`zf_*`, `zm_*`) are the ones to use.

mod kokoro;
mod layers;
pub mod zh;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::Tensor;
use candle_nn::VarBuilder;

use kokoro::config::KokoroConfig;
use kokoro::model::{Network, checkpoint, read_pth};

/// Samples per second of what Kokoro says.
pub const SAMPLE_RATE: u32 = 24_000;

/// The voice used when none is named: a clear female Mandarin voice.
pub const DEFAULT_VOICE: &str = "zf_xiaoxiao";

/// Longest phoneme run Kokoro takes at once (its context is 512 with the two pads).
const MAX_TOKENS: usize = 500;

/// A loaded Kokoro model and the voices beside it.
pub struct Kokoro {
    net: Network,
    voices_dir: PathBuf,
    voices: HashMap<String, Tensor>,
}

/// Where the model is looked for: `$REELFORGE_KOKORO`, else `.reelforge/models/kokoro` in the home
/// folder.
pub fn default_model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("REELFORGE_KOKORO") {
        return dir.into();
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".reelforge").join("models").join("kokoro")
}

/// Where the desktop editor keeps the lines it reads aloud: `.reelforge/voiceover` in the home
/// folder.
pub fn voiceover_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".reelforge").join("voiceover")
}

/// The checkpoint in `dir`: `kokoro-v1_0.pth`, or the only `.pth` there.
fn checkpoint_in(dir: &Path) -> Option<PathBuf> {
    let named = dir.join("kokoro-v1_0.pth");
    if named.is_file() {
        return Some(named);
    }
    let mut found = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pth"));
    found.next()
}

/// Whether the model is there; the error says where to put it.
pub fn check(dir: &Path) -> Result<(), String> {
    if dir.join("config.json").is_file() && checkpoint_in(dir).is_some() && dir.join("voices").is_dir() {
        return Ok(());
    }
    Err(format!(
        "no voice model in {}: put Kokoro-82M's config.json, kokoro-v1_0.pth and voices/ there (huggingface.co/hexgrad/Kokoro-82M, `reelforge-cli fetch-model --kokoro`), or set REELFORGE_KOKORO",
        dir.display()
    ))
}

impl Kokoro {
    /// Loads the model in `dir`.
    pub fn load(dir: &Path) -> Result<Kokoro, String> {
        check(dir)?;
        let config = KokoroConfig::from_file(dir.join("config.json"))?;
        let vb = checkpoint(&checkpoint_in(dir).expect("checked"))?;
        Kokoro::new(config, vb, dir.join("voices"))
    }

    fn new(config: KokoroConfig, vb: VarBuilder, voices_dir: PathBuf) -> Result<Kokoro, String> {
        Ok(Kokoro { net: Network::new(config, vb)?, voices_dir, voices: HashMap::new() })
    }

    /// The voices there are, Mandarin ones first.
    pub fn voices(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.voices_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.path().file_stem()?.to_str().map(str::to_string))
            .collect();
        out.sort_by_key(|v| (!v.starts_with('z'), v.clone()));
        out
    }

    fn voice(&mut self, name: &str) -> Result<Tensor, String> {
        if let Some(v) = self.voices.get(name) {
            return Ok(v.clone());
        }
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("not a voice name: {name:?}"));
        }
        let path = self.voices_dir.join(format!("{name}.pt"));
        let bytes = std::fs::read(&path).map_err(|_| format!("no voice {name:?} in {}", self.voices_dir.display()))?;
        let t = read_pth(&bytes, None)?.into_values().next().ok_or(format!("{} holds no voice", path.display()))?;
        self.voices.insert(name.to_string(), t.clone());
        Ok(t)
    }

    /// `text` read aloud in `voice` (`speed` 1 is the voice's own pace; 0.5 to 2), as mono samples
    /// at [`SAMPLE_RATE`]. Sentences are read one at a time with a short pause between them.
    pub fn speak(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>, String> {
        if !(0.5..=2.0).contains(&speed) {
            return Err("`speed` must be from 0.5 to 2".into());
        }
        let style_table = self.voice(voice)?;
        let mut out = Vec::new();
        for part in pieces(&zh::phonemes(text), MAX_TOKENS) {
            let mut ids = vec![0];
            ids.extend(part.chars().filter_map(|c| self.net.config.vocab.get(c.encode_utf8(&mut [0; 4]) as &str)));
            ids.push(0);
            if ids.len() <= 2 {
                continue;
            }
            let style = Network::style(&style_table, ids.len())?;
            out.extend(self.net.forward(&ids, &style, speed)?);
            if part.ends_with(['.', '!', '?']) {
                out.extend(std::iter::repeat_n(0.0, (SAMPLE_RATE as f32 * 0.18 / speed) as usize));
            }
        }
        if out.is_empty() {
            return Err("nothing to say".into());
        }
        Ok(out)
    }
}

/// `phonemes` cut into runs of at most `max` symbols: at sentence ends, else at pauses, else at
/// spaces, else anywhere.
fn pieces(phonemes: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = phonemes.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = chars.len().min(start + max);
        // a sentence end within reach closes this piece
        if let Some(i) = chars[start..end].iter().position(|c| matches!(c, '.' | '!' | '?')) {
            end = start + i + 1;
        } else if end < chars.len() {
            let cut = [&[',', ';', ':'][..], &[' '][..]]
                .iter()
                .find_map(|set| chars[start..end].iter().rposition(|c| set.contains(c)));
            if let Some(i) = cut.filter(|&i| i > 0) {
                end = start + i + 1;
            }
        }
        let s: String = chars[start..end].iter().collect::<String>().trim().to_string();
        if !s.is_empty() {
            out.push(s);
        }
        start = end;
    }
    out
}

/// `samples` (mono, `rate` Hz) as a 16-bit WAV file.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let n = samples.len() as u32;
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    b.extend(b"RIFF");
    b.extend((36 + n * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((n * 2).to_le_bytes());
    for s in samples {
        b.extend(((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    b
}

#[doc(hidden)]
pub mod tiny;

#[cfg(test)]
mod tests;
