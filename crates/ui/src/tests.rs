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
fn an_unsaved_editor_keeps_drafts_to_open_start_copy_and_delete() {
    use crate::drafts::Ask;
    use crate::i18n::Lang;
    type Kept = std::collections::HashMap<String, String>;
    let mut store = Kept::new();
    let start = |store: &Kept| {
        let held = store.clone();
        EditorApp::demo().with_lang(Lang::En).with_drafts(&move |key| held.get(key).cloned())
    };
    let clips = |app: &EditorApp| app.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    let listed = |app: &EditorApp| -> Vec<(String, usize)> {
        app.drafts.as_ref().unwrap().list().iter().map(|d| (d.name.clone(), d.clips)).collect()
    };
    let current = |app: &EditorApp| app.drafts.as_ref().unwrap().current().unwrap().clone();
    let title = |app: &mut EditorApp| {
        let track = first_track(app, reelforge_model::TrackKind::Text);
        app.execute("clip.add", json!({"track": track, "text": "hi", "start": {"seconds": 20}})).unwrap();
    };

    // the one project an editor from before there were drafts kept is the first draft
    let mut old = EditorApp::demo();
    title(&mut old);
    store.insert(super::RECOVERY_KEY.to_string(), serde_json::to_string(&*old.project()).unwrap());
    let mut app = start(&store);
    assert_eq!(clips(&app), 5);
    app.keep_draft(&mut store);
    assert!(!store.contains_key(super::RECOVERY_KEY));
    assert_eq!(listed(&app), [("Draft 1".to_string(), 5)]);
    let first = current(&app).id;

    // going to the drafts page keeps the draft, with a picture of it
    app.drafts_do(Ask::Home, &mut store);
    assert!(app.drafts.as_ref().unwrap().home);
    assert!(app.drafts.as_ref().unwrap().has_cover(&store));

    // a new draft is empty, and the one being edited; nothing of the first is in its history
    app.drafts_do(Ask::New, &mut store);
    assert!(!app.drafts.as_ref().unwrap().home);
    assert_eq!((clips(&app), current(&app).name.as_str()), (0, "Draft 2"));
    assert!(app.execute("edit.undo", json!({})).is_err());
    title(&mut app);
    app.selected = app.project().tracks.iter().flat_map(|t| &t.clips).next().map(|c| c.id.clone());
    app.playhead = Tick::from_seconds(20.5);

    // opening the first keeps the second as it is, and starts at the first's beginning
    app.drafts_do(Ask::Open(first.clone()), &mut store);
    assert_eq!((clips(&app), current(&app).id), (5, first.clone()));
    assert_eq!((app.playhead, app.selected.clone()), (Tick::ZERO, None));
    assert!(app.status.contains("Draft 1"), "{}", app.status);
    let mut all = listed(&app);
    all.sort();
    assert_eq!(all, [("Draft 1".to_string(), 5), ("Draft 2".to_string(), 1)]);
    // a draft that is not there changes nothing
    app.drafts_do(Ask::Open("d99".into()), &mut store);
    assert_eq!((clips(&app), current(&app).id), (5, first.clone()));

    // the next run starts on the draft edited last
    let again = start(&store);
    assert_eq!((clips(&again), current(&again).id), (5, first.clone()));
    assert!(again.status.contains("last project"), "{}", again.status);

    // a copy and a name
    app.drafts_do(Ask::Duplicate(first.clone()), &mut store);
    app.drafts_do(Ask::Rename(first.clone(), "Beach".into()), &mut store);
    let mut all = listed(&app);
    all.sort();
    assert_eq!(all, [("Beach".to_string(), 5), ("Draft 1 copy".to_string(), 5), ("Draft 2".to_string(), 1)]);

    // deleting the draft being edited puts the editor on another; deleting the last starts a new one
    app.drafts_do(Ask::Delete(first.clone()), &mut store);
    assert_eq!(listed(&app).len(), 2);
    assert_ne!(current(&app).id, first);
    assert_eq!(clips(&app), current(&app).clips);
    for _ in 0..2 {
        let id = current(&app).id;
        app.drafts_do(Ask::Delete(id), &mut store);
    }
    assert_eq!(listed(&app), [("Draft 1".to_string(), 0)]);
    assert_eq!(clips(&app), 0);
    // nothing of the deleted drafts is left in storage: the index, and the one draft
    assert_eq!(store.len(), 2, "{:?}", store.keys().collect::<Vec<_>>());
}

#[test]
fn a_template_starts_a_draft_whose_slots_take_the_pictures_added() {
    use crate::assets::Item;
    use crate::drafts::Ask;
    use crate::i18n::Lang;
    use reelforge_engine::slot_states;
    use reelforge_model::ClipContent;
    let mut store = std::collections::HashMap::<String, String>::new();
    let mut app = EditorApp::demo().with_lang(Lang::Zh).with_drafts(&|_| None);
    let png = |shade: u8| {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(32, 18, image::Rgb([shade, 60, 90]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    };
    let filled = |app: &EditorApp| slot_states(&app.project()).iter().map(|(_, _, f)| *f).collect::<Vec<_>>();
    let clip_of = |app: &EditorApp, n: u32| slot_states(&app.project())[n as usize - 1].1.clone();
    let shows = |app: &EditorApp, clip: &str| match &app.project().clip(clip).unwrap().1.content {
        ClipContent::Media { media, .. } => app.project().media[media].name.clone(),
        _ => String::new(),
    };

    // a template from the drafts page is a new draft called what the template is called, with
    // its slots empty and its titles in the interface's language
    app.drafts_do(Ask::Template("showcase"), &mut store);
    let drafts = app.drafts.as_ref().unwrap();
    assert!(!drafts.home);
    assert_eq!(drafts.current().unwrap().name, "新品展示");
    assert_eq!(filled(&app), [false; 6]);
    let p = app.project();
    let titles: Vec<&str> = p
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["新品上市", "现已发售"]);
    assert_eq!(app.status, "新品展示：添加 6 段图片或视频即可填满");

    // pictures dropped on the editor take the empty slots in order
    app.import_bytes("a.png", png(40), true);
    app.import_bytes("b.png", png(90), true);
    assert_eq!(filled(&app), [true, true, false, false, false, false]);
    assert_eq!((shows(&app, &clip_of(&app, 1)), shows(&app, &clip_of(&app, 2))), ("a.png".into(), "b.png".into()));
    assert_eq!(app.selected, Some(clip_of(&app, 2)));
    assert_eq!(app.status, "b.png 已填入空位 2");
    let tracks = app.project().tracks.iter().map(|t| t.clips.len()).collect::<Vec<_>>();

    // one dragged from the media onto a slot goes into that slot, filled or not
    let a = app.project().media.values().find(|m| m.name == "a.png").unwrap().id.clone();
    let (track, at) = {
        let p = app.project();
        let (t, c) = p.clip(&clip_of(&app, 5)).unwrap();
        (p.tracks[t].id.clone(), c.start + Tick::from_seconds(0.5))
    };
    app.add_item(&Item::Media(a.clone()), at, Some(track.clone()));
    assert_eq!(filled(&app), [true, true, false, false, true, false]);
    let start_of_2 = app.project().clip(&clip_of(&app, 2)).unwrap().1.start;
    app.add_item(&Item::Media(a), start_of_2, Some(track));
    assert_eq!(shows(&app, &clip_of(&app, 2)), "a.png");
    assert_eq!(app.project().tracks.iter().map(|t| t.clips.len()).collect::<Vec<_>>(), tracks, "no clip was added");

    // Replace on a clip: the next picture picked takes its place
    let first = clip_of(&app, 1);
    app.selected = Some(first.clone());
    app.into = Some(first.clone());
    app.import_bytes("c.png", png(140), true);
    assert_eq!(shows(&app, &first), "c.png");
    assert_eq!((app.into.clone(), app.status.as_str()), (None, "已替换为 c.png"));
    assert_eq!(filled(&app), [true, true, false, false, true, false]);

    // a clip can stop being a slot; with no empty slot left, pictures go on the timeline as ever
    app.selected = Some(clip_of(&app, 6));
    app.mark_slots(false);
    assert_eq!(slot_states(&app.project()).len(), 5);
    for name in ["d.png", "e.png"] {
        app.import_bytes(name, png(name.as_bytes()[0]), true);
    }
    assert_eq!(filled(&app), [true; 5]);
    let before = app.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    app.playhead = Tick::from_seconds(20.0);
    app.import_bytes("f.png", png(200), true);
    assert_eq!(app.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>(), before + 1);

    // in an editor on a project that is not empty, a template gets a timeline of its own
    let mut app = EditorApp::demo().with_lang(Lang::En);
    app.use_template("beat");
    let p = app.project();
    assert_eq!(p.timeline_list().len(), 2);
    assert_eq!(p.timeline_list().iter().find(|t| t.current).unwrap().name, "Beat cuts");
    assert_eq!(slot_states(&p).len(), 10);
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(app.project().timeline_list().len(), 1);
}

#[test]
fn a_storyboard_is_used_in_the_interface_s_language_and_its_copy_typed_over() {
    use crate::i18n::Lang;
    use reelforge_engine::{made_of, slot_states};
    let mut app = EditorApp::new(Session::default()).with_lang(Lang::Zh);
    app.use_storyboard("launch");
    let p = app.project();
    // its slots wait for pictures, its copy is the storyboard's in Chinese, and the status says what next
    assert_eq!(slot_states(&p).len(), 4);
    assert_eq!(p.copy.iter().find(|f| f.name == "cta").unwrap().value, "立即抢购");
    assert_eq!(app.status, "新品首发：添加 4 段图片或视频，再改文案");
    assert_eq!(p.markers.len(), 5);
    // folded pieces are not counted among the timelines to switch between
    let pieces = p.timelines.iter().filter(|t| t.made.is_some()).count();
    assert!(pieces >= 10 && p.timeline_list().len() > pieces);
    // a field typed over: the piece written from it is made again, in one step
    let button = {
        let clips = p.tracks.iter().flat_map(|t| &t.clips);
        clips.filter(|c| made_of(&p, &c.id).is_some_and(|(_, m)| m.kind == "buyButton")).map(|c| c.id.clone()).next()
    };
    let button = button.expect("a button");
    app.fill_copy("cta", "马上带一个回家");
    assert_eq!(made_of(&app.project(), &button).unwrap().1.text, "马上带一个回家");
    assert_eq!(app.status, "文案已更新，1 个组件已重排");
    // a price that is no number is refused, and says why
    let before = app.project();
    app.fill_copy("price", "面议");
    assert_eq!(app.project(), before);
    assert!(app.status.contains("price"), "{}", app.status);
    // the templates tab draws the storyboards and the copy form without going wrong, in every language
    for lang in Lang::ALL {
        app.lang = lang;
        app.assets.show_storyboards();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, lang == Lang::Ja);
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |ui| {
                let (p, media) = (app.project(), app.media.clone());
                let _ = app.assets.show(ui, &p, &media, lang, &[], false, Tick::ZERO, &[]);
            })
            .textures_delta
            .clear();
        }
    }
    // on a project with something on it, a storyboard gets a timeline of its own, undone in one step
    let mut app = EditorApp::demo().with_lang(Lang::En);
    app.use_storyboard("flashSale");
    assert_eq!(app.project().timeline_list().iter().find(|t| t.current).unwrap().name, "Flash sale");
    app.execute("edit.undo", json!({})).unwrap();
    assert!(app.project().copy.is_empty());
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
    // an animated decoration goes over whatever shows there: a Lottie clip, as long as it says
    app.add_item(&assets::Item::Decor("tapRing"), Tick::ZERO, None);
    let ring = app.selected.clone().unwrap();
    assert_eq!(videos(&app).len(), 4);
    assert_eq!(videos(&app)[3], std::slice::from_ref(&ring));
    let p = app.project();
    let m = p.media.values().find(|m| m.kind == reelforge_model::MediaKind::Lottie).unwrap();
    assert_eq!((m.name.as_str(), m.uri.as_str()), ("点击提示", "library:decor/tapRing"));
    assert_eq!(p.clip(&ring).unwrap().1.duration, Tick::SECOND * 4);
    // dropped on a track with room it stays there; with none, it goes above as before
    app.add_item(&assets::Item::Decor("tapRing"), Tick::from_seconds(8.0), Some(videos_track(&app, 1)));
    assert_eq!(videos(&app)[1].len(), 3);
    app.add_item(&assets::Item::Decor("tapRing"), Tick::from_seconds(8.0), Some(videos_track(&app, 1)));
    assert_eq!((videos(&app)[1].len(), videos(&app).len()), (3, 4));
    assert_eq!(app.project().media.values().filter(|m| m.kind == reelforge_model::MediaKind::Lottie).count(), 1);
}

#[test]
fn a_ready_made_piece_of_many_parts_lands_as_one_clip_and_every_piece_has_a_name() {
    use crate::{assets, i18n};
    let mut app = EditorApp::new(Session::default());
    let clips = |app: &EditorApp| app.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    // a small piece is its two clips, grouped, the first of them selected
    app.add_item(&assets::Item::Promo("priceTag"), Tick::ZERO, None);
    assert_eq!(clips(&app), 2);
    let p = app.project();
    assert!(p.clip(app.selected.as_deref().unwrap()).unwrap().1.group.is_some());
    // a list would be a track a row: it comes folded into one compound clip, which is selected
    app.add_item(&assets::Item::Promo("checklist"), Tick::from_seconds(4.0), None);
    assert_eq!(clips(&app), 3);
    let p = app.project();
    let (_, folded) = p.clip(app.selected.as_deref().unwrap()).unwrap();
    assert_eq!(folded.start, Tick::from_seconds(4.0));
    assert_eq!(p.tracks_of(folded.compound().unwrap().0).unwrap().iter().map(|t| t.clips.len()).sum::<usize>(), 4);
    // the inspector shows the piece's lines, to be given others without going inside it
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| app.inspector(ui)).textures_delta.clear();
    let piece = app.inspector.as_ref().unwrap().piece.clone().expect("a piece");
    assert_eq!((piece.name, piece.text, piece.value), ("Feature checklist", None, None));
    assert_eq!(piece.lines.as_deref(), Some("持久保湿 12 小时\n温和不含香精\n敏感肌适用"));
    // the small piece's clips are ordinary clips: nothing of the kind is shown for them
    app.selected = p.tracks.iter().flat_map(|t| &t.clips).find(|c| c.group.is_some()).map(|c| c.id.clone());
    ctx.run_ui(Default::default(), |ui| app.inspector(ui)).textures_delta.clear();
    assert!(app.inspector.as_ref().unwrap().piece.is_none());
    // every piece and every list of them is named in every language
    for lang in i18n::Lang::ALL {
        let s = lang.strings();
        assert_eq!(s.promo_names.len(), reelforge_engine::PROMOS.len());
        assert_eq!(s.promo_categories.len(), reelforge_engine::Category::ALL.len());
        assert!(s.promo_names.iter().chain(&s.promo_categories).all(|n| !n.is_empty()));
        for piece in reelforge_engine::PROMOS {
            assert!(!assets::Item::Promo(piece.id).label(&app.project(), s).is_empty(), "{}", piece.id);
        }
    }
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
fn a_person_is_cut_out_of_a_picture_and_the_frame_shows_around_them() {
    use reelforge_model::{MediaKind, TrackKind};
    let mut app = EditorApp::new(Session::default());
    // a portrait in front of a studio backdrop, filling a green frame
    app.execute("project.setCanvas", json!({"width": 256, "height": 256, "background": [0, 1, 0, 1]})).unwrap();
    let portrait = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../matte/tests/astronaut.png")).unwrap();
    app.import_bytes("portrait.png", portrait, false);
    let media = app.project().media.values().find(|m| m.kind == MediaKind::Image).expect("imported").id.clone();
    let track = first_track(&app, TrackKind::Video);
    let clip =
        app.execute("clip.add", json!({"track": track, "media": media})).unwrap()["clip"].as_str().unwrap().to_string();
    let frame = |app: &EditorApp| {
        let spec = reelforge_compile::compile(&app.project(), &Default::default()).unwrap();
        reelforge_media::still(&spec, &app.media, Tick::ZERO, 1.0).unwrap()
    };
    let green = |p: [f32; 4]| p[1] > 0.98 && p[0] < 0.02 && p[2] < 0.02;
    let (face, beside, suit) = ((128, 60), (20, 30), (110, 170));
    let whole = frame(&app);
    assert!(!green(whole.get(beside.0, beside.1)));

    app.execute("clip.setPersonKey", json!({"clip": clip})).unwrap();
    let cut = frame(&app);
    // the person is as they were; beside the head the frame's own green shows
    assert_eq!(cut.get(face.0, face.1), whole.get(face.0, face.1));
    assert_eq!(cut.get(suit.0, suit.1), whole.get(suit.0, suit.1));
    assert!(green(cut.get(beside.0, beside.1)), "{:?}", cut.get(beside.0, beside.1));
    assert!(green(cut.get(235, 30)), "{:?}", cut.get(235, 30));
    // and back
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(frame(&app).pixels, whole.pixels);
}

#[test]
fn a_product_is_lifted_off_its_backdrop_and_stands_on_its_shadow() {
    use reelforge_model::{MediaKind, TrackKind};
    let mut app = EditorApp::new(Session::default());
    // a red box photographed on a pale sweep, filling a green frame
    app.execute("project.setCanvas", json!({"width": 160, "height": 120, "background": [0, 1, 0, 1]})).unwrap();
    let mut png = Vec::new();
    let photo = image::RgbaImage::from_fn(160, 120, |x, y| {
        let pale = (236 - 30 * y / 120) as u8;
        if (60..100).contains(&x) && (30..90).contains(&y) {
            image::Rgba([200, 40, 30, 255])
        } else {
            image::Rgba([pale, pale, pale - 4, 255])
        }
    });
    photo.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.import_bytes("box.png", png, false);
    let media = app.project().media.values().find(|m| m.kind == MediaKind::Image).expect("imported").id.clone();
    let track = first_track(&app, TrackKind::Video);
    let clip =
        app.execute("clip.add", json!({"track": track, "media": media})).unwrap()["clip"].as_str().unwrap().to_string();
    let frame = |app: &EditorApp| {
        let spec = reelforge_compile::compile(&app.project(), &Default::default()).unwrap();
        reelforge_media::still(&spec, &app.media, Tick::ZERO, 1.0).unwrap()
    };
    let green = |p: [f32; 4]| p[1] > 0.98 && p[0] < 0.02 && p[2] < 0.02;
    let whole = frame(&app);
    assert!(!green(whole.get(20, 20)));

    app.execute("clip.setCutout", json!({"clip": clip})).unwrap();
    let cut = frame(&app);
    // the box is as it was; all round it the frame's own green shows
    assert_eq!(cut.get(80, 60), whole.get(80, 60));
    for at in [(20, 20), (140, 100), (80, 10), (80, 110), (50, 60)] {
        assert!(green(cut.get(at.0, at.1)), "{at:?} {:?}", cut.get(at.0, at.1));
    }
    // standing on the ground: a dark pool at its foot, none of it at its head
    app.execute("clip.setShadow", json!({"clip": clip, "kind": "ground", "blur": 0.01, "opacity": 0.8})).unwrap();
    let lit = frame(&app);
    assert_eq!(lit.get(80, 60), whole.get(80, 60));
    let foot = lit.get(80, 91);
    assert!(foot[1] < 0.8 && foot[0] < 0.02, "{foot:?}");
    assert!(green(lit.get(80, 25)) && green(lit.get(80, 100)), "{:?}", lit.get(80, 100));
    // the inspector reads both back
    app.selected = Some(clip.clone());
    let p = app.project();
    let look = p.clip(&clip).unwrap().1.look.clone();
    assert!(look.cutout.is_some() && look.shadow.is_some_and(|s| s.flat > 0.8));
    // and back
    app.execute("edit.undo", json!({})).unwrap();
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(frame(&app).pixels, whole.pixels);
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

#[test]
fn selected_pictures_are_laid_out_from_the_lowest_track_up() {
    use crate::timeline::{Action, Select};
    use reelforge_model::TrackKind;
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let mut clips = vec![];
    for i in 0..2 {
        let file = json!({
            "path": format!("/nowhere/{i}.mp4"), "duration": {"seconds": 10}, "size": [1920, 1080],
            "sha256": format!("{i:02x}").repeat(32),
        });
        let media = app.execute("media.import", file).unwrap()["media"]["id"].as_str().unwrap().to_string();
        let track = match i {
            0 => first_track(&app, TrackKind::Video),
            _ => app.execute("track.add", json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string(),
        };
        let at = json!({"track": track, "media": media, "duration": {"seconds": 4}});
        clips.push(app.execute("clip.add", at).unwrap()["clip"].as_str().unwrap().to_string());
    }
    // chosen from the top down, laid out from the bottom up: the lower clip is the large one
    app.select(Select::Set(vec![clips[1].clone(), clips[0].clone()]));
    app.act(Action::Layout("pip"));
    let p = app.project();
    assert_eq!(p.clip(&clips[0]).unwrap().1.transform.scale, 1.0);
    assert!(p.clip(&clips[1]).unwrap().1.transform.scale < 0.5);
    assert!(app.status.contains("画中画"), "{}", app.status);
    app.act(Action::Layout("rows"));
    let p = app.project();
    assert!(p.clip(&clips[0]).unwrap().1.transform.y < 0.0 && p.clip(&clips[1]).unwrap().1.transform.y > 0.0);
    // one clip alone is left as it is
    app.select(Select::Only(clips[0].clone()));
    let before = app.project().clip(&clips[0]).unwrap().1.clone();
    app.act(Action::Layout("columns"));
    assert_eq!(*app.project().clip(&clips[0]).unwrap().1, before);
    // every layout is named in every language, and found in the palette
    for lang in crate::i18n::Lang::ALL {
        let s = lang.strings();
        assert_eq!(s.layout_names.len(), reelforge_engine::LAYOUTS.len());
        assert!(s.layout_names.iter().all(|n| !n.is_empty()) && s.layout_done.contains("{}"));
    }
    assert!(app.palette_entries().len() > reelforge_engine::LAYOUTS.len());
}

#[test]
fn selected_clips_move_and_are_cut_on_the_marked_beats() {
    use crate::timeline::{Action, Select};
    use reelforge_model::{Prop, TrackKind};
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let track = first_track(&app, TrackKind::Video);
    let mut clips = vec![];
    for i in 0..3 {
        let at = json!({"track": track, "color": [1, 0, 0], "start": {"seconds": 1.4 * i as f64}, "duration": {"seconds": 1.4}});
        clips.push(app.execute("clip.add", at).unwrap()["clip"].as_str().unwrap().to_string());
    }
    // with no beat marked yet, nothing changes and the status line says what to do first
    app.select(Select::Set(clips.clone()));
    let before = app.project().clone();
    app.act(Action::Beat("punch"));
    app.act(Action::BeatCut);
    assert_eq!(app.project(), before);
    assert!(app.status.contains("踩点"), "{}", app.status);

    for i in 0..6 {
        app.execute("marker.add", json!({"at": {"seconds": i as f64}, "note": reelforge_engine::listen::BEAT}))
            .unwrap();
    }
    // chosen in any order, cut in the order they are on the track: a second each
    app.select(Select::Set(vec![clips[2].clone(), clips[0].clone(), clips[1].clone()]));
    app.act(Action::BeatCut);
    let p = app.project();
    let spans: Vec<(f64, f64)> =
        clips.iter().map(|c| p.clip(c).unwrap().1).map(|c| (c.start.seconds(), c.duration.seconds())).collect();
    assert_eq!(spans, [(0.0, 1.0), (1.0, 1.0), (2.0, 1.0)]);
    assert!(app.status.contains('3'), "{}", app.status);
    // and each jumps larger where a beat falls within it
    app.act(Action::Beat("punch"));
    let p = app.project();
    assert!(clips.iter().all(|c| p.clip(c).unwrap().1.value_at(Prop::Scale, reelforge_time::Tick::ZERO) > 1.1));
    assert!(app.status.contains("缩放冲击"), "{}", app.status);
    // every move is named in every language, and found in the palette
    for lang in crate::i18n::Lang::ALL {
        let s = lang.strings();
        assert_eq!(s.beat_names.len(), reelforge_engine::BEAT_MOVES.len());
        assert!(s.beat_names.iter().all(|n| !n.is_empty()) && s.beat_done.contains("{}"));
        assert!(s.beat_cut_done.contains("{}") && !s.beat_none.is_empty() && !s.beat_cut.is_empty());
    }
    let words = app.palette_entries();
    assert!(words.len() > reelforge_engine::BEAT_MOVES.len() + reelforge_engine::LAYOUTS.len());
}

#[test]
fn a_group_is_chosen_as_one_and_a_compound_clip_is_opened_and_left() {
    use crate::timeline::{Action, Select};
    use reelforge_model::TrackKind;
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let video = first_track(&app, TrackKind::Video);
    let add = |app: &mut EditorApp, start: f64| {
        let at =
            json!({"track": video, "color": [1, 0, 0, 1], "start": {"seconds": start}, "duration": {"seconds": 2}});
        app.execute("clip.add", at).unwrap()["clip"].as_str().unwrap().to_string()
    };
    let (a, b, c) = (add(&mut app, 0.0), add(&mut app, 2.0), add(&mut app, 4.0));
    // two clips grouped: choosing either chooses both, the one clicked first
    app.select(Select::Set(vec![a.clone(), b.clone()]));
    app.act(Action::Group);
    app.select(Select::Only(b.clone()));
    assert_eq!(app.selection(), [b.clone(), a.clone()]);
    app.select(Select::Toggle(c.clone()));
    assert_eq!(app.selection().len(), 3);
    // and letting go of either lets go of both
    app.select(Select::Toggle(a.clone()));
    assert_eq!(app.selection(), std::slice::from_ref(&c));
    app.select(Select::Only(a.clone()));
    app.act(Action::Ungroup);
    app.select(Select::Only(a.clone()));
    assert_eq!(app.selection(), std::slice::from_ref(&a));

    // two clips joined: the compound clip is selected in their place
    app.select(Select::Set(vec![b.clone(), c.clone()]));
    app.act(Action::Compound);
    let made = app.selected.clone().unwrap();
    assert!(app.project().clip(&made).unwrap().1.compound().is_some());
    assert!(app.status.starts_with("已把 2 个片段"), "{}", app.status);
    // opened on the picture the playhead shows: it starts at 2 s, so 3 s is 1 s inside
    app.playhead = Tick::from_seconds(3.0);
    app.act(Action::OpenCompound);
    let p = app.project();
    assert!(p.timeline_list().iter().any(|t| t.current && t.compound));
    assert_eq!(p.tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 2);
    assert_eq!(app.playhead, Tick::from_seconds(1.0));
    assert!(app.selection().is_empty());
    assert!(app.status.starts_with("正在编辑复合片段 1"), "{}", app.status);
    // left again: back on the timeline that plays it, on the same picture, with it selected
    app.playhead = Tick::from_seconds(3.5);
    app.leave_compound();
    assert!(app.project().timeline_list().iter().any(|t| t.current && !t.compound));
    assert_eq!(app.playhead, Tick::from_seconds(5.5));
    assert_eq!(app.selection(), std::slice::from_ref(&made));
    // taken apart: its clips are back and selected, and its timeline is gone
    app.act(Action::Uncompound);
    assert_eq!(app.selection().len(), 2);
    assert!(app.project().timeline_list().iter().all(|t| !t.compound));
}

#[test]
fn trimming_speech_lists_pauses_and_fillers_and_cuts_the_ticked_ones() {
    use crate::timeline::Select;
    use reelforge_model::TrackKind;
    // someone speaks from 0.5 s to 1.5 s and from 3 s to 4 s; music plays under all five seconds
    let spoken = |seconds: f32| {
        let mut wav = tone_wav(48_000, seconds);
        for (i, frame) in wav[44..].chunks_mut(4).enumerate() {
            let t = i as f32 / 48_000.0;
            if !((0.5..1.5).contains(&t) || (3.0..4.0).contains(&t)) {
                frame.fill(0);
            }
        }
        wav
    };
    let dir = std::env::temp_dir();
    let (voice, bed) = (
        dir.join(format!("reelforge-ui-spoken-{}.wav", std::process::id())),
        dir.join(format!("reelforge-ui-bed-{}.wav", std::process::id())),
    );
    std::fs::write(&voice, spoken(5.0)).unwrap();
    std::fs::write(&bed, tone_wav(44_100, 5.0)).unwrap();
    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    let id = |v: Value, key: &str| v[key].as_str().unwrap().to_string();
    let import = |app: &mut EditorApp, path: &std::path::Path| {
        id(app.execute("media.import", json!({"path": path})).unwrap()["media"].clone(), "id")
    };
    let (voice, bed) = (import(&mut app, &voice), import(&mut app, &bed));
    let a1 = first_track(&app, TrackKind::Audio);
    let a2 = id(app.execute("track.add", json!({"kind": "audio"})).unwrap(), "track");
    let voice = id(app.execute("clip.add", json!({"track": a1, "media": voice, "start": 0})).unwrap(), "clip");
    app.execute("clip.add", json!({"track": a2, "media": bed, "start": 0})).unwrap();
    let text = first_track(&app, TrackKind::Text);
    for (words, from, len) in [("你好", 0.5, 1.0), ("嗯……", 3.0, 0.4)] {
        let cue = json!({"track": text, "text": words, "start": {"seconds": from}, "duration": {"seconds": len}});
        app.execute("clip.add", cue).unwrap();
    }
    let secs = |t: Tick| (t.seconds() * 100.0).round() / 100.0;
    let on = |app: &EditorApp, track: &str| {
        let p = app.project();
        p.track(track).unwrap().clips.iter().map(|c| (secs(c.start), secs(c.end()))).collect::<Vec<_>>()
    };

    // the selected clip is listened to: three pauses, and the caption that says nothing
    app.select(Select::Only(voice.clone()));
    app.spoken_find();
    let found: Vec<_> = app.spoken.found.iter().map(|f| (f.kind, secs(f.from), secs(f.to), f.on)).collect();
    use crate::spoken::Kind::{Filler, Pause};
    assert_eq!(
        found,
        [(Pause, 0.0, 0.4, true), (Pause, 1.6, 2.9, true), (Filler, 3.0, 3.4, true), (Pause, 4.1, 5.0, true)]
    );
    assert!(!app.spoken.unheard);
    // the last pause stays; the rest goes from the voice and the captions, not from the music
    app.spoken.found[3].on = false;
    // (the tenth of a second kept between a pause and the filler after it goes with them)
    app.spoken.found[3].on = false;
    assert_eq!(app.spoken.total(), (3, Tick::from_seconds(2.2)));
    app.playhead = Tick::from_seconds(3.2);
    app.spoken_cut();
    assert_eq!(on(&app, &a1), [(0.0, 1.2), (1.2, 2.8)]);
    assert_eq!(on(&app, &text), [(0.1, 1.1)]);
    assert_eq!(on(&app, &a2), [(0.0, 5.0)]);
    assert_eq!(app.status, "已删除 3 处，缩短 2.2 秒");
    assert_eq!(secs(app.playhead), 1.2, "the playhead is on what it showed");
    assert!(app.spoken.found.is_empty());
    // one step back; and a list found before another edit is not offered after it
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(on(&app, &a1), [(0.0, 5.0)]);
    app.spoken_find();
    assert_eq!(app.spoken.found.len(), 4);
    app.execute("clip.set", json!({"clip": voice, "volumeDb": -3})).unwrap();
    app.spoken_window(&egui::Context::default());
    assert!(app.spoken.found.is_empty());
    // with nothing selected the music is heard too, and only the caption is found
    app.select(Select::Set(Vec::new()));
    app.spoken_find();
    let kinds: Vec<_> = app.spoken.found.iter().map(|f| f.kind).collect();
    assert_eq!(kinds, [Filler]);
}

#[test]
fn the_palette_finds_actions_titles_and_markers_and_does_what_is_chosen() {
    use crate::palette::Run;
    let mut app = EditorApp::new(Session::default());
    let ctx = eframe::egui::Context::default();
    let text = first_track(&app, reelforge_model::TrackKind::Text);
    let at = Tick::from_seconds(2.0);
    let added = app.execute("clip.add", json!({"track": text, "text": "Hello there", "start": at})).unwrap();
    let title = added["clip"].as_str().unwrap().to_string();
    app.execute("marker.add", json!({"at": Tick::from_seconds(1.0), "note": "chorus"})).unwrap();

    let entries = app.palette_entries();
    let find = |label: &str| entries.iter().find(|e| e.label.contains(label)).unwrap_or_else(|| panic!("{label}"));
    assert_eq!(find("Hello there").run, Run::Clip(title.clone(), at));
    assert_eq!(find("chorus").run, Run::Seek(Tick::from_seconds(1.0)));
    // nothing is selected yet: there is nothing to delete, and one clip is no group
    assert!(!find("Duplicate").enabled);
    assert!(find("Split").enabled);

    // a title chosen is selected, with the playhead at its start
    app.palette_run(Run::Clip(title.clone(), at), &ctx);
    assert_eq!(app.selection(), vec![title.clone()]);
    assert_eq!(app.playhead, at);
    assert!(app.palette_entries().iter().any(|e| e.label.contains("Duplicate") && e.enabled));
    // an edit chosen is the edit the key makes
    app.palette_run(Run::Act(crate::timeline::Action::Delete), &ctx);
    assert!(app.session().project().clip(&title).is_none());
    app.palette_run(Run::Undo, &ctx);
    assert!(app.session().project().clip(&title).is_some());
    // a time past the end stops at the end
    app.palette_run(Run::Seek(Tick::from_seconds(99.0)), &ctx);
    assert_eq!(app.playhead, app.session().project().duration());
    app.palette_run(Run::Lang(crate::i18n::Lang::Zh), &ctx);
    assert!(app.palette_entries().iter().any(|e| e.label == "分割"));
}

#[test]
fn a_shaky_video_is_watched_and_held_steady() {
    use reelforge_media::export::{Renderer, export_with};
    use reelforge_model::{MediaKind, TrackKind};
    // a picture with something to follow everywhere in it, shaken a little each frame and
    // exported: a second and a half of hand-held video whose shake is known
    let (frames, shake) = (45usize, |i: usize| [0.015 * (i as f32 * 1.7).sin(), 0.012 * (i as f32 * 2.1).cos()]);
    let mut png = Vec::new();
    let hash =
        |x: u32, y: u32| (x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35) >> 24;
    let scene = image::RgbaImage::from_fn(480, 270, |x, y| {
        let v = (hash(x / 13, y / 11) / 2 + hash(x / 37, y / 29) / 2) as u8;
        image::Rgba([v, 255 - v / 2, (hash(x / 19, y / 17)) as u8, 255])
    });
    scene.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut shot = EditorApp::new(Session::default());
    shot.execute("project.setCanvas", json!({"width": 320, "height": 180})).unwrap();
    shot.import_bytes("scene.png", png, false);
    let scene = shot.project().media.values().find(|m| m.kind == MediaKind::Image).expect("imported").id.clone();
    let track = first_track(&shot, TrackKind::Video);
    let length = Tick(Tick::SECOND.0 / 30);
    let on = json!({"track": track, "media": scene, "duration": {"ticks": (length * frames as i64).0}});
    let held = shot.execute("clip.add", on).unwrap()["clip"].as_str().unwrap().to_string();
    // larger than the frame, so that no edge comes into sight
    shot.execute("clip.set", json!({"clip": held, "scale": 1.3})).unwrap();
    let steps: Vec<Value> = (0..frames)
        .flat_map(|i| {
            let at = json!({"ticks": (length * i as i64).0});
            [("x", shake(i)[0]), ("y", shake(i)[1])].map(|(prop, value)| {
                json!({"id": "clip.setKey", "params": {"clip": held, "prop": prop, "at": at, "value": value, "ease": "hold"}})
            })
        })
        .collect();
    shot.execute("edit.batch", json!({"steps": steps})).unwrap();
    let spec = reelforge_compile::compile(&shot.project(), &Default::default()).unwrap();
    let (file, _) =
        export_with(&spec, &shot.media, std::io::Cursor::new(Vec::new()), Renderer::Cpu, &mut |_| true).unwrap();

    let mut app = EditorApp::new(Session::default());
    app.lang = crate::i18n::Lang::Zh;
    app.import_bytes("shaky.mp4", file.into_inner(), false);
    let video = app.project().media.values().find(|m| m.kind == MediaKind::Video).expect("imported").id.clone();
    let track = first_track(&app, TrackKind::Video);
    let clip =
        app.execute("clip.add", json!({"track": track, "media": video})).unwrap()["clip"].as_str().unwrap().to_string();
    let steady = |app: &EditorApp| app.project().clip(&clip).unwrap().1.steady.clone();

    // every frame the clip shows is watched; what is kept is where the picture was in each
    app.listen("clip.stabilize", &json!({"clip": clip}));
    assert!(app.status.starts_with("已分析 ") && app.status.contains("画面已稳住"), "{}", app.status);
    let found = steady(&app).expect("steadied");
    assert_eq!((found.amount, found.from, found.step), (0.5, Tick::ZERO, length));
    assert!(found.path.len() >= frames, "{}", found.path.len());
    for i in 1..frames {
        let (was, x, y) = (found.path[i], shake(i)[0] - shake(0)[0], shake(i)[1] - shake(0)[1]);
        assert!((was[0] - x).abs() < 0.004 && (was[1] - y).abs() < 0.004, "{i}: {was:?} against {x} {y}");
        assert!(was[2].abs() < 0.3, "{i}: turned {}", was[2]);
    }
    // the exported layer is moved against it: where the picture ends up no longer jumps
    let spec = reelforge_compile::compile(&app.project(), &Default::default()).unwrap();
    let moved = &spec.layers[0].transform.position;
    let jumps = |at: &dyn Fn(usize) -> f32| {
        (2..frames - 2).map(|i| (at(i + 1) - 2.0 * at(i) + at(i - 1)).abs()).fold(0.0f32, f32::max)
    };
    let zoom = spec.layers[0].transform.scale.sample(Tick::ZERO);
    let (before, after) =
        (jumps(&|i| shake(i)[0]), jumps(&|i| shake(i)[0] + moved.sample(length * i as i64)[0] / zoom));
    assert!(zoom > 1.0 && zoom < 1.2 && after < before / 4.0, "{zoom} {before} {after}");
    // one step, which undo takes back; a title has no camera to follow, and it says so
    app.execute("edit.undo", json!({})).unwrap();
    assert_eq!(steady(&app), None);
    let text = first_track(&app, TrackKind::Text);
    let title =
        app.execute("clip.add", json!({"track": text, "text": "标题"})).unwrap()["clip"].as_str().unwrap().to_string();
    app.listen("clip.stabilize", &json!({"clip": title}));
    assert!(app.status.starts_with("防抖: "), "{}", app.status);
}

#[test]
fn clips_another_client_changed_are_lit_with_its_name_and_the_news_is_on_a_toast() {
    let hub = Arc::new(Hub::new(Session::default()));
    let mut app = EditorApp::shared(hub.session().clone());
    let ctx = eframe::egui::Context::default();
    let video = first_track(&app, reelforge_model::TrackKind::Video);
    // the editor's own edit lights nothing
    let own = app.execute("clip.add", json!({"track": video, "color": [0, 0, 1], "duration": {"seconds": 1}})).unwrap();
    let own = own["clip"].as_str().unwrap().to_string();
    app.notice(1.0);
    assert!(app.timeline.touched.is_empty());

    // an agent adds a clip, then changes the editor's: both are lit, each once, with who it was
    let added = agent(&hub, "clip.add", json!({"track": video, "color": [1, 0, 0], "start": {"seconds": 2}}));
    app.notice(2.0);
    agent(&hub, "clip.set", json!({"clip": own, "opacity": 0.5}));
    app.notice(3.0);
    agent(&hub, "clip.set", json!({"clip": own, "opacity": 0.8}));
    app.notice(4.0);
    let lit: Vec<(&str, &str, f64)> =
        app.timeline.touched.iter().map(|t| (t.clip.as_str(), t.by.as_str(), t.at)).collect();
    assert_eq!(lit, [(added["clip"].as_str().unwrap(), "agent test", 2.0), (own.as_str(), "agent test", 4.0)]);

    // the news is put on a toast once, and it leaves after a while
    ctx.run_ui(Default::default(), |ui| app.toasts(ui.ctx())).textures_delta.clear();
    let (text, warn, _) = app.toast.clone().expect("a toast");
    assert!(text.contains("agent test") && !warn, "{text}");
    app.toast = None;
    ctx.run_ui(Default::default(), |ui| app.toasts(ui.ctx())).textures_delta.clear();
    assert!(app.toast.is_none(), "the same news is not shown twice");
}

#[test]
fn a_lottie_animation_dropped_on_the_editor_is_placed_and_says_what_it_cannot_draw() {
    use crate::i18n::Lang;
    use reelforge_model::{ClipContent, MediaKind, TrackKind};
    // a dot, two seconds long; the second one is moved by an expression, which is not run
    let dot = |extra: &str| {
        format!(
            r#"{{"v":"5.7.0","fr":25,"ip":0,"op":50,"w":200,"h":100,"layers":[
          {{"ty":4,"ind":1,"ip":0,"op":50,"st":0,"ks":{{"p":{{"a":0,"k":[100,50]{extra}}}}},
           "shapes":[{{"ty":"el","p":{{"a":0,"k":[0,0]}},"s":{{"a":0,"k":[40,40]}}}},
                     {{"ty":"fl","c":{{"a":0,"k":[0,0,1,1]}},"o":{{"a":0,"k":100}}}}]}}]}}"#
        )
        .into_bytes()
    };
    let mut app = EditorApp::demo().with_lang(Lang::Zh);
    let clips = |app: &EditorApp| app.project().tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    let before = clips(&app);

    app.import_bytes("dot.json", dot(""), true);
    let p = app.project();
    let m = p.media.values().find(|m| m.name == "dot.json").expect("imported");
    assert_eq!((m.kind, m.size, m.duration), (MediaKind::Lottie, Some([800, 400]), Some(Tick::SECOND * 2)));
    // placed on a video track at the playhead, selected, as long as it is
    let (t, c) = p.clip(app.selected.as_ref().expect("selected")).unwrap();
    assert_eq!(p.tracks[t].kind, TrackKind::Video);
    assert!(matches!(&c.content, ClipContent::Media { media, .. } if *media == m.id));
    assert_eq!((clips(&app), c.duration), (before + 1, Tick::SECOND * 2));
    assert_eq!(app.status, "已导入 dot.json");

    app.import_bytes("moved.json", dot(r#","x":"var $bm_rt = time;""#), false);
    assert_eq!(app.status, "已添加 moved.json。其中这些内容不会绘制：表达式");
    // what is not an animation is not added
    let media = app.project().media.len();
    app.import_bytes("package.json", br#"{"name": "x"}"#.to_vec(), true);
    assert_eq!(app.project().media.len(), media);
}

#[test]
fn the_inspector_keeps_its_width_whatever_is_selected() {
    use eframe::egui;
    for lang in crate::i18n::Lang::ALL {
        let mut app = EditorApp::new(Session::default()).with_lang(lang);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, lang == crate::i18n::Lang::Ja);
        let text = first_track(&app, reelforge_model::TrackKind::Text);
        let video = first_track(&app, reelforge_model::TrackKind::Video);
        let title = app.execute("clip.add", json!({"track": text, "text": "Hello", "start": 0})).unwrap();
        let solid = app.execute("clip.add", json!({"track": video, "color": [1, 0, 0], "start": 0})).unwrap();
        for clip in [title, solid] {
            app.selected = clip["clip"].as_str().map(str::to_string);
            let mut widths = Vec::new();
            for _ in 0..12 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))),
                    ..Default::default()
                };
                let mut width = 0.0;
                ctx.run_ui(input, |ui| {
                    let shown = egui::Panel::right("inspector")
                        .default_size(290.0)
                        .resizable(true)
                        .frame(crate::theme::island(12))
                        .show(ui, |ui| app.inspector(ui));
                    width = shown.response.rect.width();
                })
                .textures_delta
                .clear();
                widths.push(width);
            }
            // as wide as it was made, frame after frame: a row that asks for more than there is
            // widens the panel by that much every frame
            assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.5 && *w < 300.0), "{lang:?} {clip}: {widths:?}");
        }
    }
}

#[test]
fn scrolling_over_the_picture_moves_it_and_keeps_some_of_it_in_sight() {
    use eframe::egui;
    let mut app = EditorApp::new(Session::default());
    let ctx = egui::Context::default();
    crate::theme::install(&ctx, false);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
    let frame = |app: &mut EditorApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| app.preview(ui, 16.0));
        })
        .textures_delta
        .clear();
    };
    let wheel = |delta: egui::Vec2, modifiers: egui::Modifiers| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers,
    };
    frame(&mut app, vec![egui::Event::PointerMoved(egui::pos2(450.0, 300.0))]);
    frame(&mut app, vec![]);
    assert_eq!((app.pan, app.zoom), (egui::Vec2::ZERO, 1.0));
    // the picture goes the way the content is scrolled
    frame(&mut app, vec![wheel(egui::vec2(30.0, -60.0), egui::Modifiers::NONE)]);
    for _ in 0..30 {
        frame(&mut app, vec![]);
    }
    assert!((app.pan - egui::vec2(30.0, -60.0)).length() < 2.0 && app.zoom == 1.0, "{:?} {}", app.pan, app.zoom);
    // with the zoom modifier the wheel enlarges it, and does not move it by the scroll
    let before = app.pan;
    frame(&mut app, vec![wheel(egui::vec2(0.0, 40.0), egui::Modifiers::COMMAND)]);
    for _ in 0..30 {
        frame(&mut app, vec![]);
    }
    assert!(app.zoom > 1.0 && (app.pan.y - before.y).abs() < 40.0, "{:?} {}", app.pan, app.zoom);
    // however far it is scrolled, some of the picture stays on the stage
    for _ in 0..40 {
        frame(&mut app, vec![wheel(egui::vec2(0.0, -500.0), egui::Modifiers::NONE)]);
    }
    for _ in 0..30 {
        frame(&mut app, vec![]);
    }
    let lit = app.lit.expect("a picture was drawn");
    assert!(lit.frame.bottom() >= 16.0 + 60.0 && lit.frame.bottom() < 200.0, "{:?}", lit.frame);
    // and away from the picture the wheel is not its business
    let before = app.pan;
    frame(&mut app, vec![egui::Event::PointerGone]);
    frame(&mut app, vec![wheel(egui::vec2(0.0, 50.0), egui::Modifiers::NONE)]);
    for _ in 0..10 {
        frame(&mut app, vec![]);
    }
    assert_eq!(app.pan, before);
}
