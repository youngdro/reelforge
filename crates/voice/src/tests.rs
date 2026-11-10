use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reelforge-voice-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pieces_break_at_sentences_then_pauses() {
    assert_eq!(pieces("ni↗ χau↓. ʃɻ↘ ʨjɛ↘!", 500), ["ni↗ χau↓.", "ʃɻ↘ ʨjɛ↘!"]);
    assert_eq!(pieces("aa, bb cc", 6), ["aa,", "bb cc"]);
    assert_eq!(pieces("aa bb cc dd", 6), ["aa bb", "cc dd"]);
    assert_eq!(pieces("abcdefgh", 3), ["abc", "def", "gh"]);
    assert!(pieces("  ", 10).is_empty());
}

#[test]
fn wav_header_and_samples() {
    let b = wav(&[0.0, 1.0, -2.0], 24_000);
    assert_eq!(&b[..4], b"RIFF");
    assert_eq!(&b[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 24_000);
    assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 6);
    assert_eq!(b.len(), 50);
    assert_eq!(i16::from_le_bytes([b[46], b[47]]), 32767);
    assert_eq!(i16::from_le_bytes([b[48], b[49]]), -32767);
}

#[test]
fn style_row_follows_length() {
    let table = Tensor::arange(0f32, 510.0, &candle_core::Device::Cpu)
        .unwrap()
        .reshape((510, 1, 1))
        .unwrap()
        .broadcast_as((510, 1, 4))
        .unwrap()
        .contiguous()
        .unwrap();
    let row = |n| Network::style(&table, n).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap()[0];
    assert_eq!(row(2), 0.0);
    assert_eq!(row(12), 9.0);
    assert_eq!(row(10_000), 509.0);
}

#[test]
fn missing_model_says_where_to_get_it() {
    let e = check(&scratch("missing")).unwrap_err();
    assert!(e.contains("fetch-model --kokoro"), "{e}");
}

#[test]
fn tiny_model_speaks_from_its_files() {
    let dir = scratch("tiny");
    tiny::write(&dir).unwrap();
    check(&dir).unwrap();
    let mut k = Kokoro::load(&dir).unwrap();
    assert_eq!(k.voices(), [DEFAULT_VOICE]);
    let one = k.speak("你好。", DEFAULT_VOICE, 1.0).unwrap();
    assert!(one.iter().all(|s| s.is_finite()));
    // every phoneme lasts at least one frame of 300 samples, plus the pause after a sentence
    let symbols = zh::phonemes("你好。").chars().count();
    assert!(one.len() >= symbols * 300 + 4000, "{} samples", one.len());
    let two = k.speak("你好。世界！", DEFAULT_VOICE, 1.0).unwrap();
    assert!(two.len() > one.len());
    assert!(k.speak("你好", "../x", 1.0).unwrap_err().contains("not a voice name"));
    assert!(k.speak("你好", "zm_nobody", 1.0).unwrap_err().contains("no voice"));
    assert!(k.speak("你好", DEFAULT_VOICE, 3.0).is_err());
    assert_eq!(k.speak("……", DEFAULT_VOICE, 1.0).unwrap_err(), "nothing to say");
    std::fs::remove_dir_all(&dir).unwrap();
}
