//! Round trips: what the exporter writes, the decoder reads back.

use std::io::Cursor;
use std::path::PathBuf;

use reelforge_render::{MediaSource, NoMedia, Quality, RenderError, render_frame};
use reelforge_renderspec::*;
use reelforge_time::{FrameRate, Tick};

use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reelforge-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn layer(id: &str, at: Tick, dur: Tick, content: LayerContent) -> Layer {
    Layer {
        id: id.into(),
        at,
        dur,
        content,
        transform: Transform::default(),
        opacity: Animated::Constant(1.0),
        blend: Blend::Normal,
        mask: None,
        fx: vec![],
        crop: None,
        flip_x: false,
        flip_y: false,
        shape_mask: None,
        crop_keys: None,
        shape_mask_keys: None,
    }
}

/// Red for a second, then blue for a second, 64×48 at 30 fps.
fn two_colours() -> RenderSpec {
    let mut s = RenderSpec::new(Output {
        width: 64,
        height: 48,
        fps: FrameRate::FPS_30,
        duration: Tick::SECOND * 2,
        background: Color([0.0, 0.0, 0.0, 1.0]),
        video: None,
        audio: None,
        format: Default::default(),
    });
    s.layers.push(layer("red", Tick::ZERO, Tick::SECOND, LayerContent::Solid { color: Color([1.0, 0.0, 0.0, 1.0]) }));
    s.layers.push(layer(
        "blue",
        Tick::SECOND,
        Tick::SECOND,
        LayerContent::Solid { color: Color([0.0, 0.0, 1.0, 1.0]) },
    ));
    s
}

fn export_to_file(spec: &RenderSpec, media: &Footage, name: &str) -> (PathBuf, ExportStats) {
    let mut calls = 0;
    let (cur, stats) = export(spec, media, Cursor::new(Vec::new()), &mut |_| {
        calls += 1;
        true
    })
    .expect("export");
    assert_eq!(calls as u64, stats.frames);
    let path = temp(name);
    std::fs::write(&path, cur.into_inner()).expect("write");
    (path, stats)
}

fn centre(img: &Image) -> [u8; 4] {
    let b = img.to_srgba8();
    let i = (((img.height / 2) * img.width + img.width / 2) * 4) as usize;
    [b[i], b[i + 1], b[i + 2], b[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 3]) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= 12)
}

fn sha(i: u8) -> String {
    format!("{i:02x}").repeat(32)
}

#[test]
fn exported_mp4_probes_and_decodes_to_the_same_pictures() {
    let (path, stats) = export_to_file(&two_colours(), &Footage::new(), "colours.mp4");
    assert_eq!((stats.frames, stats.audio_samples), (60, 0));

    let probe = probe_path(&path).expect("probe");
    assert_eq!(probe.kind, AssetKind::Video);
    assert_eq!(probe.size, Some([64, 48]));
    assert_eq!(probe.fps, Some(FrameRate::FPS_30));
    assert!(!probe.has_audio);
    assert_eq!(probe.duration, Some(Tick::SECOND * 2));

    let asset = Asset {
        kind: AssetKind::Video,
        sha256: sha(1),
        uri: path.to_string_lossy().into_owned(),
        of: None,
        probe: None,
    };
    let footage = Footage::new();
    let early = footage.picture("v", &asset, Tick::from_seconds(0.5)).expect("decode").expect("online");
    let late = footage.picture("v", &asset, Tick::from_seconds(1.5)).expect("decode").expect("online");
    let past_end = footage.picture("v", &asset, Tick::SECOND * 5).expect("decode").expect("online");
    assert!(near(centre(&early), [255, 0, 0]), "{:?}", centre(&early));
    assert!(near(centre(&late), [0, 0, 255]), "{:?}", centre(&late));
    assert!(near(centre(&past_end), [0, 0, 255]), "the last frame holds");

    // the timeline's pictures: small, and of the right moment
    let thumb = |t: f64| footage.thumbnail(&asset, Tick::from_seconds(t), 12).expect("decode").expect("online");
    let (red, blue) = (thumb(0.5), thumb(1.5));
    assert_eq!((red.width, red.height, red.rgba.len()), (16, 12, 16 * 12 * 4));
    let mid = |t: &Thumbnail| {
        let i = ((t.height / 2 * t.width + t.width / 2) * 4) as usize;
        [t.rgba[i], t.rgba[i + 1], t.rgba[i + 2], t.rgba[i + 3]]
    };
    assert!(near(mid(&red), [255, 0, 0]), "{:?}", mid(&red));
    assert!(near(mid(&blue), [0, 0, 255]), "{:?}", mid(&blue));

    // the exported file is footage for the next edit: play it from 1.5 s, at half size
    let mut spec = two_colours();
    spec.assets.insert("v".into(), asset);
    spec.layers.clear();
    spec.layers.push(layer(
        "clip",
        Tick::ZERO,
        Tick::SECOND,
        LayerContent::Media {
            asset: "v".into(),
            time: TimeMap::new(Tick::from_seconds(1.5), 1.0),
            fit: Fit::Contain,
            between: Default::default(),
        },
    ));
    let (img, report) = render_frame(&spec, Tick::ZERO, 1.0, &footage, Quality::Final).expect("render");
    assert!(report.placeholders.is_empty());
    assert!(near(centre(&img), [0, 0, 255]));
}

#[test]
fn shrinking_averages_what_each_pixel_covers() {
    // 4×2: left half black, right half white, shrunk to 2×1
    let px = |v: u8| [v, v, v, 255];
    let rgba: Vec<u8> = [0, 0, 255, 255, 0, 0, 255, 255].iter().flat_map(|&v| px(v)).collect();
    let t = Thumbnail::shrink(4, 2, &rgba, 1);
    assert_eq!((t.width, t.height), (2, 1));
    assert_eq!(t.rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
    // never enlarged
    assert_eq!(Thumbnail::shrink(4, 2, &rgba, 10).height, 2);
}

#[test]
fn offline_footage_is_a_placeholder_not_an_error() {
    let asset =
        Asset { kind: AssetKind::Video, sha256: sha(2), uri: "/nowhere/missing.mp4".into(), of: None, probe: None };
    assert!(Footage::new().picture("v", &asset, Tick::ZERO).expect("no error").is_none());
    assert!(NoMedia.picture("v", &asset, Tick::ZERO).expect("no error").is_none());
}

/// A 16-bit stereo WAV: a 440 Hz tone on the left, silence on the right.
fn tone_wav(rate: u32, seconds: f32) -> Vec<u8> {
    let n = (rate as f32 * seconds) as u32;
    let mut b = Vec::new();
    let data_len = n * 4;
    b.extend(b"RIFF");
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * 4).to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for i in 0..n {
        let v = ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 16000.0) as i16;
        b.extend(v.to_le_bytes());
        b.extend(0i16.to_le_bytes());
    }
    b
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
}

#[test]
fn sound_is_mixed_with_gain_and_written_as_aac() {
    let wav = temp("tone.wav");
    std::fs::write(&wav, tone_wav(44_100, 2.0)).expect("write wav");
    let probe = probe_path(&wav).expect("probe wav");
    assert_eq!(probe.kind, AssetKind::Audio);
    assert!(probe.has_audio);

    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(3),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    // the tone plays for the first second only, 6 dB down
    spec.audio.push(AudioClip {
        id: "tone".into(),
        asset: "a".into(),
        comp: None,
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::default(),
        gain_db: Animated::Constant(-6.0),
        keep_pitch: false,
        pitch: 0.0,
        fx: vec![],
    });
    let footage = Footage::new();
    let m = export::mix(&spec, &footage, 48_000, 0, 96_000).expect("mix");
    let (first, second) = m[0].split_at(48_000);
    let expected = 16000.0 / 32768.0 / 2f32.sqrt() * 10f32.powf(-6.0 / 20.0);
    assert!((rms(first) - expected).abs() < 0.02, "{} vs {expected}", rms(first));
    assert_eq!(rms(second), 0.0);
    assert_eq!(rms(&m[1]), 0.0, "the right channel stays silent");

    let (path, stats) = export_to_file(&spec, &footage, "tone.mp4");
    assert_eq!(stats.audio_samples, 96_000);
    let probe = probe_path(&path).expect("probe export");
    assert!(probe.has_audio);
    let asset = Asset {
        kind: AssetKind::Video,
        sha256: sha(4),
        uri: path.to_string_lossy().into_owned(),
        of: None,
        probe: None,
    };
    let back = Footage::new().audio(&asset, 0, 96_000, 48_000).expect("decode").expect("has sound");
    let (first, second) = back[0].split_at(48_000);
    assert!((rms(&first[4_800..43_200]) - expected).abs() < 0.03, "{}", rms(first));
    assert!(rms(&second[4_800..]) < 0.01, "{}", rms(second));
}

/// [`two_colours`] with a 440 Hz tone on the left for its first second, 6 dB down.
fn with_a_tone(name: &str) -> RenderSpec {
    let wav = temp(name);
    std::fs::write(&wav, tone_wav(44_100, 2.0)).expect("write wav");
    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(3),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    spec.audio.push(AudioClip {
        id: "tone".into(),
        asset: "a".into(),
        comp: None,
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::default(),
        gain_db: Animated::Constant(-6.0),
        keep_pitch: false,
        pitch: 0.0,
        fx: vec![],
    });
    spec
}

#[test]
fn a_gif_plays_every_frame_at_the_encoded_size_round_and_round() {
    let mut spec = two_colours();
    spec.output.format = OutputFormat::Gif;
    spec.output.fps = FrameRate { num: 25, den: 2 };
    spec.output.video =
        Some(VideoEncoding { codec: VideoCodec::H264, crf: None, bitrate_kbps: None, size: Some([32, 24]) });
    assert!(spec.required_features().contains("encode.gif/1"));
    let mut calls = 0;
    let (out, stats) = export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |p| {
        calls += 1;
        assert_eq!((p.done, p.total), (calls, 25));
        true
    })
    .expect("export");
    assert_eq!((stats.frames, stats.audio_samples, stats.encoder.as_str()), (25, 0, "gif"));
    let bytes = out.into_inner();
    assert_eq!(&bytes[..6], b"GIF89a");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut back = options.read_info(Cursor::new(&bytes)).expect("a GIF");
    assert_eq!((back.width(), back.height()), (32, 24));
    let mut frames = vec![];
    while let Some(f) = back.read_next_frame().expect("a frame") {
        // 12.5 frames a second: eight hundredths each
        assert_eq!((f.width, f.height, f.delay), (32, 24, 8));
        let i = (12 * 32 + 16) * 4;
        frames.push([f.buffer[i], f.buffer[i + 1], f.buffer[i + 2], f.buffer[i + 3]]);
    }
    assert_eq!(frames.len(), 25);
    assert!(near(frames[0], [255, 0, 0]) && near(frames[12], [255, 0, 0]), "{:?}", frames[0]);
    assert!(near(frames[13], [0, 0, 255]) && near(frames[24], [0, 0, 255]), "{:?}", frames[24]);
    assert_eq!(back.repeat(), gif::Repeat::Infinite);

    // a cancel stops it
    let r = export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |p| p.done < 3);
    assert!(matches!(r, Err(ExportError::Cancelled)));
}

#[test]
fn the_sound_alone_is_written_as_a_wav_or_an_m4a() {
    let footage = Footage::new();
    let expected = 16000.0 / 32768.0 / 2f32.sqrt() * 10f32.powf(-6.0 / 20.0);
    for (format, name) in [(OutputFormat::Wav, "alone.wav"), (OutputFormat::M4a, "alone.m4a")] {
        let mut spec = with_a_tone("alone-tone.wav");
        spec.output.format = format;
        assert!(spec.required_features().contains("encode.audio/1"));
        let mut seconds = vec![];
        let (out, stats) = export(&spec, &footage, Cursor::new(Vec::new()), &mut |p| {
            seconds.push((p.done, p.total));
            true
        })
        .expect("export");
        assert_eq!(seconds, [(1, 2), (2, 2)]);
        assert_eq!((stats.frames, stats.audio_samples), (0, 96_000));
        let bytes = out.into_inner();
        if format == OutputFormat::Wav {
            assert_eq!((&bytes[..4], &bytes[8..16], bytes.len()), (&b"RIFF"[..], &b"WAVEfmt "[..], 44 + 96_000 * 4));
            assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 48_000);
            assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 96_000 * 4);
        }
        let path = temp(name);
        std::fs::write(&path, bytes).expect("write");
        let probe = probe_path(&path).expect("probe");
        assert!(probe.has_audio && probe.size.is_none(), "{probe:?}");
        let asset = Asset {
            kind: AssetKind::Audio,
            sha256: sha(if format == OutputFormat::Wav { 5 } else { 6 }),
            uri: path.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        };
        let back = Footage::new().audio(&asset, 0, 96_000, 48_000).expect("decode").expect("has sound");
        let (first, second) = back[0].split_at(48_000);
        assert!((rms(&first[4_800..43_200]) - expected).abs() < 0.03, "{name}: {}", rms(first));
        assert!(rms(&second[4_800..]) < 0.01 && rms(&back[1]) < 0.01, "{name}");
    }
    // with nothing to hear there is nothing to write
    let mut silent = two_colours();
    silent.output.format = OutputFormat::Wav;
    let r = export(&silent, &footage, Cursor::new(Vec::new()), &mut |_| true);
    assert!(matches!(r, Err(ExportError::Invalid(why)) if why.contains("no sound")));
}

#[test]
fn one_frame_is_a_png_or_a_jpeg() {
    let spec = two_colours();
    let footage = Footage::new();
    let red = still(&spec, &footage, Tick::from_seconds(0.5), 1.0).expect("a frame");
    assert_eq!((red.width, red.height), (64, 48));
    assert!(near(centre(&red), [255, 0, 0]));
    // half the size, and past the end the last frame stands in
    let blue = still(&spec, &footage, Tick::SECOND * 9, 0.5).expect("a frame");
    assert_eq!((blue.width, blue.height), (32, 24));
    assert!(near(centre(&blue), [0, 0, 255]));
    for (format, magic) in [(StillFormat::Png, &b"\x89PNG"[..]), (StillFormat::Jpeg, &b"\xff\xd8\xff"[..])] {
        let bytes = encode_still(&red, format).expect("encodes");
        assert!(bytes.starts_with(magic), "{format:?}");
        let back = image::load_from_memory(&bytes).expect("decodes").to_rgba8();
        assert_eq!((back.width(), back.height()), (64, 48));
        let p = back.get_pixel(32, 24).0;
        assert!(near(p, [255, 0, 0]), "{format:?}: {p:?}");
    }
}

#[test]
fn a_cancelled_export_stops_and_says_so() {
    let mut seen = 0;
    let r = export(&two_colours(), &Footage::new(), Cursor::new(Vec::new()), &mut |p| {
        seen = p.done;
        p.done < 5
    });
    assert!(matches!(r, Err(ExportError::Cancelled)));
    assert_eq!(seen, 5);
}

#[test]
fn a_spec_this_renderer_cannot_draw_is_refused_before_any_work() {
    let mut spec = two_colours();
    spec.layers[0].fx.push(Effect { id: "blur.motion@1".into(), params: Default::default(), asset: None });
    let r = export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |_| panic!("no frame is drawn"));
    assert!(matches!(r, Err(ExportError::Render(RenderError::Unsupported(_)))), "{r:?}");
}

#[test]
fn cpu_and_gpu_exports_decode_to_the_same_pictures() {
    let spec = two_colours();
    let run = |renderer| {
        export_with(&spec, &Footage::new(), Cursor::new(Vec::new()), renderer, &mut |_| true)
            .map(|(cur, stats)| (cur.into_inner(), stats))
    };
    let (cpu, stats) = run(Renderer::Cpu).expect("cpu export");
    assert_eq!(stats.renderer, "cpu");
    let (gpu, stats) = match run(Renderer::Gpu) {
        Ok(r) => r,
        Err(e) => {
            assert!(std::env::var_os("REELFORGE_REQUIRE_GPU").is_none(), "{e}");
            eprintln!("skipping: {e}");
            return;
        }
    };
    assert_ne!(stats.renderer, "cpu");
    let footage = Footage::new();
    for (i, bytes) in [cpu, gpu].into_iter().enumerate() {
        let path = temp(&format!("renderer{i}.mp4"));
        std::fs::write(&path, bytes).expect("write");
        let asset = Asset {
            kind: AssetKind::Video,
            sha256: sha(40 + i as u8),
            uri: path.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        };
        for (t, want) in [(0.5, [255, 0, 0]), (1.5, [0, 0, 255])] {
            let pic = footage.picture("v", &asset, Tick::from_seconds(t)).expect("decode").expect("online");
            assert!(near(centre(&pic), want), "renderer {i} at {t}s: {:?}", centre(&pic));
        }
    }
}

#[test]
fn faster_sound_keeps_its_pitch_when_asked() {
    let wav = temp("tone-stretch.wav");
    std::fs::write(&wav, tone_wav(48_000, 3.0)).expect("write wav");
    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(5),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    let clip = |keep_pitch| AudioClip {
        id: "tone".into(),
        asset: "a".into(),
        comp: None,
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::new(Tick::ZERO, 2.0),
        gain_db: Animated::Constant(0.0),
        keep_pitch,
        pitch: 0.0,
        fx: vec![],
    };
    let crossings = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    let footage = Footage::new();
    spec.audio = vec![clip(false)];
    let tape = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    spec.audio = vec![clip(true)];
    let kept = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    // half a second of a 440 Hz tone: 220 rises through zero; on tape twice the speed doubles it
    let (t, k) = (crossings(&tape[0][12_000..36_000]), crossings(&kept[0][12_000..36_000]));
    assert!((t as i64 - 440).abs() <= 4, "tape {t}");
    assert!((k as i64 - 220).abs() <= 4, "kept {k}");
    // and the level holds through the grains
    let expected = 16000.0 / 32768.0 / 2f32.sqrt();
    assert!((rms(&kept[0][4_800..43_200]) - expected).abs() < 0.04, "{}", rms(&kept[0]));
    // made in slices, the same sound
    let mut sliced = vec![];
    for i in 0..4 {
        sliced.extend(export::mix(&spec, &footage, 48_000, i * 12_000, 12_000).expect("mix")[0].clone());
    }
    let worst = sliced.iter().zip(&kept[0]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(worst < 1e-4, "{worst}");
}

#[test]
fn sound_plays_backwards_ramps_and_holds() {
    let wav = temp("tone-time.wav");
    std::fs::write(&wav, tone_wav(48_000, 3.0)).expect("write wav");
    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(6),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    let clip = |time: TimeMap, keep_pitch| AudioClip {
        id: "tone".into(),
        asset: "a".into(),
        comp: None,
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time,
        gain_db: Animated::Constant(0.0),
        keep_pitch,
        pitch: 0.0,
        fx: vec![],
    };
    let crossings = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    let footage = Footage::new();
    let expected = 16000.0 / 32768.0 / 2f32.sqrt();
    // backwards at normal speed: the same tone, from 2 s down to 1 s
    spec.audio = vec![clip(TimeMap::new(Tick::SECOND * 2, -1.0), true)];
    let back = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    let c = crossings(&back[0][12_000..36_000]);
    assert!((c as i64 - 220).abs() <= 4, "backwards {c}");
    assert!((rms(&back[0][4_800..43_200]) - expected).abs() < 0.04);
    // a speed rising from 1 to 2 keeps the pitch, and slices match the whole
    let ramp = TimeMap {
        source_in: Tick::ZERO,
        speed: 1.0,
        ramp: vec![SpeedKey { t: Tick::ZERO, speed: 1.0 }, SpeedKey { t: Tick::SECOND, speed: 2.0 }],
    };
    spec.audio = vec![clip(ramp.clone(), true)];
    let kept = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    let c = crossings(&kept[0][12_000..36_000]);
    assert!((c as i64 - 220).abs() <= 6, "ramp kept {c}");
    let mut sliced = vec![];
    for i in 0..4 {
        sliced.extend(export::mix(&spec, &footage, 48_000, i * 12_000, 12_000).expect("mix")[0].clone());
    }
    let worst = sliced.iter().zip(&kept[0]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(worst < 1e-4, "{worst}");
    // on tape the same ramp rises in pitch: about 1.5 times the crossings over the middle half
    spec.audio = vec![clip(ramp, false)];
    let tape = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    let c = crossings(&tape[0][12_000..36_000]);
    assert!((c as i64 - 330).abs() <= 8, "ramp tape {c}");
    // a freeze frame is silent
    spec.audio = vec![clip(TimeMap::new(Tick::SECOND, 0.0), true)];
    let held = export::mix(&spec, &footage, 48_000, 0, 48_000).expect("mix");
    assert!(held[0].iter().all(|s| *s == 0.0));
}

#[test]
fn a_voice_moves_in_pitch_and_keeps_its_length() {
    let wav = temp("tone-pitch.wav");
    std::fs::write(&wav, tone_wav(48_000, 3.0)).expect("write wav");
    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(9),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    let clip = |pitch: f32, speed: f64, keep_pitch, fx| AudioClip {
        id: "tone".into(),
        asset: "a".into(),
        comp: None,
        at: Tick::from_seconds(0.25),
        dur: Tick::SECOND,
        time: TimeMap::new(Tick::ZERO, speed),
        gain_db: Animated::Constant(0.0),
        keep_pitch,
        pitch,
        fx,
    };
    let crossings = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as i64;
    let footage = Footage::new();
    let expected = 16000.0 / 32768.0 / 2f32.sqrt();
    // half a second of a 440 Hz tone has 220 rises through zero; an octave up 440, an octave down 110
    for (pitch, speed, keep, want) in [
        (12.0, 1.0, true, 440),
        (-12.0, 1.0, true, 110),
        (7.0, 1.0, true, 330),
        // at another speed with its pitch kept, the move is from the pitch it had
        (12.0, 2.0, true, 440),
        // and like tape, on top of the tape's own
        (12.0, 0.5, false, 220),
    ] {
        spec.audio = vec![clip(pitch, speed, keep, vec![])];
        let out = export::mix(&spec, &footage, 48_000, 0, 72_000).expect("mix");
        let c = crossings(&out[0][24_000..48_000]);
        assert!((c - want).abs() <= 6, "{pitch} at {speed}: {c}");
        assert!((rms(&out[0][16_800..55_200]) - expected).abs() < 0.05, "{pitch}: {}", rms(&out[0][16_800..55_200]));
        // nothing before the clip or after it
        assert!(out[0][..12_000].iter().chain(&out[0][60_000..]).all(|s| *s == 0.0));
    }
    // moved, ringed and echoed, slices of the mix are the mix
    let fx = vec![
        AudioEffect::Ring { hz: 50.0, mix: 0.5 },
        AudioEffect::Echo { delay: Tick::from_seconds(0.05), decay: 0.5, repeats: 2 },
    ];
    spec.audio = vec![clip(5.0, 1.0, true, fx)];
    let whole = export::mix(&spec, &footage, 48_000, 0, 72_000).expect("mix");
    let mut sliced = vec![];
    for i in 0..6 {
        sliced.extend(export::mix(&spec, &footage, 48_000, i * 12_000, 12_000).expect("mix")[0].clone());
    }
    let worst = sliced.iter().zip(&whole[0]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(worst < 1e-4, "{worst}");
    assert!(spec.required_features().contains("audio.pitch/1") && spec.required_features().contains("audio.fx/1"));
    reelforge_renderspec::validate(&spec).expect("valid");
}

#[test]
fn lookup_tables_load_from_files_and_bytes() {
    let cube = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let path = temp("look.cube");
    std::fs::write(&path, cube).expect("write cube");
    let footage = Footage::new();
    let asset =
        Asset { kind: AssetKind::Lut, sha256: sha(7), uri: path.to_string_lossy().into_owned(), of: None, probe: None };
    let lut = footage.lut("l", &asset).expect("reads").expect("online");
    assert_eq!(lut.size, 2);
    let probe = footage.insert_bytes(&sha(8), "Teal.CUBE", cube.as_bytes().to_vec()).expect("parses");
    assert_eq!(probe.kind, AssetKind::Lut);
    let offline = Asset { sha256: sha(8), uri: "https://example.com/teal.cube".into(), ..asset };
    assert!(footage.lut("l", &offline).expect("reads").is_some());
    assert!(footage.insert_bytes(&sha(9), "bad.cube", b"LUT_3D_SIZE 2\n".to_vec()).is_err());
}

#[test]
fn an_export_can_be_smaller_and_at_another_rate() {
    let mut spec = two_colours();
    spec.output.fps = FrameRate::FPS_25;
    spec.output.video = Some(reelforge_renderspec::VideoEncoding {
        codec: reelforge_renderspec::VideoCodec::H264,
        crf: Some(23),
        bitrate_kbps: Some(500),
        size: Some([32, 24]),
    });
    let (path, stats) = export_to_file(&spec, &Footage::new(), "small.mp4");
    assert_eq!(stats.frames, 50);
    let probe = probe_path(&path).expect("probe");
    assert_eq!((probe.size, probe.fps), (Some([32, 24]), Some(FrameRate::FPS_25)));
    // a size that would stretch the picture is refused, as is a codec this exporter lacks
    spec.output.video.as_mut().unwrap().size = Some([32, 30]);
    assert!(matches!(
        export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |_| true),
        Err(ExportError::Invalid(_))
    ));
    let v = spec.output.video.as_mut().unwrap();
    v.size = None;
    v.codec = reelforge_renderspec::VideoCodec::Hevc;
    assert!(matches!(
        export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |_| true),
        Err(ExportError::Invalid(_))
    ));
}

/// An encoder like the browser's: its configuration and frames arrive a step late.
struct Late {
    inner: crate::SoftwareH264,
    ready: Vec<crate::EncodedFrame>,
    told: bool,
}

impl crate::VideoEncoder for Late {
    fn encode(&mut self, frame: &crate::Yuv420, index: u64) -> Result<(), ExportError> {
        self.ready.extend(self.inner.take()?);
        self.inner.encode(frame, index)
    }
    fn take(&mut self) -> Result<Vec<crate::EncodedFrame>, ExportError> {
        if !self.ready.is_empty() {
            self.told = true;
        }
        Ok(std::mem::take(&mut self.ready))
    }
    fn config(&self) -> Option<crate::VideoConfig> {
        if self.told { self.inner.config() } else { None }
    }
    fn finish(&mut self) -> Result<(), ExportError> {
        self.inner.finish()?;
        self.ready.extend(self.inner.take()?);
        Ok(())
    }
    fn decode_delay(&self) -> i64 {
        self.inner.decode_delay()
    }
    fn name(&self) -> String {
        "late".into()
    }
}

#[test]
fn an_encoder_that_answers_late_still_makes_a_playable_file() {
    let spec = two_colours();
    let venc = crate::video_encoding(&spec);
    let inner = crate::SoftwareH264::new(64, 48, spec.output.fps, &venc).unwrap();
    let footage = Footage::new();
    let mut w =
        crate::ExportWriter::new(&spec, Cursor::new(Vec::new()), Box::new(Late { inner, ready: vec![], told: false }))
            .unwrap();
    let mut yuv = crate::Yuv420::default();
    for i in 0..w.frames() {
        crate::draw_frame(&spec, &footage, i, &mut yuv).unwrap();
        w.push(&spec, &footage, i, &yuv).unwrap();
    }
    w.end().unwrap();
    let (cur, stats) = w.finish(&spec, &footage).unwrap();
    assert_eq!(stats.frames, 60);
    let path = temp("late.mp4");
    std::fs::write(&path, cur.into_inner()).unwrap();
    let asset = Asset {
        kind: AssetKind::Video,
        sha256: sha(7),
        uri: path.to_string_lossy().into_owned(),
        of: None,
        probe: None,
    };
    let late = footage.picture("v", &asset, Tick::from_seconds(1.5)).expect("decode").expect("online");
    assert!(near(centre(&late), [0, 0, 255]), "{:?}", centre(&late));
}

#[test]
fn library_sounds_play_without_a_file() {
    let asset = Asset { kind: AssetKind::Audio, sha256: sha(9), uri: "library:ding".into(), of: None, probe: None };
    let footage = Footage::new();
    let sound = footage.audio(&asset, 0, 4800, 48_000).expect("made").expect("online");
    assert_eq!(sound.len(), 2);
    assert!(rms(&sound[0]) > 0.01);
}

#[test]
fn h264_files_open_for_the_browser_decoder_and_prepared_pictures_win() {
    let (path, _) = export_to_file(&two_colours(), &Footage::new(), "for-webcodecs.mp4");
    let bytes = std::fs::read(&path).expect("read");
    let v = crate::webcodecs::DecodableVideo::open(bytes.clone().into()).expect("H.264 in MP4");
    assert!(v.codec.starts_with("avc1."), "{}", v.codec);
    assert_eq!(v.size, [64, 48]);
    assert_eq!(v.len(), 60);
    assert_eq!(v.description[0], 1, "an avcC record");
    let (data, pts, dur, key) = v.sample(0).expect("first sample");
    assert!(key && !data.is_empty());
    assert_eq!((pts, dur), (0, 33_333));
    // the frame shown at a time (samples are in decode order), the last one past the end
    let shown = |t: Tick| v.sample(v.sample_at(t)).expect("sample").1;
    assert_eq!(shown(Tick::SECOND), 1_000_000);
    assert_eq!(shown(Tick::SECOND * 9), (0..v.len()).map(|i| v.sample(i).unwrap().1).max().unwrap());
    assert!(v.sync_before(30) <= 30);
    assert!(crate::webcodecs::DecodableVideo::open(b"not a video".to_vec().into()).is_none());

    let media = Footage::new();
    media.insert_bytes(&sha(7), "v.mp4", bytes).expect("insert");
    let asset = Asset { kind: AssetKind::Video, sha256: sha(7), uri: "v.mp4".into(), of: None, probe: None };
    assert!(media.video_bytes(&asset).is_some());
    let marker = Arc::new(Image::from_srgba8(2, 2, &[9; 16]));
    media.prepare(&asset, Tick::SECOND, marker.clone());
    let got = media.picture("a", &asset, Tick::SECOND).expect("picture").expect("some");
    assert!(Arc::ptr_eq(&got, &marker));
    let other = media.picture("a", &asset, Tick::ZERO).expect("picture").expect("some");
    assert!(!Arc::ptr_eq(&other, &marker));
    media.unprepare(&asset);
    let again = media.picture("a", &asset, Tick::SECOND).expect("picture").expect("some");
    assert!(!Arc::ptr_eq(&again, &marker));
}

#[test]
fn pictures_made_ahead_are_the_ones_drawn_and_only_a_few_are_kept() {
    let (path, _) = export_to_file(&two_colours(), &Footage::new(), "ahead.mp4");
    let asset = Asset {
        kind: AssetKind::Video,
        sha256: sha(10),
        uri: path.to_string_lossy().into_owned(),
        of: None,
        probe: None,
    };
    let media = Footage::new();
    let frame = |i: i64| FrameRate::FPS_30.tick_of(i);
    assert!(media.prefetch(&asset, frame(40), 1));
    let made = media.picture("v", &asset, frame(40)).expect("picture").expect("some");
    // the decoder hands back the same picture for that instant: made ahead, not again
    let again = media.picture("v", &asset, frame(40)).expect("picture").expect("some");
    assert!(Arc::ptr_eq(&made, &again));
    assert!(near(centre(&made), [0, 0, 255]), "{:?}", centre(&made));
    for i in 41..41 + crate::AHEAD_PICTURES as i64 {
        assert!(media.prefetch(&asset, frame(i), 1));
    }
    assert_eq!(media.ahead.lock().unwrap().values().next().map(Vec::len), Some(crate::AHEAD_PICTURES));
    assert!(!media.ahead.lock().unwrap().values().next().unwrap().iter().any(|(t, _)| *t == frame(40)), "oldest went");
    media.clear_ahead();
    assert!(media.ahead.lock().unwrap().is_empty());
    let still = Asset { kind: AssetKind::Image, ..asset.clone() };
    assert!(!media.prefetch(&still, frame(0), 1), "only videos are made ahead");
    media.warm(&asset, frame(10));
}

#[test]
fn a_composition_plays_its_own_mix_under_the_clips_gain() {
    let wav = temp("comp-tone.wav");
    std::fs::write(&wav, tone_wav(44_100, 2.0)).expect("write wav");
    let mut spec = two_colours();
    spec.assets.insert(
        "a".into(),
        Asset {
            kind: AssetKind::Audio,
            sha256: sha(5),
            uri: wav.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        },
    );
    let clip = |id: &str, asset: &str, comp: Option<&str>, at: Tick, dur: Tick, from: Tick, db: f32| AudioClip {
        id: id.into(),
        asset: asset.into(),
        comp: comp.map(str::to_string),
        at,
        dur,
        time: TimeMap { source_in: from, ..TimeMap::default() },
        gain_db: Animated::Constant(db),
        keep_pitch: false,
        pitch: 0.0,
        fx: vec![],
    };
    let half = Tick(Tick::SECOND.0 / 2);
    // inside: the tone for its first second. Outside: that from half a second in, for a second,
    // starting at 1 s, 6 dB down
    let inside = clip("tone", "a", None, Tick::ZERO, Tick::SECOND, Tick::ZERO, 0.0);
    spec.comps.insert("k".into(), Comp { layers: vec![], audio: vec![inside] });
    spec.audio.push(clip("c", "", Some("k"), Tick::SECOND, Tick::SECOND, half, -6.0));
    reelforge_renderspec::validate(&spec).expect("valid");
    assert!(reelforge_render::unsupported_features(&spec).is_empty());
    let footage = Footage::new();
    let m = export::mix(&spec, &footage, 48_000, 0, 96_000).expect("mix");
    let full = 16000.0 / 32768.0 / 2f32.sqrt();
    let down = full * 10f32.powf(-6.0 / 20.0);
    assert_eq!(rms(&m[0][..48_000]), 0.0, "nothing before the clip");
    assert!((rms(&m[0][48_000..72_000]) - down).abs() < 0.02, "{} vs {down}", rms(&m[0][48_000..72_000]));
    assert_eq!(rms(&m[0][72_100..]), 0.0, "the composition's tone is over");
    // mixed in pieces it is the same sound
    let mut pieces = Vec::new();
    for start in (0..96_000).step_by(7_000) {
        let n = 7_000.min(96_000 - start);
        pieces.extend(export::mix(&spec, &footage, 48_000, start as i64, n).expect("mix")[0].iter().copied());
    }
    assert_eq!(pieces, m[0]);
    // an effect on the clip works on the mix
    spec.audio[0].fx = vec![AudioEffect::Echo { delay: Tick(Tick::SECOND.0 / 4), decay: 0.5, repeats: 2 }];
    let echoed = export::mix(&spec, &footage, 48_000, 0, 96_000).expect("mix");
    assert!(rms(&echoed[0][74_000..84_000]) > 0.01, "the echo rings on after the tone");
}

#[test]
fn a_picture_between_two_frames_of_a_video_is_mixed_or_made_from_them() {
    // a picture with something to follow all over it slides 6 pixels a frame across a 96×64
    // frame, for ten frames
    let hash = |x: i64, y: i64| {
        let n = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
        (n.wrapping_mul(0xC2B2_AE35) >> 24) as f32
    };
    let soft = |x: f32, y: f32| {
        let (cx, cy) = (x / 7.0, y / 7.0);
        let (x0, y0) = (cx.floor(), cy.floor());
        let v = |dx: i64, dy: i64| hash(x0 as i64 + dx, y0 as i64 + dy);
        let (tx, ty) = (cx - x0, cy - y0);
        let (top, bottom) = (v(0, 0) + (v(1, 0) - v(0, 0)) * tx, v(0, 1) + (v(1, 1) - v(0, 1)) * tx);
        (top + (bottom - top) * ty) as u8
    };
    let mut png = Vec::new();
    image::RgbaImage::from_fn(192, 64, |x, y| {
        let v = soft(x as f32, y as f32);
        image::Rgba([v, 255 - v, v / 2 + 60, 255])
    })
    .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    let stills = Footage::new();
    stills.insert_bytes(&sha(40), "scene.png", png).expect("a picture");
    let length = FrameRate::FPS_30.frame_duration();
    let mut spec = RenderSpec::new(Output {
        width: 96,
        height: 64,
        fps: FrameRate::FPS_30,
        duration: length * 10,
        background: Color([0.0, 0.0, 0.0, 1.0]),
        video: None,
        audio: None,
        format: Default::default(),
    });
    let still = Asset { kind: AssetKind::Image, sha256: sha(40), uri: "mem:scene.png".into(), of: None, probe: None };
    spec.assets.insert("p".into(), still.clone());
    let mut sliding = layer(
        "p",
        Tick::ZERO,
        length * 10,
        LayerContent::Media {
            asset: "p".into(),
            time: TimeMap::default(),
            fit: Fit::Contain,
            between: Between::Nearest,
        },
    );
    // fitted it is half the frame's height: twice as large it fills it, and is two frames wide
    sliding.transform.scale = Animated::Constant(2.0);
    sliding.transform.position = Animated::Keys(vec![
        Key { t: Tick::ZERO, v: [-0.28125, 0.0], ease: Ease::Linear },
        Key { t: length * 9, v: [0.28125, 0.0], ease: Ease::Linear },
    ]);
    spec.layers.push(sliding);
    let (path, _) = export_to_file(&spec, &stills, "sliding.mp4");
    let video = Asset {
        kind: AssetKind::Video,
        sha256: sha(41),
        uri: path.to_string_lossy().into_owned(),
        of: None,
        probe: None,
    };
    let footage = Footage::new();
    assert_eq!(footage.frame_length(&video), Some(length));
    let picture = |at: Tick| footage.picture("v", &video, at).unwrap().expect("online");
    let made = |at: Tick, how: Between| footage.picture_between("v", &video, at, how, 1).unwrap().expect("online");
    // how far two pictures are apart away from the edges, where what slides in was not in the other frame
    let apart = |a: &Image, b: &dyn Fn(usize, usize) -> [f32; 4]| {
        let mut sum = 0.0;
        for y in 8..56 {
            for x in 16..80 {
                let (p, q) = (a.pixels[y * 96 + x], b(x, y));
                sum += (p[0] - q[0]).abs() + (p[1] - q[1]).abs();
            }
        }
        sum / (48.0 * 64.0)
    };
    let (third, fourth) = (picture(length * 3), picture(length * 4));
    let (quarter, half) = (length * 3 + Tick(length.0 / 4), length * 3 + Tick(length.0 / 2));

    // frames as they are: the frame of that instant
    assert_eq!(*made(quarter, Between::Nearest), *picture(quarter));
    // mixed: a quarter of the way from the one frame to the next
    let mixed = made(quarter, Between::Blend);
    let by_hand = |x: usize, y: usize| {
        let (p, q) = (third.pixels[y * 96 + x], fourth.pixels[y * 96 + x]);
        [0, 1, 2, 3].map(|i| p[i] + (q[i] - p[i]) * 0.25)
    };
    assert!(apart(&mixed, &by_hand) < 1e-4, "{}", apart(&mixed, &by_hand));
    // made: halfway, everything is 3 pixels further along than in the frame before
    let truth = |x: usize, y: usize| third.pixels[y * 96 + x - 3];
    let (flowed, blended) = (apart(&made(half, Between::Flow), &truth), apart(&made(half, Between::Blend), &truth));
    assert!(flowed < blended / 3.0, "{flowed} against {blended}");
    // asked again it is the same picture, kept
    assert!(Arc::ptr_eq(&made(half, Between::Flow), &made(half, Between::Flow)));
    // on a frame, that frame; past the last one, which has nothing after it, the last
    assert_eq!(*made(length * 3, Between::Flow), *third);
    let end = length * 9 + Tick(length.0 / 3);
    assert_eq!(*made(end, Between::Flow), *picture(end));
    // smaller for a draft, when asked so; and a still has no frames to be between
    let small = footage.picture_between("v", &video, half, Between::Flow, 2).unwrap().expect("online");
    assert_eq!((small.width, small.height), (48, 32));
    let still_made = stills.picture_between("p", &still, half, Between::Flow, 1).unwrap().expect("a picture");
    assert!(Arc::ptr_eq(&still_made, &stills.picture("p", &still, half).unwrap().expect("a picture")));
}
