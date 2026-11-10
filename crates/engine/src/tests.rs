use reelforge_model::{ClipContent, Fx, FxKind, MediaKind, Prop, TrackKind};
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
    assert_eq!(
        out,
        json!({"track": t, "name": "B-roll", "hidden": true, "muted": false, "magnetic": false, "locked": false})
    );
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
fn a_keyframe_curve_is_named_or_drawn_and_followed_frame_by_frame() {
    use reelforge_model::{Ease, Prop};
    let mut s = Session::default();
    let c = add_red(&mut s, 1.0, 4.0);
    for (prop, to) in [("x", 0.5), ("scale", 2.0), ("opacity", 1.0)] {
        s.execute("clip.setKey", &json!({"clip": c, "prop": prop, "at": {"seconds": 1}, "value": to / 2.0})).unwrap();
        s.execute("clip.setKey", &json!({"clip": c, "prop": prop, "at": {"seconds": 3}, "value": to})).unwrap();
    }
    let at = |s: &Session, prop: Prop, secs: f64| {
        let p = s.project();
        let clip = p.clip(&c).unwrap().1;
        clip.value_at(prop, Tick::from_seconds(secs) - clip.start)
    };
    assert!((at(&s, Prop::X, 2.0) - 0.375).abs() < 1e-6);

    // one command shapes the way on from every key at that time, within half a frame
    let r = s.execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 1.01}, "ease": "in"})).unwrap();
    assert_eq!(r["props"], json!(["x", "scale", "opacity"]));
    assert_eq!(r["ease"], json!({"bezier": [0.42f32, 0.0f32, 1.0f32, 1.0f32]}));
    assert!(at(&s, Prop::X, 1.5) < 0.29 && at(&s, Prop::Scale, 1.5) < 1.15, "slow away");
    assert!((at(&s, Prop::X, 3.0) - 0.5).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((at(&s, Prop::X, 2.0) - 0.375).abs() < 1e-6, "one step");

    // or only some properties, by a curve drawn by hand
    let drawn = json!([0.2, 1.8, 0.6, 1.0]);
    let r = s
        .execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 1}, "ease": drawn, "props": ["x", "opacity"]}))
        .unwrap();
    assert_eq!(r["props"], json!(["x", "opacity"]));
    let keys = s.project().clip(&c).unwrap().1.keys.clone();
    assert_eq!(keys[&Prop::X][0].ease, Ease::Bezier([0.2, 1.8, 0.6, 1.0]));
    assert_eq!(keys[&Prop::Scale][0].ease, Ease::Linear);
    // it passes the next key and comes back; what cannot be more than whole stays whole
    assert!(at(&s, Prop::X, 1.8) > 0.5);
    assert_eq!(at(&s, Prop::Opacity, 1.8), 1.0);

    // the renderer is handed the curve frame by frame, and nothing it does not know
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = &spec["layers"][0];
    let position = layer["transform"]["position"].as_array().unwrap();
    assert!(position.len() > 50, "{}", position.len());
    assert!(position.iter().any(|k| k["v"][0].as_f64().unwrap() > 0.5));
    assert!(!spec.to_string().contains("bezier"));
    let top = layer["opacity"].as_array().unwrap().iter().map(|k| k["v"].as_f64().unwrap()).fold(0.0, f64::max);
    assert_eq!(top, 1.0);

    // setKey takes the same curves, and keeps the one a key had
    s.execute("clip.setKey", &json!({"clip": c, "prop": "scale", "at": {"seconds": 1}, "ease": "overshoot"})).unwrap();
    s.execute("clip.setKey", &json!({"clip": c, "prop": "scale", "at": {"seconds": 1}, "value": 0.5})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.keys[&Prop::Scale][0].ease, Ease::OVERSHOOT);

    // what cannot be a curve is refused
    for bad in [json!("spring"), json!([0.2, 0.0, 1.4, 1.0]), json!([0.2, 5.0, 0.6, 1.0]), json!([0.1, 0.2])] {
        let e = s.execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 1}, "ease": bad})).unwrap_err();
        assert!(e.to_string().contains("`ease`"), "{e}");
    }
    assert!(s.execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 2}, "ease": "in"})).is_err());
    assert!(
        s.execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 1}, "ease": "in", "props": ["y"]})).is_err()
    );
    assert!(
        s.execute("clip.setEase", &json!({"clip": c, "at": {"seconds": 1}, "ease": "in", "props": ["z"]})).is_err()
    );
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
fn a_gif_or_the_sound_alone_is_a_kind_of_export_and_the_cover_is_a_frame() {
    let mut s = Session::default();
    add_red(&mut s, 0.0, 4.0);
    s.execute("sound.add", &json!({"sound": "whoosh"})).unwrap();

    // a GIF: pictures only, small and slow by its quality
    let r = s.execute("project.setExport", &json!({"format": "gif", "quality": "low"})).unwrap();
    assert_eq!((r["format"].clone(), r["preset"].clone()), (json!("gif"), json!(null)));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let o = &spec["output"];
    assert_eq!((o["format"].clone(), o["fps"].clone()), (json!("gif"), json!({"num": 10, "den": 1})));
    assert_eq!(o["video"]["size"], json!([242, 430]));
    assert!(o.get("audio").is_none_or(|a| a.is_null()), "{o}");
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("encode.gif/1")));

    // the sound alone: no pictures are encoded
    for f in ["m4a", "wav"] {
        s.execute("project.setExport", &json!({"format": f})).unwrap();
        let spec = s.execute("render.compile", &json!({})).unwrap();
        let o = &spec["output"];
        assert_eq!(o["format"], json!(f));
        assert!(o.get("video").is_none_or(|v| v.is_null()), "{o}");
        assert!(o["audio"].is_object());
        assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("encode.audio/1")));
    }
    // a video again says nothing about its format, as before there were others
    s.execute("project.setExport", &json!({"format": "mp4"})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["output"].get("format").is_none());
    assert!(s.execute("project.setExport", &json!({"format": "webm"})).is_err());

    // the cover is a frame of the timeline
    let fps = s.project().canvas.fps;
    let r = s.execute("project.setCover", &json!({"at": {"seconds": 2.51}})).unwrap();
    let at = fps.snap(Tick::from_seconds(2.51));
    assert_eq!((r["cover"].clone(), s.project().cover), (json!(at), Some(at)));
    assert!(s.execute("project.setCover", &json!({})).is_err(), "`at` is asked for, so that null means none");
    assert!(s.execute("project.setCover", &json!({"at": {"seconds": 9}})).is_err());
    assert!(s.execute("project.setCover", &json!({"at": {"seconds": -1}})).is_err());
    // it moves with what is cut out before it, and lands on the cut when its own frame goes
    s.execute("timeline.cut", &json!({"ranges": [{"from": {"seconds": 0.5}, "to": {"seconds": 1.5}}]})).unwrap();
    assert_eq!(s.project().cover, Some(at - Tick::SECOND));
    s.execute("timeline.cut", &json!({"ranges": [{"from": {"seconds": 1}, "to": {"seconds": 2}}]})).unwrap();
    assert_eq!(s.project().cover, Some(Tick::SECOND));
    // each timeline has its own; a copy starts with the same one
    s.execute("timeline.add", &json!({"copy": true})).unwrap();
    let first = s.project().timelines[0].id.clone();
    assert_eq!(s.project().cover, Some(Tick::SECOND));
    s.execute("project.setCover", &json!({"at": null})).unwrap();
    assert_eq!(s.project().cover, None);
    s.execute("timeline.switch", &json!({"timeline": first})).unwrap();
    assert_eq!(s.project().cover, Some(Tick::SECOND));
    s.execute("edit.undo", &json!({})).unwrap();
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

#[test]
fn timelines_are_added_copied_switched_renamed_and_removed() {
    let mut s = Session::default();
    let c = add_red(&mut s, 0.0, 4.0);
    let list = s.execute("timeline.list", &json!({})).unwrap();
    assert_eq!(list["timelines"].as_array().unwrap().len(), 1);
    assert!(s.execute("timeline.remove", &json!({})).is_err());

    // a copy has the same clips under new ids and becomes the current timeline
    let copy = s.execute("timeline.add", &json!({"copy": true})).unwrap()["timeline"].as_str().unwrap().to_string();
    let p = s.project();
    assert_eq!(p.timeline, copy);
    assert_eq!(p.timelines.len(), 2);
    assert_eq!(p.timelines[1].name, "Timeline 1 copy");
    assert_eq!(p.duration(), Tick::SECOND * 4);
    assert!(p.clip(&c).is_none());
    let first = p.timelines[0].id.clone();

    // edits land on the current timeline only
    add_red(&mut s, 4.0, 2.0);
    assert_eq!(s.project().duration(), Tick::SECOND * 6);
    let empty =
        s.execute("timeline.add", &json!({"name": "B-roll"})).unwrap()["timeline"].as_str().unwrap().to_string();
    assert_eq!(s.project().duration(), Tick::ZERO);
    assert_eq!(s.project().timelines[2].name, "B-roll");

    s.execute("timeline.switch", &json!({"timeline": first})).unwrap();
    assert!(s.project().clip(&c).is_some());
    assert_eq!(s.project().duration(), Tick::SECOND * 4);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec.to_string().contains("1, 0, 0") || spec.to_string().contains("solid"));

    s.execute("timeline.rename", &json!({"timeline": empty, "name": "Spare"})).unwrap();
    s.execute("timeline.remove", &json!({"timeline": empty})).unwrap();
    s.execute("timeline.remove", &json!({})).unwrap();
    let p = s.project();
    assert_eq!(p.timeline, copy, "removing the current one moves to its neighbour");
    assert_eq!(p.timelines.len(), 1);
    assert_eq!(p.duration(), Tick::SECOND * 6);
    assert!(s.execute("timeline.switch", &json!({"timeline": "s999"})).is_err());

    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().clip(&c).is_some());
}

#[test]
fn a_capcut_draft_becomes_a_timeline() {
    let draft = include_str!("fixtures/capcut_draft.json");
    // into an untouched project: its canvas and its only timeline
    let mut s = Session::default();
    let r = s.execute("capcut.import", &json!({"draft": draft, "folder": "/drafts/trip"})).unwrap();
    assert_eq!(r["clips"], 4);
    assert_eq!(r["skipped"]["sticker"], 1);
    assert_eq!(r["skipped"]["stickers"], 1);
    let p = s.project();
    assert_eq!((p.canvas.width, p.canvas.height), (720, 1280));
    assert_eq!(p.timelines.len(), 1);
    assert_eq!(p.timelines[0].name, "旅行 vlog");
    assert_eq!(p.duration(), Tick::SECOND * 9);
    let beach = p.media.values().find(|m| m.name == "beach.mp4").unwrap();
    assert_eq!(beach.uri, "/drafts/trip/assets/beach.mp4");
    assert_eq!(beach.size, Some([1080, 1920]));

    let kinds: Vec<TrackKind> = p.tracks.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, [TrackKind::Video, TrackKind::Text, TrackKind::Audio]);
    let v = &p.tracks[0].clips;
    match &v[0].content {
        ClipContent::Media { source_in, speed, .. } => {
            assert_eq!(*source_in, Tick::SECOND * 2);
            assert_eq!(*speed, 2.0);
        }
        c => panic!("{c:?}"),
    }
    assert!(v[0].look.flip_x);
    assert!((v[0].volume_db + 6.02).abs() < 0.01);
    assert_eq!(v[1].transition.unwrap().duration, Tick::from_seconds(0.5));
    assert_eq!((v[1].transform.x, v[1].transform.y, v[1].transform.rotation), (0.25, -0.125, 15.0));
    assert_eq!(v[1].opacity, 0.8);
    match &p.tracks[1].clips[0].content {
        ClipContent::Text { text, color, .. } => {
            assert_eq!(text, "海边的一天");
            assert_eq!(*color, [1.0, 0.8, 0.0, 1.0]);
        }
        c => panic!("{c:?}"),
    }
    let music = &p.tracks[2].clips[0];
    assert_eq!((music.fade.fade_in, music.fade.fade_out), (Tick::SECOND, Tick::SECOND * 2));
    // offline files preview as a slate, cannot export, and relink when the file arrives
    s.execute("render.compile", &json!({"encode": false})).unwrap();
    assert!(s.execute("render.compile", &json!({})).is_err());
    assert_eq!(r["offline"].as_array().unwrap().len(), 3);
    let sha = "ab".repeat(32);
    for (name, kind) in [("beach.mp4", "video"), ("sunset.jpg", "image"), ("song.mp3", "audio")] {
        let r = s.execute("media.import", &json!({"path": name, "kind": kind, "sha256": sha})).unwrap();
        assert_eq!(r["relinked"], true, "{name}");
    }
    s.execute("render.compile", &json!({})).unwrap();
    let p = s.project();

    // a second import goes beside the first, reusing its files
    let media = p.media.len();
    s.execute("capcut.import", &json!({"draft": draft, "folder": "/drafts/trip", "name": "B"})).unwrap();
    let p = s.project();
    assert_eq!(p.timelines.len(), 2);
    assert_eq!(p.media.len(), media);
    assert!(s.execute("capcut.import", &json!({"draft": "not json"})).is_err());
    assert!(s.execute("capcut.import", &json!({"draft": "{}"})).is_err());
}

#[test]
fn media_leaves_the_project_only_when_no_clip_uses_it() {
    let mut s = Session::default();
    let m = s
        .execute("media.import", &json!({"path": "missing/clip.mp4", "kind": "video", "duration": {"seconds": 4}}))
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.execute("clip.add", &json!({"track": video_track(&s), "media": m})).unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(media_uses(&s.project(), &m), 1);
    // a clip parked on another timeline still counts
    s.execute("timeline.add", &json!({})).unwrap();
    assert!(s.execute("media.remove", &json!({"media": m})).is_err());
    s.execute("timeline.switch", &json!({"timeline": s.project().timelines[0].id})).unwrap();
    s.execute("clip.remove", &json!({"clips": c})).unwrap();
    assert_eq!(media_uses(&s.project(), &m), 0);
    s.execute("media.remove", &json!({"media": m})).unwrap();
    assert!(s.project().media.is_empty());
    assert!(s.execute("media.remove", &json!({"media": m})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().media.contains_key(&m));
}

#[test]
fn clips_copy_paste_duplicate_and_shift_together() {
    let mut s = Session::default();
    let a = add_red(&mut s, 0.0, 1.0);
    let b = add_red(&mut s, 2.0, 1.0);
    let track = video_track(&s);
    let tracks_before = s.project().tracks.len();

    // copying changes nothing; pasting where there is room keeps the clips on their track
    let rev = s.revision();
    let board = s.execute("clip.copy", &json!({"clips": [a, b]})).unwrap();
    assert_eq!(s.revision(), rev);
    let pasted = s.execute("clip.paste", &json!({"clipboard": board, "at": {"seconds": 4}})).unwrap();
    let placed = pasted["clips"].as_array().unwrap();
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0]["track"], track.as_str());
    let starts: Vec<f64> =
        placed.iter().map(|c| s.project().clip(c["clip"].as_str().unwrap()).unwrap().1.start.seconds()).collect();
    assert_eq!(starts, [4.0, 6.0], "the same distance apart");
    assert_ne!(placed[0]["clip"], a.as_str(), "new ids");

    // where they would overlap, they go on another track of the kind
    s.execute("clip.paste", &json!({"clipboard": board, "at": 0})).unwrap();
    assert_eq!(s.project().tracks.len(), tracks_before + 1);
    assert!(s.project().check().is_ok());

    // a copy lands right after the last of the clips
    let d = s.execute("clip.duplicate", &json!({"clips": b})).unwrap();
    let copy = d["clips"][0]["clip"].as_str().unwrap().to_string();
    let end_of_b = s.project().clip(&b).unwrap().1.end();
    assert_eq!(s.project().clip(&copy).unwrap().1.start, end_of_b);

    // several clips move together, and never before the start
    s.execute("clip.shift", &json!({"clips": [b, copy], "by": {"seconds": -0.5}})).unwrap();
    assert_eq!(s.project().clip(&b).unwrap().1.start.seconds(), 1.5);
    assert_eq!(s.project().clip(&copy).unwrap().1.start.seconds(), 2.5);
    assert!(s.execute("clip.shift", &json!({"clips": [a], "by": {"seconds": -1}})).is_err());
    assert!(s.execute("clip.paste", &json!({"clipboard": {"clips": []}, "at": 0})).is_err());
}

#[test]
fn a_canvas_blur_puts_a_blurred_copy_behind_the_clip() {
    let mut s = Session::default();
    let media = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/a.mp4", "duration": {"seconds": 4}, "sha256": "cd".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let track = video_track(&s);
    let a =
        s.execute("clip.add", &json!({"track": track, "media": media})).unwrap()["clip"].as_str().unwrap().to_string();
    let b =
        s.execute("clip.add", &json!({"track": track, "media": media})).unwrap()["clip"].as_str().unwrap().to_string();
    s.execute("clip.setBackdrop", &json!({"clip": a, "blur": 0.5, "all": true})).unwrap();
    assert_eq!(s.project().clip(&b).unwrap().1.look.backdrop, Some(0.5));
    let spec = s.execute("render.compile", &json!({"encode": false})).unwrap();
    let layers = spec["layers"].as_array().unwrap();
    let at = |id: &str| layers.iter().position(|l| l["id"] == id).unwrap();
    assert!(at(&format!("{a}~backdrop")) < at(&a), "behind the clip");
    let back = &layers[at(&format!("{a}~backdrop"))];
    assert_eq!(back["content"]["fit"], "cover");
    assert!(back["fx"].to_string().contains("blur.gaussian@1"));
    s.execute("clip.setBackdrop", &json!({"clip": a, "blur": null})).unwrap();
    assert_eq!(s.project().clip(&a).unwrap().1.look.backdrop, None);
    assert!(s.execute("clip.setBackdrop", &json!({"clip": a, "blur": 2})).is_err());
}

#[test]
fn locked_tracks_markers_and_track_order() {
    let mut s = Session::default();
    let a = add_red(&mut s, 0.0, 1.0);
    let track = video_track(&s);
    s.execute("track.set", &json!({"track": track, "locked": true})).unwrap();
    assert!(s.execute("clip.move", &json!({"clip": a, "start": {"seconds": 2}})).is_err(), "locked");
    assert!(s.execute("clip.remove", &json!({"clips": a})).is_err());
    s.execute("track.set", &json!({"track": track, "locked": false})).unwrap();
    s.execute("clip.move", &json!({"clip": a, "start": {"seconds": 2}})).unwrap();

    // markers stay in time order and snap to frames
    let m2 = s.execute("marker.add", &json!({"at": {"seconds": 2}, "note": "drop"})).unwrap()["marker"].clone();
    let m1 = s.execute("marker.add", &json!({"at": {"seconds": 1}})).unwrap()["marker"].clone();
    assert_eq!(s.project().markers.iter().map(|m| json!(m.id)).collect::<Vec<_>>(), [m1.clone(), m2.clone()]);
    s.execute("marker.set", &json!({"marker": m1, "at": {"seconds": 3}})).unwrap();
    assert_eq!(s.project().markers[1].id, m1.as_str().unwrap());
    s.execute("marker.remove", &json!({"marker": m2})).unwrap();
    assert_eq!(s.project().markers.len(), 1);

    // a second video track moves below the first; the last of a kind stays
    let t2 = s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string();
    s.execute("track.move", &json!({"track": t2, "index": 0})).unwrap();
    let videos: Vec<String> =
        s.project().tracks.iter().filter(|t| t.kind == TrackKind::Video).map(|t| t.id.clone()).collect();
    assert_eq!(videos, [t2.clone(), track.clone()]);
    s.execute("track.remove", &json!({"track": t2})).unwrap();
    assert!(s.execute("track.remove", &json!({"track": track})).is_err());
}

#[test]
fn roll_slip_and_detach_audio() {
    let mut s = Session::default();
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/a.mp4", "duration": {"seconds": 10}, "sha256": "cd".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t = video_track(&s);
    let a = s
        .execute("clip.add", &json!({"track": t, "media": m, "sourceIn": {"seconds": 2}, "duration": {"seconds": 3}}))
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let r = s.execute("clip.split", &json!({"clip": a, "at": {"seconds": 1}})).unwrap();
    let b = r["right"].as_str().unwrap().to_string();

    // roll: the cut moves, the pair keeps its length
    s.execute("clip.roll", &json!({"clip": a, "by": {"seconds": 0.5}})).unwrap();
    let p = s.project();
    assert_eq!(p.clip(&a).unwrap().1.duration, Tick::from_seconds(1.5));
    let right = p.clip(&b).unwrap().1;
    assert_eq!((right.start, right.duration), (Tick::from_seconds(1.5), Tick::from_seconds(1.5)));
    assert!(s.execute("clip.roll", &json!({"clip": b, "by": 1})).is_err(), "nothing follows b");

    // slip: same place on the timeline, other source frames
    s.execute("clip.slip", &json!({"clip": a, "by": {"seconds": -1}})).unwrap();
    let p = s.project();
    let ClipContent::Media { source_in, .. } = &p.clip(&a).unwrap().1.content else { panic!() };
    assert_eq!(*source_in, Tick::SECOND);
    assert!(s.execute("clip.slip", &json!({"clip": a, "by": {"seconds": -2}})).is_err(), "before the start");
    assert!(s.execute("clip.slip", &json!({"clip": a, "by": {"seconds": 9}})).is_err(), "past the end");

    // detach: the sound moves to an audio track, the video goes quiet
    let r = s.execute("clip.detachAudio", &json!({"clip": a})).unwrap();
    let audio = r["audio"].as_str().unwrap();
    let p = s.project();
    let (ti, sound) = p.clip(audio).unwrap();
    assert_eq!(p.tracks[ti].kind, TrackKind::Audio);
    assert_eq!((sound.start, sound.duration), (Tick::ZERO, Tick::from_seconds(1.5)));
    assert!(matches!(p.clip(&a).unwrap().1.content, ClipContent::Media { silent: true, .. }));
    assert!(s.execute("clip.detachAudio", &json!({"clip": a})).is_err(), "already detached");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(s.project().clip(&a).unwrap().1.content, ClipContent::Media { silent: false, .. }));
}

#[test]
fn titles_type_themselves_in_and_box_each_line() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let id = s
        .execute("clip.add", &json!({"track": text, "text": "第一行\n二", "duration": {"seconds": 2}, "background": [0, 0, 0, 0.5], "lineBoxes": true}))
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute(
        "clip.setAnimation",
        &json!({"clip": id, "slot": "in", "kind": "typewriter", "duration": {"seconds": 1}}),
    )
    .unwrap();
    s.execute(
        "clip.setAnimation",
        &json!({"clip": id, "slot": "out", "kind": "typewriter", "duration": {"seconds": 0.5}}),
    )
    .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let content = &spec["layers"][0]["content"];
    assert_eq!(content["background"]["lines"], json!(true));
    // typed in over the first second, erased over the last half
    let keys: Vec<(i64, f64)> = content["reveal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["t"].as_i64().unwrap(), k["v"].as_f64().unwrap()))
        .collect();
    let at = |secs: f64| Tick::from_seconds(secs).0;
    assert_eq!(keys, [(0, 0.0), (at(1.0), 1.0), (at(1.5), 1.0), (at(2.0), 0.0)]);
    // the opacity is left alone
    assert_eq!(spec["layers"][0]["opacity"], json!(1.0));

    // pictures do not type
    let video = video_track(&s);
    let pic = s.execute("clip.add", &json!({"track": video, "color": [1, 0, 0]})).unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let err = s.execute("clip.setAnimation", &json!({"clip": pic, "slot": "in", "kind": "typewriter"})).unwrap_err();
    assert!(err.to_string().contains("titles"), "{err}");
}

#[test]
fn clips_loop_and_titles_move_a_character_at_a_time() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let id = s.execute("clip.add", &json!({"track": text, "text": "逐字 动画", "duration": {"seconds": 2}})).unwrap()
        ["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let set = |s: &mut Session, clip: &str, slot: &str, kind: Value, secs: Option<f64>| {
        let mut v = json!({"clip": clip, "slot": slot, "kind": kind});
        if let Some(secs) = secs {
            v["duration"] = json!({"seconds": secs});
        }
        s.execute("clip.setAnimation", &v)
    };
    set(&mut s, &id, "in", json!("charRise"), Some(1.0)).unwrap();
    set(&mut s, &id, "out", json!("charPop"), None).unwrap();
    let out = set(&mut s, &id, "loop", json!("wave"), None).unwrap();
    assert_eq!(out["anim"]["loop"], json!({"kind": "wave", "period": Tick::SECOND}));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("text/5")));
    let chars = spec["layers"][0]["content"]["chars"].as_array().unwrap().clone();
    assert_eq!(chars.len(), 3);
    assert_eq!(
        (&chars[0]["curve"], &chars[1]["curve"], &chars[2]["curve"]),
        (&json!("settle"), &json!("spring"), &json!("wave"))
    );
    assert_eq!(chars[0]["pose"]["opacity"], json!(0.0));
    assert_eq!(chars[1]["reverse"], json!(true));
    // two rounds of the wave in two seconds
    assert_eq!(chars[2]["progress"][1], json!({"t": Tick::from_seconds(2.0), "v": 2.0, "ease": "linear"}));
    // the title as a whole stays put and whole
    assert_eq!(spec["layers"][0]["opacity"], json!(1.0));
    assert_eq!(spec["layers"][0]["transform"]["scale"], json!(1.0));

    // a loop of the whole clip: its round is kept when it is set again, and starts over for another kind
    set(&mut s, &id, "loop", json!("swing"), Some(0.5)).unwrap();
    let out = set(&mut s, &id, "loop", json!("swing"), None).unwrap();
    assert_eq!(out["anim"]["loop"]["period"], json!(Tick::from_seconds(0.5)));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["layers"][0]["content"]["chars"].as_array().unwrap().len(), 2);
    let turned = spec["layers"][0]["transform"]["rotation"].as_array().unwrap().clone();
    assert!(turned.len() > 50, "sampled at every frame: {}", turned.len());
    let far = turned.iter().map(|k| k["v"].as_f64().unwrap()).fold(0.0f64, |m, v| m.max(v.abs()));
    assert!((far - 8.0).abs() < 0.2, "{far}");
    let out = set(&mut s, &id, "loop", json!("breathe"), None).unwrap();
    assert_eq!(out["anim"]["loop"]["period"], json!(Tick::from_seconds(2.0)));
    // a split leaves the loop on both halves
    let right = s.execute("clip.split", &json!({"clip": id, "at": {"seconds": 1}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    let p = s.project();
    let (l, r) = (p.clip(&id).unwrap().1, p.clip(&right).unwrap().1);
    assert!(l.anim.repeat.is_some() && r.anim.repeat == l.anim.repeat);
    assert!(l.anim.exit.is_none() && r.anim.enter.is_none());
    let out = set(&mut s, &id, "loop", Value::Null, None).unwrap();
    assert!(out["anim"].get("loop").is_none());
    s.execute("edit.undo", &json!({})).unwrap();

    // what is refused: a loop's kind in another slot and the other way round, a blur of a round,
    // and characters on a picture
    assert!(set(&mut s, &id, "in", json!("wave"), None).is_err());
    assert!(set(&mut s, &id, "loop", json!("fade"), None).is_err());
    assert!(set(&mut s, &id, "loop", json!("blink"), Some(0.01)).is_err());
    let video = video_track(&s);
    let pic = s.execute("clip.add", &json!({"track": video, "color": [1, 0, 0]})).unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    for (slot, kind) in [("in", "charFade"), ("loop", "jump")] {
        let err = set(&mut s, &pic, slot, json!(kind), None).unwrap_err();
        assert!(err.to_string().contains("titles"), "{err}");
    }
    set(&mut s, &pic, "loop", json!("spin"), None).unwrap();
    assert!(s.project().check().is_ok());
}

#[test]
fn a_new_project_keeps_the_canvas_and_undoes_back() {
    let mut s = Session::default();
    let video = s.project().tracks[0].id.clone();
    s.execute("project.setCanvas", &json!({"width": 720, "height": 1280})).unwrap();
    s.execute("clip.add", &json!({"track": video, "color": [1, 0, 0], "duration": {"seconds": 2}})).unwrap();
    s.execute("marker.add", &json!({"at": {"seconds": 1}})).ok();
    let before = s.project();
    let out = s.execute("project.new", &json!({})).unwrap();
    let p = s.project();
    assert!(p.tracks.iter().all(|t| t.clips.is_empty()) && p.media.is_empty() && p.markers.is_empty());
    assert_eq!((p.canvas.width, p.canvas.height), (720, 1280));
    assert!(p.tracks.iter().all(|t| before.tracks.iter().all(|o| o.id != t.id)), "fresh track ids");
    assert_eq!(out["tracks"].as_array().unwrap().len(), p.tracks.len());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.project(), *before);
}

#[test]
fn captions_are_edited_as_a_list() {
    let mut s = Session::default();
    let srt = "1\n00:00:00,000 --> 00:00:02,000\n大家好欢迎回来\n\n2\n00:00:02,000 --> 00:00:03,000\nhello\n\n3\n00:00:03,000 --> 00:00:04,000\nworld\n";
    let r = s.execute("captions.import", &json!({"srt": srt})).unwrap();
    let ids: Vec<String> = r["clips"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let words = |s: &mut Session| -> Vec<String> {
        s.execute("captions.list", &json!({})).unwrap()["cues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["text"].as_str().unwrap().to_string())
            .collect()
    };

    // cut after three characters: the time is cut in proportion, on a frame
    let cut = s.execute("captions.split", &json!({"clip": ids[0], "at": 3})).unwrap();
    assert_eq!(words(&mut s), ["大家好", "欢迎回来", "hello", "world"]);
    let p = s.project();
    let left = p.clip(&ids[0]).unwrap().1;
    assert_eq!(left.end(), p.canvas.fps.snap(Tick::from_seconds(6.0 / 7.0)));
    assert_eq!(p.clip(cut["right"].as_str().unwrap()).unwrap().1.end(), Tick::from_seconds(2.0));
    assert!(s.execute("captions.split", &json!({"clip": ids[1], "at": 5})).is_err(), "nothing on the right");

    // Latin words join with a space, Chinese without; the merged caption spans them all
    s.execute("captions.merge", &json!({"clips": [ids[2], ids[1]]})).unwrap();
    s.execute("captions.merge", &json!({"clips": [ids[0], cut["right"]]})).unwrap();
    assert_eq!(words(&mut s), ["大家好欢迎回来", "hello world"]);
    let p = s.project();
    assert_eq!(
        (p.clip(&ids[1]).unwrap().1.start, p.clip(&ids[1]).unwrap().1.end()),
        (Tick::from_seconds(2.0), Tick::from_seconds(4.0))
    );
    assert!(p.clip(&ids[2]).is_none());

    // replacing counts every hit and is one undo step; nothing found is none
    let r = s.execute("captions.replace", &json!({"find": "l", "replace": "L"})).unwrap();
    assert_eq!((r["count"].as_u64(), r["clips"].as_array().unwrap().len()), (Some(3), 1));
    assert_eq!(words(&mut s)[1], "heLLo worLd");
    let rev = s.revision();
    assert_eq!(s.execute("captions.replace", &json!({"find": "zzz", "replace": ""})).unwrap()["count"], 0);
    assert_eq!(s.revision(), rev);

    // one caption's look goes to the others
    s.execute("clip.set", &json!({"clip": ids[1], "size": 90, "color": [1, 0, 0], "y": 0.1})).unwrap();
    let r = s.execute("captions.restyle", &json!({"from": ids[1]})).unwrap();
    assert_eq!(r["clips"], json!([ids[0]]));
    let p = s.project();
    let first = p.clip(&ids[0]).unwrap().1;
    assert!((first.transform.y - 0.1).abs() < 1e-6);
    assert!(
        matches!(&first.content, ClipContent::Text { size, color, text, .. } if *size == 90.0 && color[1] == 0.0 && text == "大家好欢迎回来")
    );

    // all of them move together, never before the start
    s.execute("captions.shift", &json!({"by": {"seconds": 0.5}})).unwrap();
    assert_eq!(s.project().clip(&ids[0]).unwrap().1.start, Tick::from_seconds(0.5));
    assert!(s.execute("captions.shift", &json!({"by": {"seconds": -1}})).is_err());

    // a second language goes smaller, under the first, on its own track
    let second = s
        .execute("captions.import", &json!({"srt": "1\n00:00:00,500 --> 00:00:02,000\nHello everyone\n", "line": 2}))
        .unwrap();
    let p = s.project();
    let t = p.track(second["track"].as_str().unwrap()).unwrap();
    assert_eq!(t.name, crate::CAPTIONS_TRACK_2);
    assert!(t.clips[0].transform.y > 0.32);
    assert!(matches!(&t.clips[0].content, ClipContent::Text { size, .. } if *size < 60.0));
    // the first language is still what exports by default
    assert!(s.execute("captions.export", &json!({})).unwrap()["srt"].as_str().unwrap().contains("heLLo"));
}

#[test]
fn crops_masks_and_colour_move_with_keyframes() {
    let mut s = Session::default();
    let v = add_video(&mut s, 4.0);
    // a crop that closes in from the left over two seconds
    s.execute("clip.setKey", &json!({"clip": v, "prop": "cropLeft", "at": 0, "value": 0.0})).unwrap();
    s.execute("clip.setKey", &json!({"clip": v, "prop": "cropLeft", "at": {"seconds": 2}, "value": 0.4})).unwrap();
    // once keyed, the setter needs to know where, and changes the key there
    assert!(s.execute("clip.setCrop", &json!({"clip": v, "left": 0.5})).unwrap_err().to_string().contains("keyframes"));
    s.execute("clip.setCrop", &json!({"clip": v, "left": 0.5, "top": 0.1, "at": {"seconds": 2}})).unwrap();
    // a mask that slides, and a look that fades to grey
    assert!(s.execute("clip.setKey", &json!({"clip": v, "prop": "maskX", "at": 0})).is_err(), "no mask yet");
    s.execute("clip.setMask", &json!({"clip": v, "shape": "circle"})).unwrap();
    s.execute("clip.setKey", &json!({"clip": v, "prop": "maskX", "at": 0, "value": 0.2})).unwrap();
    s.execute("clip.setKey", &json!({"clip": v, "prop": "maskX", "at": {"seconds": 4}, "value": 0.8})).unwrap();
    s.execute("clip.setColor", &json!({"clip": v, "exposure": 0.5})).unwrap();
    s.execute("clip.setKey", &json!({"clip": v, "prop": "saturation", "at": 0})).unwrap();
    s.execute("clip.setColor", &json!({"clip": v, "saturation": -1, "at": {"seconds": 4}})).unwrap();
    assert!(s.execute("clip.setKey", &json!({"clip": v, "prop": "saturation", "at": 0, "value": 3})).is_err());

    let p = s.project();
    let clip = p.clip(&v).unwrap().1;
    assert_eq!(clip.crop_at(Tick::from_seconds(1.0)), [0.25, 0.1, 0.0, 0.0]);
    assert!((clip.mask_at(Tick::from_seconds(2.0)).unwrap().center[0] - 0.5).abs() < 1e-6);
    assert!((clip.adjust_at(Tick::from_seconds(2.0)).saturation + 0.5).abs() < 1e-6);

    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = spec["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(v)).unwrap();
    assert!(layer.get("crop").is_none());
    assert_eq!(layer["crop_keys"]["left"][1]["v"], json!(0.5f32));
    assert_eq!(layer["crop_keys"]["top"], json!(0.1f32));
    assert_eq!(layer["shape_mask_keys"]["center"][1]["v"], json!([0.8f32, 0.5f32]));
    let grade = layer["fx"].as_array().unwrap().iter().find(|f| f["id"] == "color.grade@1").unwrap();
    assert_eq!(grade["params"]["exposure"], json!(0.5f32));
    assert_eq!(grade["params"]["saturation"][1]["v"], json!(-1.0f32));
    let features = spec["engine"]["features"].as_array().unwrap();
    for f in ["crop/2", "mask.shape/2"] {
        assert!(features.contains(&json!(f)), "{f}");
    }
    // taking the mask away takes its keys; resetting the colour takes its keys
    s.execute("clip.setMask", &json!({"clip": v, "shape": null})).unwrap();
    s.execute("clip.setColor", &json!({"clip": v, "reset": true})).unwrap();
    let p = s.project();
    let keys: Vec<Prop> = p.clip(&v).unwrap().1.keys.keys().copied().collect();
    assert_eq!(keys, [Prop::CropLeft]);
}

#[test]
fn stickers_effects_and_adjustment_clips() {
    let mut s = Session::default();
    let v = video_track(&s);
    assert_eq!(s.execute("sticker.library", &json!({})).unwrap()["stickers"].as_array().unwrap().len(), 20);
    let fx = s.execute("fx.list", &json!({})).unwrap();
    assert!(fx["effects"].as_array().unwrap().iter().any(|e| e["id"] == "shake" && e["group"] == "motion"));

    // a sticker is an image made here: one media however often it is placed
    let a = s.execute("clip.add", &json!({"track": v, "sticker": "heart", "fx": ["pulse"]})).unwrap();
    let b = s.execute("clip.add", &json!({"track": v, "sticker": "heart"})).unwrap();
    let p = s.project();
    assert_eq!(p.media.len(), 1);
    let m = p.media.values().next().unwrap();
    assert_eq!((m.uri.as_str(), m.kind, m.size), ("library:sticker/heart", MediaKind::Image, Some([512, 512])));
    assert_eq!(m.sha256.as_ref().map(String::len), Some(64));
    let (_, clip) = p.clip(a["clip"].as_str().unwrap()).unwrap();
    assert_eq!(clip.transform.scale, 0.35);
    assert_eq!(clip.fx, [Fx::new(FxKind::Pulse)]);
    assert_eq!(p.clip(b["clip"].as_str().unwrap()).unwrap().1.start, clip.end());
    assert!(s.execute("clip.add", &json!({"track": v, "sticker": "kazoo"})).is_err());
    assert!(s.execute("clip.add", &json!({"track": v, "sticker": "star", "text": "x"})).is_err());

    // effects are set as a list, each with its strength and pace
    let id = a["clip"].as_str().unwrap();
    s.execute("clip.setFx", &json!({"clip": id, "fx": [{"kind": "shake", "strength": 1, "speed": 2}, "flash"]}))
        .unwrap();
    let fx = s.project().clip(id).unwrap().1.fx.clone();
    assert_eq!(fx, [Fx { kind: FxKind::Shake, strength: 1.0, speed: 2.0 }, Fx::new(FxKind::Flash)]);
    for bad in [
        json!(["wobble"]),
        json!([{"kind": "shake", "strength": 2}]),
        json!([{"kind": "shake", "speed": 9}]),
        json!("shake"),
    ] {
        assert!(s.execute("clip.setFx", &json!({"clip": id, "fx": bad})).is_err());
    }
    s.execute("clip.setFx", &json!({"clip": id, "fx": []})).unwrap();
    assert!(s.project().clip(id).unwrap().1.fx.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().clip(id).unwrap().1.fx.len(), 2);

    // an adjustment clip on a track above works on the sticker for as long as it lasts
    let over = s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string();
    let adj = s
        .execute(
            "clip.add",
            &json!({"track": over, "adjustment": true, "fx": ["zoomIn"], "start": {"seconds": 1}, "duration": {"seconds": 2}}),
        )
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("clip.setColor", &json!({"clip": adj, "exposure": 1, "filter": "warm"})).unwrap();
    assert!(s.project().clip(&adj).unwrap().1.is_adjustment());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layers = spec["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2, "the adjustment clip is no layer");
    let grades = layers[0]["fx"].as_array().unwrap().iter().filter(|e| e["id"] == "color.grade@1").count();
    assert_eq!(grades, 3, "the flash, the filter and the adjustments");
    // it is a clip like any other on the timeline
    let right = s.execute("clip.split", &json!({"clip": adj, "at": {"seconds": 2}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.project().clip(&right).unwrap().1.fx, [Fx::new(FxKind::ZoomIn)]);
    // but takes no title, no transition and no lookup table, and goes on no text track
    assert!(s.execute("clip.set", &json!({"clip": adj, "text": "x"})).is_err());
    assert!(s.execute("clip.setTransition", &json!({"clip": right, "kind": "dissolve"})).is_err());
    let text = s.execute("track.add", &json!({"kind": "text"})).unwrap()["track"].as_str().unwrap().to_string();
    assert!(s.execute("clip.add", &json!({"track": text, "adjustment": true})).is_err());
    // sound has no picture to shake
    let snd = s.execute("sound.add", &json!({"sound": "pop"})).unwrap()["clip"].as_str().unwrap().to_string();
    assert!(s.execute("clip.setFx", &json!({"clip": snd, "fx": ["shake"]})).is_err());
}

#[test]
fn a_voice_is_changed_and_its_noise_taken_out() {
    let mut s = Session::default();
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/talk.wav", "duration": {"seconds": 10}, "sha256": "ce".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let audio = s.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id.clone();
    let c = s.execute("clip.add", &json!({"track": audio, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();

    let voices = s.execute("voice.list", &json!({})).unwrap();
    assert_eq!(voices["voices"].as_array().unwrap().len(), 8);
    // a voice and a nudge by hand add up; the sound keeps its length
    let set = s.execute("clip.setVoice", &json!({"clip": c, "voice": "low", "pitch": -1})).unwrap();
    assert_eq!((set["voice"].as_str(), set["semitones"].as_f64()), (Some("low"), Some(-5.0)));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["audio"][0]["pitch"], json!(-5.0));
    assert!(spec["audio"][0].get("fx").is_none());
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("audio.pitch/1")));
    // the pitch alone can change, and null takes the voice off
    s.execute("clip.setVoice", &json!({"clip": c, "pitch": 0})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.voice.semitones(), -4.0);
    s.execute("clip.setVoice", &json!({"clip": c, "voice": null})).unwrap();
    assert!(s.project().clip(&c).unwrap().1.voice.is_empty());
    assert!(s.execute("render.compile", &json!({})).unwrap()["audio"][0].get("pitch").is_none());

    // voices made of effects, in the order they are done, after the noise is out
    s.execute("clip.setVoice", &json!({"clip": c, "voice": "megaphone"})).unwrap();
    assert!(
        s.execute("clip.setDenoise", &json!({"clip": c, "amount": 0.5})).unwrap_err().to_string().contains("floor")
    );
    let floor: Vec<f32> = (0..24).map(|i| -90.0 - i as f32).collect();
    s.execute("clip.setDenoise", &json!({"clip": c, "amount": 0.5, "floor": floor})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let fx: Vec<&str> =
        spec["audio"][0]["fx"].as_array().unwrap().iter().map(|f| f["type"].as_str().unwrap()).collect();
    assert_eq!(fx, ["denoise", "band", "ring"]);
    assert_eq!(spec["audio"][0]["fx"][0]["floor"][23], json!(-113.0));
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("audio.fx/1")));
    // the amount alone keeps what was measured; none turns it off
    s.execute("clip.setDenoise", &json!({"clip": c, "amount": 1})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.voice.denoise.as_ref().unwrap().floor[1], -91.0);
    s.execute("clip.setDenoise", &json!({"clip": c, "amount": 0})).unwrap();
    assert!(s.project().clip(&c).unwrap().1.voice.denoise.is_none());
    s.execute("clip.setVoice", &json!({"clip": c, "voice": "echo"})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["audio"][0]["fx"][0]["repeats"], json!(4));

    // what is refused: a voice nobody has, a pitch too far, a wrong profile, a clip with no sound
    for bad in [
        json!({"clip": c, "voice": "dragon"}),
        json!({"clip": c, "pitch": 13}),
        json!({"clip": "nope", "voice": "low"}),
    ] {
        assert!(s.execute("clip.setVoice", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("clip.setDenoise", &json!({"clip": c, "amount": 0.5, "floor": [1, 2]})).is_err());
    assert!(s.execute("clip.setDenoise", &json!({"clip": c, "amount": 2, "floor": vec![-90; 24]})).is_err());
    let red = add_red(&mut s, 0.0, 1.0);
    assert!(
        s.execute("clip.setVoice", &json!({"clip": red, "voice": "low"})).unwrap_err().to_string().contains("no sound")
    );

    // detached from its picture, the sound takes its voice along
    let v = s
        .execute("media.import", &json!({"path": "/nowhere/v.mp4", "duration": {"seconds": 4}, "size": [64, 64]}))
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let video = video_track(&s);
    let vc = s.execute("clip.add", &json!({"track": video, "media": v})).unwrap()["clip"].as_str().unwrap().to_string();
    s.execute("clip.setVoice", &json!({"clip": vc, "voice": "robot"})).unwrap();
    let sound = s.execute("clip.detachAudio", &json!({"clip": vc})).unwrap()["audio"].as_str().unwrap().to_string();
    let p = s.project();
    assert!(p.clip(&vc).unwrap().1.voice.is_empty());
    assert_eq!(p.clip(&sound).unwrap().1.voice.kind, Some(reelforge_model::VoiceKind::Robot));
    assert!(s.execute("clip.setVoice", &json!({"clip": vc, "voice": "low"})).is_err());
}

#[test]
fn a_title_is_set_in_an_imported_font_and_spaced() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    // a font file is read where it is and called by its family
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../render/fonts/NotoSansSC-Bold.subset.ttf");
    let font = s.execute("media.import", &json!({"path": file})).unwrap();
    assert_eq!(
        (font["media"]["kind"].as_str(), font["media"]["name"].as_str()),
        (Some("font"), Some("Noto Sans SC Bold"))
    );
    let font = font["media"]["id"].as_str().unwrap().to_string();
    // the same file again is the same font
    assert_eq!(s.execute("media.import", &json!({"path": file})).unwrap()["media"]["id"], json!(font));
    assert_eq!(s.project().media.len(), 1);
    assert!(s.execute("media.import", &json!({"path": "/nowhere/readme.txt", "kind": "font"})).is_ok(), "offline");
    // a file that is no font is refused where it can be read
    let not = std::env::temp_dir().join(format!("reelforge-not-a-font-{}.ttf", std::process::id()));
    std::fs::write(&not, b"not a font").unwrap();
    assert!(s.execute("media.import", &json!({"path": not.to_string_lossy()})).is_err());
    let _ = std::fs::remove_file(&not);
    // a font is not placed as a clip
    assert!(s.execute("clip.add", &json!({"track": video_track(&s), "media": font})).is_err());

    let c = s
        .execute(
            "clip.add",
            &json!({"track": text, "text": "字体", "font": font, "tracking": 0.1, "leading": 0.25, "italic": true}),
        )
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let style = |s: &Session| match &s.project().clip(&c).unwrap().1.content {
        ClipContent::Text { style, .. } => style.clone(),
        _ => unreachable!(),
    };
    let st = style(&s);
    assert_eq!(
        (st.font.as_deref(), st.tracking, st.leading, st.italic, st.underline),
        (Some(font.as_str()), 0.1, 0.25, true, false)
    );
    // it reaches the renderer as a font asset and text/4
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let content = &spec["layers"][0]["content"];
    assert_eq!(spec["assets"][content["font"].as_str().unwrap()]["kind"], json!("font"));
    assert_eq!((content["italic"].as_bool(), content.get("underline")), (Some(true), None));
    assert!((content["tracking"].as_f64().unwrap() - 0.1).abs() < 1e-6);
    assert!((content["leading"].as_f64().unwrap() - 0.25).abs() < 1e-6);
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("text/4")));

    // a template changes the look and leaves the face, the spacing and the slant
    s.execute("clip.set", &json!({"clip": c, "template": "candy", "underline": true})).unwrap();
    let st = style(&s);
    assert_eq!(st.gradient, crate::title_template("candy").unwrap().style.gradient);
    assert_eq!((st.font.as_deref(), st.tracking, st.italic, st.underline), (Some(font.as_str()), 0.1, true, true));
    // the project's media says a title uses it, and keeps it while one does
    assert_eq!(crate::commands::media_uses(&s.project(), &font), 1);
    assert!(s.execute("media.remove", &json!({"media": font})).is_err());

    // out of range, not a font, and back to the built-in face
    assert!(s.execute("clip.set", &json!({"clip": c, "tracking": 5})).is_err());
    assert!(s.execute("clip.set", &json!({"clip": c, "leading": -1})).is_err());
    assert!(s.execute("clip.set", &json!({"clip": c, "font": c})).is_err());
    s.execute(
        "clip.set",
        &json!({"clip": c, "font": null, "tracking": 0, "leading": 0, "italic": false, "underline": false}),
    )
    .unwrap();
    assert!(style(&s).font.is_none());
    assert!(
        !s.execute("render.compile", &json!({})).unwrap()["engine"]["features"]
            .as_array()
            .unwrap()
            .contains(&json!("text/4"))
    );
    s.execute("media.remove", &json!({"media": font})).unwrap();
}

#[test]
fn a_colour_is_keyed_out_and_pictures_are_distorted() {
    let mut s = Session::default();
    let v = video_track(&s);
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/host.mp4", "kind": "video", "duration": {"seconds": 6}, "size": [1920, 1080], "sha256": "ab".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.execute("clip.add", &json!({"track": v, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();
    let chroma = |s: &Session| s.project().clip(&c).unwrap().1.look.chroma;

    // a key needs a colour first; the rest starts as most screens need and is changed one by one
    assert!(s.execute("clip.setChromaKey", &json!({"clip": c, "tolerance": 0.3})).is_err());
    let set = s.execute("clip.setChromaKey", &json!({"clip": c, "color": [0.1, 0.8, 0.2]})).unwrap();
    assert_eq!(set["chroma"]["tolerance"], json!(0.2f32));
    s.execute("clip.setChromaKey", &json!({"clip": c, "softness": 0.25, "spill": 1})).unwrap();
    let k = chroma(&s).unwrap();
    assert_eq!((k.color, k.tolerance, k.softness, k.spill), ([0.1, 0.8, 0.2], 0.2, 0.25, 1.0));
    s.execute("clip.setChromaKey", &json!({"clip": c, "color": [0, 0, 1]})).unwrap();
    assert_eq!((chroma(&s).unwrap().color, chroma(&s).unwrap().softness), ([0.0, 0.0, 1.0], 0.25));
    for bad in [json!({"tolerance": 2}), json!({"color": [0, 1]}), json!({"color": "green"}), json!({"spill": -1})] {
        let mut params = bad.clone();
        params["clip"] = json!(c);
        assert!(s.execute("clip.setChromaKey", &params).is_err(), "{bad}");
    }
    // only videos and pictures: not a colour, not a title
    let red = add_red(&mut s, 10.0, 2.0);
    assert!(s.execute("clip.setChromaKey", &json!({"clip": red, "color": [0, 1, 0]})).is_err());

    // it is the layer's first effect, before its colour work
    s.execute("clip.setColor", &json!({"clip": c, "exposure": 0.5})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let fx = spec["layers"][0]["fx"].as_array().unwrap();
    assert_eq!((fx[0]["id"].as_str(), fx[1]["id"].as_str()), (Some("key.chroma@1"), Some("color.grade@1")));
    assert_eq!(fx[0]["params"]["b"], json!(1.0));
    assert_eq!(fx[0]["params"]["softness"], json!(0.25));
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("key.chroma@1")));
    // null takes it off
    s.execute("clip.setChromaKey", &json!({"clip": c, "color": null})).unwrap();
    assert_eq!(chroma(&s), None);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(!spec["engine"]["features"].as_array().unwrap().contains(&json!("key.chroma@1")));

    // the distortions are listed as a group of their own and become the renderer's four
    let list = s.execute("fx.list", &json!({})).unwrap();
    let distort: Vec<&str> = list["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["group"] == "distort")
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(distort, ["rgbSplit", "glitch", "mosaic", "wave"]);
    s.execute("clip.setFx", &json!({"clip": c, "fx": ["glitch", "mosaic", "wave"]})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let ids: Vec<&str> =
        spec["layers"][0]["fx"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["color.grade@1", "distort.wave@1", "distort.glitch@1", "distort.mosaic@1", "distort.split@1"]);
    let params = |id: &str| {
        spec["layers"][0]["fx"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap()["params"].clone()
    };
    // a mosaic stands still; a glitch tears in bursts, each tear a whole-numbered pattern
    let size: Vec<f64> =
        params("distort.mosaic@1")["size"].as_array().unwrap().iter().map(|k| k["v"].as_f64().unwrap()).collect();
    assert!(size.iter().all(|v| (*v - 0.05).abs() < 1e-6));
    let glitch = params("distort.glitch@1");
    let amounts: Vec<f64> = glitch["amount"].as_array().unwrap().iter().map(|k| k["v"].as_f64().unwrap()).collect();
    assert!(amounts.iter().any(|v| *v > 0.0) && amounts.contains(&0.0));
    assert!(glitch["seed"].as_array().unwrap().iter().all(|k| k["v"].as_f64().unwrap().fract() == 0.0));
    let features = spec["engine"]["features"].as_array().unwrap();
    for id in ["distort.wave@1", "distort.glitch@1", "distort.mosaic@1", "distort.split@1"] {
        assert!(features.contains(&json!(id)), "{id}");
    }

    // from an adjustment clip above, a distortion plays on the layers below only while it lasts
    s.execute("clip.setFx", &json!({"clip": c, "fx": []})).unwrap();
    let above = s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string();
    s.execute(
        "clip.add",
        &json!({"track": above, "adjustment": true, "fx": ["mosaic"], "start": {"seconds": 2}, "duration": {"seconds": 1}}),
    )
    .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = spec["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(c)).unwrap();
    let size =
        &layer["fx"].as_array().unwrap().iter().find(|e| e["id"] == "distort.mosaic@1").unwrap()["params"]["size"];
    let at = |secs: f64| {
        let t = (secs * reelforge_time::TICKS_PER_SECOND as f64) as i64;
        size.as_array().unwrap().iter().rev().find(|k| k["t"].as_i64().unwrap() <= t).unwrap()["v"].as_f64().unwrap()
    };
    assert_eq!((at(1.0), at(3.5)), (0.0, 0.0));
    assert!((at(2.5) - 0.05).abs() < 1e-6);
}

#[test]
fn clips_are_grouped_and_joined_into_a_compound_clip() {
    let mut s = Session::default();
    let v = video_track(&s);
    let track = |s: &Session, kind: TrackKind| s.project().tracks.iter().find(|t| t.kind == kind).unwrap().id.clone();
    let (text, sound) = (track(&s, TrackKind::Text), track(&s, TrackKind::Audio));
    let id = |v: &serde_json::Value, key: &str| v[key].as_str().unwrap().to_string();
    let secs = |x: f64| json!({ "seconds": x });
    // a video with sound from 1 s to 7 s, a title over it from 2 s to 4 s, a red card at 8 s
    let m = id(
        &s.execute(
            "media.import",
            &json!({"path": "/nowhere/v.mp4", "kind": "video", "duration": secs(6.0), "sha256": "ef".repeat(32)}),
        )
        .unwrap()["media"],
        "id",
    );
    let a = id(&s.execute("clip.add", &json!({"track": v, "media": m, "start": secs(1.0)})).unwrap(), "clip");
    let t = id(
        &s.execute("clip.add", &json!({"track": text, "text": "Hi", "start": secs(2.0), "duration": secs(2.0)}))
            .unwrap(),
        "clip",
    );
    let red = add_red(&mut s, 8.0, 2.0);
    let group = |s: &Session, c: &str| s.project().clip(c).unwrap().1.group.clone();

    // a group is two clips or more, and a clip is in one group at a time
    assert!(s.execute("clip.group", &json!({"clips": [a]})).is_err());
    let g = id(&s.execute("clip.group", &json!({"clips": [a, t]})).unwrap(), "group");
    assert_eq!((group(&s, &a), group(&s, &t)), (Some(g.clone()), Some(g.clone())));
    s.execute("clip.group", &json!({"clips": [t, red]})).unwrap();
    assert_eq!(group(&s, &a), None, "what is left of a group of two is no group");
    assert!(group(&s, &t).is_some() && group(&s, &t) == group(&s, &red));
    // any member dissolves it
    let freed = s.execute("clip.ungroup", &json!({"clips": t})).unwrap();
    assert_eq!(freed["clips"].as_array().unwrap().len(), 2);
    assert_eq!((group(&s, &t), group(&s, &red)), (None, None));
    assert!(s.execute("clip.ungroup", &json!({"clips": [t]})).is_err());
    // copies of grouped clips are a group of their own
    let g = id(&s.execute("clip.group", &json!({"clips": [a, t]})).unwrap(), "group");
    let copies = s.execute("clip.duplicate", &json!({"clips": [a, t]})).unwrap();
    let copies: Vec<String> = copies["clips"].as_array().unwrap().iter().map(|c| id(c, "clip")).collect();
    assert!(group(&s, &copies[0]).is_some_and(|c| c != g) && group(&s, &copies[0]) == group(&s, &copies[1]));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("clip.ungroup", &json!({"clips": a})).unwrap();

    // sound alone is no compound clip
    let alone = id(&s.execute("clip.add", &json!({"track": sound, "media": m, "start": secs(20.0)})).unwrap(), "clip");
    assert!(s.execute("clip.compound", &json!({"clips": alone})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();

    // the video and its title become one clip where the video was, playing a timeline of their own
    let made = s.execute("clip.compound", &json!({"clips": [a, t], "name": "Intro"})).unwrap();
    let (cc, inner) = (id(&made, "clip"), id(&made, "timeline"));
    assert_eq!(made["track"], json!(v));
    let p = s.project();
    let (_, clip) = p.clip(&cc).unwrap();
    assert_eq!((clip.start, clip.duration), (Tick::SECOND, Tick::SECOND * 6));
    assert_eq!(clip.compound(), Some((inner.as_str(), Tick::ZERO)));
    assert!(p.clip(&a).is_none() && p.clip(&t).is_none() && p.clip(&red).is_some());
    let tracks = p.tracks_of(&inner).unwrap();
    assert_eq!(tracks.iter().map(|t| t.kind).collect::<Vec<_>>(), [TrackKind::Video, TrackKind::Text]);
    assert_eq!((tracks[0].clips[0].id.as_str(), tracks[0].clips[0].start), (a.as_str(), Tick::ZERO));
    assert_eq!((tracks[1].clips[0].id.as_str(), tracks[1].clips[0].start), (t.as_str(), Tick::SECOND));
    let list = s.execute("timeline.list", &json!({})).unwrap();
    let listed = list["timelines"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!((&listed[1]["name"], &listed[1]["compound"]), (&json!("Intro"), &json!(true)));
    assert_eq!(listed[1]["duration"], json!(Tick::SECOND * 6));
    assert!(listed[0].get("compound").is_none());

    // the renderer gets a composition, a layer that shows it and a clip that plays its mix
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let comp = &spec["comps"][&inner];
    assert_eq!((&comp["layers"][0]["id"], &comp["layers"][1]["id"]), (&json!(a), &json!(t)));
    assert_eq!(comp["audio"][0]["id"], json!(a));
    assert_eq!(spec["layers"][0]["content"], json!({"type": "comp", "comp": inner, "from": 0}));
    assert_eq!((&spec["audio"][0]["id"], &spec["audio"][0]["comp"]), (&json!(cc), &json!(inner)));
    assert!(spec["audio"][0].get("asset").is_none());
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("comp/1")));

    // it is a clip like any other: trimmed and split it goes on from where it is in its timeline
    s.execute("clip.trim", &json!({"clip": cc, "start": secs(2.0)})).unwrap();
    assert_eq!(s.project().clip(&cc).unwrap().1.compound(), Some((inner.as_str(), Tick::SECOND)));
    assert!(s.execute("clip.trim", &json!({"clip": cc, "start": secs(0.5)})).is_err(), "before its timeline starts");
    let right = id(&s.execute("clip.split", &json!({"clip": cc, "at": secs(4.0)})).unwrap(), "right");
    assert_eq!(s.project().clip(&right).unwrap().1.compound(), Some((inner.as_str(), Tick::SECOND * 3)));
    s.execute("clip.set", &json!({"clip": cc, "opacity": 0.5, "x": 0.1, "scale": 0.8})).unwrap();
    s.execute("clip.setFx", &json!({"clip": cc, "fx": ["glitch"]})).unwrap();
    s.execute("clip.setCrop", &json!({"clip": cc, "flipX": true, "left": 0.1})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = &spec["layers"][0];
    assert_eq!((&layer["flip_x"], &layer["crop"]["left"]), (&json!(true), &json!(0.1f32)));
    assert_eq!(spec["layers"][1]["content"]["from"], json!(Tick::SECOND * 3));
    assert_eq!(spec["audio"][1]["time"]["source_in"], json!(Tick::SECOND * 3));

    // a played timeline stays, and no timeline plays inside itself
    assert!(s.execute("timeline.remove", &json!({"timeline": inner})).is_err());
    let first = s.project().timeline.clone();
    s.execute("timeline.switch", &json!({"timeline": inner})).unwrap();
    let inside = video_track(&s);
    for outer in [&inner, &first] {
        let err = s.execute("clip.add", &json!({"track": inside, "timeline": outer, "start": secs(30.0)}));
        assert!(err.is_err(), "{outer}");
    }
    s.execute("timeline.switch", &json!({"timeline": first})).unwrap();

    // taken apart, the part it played is back on the tracks, cut as the clip was
    assert!(s.execute("clip.uncompound", &json!({"clip": red})).is_err());
    let out = s.execute("clip.uncompound", &json!({"clip": cc})).unwrap();
    let out = out["clips"].as_array().unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!((&out[0]["track"], &out[1]["track"]), (&json!(v), &json!(text)));
    let p = s.project();
    let (_, video) = p.clip(out[0]["clip"].as_str().unwrap()).unwrap();
    assert_eq!((video.start, video.duration), (Tick::SECOND * 2, Tick::SECOND * 2));
    assert!(matches!(&video.content, ClipContent::Media { source_in, .. } if *source_in == Tick::SECOND));
    let (_, title) = p.clip(out[1]["clip"].as_str().unwrap()).unwrap();
    assert_eq!((title.start, title.duration), (Tick::SECOND * 2, Tick::SECOND * 2));
    assert!(p.clip(&cc).is_none() && p.tracks_of(&inner).is_some(), "the other half still plays it");
    // the last of its clips takes the timeline with it
    let out = s.execute("clip.uncompound", &json!({"clip": right})).unwrap();
    assert_eq!(out["clips"].as_array().unwrap().len(), 1, "the title was over before this half");
    let p = s.project();
    let (_, rest) = p.clip(out["clips"][0]["clip"].as_str().unwrap()).unwrap();
    assert_eq!((rest.start, rest.duration), (Tick::SECOND * 4, Tick::SECOND * 3));
    assert!(matches!(&rest.content, ClipContent::Media { source_in, .. } if *source_in == Tick::SECOND * 3));
    assert!(p.tracks_of(&inner).is_none() && p.timelines.len() == 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().tracks_of(&inner).is_some());

    // a timeline made as a cut of its own is placed with clip.add, and stays when taken apart
    s.execute("timeline.add", &json!({"name": "Outro"})).unwrap();
    let outro = s.project().timeline.clone();
    add_red(&mut s, 0.0, 3.0);
    s.execute("timeline.switch", &json!({"timeline": first})).unwrap();
    let placed = s.execute("clip.add", &json!({"track": v, "timeline": outro, "start": secs(40.0)})).unwrap();
    assert_eq!(placed["duration"], json!(Tick::SECOND * 3));
    s.execute("clip.uncompound", &json!({"clip": placed["clip"]})).unwrap();
    assert!(s.project().tracks_of(&outro).is_some());
}

#[test]
fn time_is_cut_out_of_every_track_and_filler_captions_go_with_theirs() {
    let mut s = Session::default();
    let v = video_track(&s);
    let track = |s: &Session, kind: TrackKind| s.project().tracks.iter().find(|t| t.kind == kind).unwrap().id.clone();
    let (text, sound) = (track(&s, TrackKind::Text), track(&s, TrackKind::Audio));
    let id = |v: &serde_json::Value, key: &str| v[key].as_str().unwrap().to_string();
    let secs = |x: f64| json!({ "seconds": x });
    let t = |x: f64| Tick::from_seconds(x);
    let import = |s: &mut Session, name: &str, kind: &str, sha: &str| {
        let m =
            json!({"path": format!("/nowhere/{name}"), "kind": kind, "duration": secs(10.0), "sha256": sha.repeat(32)});
        id(&s.execute("media.import", &m).unwrap()["media"], "id")
    };
    // ten seconds of someone talking over music, with what is said as captions
    let (talk, song) = (import(&mut s, "talk.mp4", "video", "ab"), import(&mut s, "song.wav", "audio", "cd"));
    s.execute("clip.add", &json!({"track": v, "media": talk, "start": 0})).unwrap();
    s.execute("clip.add", &json!({"track": sound, "media": song, "start": 0})).unwrap();
    for (words, from, to) in [
        ("大家好", 0.0, 2.0),
        ("嗯，那个……", 2.0, 3.0),
        ("今天讲三件事", 3.0, 6.0),
        ("Um, you know", 6.0, 7.0),
        ("就是这样", 7.0, 9.0),
    ] {
        let cue = json!({"track": text, "text": words, "start": secs(from), "duration": secs(to - from)});
        s.execute("clip.add", &cue).unwrap();
    }
    s.execute("marker.add", &json!({"at": secs(2.5)})).unwrap();
    s.execute("marker.add", &json!({"at": secs(8.0)})).unwrap();
    // what each track holds: (start, end, where it starts in its source)
    let on = |s: &Session, track: &str| -> Vec<(Tick, Tick, Tick)> {
        let p = s.project();
        let clips = p.track(track).unwrap().clips.iter();
        clips.map(|c| (c.start, c.end(), c.timing().map_or(Tick::ZERO, |m| m.source_in))).collect()
    };
    let said = |s: &Session| -> Vec<String> {
        let p = s.project();
        let clips = p.track(&text).unwrap().clips.iter();
        clips.filter_map(|c| serde_json::to_value(&c.content).unwrap()["text"].as_str().map(str::to_string)).collect()
    };
    let marks = |s: &Session| s.project().markers.iter().map(|m| m.at).collect::<Vec<_>>();

    // the captions that say nothing
    let found = s.execute("captions.fillers", &json!({})).unwrap();
    let texts: Vec<String> = found["fillers"].as_array().unwrap().iter().map(|f| id(f, "text")).collect();
    assert_eq!(texts, ["嗯，那个……", "Um, you know"]);
    assert_eq!(found["time"], json!(t(2.0)));
    assert_eq!(
        s.execute("captions.fillers", &json!({"words": ["就是这样"]})).unwrap()["fillers"].as_array().unwrap().len(),
        3
    );

    // cut from the picture and the captions, and the music plays on
    let out = s.execute("captions.cutFillers", &json!({"tracks": [v, text]})).unwrap();
    assert_eq!((&out["removed"], &out["duration"]), (&json!(t(2.0)), &json!(t(10.0))));
    assert_eq!(on(&s, &v), [(t(0.0), t(2.0), t(0.0)), (t(2.0), t(5.0), t(3.0)), (t(5.0), t(8.0), t(7.0))]);
    assert_eq!(said(&s), ["大家好", "今天讲三件事", "就是这样"]);
    assert_eq!(
        on(&s, &text).iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(),
        [(t(0.0), t(2.0)), (t(2.0), t(5.0)), (t(5.0), t(7.0))]
    );
    assert_eq!(on(&s, &sound), [(t(0.0), t(10.0), t(0.0))]);
    assert_eq!(marks(&s), [t(2.5), t(8.0)], "markers stay with the music");
    assert!(s.execute("captions.cutFillers", &json!({})).is_err(), "none is left");
    // one step back
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(on(&s, &v), [(t(0.0), t(10.0), t(0.0))]);

    // ranges that overlap are one; every track is cut, a clip through a range is cut in two, and
    // markers move
    let ranges = json!([{"from": secs(1.0), "to": secs(2.0)}, {"from": secs(1.5), "to": secs(3.0)}]);
    let out = s.execute("timeline.cut", &json!({"ranges": ranges})).unwrap();
    assert_eq!(out["ranges"], json!([{"from": t(1.0), "to": t(3.0)}]));
    assert_eq!((&out["removed"], &out["duration"]), (&json!(t(2.0)), &json!(t(8.0))));
    assert_eq!(on(&s, &v), [(t(0.0), t(1.0), t(0.0)), (t(1.0), t(8.0), t(3.0))]);
    assert_eq!(on(&s, &sound), [(t(0.0), t(1.0), t(0.0)), (t(1.0), t(8.0), t(3.0))]);
    assert_eq!(said(&s), ["大家好", "今天讲三件事", "Um, you know", "就是这样"]);
    assert_eq!(on(&s, &text)[..2].iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(), [(t(0.0), t(1.0)), (t(1.0), t(4.0))]);
    assert_eq!(marks(&s), [t(6.0)]);
    // a locked track is left as it is
    s.execute("track.set", &json!({"track": sound, "locked": true})).unwrap();
    s.execute("timeline.cut", &json!({"ranges": [{"from": 0, "to": secs(1.0)}]})).unwrap();
    assert_eq!(on(&s, &v), [(t(0.0), t(7.0), t(3.0))]);
    assert_eq!(on(&s, &sound).len(), 2);
    // and a range is a stretch of time
    assert!(s.execute("timeline.cut", &json!({"ranges": []})).is_err());
    assert!(s.execute("timeline.cut", &json!({"ranges": [{"from": secs(2.0), "to": secs(2.0)}]})).is_err());
    assert!(s.execute("timeline.cut", &json!({"ranges": [{"from": 0, "to": 5}], "tracks": ["t99"]})).is_err());
}
