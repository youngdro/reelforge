//! Whisper's vocabulary read from a Hugging Face `tokenizer.json` (decoding only: byte-level BPE
//! pieces back to bytes), the timestamp rules that keep greedy decoding well formed, and the
//! splitting of a decoded window into timed segments.

use std::collections::HashMap;

use serde_json::Value;

/// The tokens Whisper decodes with.
#[derive(Clone, Debug)]
pub struct Vocab {
    /// Bytes of each text token (ids below `eot`).
    pieces: Vec<Vec<u8>>,
    /// Text tokens by their byte-level spelling, for encoding a prompt.
    ids: HashMap<String, u32>,
    /// BPE merges and their rank.
    merges: HashMap<(String, String), usize>,
    /// Special tokens by their text.
    specials: HashMap<String, u32>,
    /// `<|endoftext|>`.
    pub eot: u32,
    /// `<|startoftranscript|>`.
    pub sot: u32,
    /// `<|transcribe|>`.
    pub transcribe: u32,
    /// `<|notimestamps|>`.
    pub no_timestamps: u32,
    /// `<|0.00|>`; every later id is another 0.02 s.
    pub timestamp_begin: u32,
    /// `<|nospeech|>` (or `<|nocaptions|>` in older vocabularies).
    pub no_speech: Option<u32>,
}

/// GPT-2's byte ↔ printable character table, reversed.
fn byte_decoder() -> HashMap<char, u8> {
    let mut printable: Vec<u32> = (b'!' as u32..=b'~' as u32).chain(0xA1..=0xAC).chain(0xAE..=0xFF).collect();
    let mut chars = printable.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !printable.contains(&b) {
            printable.push(b);
            chars.push(256 + n);
            n += 1;
        }
    }
    printable.into_iter().zip(chars).map(|(b, c)| (char::from_u32(c).expect("valid"), b as u8)).collect()
}

impl Vocab {
    /// Reads `tokenizer.json`.
    pub fn from_json(json: &str) -> Result<Vocab, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("tokenizer.json: {e}"))?;
        let mut ids: HashMap<String, u32> = HashMap::new();
        if let Some(vocab) = v.pointer("/model/vocab").and_then(Value::as_object) {
            for (piece, id) in vocab {
                if let Some(id) = id.as_u64() {
                    ids.insert(piece.clone(), id as u32);
                }
            }
        }
        let mut specials = HashMap::new();
        for t in v.get("added_tokens").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(c), Some(id)) = (t.get("content").and_then(Value::as_str), t.get("id").and_then(Value::as_u64)) {
                specials.insert(c.to_string(), id as u32);
            }
        }
        for (piece, &id) in &ids {
            if piece.starts_with("<|") && piece.ends_with("|>") {
                specials.entry(piece.clone()).or_insert(id);
            }
        }
        let special = |name: &str| specials.get(name).copied().ok_or(format!("tokenizer.json has no {name}"));
        let eot = special("<|endoftext|>")?;
        let no_timestamps = special("<|notimestamps|>")?;
        let mut merges = HashMap::new();
        for (rank, m) in v.pointer("/model/merges").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let pair = match m {
                Value::String(s) => s.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())),
                Value::Array(a) => match (a.first().and_then(Value::as_str), a.get(1).and_then(Value::as_str)) {
                    (Some(a), Some(b)) => Some((a.to_string(), b.to_string())),
                    _ => None,
                },
                _ => None,
            };
            if let Some(pair) = pair {
                merges.insert(pair, rank);
            }
        }
        let decoder = byte_decoder();
        let mut pieces = vec![Vec::new(); eot as usize];
        for (piece, &id) in &ids {
            if id < eot {
                pieces[id as usize] = piece.chars().filter_map(|c| decoder.get(&c).copied()).collect();
            }
        }
        ids.retain(|_, id| *id < eot);
        Ok(Vocab {
            pieces,
            ids,
            merges,
            eot,
            sot: special("<|startoftranscript|>")?,
            transcribe: special("<|transcribe|>")?,
            no_timestamps,
            timestamp_begin: no_timestamps + 1,
            no_speech: specials.get("<|nospeech|>").or(specials.get("<|nocaptions|>")).copied(),
            specials,
        })
    }

    /// The token for language `code` (`zh`, `en`, …).
    pub fn language(&self, code: &str) -> Option<u32> {
        self.specials.get(&format!("<|{code}|>")).copied()
    }

    /// Every language token, with its code.
    pub fn languages(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = self
            .specials
            .iter()
            .filter_map(|(name, &id)| {
                let code = name.strip_prefix("<|")?.strip_suffix("|>")?;
                let ok = (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase());
                (ok && id > self.sot && id < self.transcribe).then(|| (code.to_string(), id))
            })
            .collect();
        out.sort_by_key(|(_, id)| *id);
        out
    }

    /// `<|startofprev|>`, which opens a prompt.
    pub fn start_of_prev(&self) -> Option<u32> {
        self.specials.get("<|startofprev|>").copied()
    }

    /// `text` as tokens (byte-level BPE), for a prompt. Pieces the vocabulary lacks are dropped.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let enc: HashMap<u8, char> = byte_decoder().into_iter().map(|(c, b)| (b, c)).collect();
        let mut out = Vec::new();
        for word in words(text) {
            let mut parts: Vec<String> = word.bytes().map(|b| enc[&b].to_string()).collect();
            loop {
                let best = parts
                    .windows(2)
                    .enumerate()
                    .filter_map(|(i, w)| self.merges.get(&(w[0].clone(), w[1].clone())).map(|r| (*r, i)))
                    .min();
                let Some((_, i)) = best else { break };
                let next = parts.remove(i + 1);
                parts[i].push_str(&next);
            }
            out.extend(parts.iter().filter_map(|p| self.ids.get(p)));
        }
        out
    }

    /// Whether `t` is a timestamp.
    pub fn is_timestamp(&self, t: u32) -> bool {
        t >= self.timestamp_begin
    }

    /// The text of `tokens`, skipping anything that is not text.
    pub fn text(&self, tokens: &[u32]) -> String {
        let bytes: Vec<u8> =
            tokens.iter().filter_map(|&t| self.pieces.get(t as usize)).flatten().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// GPT-2's pre-split: runs of letters, of digits, of other symbols (each may take one leading
/// space), and of whitespace.
fn words(text: &str) -> Vec<&str> {
    let class = |c: char| -> u8 {
        if c.is_alphabetic() {
            0
        } else if c.is_numeric() {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut run: Option<u8> = None;
    for (i, c) in text.char_indices() {
        let k = class(c);
        match run {
            Some(r) if r == k => continue,
            Some(2) => {
                // the space just before a word goes with the word
                let cut = if text[..i].ends_with(' ') { i - 1 } else { i };
                if cut > start {
                    out.push(&text[start..cut]);
                }
                start = cut;
            }
            Some(_) => {
                out.push(&text[start..i]);
                start = i;
            }
            None => {}
        }
        run = Some(k);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Sets to −∞ every logit Whisper's rules forbid after `sampled` (the tokens decoded so far in this
/// window, without the prompt): specials other than end-of-text, the configured `suppress`
/// tokens, timestamps out of pairs or going back in time, a first token that is not a timestamp
/// within `max_initial` steps of 0.02 s, and text when the timestamps together outweigh it.
pub fn apply_rules(logits: &mut [f32], sampled: &[u32], v: &Vocab, suppress: &[u32], max_initial: u32) {
    let n = logits.len();
    let tb = (v.timestamp_begin as usize).min(n);
    let mut forbid = |range: std::ops::Range<usize>| {
        for l in &mut logits[range.start.min(n)..range.end.min(n)] {
            *l = f32::NEG_INFINITY;
        }
    };
    forbid(v.eot as usize + 1..tb);
    for &t in suppress {
        forbid(t as usize..t as usize + 1);
    }
    let last_ts = sampled.last().is_some_and(|&t| v.is_timestamp(t));
    let penult_ts = sampled.len() < 2 || v.is_timestamp(sampled[sampled.len() - 2]);
    if last_ts {
        if penult_ts {
            forbid(tb..n);
        } else {
            forbid(0..v.eot as usize);
        }
    }
    if let Some(&last) = sampled.iter().rev().find(|&&t| v.is_timestamp(t)) {
        // a closing timestamp may repeat the opening one; otherwise time moves on
        let floor = if last_ts && !penult_ts { last } else { last + 1 };
        forbid(tb..floor as usize);
    }
    if sampled.is_empty() {
        forbid(0..tb);
        forbid(tb + max_initial as usize + 1..n);
    }
    // when the timestamps together are likelier than any one text token, take a timestamp
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max.is_finite() && tb < n {
        let sum = |s: &[f32]| s.iter().map(|&l| ((l - max) as f64).exp()).sum::<f64>();
        let ts = sum(&logits[tb..]);
        let text_max = logits[..tb].iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if ts > 0.0 && ts.ln() as f32 + max > text_max {
            forbid(0..tb);
        }
    }
}

/// A piece of a decoded window: text from `start` to `end`, in 0.02 s steps from the window's start.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Start, in timestamp steps.
    pub start: u32,
    /// End, in timestamp steps.
    pub end: u32,
    /// Text tokens.
    pub tokens: Vec<u32>,
}

/// Splits a window's decoded tokens into timed pieces, and says how far to move on, in timestamp
/// steps (`window` is the window's length in steps). Text left open at the end is decoded again
/// from its start in the next window, unless it is all the window holds.
pub fn split(tokens: &[u32], v: &Vocab, window: u32) -> (Vec<Piece>, u32) {
    let mut pieces = Vec::new();
    let mut start: Option<u32> = None;
    let mut text = Vec::new();
    for &t in tokens {
        if v.is_timestamp(t) {
            let at = t - v.timestamp_begin;
            match start {
                Some(s) if !text.is_empty() => {
                    pieces.push(Piece { start: s, end: at.max(s), tokens: std::mem::take(&mut text) });
                    start = None;
                }
                _ => start = Some(at),
            }
        } else if t < v.eot {
            start.get_or_insert(0);
            text.push(t);
        }
    }
    let advance = match start {
        Some(s) if !text.is_empty() => {
            if pieces.is_empty() {
                pieces.push(Piece { start: s, end: window.max(s), tokens: text });
                window
            } else {
                s
            }
        }
        Some(s) if s > 0 && tokens.last().is_some_and(|&t| v.is_timestamp(t)) && !ends_single(tokens, v) => s,
        _ => window,
    };
    (pieces, if advance == 0 { window } else { advance.min(window) })
}

/// Whether the tokens end with a timestamp that closes a piece (not one that opens the next).
fn ends_single(tokens: &[u32], v: &Vocab) -> bool {
    let n = tokens.len();
    n >= 2 && v.is_timestamp(tokens[n - 1]) && !v.is_timestamp(tokens[n - 2])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small vocabulary shaped like Whisper's: byte tokens, a few words, the specials, two
    /// languages and timestamps up to 30 s.
    pub(crate) fn tiny_tokenizer() -> String {
        let enc: HashMap<u8, char> = byte_decoder().into_iter().map(|(c, b)| (b, c)).collect();
        let piece = |s: &str| s.bytes().map(|b| enc[&b]).collect::<String>();
        let mut vocab = serde_json::Map::new();
        let mut merges = vec![];
        let words = ["你好", "世界", " hello", "，", "字幕"];
        for (i, w) in words.iter().enumerate() {
            vocab.insert(piece(w), (i as u32).into());
            // merge the word's bytes left to right
            let chars: Vec<char> = piece(w).chars().collect();
            for k in 1..chars.len() {
                merges.push(format!("{} {}", chars[..k].iter().collect::<String>(), chars[k]));
            }
        }
        let mut added = vec![];
        let mut id = words.len() as u32;
        let mut add = |name: String| {
            added.push(serde_json::json!({ "id": id, "content": name, "special": true }));
            id += 1;
        };
        for name in ["<|endoftext|>", "<|startoftranscript|>", "<|en|>", "<|zh|>", "<|translate|>", "<|transcribe|>"] {
            add(name.into());
        }
        for name in ["<|startoflm|>", "<|startofprev|>", "<|nospeech|>", "<|notimestamps|>"] {
            add(name.into());
        }
        for i in 0..=1500 {
            add(format!("<|{:.2}|>", i as f32 * 0.02));
        }
        serde_json::json!({ "model": { "type": "BPE", "vocab": vocab, "merges": merges }, "added_tokens": added }).to_string()
    }

    #[test]
    fn reads_the_vocabulary_and_decodes_bytes() {
        let v = Vocab::from_json(&tiny_tokenizer()).unwrap();
        assert_eq!((v.eot, v.sot, v.transcribe, v.no_timestamps, v.timestamp_begin), (5, 6, 10, 14, 15));
        assert_eq!(v.no_speech, Some(13));
        assert_eq!(v.language("zh"), Some(8));
        assert_eq!(v.languages(), vec![("en".to_string(), 7), ("zh".to_string(), 8)]);
        assert_eq!(v.text(&[0, 3, 1, 15, 5]), "你好，世界");
        assert_eq!(v.text(&[2]), " hello");
        assert_eq!(v.encode("你好，世界"), vec![0, 3, 1]);
        assert_eq!(v.encode(" hello"), vec![2]);
        assert_eq!(words("你好 hello, 12 ab"), vec!["你好", " hello", ",", " 12", " ab"]);
    }

    #[test]
    fn timestamps_come_in_order_and_in_pairs() {
        let v = Vocab::from_json(&tiny_tokenizer()).unwrap();
        let n = 15 + 1501;
        let tb = 15usize;
        let allowed = |sampled: &[u32]| {
            let mut l = vec![0f32; n];
            apply_rules(&mut l, sampled, &v, &[], 50);
            l.iter().enumerate().filter(|(_, x)| x.is_finite()).map(|(i, _)| i).collect::<Vec<_>>()
        };
        // first: a timestamp within a second
        assert_eq!(allowed(&[]), (tb..=tb + 50).collect::<Vec<_>>());
        // after an opening timestamp: text only
        let a = allowed(&[15 + 10]);
        assert!(a.iter().all(|&t| t < tb) && a.contains(&0) && !a.contains(&6));
        // after text: text, the end, or a timestamp not before the opening one (uniform logits
        // make the timestamps outweigh any text token, so only timestamps remain)
        let a = allowed(&[15 + 10, 0]);
        assert_eq!(a[0], tb + 11);
        // after a closing timestamp: no text; a timestamp from the closing one on
        let a = allowed(&[15 + 10, 0, 15 + 20]);
        assert!(a.contains(&(tb + 20)) && !a.contains(&0) && !a.contains(&(tb + 19)));
    }

    #[test]
    fn windows_split_into_timed_pieces() {
        let v = Vocab::from_json(&tiny_tokenizer()).unwrap();
        let ts = |s: u32| 15 + s;
        // two closed pieces, ending on a closing timestamp: move on a whole window
        let (p, adv) = split(&[ts(0), 0, ts(50), ts(50), 1, ts(120)], &v, 1500);
        assert_eq!(
            p,
            vec![
                Piece { start: 0, end: 50, tokens: vec![0] },
                Piece { start: 50, end: 120, tokens: vec![1] }
            ]
        );
        assert_eq!(adv, 1500);
        // a piece left open: decode it again next window
        let (p, adv) = split(&[ts(0), 0, ts(50), ts(1400), 1], &v, 1500);
        assert_eq!(p.len(), 1);
        assert_eq!(adv, 1400);
        // only open text: keep it to the window's end
        let (p, adv) = split(&[ts(10), 0, 3, 1], &v, 1500);
        assert_eq!(p, vec![Piece { start: 10, end: 1500, tokens: vec![0, 3, 1] }]);
        assert_eq!(adv, 1500);
        // ending on an opening timestamp: start the next window there
        let (_, adv) = split(&[ts(0), 0, ts(50), ts(900)], &v, 1500);
        assert_eq!(adv, 900);
        // nothing at all
        assert_eq!(split(&[], &v, 1500), (vec![], 1500));
    }
}
