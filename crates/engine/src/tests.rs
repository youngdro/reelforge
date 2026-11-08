use reelforge_model::{ClipContent, TrackKind};
use reelforge_time::Tick;
use serde_json::json;

use super::*;

fn video_track(s: &Session) -> String {
    s.project().tracks[0].id.clone()
}

fn add_red(s: &mut Session, start: f64, secs: f64) -> String {
    let t = video_track(s);
    let r = s
        .execute(
            "clip.add",
            &json!({"track": t, "color": [1, 0, 0], "start": {"seconds": start}, "duration": {"seconds": secs}}),
        )
        .unwrap();
    r["clip"].as_str().unwrap().to_string()
}

#[test]
fn every_command_is_documented_and_unique() {
    let mut ids = std::collections::BTreeSet::new();
    for c in registry() {
        assert!(ids.insert(c.id), "duplicate {}", c.id);
        assert!(c.id.contains('.') && !c.doc.is_empty() && !c.params.is_empty(), "{}", c.id);
    }
}

#[test]
fn split_trim_move_and_undo() {
    let mut s = Session::default();
    let c = add_red(&mut s, 0.0, 4.0);
    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 1}})).unwrap();
    let right = r["right"].as_str().unwrap().to_string();
    assert_eq!(s.project().tracks[0].clips.len(), 2);

    s.execute("clip.trim", &json!({"clip": right, "end": {"seconds": 3}})).unwrap();
    assert_eq!(s.project().clip(&right).unwrap().1.duration, Tick::SECOND * 2);

    // Moving onto the left clip overlaps it: refused, nothing changes.
    let rev = s.revision();
    assert!(s.execute("clip.move", &json!({"clip": right, "start": 0})).is_err());
    assert_eq!(s.revision(), rev);

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().clip(&right).unwrap().1.duration, Tick::SECOND * 3);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.project().clip(&right).unwrap().1.duration, Tick::SECOND * 2);
    assert_eq!(s.history(), ["Add Clip", "Split", "Trim"]);
}

#[test]
fn split_keeps_media_in_sync_and_speed_keeps_the_source_span() {
    let mut s = Session::default();
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/a.mp4", "duration": {"seconds": 10}, "sha256": "ab".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t = video_track(&s);
    let c = s.execute("clip.add", &json!({"track": t, "media": m, "sourceIn": {"seconds": 2}})).unwrap();
    let c = c["clip"].as_str().unwrap().to_string();
    assert_eq!(s.project().clip(&c).unwrap().1.duration, Tick::SECOND * 8);

    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 3}})).unwrap();
    let right = r["right"].as_str().unwrap().to_string();
    let p = s.project();
    let ClipContent::Media { source_in, .. } = &p.clip(&right).unwrap().1.content else { panic!() };
    assert_eq!(*source_in, Tick::SECOND * 5);

    s.execute("clip.setSpeed", &json!({"clip": right, "speed": 2.0})).unwrap();
    assert_eq!(s.project().clip(&right).unwrap().1.duration, Tick::from_seconds(2.5));
}

#[test]
fn a_failing_batch_applies_nothing_and_a_good_one_is_one_undo_step() {
    let mut s = Session::default();
    let t = video_track(&s);
    let bad = json!({"steps": [
        {"id": "clip.add", "params": {"track": t, "color": [0, 1, 0]}},
        {"id": "clip.split", "params": {"clip": "nope", "at": 1}},
    ]});
    assert!(s.execute("edit.batch", &bad).is_err());
    assert_eq!(s.project().duration(), Tick::ZERO);
    assert!(s.history().is_empty());

    let good = json!({"label": "Intro", "steps": [
        {"id": "clip.add", "params": {"track": t, "color": [0, 1, 0]}},
        {"id": "clip.add", "params": {"track": t, "color": [0, 0, 1]}},
    ]});
    s.execute("edit.batch", &good).unwrap();
    assert_eq!(s.project().tracks[0].clips.len(), 2);
    assert_eq!(s.history(), ["Intro"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().tracks[0].clips.len(), 0);
}

#[test]
fn compile_produces_a_valid_spec() {
    let mut s = Session::default();
    add_red(&mut s, 0.0, 2.0);
    let text_track = s.project().tracks[1].id.clone();
    s.execute("clip.add", &json!({"track": text_track, "text": "Hello", "start": 0})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let spec: reelforge_renderspec::RenderSpec = serde_json::from_value(spec).unwrap();
    reelforge_renderspec::validate(&spec).unwrap();
    assert_eq!(spec.layers.len(), 2);
}

#[test]
fn errors_explain_themselves() {
    let mut s = Session::default();
    assert_eq!(s.execute("nope", &json!({})), Err(EngineError::Unknown("nope".into())));
    assert_eq!(s.execute("edit.undo", &json!({})), Err(EngineError::Disabled("nothing to undo".into())));
    let e = s.execute("clip.add", &json!({"track": "t1"})).unwrap_err();
    assert!(e.to_string().contains("`media`, `text` or `color`"));
}

#[test]
fn importing_a_file_measures_it_and_clips_default_to_its_length() {
    // half a second of 8 kHz mono silence, as a WAV
    let n: u32 = 4000;
    let mut wav = Vec::new();
    for part in [&b"RIFF"[..], &(36 + n * 2).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes()] {
        wav.extend_from_slice(part);
    }
    for part in [&1u16.to_le_bytes()[..], &8000u32.to_le_bytes(), &16000u32.to_le_bytes(), &2u16.to_le_bytes()] {
        wav.extend_from_slice(part);
    }
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(n * 2).to_le_bytes());
    wav.resize(wav.len() + n as usize * 2, 0);
    let path = std::env::temp_dir().join(format!("reelforge-engine-{}.wav", std::process::id()));
    std::fs::write(&path, &wav).unwrap();

    let mut s = Session::default();
    let r = s.execute("media.import", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["media"]["kind"], "audio");
    assert_eq!(r["media"]["sha256"].as_str().unwrap().len(), 64);
    let id = r["media"]["id"].as_str().unwrap().to_string();
    assert_eq!(s.project().media[&id].duration, Some(Tick::from_seconds(0.5)));

    let audio = s.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id.clone();
    let c = s.execute("clip.add", &json!({"track": audio, "media": id})).unwrap();
    assert_eq!(c["duration"], json!(Tick::from_seconds(0.5)));

    // what cannot be read is refused, and nothing changes
    std::fs::write(&path, b"not media").unwrap();
    let before = s.revision();
    assert!(s.execute("media.import", &json!({"path": path.to_string_lossy(), "kind": "audio"})).is_err());
    assert_eq!(s.revision(), before);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn titles_get_the_subtitle_look_and_can_be_restyled() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let c = s.execute("clip.add", &json!({"track": text, "text": "第一句字幕", "size": 60})).unwrap();
    let id = c["clip"].as_str().unwrap().to_string();
    let style = |s: &Session| match &s.project().clip(&id).unwrap().1.content {
        ClipContent::Text { style, .. } => style.clone(),
        _ => unreachable!(),
    };
    assert_eq!(style(&s), reelforge_model::TitleStyle::subtitle());

    let spec = s.execute("render.compile", &json!({})).unwrap();
    let content = &spec["layers"][0]["content"];
    assert_eq!(content["weight"], 700);
    assert_eq!(content["width"].as_f64().unwrap(), 0.9 * 1080.0);
    assert!((content["stroke"]["width"].as_f64().unwrap() - 4.8).abs() < 1e-4, "0.08 em of 60 px");

    s.execute(
        "clip.set",
        &json!({"clip": id, "weight": 400, "outline": null, "background": [0, 0, 0, 0.6], "align": "left"}),
    )
    .unwrap();
    let st = style(&s);
    assert_eq!((st.weight, st.outline, st.background), (400, None, Some([0.0, 0.0, 0.0, 0.6])));
    assert_eq!(st.align, reelforge_model::TitleAlign::Left);

    let before = s.revision();
    assert!(s.execute("clip.set", &json!({"clip": id, "weight": 1000})).is_err());
    assert!(s.execute("clip.set", &json!({"clip": id, "wrap": 2})).is_err());
    let colour = add_red(&mut s, 0.0, 1.0);
    assert!(s.execute("clip.set", &json!({"clip": colour, "weight": 700})).is_err());
    assert_eq!(s.revision(), before + 1, "only the colour clip was added");
}

#[test]
fn every_command_has_a_schema_that_names_its_required_fields() {
    for c in registry() {
        let schema = crate::input_schema(c.id).unwrap_or_else(|| panic!("{} has no schema", c.id));
        assert_eq!(schema["type"], "object", "{}", c.id);
        for r in schema["required"].as_array().expect("required") {
            assert!(schema["properties"].get(r.as_str().unwrap()).is_some(), "{} requires unknown {r}", c.id);
        }
        // every property the informal description names is in the schema, and the other way round
        for (k, _) in schema["properties"].as_object().unwrap() {
            assert!(c.params.contains(&format!("\"{k}\"")), "{}: {k} is not in {:?}", c.id, c.params);
        }
    }
    assert!(crate::input_schema("no.such").is_none());
}

#[test]
fn a_shared_session_announces_changes_and_who_made_them() {
    use std::sync::Mutex;
    let shared = crate::SharedSession::new(Session::default());
    let heard = std::sync::Arc::new(Mutex::new(Vec::new()));
    let h = heard.clone();
    shared.on_change(move |c, p| h.lock().unwrap().push((c.by.clone(), c.command.clone(), p.tracks.len())));
    let track = shared.project().tracks[0].id.clone();
    // queries and refusals announce nothing
    shared.execute("project.inspect", &json!({}), "agent").unwrap();
    assert!(shared.execute("clip.split", &json!({"clip": "x", "at": 0}), "agent").is_err());
    assert!(heard.lock().unwrap().is_empty());

    let since = shared.revision();
    let waiter = {
        let shared = shared.clone();
        std::thread::spawn(move || shared.wait_for_change(since, std::time::Duration::from_secs(10)))
    };
    shared.execute("clip.add", &json!({"track": track, "color": [1, 0, 0]}), "agent").unwrap();
    let change = waiter.join().unwrap();
    assert_eq!((change.revision, change.command.as_str(), change.by.as_str()), (since + 1, "clip.add", "agent"));
    shared.execute("edit.undo", &json!({}), "editor").unwrap();
    let heard = heard.lock().unwrap();
    assert_eq!(
        heard.iter().map(|(by, cmd, _)| format!("{by}:{cmd}")).collect::<Vec<_>>(),
        ["agent:clip.add", "editor:edit.undo"]
    );
    // a wait with nothing new times out with the last change
    assert_eq!(shared.wait_for_change(shared.revision(), std::time::Duration::from_millis(10)).command, "edit.undo");
}

#[test]
fn a_mirror_adopts_the_state_it_mirrors_without_history() {
    let origin = crate::SharedSession::new(Session::default());
    let mirror = crate::SharedSession::new(Session::default());
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let h = heard.clone();
    mirror.on_change(move |c, _| h.lock().unwrap().push(format!("{}:{}", c.by, c.command)));
    let track = origin.project().tracks[0].id.clone();
    origin.execute("clip.add", &json!({"track": track, "color": [1, 0, 0]}), "agent").unwrap();
    mirror.execute("track.add", &json!({"kind": "video"}), "editor").unwrap();

    let before = mirror.revision();
    mirror.adopt((*origin.project()).clone(), "clip.add", "agent");
    assert_eq!(mirror.project(), origin.project());
    assert_eq!(mirror.revision(), before + 1);
    assert_eq!(mirror.last_change().by, "agent");
    assert_eq!(*heard.lock().unwrap(), ["editor:track.add", "agent:clip.add"]);
    // the mirror's own history went with its old state
    assert!(mirror.execute("edit.undo", &json!({}), "editor").is_err());
    // ids continue from the adopted project, so the same edit mints the same ids on both
    let a = origin.execute("track.add", &json!({"kind": "text"}), "agent").unwrap();
    let b = mirror.execute("track.add", &json!({"kind": "text"}), "editor").unwrap();
    assert_eq!(a, b);
}

#[test]
fn tracks_are_renamed_hidden_and_muted() {
    let mut s = Session::default();
    let t = s.project().tracks[0].id.clone();
    let out = s.execute("track.set", &json!({"track": t, "hidden": true, "name": "B-roll"})).unwrap();
    assert_eq!(out, json!({"track": t, "name": "B-roll", "hidden": true, "muted": false, "magnetic": false}));
    s.execute("track.set", &json!({"track": t, "muted": true})).unwrap();
    let track = s.project().tracks[0].clone();
    assert!(track.hidden && track.muted && track.name == "B-roll");
    assert!(s.execute("track.set", &json!({"track": "nope", "muted": true})).is_err());
    assert!(s.execute("track.set", &json!({"track": t, "name": " "})).is_err());
    assert!(s.execute("track.set", &json!({"track": t, "muted": "yes"})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.project().tracks[0].muted);
}

#[test]
fn keyframes_move_a_clip_and_stay_put_on_the_timeline() {
    use reelforge_model::Prop;
    let mut s = Session::default();
    let c = add_red(&mut s, 1.0, 4.0);
    s.execute("clip.setKey", &json!({"clip": c, "prop": "x", "at": {"seconds": 1}, "value": -0.5})).unwrap();
    s.execute("clip.setKey", &json!({"clip": c, "prop": "x", "at": {"seconds": 3}, "value": 0.5, "ease": "inOut"}))
        .unwrap();
    let x_at = |s: &Session, secs: f64| {
        let p = s.project();
        let clip = p.clip(&c).unwrap().1;
        clip.value_at(Prop::X, Tick::from_seconds(secs) - clip.start)
    };
    assert_eq!(x_at(&s, 2.0), 0.0);

    // a keyed property is set at a time; without one the command explains itself
    let e = s.execute("clip.set", &json!({"clip": c, "x": 0.2})).unwrap_err().to_string();
    assert!(e.contains("`at`"), "{e}");
    s.execute("clip.set", &json!({"clip": c, "x": 0.25, "at": {"seconds": 2}, "opacity": 0.5})).unwrap();
    assert_eq!(x_at(&s, 2.0), 0.25);
    assert_eq!(s.project().clip(&c).unwrap().1.keys[&Prop::X].len(), 3);
    assert_eq!(s.project().clip(&c).unwrap().1.opacity, 0.5);

    // trimming the start and splitting keep every key at its timeline time
    s.execute("clip.trim", &json!({"clip": c, "start": {"seconds": 1.5}})).unwrap();
    assert_eq!(x_at(&s, 2.0), 0.25);
    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 2.5}})).unwrap();
    let right = r["right"].as_str().unwrap().to_string();
    let p = s.project();
    let rc = p.clip(&right).unwrap().1;
    assert_eq!(rc.value_at(Prop::X, Tick::from_seconds(3.0) - rc.start), 0.5);

    // removing the last key leaves its value
    for at in [1.0, 2.0] {
        s.execute("clip.removeKey", &json!({"clip": c, "prop": "x", "at": {"seconds": at}})).unwrap();
    }
    s.execute("clip.removeKey", &json!({"clip": c, "prop": "x", "at": {"seconds": 3}})).unwrap();
    let p = s.project();
    let lc = p.clip(&c).unwrap().1;
    assert!(!lc.keyed(Prop::X));
    assert_eq!(lc.transform.x, 0.5);
    assert!(s.execute("clip.removeKey", &json!({"clip": c, "prop": "x", "at": 0})).is_err());

    s.execute("clip.clearKeys", &json!({"clip": right, "at": {"seconds": 3}})).unwrap();
    assert!(s.project().clip(&right).unwrap().1.keys.is_empty());
    s.execute("render.compile", &json!({})).unwrap();
}

#[test]
fn animations_are_set_kept_apart_by_a_split_and_compiled() {
    let mut s = Session::default();
    let c = add_red(&mut s, 0.0, 4.0);
    s.execute("clip.setAnimation", &json!({"clip": c, "slot": "in", "kind": "pop"})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": c, "slot": "out", "kind": "slideDown", "duration": {"seconds": 1}}))
        .unwrap();
    assert!(s.execute("clip.setAnimation", &json!({"clip": c, "slot": "in", "kind": "wobble"})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["layers"][0]["transform"]["scale"].is_array());
    assert!(spec["layers"][0]["transform"]["position"][0]["t"].is_number());

    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 2}})).unwrap();
    let right = r["right"].as_str().unwrap().to_string();
    let p = s.project();
    let (l, rt) = (p.clip(&c).unwrap().1, p.clip(&right).unwrap().1);
    assert!(l.anim.enter.is_some() && l.anim.exit.is_none());
    assert!(rt.anim.enter.is_none() && rt.anim.exit.is_some_and(|a| a.duration == Tick::SECOND));

    s.execute("clip.setAnimation", &json!({"clip": c, "slot": "in", "kind": null})).unwrap();
    assert!(s.project().clip(&c).unwrap().1.anim.is_empty());
}

#[test]
fn transitions_need_touching_clips_on_a_video_track() {
    let mut s = Session::default();
    let a = add_red(&mut s, 0.0, 2.0);
    let b = add_red(&mut s, 3.0, 2.0);
    let e = s.execute("clip.setTransition", &json!({"clip": b, "kind": "dissolve"})).unwrap_err().to_string();
    assert!(e.contains("move them together"), "{e}");
    s.execute("clip.move", &json!({"clip": b, "start": {"seconds": 2}})).unwrap();
    s.execute("clip.setTransition", &json!({"clip": b, "kind": "pushLeft", "duration": {"seconds": 0.8}})).unwrap();
    assert!(s.execute("clip.setTransition", &json!({"clip": a, "kind": "zoom"})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["layers"][1]["at"], json!(Tick::from_seconds(1.6).0));
    // splitting the incoming clip keeps the transition on its left half
    let r = s.execute("clip.split", &json!({"clip": b, "at": {"seconds": 3}})).unwrap();
    let p = s.project();
    assert!(p.clip(&b).unwrap().1.transition.is_some());
    assert!(p.clip(r["right"].as_str().unwrap()).unwrap().1.transition.is_none());
    s.execute("clip.setTransition", &json!({"clip": b, "kind": null})).unwrap();
    assert!(s.project().clip(&b).unwrap().1.transition.is_none());
}

#[test]
fn title_templates_restyle_a_title_in_one_step() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let list = s.execute("title.templates", &json!({})).unwrap();
    let ids: Vec<&str> = list["templates"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert!(ids.len() >= 8 && ids.contains(&"candy") && ids.contains(&"classic"), "{ids:?}");

    // added with a template: its colour and look
    let c =
        s.execute("clip.add", &json!({"track": text, "text": "花字", "template": "candy", "align": "left"})).unwrap();
    let id = c["clip"].as_str().unwrap().to_string();
    let look = |s: &Session| match &s.project().clip(&id).unwrap().1.content {
        ClipContent::Text { color, style, .. } => (*color, style.clone()),
        _ => unreachable!(),
    };
    let candy = crate::title_template("candy").unwrap();
    let (color, style) = look(&s);
    assert_eq!(color, candy.color);
    assert_eq!((style.gradient, style.outer.clone()), (candy.style.gradient, candy.style.outer.clone()));
    assert_eq!(style.align, reelforge_model::TitleAlign::Left, "given parameters still apply");

    // switched to another; where it wraps and lines up stays, other parameters override
    s.execute("clip.set", &json!({"clip": id, "template": "label", "radius": 0.5})).unwrap();
    let (color, style) = look(&s);
    let label = crate::title_template("label").unwrap();
    assert_eq!(color, label.color);
    assert_eq!((style.gradient, style.background, style.radius), (None, label.style.background, 0.5));
    assert_eq!(style.align, reelforge_model::TitleAlign::Left);

    // the looks reach the renderer as text/2
    s.execute(
        "clip.set",
        &json!({"clip": id, "shadow": {"color": [0, 0, 0, 1], "offset": [0.1, 0.1], "blur": 0.2}, "gradient": [1, 0, 0, 1]}),
    )
    .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let content = &spec["layers"][0]["content"];
    assert!((content["shadow"]["offset"][1].as_f64().unwrap() - 0.1).abs() < 1e-6);
    assert_eq!(content["background"]["radius"].as_f64(), Some(0.5));
    assert!(s.execute("clip.set", &json!({"clip": id, "template": "nope"})).unwrap_err().to_string().contains("candy"));
}

#[test]
fn captions_come_in_and_go_out_as_subrip() {
    let mut s = Session::default();
    let srt = "1\n00:00:00,500 --> 00:00:02,000\n大家好\n\n2\n00:00:01,800 --> 00:00:03,000\n欢迎来到 reelforge\n";
    let r = s.execute("captions.import", &json!({"srt": srt, "template": "variety"})).unwrap();
    let track = r["track"].as_str().unwrap().to_string();
    assert_eq!(r["clips"].as_array().unwrap().len(), 2);
    let p = s.project();
    let t = p.track(&track).unwrap();
    assert_eq!((t.name.as_str(), t.kind), (crate::CAPTIONS_TRACK, TrackKind::Text));
    // the overlap is cut where the next cue begins; captions sit low on the frame
    assert_eq!(t.clips[0].end(), Tick::from_seconds(1.8));
    assert!((t.clips[0].transform.y - 0.32).abs() < 1e-6);
    match &t.clips[1].content {
        ClipContent::Text { color, .. } => assert_eq!(*color, crate::title_template("variety").unwrap().color),
        _ => unreachable!(),
    }
    // one undo step takes them all away
    let out = s.execute("captions.export", &json!({})).unwrap();
    assert_eq!(out["count"], 2);
    assert!(out["srt"].as_str().unwrap().contains("00:00:00,500 --> 00:00:01,800\n大家好"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().track(&track).is_none());
    // a second import onto the same track adds to it
    s.execute("captions.import", &json!({"srt": srt})).unwrap();
    let again = s
        .execute(
            "captions.import",
            &json!({"srt": "1\n00:00:05,000 --> 00:00:06,000\n再见\n", "track": crate::captions_track_for_tests(&s)}),
        )
        .unwrap();
    assert_eq!(s.project().track(again["track"].as_str().unwrap()).unwrap().clips.len(), 3);
    assert!(
        s.execute("captions.import", &json!({"srt": "1\nnot a time\n"})).unwrap_err().to_string().contains("line 1")
    );
}

#[test]
fn sound_fades_and_keeps_its_pitch_at_another_speed() {
    let mut s = Session::default();
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/a.wav", "duration": {"seconds": 10}, "sha256": "cd".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let audio = s.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id.clone();
    let c = s.execute("clip.add", &json!({"track": audio, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();
    s.execute("clip.set", &json!({"clip": c, "fadeIn": {"seconds": 1}, "fadeOut": {"seconds": 2}})).unwrap();
    s.execute("clip.setSpeed", &json!({"clip": c, "speed": 2})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["audio"][0]["keep_pitch"], json!(true));
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("audio.stretch/1")));
    assert!(spec["audio"][0]["gain_db"][0]["v"].as_f64().unwrap() < -60.0);

    s.execute("clip.setSpeed", &json!({"clip": c, "keepPitch": false})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["audio"][0].get("keep_pitch").is_none());
    assert!(s.execute("clip.setSpeed", &json!({"clip": c})).is_err());

    // a split keeps the fade-in on the left and the fade-out on the right
    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 2}})).unwrap();
    let p = s.project();
    let (l, rt) = (p.clip(&c).unwrap().1, p.clip(r["right"].as_str().unwrap()).unwrap().1);
    assert_eq!((l.fade.fade_in, l.fade.fade_out), (Tick::SECOND, Tick::ZERO));
    assert_eq!((rt.fade.fade_in, rt.fade.fade_out), (Tick::ZERO, Tick::SECOND * 2));
}

fn add_video(s: &mut Session, secs: f64) -> String {
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/v.mp4", "kind": "video", "duration": {"seconds": secs}, "sha256": "ef".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t = video_track(s);
    s.execute("clip.add", &json!({"track": t, "media": m})).unwrap()["clip"].as_str().unwrap().to_string()
}

fn source_at(s: &Session, clip: &str, t: f64) -> Tick {
    let p = s.project();
    let c = p.clip(clip).unwrap().1;
    c.timing().unwrap().source_at(Tick::from_seconds(t) - c.start)
}

#[test]
fn ripple_delete_closes_the_gap_and_magnetic_tracks_stay_packed() {
    let mut s = Session::default();
    let (a, b, c) = (add_red(&mut s, 0.0, 2.0), add_red(&mut s, 2.0, 3.0), add_red(&mut s, 6.0, 1.0));
    s.execute("clip.remove", &json!({"clips": b, "ripple": true})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.start, Tick::SECOND * 3);
    s.execute("edit.undo", &json!({})).unwrap();

    let t = video_track(&s);
    s.execute("track.set", &json!({"track": t, "magnetic": true})).unwrap();
    let starts = |s: &Session| [&a, &b, &c].map(|id| s.project().clip(id).unwrap().1.start.seconds());
    // the gap before c closes as soon as the track is magnetic
    assert_eq!(starts(&s), [0.0, 2.0, 5.0]);
    // a plain delete closes up too, and a move reorders
    s.execute("clip.remove", &json!({"clips": b})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.start, Tick::SECOND * 2);
    s.execute("clip.move", &json!({"clip": a, "start": {"seconds": 2.5}})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.start, Tick::ZERO);
    assert_eq!(s.project().clip(&a).unwrap().1.start, Tick::SECOND);
}

#[test]
fn reverse_plays_the_same_span_backwards_and_splits_in_order() {
    let mut s = Session::default();
    let c = add_video(&mut s, 4.0);
    s.execute("clip.setReverse", &json!({"clip": c, "reversed": true})).unwrap();
    assert_eq!(source_at(&s, &c, 0.0), Tick::SECOND * 4 - Tick(1));
    assert_eq!(source_at(&s, &c, 3.0), Tick::SECOND - Tick(1));
    let r = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 1}})).unwrap();
    let right = r["right"].as_str().unwrap().to_string();
    // each half shows what the whole showed there
    assert_eq!(source_at(&s, &c, 0.5), Tick::from_seconds(3.5) - Tick(1));
    assert_eq!(source_at(&s, &right, 3.0), Tick::SECOND - Tick(1));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("time.reverse/1")));
    assert_eq!(spec["layers"][0]["content"]["time"]["speed"], json!(-1.0));
}

#[test]
fn speed_curves_reshape_the_speed_and_keep_the_span() {
    let mut s = Session::default();
    let c = add_video(&mut s, 4.0);
    let out = s.execute("clip.setSpeedCurve", &json!({"clip": c, "curve": "bullet"})).unwrap();
    assert_eq!(out["preset"], json!("bullet"));
    assert_eq!(s.project().clip(&c).unwrap().1.duration, Tick::SECOND * 4);
    assert!((source_at(&s, &c, 4.0) - Tick::SECOND * 4).0.abs() < 1000);
    // the slow middle covers less source than the fast start
    let first = source_at(&s, &c, 1.0);
    let middle = source_at(&s, &c, 2.5) - source_at(&s, &c, 1.5);
    assert!(first > middle * 3);
    // a split keeps the motion
    let before = source_at(&s, &c, 3.0);
    let right = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 2}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    assert!((source_at(&s, &right, 3.0) - before).0.abs() < 1000);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("time.ramp/1")));
    s.execute("clip.setSpeedCurve", &json!({"clip": c, "curve": [[0, 1], [1, 3]]})).unwrap();
    assert!(s.execute("clip.setSpeedCurve", &json!({"clip": c, "curve": [[0.5, 1], [0.2, 3]]})).is_err());
    assert!(s.execute("clip.setSpeedCurve", &json!({"clip": c, "curve": "wobble"})).is_err());
    s.execute("clip.setSpeedCurve", &json!({"clip": c, "curve": null})).unwrap();
}

#[test]
fn freeze_holds_a_frame_and_pushes_the_rest_along() {
    let mut s = Session::default();
    let c = add_video(&mut s, 4.0);
    let after = add_red(&mut s, 5.0, 1.0);
    s.execute("clip.set", &json!({"clip": c, "scale": 2.0, "at": {"seconds": 0}})).unwrap();
    let out = s.execute("clip.freeze", &json!({"clip": c, "at": {"seconds": 1.5}})).unwrap();
    let still = out["clip"].as_str().unwrap().to_string();
    let right = out["right"].as_str().unwrap().to_string();
    let p = s.project();
    let st = p.clip(&still).unwrap().1;
    assert_eq!((st.start, st.duration), (Tick::from_seconds(1.5), Tick::SECOND * 3));
    assert_eq!(st.transform.scale, 2.0);
    let ClipContent::Media { source_in, speed, .. } = st.content else { panic!() };
    assert_eq!((source_in, speed), (Tick::from_seconds(1.5), 0.0));
    assert_eq!(p.clip(&right).unwrap().1.start, Tick::from_seconds(4.5));
    assert_eq!(source_at(&s, &right, 4.5), Tick::from_seconds(1.5));
    assert_eq!(p.clip(&after).unwrap().1.start, Tick::SECOND * 8);
    // a still is silent, cannot change speed and cannot freeze again
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["audio"].as_array().unwrap().iter().all(|a| a["id"] != json!(still)));
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("time.hold/1")));
    assert!(s.execute("clip.setSpeed", &json!({"clip": still, "speed": 2})).is_err());
    assert!(s.execute("clip.setReverse", &json!({"clip": still, "reversed": true})).is_err());
    assert!(s.execute("clip.freeze", &json!({"clip": still, "at": {"seconds": 2}})).is_err());
    assert!(s.execute("clip.freeze", &json!({"clip": after, "at": {"seconds": 8.5}})).is_err());
}

#[test]
fn colour_is_a_filter_a_lookup_table_and_adjustments() {
    let mut s = Session::default();
    let c = add_red(&mut s, 0.0, 2.0);
    let lut =
        s.execute("media.import", &json!({"path": "/nowhere/teal.cube", "sha256": "cd".repeat(32)})).unwrap()["media"]
            ["id"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(s.project().media[&lut].kind, reelforge_model::MediaKind::Lut);
    s.execute("clip.setColor", &json!({"clip": c, "filter": "vintage", "filterStrength": 0.5})).unwrap();
    s.execute("clip.setColor", &json!({"clip": c, "lut": lut, "lutStrength": 0.7, "exposure": 0.5, "vignette": 0.3}))
        .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let fx = spec["layers"][0]["fx"].as_array().unwrap();
    assert_eq!(fx.len(), 3);
    assert_eq!(fx[0]["id"], json!("color.grade@1"));
    assert_eq!(fx[0]["params"]["fade"], json!(0.3f32));
    assert_eq!(fx[1]["id"], json!("color.lut@1"));
    assert_eq!(fx[1]["asset"], json!(lut));
    assert_eq!(fx[1]["params"]["amount"], json!(0.7f32));
    assert_eq!(fx[2]["params"], json!({"exposure": 0.5, "vignette": 0.3f32}));
    let features = spec["engine"]["features"].as_array().unwrap();
    assert!(features.contains(&json!("color.grade@1")) && features.contains(&json!("color.lut@1")));
    // out of range, an unknown look, a lut that is not one, a clip with no picture
    assert!(s.execute("clip.setColor", &json!({"clip": c, "exposure": 9})).is_err());
    assert!(s.execute("clip.setColor", &json!({"clip": c, "filter": "sepia"})).is_err());
    assert!(s.execute("clip.setColor", &json!({"clip": c, "lut": c})).is_err());
    // a lookup table cannot be placed as a clip
    let t = video_track(&s);
    assert!(s.execute("clip.add", &json!({"track": t, "media": lut, "start": {"seconds": 5}})).is_err());
    s.execute("clip.setColor", &json!({"clip": c, "reset": true})).unwrap();
    assert!(s.project().clip(&c).unwrap().1.color.is_empty());
}

#[test]
fn crops_flips_blends_and_masks_reach_the_spec() {
    let mut s = Session::default();
    let v = add_video(&mut s, 4.0);
    let solid = add_red(&mut s, 5.0, 2.0);
    s.execute("clip.setCrop", &json!({"clip": v, "left": 0.1, "bottom": 0.2, "flipX": true})).unwrap();
    s.execute("clip.setBlend", &json!({"clip": v, "mode": "softLight"})).unwrap();
    s.execute("clip.setMask", &json!({"clip": v, "shape": "circle", "feather": 0.1})).unwrap();
    s.execute("clip.setMask", &json!({"clip": v, "x": 0.3, "invert": true})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = spec["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(v)).unwrap();
    assert_eq!(layer["crop"]["left"], json!(0.1f32));
    assert_eq!(layer["crop"]["bottom"], json!(0.2f32));
    assert_eq!(layer["flip_x"], json!(true));
    assert_eq!(layer["blend"], json!("soft_light"));
    let m = &layer["shape_mask"];
    assert_eq!(
        (m["shape"].clone(), m["center"][0].clone(), m["invert"].clone()),
        (json!("circle"), json!(0.3f32), json!(true))
    );
    let features = spec["engine"]["features"].as_array().unwrap();
    for f in ["crop/1", "flip/1", "mask.shape/1", "blend/2"] {
        assert!(features.contains(&json!(f)), "{f}");
    }
    // a split keeps the look on both halves
    let right = s.execute("clip.split", &json!({"clip": v, "at": {"seconds": 2}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.project().clip(&right).unwrap().1.look, s.project().clip(&v).unwrap().1.look);
    // solids blend and mask but are not cropped; crops must leave something; masks need a shape first
    assert!(s.execute("clip.setCrop", &json!({"clip": solid, "left": 0.1})).is_err());
    s.execute("clip.setBlend", &json!({"clip": solid, "mode": "multiply"})).unwrap();
    assert!(s.execute("clip.setMask", &json!({"clip": solid, "x": 0.2})).is_err());
    assert!(s.execute("clip.setCrop", &json!({"clip": v, "left": 0.6, "right": 0.5})).is_err());
    assert!(s.execute("clip.setBlend", &json!({"clip": v, "mode": "glow"})).is_err());
    assert!(s.execute("clip.setMask", &json!({"clip": v, "feather": -1})).is_err());
    s.execute("clip.setMask", &json!({"clip": v, "shape": null})).unwrap();
    assert!(s.project().clip(&v).unwrap().1.look.mask.is_none());
}

#[test]
fn export_settings_shape_the_compiled_output() {
    let mut s = Session::default();
    add_red(&mut s, 0.0, 2.0);
    let r = s.execute("project.setExport", &json!({"preset": "standard"})).unwrap();
    assert_eq!(
        (r["preset"].clone(), r["width"].clone(), r["height"].clone()),
        (json!("standard"), json!(1080), json!(1920))
    );
    let r = s.execute("project.setExport", &json!({"resolution": "720p", "fps": 60, "codec": "hevc"})).unwrap();
    assert_eq!(r["preset"], json!(null));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let o = &spec["output"];
    assert_eq!(o["fps"], json!({"num": 60, "den": 1}));
    assert_eq!(o["video"]["codec"], json!("hevc"));
    assert_eq!(o["video"]["size"], json!([720, 1280]));
    assert!(o["video"]["bitrate_kbps"].as_u64().unwrap() > 1000);
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("encode.size/1")));
    // the settings travel with the project and undo like any edit
    assert_eq!(s.project().export.resolution, reelforge_model::Resolution::P720);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().export.preset(), Some("standard"));
    assert!(s.execute("project.setExport", &json!({"fps": 29})).is_err());
    assert!(s.execute("project.setExport", &json!({"codec": "vp8"})).is_err());
    assert!(s.execute("project.setExport", &json!({"preset": "cinema"})).is_err());
}

#[test]
fn library_sounds_go_on_an_audio_track_and_into_the_mix() {
    let mut s = Session::default();
    let list = s.execute("sound.library", &json!({})).unwrap();
    assert!(list["sounds"].as_array().unwrap().len() >= 12);
    let a = s.execute("sound.add", &json!({"sound": "whoosh", "at": {"seconds": 1}})).unwrap();
    let b = s.execute("sound.add", &json!({"sound": "whoosh", "name": "嗖"})).unwrap();
    // one media for both, the second after the first
    assert_eq!(a["media"], b["media"]);
    assert_eq!(a["track"], b["track"]);
    let m = &s.project().media[a["media"].as_str().unwrap()];
    assert_eq!(m.uri, "library:whoosh");
    assert!(m.sha256.is_some());
    let end = Tick::SECOND + m.duration.unwrap();
    assert_eq!(s.project().clip(b["clip"].as_str().unwrap()).unwrap().1.start, end);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["audio"].as_array().unwrap().len(), 2);
    assert!(s.execute("sound.add", &json!({"sound": "kazoo"})).is_err());
    let v = video_track(&s);
    assert!(s.execute("sound.add", &json!({"sound": "pop", "track": v})).is_err());
}

#[test]
fn a_sound_that_does_not_fit_gets_its_own_track() {
    let mut s = Session::default();
    let a = s.execute("sound.add", &json!({"sound": "boom", "at": {"seconds": 0}})).unwrap();
    let b = s.execute("sound.add", &json!({"sound": "pop", "at": {"seconds": 0.5}})).unwrap();
    assert_ne!(a["track"], b["track"]);
    // naming a full track refuses
    let t = a["track"].as_str().unwrap();
    assert!(s.execute("sound.add", &json!({"sound": "pop", "at": {"seconds": 0.5}, "track": t})).is_err());
}
