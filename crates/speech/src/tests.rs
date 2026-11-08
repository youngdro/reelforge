use candle_core::DType;
use candle_nn::{VarBuilder, VarMap};

use super::*;

/// A Whisper-shaped model far too small to understand anything, with random weights: it checks
/// the plumbing (mel, encoder, prompt, timestamp rules, windows), not the hearing.
fn tiny() -> Whisper {
    let (config, map) = tiny_parts();
    Whisper::new(config, VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu), &tokens::tests::tiny_tokenizer()).unwrap()
}

fn tiny_parts() -> (Config, VarMap) {
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
        let t = Tensor::randn(0f32, 0.5, var.shape(), &Device::Cpu).unwrap();
        var.set(&t).unwrap();
    }
    (config, map)
}

#[test]
fn a_model_folder_loads() {
    let dir = std::env::temp_dir().join(format!("reelforge-whisper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(!is_model_dir(&dir));
    let (config, map) = tiny_parts();
    map.save(dir.join("model.safetensors")).unwrap();
    let c = serde_json::json!({
        "num_mel_bins": config.num_mel_bins, "max_source_positions": config.max_source_positions,
        "d_model": config.d_model, "encoder_attention_heads": config.encoder_attention_heads,
        "encoder_layers": config.encoder_layers, "vocab_size": config.vocab_size,
        "max_target_positions": config.max_target_positions,
        "decoder_attention_heads": config.decoder_attention_heads, "decoder_layers": config.decoder_layers,
        "suppress_tokens": [1, 2]
    });
    std::fs::write(dir.join("config.json"), c.to_string()).unwrap();
    std::fs::write(dir.join("tokenizer.json"), tokens::tests::tiny_tokenizer()).unwrap();
    assert!(is_model_dir(&dir));
    let mut w = Whisper::load(&dir).unwrap();
    let out = w.transcribe(&vec![0.0; SAMPLE_RATE as usize], &Options::default(), &mut |_| true).unwrap();
    assert!(!out.language.is_empty());
    let missing = Recogniser::new(dir.join("nothing"));
    assert!(missing.dir().ends_with("nothing"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn segments_become_captions_parameters() {
    let segs = [Segment { start: Tick::SECOND, end: Tick::SECOND * 2, text: "你好".into() }];
    let p = captions_params(&segs, None, Some("variety"));
    assert_eq!(p["cues"][0]["start"], Tick::SECOND.0);
    assert_eq!(p["cues"][0]["text"], "你好");
    assert_eq!(p["template"], "variety");
    assert!(p.get("track").is_none());
}

#[test]
fn a_tiny_model_listens_window_by_window() {
    let mut whisper = tiny();
    // 40 s of a wobbling tone: two windows
    let samples: Vec<f32> =
        (0..40 * SAMPLE_RATE as usize).map(|i| (i as f32 * 0.05).sin() * (i as f32 * 0.0003).sin() * 0.5).collect();
    let mut seen = vec![];
    let out = whisper
        .transcribe(&samples, &Options { language: Some("zh".into()), prompt: None }, &mut |p| {
            seen.push(p);
            true
        })
        .unwrap();
    assert_eq!(out.language, "zh");
    assert!(!seen.is_empty() && *seen.last().unwrap() == 1.0, "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    let end = Tick::from_seconds(40.0);
    for s in &out.segments {
        assert!(s.start < s.end && s.end <= end, "{s:?}");
        assert!(!s.text.is_empty());
    }
    assert!(out.segments.windows(2).all(|w| w[0].start <= w[1].start));
}

#[test]
fn a_tiny_model_tells_a_language_and_can_be_stopped() {
    let mut whisper = tiny();
    let samples = vec![0.0f32; 5 * SAMPLE_RATE as usize];
    let out = whisper.transcribe(&samples, &Options::default(), &mut |_| true).unwrap();
    assert!(out.language == "zh" || out.language == "en", "{}", out.language);
    assert_eq!(whisper.transcribe(&samples, &Options::default(), &mut |_| false).unwrap_err(), "stopped");
}

#[test]
fn chinese_gets_a_simplified_prompt() {
    assert!(default_prompt("zh").unwrap().contains("简体"));
    assert_eq!(default_prompt("en"), None);
}
