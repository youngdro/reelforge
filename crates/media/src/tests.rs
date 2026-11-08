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
            time: TimeMap { source_in: Tick::from_seconds(1.5), speed: 1.0 },
            fit: Fit::Contain,
        },
    ));
    let (img, report) = render_frame(&spec, Tick::ZERO, 1.0, &footage, Quality::Final).expect("render");
    assert!(report.placeholders.is_empty());
    assert!(near(centre(&img), [0, 0, 255]));
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
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::default(),
        gain_db: Animated::Constant(-6.0),
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
    spec.layers[0].fx.push(Effect { id: "blur.gaussian@1".into(), params: Default::default() });
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
