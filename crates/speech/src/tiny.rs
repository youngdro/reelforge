//! A Whisper-shaped model far too small to understand anything, with random weights, and its
//! vocabulary: for testing the plumbing (here and in crates that use this one) without a download.

use std::collections::HashMap;

use candle_core::{DType, Device, Tensor};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::whisper::{Config, model::Whisper as Model};

use crate::tokens::byte_decoder;

/// A small vocabulary shaped like Whisper's: byte tokens, a few words, the specials, two
/// languages and timestamps up to 30 s.
pub fn tokenizer() -> String {
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
    serde_json::json!({ "model": { "type": "BPE", "vocab": vocab, "merges": merges }, "added_tokens": added })
        .to_string()
}

/// The model's settings and random weights.
pub(crate) fn parts() -> (Config, VarMap) {
    let config = Config {
        num_mel_bins: 80,
        max_source_positions: 1500,
        d_model: 16,
        encoder_attention_heads: 2,
        encoder_layers: 1,
        vocab_size: 15 + 1501,
        max_target_positions: 48,
        decoder_attention_heads: 2,
        decoder_layers: 1,
        suppress_tokens: vec![],
    };
    let map = VarMap::new();
    // let the model create its weights, then make them random
    Model::load(&VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu), config.clone()).expect("shapes");
    for var in map.all_vars() {
        let t = Tensor::randn(0f32, 0.5, var.shape(), &Device::Cpu).expect("tensor");
        var.set(&t).expect("tensor");
    }
    (config, map)
}

/// Writes the tiny model's three files into `dir`.
pub fn write(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (c, map) = parts();
    map.save(dir.join("model.safetensors")).map_err(|e| e.to_string())?;
    let config = serde_json::json!({
        "num_mel_bins": c.num_mel_bins, "max_source_positions": c.max_source_positions,
        "d_model": c.d_model, "encoder_attention_heads": c.encoder_attention_heads,
        "encoder_layers": c.encoder_layers, "vocab_size": c.vocab_size,
        "max_target_positions": c.max_target_positions,
        "decoder_attention_heads": c.decoder_attention_heads, "decoder_layers": c.decoder_layers,
        "suppress_tokens": [1]
    });
    std::fs::write(dir.join("config.json"), config.to_string()).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("tokenizer.json"), tokenizer()).map_err(|e| e.to_string())
}
