//! An editor joined to a real server, with an agent editing the same session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reelforge_engine::Session;
use reelforge_server::Hub;
use reelforge_server::http::HttpServer;
use serde_json::{Value, json};

use reelforge_time::Tick;

use crate::EditorApp;

fn serve(hub: &Arc<Hub>) -> String {
    let server = HttpServer::bind(hub.clone(), "127.0.0.1:0", Some("t".into())).expect("binds");
    let base = format!("http://{}", server.local_addr());
    std::thread::spawn(move || server.run());
    base
}

/// Waits up to ten seconds for `ok`.
fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn agent(hub: &Hub, op: &str, params: Value) -> Value {
    match hub.call_as("agent test", op, &params).expect("the agent's edit runs") {
        reelforge_server::Output::Json(v) => v,
        other => panic!("{other:?}"),
    }
}

fn first_track(app: &EditorApp, kind: reelforge_model::TrackKind) -> String {
    app.session().project().tracks.iter().find(|t| t.kind == kind).expect("a track").id.clone()
}

#[test]
fn an_editor_and_an_agent_edit_one_session_through_a_server() {
    let hub = Arc::new(Hub::new(Session::default()));
    let base = serve(&hub);
    let video = hub.session().project().tracks[0].id.clone();
    agent(&hub, "clip.add", json!({"track": video, "color": [1, 0, 0], "duration": {"seconds": 2}}));

    let mut app = EditorApp::join(&base, Some("t".into()), "desktop");
    let remote = app.remote().expect("joined").clone();
    let same = || *app.session().project() == *hub.session().project();
    until("the editor has the server's project", || remote.settled() && same());

    // the editor's edit reaches the server, with the same ids on both
    let video = first_track(&app, reelforge_model::TrackKind::Video);
    let added =
        app.execute("clip.add", json!({"track": video, "color": [0, 0, 1], "duration": {"seconds": 1}})).unwrap();
    let mine = added["clip"].as_str().unwrap().to_string();
    until("the server has the editor's clip", || hub.session().project().clip(&mine).is_some());
    assert_eq!(hub.session().last_change().by, "desktop");
    until("both agree", || remote.settled() && *app.session().project() == *hub.session().project());

    // the agent's edit reaches the editor, by name
    let title = first_track(&app, reelforge_model::TrackKind::Text);
    let theirs = agent(&hub, "clip.add", json!({"track": title, "text": "你好"}))["clip"].as_str().unwrap().to_string();
    until("the editor has the agent's title", || app.session().project().clip(&theirs).is_some());
    assert_eq!(app.session().last_change().by, "agent test");

    // an edit the server refuses (the agent removed the clip first) gives way to the server's state
    agent(&hub, "clip.remove", json!({"clips": mine}));
    let _ = app.execute("clip.set", json!({"clip": mine, "opacity": 0.5}));
    until("the editor follows the server", || remote.settled() && *app.session().project() == *hub.session().project());
    assert!(app.session().project().clip(&mine).is_none());

    // undo runs on the server, whose history holds everyone's edits: it brings the clip back
    app.execute("edit.undo", json!({})).unwrap();
    until("the undo comes back", || app.session().project().clip(&mine).is_some());
    until("both agree after undo", || remote.settled() && *app.session().project() == *hub.session().project());
}

#[test]
fn files_go_to_the_server_and_other_editors_fetch_them() {
    let dir = std::env::temp_dir().join(format!("reelforge-ui-remote-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shared.reel.json");
    let _ = std::fs::remove_file(&file);
    let hub = Arc::new(Hub::open(&file).unwrap());
    let base = serve(&hub);

    let png = {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(8, 4, image::Rgba([0, 200, 0, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let mut app = EditorApp::join(&base, Some("t".into()), "web");
    let remote = app.remote().unwrap().clone();
    until("joined", || remote.joined());
    remote.upload("绿色.png", png.clone(), false).unwrap();
    let mut up = Vec::new();
    until("the server has the file", || {
        up.extend(remote.take_uploaded());
        !up.is_empty()
    });
    let imported = app.execute("media.import", up[0].import.clone()).unwrap();
    let id = imported["media"]["id"].as_str().unwrap().to_string();
    until("the server has the media", || hub.session().project().media.contains_key(&id));
    let m = hub.session().project().media[&id].clone();
    assert_eq!((m.name.as_str(), m.size), ("绿色.png", Some([8, 4])));
    assert!(m.uri.starts_with(dir.join("media").to_str().unwrap()), "{}", m.uri);
    assert_eq!(std::fs::read(&m.uri).unwrap(), png);
    until("both agree", || remote.settled() && *app.session().project() == *hub.session().project());

    // a font goes the same way, called by its family
    remote.upload("a.ttf", reelforge_render::text::NOTO_SANS_SC_REGULAR.to_vec(), false).unwrap();
    let mut up = Vec::new();
    until("the server has the font", || {
        up.extend(remote.take_uploaded());
        !up.is_empty()
    });
    assert_eq!((up[0].import["kind"].as_str(), up[0].import["name"].as_str()), (Some("font"), Some("Noto Sans SC")));
    let font = app.execute("media.import", up[0].import.clone()).unwrap()["media"]["id"].as_str().unwrap().to_string();
    until("the server has the font's media", || hub.session().project().media.contains_key(&font));
    assert_eq!(hub.session().project().media[&font].kind, reelforge_model::MediaKind::Font);
    until("both agree on the font", || remote.settled() && *app.session().project() == *hub.session().project());

    // another editor joining later gets the files from the server
    let other = EditorApp::join(&base, Some("t".into()), "web");
    let r2 = other.remote().unwrap().clone();
    until("the second editor has the footage", || r2.media_epoch() > 1);
    assert_eq!(*other.session().project(), *hub.session().project());
}

#[test]
fn a_wrong_token_is_reported_and_nothing_is_joined() {
    let hub = Arc::new(Hub::new(Session::default()));
    let base = serve(&hub);
    let app = EditorApp::join(&base, Some("wrong".into()), "web");
    let remote = app.remote().unwrap().clone();
    let mut notes = Vec::new();
    until("the refusal is reported", || {
        notes.extend(remote.take_notes());
        !notes.is_empty()
    });
    assert!(notes[0].text(crate::i18n::Lang::En).contains("token"), "{notes:?}");
    assert!(!remote.joined());
}

/// A stereo 16-bit WAV: a 440 Hz tone on the left, silence on the right.
fn tone_wav(rate: u32, seconds: f32) -> Vec<u8> {
    let n = (rate as f32 * seconds) as u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + n * 4).to_le_bytes());
    b.extend(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 2, 0], rate.to_le_bytes(), (rate * 4).to_le_bytes(), [4, 0, 16, 0]] {
        b.extend(v);
    }
    b.extend(b"data");
    b.extend((n * 4).to_le_bytes());
    for i in 0..n {
        let v = ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 16000.0) as i16;
        b.extend(v.to_le_bytes());
        b.extend(0i16.to_le_bytes());
    }
    b
}

fn rms(frames: &[[f32; 2]], channel: usize) -> f32 {
    (frames.iter().map(|f| f[channel] * f[channel]).sum::<f32>() / frames.len() as f32).sqrt()
}

#[test]
fn the_preview_plays_the_mix_and_follows_edits() {
    let wav = std::env::temp_dir().join(format!("reelforge-ui-tone-{}.wav", std::process::id()));
    std::fs::write(&wav, tone_wav(44_100, 3.0)).unwrap();
    let mut app = EditorApp::new(Session::default());
    app.sound = crate::audio::Playback::silent(48_000);
    let m = app.execute("media.import", json!({"path": wav})).unwrap()["media"]["id"].as_str().unwrap().to_string();
    let audio = first_track(&app, reelforge_model::TrackKind::Audio);
    let clip =
        app.execute("clip.add", json!({"track": audio, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();
    // something to look at, so the cut is longer than the sound
    let video = first_track(&app, reelforge_model::TrackKind::Video);
    app.execute("clip.add", json!({"track": video, "color": [0, 0, 0], "duration": {"seconds": 5}})).unwrap();

    app.set_playing(true);
    app.advance(0.0);
    let (heard, at) = app.sound.take(4_800);
    assert!((rms(&heard, 0) - 16000.0 / 32768.0 / 2f32.sqrt()).abs() < 0.02, "{}", rms(&heard, 0));
    assert_eq!(rms(&heard, 1), 0.0);
    assert_eq!(at, Tick::from_seconds(0.1), "the playhead is where the sound is");

    // removing the clip while playing silences what was mixed ahead too
    app.execute("clip.remove", json!({"clips": clip})).unwrap();
    app.advance(0.0);
    assert_eq!(rms(&app.sound.take(4_800).0, 0), 0.0);

    // pausing stops the sound; playing again starts where the playhead is
    app.set_playing(false);
    assert_eq!(rms(&app.sound.take(100).0, 0), 0.0);
    app.execute("edit.undo", json!({})).unwrap();
    app.playhead = Tick::from_seconds(2.95);
    app.set_playing(true);
    app.advance(0.0);
    let (tail, _) = app.sound.take(4_800);
    assert!(rms(&tail[..2_000], 0) > 0.2 && rms(&tail[2_600..], 0) == 0.0, "the tone ends at 3 s");

    // at the end of the cut playback stops
    app.playhead = Tick::from_seconds(4.99);
    app.advance(0.1);
    assert!(!app.playing && !app.sound.running());
}

#[test]
fn dragging_the_playhead_while_paused_plays_a_moment_of_sound() {
    let wav = std::env::temp_dir().join(format!("reelforge-ui-scrub-{}.wav", std::process::id()));
    std::fs::write(&wav, tone_wav(44_100, 3.0)).unwrap();
    let mut app = EditorApp::new(Session::default());
    app.sound = crate::audio::Playback::silent(48_000);
    let m = app.execute("media.import", json!({"path": wav})).unwrap()["media"]["id"].as_str().unwrap().to_string();
    let audio = first_track(&app, reelforge_model::TrackKind::Audio);
    app.execute("clip.add", json!({"track": audio, "media": m})).unwrap();

    app.seek(Tick::from_seconds(1.0));
    app.scrub();
    let (heard, _) = app.sound.take(9_600);
    // faded in, a tone in the middle, then nothing once the moment is over
    assert!(heard[0][0].abs() < 0.01);
    assert!(rms(&heard[1_000..3_000], 0) > 0.3, "{}", rms(&heard[1_000..3_000], 0));
    assert_eq!(rms(&heard[4_400..], 0), 0.0);

    // after the end of the sound: silence
    app.seek(Tick::from_seconds(2.99));
    app.scrub();
    assert!(rms(&app.sound.take(4_320).0[2_000..], 0) == 0.0);
}

#[test]
fn the_preview_draws_as_sharp_as_the_screen_shows_it() {
    use crate::preview_scale;
    // a 1080×1920 canvas shown 960 pixels tall on a Retina screen (480 points × 2)
    let paused = preview_scale(1920.0, 960.0, true, false, false);
    assert!((paused - 0.5).abs() < 1.0 / 32.0, "{paused}");
    // never above the canvas
    assert_eq!(preview_scale(1920.0, 4000.0, true, false, false), 1.0);
    // playing in Smooth quality draws at most 720 pixels long; Full quality does not
    let smooth = preview_scale(1920.0, 1600.0, true, true, false);
    assert!(smooth * 1920.0 <= 720.0 + 1920.0 / 32.0, "{smooth}");
    assert!(preview_scale(1920.0, 1600.0, true, true, true) * 1920.0 >= 1600.0);
    // the CPU keeps to 640
    assert!(preview_scale(1920.0, 1600.0, false, false, true) * 1920.0 <= 640.0 + 1920.0 / 32.0);
    // a few points of resizing draw nothing new
    assert_eq!(preview_scale(1920.0, 950.0, true, false, false), preview_scale(1920.0, 955.0, true, false, false));
}

#[test]
fn an_unsaved_project_comes_back_and_files_dropped_again_relink() {
    let wav = tone_wav(48_000, 0.5);
    let mut app = EditorApp::demo();
    app.import_bytes("tone.wav", wav.clone(), true);
    let kept = serde_json::to_string(&*app.project()).unwrap();

    // the next run: a demo at first, then what storage held, on the same session a server would hold
    let fresh = EditorApp::demo();
    let shared = fresh.session().clone();
    let mut back = fresh.recover(Some(kept.clone()));
    assert!(Arc::ptr_eq(&shared, back.session()));
    assert_eq!(serde_json::to_string(&*back.project()).unwrap(), kept);
    assert!(back.status.contains("last project"), "{}", back.status);

    // the browser kept only the project: the same file dropped in again is not added twice
    let media = back.project().media.len();
    back.import_bytes("tone.wav", wav, false);
    assert_eq!(back.project().media.len(), media);
    assert!(back.status.contains("relinked"), "{}", back.status);

    // nothing usable kept: the demo stays
    let demo = EditorApp::demo().recover(Some("{not json".into()));
    assert_eq!(demo.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 4);
}

#[test]
fn new_empties_the_project_and_undo_brings_it_back() {
    let mut app = EditorApp::demo();
    let before = app.project();
    app.new_project();
    assert!(app.project().tracks.iter().all(|t| t.clips.is_empty()));
    assert_eq!(app.project().canvas, before.canvas);
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*app.project(), *before);
}

#[test]
fn the_captions_list_splits_and_joins_what_was_typed_in_one_step() {
    use crate::captions::Ask;
    let ctx = eframe::egui::Context::default();
    let mut app = EditorApp::new(Session::default());
    let srt = "1\n00:00:00,000 --> 00:00:02,000\n大家好欢迎\n\n2\n00:00:02,000 --> 00:00:03,000\n再见\n";
    let r = app.execute("captions.import", json!({"srt": srt})).unwrap();
    let (a, b) = (r["clips"][0].as_str().unwrap().to_string(), r["clips"][1].as_str().unwrap().to_string());
    let words = |app: &EditorApp| -> Vec<String> {
        let p = app.project();
        let t = p.track(r["track"].as_str().unwrap()).unwrap();
        t.clips
            .iter()
            .map(|c| match &c.content {
                reelforge_model::ClipContent::Text { text, .. } => text.clone(),
                _ => unreachable!(),
            })
            .collect()
    };
    let before = app.project();

    // Enter after three characters of what was typed: the typing and the cut are one undo step
    app.captions_ask(Ask::Split { clip: a.clone(), text: "大家好，欢迎回来".into(), at: 4 }, &ctx);
    assert_eq!(words(&app), ["大家好，", "欢迎回来", "再见"]);
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*app.project(), *before);

    // Backspace at the start of the second joins it, as typed, to the first, which is selected
    app.captions_ask(
        Ask::Merge { first: a.clone(), second: b.clone(), typed: Some((b.clone(), "，再会".into())) },
        &ctx,
    );
    assert_eq!(words(&app), ["大家好欢迎，再会"]);
    assert_eq!(app.selected.as_deref(), Some(a.as_str()));
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*app.project(), *before);

    // clicking a row selects its caption and goes to it
    app.captions_ask(Ask::Focus { clip: b.clone(), start: Tick::from_seconds(2.0) }, &ctx);
    assert_eq!((app.selected.as_deref(), app.playhead), (Some(b.as_str()), Tick::from_seconds(2.0)));
    app.captions_ask(
        Ask::Replace { track: r["track"].as_str().unwrap().into(), find: "再见".into(), with: "拜拜".into() },
        &ctx,
    );
    assert_eq!(words(&app)[1], "拜拜");
}

#[test]
fn stickers_and_effects_land_over_the_picture() {
    use crate::{assets, i18n};
    use reelforge_model::{FxKind, TrackKind};
    let mut app = EditorApp::new(Session::default());
    let video = app.project().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id.clone();
    app.execute("clip.add", json!({"track": video, "color": [1, 0, 0, 1], "start": 0, "duration": {"seconds": 6}}))
        .unwrap();
    let videos = |app: &EditorApp| -> Vec<Vec<String>> {
        let p = app.project();
        p.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .map(|t| t.clips.iter().map(|c| c.id.clone()).collect())
            .collect()
    };
    // the picture's track is taken there: the sticker gets a track above, named in the interface's
    // language
    app.lang = i18n::Lang::Zh;
    app.add_item(&assets::Item::Sticker("star"), Tick::ZERO, None);
    let sticker = app.selected.clone().unwrap();
    assert_eq!(videos(&app).len(), 2);
    assert_eq!(videos(&app)[1], std::slice::from_ref(&sticker));
    assert_eq!(app.project().media.values().next().unwrap().name, "星星");
    // an effect goes above that again, as an adjustment clip that is selected
    app.add_item(&assets::Item::Effect(Some(FxKind::Shake)), Tick::ZERO, None);
    let fx = app.selected.clone().unwrap();
    assert_eq!(videos(&app).len(), 3);
    let p = app.project();
    let (_, clip) = p.clip(&fx).unwrap();
    assert!(clip.is_adjustment());
    assert_eq!(clip.fx.iter().map(|f| f.kind).collect::<Vec<_>>(), [FxKind::Shake]);
    assert_eq!(clip.duration, assets::OVERLAY_LENGTH);
    // where the top track has room, the next one shares it
    app.add_item(&assets::Item::Effect(None), Tick::from_seconds(4.0), None);
    assert_eq!(videos(&app).len(), 3);
    assert_eq!(videos(&app)[2].len(), 2);
    // dropped on a track with room, it stays there
    app.add_item(&assets::Item::Sticker("heart"), Tick::from_seconds(3.5), Some(videos_track(&app, 1)));
    assert_eq!(videos(&app)[1].len(), 2);
}

/// The id of the `n`th video track from the bottom.
fn videos_track(app: &EditorApp, n: usize) -> String {
    let p = app.project();
    p.tracks.iter().filter(|t| t.kind == reelforge_model::TrackKind::Video).nth(n).unwrap().id.clone()
}

#[test]
fn the_editor_listens_with_the_media_it_holds_and_says_what_it_found() {
    let wav = std::env::temp_dir().join(format!("reelforge-ui-listen-{}.wav", std::process::id()));
    std::fs::write(&wav, tone_wav(44_100, 3.0)).unwrap();
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let m = app.execute("media.import", json!({"path": wav})).unwrap()["media"]["id"].as_str().unwrap().to_string();
    let audio = first_track(&app, reelforge_model::TrackKind::Audio);
    let clip =
        app.execute("clip.add", json!({"track": audio, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();
    let volume = |app: &EditorApp| app.project().clip(&clip).unwrap().1.volume_db;

    // the tone (in one ear, at about -9 dBFS) is brought to the target in one step that undoes as one
    app.listen("audio.normalize", &json!({"clips": [clip], "target": -20}));
    assert!(app.status.contains("1 个片段"), "{}", app.status);
    assert!((-12.0..-4.0).contains(&volume(&app)), "{}", volume(&app));
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(volume(&app), 0.0);

    // a steady tone has no beat and nothing to duck under: said, and nothing changes
    let before = app.project();
    app.listen("audio.beats", &json!({"clip": clip}));
    assert!(app.status.starts_with("踩点: "), "{}", app.status);
    app.listen("audio.duck", &json!({"clip": clip}));
    assert!(app.status.starts_with("自动压低配乐: "), "{}", app.status);
    assert!(Arc::ptr_eq(&before, &app.project()) || *before == *app.project());

    // its noise is measured (a tone is all there is, so that is what is found) and kept on the clip
    app.listen("audio.denoise", &json!({"clip": clip, "amount": 0.4}));
    assert!(app.status.contains("dB"), "{}", app.status);
    assert_eq!(app.project().clip(&clip).unwrap().1.voice.denoise.as_ref().unwrap().amount, 0.4);
}

#[test]
fn a_font_dropped_in_is_set_on_the_selected_title() {
    use crate::i18n;
    use reelforge_model::{ClipContent, MediaKind, TrackKind};
    let mut app = EditorApp::new(Session::default());
    app.lang = i18n::Lang::Zh;
    let font_of = |app: &EditorApp, clip: &str| match &app.project().clip(clip).unwrap().1.content {
        ClipContent::Text { style, .. } => style.font.clone(),
        _ => unreachable!(),
    };
    let text = first_track(&app, TrackKind::Text);
    let title =
        app.execute("clip.add", json!({"track": text, "text": "标题"})).unwrap()["clip"].as_str().unwrap().to_string();

    // nothing selected: the font waits in the project, called by its family and style
    app.selected = None;
    app.import_bytes("a.ttf", reelforge_render::text::NOTO_SANS_SC_REGULAR.to_vec(), true);
    let p = app.project();
    let regular = p.media.values().find(|m| m.kind == MediaKind::Font).expect("imported").clone();
    assert_eq!(regular.name, "Noto Sans SC");
    assert_eq!(app.status, "已导入字体 Noto Sans SC，选中标题后可在字体里选用");
    assert_eq!(font_of(&app, &title), None);
    assert!(p.tracks.iter().all(|t| t.clips.iter().all(|c| c.id == title)), "a font is not a clip");

    // with a title selected, it is set in the font, in one step that undo takes back
    app.selected = Some(title.clone());
    let bold =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../render/fonts/NotoSansSC-Bold.subset.ttf")).unwrap();
    app.import_bytes("b.otf", bold, false);
    let p = app.project();
    let bold = p.media.values().find(|m| m.name == "Noto Sans SC Bold").expect("imported");
    assert_eq!(font_of(&app, &title), Some(bold.id.clone()));
    assert_eq!(app.status, "标题已换用字体 Noto Sans SC Bold");
    // the same file again is the same font: back on the title, not in the project twice
    app.execute("clip.set", json!({"clip": title, "font": null})).unwrap();
    app.import_bytes("again.ttf", reelforge_render::text::NOTO_SANS_SC_REGULAR.to_vec(), false);
    assert_eq!(font_of(&app, &title), Some(regular.id.clone()));
    assert_eq!(app.project().media.len(), 2);
    // a file that is no font says so and adds nothing
    app.import_bytes("c.ttf", b"nothing".to_vec(), false);
    assert!(app.status.starts_with("c.ttf: "), "{}", app.status);
    assert_eq!(app.project().media.len(), 2);
}

#[test]
fn keying_finds_the_backdrop_or_says_it_guessed() {
    use reelforge_model::{ChromaKey, MediaKind, TrackKind};
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let png = |f: &dyn Fn(u32, u32) -> [u8; 3]| {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_fn(96, 54, |x, y| {
            let [r, g, b] = f(x, y);
            image::Rgba([r, g, b, 255])
        });
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let track = first_track(&app, TrackKind::Video);
    let place = |app: &mut EditorApp, name: &str, bytes: Vec<u8>, start: f64| {
        app.import_bytes(name, bytes, false);
        let p = app.project();
        let m = p.media.values().find(|m| m.kind == MediaKind::Image && m.name.starts_with(name)).expect("imported");
        let at = json!({"track": track, "media": m.id, "start": {"seconds": start}});
        app.execute("clip.add", at).unwrap()["clip"].as_str().unwrap().to_string()
    };
    let key = |app: &EditorApp, clip: &str| app.project().clip(clip).unwrap().1.look.chroma;
    // a blue screen behind a subject: its colour is found in the frame, in one step
    let screen = png(&|x, y| if (30..60).contains(&x) && y > 20 { [200, 150, 120] } else { [20, 50, 200] });
    let host = place(&mut app, "screen.png", screen, 0.0);
    app.key_out(&host);
    let found = key(&app, &host).expect("keyed");
    assert!(found.color[2] > 0.7 && found.color[0] < 0.2, "{found:?}");
    assert_eq!(app.status, "已按这一帧的背景色抠图");
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(key(&app, &host), None);

    // no single colour around the frame: green to start from, and it says so
    let busy = png(&|x, y| {
        let t = (x as f32 / 96.0 + y as f32 / 54.0) * 9.0;
        let ch = |o: f32| (127.0 + 120.0 * (t + o).sin()) as u8;
        [ch(0.0), ch(2.1), ch(4.2)]
    });
    let other = place(&mut app, "busy.png", busy, 20.0);
    app.key_out(&other);
    assert_eq!(key(&app, &other).unwrap().color, ChromaKey::GREEN);
    assert!(app.status.starts_with("没找到单一的背景色"), "{}", app.status);
    // asked again with a colour chosen by hand: it is kept
    app.execute("clip.setChromaKey", json!({"clip": other, "color": [1, 0, 1], "tolerance": 0.3})).unwrap();
    app.key_out(&other);
    let kept = key(&app, &other).unwrap();
    assert_eq!((kept.color, kept.tolerance), ([1.0, 0.0, 1.0], 0.3));
    assert!(app.status.starts_with("这一帧四周没有单一的背景色"), "{}", app.status);
}
