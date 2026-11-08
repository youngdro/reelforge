use candle_core::DType;
use candle_nn::VarBuilder;

use super::*;

/// A Whisper-shaped model far too small to understand anything, with random weights: it checks
/// the plumbing (mel, encoder, prompt, timestamp rules, windows), not the hearing.
fn tiny() -> Whisper {
    let (config, map) = crate::tiny::parts();
    Whisper::new(config, VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu), &crate::tiny::tokenizer()).unwrap()
}

#[test]
fn a_model_folder_loads() {
    let dir = std::env::temp_dir().join(format!("reelforge-whisper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(!is_model_dir(&dir));
    crate::tiny::write(&dir).unwrap();
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
