use reelforge_renderspec::{VideoCodec, VideoEncoding};
use reelforge_time::FrameRate;

fn venc(crf: Option<u8>, bitrate_kbps: Option<u32>) -> VideoEncoding {
    VideoEncoding { codec: VideoCodec::H264, crf, bitrate_kbps, size: None }
}

#[test]
fn the_bitrate_follows_quality_size_and_rate() {
    let fps = FrameRate { num: 30, den: 1 };
    let at = |crf| crate::bitrate_kbps([1080, 1920], fps, &venc(Some(crf), None));
    assert!((5500..7000).contains(&at(20)), "{}", at(20));
    assert!(at(14).abs_diff(at(20) * 2) <= 1);
    assert!(at(26) < at(20));
    // a bitrate asked for is kept; with neither, the default quality
    assert_eq!(crate::bitrate_kbps([1080, 1920], fps, &venc(None, Some(4000))), 4000);
    assert_eq!(crate::bitrate_kbps([1080, 1920], fps, &venc(None, None)), at(20));
    assert_eq!(crate::bitrate_kbps([16, 16], fps, &venc(Some(51), None)), 200);
}

mod round_trip {
    use std::io::Cursor;

    use reelforge_media::{Footage, export};
    use reelforge_render::MediaSource;
    use reelforge_renderspec::*;
    use reelforge_time::{FrameRate, Tick};

    /// Red for a second, then blue for a second, 640×360 at 30 fps.
    fn two_colours(codec: VideoCodec) -> RenderSpec {
        let mut s = RenderSpec::new(Output {
            width: 640,
            height: 360,
            fps: FrameRate::FPS_30,
            duration: Tick::SECOND * 2,
            background: Color([0.0, 0.0, 0.0, 1.0]),
            video: Some(VideoEncoding { codec, crf: Some(20), bitrate_kbps: None, size: None }),
            audio: None,
            format: Default::default(),
        });
        for (id, at, color) in [("red", Tick::ZERO, [1.0, 0.0, 0.0, 1.0]), ("blue", Tick::SECOND, [0.0, 0.0, 1.0, 1.0])]
        {
            s.layers.push(Layer {
                id: id.into(),
                at,
                dur: Tick::SECOND,
                content: LayerContent::Solid { color: Color(color) },
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
                parent: None,
                motion_blur: None,
            });
        }
        s
    }

    /// Exports with the system's encoder for `codec`, or says why not: `None` where there is none
    /// (a failure when `REELFORGE_REQUIRE_HWENC` names the codec, as CI does where it must exist).
    fn exported(codec: VideoCodec, name: &str) -> Option<Vec<u8>> {
        crate::register();
        let spec = two_colours(codec);
        let must = std::env::var("REELFORGE_REQUIRE_HWENC").is_ok_and(|v| v.split(',').any(|c| c == name));
        let venc = spec.output.video.clone().expect("set above");
        if crate::open([640, 360], spec.output.fps, &venc).is_none() {
            assert!(!must, "no system encoder for {name}");
            eprintln!("skipping: no system encoder for {name}");
            return None;
        }
        let (out, stats) = export(&spec, &Footage::new(), Cursor::new(Vec::new()), &mut |_| true).expect("export");
        assert_eq!(stats.frames, 60);
        assert!(!stats.encoder.contains("software"), "{}", stats.encoder);
        eprintln!("{name}: {} · {} bytes", stats.encoder, out.get_ref().len());
        Some(out.into_inner())
    }

    #[test]
    fn the_system_h264_encoder_writes_a_file_the_decoder_reads() {
        let Some(bytes) = exported(VideoCodec::H264, "h264") else { return };
        let path = std::env::temp_dir().join(format!("reelforge-hwenc-{}.mp4", std::process::id()));
        std::fs::write(&path, bytes).expect("write");
        let asset = Asset {
            kind: AssetKind::Video,
            sha256: "ab".repeat(32),
            uri: path.to_string_lossy().into_owned(),
            of: None,
            probe: None,
        };
        let footage = Footage::new();
        for (t, want) in [(0.5, [255, 0, 0]), (1.5, [0, 0, 255]), (1.95, [0, 0, 255])] {
            let pic = footage.picture("v", &asset, Tick::from_seconds(t)).expect("decode").expect("online");
            let px = pic.to_srgba8();
            let i = (((pic.height / 2) * pic.width + pic.width / 2) * 4) as usize;
            let near = (0..3).all(|c| (i32::from(px[i + c]) - want[c]).abs() <= 14);
            assert!(near, "at {t}s: {:?}", &px[i..i + 3]);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_system_hevc_encoder_writes_an_hevc_track() {
        let Some(bytes) = exported(VideoCodec::Hevc, "hevc") else { return };
        let has = |tag: &[u8]| bytes.windows(4).any(|w| w == tag);
        assert!(has(b"hvcC") && (has(b"hvc1") || has(b"hev1")));
    }
}
