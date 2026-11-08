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
        LayerContent::Media { asset: "v".into(), time: TimeMap::new(Tick::from_seconds(1.5), 1.0), fit: Fit::Contain },
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
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::default(),
        gain_db: Animated::Constant(-6.0),
        keep_pitch: false,
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
    spec.layers[0].fx.push(Effect { id: "blur.gaussian@1".into(), params: Default::default(), asset: None });
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
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time: TimeMap::new(Tick::ZERO, 2.0),
        gain_db: Animated::Constant(0.0),
        keep_pitch,
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
        at: Tick::ZERO,
        dur: Tick::SECOND,
        time,
        gain_db: Animated::Constant(0.0),
        keep_pitch,
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
