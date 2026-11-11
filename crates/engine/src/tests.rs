use reelforge_model::{AnimKind, ClipContent, Fill, Fx, FxKind, MediaKind, Prop, TrackKind};
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
fn media_is_renamed_and_filed_under_folders_that_last_while_used() {
    let mut s = Session::default();
    let mut ids = Vec::new();
    for name in ["beach.mp4", "street.mp4", "logo.png"] {
        let r = s
            .execute(
                "media.import",
                &json!({"path": format!("/nowhere/{name}"), "kind": "video", "sha256": "ab".repeat(32)}),
            )
            .unwrap();
        ids.push(r["media"]["id"].as_str().unwrap().to_string());
    }
    // several at once; the folder is there because they are in it
    let r = s.execute("media.set", &json!({"media": [ids[0], ids[1]], "folder": " B-roll "})).unwrap();
    assert_eq!(r["folders"], json!(["B-roll"]));
    assert_eq!(s.project().media[&ids[0]].folder.as_deref(), Some("B-roll"));
    assert_eq!(s.project().media[&ids[2]].folder, None);
    // one is renamed and moved in one step
    let r = s.execute("media.set", &json!({"media": ids[2], "name": "Logo", "folder": "Brand"})).unwrap();
    assert_eq!(r["folders"], json!(["B-roll", "Brand"]));
    assert_eq!(s.project().media[&ids[2]].name, "Logo");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((s.project().media[&ids[2]].name.as_str(), &s.project().media[&ids[2]].folder), ("logo.png", &None));
    // taking the last file out leaves no folder behind
    s.execute("media.set", &json!({"media": ids[0], "folder": null})).unwrap();
    let r = s.execute("media.set", &json!({"media": [ids[1]], "folder": null})).unwrap();
    assert_eq!(r["folders"], json!([]));
    // a project file written before folders reads as it did, and one without folders says nothing of them
    assert!(!serde_json::to_string(&*s.project()).unwrap().contains("folder"));

    for bad in [
        json!({"media": ids[0]}),
        json!({"media": [], "folder": "x"}),
        json!({"media": [ids[0], ids[1]], "name": "both"}),
        json!({"media": ids[0], "name": "  "}),
        json!({"media": ids[0], "folder": ""}),
        json!({"media": ids[0], "folder": "x".repeat(41)}),
        json!({"media": ids[0], "folder": 3}),
        json!({"media": "m999", "folder": "x"}),
        json!({"folder": "x"}),
    ] {
        assert!(s.execute("media.set", &bad).is_err(), "{bad}");
    }
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
fn the_people_in_a_clip_are_cut_out_and_put_back() {
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
    let person = |s: &Session| s.project().clip(&c).unwrap().1.look.person;
    let features = |s: &mut Session| s.execute("render.compile", &json!({})).unwrap()["engine"]["features"].clone();

    // on when nothing else is said; it is the layer's first effect, before a keyed colour
    assert_eq!(s.execute("clip.setPersonKey", &json!({"clip": c})).unwrap(), json!({"clip": c, "person": true}));
    assert!(person(&s));
    s.execute("clip.setChromaKey", &json!({"clip": c, "color": [0.1, 0.8, 0.2]})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let fx = spec["layers"][0]["fx"].as_array().unwrap();
    assert_eq!((fx[0]["id"].as_str(), fx[1]["id"].as_str()), (Some("key.person@1"), Some("key.chroma@1")));
    assert!(features(&mut s).as_array().unwrap().contains(&json!("key.person@1")));
    // it is part of the clip: saved with it, kept when the clip is split
    assert_eq!(serde_json::to_value(s.project().clip(&c).unwrap().1).unwrap()["look"]["person"], json!(true));
    let right = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 2}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.project().clip(&right).unwrap().1.look.person);
    s.execute("edit.undo", &json!({})).unwrap();

    // off, and undone
    s.execute("clip.setPersonKey", &json!({"clip": c, "on": false})).unwrap();
    assert!(!person(&s));
    assert!(!features(&mut s).as_array().unwrap().contains(&json!("key.person@1")));
    assert!(!serde_json::to_string(s.project().clip(&c).unwrap().1).unwrap().contains("person"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(person(&s));

    // only videos and pictures, and only a yes or a no
    let red = add_red(&mut s, 10.0, 2.0);
    assert!(s.execute("clip.setPersonKey", &json!({"clip": red})).is_err());
    assert!(s.execute("clip.setPersonKey", &json!({"clip": c, "on": "yes"})).is_err());
    assert!(s.execute("clip.setPersonKey", &json!({"clip": "nothing"})).is_err());
}

#[test]
fn a_product_is_cut_out_from_its_backdrop_and_casts_a_shadow() {
    let mut s = Session::default();
    let v = video_track(&s);
    let m = s
        .execute(
            "media.import",
            &json!({"path": "/nowhere/flask.png", "kind": "image", "size": [1200, 1600], "sha256": "cd".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.execute("clip.add", &json!({"track": v, "media": m})).unwrap()["clip"].as_str().unwrap().to_string();
    let look = |s: &Session| serde_json::to_value(&s.project().clip(&c).unwrap().1.look).unwrap();
    let fx = |s: &mut Session| -> Vec<Value> {
        s.execute("render.compile", &json!({})).unwrap()["layers"][0]["fx"].as_array().cloned().unwrap_or_default()
    };

    // cut out as most studio pictures need, when nothing else is said
    let set = s.execute("clip.setCutout", &json!({"clip": c})).unwrap();
    assert_eq!(set, json!({"clip": c, "cutout": {"tolerance": 0.5, "holes": 0.5}}));
    // one thing changed: the other stays as it was
    s.execute("clip.setCutout", &json!({"clip": c, "tolerance": 0.75})).unwrap();
    s.execute("clip.setCutout", &json!({"clip": c, "holes": 0})).unwrap();
    assert_eq!(look(&s)["cutout"], json!({"tolerance": 0.75, "holes": 0.0}));
    let layer = fx(&mut s);
    assert_eq!(layer[0]["id"], json!("key.backdrop@1"));
    let near = |v: &Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-6;
    assert!(near(&layer[0]["params"]["tolerance"], 0.75) && near(&layer[0]["params"]["holes"], 0.0), "{layer:?}");

    // a shadow: floating when nothing is said, a kind by name, then one thing at a time
    let set = s.execute("clip.setShadow", &json!({"clip": c})).unwrap();
    assert!(near(&set["shadow"]["opacity"], 0.45) && near(&set["shadow"]["flat"], 0.0), "{set}");
    let set = s.execute("clip.setShadow", &json!({"clip": c, "kind": "ground", "opacity": 0.3})).unwrap();
    assert!(near(&set["shadow"]["flat"], 0.86) && near(&set["shadow"]["opacity"], 0.3), "{set}");
    let set =
        s.execute("clip.setShadow", &json!({"clip": c, "color": [0.2, 0, 0.4], "lean": -0.25, "blur": 0.05})).unwrap();
    let shadow = &set["shadow"];
    assert!(near(&shadow["flat"], 0.86) && near(&shadow["opacity"], 0.3) && near(&shadow["lean"], -0.25), "{set}");
    assert!(near(&shadow["color"][2], 0.4) && near(&shadow["blur"], 0.05), "{set}");
    // the renderer is told after everything that changes the picture's outline or colours
    let layer = fx(&mut s);
    assert_eq!(layer.last().unwrap()["id"], json!("shadow.drop@1"));
    assert!(near(&layer.last().unwrap()["params"]["lean"], -0.25));
    let features = s.execute("render.compile", &json!({})).unwrap()["engine"]["features"].clone();
    assert!(features.as_array().unwrap().contains(&json!("shadow.drop@1")));
    assert!(features.as_array().unwrap().contains(&json!("key.backdrop@1")));
    // both are the clip's: kept by its halves when it is split
    let right = s.execute("clip.split", &json!({"clip": c, "at": {"seconds": 1}})).unwrap()["right"]
        .as_str()
        .unwrap()
        .to_string();
    let half = s.project().clip(&right).unwrap().1.look.clone();
    assert!(half.cutout.is_some() && half.shadow.is_some_and(|s| s.lean == -0.25));
    s.execute("edit.undo", &json!({})).unwrap();

    // off, and undone
    s.execute("clip.setShadow", &json!({"clip": c, "on": false})).unwrap();
    s.execute("clip.setCutout", &json!({"clip": c, "on": false})).unwrap();
    assert_eq!(look(&s), json!({}));
    assert!(fx(&mut s).is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(look(&s)["cutout"].is_object() && look(&s)["shadow"].is_object());

    // a compound clip is a picture too; a colour or a title is not, and values keep to their ranges
    let red = add_red(&mut s, 10.0, 2.0);
    for command in ["clip.setCutout", "clip.setShadow"] {
        assert!(s.execute(command, &json!({"clip": red})).is_err(), "{command}");
        assert!(s.execute(command, &json!({"clip": "nothing"})).is_err(), "{command}");
        assert!(s.execute(command, &json!({"clip": c, "on": "yes"})).is_err(), "{command}");
    }
    for bad in [json!({"tolerance": 1.5}), json!({"holes": -0.1}), json!({"softness": 0.2})] {
        let mut params = bad.clone();
        params["clip"] = json!(c);
        assert!(s.execute("clip.setCutout", &params).is_err(), "{bad}");
    }
    for bad in [
        json!({"kind": "sun"}),
        json!({"opacity": 2}),
        json!({"blur": 0.6}),
        json!({"x": -2}),
        json!({"color": "black"}),
    ] {
        let mut params = bad.clone();
        params["clip"] = json!(c);
        assert!(s.execute("clip.setShadow", &params).is_err(), "{bad}");
    }
    let folded = s.execute("clip.compound", &json!({"clips": [c]})).unwrap()["clip"].as_str().unwrap().to_string();
    assert!(s.execute("clip.setShadow", &json!({"clip": folded, "kind": "glow"})).is_ok());
    assert!(s.execute("clip.setCutout", &json!({"clip": folded})).is_ok());
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

fn import(s: &mut Session, name: &str, kind: &str, secs: Option<f64>) -> String {
    let mut params = json!({"path": format!("/nowhere/{name}"), "kind": kind, "sha256": "cd".repeat(32)});
    if let Some(secs) = secs {
        params["duration"] = json!({"seconds": secs});
    }
    s.execute("media.import", &params).unwrap()["media"]["id"].as_str().unwrap().to_string()
}

#[test]
fn a_template_is_built_with_slots_that_files_fill_keeping_what_is_set_on_them() {
    let mut s = Session::default();
    let listed = s.execute("template.list", &json!({})).unwrap();
    let ids: Vec<&str> = listed["templates"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["beat", "vlog", "cinematic", "showcase"]);
    assert_eq!(
        (listed["templates"][0]["slots"].as_u64(), listed["templates"][0]["seconds"].as_f64()),
        (Some(10), Some(16.0))
    );

    // every built-in template builds a timeline that compiles, with all its slots empty
    for id in &ids {
        let mut s = Session::default();
        let r = s.execute("template.apply", &json!({"template": id})).unwrap();
        let slots = crate::timeline_template(id).unwrap().slots();
        assert_eq!((r["slots"].as_array().unwrap().len(), r["empty"].as_u64()), (slots, Some(slots as u64)), "{id}");
        s.execute("render.compile", &json!({})).unwrap_or_else(|e| panic!("{id}: {e}"));
        // one undo step takes all of it away
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.project().tracks.iter().all(|t| t.clips.is_empty()), "{id}");
    }

    // on an empty timeline it is built in place, with the titles' words as given
    let r = s.execute("template.apply", &json!({"template": "cinematic", "texts": ["海边", null]})).unwrap();
    assert_eq!(s.project().timeline_list().len(), 1);
    assert_eq!(r["texts"][0]["text"], "海边");
    assert_eq!(r["texts"][1]["text"], "THE END");
    let clips: Vec<String> = r["clips"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    let before = s.project().clip(&clips[1]).unwrap().1.clone();
    assert!(crate::is_empty_slot(&before) && before.transition.is_some() && !before.fx.is_empty());

    // files fill the empty slots in order: a still, a long video, a short one (slowed to fill)
    let still = import(&mut s, "a.png", "image", None);
    let long = import(&mut s, "b.mp4", "video", Some(10.0));
    let short = import(&mut s, "c.mp4", "video", Some(1.0));
    let r = s.execute("template.fill", &json!({"media": [still, long]})).unwrap();
    assert_eq!((r["filled"].clone(), r["empty"].as_u64()), (json!([1, 2]), Some(2)));
    let after = s.project().clip(&clips[1]).unwrap().1.clone();
    assert_eq!(after.content, ClipContent::media(long.clone(), Tick::ZERO));
    // the place, the length and everything set on the slot stay
    assert_eq!(
        (after.start, after.duration, &after.transition, &after.fx, after.slot),
        (before.start, before.duration, &before.transition, &before.fx, Some(2))
    );
    let r = s.execute("template.fill", &json!({"media": [short, null, long]})).unwrap();
    assert_eq!((r["filled"].clone(), r["empty"].as_u64(), r["left"].clone()), (json!([3]), Some(1), json!([long])));
    match &s.project().clip(&clips[2]).unwrap().1.content {
        ClipContent::Media { media, speed, .. } => assert_eq!((media, *speed), (&short, 0.25)),
        other => panic!("{other:?}"),
    }
    // asked not to slow, a short file is refused and nothing changes
    let e = s.execute("template.fill", &json!({"media": [short], "fit": "refuse"})).unwrap_err().to_string();
    assert!(e.contains("shorter"), "{e}");
    assert!(crate::is_empty_slot(s.project().clip(&clips[3]).unwrap().1));

    // from a slot on, what is there is replaced
    let r = s.execute("template.fill", &json!({"media": [long], "from": 1})).unwrap();
    assert_eq!(r["slots"][0]["media"], json!(long));
    s.execute("render.compile", &json!({})).unwrap();

    // what fills the frame goes on filling it: a wide picture in the tall canvas's empty slot is
    // scaled to cover it, and a picture of the canvas's shape after it is back at 1
    let sized = |s: &mut Session, name: &str, size: [u32; 2]| {
        let params =
            json!({"path": format!("/nowhere/{name}"), "kind": "image", "size": size, "sha256": "ef".repeat(32)});
        s.execute("media.import", &params).unwrap()["media"]["id"].as_str().unwrap().to_string()
    };
    assert_eq!((s.project().canvas.width, s.project().canvas.height), (1080, 1920));
    let (wide, tall) = (sized(&mut s, "wide.png", [1920, 1080]), sized(&mut s, "tall.png", [1080, 1920]));
    let scale = |s: &Session| s.project().clip(&clips[3]).unwrap().1.transform.scale;
    s.execute("template.fill", &json!({"media": [wide]})).unwrap();
    assert!((scale(&s) - 3.1605).abs() < 0.001, "{}", scale(&s));
    s.execute("template.fill", &json!({"media": [tall], "from": 4})).unwrap();
    assert!((scale(&s) - 1.0).abs() < 0.001, "{}", scale(&s));
    // a size the person set is theirs: it is not touched
    s.execute("clip.set", &json!({"clip": clips[3], "scale": 0.5})).unwrap();
    s.execute("template.fill", &json!({"media": [wide], "from": 4})).unwrap();
    assert_eq!(scale(&s), 0.5);

    // a second template gets a timeline of its own, filled as it is built
    let r = s.execute("template.apply", &json!({"template": "beat", "media": [still, long], "name": "卡点"})).unwrap();
    assert_eq!(s.project().timeline_list().len(), 2);
    assert_eq!((r["empty"].as_u64(), r["slots"][1]["media"].clone()), (Some(8), json!(long)));
    assert!(s.project().timeline_list().iter().any(|t| t.current && t.name == "卡点"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().timeline_list().len(), 1);
}

#[test]
fn any_picture_clip_is_replaced_or_made_a_slot() {
    let mut s = Session::default();
    let video = video_track(&s);
    let a = add_video(&mut s, 6.0);
    let b = add_red(&mut s, 6.0, 2.0);
    let audio = s.project().tracks.iter().find(|t| t.kind == reelforge_model::TrackKind::Audio).unwrap().id.clone();
    let song = import(&mut s, "song.wav", "audio", Some(30.0));
    let music = s.execute("clip.add", &json!({"track": audio, "media": song, "duration": {"seconds": 8}})).unwrap();
    let music = music["clip"].as_str().unwrap().to_string();
    let title = s.execute("clip.add", &json!({"track": s.project().tracks[1].id, "text": "hi"})).unwrap();
    let title = title["clip"].as_str().unwrap().to_string();

    // a video at half speed with an animation: another video takes its place and runs the same way
    s.execute("clip.setSpeed", &json!({"clip": a, "speed": 2.0})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": a, "slot": "in", "kind": "fade"})).unwrap();
    let before = s.project().clip(&a).unwrap().1.clone();
    let other = import(&mut s, "other.mp4", "video", Some(20.0));
    let r = s.execute("clip.replace", &json!({"clip": a, "media": other, "sourceIn": {"seconds": 5}})).unwrap();
    assert_eq!(r["speed"], 2.0);
    let after = s.project().clip(&a).unwrap().1.clone();
    assert_eq!((after.start, after.duration, &after.anim), (before.start, before.duration, &before.anim));
    assert_eq!(source_at(&s, &a, 1.0), Tick::from_seconds(7.0));
    // one that ends too soon is refused, unless it may be slowed down
    let brief = import(&mut s, "brief.mp4", "video", Some(1.5));
    let e = s.execute("clip.replace", &json!({"clip": a, "media": brief})).unwrap_err().to_string();
    assert!(e.contains("shorter") && e.contains("slow"), "{e}");
    s.execute("clip.replace", &json!({"clip": a, "media": brief, "fit": "slow"})).unwrap();
    assert_eq!(source_at(&s, &a, before.duration.seconds()), Tick::from_seconds(1.5));
    // a colour takes a picture; a sound takes a sound; nothing else is replaced
    let still = import(&mut s, "p.png", "image", None);
    s.execute("clip.replace", &json!({"clip": b, "media": still})).unwrap();
    s.execute("clip.replace", &json!({"clip": music, "media": song, "sourceIn": {"seconds": 10}})).unwrap();
    for (clip, media) in [(&music, &still), (&b, &song), (&title, &still)] {
        assert!(s.execute("clip.replace", &json!({"clip": clip, "media": media})).is_err(), "{clip} {media}");
    }

    // pictures become slots in the order they play; a sound or a title cannot
    let r = s.execute("template.mark", &json!({"clips": [b, a]})).unwrap();
    assert_eq!(r["clips"], json!([{"clip": a, "slot": 1}, {"clip": b, "slot": 2}]));
    assert!(s.execute("template.mark", &json!({"clips": [music]})).is_err());
    assert!(s.execute("template.mark", &json!({"clips": [title]})).is_err());
    let r = s.execute("template.slots", &json!({})).unwrap();
    assert_eq!(
        (r["slots"].as_array().unwrap().len(), r["empty"].as_u64(), r["texts"][0]["text"].clone()),
        (2, Some(0), json!("hi"))
    );
    // the same edit with other files
    s.execute("template.fill", &json!({"media": [other, other], "from": 1})).unwrap();
    assert_eq!(source_at(&s, &a, 0.0), Tick::ZERO);
    // two clips with one number take the same file; null makes them ordinary again
    s.execute("template.mark", &json!({"clips": [b], "slot": 1})).unwrap();
    s.execute("template.fill", &json!({"media": [still], "from": 1})).unwrap();
    for clip in [&a, &b] {
        assert_eq!(s.project().clip(clip).unwrap().1.content, ClipContent::media(still.clone(), Tick::ZERO));
    }
    s.execute("template.mark", &json!({"clips": [a, b], "slot": null})).unwrap();
    assert!(s.execute("template.fill", &json!({"media": [still]})).is_err());
    let _ = video;
}

#[test]
fn a_parameter_a_command_does_not_take_is_refused_not_dropped() {
    let mut s = Session::default();
    let t = video_track(&s);
    // a misplaced name says what the command takes; a near miss says which one was meant
    let err = s.execute("clip.add", &json!({"track": t, "color": [1, 0, 0], "bogus": 1})).unwrap_err().to_string();
    assert!(err.contains("no parameter `bogus`") && err.contains("`track`"), "{err}");
    let err = s.execute("clip.add", &json!({"track": t, "color": [1, 0, 0], "Duration": 5})).unwrap_err().to_string();
    assert!(err.contains("did you mean `duration`?"), "{err}");
    let err = s.execute("edit.undo", &json!({"steps": 2})).unwrap_err().to_string();
    assert!(err.contains("it takes none"), "{err}");
    assert_eq!(s.revision(), 0, "nothing applied");

    // every parameter each command's schema lists is one it takes
    for c in registry() {
        let schema = input_schema(c.id).expect("every command has a schema");
        for name in schema["properties"].as_object().expect("an object").keys() {
            let r = Session::default().execute(c.id, &json!({ name: null }));
            assert!(!matches!(&r, Err(e) if e.to_string().contains("no parameter")), "{} {name}: {r:?}", c.id);
        }
    }
}

#[test]
fn a_clip_lands_placed_and_a_batch_step_uses_what_an_earlier_one_replied() {
    let mut s = Session::default();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let r = s
        .execute(
            "edit.batch",
            &json!({"steps": [
                {"id": "clip.add", "params": {"track": text, "text": "Hi", "y": -0.3, "scale": 0.5, "opacity": 0.8}},
                {"id": "clip.setAnimation", "params": {"clip": "${0.clip}", "slot": "in", "kind": "pop"}},
                {"id": "clip.duplicate", "params": {"clips": ["${0.clip}"]}},
                {"id": "clip.set", "params": {"clip": "${2.clips.0.clip}", "text": "again"}},
            ]}),
        )
        .unwrap();
    let first = r["results"][0]["clip"].as_str().unwrap();
    let (_, c) = s.project().clip(first).map(|(t, c)| (t, c.clone())).unwrap();
    assert_eq!((c.transform.y, c.transform.scale, c.opacity), (-0.3, 0.5, 0.8));
    assert!(c.anim.enter.is_some());
    assert_eq!(s.history(), ["Batch"]);
    let outline = s.execute("project.outline", &json!({})).unwrap();
    let clips = &outline["tracks"].as_array().unwrap().iter().find(|t| t["id"] == text).unwrap()["clips"];
    assert_eq!(clips[0]["place"]["y"], json!(-0.3));
    assert_eq!(clips[0]["anim"]["in"], json!("pop"));
    assert_eq!((&clips[0]["start"], &clips[0]["end"]), (&json!(0.0), &json!(5.0)));
    assert_eq!(clips[1]["text"], json!("again"));

    // a reference to a step that has not run, or to nothing, fails the whole batch
    let rev = s.revision();
    for bad in ["${5.clip}", "${0.nothing}", "${x}"] {
        let r = s.execute(
            "edit.batch",
            &json!({"steps": [
                {"id": "clip.add", "params": {"track": text, "text": "x"}},
                {"id": "clip.remove", "params": {"clips": [bad]}},
            ]}),
        );
        assert!(r.is_err(), "{bad}");
    }
    assert_eq!(s.revision(), rev);
}

#[test]
fn recognised_lines_become_captions() {
    let mut s = Session::default();
    let out = s
        .execute(
            "captions.add",
            &json!({"cues": [
                {"start": {"seconds": 0.5}, "end": {"seconds": 2.0}, "text": "大家好"},
                {"start": {"seconds": 2.0}, "end": {"seconds": 3.5}, "text": "今天剪一条短视频"}
            ], "template": "variety"}),
        )
        .unwrap();
    let p = s.project();
    let track = p.track(out["track"].as_str().unwrap()).unwrap();
    assert_eq!(track.name, crate::CAPTIONS_TRACK);
    assert_eq!(track.clips.len(), 2);
    assert_eq!(track.clips[1].start, Tick::SECOND * 2);
    let srt = s.execute("captions.export", &json!({})).unwrap();
    assert!(srt["srt"].as_str().unwrap().contains("00:00:00,500 --> 00:00:02,000\n大家好"));
    let bad = s.execute("captions.add", &json!({"cues": [{"start": 10, "end": 5, "text": "x"}]})).unwrap_err();
    assert!(bad.to_string().contains("cues[0]"), "{bad}");
    let missing = s.execute("captions.add", &json!({"cues": [{"start": 0, "text": "x"}]})).unwrap_err();
    assert!(missing.to_string().contains("cues[0]") && missing.to_string().contains("end"), "{missing}");
}

#[test]
fn a_video_is_stabilized_with_a_measured_path_and_how_much_is_changed_alone() {
    let mut s = Session::default();
    let a = add_video(&mut s, 6.0);
    let steady = |s: &Session| s.project().clip(&a).unwrap().1.steady.clone();
    let path: Vec<[f64; 3]> = (0..60).map(|i| [0.01 * (f64::from(i) * 1.9).sin(), 0.0, 0.25]).collect();

    // how much cannot be set before the camera's path is known; a path needs its step
    let e = s.execute("clip.setStabilize", &json!({"clip": a, "amount": 0.5})).unwrap_err().to_string();
    assert!(e.contains("clip.stabilize"), "{e}");
    assert!(s.execute("clip.setStabilize", &json!({"clip": a, "amount": 0.5, "path": path})).is_err());
    let step = json!({"seconds": 1.0 / 30.0});
    let r = s
        .execute(
            "clip.setStabilize",
            &json!({"clip": a, "amount": 0.5, "path": path, "from": {"seconds": 1}, "step": step}),
        )
        .unwrap();
    assert_eq!(r["samples"], 60);
    let set = steady(&s).unwrap();
    assert_eq!((set.amount, set.from, set.path.len()), (0.5, Tick::SECOND, 60));
    assert_eq!(set.path[1], [0.01 * 1.9f64.sin() as f32, 0.0, 0.25]);
    // how much is changed without the path being given again
    s.execute("clip.setStabilize", &json!({"clip": a, "amount": 1})).unwrap();
    assert_eq!((steady(&s).unwrap().amount, steady(&s).unwrap().path), (1.0, set.path.clone()));
    for bad in [
        json!({"amount": 1.5}),
        json!({"amount": 0.5, "path": [[0, 0]], "step": step}),
        json!({"amount": 0.5, "path": [], "step": step}),
        json!({"amount": 0.5, "path": "shaky", "step": step}),
        json!({"amount": 0.5, "path": [[0, 0, 0]], "step": {"seconds": 0}}),
        json!({}),
    ] {
        let mut params = bad.clone();
        params["clip"] = json!(a);
        assert!(s.execute("clip.setStabilize", &params).is_err(), "{bad}");
    }
    assert_eq!(steady(&s).unwrap().path, set.path);
    // it is saved with the clip, and both halves of a cut keep it
    let saved = serde_json::to_value(s.project()).unwrap();
    let back: reelforge_model::Project = serde_json::from_value(saved).unwrap();
    assert_eq!(back.clip(&a).unwrap().1.steady, steady(&s));
    s.execute("clip.split", &json!({"clip": a, "at": {"seconds": 2}})).unwrap();
    let halves: Vec<_> = s.project().tracks[0].clips.iter().map(|c| c.steady.clone()).collect();
    assert_eq!(halves, vec![steady(&s), steady(&s)]);
    s.execute("edit.undo", &json!({})).unwrap();
    // the layer is moved a key a frame, and drawn larger
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let t = &spec["layers"][0]["transform"];
    assert!(t["position"].as_array().is_some_and(|k| k.len() > 100 && k[40]["v"][0] != k[41]["v"][0]));
    // only a video on a video track: not a colour, not a picture
    let red = add_red(&mut s, 10.0, 2.0);
    let still = import(&mut s, "p.png", "image", None);
    let v = video_track(&s);
    let pic = s.execute("clip.add", &json!({"track": v, "media": still, "start": {"seconds": 20}})).unwrap();
    for clip in [red.as_str(), pic["clip"].as_str().unwrap()] {
        let e =
            s.execute("clip.setStabilize", &json!({"clip": clip, "amount": 0.5, "path": [[0, 0, 0]], "step": step}));
        assert!(e.unwrap_err().to_string().contains("only a video"), "{clip}");
    }
    // another file in its place was shot another way: the path goes with the old one
    let other = import(&mut s, "other.mp4", "video", Some(20.0));
    s.execute("clip.replace", &json!({"clip": a, "media": other})).unwrap();
    assert_eq!(steady(&s), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(steady(&s).unwrap().amount, 1.0);
    // 0 takes it off, and undo brings it back
    s.execute("clip.setStabilize", &json!({"clip": a, "amount": 0})).unwrap();
    assert_eq!(steady(&s), None);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["layers"][0]["transform"]["position"], json!([0.0, 0.0]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(steady(&s).unwrap().path.len(), 60);
}

#[test]
fn a_video_is_asked_for_frames_between_its_own() {
    let mut s = Session::default();
    let a = add_video(&mut s, 6.0);
    let frames = |s: &Session, clip: &str| match &s.project().clip(clip).unwrap().1.content {
        ClipContent::Media { frames, .. } => *frames,
        _ => unreachable!(),
    };
    assert_eq!(frames(&s, &a), reelforge_model::Inbetween::Repeat);
    // with the speed, or alone: the length stays as the speed left it
    let r = s.execute("clip.setSpeed", &json!({"clip": a, "speed": 0.5, "frames": "flow"})).unwrap();
    assert_eq!((r["frames"].as_str(), r["duration"].clone()), (Some("flow"), json!(Tick::SECOND * 12)));
    assert_eq!(frames(&s, &a), reelforge_model::Inbetween::Flow);
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["layers"][0]["content"]["between"], "flow");
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("frames.flow/1")));
    s.execute("clip.setSpeed", &json!({"clip": a, "frames": "blend"})).unwrap();
    assert_eq!(s.project().clip(&a).unwrap().1.duration, Tick::SECOND * 12);
    // a speed set later leaves it; so does another file in the clip's place
    s.execute("clip.setSpeed", &json!({"clip": a, "speed": 1})).unwrap();
    let other = import(&mut s, "other.mp4", "video", Some(20.0));
    s.execute("clip.replace", &json!({"clip": a, "media": other})).unwrap();
    assert_eq!(frames(&s, &a), reelforge_model::Inbetween::Blend);
    // it is saved with the clip, and left out of the file while frames show as they are
    let saved = serde_json::to_string(&*s.project()).unwrap();
    assert!(saved.contains(r#""frames":"blend""#), "{saved}");
    s.execute("clip.setSpeed", &json!({"clip": a, "frames": "repeat"})).unwrap();
    assert!(!serde_json::to_string(&*s.project()).unwrap().contains("frames"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frames(&s, &a), reelforge_model::Inbetween::Blend);
    // only a way there is, and only a video: a picture and a sound have no frames to be between
    assert!(s.execute("clip.setSpeed", &json!({"clip": a, "frames": "smooth"})).is_err());
    assert!(s.execute("clip.setSpeed", &json!({"clip": a})).is_err());
    let still = import(&mut s, "p.png", "image", None);
    let v = video_track(&s);
    let pic = s.execute("clip.add", &json!({"track": v, "media": still, "start": {"seconds": 20}})).unwrap();
    let e = s.execute("clip.setSpeed", &json!({"clip": pic["clip"], "frames": "flow"})).unwrap_err().to_string();
    assert!(e.contains("only a video"), "{e}");
}

#[test]
fn shapes_draw_themselves_on_and_titles_count() {
    let mut s = Session::default();
    let video = s.project().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id.clone();
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let add = json!({"track": video, "duration": {"seconds": 2}, "shape": {
        "kind": "rectangle", "size": [600, 200], "radius": 40,
        "fill": {"type": "linear", "from": [1, 0, 0], "to": [1, 0.5, 0, 1], "angle": 90},
        "stroke": {"color": [1, 1, 1, 1], "width": 8}
    }});
    let shape = s.execute("clip.add", &add).unwrap()["clip"].as_str().unwrap().to_string();
    // what a shape cannot be is refused, and so is a shape with something else
    for bad in [json!({"kind": "blob"}), json!({"points": 2}), json!({"trim": [0, 2]}), json!({"fill": "red"})] {
        assert!(s.execute("clip.add", &json!({"track": video, "start": {"seconds": 5}, "shape": bad})).is_err());
    }
    assert!(s.execute("clip.add", &json!({"track": video, "shape": {}, "text": "x"})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("shape/1")));
    let c = &spec["layers"][0]["content"];
    assert_eq!(
        (&c["type"], &c["kind"], &c["size"], &c["radius"]),
        (&json!("shape"), &json!("rectangle"), &json!([600.0, 200.0]), &json!(40.0))
    );
    assert_eq!(c["fill"]["type"], json!("linear"));
    assert!(c.get("trim").is_none() && c.get("fill_opacity").is_none());

    // drawn on: the outline's end rises from 0 to 1, and the fill follows it in
    assert!(
        s.execute(
            "clip.setAnimation",
            &json!({"clip": shape, "slot": "in", "kind": "draw", "duration": {"seconds": 1}})
        )
        .is_ok()
    );
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let c = &spec["layers"][0]["content"];
    let end = c["trim"]["end"].as_array().unwrap();
    assert_eq!((end[0]["v"].as_f64(), end.last().unwrap()["v"].as_f64()), (Some(0.0), Some(1.0)));
    let fill = c["fill_opacity"].as_array().unwrap();
    assert_eq!((fill[0]["v"].as_f64(), fill.last().unwrap()["v"].as_f64()), (Some(0.0), Some(1.0)));
    // the shape itself is not faded by it
    assert_eq!(spec["layers"][0]["opacity"], json!(1.0));
    // a title has no outline to key by hand (it is written on by its own animation)
    let title =
        s.execute("clip.add", &json!({"track": text, "text": "¥{n}", "duration": {"seconds": 2}})).unwrap()["clip"]
            .as_str()
            .unwrap()
            .to_string();

    // its outline keyed by hand, and its values changed
    s.execute("clip.setAnimation", &json!({"clip": shape, "slot": "in", "kind": null})).unwrap();
    s.execute("clip.setKey", &json!({"clip": shape, "prop": "drawEnd", "at": 0, "value": 0.25})).unwrap();
    s.execute("clip.setKey", &json!({"clip": shape, "prop": "drawEnd", "at": {"seconds": 1}, "value": 1})).unwrap();
    assert!(s.execute("clip.setKey", &json!({"clip": title, "prop": "drawEnd", "at": 0, "value": 0.5})).is_err());
    let out = s.execute("clip.setShape", &json!({"clip": shape, "kind": "star", "points": 12, "fill": null})).unwrap();
    assert_eq!((&out["shape"]["kind"], &out["shape"]["points"]), (&json!("star"), &json!(12)));
    assert!(out["shape"].get("fill").is_none());
    assert!(s.execute("clip.setShape", &json!({"clip": title, "kind": "star"})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["layers"][0]["content"]["trim"]["end"][0]["v"], json!(0.25));
    // one colour for it with clip.set
    s.execute("clip.set", &json!({"clip": shape, "color": [0, 0, 1, 1]})).unwrap();
    assert!(
        matches!(&s.project().clip(&shape).unwrap().1.content, ClipContent::Shape { shape } if shape.fill.is_some())
    );

    // a counter: the number runs to its value over a second and is written where {n} is
    let set = json!({"clip": title, "counter": {"from": 0, "to": 1299.5, "over": {"seconds": 1}, "ease": "linear", "decimals": 2, "group": true}});
    s.execute("clip.setCounter", &set).unwrap();
    assert!(s.execute("clip.setCounter", &json!({"clip": shape, "counter": {}})).is_err());
    assert!(s.execute("clip.setCounter", &json!({"clip": title, "counter": {"decimals": 9}})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("text/6")));
    let layers = spec["layers"].as_array().unwrap();
    let n = &layers.iter().find(|l| l["id"] == json!(title)).unwrap()["content"]["number"];
    assert_eq!((&n["decimals"], &n["group"]), (&json!(2), &json!(true)));
    let keys = n["value"].as_array().unwrap();
    let at =
        |secs: f64| keys.iter().find(|k| k["t"] == json!(Tick::from_seconds(secs))).unwrap()["v"].as_f64().unwrap();
    assert!((at(0.5) - 649.75).abs() < 0.01 && at(1.0) == 1299.5 && at(2.0) == 1299.5);
    // keys on `count` say the value instead
    s.execute("clip.setKey", &json!({"clip": title, "prop": "count", "at": 0, "value": 5})).unwrap();
    s.execute("clip.setKey", &json!({"clip": title, "prop": "count", "at": {"seconds": 2}, "value": 0})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layers = spec["layers"].as_array().unwrap();
    let keys = layers.iter().find(|l| l["id"] == json!(title)).unwrap()["content"]["number"]["value"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!((keys[0]["v"].as_f64(), keys.last().unwrap()["v"].as_f64()), (Some(5.0), Some(0.0)));
    s.execute("clip.setCounter", &json!({"clip": title, "counter": null})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(!spec["engine"]["features"].as_array().unwrap().contains(&json!("text/6")));
}

#[test]
fn promo_pieces_are_grouped_clips_on_free_tracks_in_one_step() {
    let mut s = Session::default();
    let listed = s.execute("promo.list", &json!({})).unwrap();
    let kinds: Vec<String> =
        listed["promos"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(kinds.len(), 37);
    // the first seven are a shape and a title each
    let kinds = &kinds[..7];
    let tracks = s.project().tracks.len();
    for kind in kinds {
        let out = s.execute("promo.add", &json!({"kind": kind, "value": 59, "duration": {"seconds": 2}})).unwrap();
        let clips = out["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 2, "{kind}");
        let p = s.project();
        // one group, each part above the one before
        let at: Vec<usize> = clips.iter().map(|c| p.clip(c["clip"].as_str().unwrap()).unwrap().0).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{kind}: {at:?}");
        for c in clips {
            let clip = p.clip(c["clip"].as_str().unwrap()).unwrap().1;
            assert_eq!(clip.group.as_deref(), out["group"].as_str());
            assert!(clip.anim.enter.is_some());
        }
        s.execute("render.compile", &json!({})).unwrap();
    }
    // the first went on the tracks that were there; the rest brought their own
    assert_eq!(s.project().tracks.len(), tracks + 2 * (kinds.len() - 1));
    // one step each
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().tracks.len(), tracks + 2 * (kinds.len() - 2));
    // a price counts up to its value; a countdown runs down a second a second, as a clock
    let mut s = Session::default();
    let out = s.execute("promo.add", &json!({"kind": "priceTag", "value": 129.9, "text": "到手价 ¥{n}"})).unwrap();
    let p = s.project();
    let ClipContent::Text { text, style, .. } = &p.clip(out["clips"][1]["clip"].as_str().unwrap()).unwrap().1.content
    else {
        panic!()
    };
    let c = style.counter.unwrap();
    assert_eq!((text.as_str(), c.to, c.decimals), ("到手价 ¥{n}", 129.9, 2));
    let out = s.execute("promo.add", &json!({"kind": "countdown", "value": 90, "duration": {"seconds": 4}})).unwrap();
    let p = s.project();
    let ClipContent::Text { style, .. } = &p.clip(out["clips"][1]["clip"].as_str().unwrap()).unwrap().1.content else {
        panic!()
    };
    let c = style.counter.unwrap();
    assert_eq!((c.from, c.to, c.clock), (90.0, 86.0, true));
    assert!(s.execute("promo.add", &json!({"kind": "voucher"})).is_err());
    assert!(s.execute("promo.add", &json!({"kind": "banner", "scale": 0})).is_err());
}

#[test]
fn every_ready_made_piece_is_built_as_listed_and_compiles_on_any_canvas() {
    let listed = Session::default().execute("promo.list", &json!({})).unwrap();
    let listed = listed["promos"].as_array().unwrap().clone();
    let categories: std::collections::BTreeSet<&str> = listed.iter().map(|p| p["category"].as_str().unwrap()).collect();
    assert_eq!(categories.into_iter().collect::<Vec<_>>(), ["action", "price", "product", "rhythm", "titles"]);
    for canvas in [[1080, 1920], [1920, 1080], [720, 1280], [2160, 3840], [1080, 1080]] {
        for piece in &listed {
            let kind = piece["id"].as_str().unwrap();
            let mut s = Session::default();
            s.execute("project.setCanvas", &json!({"width": canvas[0], "height": canvas[1]})).unwrap();
            let out = s.execute("promo.add", &json!({"kind": kind, "start": {"seconds": 1}})).unwrap();
            let clips = out["clips"].as_array().unwrap();
            assert_eq!(clips.len() as u64, piece["clips"].as_u64().unwrap(), "{kind}");
            assert_eq!(out["duration"], piece["duration"], "{kind}");
            assert!(!piece["about"].as_str().unwrap().is_empty(), "{kind}");
            let p = s.project();
            p.check().unwrap_or_else(|e| panic!("{kind} on {canvas:?}: {e}"));
            let at: Vec<usize> = clips.iter().map(|c| p.clip(c["clip"].as_str().unwrap()).unwrap().0).collect();
            assert!(at.windows(2).all(|w| w[0] < w[1]), "{kind}: {at:?}");
            for c in clips {
                let clip = p.clip(c["clip"].as_str().unwrap()).unwrap().1;
                assert_eq!(clip.group.as_deref(), out["group"].as_str());
                // each part moves, of itself or with what carries it, and ends with the piece
                assert!(clip.anim.enter.is_some() || !clip.keys.is_empty() || clip.parent.is_some(), "{kind}");
                let long: Tick = serde_json::from_value(out["duration"].clone()).unwrap();
                assert_eq!(clip.end(), Tick::from_seconds(1.0) + long, "{kind}");
                assert!(clip.start >= Tick::from_seconds(1.0));
            }
            s.execute("render.compile", &json!({})).unwrap_or_else(|e| panic!("{kind} on {canvas:?}: {e:?}"));
        }
    }
}

#[test]
fn ready_made_pieces_take_lines_numbers_sides_and_fold_into_one_clip() {
    let mut s = Session::default();
    let text_of = |s: &Session, clip: &Value| match &s.project().clip(clip.as_str().unwrap()).unwrap().1.content {
        ClipContent::Text { text, .. } => text.clone(),
        _ => String::new(),
    };
    // a list has a row a line, under its plate
    let out = s.execute("promo.add", &json!({"kind": "checklist", "lines": ["轻", "快"]})).unwrap();
    let clips = out["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 3);
    assert_eq!((text_of(&s, &clips[1]["clip"]), text_of(&s, &clips[2]["clip"])), ("✓  轻".into(), "✓  快".into()));
    // its rows come one after another, from the left of one box
    let p = s.project();
    let (a, b) =
        (p.clip(clips[1]["clip"].as_str().unwrap()).unwrap().1, p.clip(clips[2]["clip"].as_str().unwrap()).unwrap().1);
    assert!(a.start < b.start && a.end() == b.end());
    let ClipContent::Text { style, .. } = &a.content else { panic!() };
    assert_eq!((style.align, style.wrap), (reelforge_model::TitleAlign::Left, 730.0 / 1080.0));
    assert!(s.execute("promo.add", &json!({"kind": "checklist", "lines": []})).is_err());
    assert!(
        s.execute("promo.add", &json!({"kind": "checklist", "lines": ["1", "2", "3", "4", "5", "6", "7"]})).is_err()
    );
    assert!(s.execute("promo.add", &json!({"kind": "checklist", "rows": ["1"]})).is_err());

    // two prices: the bars are in proportion and the saving is counted up to
    let out = s
        .execute("promo.add", &json!({"kind": "priceCompare", "values": [500, 350], "lines": ["别家", "我们"]}))
        .unwrap();
    let clips = out["clips"].as_array().unwrap();
    let p = s.project();
    let ClipContent::Text { style, text, .. } =
        &p.clip(clips.last().unwrap()["clip"].as_str().unwrap()).unwrap().1.content
    else {
        panic!()
    };
    assert_eq!((text.as_str(), style.counter.unwrap().to), ("立省 ¥{n}", 150.0));
    let bar = |i: usize| match &p.clip(clips[i]["clip"].as_str().unwrap()).unwrap().1.content {
        ClipContent::Shape { shape } => shape.size[0],
        _ => panic!(),
    };
    assert_eq!((bar(1), bar(4)), (520.0, 364.0));
    assert_eq!(text_of(&s, &clips[2]["clip"]), "¥500");
    assert!(s.execute("promo.add", &json!({"kind": "priceCompare", "values": [1, 2, 3, 4, 5]})).is_err());
    assert!(s.execute("promo.add", &json!({"kind": "priceCompare", "values": ["a"]})).is_err());

    // a piece that points does so the way it is told; each has a place of its own unless given one
    let out = s.execute("promo.add", &json!({"kind": "arrowHint", "side": "left"})).unwrap();
    let arrow = s.project().clip(out["clips"][0]["clip"].as_str().unwrap()).unwrap().1.clone();
    assert_eq!(arrow.transform.rotation, 180.0);
    assert!((arrow.transform.y - 0.3).abs() < 1e-6);
    // and nudges that way, round after round, from where it sits and back to it
    let nudge = &arrow.keys[&Prop::X];
    assert_eq!((nudge[0].v, nudge.last().unwrap().v), (arrow.transform.x, arrow.transform.x));
    assert!(nudge.len() == 9 && nudge[1].v < arrow.transform.x);
    assert!(s.execute("promo.add", &json!({"kind": "arrowHint", "side": "sideways"})).is_err());
    // a label leans away from the middle of the frame, but not off its edge
    let label_x = |s: &mut Session, params: Value| {
        let out = s.execute("promo.add", &params).unwrap();
        s.project().clip(out["clips"][3]["clip"].as_str().unwrap()).unwrap().1.transform.x
    };
    assert!(label_x(&mut s, json!({"kind": "callout", "x": 0.0, "text": "轻"})) > 0.0);
    assert!(label_x(&mut s, json!({"kind": "callout", "x": -0.2, "text": "轻"})) < -0.2);
    assert!(label_x(&mut s, json!({"kind": "callout", "x": -0.2, "text": "一段很长很长的卖点说明"})) > -0.2);
    assert!(label_x(&mut s, json!({"kind": "callout", "x": 0.3, "text": "一段很长很长的卖点说明"})) < 0.3);
    assert!(label_x(&mut s, json!({"kind": "callout", "x": -0.2, "side": "right", "text": "轻"})) > -0.2);
    let out = s.execute("promo.add", &json!({"kind": "buyButton", "y": -0.2})).unwrap();
    assert_eq!(s.project().clip(out["clips"][0]["clip"].as_str().unwrap()).unwrap().1.transform.y, -0.2);

    // a sweep is three bands crossing the frame, covering it at its middle
    let out = s.execute("promo.add", &json!({"kind": "colorSweep", "side": "down", "start": {"seconds": 5}})).unwrap();
    assert_eq!(out["duration"], json!(Tick::from_seconds(0.9)));
    let p = s.project();
    for c in out["clips"].as_array().unwrap() {
        let clip = p.clip(c["clip"].as_str().unwrap()).unwrap().1;
        let keys = &clip.keys[&Prop::Y];
        assert!((keys[0].v + 1.02).abs() < 1e-5 && (keys.last().unwrap().v - 1.02).abs() < 1e-5);
        assert!(keys.windows(2).all(|w| w[0].t < w[1].t));
        assert!(clip.value_at(Prop::Y, Tick(clip.duration.0 / 2)).abs() < 0.2);
        let ClipContent::Shape { shape } = &clip.content else { panic!() };
        assert_eq!(shape.size, [1084.0, 1924.0]);
    }

    // folded, a piece is one clip with its parts inside, made and unmade in one step
    let mut s = Session::default();
    let tracks = s.project().tracks.len();
    let out = s.execute("promo.add", &json!({"kind": "specTable", "compound": true, "start": {"seconds": 2}})).unwrap();
    let p = s.project();
    let clip = p.clip(out["clip"].as_str().unwrap()).unwrap().1;
    assert_eq!((clip.start, clip.duration), (Tick::from_seconds(2.0), Tick::from_seconds(3.0)));
    let inner = p.tracks_of(clip.compound().unwrap().0).unwrap();
    assert_eq!(inner.iter().map(|t| t.clips.len()).sum::<usize>(), 7);
    assert_eq!(out["parts"].as_array().unwrap().len(), 7);
    assert_eq!(p.timelines.iter().find(|t| t.compound).unwrap().name, "Spec table");
    assert_eq!(p.tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 1);
    s.execute("render.compile", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let p = s.project();
    assert_eq!((p.tracks.len(), p.tracks.iter().map(|t| t.clips.len()).sum::<usize>()), (tracks, 0));
    assert!(p.timelines.iter().all(|t| !t.compound));
    // one that cannot be made leaves nothing behind
    assert!(s.execute("promo.add", &json!({"kind": "tiers", "compound": true, "lines": []})).is_err());
    assert_eq!(s.project().tracks.len(), tracks);
}

#[test]
fn a_folded_piece_is_given_other_words_and_made_again_to_fit() {
    let mut s = Session::default();
    let shape_width = |s: &Session, timeline: &str| {
        let p = s.project();
        let clips = p.tracks_of(timeline).unwrap().iter().flat_map(|t| &t.clips);
        clips
            .filter_map(|c| if let ClipContent::Shape { shape } = &c.content { Some(shape.size[0]) } else { None })
            .next()
    };
    let words = |s: &Session, timeline: &str| -> Vec<String> {
        let p = s.project();
        let clips = p.tracks_of(timeline).unwrap().iter().flat_map(|t| &t.clips);
        clips
            .filter_map(|c| if let ClipContent::Text { text, .. } = &c.content { Some(text.clone()) } else { None })
            .collect()
    };
    let out = s
        .execute(
            "promo.add",
            &json!({"kind": "buyButton", "compound": true, "start": {"seconds": 2}, "color": [0, 0.5, 1]}),
        )
        .unwrap();
    let (clip, timeline) = (out["clip"].as_str().unwrap().to_string(), out["timeline"].as_str().unwrap().to_string());
    let narrow = shape_width(&s, &timeline).unwrap();
    // what it was made as is kept, and read back
    let got = s.execute("promo.get", &json!({"clip": clip})).unwrap();
    assert_eq!((got["piece"]["kind"].as_str(), got["piece"]["text"].as_str()), (Some("buyButton"), Some("立即抢购")));
    assert_eq!(got["piece"]["color"], json!([0.0, 0.5, 1.0, 1.0]));
    // the clip is moved and set like any other; then the piece is given longer words
    s.execute("clip.set", &json!({"clip": clip, "y": -0.1, "opacity": 0.8})).unwrap();
    let out = s.execute("promo.set", &json!({"clip": clip, "text": "现在就去店里看看吧"})).unwrap();
    assert_eq!(out["piece"]["text"], json!("现在就去店里看看吧"));
    // the button is made again as wide as its words, in the colour it had; the clip is as it was set
    assert_eq!(words(&s, &timeline), ["现在就去店里看看吧"]);
    assert!(shape_width(&s, &timeline).unwrap() > narrow + 100.0);
    let p = s.project();
    let (_, outer) = p.clip(&clip).unwrap();
    assert_eq!((outer.start, outer.transform.y, outer.opacity), (Tick::from_seconds(2.0), -0.1, 0.8));
    let ClipContent::Shape { shape } = &p.tracks_of(&timeline).unwrap()[0].clips[0].content else { panic!() };
    assert!(matches!(shape.fill, Some(Fill::Linear { to, .. }) if to == [0.0, 0.5, 1.0, 1.0]));
    s.execute("render.compile", &json!({})).unwrap();
    // one undo step
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(words(&s, &timeline), ["立即抢购"]);

    // rows, numbers and sides are given the same way; what is not named stays
    let out = s.execute("promo.add", &json!({"kind": "checklist", "compound": true, "lines": ["轻", "快"]})).unwrap();
    let (list, inner) = (out["clip"].as_str().unwrap().to_string(), out["timeline"].as_str().unwrap().to_string());
    s.execute("promo.set", &json!({"clip": list, "lines": ["轻", "快", "稳"], "scale": 0.8})).unwrap();
    assert_eq!(words(&s, &inner), ["✓  轻", "✓  快", "✓  稳"]);
    s.execute("promo.set", &json!({"clip": list, "color": [0, 0, 0]})).unwrap();
    assert_eq!(words(&s, &inner).len(), 3);
    assert_eq!(crate::made_of(&s.project(), &list).unwrap().1.scale, 0.8);
    let out = s.execute("promo.add", &json!({"kind": "flashProgress", "compound": true})).unwrap();
    let (bar, inner) = (out["clip"].as_str().unwrap().to_string(), out["timeline"].as_str().unwrap().to_string());
    s.execute("promo.set", &json!({"clip": bar, "value": 40})).unwrap();
    let p = s.project();
    let ClipContent::Shape { shape } = &p.tracks_of(&inner).unwrap()[1].clips[0].content else { panic!() };
    assert_eq!(shape.trim, [0.0, 0.4]);
    // it is refused what the piece cannot be, and nothing changes
    assert!(s.execute("promo.set", &json!({"clip": list, "lines": []})).is_err());
    assert!(s.execute("promo.set", &json!({"clip": list, "side": "inwards"})).is_err());
    assert!(s.execute("promo.set", &json!({"clip": list, "kind": "banner"})).is_err());
    assert!(s.execute("promo.set", &json!({"clip": list, "scale": 0})).is_err());
    // a piece left as grouped clips, or any other clip, is not one to rewrite
    let grouped = s.execute("promo.add", &json!({"kind": "banner"})).unwrap();
    let part = grouped["clips"][0]["clip"].clone();
    assert!(s.execute("promo.set", &json!({"clip": part, "text": "x"})).is_err());
    assert_eq!(s.execute("promo.get", &json!({"clip": part})).unwrap()["piece"], Value::Null);
    assert!(s.execute("promo.get", &json!({"clip": "c999"})).is_err());

    // a second clip playing the same piece changes with the first, also after the piece's own
    // timeline was opened and left; saved and opened, it is still a piece
    let copy = s.execute("clip.duplicate", &json!({"clips": [clip]})).unwrap();
    let copy = copy["clips"][0]["clip"].as_str().unwrap().to_string();
    s.execute("timeline.switch", &json!({"timeline": timeline})).unwrap();
    let main = s.project().timelines.iter().find(|t| !t.compound).unwrap().id.clone();
    s.execute("timeline.switch", &json!({"timeline": main})).unwrap();
    s.execute("promo.set", &json!({"clip": copy, "text": "去看看"})).unwrap();
    assert_eq!(words(&s, &timeline), ["去看看"]);
    let again = Project::from_json(&s.project().to_json()).unwrap();
    again.check().unwrap();
    assert_eq!(crate::made_of(&again, &clip).unwrap().1.text, "去看看");
    // taken apart, it is ordinary clips again
    s.execute("clip.remove", &json!({"clips": [copy]})).unwrap();
    s.execute("clip.uncompound", &json!({"clip": clip})).unwrap();
    assert!(s.project().timelines.iter().all(|t| t.id != timeline));
    s.project().check().unwrap();
}

#[test]
fn the_brand_gives_new_pieces_their_colours_font_and_logo() {
    let mut s = Session::default();
    assert_eq!(s.execute("brand.get", &json!({})).unwrap(), json!({"brand": {}}));
    let fill_of = |s: &Session, out: &Value| match &s
        .project()
        .clip(out["clips"][0]["clip"].as_str().unwrap())
        .unwrap()
        .1
        .content
    {
        ClipContent::Shape { shape } => shape.fill.clone(),
        _ => None,
    };
    let red = s.execute("promo.add", &json!({"kind": "banner"})).unwrap();
    let blue = [0.0, 0.4, 1.0, 1.0];
    let cream = [1.0, 0.95, 0.8, 1.0];
    let out = s.execute("brand.set", &json!({"primary": [0.0, 0.4, 1.0], "secondary": cream})).unwrap();
    assert_eq!(out["brand"]["primary"], json!(blue));
    // pieces made from then on take after it; those made before keep their look; a colour said wins
    let made = s.execute("promo.add", &json!({"kind": "banner"})).unwrap();
    assert!(matches!(fill_of(&s, &made), Some(Fill::Linear { to, .. }) if to == blue));
    assert!(matches!(fill_of(&s, &red), Some(Fill::Linear { to, .. }) if to != blue));
    let green = s.execute("promo.add", &json!({"kind": "banner", "color": [0.0, 0.6, 0.2]})).unwrap();
    assert!(matches!(fill_of(&s, &green), Some(Fill::Linear { to, .. }) if to == [0.0, 0.6, 0.2, 1.0]));
    // words over a light colour are dark
    let tiers = s.execute("promo.add", &json!({"kind": "tiers"})).unwrap();
    let p = s.project();
    let ClipContent::Text { color, .. } = &p.clip(tiers["clips"][5]["clip"].as_str().unwrap()).unwrap().1.content
    else {
        panic!()
    };
    assert!(color[0] < 0.2, "ink on cream");

    // a font and a logo are media of the project, of the right kind
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../render/fonts/NotoSansSC-Bold.subset.ttf");
    let font = s.execute("media.import", &json!({"path": file})).unwrap()["media"]["id"].as_str().unwrap().to_string();
    let v = video_track(&s);
    let sticker = s.execute("clip.add", &json!({"track": v, "sticker": "heart"})).unwrap();
    let logo = match &s.project().clip(sticker["clip"].as_str().unwrap()).unwrap().1.content {
        ClipContent::Media { media, .. } => media.clone(),
        _ => panic!(),
    };
    s.execute("clip.remove", &json!({"clips": sticker["clip"]})).unwrap();
    assert!(s.execute("brand.set", &json!({"font": logo})).is_err());
    assert!(s.execute("brand.set", &json!({"logo": font})).is_err());
    assert!(s.execute("brand.set", &json!({"logo": "m99"})).is_err());
    assert!(s.execute("brand.set", &json!({"primary": [2, 0, 0]})).is_err());
    assert!(s.execute("brand.set", &json!({"colour": [1, 0, 0]})).is_err());
    let out = s.execute("brand.set", &json!({"font": font, "logo": logo})).unwrap();
    assert_eq!((out["brand"]["font"].as_str(), out["brand"]["primary"].clone()), (Some(font.as_str()), json!(blue)));
    // an end card shows the logo over its words, which are set in the brand's font
    let card = s.execute("promo.add", &json!({"kind": "endCard"})).unwrap();
    let clips = card["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 4);
    let p = s.project();
    let mark = p.clip(clips[0]["clip"].as_str().unwrap()).unwrap().1;
    assert!(matches!(&mark.content, ClipContent::Media { media, .. } if *media == logo));
    assert!((mark.transform.scale - 0.3).abs() < 1e-6 && mark.transform.y < 0.0);
    let ClipContent::Text { style, .. } = &p.clip(clips[2]["clip"].as_str().unwrap()).unwrap().1.content else {
        panic!()
    };
    assert_eq!(style.font.as_deref(), Some(font.as_str()));
    s.execute("render.compile", &json!({})).unwrap();
    // it survives saving, and is one undo step; null clears a part of it
    let again = Project::from_json(&s.project().to_json()).unwrap();
    assert_eq!(again.brand, s.project().brand);
    s.execute("brand.set", &json!({"secondary": null})).unwrap();
    assert_eq!(s.project().brand.secondary, None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().brand.secondary, Some(cream));
    // media the brand holds can go only once nothing uses it, and the brand lets go of it then
    assert!(s.execute("media.remove", &json!({"media": logo})).is_err());
    s.execute("clip.remove", &json!({"clips": clips[0]["clip"]})).unwrap();
    s.execute("media.remove", &json!({"media": logo})).unwrap();
    assert_eq!(s.project().brand.logo, None);
    s.project().check().unwrap();
}

#[test]
fn a_piece_carries_its_parts_and_shows_a_picture() {
    let mut s = Session::default();
    let (cw, ch) = (s.project().canvas.width as f32, s.project().canvas.height as f32);
    let k = cw.min(ch) / 1080.0;
    // everything on a product card is carried by the card, the button's words by the button
    let out = s.execute("promo.add", &json!({"kind": "productCard", "start": {"seconds": 1}})).unwrap();
    let ids: Vec<String> =
        out["clips"].as_array().unwrap().iter().map(|c| c["clip"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 6);
    let p = s.project();
    let part = |i: usize| p.clip(&ids[i]).unwrap().1;
    let card = part(0);
    assert_eq!(card.parent, None);
    assert_eq!(card.anim.enter.map(|a| a.kind), Some(AnimKind::FlipY));
    assert!(card.transform.motion_blur > 0.0);
    for i in 1..5 {
        assert_eq!(part(i).parent.as_deref(), Some(ids[0].as_str()), "part {i}");
    }
    assert_eq!(part(5).parent.as_deref(), Some(ids[4].as_str()));
    // a carried part's place is counted from its carrier's, in shares of the frame
    let name = part(2);
    assert!(name.transform.x.abs() < 1e-6 && (name.transform.y - 130.0 * k / ch).abs() < 1e-6);
    assert_eq!(part(5).transform.y, 0.0);
    s.execute("render.compile", &json!({})).unwrap();
    s.project().check().unwrap();
    // made half as large, the card is: what it carries keeps its share of it
    let small = s.execute("promo.add", &json!({"kind": "productCard", "scale": 0.5, "x": 0.2})).unwrap();
    let p = s.project();
    let small = |i: usize| p.clip(small["clips"][i]["clip"].as_str().unwrap()).unwrap().1;
    assert!((small(0).transform.scale - 0.5 * k).abs() < 1e-6 && (small(0).transform.x - 0.2).abs() < 1e-6);
    assert!((small(2).transform.scale - 1.0).abs() < 1e-6, "{}", small(2).transform.scale);
    assert!((small(2).transform.y - 130.0 * k / ch).abs() < 1e-6 && small(2).transform.x.abs() < 1e-6);

    // a card that flips: each face is carried words and all, and one shows while the other is away
    let flip = s.execute("promo.add", &json!({"kind": "flipCard"})).unwrap();
    let p = s.project();
    let face = |i: usize| p.clip(flip["clips"][i]["clip"].as_str().unwrap()).unwrap().1;
    assert_eq!(flip["clips"].as_array().unwrap().len(), 4);
    assert_eq!(face(1).parent.as_deref(), flip["clips"][0]["clip"].as_str());
    assert_eq!(face(3).parent.as_deref(), flip["clips"][2]["clip"].as_str());
    for (front, back) in [(0, 2), (1, 3)] {
        let (seen, hidden) = (&face(front).keys[&Prop::Opacity], &face(back).keys[&Prop::Opacity]);
        assert_eq!((seen[0].v, seen[1].v, hidden[0].v, hidden[1].v), (1.0, 0.0, 0.0, 1.0));
        assert_eq!(seen[1].t, hidden[1].t);
    }
    let turn = &face(0).keys[&Prop::RotationY];
    assert_eq!((turn[0].v, turn.last().unwrap().v), (0.0, 90.0));
    assert_eq!(face(2).keys[&Prop::RotationY].last().unwrap().v, 0.0);

    // the picture a card shows is an image or video of the project
    let v = video_track(&s);
    let sticker = s.execute("clip.add", &json!({"track": v, "sticker": "heart"})).unwrap();
    let picture = match &s.project().clip(sticker["clip"].as_str().unwrap()).unwrap().1.content {
        ClipContent::Media { media, .. } => media.clone(),
        _ => panic!(),
    };
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../render/fonts/NotoSansSC-Bold.subset.ttf");
    let font = s.execute("media.import", &json!({"path": file})).unwrap()["media"]["id"].as_str().unwrap().to_string();
    assert!(s.execute("promo.add", &json!({"kind": "productCard", "media": "m99"})).is_err());
    assert!(s.execute("promo.add", &json!({"kind": "productCard", "media": font})).is_err());
    assert!(s.execute("promo.add", &json!({"kind": "productCard", "media": 3})).is_err());
    let out = s.execute("promo.add", &json!({"kind": "productCard", "media": picture, "compound": true})).unwrap();
    let (clip, timeline) = (out["clip"].as_str().unwrap().to_string(), out["timeline"].as_str().unwrap().to_string());
    let shown = |s: &Session| {
        let p = s.project();
        let inner = p.tracks_of(&timeline).unwrap();
        // inside, each part is carried by one of the same timeline
        let ids: Vec<&str> = inner.iter().flat_map(|t| &t.clips).map(|c| c.id.as_str()).collect();
        assert!(inner.iter().flat_map(|t| &t.clips).skip(1).all(|c| ids.contains(&c.parent.as_deref().unwrap())));
        let second = &inner[1].clips[0];
        match &second.content {
            ClipContent::Media { media, .. } => Some((media.clone(), second.transform.scale)),
            _ => None,
        }
    };
    assert_eq!(shown(&s), Some((picture.clone(), 0.5 / k)));
    assert_eq!(s.execute("promo.get", &json!({"clip": clip})).unwrap()["piece"]["media"], json!(picture));
    s.execute("render.compile", &json!({})).unwrap();
    // it is taken off and put back like the piece's words; other changes leave it be
    s.execute("promo.set", &json!({"clip": clip, "media": null})).unwrap();
    assert_eq!(shown(&s), None);
    assert!(s.execute("promo.set", &json!({"clip": clip, "media": font})).is_err());
    assert!(s.execute("promo.set", &json!({"clip": clip, "media": "m99"})).is_err());
    s.execute("promo.set", &json!({"clip": clip, "media": picture})).unwrap();
    s.execute("promo.set", &json!({"clip": clip, "text": "随行杯"})).unwrap();
    assert_eq!(shown(&s).map(|(m, _)| m), Some(picture));
    s.execute("render.compile", &json!({})).unwrap();
    s.project().check().unwrap();
}

#[test]
fn every_piece_is_made_folded_and_compiles() {
    let listed = Session::default().execute("promo.list", &json!({})).unwrap();
    let mut s = Session::default();
    for (i, promo) in listed["promos"].as_array().unwrap().iter().enumerate() {
        let start = json!({"seconds": i as f64 * 5.0});
        let out = s.execute("promo.add", &json!({"kind": promo["id"], "compound": true, "start": start})).unwrap();
        assert_eq!(out["parts"].as_array().unwrap().len() as u64, promo["clips"].as_u64().unwrap(), "{}", promo["id"]);
    }
    s.execute("render.compile", &json!({})).unwrap();
    s.project().check().unwrap();
}

/// `n` picture clips at the same time on tracks one above another, the first lowest: pictures
/// `sizes` across and high.
fn stacked_pictures(s: &mut Session, sizes: &[[u32; 2]]) -> Vec<String> {
    let mut clips = vec![];
    for (i, size) in sizes.iter().enumerate() {
        let params = json!({
            "path": format!("/nowhere/{i}.mp4"), "duration": {"seconds": 10}, "size": size,
            "sha256": format!("{i:02x}").repeat(32),
        });
        let m = s.execute("media.import", &params).unwrap()["media"]["id"].as_str().unwrap().to_string();
        let track = match i {
            0 => video_track(s),
            _ => s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string(),
        };
        let c = s.execute("clip.add", &json!({"track": track, "media": m, "duration": {"seconds": 4}})).unwrap();
        clips.push(c["clip"].as_str().unwrap().to_string());
    }
    clips
}

#[test]
fn a_layout_gives_each_clip_a_cell_it_fills() {
    let listed = Session::default().execute("layout.list", &json!({})).unwrap();
    let ids: Vec<&str> = listed["layouts"].as_array().unwrap().iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["columns", "rows", "grid", "focus", "pip", "compare"]);

    // where each picture shows on the canvas: the middle and the size of what its crop leaves,
    // fitted inside the frame and scaled, in pixels
    let shown = |s: &Session, id: &str| -> [f32; 4] {
        let p = s.project();
        let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
        let clip = p.clip(id).unwrap().1;
        let ClipContent::Media { media, .. } = &clip.content else { panic!() };
        let [w, h] = p.media[media].size.unwrap().map(|v| v as f32);
        let c = clip.look.crop;
        let (vw, vh) = (w * (1.0 - c.left - c.right), h * (1.0 - c.top - c.bottom));
        let fit = (cw / vw).min(ch / vh) * clip.transform.scale;
        [(0.5 + clip.transform.x) * cw, (0.5 + clip.transform.y) * ch, vw * fit, vh * fit]
    };
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.6);

    // two columns on the default upright frame: a wide picture and a tall one, each cut to its half
    let mut s = Session::default();
    let (cw, ch) = (s.project().canvas.width as f32, s.project().canvas.height as f32);
    let clips = stacked_pictures(&mut s, &[[1920, 1080], [1080, 1920], [1000, 1000], [640, 480]]);
    let gap = 0.012 * cw.min(ch);
    let out = s.execute("layout.apply", &json!({"layout": "columns", "clips": clips[..2]})).unwrap();
    let half = (cw - gap) / 2.0;
    let (left, right) = (shown(&s, &clips[0]), shown(&s, &clips[1]));
    assert!(near(left, [half / 2.0, ch / 2.0, half, ch]), "{left:?}");
    assert!(near(right, [cw - half / 2.0, ch / 2.0, half, ch]), "{right:?}");
    assert_eq!(out["clips"][0]["width"], json!(half.round()));
    // the wide one lost its sides, evenly; nothing is squashed
    let crop = s.project().clip(&clips[0]).unwrap().1.look.crop;
    assert!(crop.left > 0.3 && crop.left == crop.right && crop.top == 0.0);
    s.execute("render.compile", &json!({})).unwrap();
    // one step
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().clip(&clips[0]).unwrap().1.look.crop.is_zero());

    // every layout tiles its part of the frame without overlap, on upright and wide frames
    for canvas in [[1080, 1920], [1920, 1080], [1080, 1080]] {
        s.execute("project.setCanvas", &json!({"width": canvas[0], "height": canvas[1]})).unwrap();
        let (cw, ch) = (canvas[0] as f32, canvas[1] as f32);
        for (layout, n) in [("columns", 4), ("rows", 3), ("grid", 3), ("grid", 4), ("focus", 2), ("focus", 4)] {
            let asked = json!({"layout": layout, "clips": clips[..n], "gap": 0, "margin": 0.05});
            s.execute("layout.apply", &asked).unwrap_or_else(|e| panic!("{layout} of {n}: {e}"));
            let cells: Vec<[f32; 4]> = clips[..n].iter().map(|c| shown(&s, c)).collect();
            let m = 0.05 * cw.min(ch);
            let area: f32 = cells.iter().map(|c| c[2] * c[3]).sum();
            assert!((area - (cw - 2.0 * m) * (ch - 2.0 * m)).abs() < 0.002 * cw * ch, "{layout} of {n}: {cells:?}");
            for (i, a) in cells.iter().enumerate() {
                assert!(a[0] - a[2] / 2.0 > m - 1.0 && a[0] + a[2] / 2.0 < cw - m + 1.0, "{layout} of {n}: {a:?}");
                assert!(a[1] - a[3] / 2.0 > m - 1.0 && a[1] + a[3] / 2.0 < ch - m + 1.0, "{layout} of {n}: {a:?}");
                for b in &cells[..i] {
                    let apart = (a[0] - b[0]).abs() > (a[2] + b[2]) / 2.0 - 1.0
                        || (a[1] - b[1]).abs() > (a[3] + b[3]) / 2.0 - 1.0;
                    assert!(apart, "{layout} of {n} on {canvas:?}: {a:?} meets {b:?}");
                }
            }
            s.execute("render.compile", &json!({})).unwrap();
        }
    }
    // the first of `focus` is twice the strip beside it
    let (large, small) = (shown(&s, &clips[0]), shown(&s, &clips[1]));
    assert!((large[3] / small[3] - 2.0).abs() < 0.01, "{large:?} {small:?}");

    // what a layout cannot be given
    assert!(s.execute("layout.apply", &json!({"layout": "spiral", "clips": clips[..2]})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "columns", "clips": clips[..1]})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "compare", "clips": clips[..3]})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "rows", "clips": [clips[0], clips[0]]})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "rows", "clips": [clips[0], "c999"]})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "rows", "clips": clips[..2], "gap": 0.5})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "pip", "clips": clips[..2], "corner": "middle"})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "pip", "clips": clips[..2], "border": 4})).is_err());
    let title = s.execute("clip.add", &json!({"track": "t2", "text": "hi"})).unwrap()["clip"].clone();
    assert!(s.execute("layout.apply", &json!({"layout": "rows", "clips": [clips[0], title]})).is_err());
}

#[test]
fn small_pictures_and_a_comparison_are_drawn_over_the_first_clip() {
    let mut s = Session::default();
    let (cw, ch) = (s.project().canvas.width as f32, s.project().canvas.height as f32);
    let clips = stacked_pictures(&mut s, &[[1080, 1920], [1920, 1080], [1920, 1080]]);
    let shapes = |s: &Session| -> usize {
        let p = s.project();
        p.tracks.iter().flat_map(|t| &t.clips).filter(|c| matches!(c.content, ClipContent::Shape { .. })).count()
    };

    // picture in picture: the first fills the frame, the others sit in the corner, outlined
    let out = s.execute("layout.apply", &json!({"layout": "pip", "clips": clips})).unwrap();
    let p = s.project();
    let (main, small) = (p.clip(&clips[0]).unwrap().1, p.clip(&clips[1]).unwrap().1);
    assert_eq!((main.transform.x, main.transform.y, main.transform.scale), (0.0, 0.0, 1.0));
    assert!(main.look.crop.is_zero());
    assert!((small.transform.scale - 0.32).abs() < 1e-6 && small.transform.x > 0.25 && small.transform.y < -0.25);
    // the second is under it, a gap away
    let next = p.clip(&clips[2]).unwrap().1;
    assert!((next.transform.y - small.transform.y - 0.32 - 0.012 * cw / ch).abs() < 1e-4);
    // each outline is a shape its picture carries, as large as its cell whatever the picture's scale
    let added = out["added"].as_array().unwrap();
    assert_eq!((added.len(), shapes(&s)), (2, 2));
    let (ti, edge) = p.clip(added[0].as_str().unwrap()).unwrap();
    assert!(ti > p.clip(&clips[1]).unwrap().0);
    assert_eq!(edge.parent.as_deref(), Some(clips[1].as_str()));
    assert_eq!((edge.start, edge.duration), (small.start, small.duration));
    assert!((edge.transform.scale * small.transform.scale - 1.0).abs() < 1e-5);
    let ClipContent::Shape { shape } = &edge.content else { panic!() };
    assert!(shape.fill.is_none() && shape.stroke.is_some());
    assert!((shape.size[0] - 0.32 * cw).abs() < 0.5 && (shape.size[1] - 0.32 * ch).abs() < 0.5);
    s.execute("render.compile", &json!({})).unwrap();

    // again as circles in another corner: the old outlines are replaced, not added to
    let asked = json!({"layout": "pip", "clips": clips, "round": true, "corner": "bottomLeft", "size": 0.25});
    let out = s.execute("layout.apply", &asked).unwrap();
    assert_eq!((out["added"].as_array().unwrap().len(), shapes(&s)), (2, 2));
    let p = s.project();
    let small = p.clip(&clips[1]).unwrap().1;
    assert!(small.transform.x < -0.25 && small.transform.y > 0.25);
    let mask = small.look.mask.unwrap();
    assert_eq!((mask.kind, mask.size, mask.feather), (reelforge_model::MaskKind::Circle, [1.0, 1.0], 0.0));
    // what shows is square
    let c = small.look.crop;
    assert!(((1920.0 * (1.0 - c.left - c.right)) - 1080.0 * (1.0 - c.top - c.bottom)).abs() < 1.0);
    // without outlines, and the circles are gone from a layout that has none
    s.execute("layout.apply", &json!({"layout": "pip", "clips": clips, "border": null})).unwrap();
    assert_eq!(shapes(&s), 0);
    assert!(s.project().clip(&clips[1]).unwrap().1.look.mask.is_none());
    // a small picture under the large one would not show
    let e = s.execute("layout.apply", &json!({"layout": "pip", "clips": [clips[1], clips[0]]})).unwrap_err();
    assert!(e.to_string().contains("track above"), "{e}");

    // a comparison: the upper clip shows past a line that travels, and a divider travels with it
    let out = s.execute("layout.apply", &json!({"layout": "compare", "clips": clips[..2]})).unwrap();
    let p = s.project();
    let upper = p.clip(&clips[1]).unwrap().1;
    let mask = upper.look.mask.unwrap();
    assert_eq!((mask.kind, mask.rotation, mask.feather), (reelforge_model::MaskKind::Linear, 90.0, 0.0));
    let wipe = &upper.keys[&Prop::MaskX];
    assert_eq!((wipe[0].v, wipe[1].v), (0.85, 0.15));
    assert!(wipe[0].t > Tick::ZERO && wipe[1].t < upper.duration);
    let added: Vec<&str> = out["added"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(added.len(), 4);
    let line = p.clip(added[0]).unwrap().1;
    assert_eq!(line.parent.as_deref(), Some(clips[1].as_str()));
    let travel = &line.keys[&Prop::X];
    assert_eq!((travel[0].t, travel[1].t), (wipe[0].t, wipe[1].t));
    assert!((travel[0].v - 0.35).abs() < 1e-6 && (travel[1].v + 0.35).abs() < 1e-6);
    for part in &added[1..] {
        let part = p.clip(part).unwrap().1;
        assert_eq!((part.parent.as_deref(), part.group.as_ref()), (Some(added[0]), line.group.as_ref()));
    }
    s.execute("render.compile", &json!({})).unwrap();
    s.project().check().unwrap();
    // down the frame instead, without the line; the old divider goes
    let asked =
        json!({"layout": "compare", "clips": clips[..2], "axis": "y", "from": 0.3, "to": 0.7, "divider": false});
    s.execute("layout.apply", &asked).unwrap();
    let p = s.project();
    let upper = p.clip(&clips[1]).unwrap().1;
    assert_eq!(upper.look.mask.unwrap().rotation, 180.0);
    assert!(upper.keys.contains_key(&Prop::MaskY) && !upper.keys.contains_key(&Prop::MaskX));
    assert_eq!(shapes(&s), 0);
    assert!(s.execute("layout.apply", &json!({"layout": "compare", "clips": clips[..2], "axis": "z"})).is_err());
    assert!(s.execute("layout.apply", &json!({"layout": "compare", "clips": clips[..2], "from": 2})).is_err());
    // a compound clip is laid out like any picture
    let folded =
        s.execute("clip.compound", &json!({"clips": [clips[2]]})).unwrap()["clip"].as_str().unwrap().to_string();
    s.execute("layout.apply", &json!({"layout": "rows", "clips": [clips[0], folded], "gap": 0})).unwrap();
    let p = s.project();
    let c = p.clip(&folded).unwrap().1;
    assert!((c.look.crop.top - 0.25).abs() < 1e-6 && (c.transform.y - 0.25).abs() < 1e-6 && c.transform.scale == 1.0);
    s.execute("render.compile", &json!({})).unwrap();
}

#[test]
fn spoken_lines_are_placed_on_voiceover_tracks_in_one_step() {
    let mut s = Session::default();
    let line = |n: u8, at: f64, secs: f64| {
        json!({"path": format!("/nowhere/line{n}.wav"), "start": {"seconds": at}, "duration": {"seconds": secs},
               "sha256": format!("{n:02x}").repeat(32)})
    };
    let tracks_before = s.project().tracks.len();
    // the second line starts before the first has finished, so it goes a track down
    let out = s
        .execute("voiceover.add", &json!({"lines": [line(1, 0.0, 2.0), line(2, 1.5, 1.0), line(3, 2.0, 1.0)]}))
        .unwrap();
    let p = s.project();
    let tracks: Vec<&str> = out["tracks"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert_eq!(tracks.len(), 2);
    assert_eq!(p.tracks.len(), tracks_before + 2);
    let (a, b) = (p.track(tracks[0]).unwrap(), p.track(tracks[1]).unwrap());
    assert_eq!((a.name.as_str(), a.kind), (crate::VOICEOVER_TRACK, reelforge_model::TrackKind::Audio));
    assert_eq!(b.name, "Voice-over 2");
    assert_eq!(a.clips.iter().map(|c| c.start).collect::<Vec<_>>(), [Tick::ZERO, Tick::SECOND * 2]);
    assert_eq!(b.clips[0].start, Tick::from_seconds(1.5));
    assert_eq!(p.media.len(), 3);
    // one undo takes it all away
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((s.project().tracks.len(), s.project().media.len()), (tracks_before, 0));
    // a bad line applies nothing and says which
    let e = s.execute("voiceover.add", &json!({"lines": [line(1, 0.0, 1.0), {"path": "x.wav"}]})).unwrap_err();
    assert!(e.to_string().contains("lines[1]") && e.to_string().contains("start"), "{e}");
    let e = s.execute("voiceover.add", &json!({"lines": [{"path": "/nowhere/x.wav", "start": 0}]})).unwrap_err();
    assert!(e.to_string().contains("how long"), "{e}");
    assert_eq!(s.project().media.len(), 0);
    let v = video_track(&s);
    assert!(s.execute("voiceover.add", &json!({"lines": [line(1, 0.0, 1.0)], "track": v})).is_err());
}

#[test]
fn a_voiceover_reads_the_titles_picked_or_the_captions() {
    let mut s = Session::default();
    assert!(crate::spoken_lines(&s.project(), &[]).unwrap_err().contains("add captions"));
    let out = s
        .execute(
            "captions.add",
            &json!({"cues": [
                {"start": {"seconds": 2.0}, "end": {"seconds": 3.0}, "text": "第二句"},
                {"start": {"seconds": 0.0}, "end": {"seconds": 1.0}, "text": "第一\n句"},
                {"start": {"seconds": 1.0}, "end": {"seconds": 2.0}, "text": "  "}
            ]}),
        )
        .unwrap();
    let lines = crate::spoken_lines(&s.project(), &[]).unwrap();
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["第一 句", "第二句"]);
    assert_eq!(lines[1].start, Tick::SECOND * 2);
    let second = out["clips"][1].as_str().unwrap().to_string();
    let picked = crate::spoken_lines(&s.project(), std::slice::from_ref(&second)).unwrap();
    assert_eq!((picked.len(), picked[0].clip.as_str()), (1, second.as_str()));
    let v = add_video(&mut s, 2.0);
    assert!(crate::spoken_lines(&s.project(), &[v]).unwrap_err().contains("not a title"));
}

const CARD: &str = r##"
export const params = {
  price: { type: "number", default: 199, label: "Price" },
  tags: { type: "list", default: ["a", "b"] },
};
export function build({ params, canvas, scene, timeline }) {
  const card = scene.rect({ name: "card", width: 0.8 * canvas.width, height: 400, radius: 40, fill: "#ffffff" });
  const price = scene.text("¥{n}", { name: "price", size: 160, color: "#ff3b5c",
                                     counter: { from: 299, to: params.price, duration: 0.6 } });
  const rule = scene.line({ name: "rule", y: 150, width: 300, stroke: { color: "#999", width: 5 } });
  const tags = params.tags.map((t, i) => scene.text(t, { x: i * 200, y: 250, size: 40 }));
  timeline({ defaults: { duration: 0.5, ease: "power2.out" } })
    .from(card, { y: "+=600", opacity: 0, ease: "back.out(1.4)" })
    .from(price, { scale: 0, ease: "elastic.out(1, 0.5)", duration: 1 }, "<0.1")
    .fromTo(rule, { drawEnd: 0 }, { drawEnd: 1 }, "-=0.5")
    .from(tags, { opacity: 0, stagger: 0.1 }, "-=0.2");
}
"##;

#[test]
fn a_motion_script_builds_grouped_clips_with_keys_in_one_step() {
    let mut s = Session::default();
    let (tracks, steps) = (s.project().tracks.len(), s.history().len());
    let out = s.execute("script.add", &json!({"source": CARD, "start": {"seconds": 2}, "name": "Price card"})).unwrap();
    let group = out["group"].as_str().unwrap().to_string();
    let clips = out["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 5);
    assert_eq!(s.history().len(), steps + 1);
    let p = s.project();
    // each node above the one before, on tracks of its kind, all as long as the piece
    let at: Vec<usize> = clips.iter().map(|c| p.clip(c["clip"].as_str().unwrap()).unwrap().0).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
    let clip = |i: usize| p.clip(clips[i]["clip"].as_str().unwrap()).unwrap().1;
    let fps = p.canvas.fps;
    for i in 0..5 {
        assert_eq!(clip(i).group.as_deref(), Some(group.as_str()));
        assert_eq!(clip(i).start, Tick::from_seconds(2.0));
        assert_eq!(clip(i).duration, out["duration"].as_i64().map(Tick).unwrap());
        assert!(clip(i).keys.values().flatten().all(|k| fps.snap(k.t) == k.t && k.t <= clip(i).duration));
    }
    assert_eq!((clips[0]["node"].as_str(), clips[0]["kind"].as_str()), (Some("card"), Some("shape")));
    assert!(matches!(&clip(0).content, ClipContent::Shape { shape } if shape.size[1] == 400.0 && shape.radius == 40.0));
    // a Bézier ease is two keys; pixels have become shares of the canvas
    let y = &clip(0).keys[&Prop::Y];
    let h = p.canvas.height as f32;
    assert_eq!(y.len(), 2);
    assert!((y[0].v - 600.0 / h).abs() < 1e-6 && y[1].v == 0.0);
    assert!(matches!(y[0].ease, reelforge_model::Ease::Bezier(b) if b[1] > 1.4));
    // an elastic one is a key a frame, and the title counts
    assert_eq!(clip(1).keys[&Prop::Scale].len(), 31);
    assert!(
        matches!(&clip(1).content, ClipContent::Text { style, .. } if style.counter.is_some_and(|c| c.to == 199.0))
    );
    assert_eq!(clip(2).keys[&Prop::DrawEnd].iter().map(|k| k.v).collect::<Vec<_>>(), [0.0, 1.0]);
    assert_eq!(p.scripts[&group].name, "Price card");
    assert_eq!(p.scripts[&group].declared["price"]["label"], json!("Price"));
    s.execute("render.compile", &json!({})).unwrap();
    // it is listed and read back
    let listed = s.execute("script.list", &json!({})).unwrap();
    assert_eq!(listed["scripts"][0]["group"], json!(group));
    assert!(listed["scripts"][0].get("source").is_none());
    let got = s.execute("script.get", &json!({"group": group})).unwrap();
    assert_eq!(got["source"], json!(CARD));
    assert_eq!(got["clips"].as_array().unwrap().len(), 5);
    // one step back and nothing of it is left
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().tracks.len(), tracks);
    assert!(s.project().scripts.is_empty());
}

#[test]
fn a_script_s_parameters_and_source_build_its_group_again_where_it_is() {
    let mut s = Session::default();
    let out = s.execute("script.add", &json!({"source": CARD, "start": {"seconds": 1}})).unwrap();
    let group = out["group"].as_str().unwrap().to_string();
    let (tracks, steps) = (s.project().tracks.len(), s.history().len());
    let lowest = s.project().clip(out["clips"][0]["clip"].as_str().unwrap()).unwrap().0;

    let again = s
        .execute("script.setParams", &json!({"group": group, "params": {"price": 99, "tags": ["x", "y", "z"]}}))
        .unwrap();
    assert_eq!(again["group"], json!(group));
    assert_eq!(again["clips"].as_array().unwrap().len(), 6);
    assert_eq!(s.history().len(), steps + 1);
    let p = s.project();
    // the same tracks again, and one more for the third tag
    assert_eq!(p.tracks.len(), tracks + 1);
    let (ti, card) = p.clip(again["clips"][0]["clip"].as_str().unwrap()).unwrap();
    assert_eq!((ti, card.start), (lowest, Tick::from_seconds(1.0)));
    let price = p.clip(again["clips"][1]["clip"].as_str().unwrap()).unwrap().1;
    assert!(matches!(&price.content, ClipContent::Text { style, .. } if style.counter.is_some_and(|c| c.to == 99.0)));
    assert_eq!(p.scripts[&group].params["price"], json!(99));
    // null is the default again
    s.execute("script.setParams", &json!({"group": group, "params": {"price": null}})).unwrap();
    assert!(!s.project().scripts[&group].params.contains_key("price"));
    assert_eq!(s.project().scripts[&group].params["tags"], json!(["x", "y", "z"]));

    // a new source keeps the values it still has a parameter for
    let other = r#"
        export const params = { tags: { type: "list", default: [] }, size: { type: "number", default: 80 } };
        export function build({ params, scene, timeline }) {
          const all = params.tags.map((t) => scene.text(t, { size: params.size }));
          timeline().from(all, { opacity: 0, stagger: 0.1 });
        }"#;
    let out = s.execute("script.setSource", &json!({"group": group, "source": other})).unwrap();
    assert_eq!(out["clips"].as_array().unwrap().len(), 3);
    let p = s.project();
    assert_eq!(p.scripts[&group].params.keys().collect::<Vec<_>>(), ["tags"]);
    assert!(p.scripts[&group].declared.contains_key("size"));
    assert!(p.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.group.as_deref() == Some(group.as_str())).count() == 3);

    // a source that fails changes nothing
    let before = s.project();
    let e =
        s.execute("script.setSource", &json!({"group": group, "source": "export function build() {}"})).unwrap_err();
    assert_eq!(e.to_string(), "script: the script made nothing: call scene.rect, scene.text, … in build");
    assert_eq!(*s.project(), *before);
    let e = s.execute("script.setParams", &json!({"group": group, "params": {"colour": 1}})).unwrap_err();
    assert!(e.to_string().contains("no parameter `colour`; it has tags, size"), "{e}");
    assert!(s.execute("script.get", &json!({"group": "g99"})).unwrap_err().to_string().contains("no script \"g99\""));
}

#[test]
fn a_script_is_checked_without_a_trace_detached_and_forgotten_with_its_clips() {
    let mut s = Session::default();
    let before = s.project();
    let (steps, revision) = (s.history().len(), s.revision());
    let out = s.execute("script.check", &json!({"source": CARD, "params": {"price": 9}})).unwrap();
    assert_eq!(out["ok"], json!(true));
    assert_eq!(out["nodes"][1], json!({"node": "price", "kind": "text", "keys": {"scale": 31}}));
    assert_eq!(out["values"]["price"], json!(9));
    assert_eq!((*s.project() == *before, s.history().len(), s.revision()), (true, steps, revision));
    // what the clips themselves refuse is found too, and named by its node
    let e = s
        .execute(
            "script.check",
            &json!({"source": "export function build({ scene }) { scene.image('m9', { name: 'shot' }); }"}),
        )
        .unwrap_err();
    assert_eq!(e.to_string(), "script: shot: invalid parameters: no media \"m9\"");
    assert_eq!(*s.project(), *before);
    assert!(s.execute("script.add", &json!({"source": "  "})).is_err());

    // detached, the clips stay a group and the script is gone
    let out = s.execute("script.add", &json!({"source": CARD})).unwrap();
    let group = out["group"].as_str().unwrap().to_string();
    let first = out["clips"][0]["clip"].as_str().unwrap().to_string();
    s.execute("script.detach", &json!({"group": group})).unwrap();
    assert!(s.project().scripts.is_empty());
    assert_eq!(s.project().clip(&first).unwrap().1.group.as_deref(), Some(group.as_str()));
    assert!(s.execute("script.setParams", &json!({"group": group})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().scripts.len(), 1);

    // its clips deleted, a script is forgotten; one of a single clip keeps its group meanwhile
    let one = s
        .execute(
            "script.add",
            &json!({"source": "export function build({ scene }) { scene.rect({}); scene.setDuration(2); }"}),
        )
        .unwrap();
    let (g1, c1) = (one["group"].as_str().unwrap().to_string(), one["clips"][0]["clip"].as_str().unwrap().to_string());
    assert_eq!(one["seconds"], json!(2.0));
    let ids: Vec<&str> = out["clips"].as_array().unwrap().iter().map(|c| c["clip"].as_str().unwrap()).collect();
    s.execute("clip.remove", &json!({"clips": ids})).unwrap();
    let p = s.project();
    assert_eq!(p.scripts.keys().collect::<Vec<_>>(), [&g1]);
    assert_eq!(p.clip(&c1).unwrap().1.group.as_deref(), Some(g1.as_str()));
    // the project with a script in it is written and read back the same
    let text = serde_json::to_string(&*p).unwrap();
    assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), *p);
}

#[test]
fn a_lottie_animation_is_imported_and_placed_like_a_video_that_goes_round() {
    // a blue dot on a see-through 200 × 100 picture, two seconds long; it uses an expression
    let json = r#"{"v":"5.7.0","fr":25,"ip":0,"op":50,"w":200,"h":100,"layers":[
      {"ty":4,"ind":1,"ip":0,"op":50,"st":0,"ks":{"p":{"a":0,"k":[100,50],"x":"var $bm_rt = time;"}},
       "shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[40,40]}},
                 {"ty":"fl","c":{"a":0,"k":[0,0,1,1]},"o":{"a":0,"k":100}}]}]}"#;
    let dir = std::env::temp_dir().join(format!("reelforge-engine-lottie-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dot.json");
    std::fs::write(&path, json).unwrap();

    let mut s = Session::default();
    let r = s.execute("media.import", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["media"]["kind"], "lottie");
    assert_eq!(r["unsupported"], json!(["expressions"]));
    let id = r["media"]["id"].as_str().unwrap().to_string();
    let m = &s.project().media[&id];
    // drawn larger than it says, so that it can be placed large
    assert_eq!((m.size, m.duration), (Some([800, 400]), Some(Tick::SECOND * 2)));

    // on a video track, not a sound track; and longer than it is, since it goes round
    let audio = s.project().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id.clone();
    assert!(s.execute("clip.add", &json!({"track": audio, "media": id})).is_err());
    let video = s.project().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id.clone();
    let c = s.execute("clip.add", &json!({"track": video, "media": id})).unwrap();
    assert_eq!(c["duration"], json!(Tick::SECOND * 2));
    let c = c["clip"].as_str().unwrap().to_string();
    s.execute("clip.trim", &json!({"clip": c, "end": {"seconds": 7}})).unwrap();
    s.execute("clip.slip", &json!({"clip": c, "by": {"seconds": 5}})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.duration, Tick::SECOND * 7);

    // the renderer is told what it is, and that it must know how to draw one; it has no sound
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert_eq!(spec["assets"][&id]["kind"], "lottie");
    assert_eq!(spec["assets"][&id]["probe"]["width"], 800);
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("lottie/1")));
    assert_eq!(spec["layers"][0]["content"]["asset"], json!(id));
    assert!(spec["audio"].as_array().is_none_or(Vec::is_empty));

    // any other JSON is refused, and nothing changes
    std::fs::write(&path, br#"{"name": "not an animation"}"#).unwrap();
    let before = s.revision();
    assert!(s.execute("media.import", &json!({"path": path.to_string_lossy()})).is_err());
    assert_eq!(s.revision(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_storyboard_is_built_in_each_language_on_any_canvas_and_compiles() {
    let boards = s_list("zh");
    assert_eq!(boards.len(), 12);
    for (i, lang) in ["zh", "en", "ja"].into_iter().enumerate() {
        let canvas = [[1080, 1920], [1920, 1080], [1080, 1080]][i];
        for board in &s_list(lang) {
            let id = board["id"].as_str().unwrap();
            let mut s = Session::default();
            s.execute("project.setCanvas", &json!({"width": canvas[0], "height": canvas[1]})).unwrap();
            let before = s.revision();
            let out = s
                .execute("storyboard.apply", &json!({"storyboard": id, "lang": lang, "captions": true}))
                .unwrap_or_else(|e| panic!("{id} in {lang}: {e}"));
            // a slot for each scene with a picture, a marker for each scene, every piece folded
            let scenes = board["scenes"].as_array().unwrap();
            assert_eq!(out["slots"].as_array().unwrap().len() as u64, board["slots"].as_u64().unwrap(), "{id}");
            assert_eq!(out["scenes"].as_array().unwrap().len(), scenes.len());
            let p = s.project();
            assert_eq!(p.markers.len(), scenes.len());
            let planned: usize = scenes.iter().map(|sc| sc["pieces"].as_array().unwrap().len()).sum();
            assert_eq!(out["pieces"].as_array().unwrap().len(), planned);
            for piece in out["pieces"].as_array().unwrap() {
                assert!(crate::made_of(&p, piece["clip"].as_str().unwrap()).is_some(), "{id}: {piece}");
            }
            // as long as it says, with the music cut to it
            let length = Tick::from_seconds(board["seconds"].as_f64().unwrap());
            assert!((p.duration().0 - length.0).abs() < Tick::from_seconds(0.05).0, "{id}");
            // the copy is the storyboard's, in the language asked for, and nothing is left unwritten
            let fields = board["fields"].as_array().unwrap();
            assert_eq!(p.copy.len(), fields.len());
            for (kept, field) in p.copy.iter().zip(fields) {
                assert_eq!(kept.name, field["name"].as_str().unwrap());
                assert!(field["label"].as_str().unwrap() != kept.name, "{id}: {} has no label", kept.name);
            }
            let texts = p.timelines.iter().flat_map(|t| &t.tracks).chain(&p.tracks).flat_map(|t| &t.clips);
            for clip in texts {
                if let ClipContent::Text { text, .. } = &clip.content {
                    assert!(!text.replace("{n}", "").contains('{'), "{id} in {lang}: {text:?} was left unwritten");
                }
            }
            for scene in out["scenes"].as_array().unwrap() {
                assert!(!scene["voice"].as_str().unwrap_or("").contains('{'), "{id}: {scene}");
            }
            s.execute("render.compile", &json!({})).unwrap_or_else(|e| panic!("{id} in {lang}: {e}"));
            // one undo step for the lot
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(s.project().tracks.iter().all(|t| t.clips.is_empty()) && s.project().copy.is_empty());
            assert!(s.revision() > before);
        }
    }
}

fn s_list(lang: &str) -> Vec<Value> {
    let mut s = Session::default();
    s.execute("storyboard.list", &json!({"lang": lang})).unwrap()["storyboards"].as_array().unwrap().clone()
}

#[test]
fn a_storyboard_s_copy_is_filled_by_name_and_its_pieces_are_made_again() {
    let mut s = Session::default();
    let inside = |s: &Session, clip: &str| -> Vec<String> {
        let p = s.project();
        let (timeline, _) = crate::made_of(&p, clip).unwrap();
        let clips = p.tracks_of(timeline).unwrap().iter().flat_map(|t| &t.clips);
        clips
            .filter_map(|c| if let ClipContent::Text { text, .. } = &c.content { Some(text.clone()) } else { None })
            .collect()
    };
    let piece = |out: &Value, kind: &str| -> String {
        let all = out["pieces"].as_array().unwrap();
        all.iter().find(|p| p["kind"] == kind).unwrap()["clip"].as_str().unwrap().to_string()
    };
    // built with some of the copy given; the rest is the storyboard's own
    let out = s
        .execute(
            "storyboard.apply",
            &json!({"storyboard": "launch", "copy": {"product": "云朵拖鞋", "price": 59, "oldPrice": "89"}}),
        )
        .unwrap();
    let (third, tag, button, strike) =
        (piece(&out, "lowerThird"), piece(&out, "priceTag"), piece(&out, "buyButton"), piece(&out, "strikePrice"));
    assert!(inside(&s, &third).contains(&"云朵拖鞋".to_string()));
    assert_eq!(crate::made_of(&s.project(), &tag).unwrap().1.value, Some(59.0));
    assert_eq!(inside(&s, &strike), ["¥89"]);
    assert_eq!(inside(&s, &button), ["立即抢购"]);
    let voice = out["scenes"][1]["voice"].as_str().unwrap();
    assert!(voice.starts_with("云朵拖鞋，"), "{voice}");
    let copy = s.execute("storyboard.copy", &json!({})).unwrap();
    let product = copy["copy"].as_array().unwrap().iter().find(|f| f["name"] == "product").unwrap().clone();
    assert_eq!(
        (product["label"].as_str(), product["value"].as_str(), product["pieces"].as_u64()),
        (Some("商品名"), Some("云朵拖鞋"), Some(1))
    );

    // a clip is set like any other, then fields are filled: what is written from them is made again
    s.execute("clip.set", &json!({"clip": button, "y": 0.3})).unwrap();
    let before = s.revision();
    let filled = s
        .execute(
            "storyboard.fill",
            &json!({"copy": {"cta": "马上带一双回家", "price": 49.9, "product": "云朵拖鞋 2.0"}}),
        )
        .unwrap();
    assert_eq!(filled["pieces"], json!(3));
    assert_eq!(s.revision(), before + 1);
    assert_eq!(inside(&s, &button), ["马上带一双回家"]);
    assert_eq!(crate::made_of(&s.project(), &tag).unwrap().1.value, Some(49.9));
    assert!(inside(&s, &third).contains(&"云朵拖鞋 2.0".to_string()));
    assert_eq!(inside(&s, &strike), ["¥89"]);
    assert_eq!(s.project().clip(&button).unwrap().1.transform.y, 0.3);
    s.execute("render.compile", &json!({})).unwrap();
    // it survives saving
    let p = s.project();
    assert_eq!(Project::from_json(&p.to_json()).unwrap(), *p);
    // one undo step
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(inside(&s, &button), ["立即抢购"]);
    s.execute("edit.redo", &json!({})).unwrap();

    // a field a piece counts to is a number, or nothing changes
    let before = s.revision();
    let e = s.execute("storyboard.fill", &json!({"copy": {"price": "四十九"}})).unwrap_err().to_string();
    assert!(e.contains("`price`") && e.contains("number"), "{e}");
    assert!(s.execute("storyboard.fill", &json!({"copy": {}})).is_err());
    assert!(s.execute("storyboard.fill", &json!({"copy": {"no good": "x"}})).is_err());
    assert!(s.execute("storyboard.fill", &json!({"copy": {"n": "x"}})).is_err());
    assert!(s.execute("storyboard.fill", &json!({"copy": {"cta": ["x"]}})).is_err());
    assert_eq!(s.revision(), before);
    // a field nothing is written from yet is kept for later
    let filled = s.execute("storyboard.fill", &json!({"copy": {"shop": "南山店"}})).unwrap();
    assert_eq!(filled["pieces"], json!(0));
    assert!(s.project().copy.iter().any(|f| f.name == "shop" && f.value == "南山店"));

    // a piece told its words directly is no longer written from the field
    s.execute("promo.set", &json!({"clip": button, "text": "去看看"})).unwrap();
    let filled = s.execute("storyboard.fill", &json!({"copy": {"cta": "不再跟着变"}})).unwrap();
    assert_eq!(filled["pieces"], json!(0));
    assert_eq!(inside(&s, &button), ["去看看"]);

    // a second storyboard goes on a timeline of its own and shares the copy by name
    let first = s.project().timeline.clone();
    let second =
        s.execute("storyboard.apply", &json!({"storyboard": "flashSale", "lang": "en", "seconds": 12})).unwrap();
    assert_ne!(second["timeline"].as_str().unwrap(), first);
    assert!((s.project().duration().seconds() - 12.0).abs() < 0.05);
    assert_eq!(s.project().copy.iter().filter(|f| f.name == "product").count(), 1);
    // what cannot be built is refused
    assert!(s.execute("storyboard.apply", &json!({"storyboard": "nothing"})).is_err());
    assert!(s.execute("storyboard.apply", &json!({"storyboard": "launch", "seconds": 300})).is_err());
    assert!(s.execute("storyboard.apply", &json!({"storyboard": "launch", "lang": "fr"})).is_err());
    assert!(s.execute("storyboard.apply", &json!({"storyboard": "launch", "copy": {"price": "free"}})).is_err());
}

#[test]
fn a_storyboard_takes_the_person_s_pictures_in_scene_order() {
    let mut s = Session::default();
    let mut files = vec![];
    for n in 0..4 {
        let m = s
            .execute(
                "media.import",
                &json!({"path": format!("/nowhere/{n}.mp4"), "kind": "video", "duration": {"seconds": 8}, "sha256": format!("{n:02x}").repeat(32)}),
            )
            .unwrap();
        files.push(m["media"]["id"].as_str().unwrap().to_string());
    }
    let out = s.execute("storyboard.apply", &json!({"storyboard": "launch", "media": files})).unwrap();
    assert_eq!(out["empty"], json!(0));
    let p = s.project();
    for (slot, file) in out["slots"].as_array().unwrap().iter().zip(&files) {
        assert_eq!(slot["media"].as_str(), Some(file.as_str()));
        let (_, clip) = p.clip(slot["clips"][0].as_str().unwrap()).unwrap();
        assert!(matches!(&clip.content, ClipContent::Media { media, .. } if media == file));
    }
    // the last scene has no picture of the person's: a backdrop in the brand's colour
    let last = p.tracks.iter().filter(|t| t.kind == TrackKind::Video).flat_map(|t| &t.clips);
    assert!(last.into_iter().any(|c| c.slot.is_none() && matches!(c.content, ClipContent::Shape { .. })));
}

#[test]
fn captions_written_from_the_copy_follow_it_until_changed_by_hand() {
    let mut s = Session::default();
    let out = s.execute("storyboard.apply", &json!({"storyboard": "launch", "captions": true})).unwrap();
    let said = |s: &Session| -> Vec<String> {
        let p = s.project();
        let track = p.tracks.iter().find(|t| t.name == crate::CAPTIONS_TRACK).unwrap();
        track
            .clips
            .iter()
            .filter_map(|c| if let ClipContent::Text { text, .. } = &c.content { Some(text.clone()) } else { None })
            .collect()
    };
    // a caption for each scene's line, as the scenes say them
    let voices: Vec<String> =
        out["scenes"].as_array().unwrap().iter().map(|sc| sc["voice"].as_str().unwrap().to_string()).collect();
    assert_eq!(said(&s), voices);
    assert_eq!(said(&s)[3], "新品首发 · 限时尝鲜价，到手只要 129 元。");
    // the price is filled: its caption says the new one; the others are as they were
    let filled = s.execute("storyboard.fill", &json!({"copy": {"price": 119}})).unwrap();
    assert_eq!((filled["pieces"].as_u64(), filled["captions"].as_u64()), (Some(1), Some(1)));
    assert_eq!(said(&s)[3], "新品首发 · 限时尝鲜价，到手只要 119 元。");
    assert_eq!(said(&s)[0], voices[0]);
    // one changed by hand is left alone from then on
    let hook = s.project().copy_lines[0].clip.clone();
    s.execute("clip.set", &json!({"clip": hook, "text": "还在喝凉水？"})).unwrap();
    let filled = s.execute("storyboard.fill", &json!({"copy": {"hook": "杯子不保温？"}})).unwrap();
    assert_eq!(filled["captions"].as_u64(), Some(0));
    assert_eq!(said(&s)[0], "还在喝凉水？");
    assert_eq!(s.project().copy_lines.len(), voices.len() - 1);
    // and it survives saving
    let p = s.project();
    assert_eq!(Project::from_json(&p.to_json()).unwrap(), *p);
}

#[test]
fn a_library_decoration_is_a_lottie_clip_over_the_picture_in_the_brand_s_colours() {
    let mut s = Session::default();
    let v = video_track(&s);
    let list = s.execute("decor.list", &json!({})).unwrap();
    let list = list["decor"].as_array().unwrap();
    assert_eq!(list.len(), 20);
    let rain = list.iter().find(|d| d["id"] == "confettiRain").unwrap();
    assert_eq!(
        (&rain["plays"], &rain["scale"], &rain["colours"]),
        (&json!("round"), &json!(null), &json!(["color", "color2"]))
    );
    s.execute("clip.add", &json!({"track": v, "color": [0, 0, 0, 1], "duration": {"seconds": 5}})).unwrap();

    // as drawn: its own colours, its own length and size, on a track above the picture
    let a = s.execute("decor.add", &json!({"decor": "pulse", "start": {"seconds": 1}})).unwrap();
    let p = s.project();
    let m = &p.media[a["media"].as_str().unwrap()];
    assert_eq!((m.uri.as_str(), m.kind, m.size), ("library:decor/pulse", MediaKind::Lottie, Some([1080, 1080])));
    assert_eq!(
        (m.name.as_str(), m.duration, m.sha256.as_ref().map(String::len)),
        ("Pulse", Some(Tick::from_seconds(1.5)), Some(64))
    );
    let (ti, clip) = p.clip(a["clip"].as_str().unwrap()).unwrap();
    let track = &p.tracks[ti];
    assert_eq!(track.kind, TrackKind::Video);
    assert!(ti > p.tracks.iter().position(|t| t.id == v).unwrap());
    assert_eq!(
        (clip.start, clip.duration, clip.transform.scale),
        (Tick::from_seconds(1.0), Tick::from_seconds(3.0), 0.3)
    );
    let over = track.id.clone();

    // again beside it: the same media, the same track; over it: the next track up
    let b = s
        .execute("decor.add", &json!({"decor": "pulse", "start": {"seconds": 4}, "duration": {"seconds": 1}}))
        .unwrap();
    assert_eq!((&b["media"], &b["track"]), (&a["media"], &json!(over)));
    let c =
        s.execute("decor.add", &json!({"decor": "pulse", "start": {"seconds": 2}, "x": 0.2, "scale": 0.5})).unwrap();
    assert_eq!(&c["media"], &a["media"]);
    assert_ne!(c["track"], json!(over));
    assert_eq!(s.project().media.len(), 1);
    // a track that is named must have room, and be for pictures
    let full = s.execute("decor.add", &json!({"decor": "pulse", "start": {"seconds": 2}, "track": over}));
    assert!(matches!(full, Err(EngineError::Refused(_))), "{full:?}");
    let t = s.execute("track.add", &json!({"kind": "text"})).unwrap();
    assert!(s.execute("decor.add", &json!({"decor": "pulse", "track": t["track"]})).is_err());
    assert!(s.execute("decor.add", &json!({"decor": "kazoo"})).is_err());
    assert!(s.execute("decor.add", &json!({"decor": "pulse", "duration": 0})).is_err());

    // the brand's colours where it has them; told ones over the brand's; each look its own media
    s.execute("brand.set", &json!({"primary": [0, 0, 1, 1], "secondary": [0, 1, 0, 1]})).unwrap();
    let uri = |s: &Session, v: &serde_json::Value| s.project().media[v["media"].as_str().unwrap()].uri.clone();
    let d = s.execute("decor.add", &json!({"decor": "pulse", "start": {"seconds": 6}})).unwrap();
    assert_eq!(uri(&s, &d), "library:decor/pulse?primary=0000ff");
    let e = s.execute("decor.add", &json!({"decor": "flame", "start": {"seconds": 6}, "color": [1, 0.5, 0]})).unwrap();
    assert_eq!(uri(&s, &e), "library:decor/flame?primary=ff8000&secondary=00ff00");
    assert_ne!(s.project().media[d["media"].as_str().unwrap()].sha256, m.sha256);

    // one that covers: larger than fitting by how much the canvas is taller than it
    let f = s.execute("decor.add", &json!({"decor": "confettiRain", "start": {"seconds": 8}})).unwrap();
    let p = s.project();
    let m = &p.media[f["media"].as_str().unwrap()];
    assert_eq!(m.size, Some([1440, 1920]));
    let scale = p.clip(f["clip"].as_str().unwrap()).unwrap().1.transform.scale;
    assert!((scale - (1920.0 / 1920.0) / (1080.0 / 1440.0)).abs() < 1e-4, "{scale}");

    // it all compiles, the animations as lottie assets, and one step takes one back
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let assets = spec["assets"].as_object().unwrap_or_else(|| panic!("{spec}"));
    assert_eq!(assets.values().filter(|a| a["kind"] == "lottie").count(), 4);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().clip(f["clip"].as_str().unwrap()).is_none());
}

fn add_shape(s: &mut Session, track: &str, size: [u32; 2]) -> String {
    let add = json!({"track": track, "start": 0, "duration": {"seconds": 2}, "shape": {
        "kind": "rectangle", "size": size, "fill": [1, 0, 0, 1]
    }});
    s.execute("clip.add", &add).unwrap()["clip"].as_str().unwrap().to_string()
}

/// Where the clip's layer lands at `t`, as the renderer places it.
fn outline(s: &Session, clip: &str, t: f64) -> [[f32; 2]; 4] {
    let spec = reelforge_compile::compile(&s.project(), &Default::default()).expect("compiles");
    reelforge_renderspec::validate(&spec).expect("valid");
    let all = reelforge_render::layer_outlines(&spec, Tick::from_seconds(t), &reelforge_render::NoMedia);
    all.into_iter().find(|(id, _)| id == clip).expect("on screen").1
}

fn close(a: [[f32; 2]; 4], b: [[f32; 2]; 4]) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| (a - b).abs() < 0.6)
}

#[test]
fn a_clip_is_stretched_turned_in_space_and_keeps_its_place_under_another_anchor() {
    let mut s = Session::default();
    let v = video_track(&s);
    let c = add_shape(&mut s, &v, [540, 192]);
    let flat = outline(&s, &c, 0.0);
    assert_eq!(flat[0], [270.0, 864.0]);

    // half as wide, swung in space, seen from nearer
    s.execute("clip.set", &json!({"clip": c, "scaleX": 0.5, "rotationY": 30, "perspective": 1.5})).unwrap();
    let t = s.project().clip(&c).unwrap().1.transform;
    assert_eq!((t.scale_x, t.scale_y, t.rotation_y, t.distance), (0.5, 1.0, 30.0, 1.5));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let tr = &spec["layers"][0]["transform"];
    assert_eq!((&tr["stretch"], &tr["turn"]), (&json!([0.5, 1.0]), &json!({"x": 0.0, "y": 30.0, "distance": 1.5})));
    let needs: Vec<&str> = spec["engine"]["features"].as_array().unwrap().iter().filter_map(|f| f.as_str()).collect();
    assert!(needs.contains(&"transform.3d/1") && needs.contains(&"transform.stretch/1"), "{needs:?}");
    // its left edge, nearer, is taller than its right
    let o = outline(&s, &c, 0.0);
    assert!(o[3][1] - o[0][1] > o[2][1] - o[1][1] + 6.0 && o[1][0] - o[0][0] < 260.0, "{o:?}");
    for bad in [json!({"perspective": 0}), json!({"anchorX": 9}), json!({"scaleX": "wide"})] {
        let mut with = bad.clone();
        with["clip"] = json!(c);
        assert!(s.execute("clip.set", &with).is_err(), "{bad}");
    }

    // another anchor: the picture stays where it lies, x and y say where the anchor now is
    s.execute("clip.set", &json!({"clip": c, "rotationY": 0})).unwrap();
    let before = outline(&s, &c, 0.0);
    s.execute("clip.set", &json!({"clip": c, "anchorX": 0, "anchorY": 1})).unwrap();
    let t = s.project().clip(&c).unwrap().1.transform;
    assert_eq!(t.anchor, [0.0, 1.0]);
    assert!((t.x + 0.125).abs() < 1e-4 && (t.y - 0.05).abs() < 1e-4, "{t:?}");
    assert!(close(outline(&s, &c, 0.0), before));
    // turned about it: its bottom left corner is where it was
    s.execute("clip.set", &json!({"clip": c, "rotation": 90})).unwrap();
    let turned = outline(&s, &c, 0.0);
    assert!((turned[3][0] - before[3][0]).abs() < 0.6 && (turned[3][1] - before[3][1]).abs() < 0.6, "{turned:?}");
    // and so when it is turned already; told not to keep it, only the anchor changes
    s.execute("clip.set", &json!({"clip": c, "anchorX": 0.5, "anchorY": 0.5})).unwrap();
    assert!(close(outline(&s, &c, 0.0), turned));
    let was = s.project().clip(&c).unwrap().1.transform;
    s.execute("clip.set", &json!({"clip": c, "anchorX": 1, "keep": false})).unwrap();
    let t = s.project().clip(&c).unwrap().1.transform;
    assert_eq!((t.anchor, t.x, t.y), ([1.0, 0.5], was.x, was.y));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().clip(&c).unwrap().1.transform, was);

    // a title is measured as it is laid out
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let add = json!({"track": text, "text": "限时特价", "size": 96, "start": 0, "duration": {"seconds": 2}});
    let title = s.execute("clip.add", &add).unwrap()["clip"].as_str().unwrap().to_string();
    let before = outline(&s, &title, 0.0);
    let y = s.project().clip(&title).unwrap().1.transform.y;
    s.execute("clip.set", &json!({"clip": title, "anchorX": 0})).unwrap();
    let t = s.project().clip(&title).unwrap().1.transform;
    assert!(t.x < -0.1 && t.x > -0.3 && t.y == y, "{t:?}");
    assert!(close(outline(&s, &title, 0.0), before));

    // turns in space are keyed and animated like any other property
    s.execute("clip.setKey", &json!({"clip": title, "prop": "rotationY", "at": 0, "value": -80})).unwrap();
    s.execute("clip.setKey", &json!({"clip": title, "prop": "rotationY", "at": {"seconds": 1}, "value": 0})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": c, "slot": "in", "kind": "flipX"})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": c, "slot": "loop", "kind": "tilt"})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layers = spec["layers"].as_array().unwrap();
    let turn = |id: &str| layers.iter().find(|l| l["id"] == id).unwrap()["transform"]["turn"].clone();
    assert_eq!((&turn(&title)["y"][0]["v"], &turn(&title)["y"][1]["v"]), (&json!(-80.0), &json!(0.0)));
    // tipping up from edge on while it rocks from side to side
    let lid = turn(&c);
    assert!((lid["x"][0]["v"].as_f64().unwrap() + 82.0).abs() < 1e-3, "{}", lid["x"][0]);
    assert!(lid["y"].as_array().unwrap().len() > 30 && lid["x"].as_array().unwrap().len() > 30);
    // a door swung half open shows narrower than it lies
    let o = outline(&s, &title, 0.5);
    assert!(o[1][0] - o[0][0] < (before[1][0] - before[0][0]) * 0.9, "{o:?} {before:?}");
}

#[test]
fn a_clip_carried_by_another_follows_it_and_keeps_its_place() {
    let mut s = Session::default();
    let v = video_track(&s);
    let parent = add_shape(&mut s, &v, [400, 400]);
    s.execute("clip.set", &json!({"clip": parent, "x": 0.2, "scale": 0.5, "rotation": 90})).unwrap();
    let over = s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string();
    let child = add_shape(&mut s, &over, [200, 100]);
    s.execute("clip.set", &json!({"clip": child, "x": -0.1, "y": 0.1})).unwrap();
    let before = outline(&s, &child, 0.0);

    // carried: it stays where it shows, its placement now counted from its parent
    let r = s.execute("clip.setParent", &json!({"clip": child, "parent": parent})).unwrap();
    let t = s.project().clip(&child).unwrap().1.transform;
    assert!((t.x - 384.0 / 1080.0).abs() < 1e-4 && (t.y - 648.0 / 1920.0).abs() < 1e-4, "{t:?}");
    assert!((t.scale - 2.0).abs() < 1e-5 && (t.rotation + 90.0).abs() < 1e-4, "{t:?}");
    assert_eq!((&r["parent"], r["scale"].as_f64().map(|v| v.round())), (&json!(parent), Some(2.0)));
    assert!(close(outline(&s, &child, 0.0), before));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let layer = spec["layers"].as_array().unwrap().iter().find(|l| l["id"] == json!(child)).unwrap();
    assert_eq!(layer["parent"], json!(parent));
    assert!(spec["engine"]["features"].as_array().unwrap().contains(&json!("layer.parent/1")));

    // the parent moves, grows and turns: the child goes with it as one object
    let centre = |o: [[f32; 2]; 4]| [(o[0][0] + o[2][0]) / 2.0, (o[0][1] + o[2][1]) / 2.0];
    let (was, pivot) = (centre(before), centre(outline(&s, &parent, 0.0)));
    s.execute("clip.set", &json!({"clip": parent, "x": 0.3, "scale": 1.0, "rotation": 180})).unwrap();
    let (now, moved) = (centre(outline(&s, &child, 0.0)), centre(outline(&s, &parent, 0.0)));
    // twice as far from the parent's middle, a quarter turn on
    let (dx, dy) = (was[0] - pivot[0], was[1] - pivot[1]);
    assert!((now[0] - (moved[0] - 2.0 * dy)).abs() < 0.6 && (now[1] - (moved[1] + 2.0 * dx)).abs() < 0.6, "{now:?}");

    // a ring is refused, and so is what cannot carry
    for (clip, of) in [(&parent, &child), (&child, &child)] {
        assert!(s.execute("clip.setParent", &json!({"clip": clip, "parent": of})).is_err());
    }
    assert!(s.execute("clip.setParent", &json!({"clip": child, "parent": "c999"})).is_err());
    // set free where it shows, then carried again without keeping its place
    let carried = outline(&s, &child, 0.0);
    s.execute("clip.setParent", &json!({"clip": child, "parent": null})).unwrap();
    assert!(s.project().clip(&child).unwrap().1.parent.is_none());
    assert!(close(outline(&s, &child, 0.0), carried));
    let free = s.project().clip(&child).unwrap().1.transform;
    s.execute("clip.setParent", &json!({"clip": child, "parent": parent, "keep": false})).unwrap();
    assert_eq!(s.project().clip(&child).unwrap().1.transform, free);
    assert!(!close(outline(&s, &child, 0.0), carried));

    // its parent gone, it is drawn on its own
    s.execute("clip.remove", &json!({"clips": [parent]})).unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["layers"][0].get("parent").is_none(), "{}", spec["layers"][0]);
}

#[test]
fn a_clip_travels_along_a_path_at_an_even_speed() {
    let mut s = Session::default();
    let v = video_track(&s);
    let c = add_shape(&mut s, &v, [120, 60]);
    let path = json!({"clip": c, "points": [[-0.3, 0.2], [0, -0.2], [0.3, 0.2]], "orient": true,
        "duration": {"seconds": 1}, "ease": "linear"});
    let r = s.execute("clip.setPath", &path).unwrap();
    assert_eq!((&r["path"]["orient"], &r["path"]["points"][1][0]), (&json!(true), &json!(0.0)));
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let tr = &spec["layers"][0]["transform"];
    // frame by frame while it travels, then it stays at the last point
    let keys = tr["position"].as_array().unwrap();
    assert_eq!(keys.len(), 32, "{keys:?}");
    let at = |i: usize| [keys[i]["v"][0].as_f64().unwrap(), keys[i]["v"][1].as_f64().unwrap()];
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 2e-3 && (a[1] - b[1]).abs() < 2e-3;
    assert!(
        near(at(0), [-0.3, 0.2]) && near(at(15), [0.0, -0.2]) && near(at(30), [0.3, 0.2]) && near(at(31), [0.3, 0.2])
    );
    // as far each frame, on the canvas (which is taller than it is wide)
    let step = |i: usize| ((at(i + 1)[0] - at(i)[0]) * 1080.0).hypot((at(i + 1)[1] - at(i)[1]) * 1920.0);
    assert!((step(3) / step(14) - 1.0).abs() < 0.03, "{} {}", step(3), step(14));
    // facing the way it goes: up and to the right, level over the top, then down
    let turn = |i: usize| tr["rotation"][i]["v"].as_f64().unwrap();
    assert!(turn(1) < -45.0 && turn(15).abs() < 4.0 && turn(29) > 45.0, "{} {} {}", turn(1), turn(15), turn(29));

    // its other settings change on their own; a path needs its points
    let r = s.execute("clip.setPath", &json!({"clip": c, "closed": true, "start": {"seconds": 0.5}})).unwrap();
    assert_eq!((&r["path"]["closed"], r["path"]["points"].as_array().map(Vec::len)), (&json!(true), Some(3)));
    let p = s.project().clip(&c).unwrap().1.path.clone().unwrap();
    assert_eq!((p.start, p.duration, p.orient), (Tick::from_seconds(0.5), Tick::SECOND, true));
    for bad in [json!({"points": [[0, 0]]}), json!({"points": "round"}), json!({"duration": 0})] {
        let mut with = bad.clone();
        with["clip"] = json!(c);
        assert!(s.execute("clip.setPath", &with).is_err(), "{bad}");
    }
    // taking the path away puts the clip back where its x and y say
    s.execute("clip.setPath", &json!({"clip": c, "points": null})).unwrap();
    assert!(s.project().clip(&c).unwrap().1.path.is_none());
    assert!(s.execute("clip.setPath", &json!({"clip": c, "orient": true})).is_err());
    let spec = s.execute("render.compile", &json!({})).unwrap();
    assert!(spec["layers"][0]["transform"]["position"][0].is_number());
}

#[test]
fn titles_come_in_by_the_word_are_written_on_and_sung() {
    let mut s = Session::default();
    let track = |kind: TrackKind| s.project().tracks.iter().find(|t| t.kind == kind).unwrap().id.clone();
    let (video, text) = (track(TrackKind::Video), track(TrackKind::Text));
    let add =
        |s: &mut Session, params: Value| s.execute("clip.add", &params).unwrap()["clip"].as_str().unwrap().to_string();
    let title = add(&mut s, json!({"track": text, "text": "限时特价 big sale", "duration": {"seconds": 4}}));
    let content = |s: &mut Session| s.execute("render.compile", &json!({})).unwrap()["layers"][0]["content"].clone();
    let features = |s: &mut Session| s.execute("render.compile", &json!({})).unwrap()["engine"]["features"].clone();
    let has = |f: &Value, id: &str| f.as_array().unwrap().iter().any(|v| v == id);
    assert!(!has(&features(&mut s), "text/7"));

    // word by word, then springing letter by letter as it leaves... which settles instead
    s.execute(
        "clip.setAnimation",
        &json!({"clip": title, "slot": "in", "kind": "wordRise", "duration": {"seconds": 1}}),
    )
    .unwrap();
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "out", "kind": "charElastic"})).unwrap();
    let c = content(&mut s);
    let chars = c["chars"].as_array().unwrap();
    assert_eq!((chars[0]["unit"].as_str(), chars[0]["curve"].as_str()), (Some("word"), Some("settle")));
    assert_eq!(
        (chars[1].get("unit"), chars[1]["curve"].as_str(), chars[1]["reverse"].as_bool()),
        (None, Some("settle"), Some(true))
    );
    assert!(has(&features(&mut s), "text/7"));
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "in", "kind": "charBounce"})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "out", "kind": null})).unwrap();
    assert_eq!(content(&mut s)["chars"][0]["curve"], "bounce");
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "in", "kind": "lineRise"})).unwrap();
    assert_eq!(content(&mut s)["chars"][0]["unit"], "line");

    // written on: a pen over the animation's length, and off again
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "in", "kind": "draw", "duration": {"seconds": 2}}))
        .unwrap();
    let c = content(&mut s);
    let keys = c["draw"]["progress"].as_array().unwrap();
    assert_eq!((keys[0]["v"].as_f64(), keys[1]["v"].as_f64()), (Some(0.0), Some(1.0)));
    assert_eq!(keys[1]["t"], json!(Tick::from_seconds(2.0)));
    assert!(c.get("chars").is_none() && c["draw"]["width"].as_f64().unwrap() > 0.0);
    // a picture has no outline for a pen, and words are a title's
    let picture = add(
        &mut s,
        json!({"track": video, "shape": {"kind": "rectangle", "size": [200, 200]}, "duration": {"seconds": 2}}),
    );
    assert!(s.execute("clip.setAnimation", &json!({"clip": picture, "slot": "in", "kind": "wordFade"})).is_err());
    s.execute("clip.setAnimation", &json!({"clip": picture, "slot": "in", "kind": "draw"})).unwrap();
    // any clip springs in or drops in: its scale goes past its size, its place comes down from above
    s.execute(
        "clip.setAnimation",
        &json!({"clip": picture, "slot": "in", "kind": "elastic", "duration": {"seconds": 1}}),
    )
    .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let scales = spec["layers"][0]["transform"]["scale"].as_array().unwrap();
    assert!(scales.iter().any(|k| k["v"].as_f64().unwrap() > 1.15) && scales.last().unwrap()["v"] == json!(1.0));
    s.execute("clip.setAnimation", &json!({"clip": picture, "slot": "in", "kind": "drop", "duration": {"seconds": 1}}))
        .unwrap();
    let spec = s.execute("render.compile", &json!({})).unwrap();
    let places = spec["layers"][0]["transform"]["position"].as_array().unwrap();
    let y = |k: &Value| k["v"][1].as_f64().unwrap();
    assert!(y(&places[0]) < -0.3 && places.iter().all(|k| y(k) <= 1e-6) && y(places.last().unwrap()).abs() < 1e-6);
    s.execute("clip.remove", &json!({"clips": [picture]})).unwrap();
    s.execute("clip.setAnimation", &json!({"clip": title, "slot": "in", "kind": null})).unwrap();

    // sung: the voice goes through the title over its length, yellow unless told
    let k = s.execute("clip.set", &json!({"clip": title, "karaoke": {}})).unwrap();
    assert_eq!(k["clip"]["content"]["style"]["karaoke"]["mode"], "sweep");
    let c = content(&mut s);
    let h = &c["highlight"];
    let keys = h["progress"].as_array().unwrap();
    assert_eq!((keys.len(), keys[0]["v"].as_f64(), keys[1]["v"].as_f64()), (2, Some(0.0), Some(1.0)));
    assert_eq!((keys[0]["t"].clone(), keys[1]["t"].clone()), (json!(Tick::ZERO), json!(Tick::from_seconds(4.0))));
    assert!(h.get("unit").is_none() && h["color"][0] == json!(1.0));
    // the word being said, from one second in for two: none lit before, and what it had is kept
    s.execute(
        "clip.set",
        &json!({"clip": title, "karaoke": {"mode": "word", "color": [1, 0, 0, 1], "start": {"seconds": 1}, "duration": {"seconds": 2}}}),
    )
    .unwrap();
    s.execute("clip.set", &json!({"clip": title, "karaoke": {"mode": "word"}})).unwrap();
    let c = content(&mut s);
    let h = &c["highlight"];
    let keys = h["progress"].as_array().unwrap();
    assert_eq!((h["unit"].as_str(), h["color"].clone()), (Some("word"), json!([1.0, 0.0, 0.0, 1.0])));
    assert_eq!((keys[0]["v"].as_f64(), keys[0]["ease"].as_str()), (Some(-1.0), Some("hold")));
    assert_eq!(
        (keys[1]["t"].clone(), keys[2]["t"].clone()),
        (json!(Tick::from_seconds(1.0)), json!(Tick::from_seconds(3.0)))
    );
    // a voice that would outlast the clip keeps its pace and is cut short
    s.execute("clip.set", &json!({"clip": title, "karaoke": {"start": 0, "duration": {"seconds": 8}}})).unwrap();
    let keys = content(&mut s)["highlight"]["progress"].as_array().unwrap().clone();
    assert_eq!((keys.len(), keys[1]["v"].as_f64()), (2, Some(0.5)));
    // the frame shows it: red ink half way through, none without
    s.execute("clip.set", &json!({"clip": title, "karaoke": {"mode": "sweep", "duration": null}})).unwrap();
    let red = |s: &mut Session| {
        let spec: reelforge_renderspec::RenderSpec =
            serde_json::from_value(s.execute("render.compile", &json!({})).unwrap()).unwrap();
        let (img, _) = reelforge_render::render_frame(
            &spec,
            Tick::from_seconds(2.0),
            0.25,
            &reelforge_render::NoMedia,
            reelforge_render::Quality::Final,
        )
        .unwrap();
        img.pixels.iter().filter(|p| p[0] > 0.8 && p[1] < 0.2 && p[3] > 0.8).count()
    };
    assert!(red(&mut s) > 20);
    // bad values are refused, null takes it off, and captions take it from one another
    for bad in [json!({"mode": "loud"}), json!({"start": {"seconds": -1}}), json!({"duration": 0}), json!(3)] {
        assert!(s.execute("clip.set", &json!({"clip": title, "karaoke": bad})).is_err(), "{bad}");
    }
    let other =
        add(&mut s, json!({"track": text, "text": "第二句", "start": {"seconds": 5}, "duration": {"seconds": 2}}));
    s.execute("captions.restyle", &json!({"from": title})).unwrap();
    let style = |s: &Session, id: &str| match &s.project().clip(id).unwrap().1.content {
        ClipContent::Text { style, .. } => style.karaoke,
        _ => None,
    };
    assert_eq!(style(&s, &other).map(|k| (k.mode, k.duration)), Some((reelforge_model::KaraokeMode::Sweep, None)));
    s.execute("clip.set", &json!({"clip": title, "karaoke": null})).unwrap();
    assert!(style(&s, &title).is_none() && red(&mut s) == 0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(style(&s, &title).is_some());
}

#[test]
fn the_next_clip_is_wiped_opened_flipped_and_torn_in() {
    let mut s = Session::default();
    let track = video_track(&s);
    add_red(&mut s, 0.0, 2.0);
    let blue = s
        .execute(
            "clip.add",
            &json!({"track": track, "color": [0, 0, 1], "start": {"seconds": 2}, "duration": {"seconds": 2}}),
        )
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let set = |s: &mut Session, kind: &str| {
        s.execute("clip.setTransition", &json!({"clip": blue, "kind": kind, "duration": {"seconds": 1}})).unwrap();
    };
    let spec = |s: &mut Session| s.execute("render.compile", &json!({})).unwrap();
    // a frame a quarter the size, and the pixel at a place in it as [red, blue] (sRGB-ish, 0 to 1)
    let frame = |s: &mut Session, at: f64| {
        let spec: reelforge_renderspec::RenderSpec = serde_json::from_value(spec(s)).unwrap();
        reelforge_render::render_frame(
            &spec,
            Tick::from_seconds(at),
            0.25,
            &reelforge_render::NoMedia,
            reelforge_render::Quality::Final,
        )
        .unwrap()
        .0
    };
    let at = |img: &reelforge_render::Image, x: f32, y: f32| {
        let (px, py) = ((x * img.width as f32) as u32, (y * img.height as f32) as u32);
        let p = img.pixels[(py.min(img.height - 1) * img.width + px.min(img.width - 1)) as usize];
        [p[0], p[2]]
    };
    let (is_red, is_blue) = (|p: [f32; 2]| p[0] > 0.9 && p[1] < 0.1, |p: [f32; 2]| p[1] > 0.9 && p[0] < 0.1);
    let plain = frame(&mut s, 3.0);
    for kind in reelforge_model::TransitionKind::ALL {
        set(&mut s, kind.name());
        // before it the last clip, after it the next, as if there were no transition
        let (before, after) = (frame(&mut s, 1.0), frame(&mut s, 3.0));
        assert!(is_red(at(&before, 0.5, 0.5)) && is_red(at(&before, 0.05, 0.05)), "{kind:?}");
        assert!(after.pixels == plain.pixels, "{kind:?}");
    }

    // a wipe to the right has uncovered the left half at the cut, behind a soft edge
    set(&mut s, "wipeRight");
    let mid = frame(&mut s, 2.0);
    assert!(is_blue(at(&mid, 0.2, 0.5)) && is_red(at(&mid, 0.8, 0.5)), "{:?}", at(&mid, 0.2, 0.5));
    let edge = at(&mid, 0.5, 0.5);
    assert!(edge[0] > 0.05 && edge[1] > 0.05, "{edge:?}");
    set(&mut s, "wipeUp");
    let mid = frame(&mut s, 2.0);
    assert!(is_blue(at(&mid, 0.5, 0.8)) && is_red(at(&mid, 0.5, 0.2)));
    // a circle and a diamond open from the middle
    for kind in ["circle", "diamond", "box"] {
        set(&mut s, kind);
        let early = frame(&mut s, 1.75);
        assert!(is_blue(at(&early, 0.5, 0.5)) && is_red(at(&early, 0.1, 0.1)), "{kind}");
    }
    let layers = spec(&mut s)["layers"].clone();
    assert_eq!(layers[1]["shape_mask"]["shape"], "rectangle");
    assert!(layers[1]["shape_mask_keys"]["size"].as_array().unwrap().len() > 20);
    // a clip with a mask of its own dissolves instead
    s.execute("clip.setMask", &json!({"clip": blue, "shape": "circle"})).unwrap();
    let layers = spec(&mut s)["layers"].clone();
    assert!(layers[1]["shape_mask"]["shape"] == "circle" && layers[1].get("shape_mask_keys").is_none());
    assert!(layers[1]["opacity"].as_array().unwrap()[0]["v"] == json!(0.0));
    s.execute("clip.setMask", &json!({"clip": blue, "shape": null})).unwrap();

    // a card: before the cut the last clip is turned, seen narrower; after it the next one is
    set(&mut s, "flip");
    let (early, late) = (frame(&mut s, 1.85), frame(&mut s, 2.15));
    assert!(is_red(at(&early, 0.5, 0.5)) && !is_red(at(&early, 0.03, 0.5)) && !is_blue(at(&early, 0.5, 0.5)));
    assert!(is_blue(at(&late, 0.5, 0.5)) && !is_blue(at(&late, 0.03, 0.5)));
    let features = spec(&mut s)["engine"]["features"].clone();
    assert!(features.as_array().unwrap().iter().any(|f| f == "transform.3d/1"));
    // a page closes from the right edge: at the cut the left of the frame still shows the last clip
    set(&mut s, "pageTurn");
    let mid = frame(&mut s, 2.0);
    assert!(
        is_red(at(&mid, 0.1, 0.5)) && is_blue(at(&mid, 0.9, 0.5)),
        "{:?} {:?}",
        at(&mid, 0.1, 0.5),
        at(&mid, 0.9, 0.5)
    );

    // light: three glows screened over both clips, brightest at the cut
    set(&mut s, "lightLeak");
    let layers = spec(&mut s)["layers"].clone();
    let glows: Vec<&Value> =
        layers.as_array().unwrap().iter().filter(|l| l["id"].as_str().unwrap().contains("~light")).collect();
    assert!(glows.len() == 3 && glows.iter().all(|l| l["blend"] == "screen" && l["content"]["type"] == "shape"));
    let mid = frame(&mut s, 2.0);
    let lit: f32 = mid.pixels.iter().map(|p| p[1]).sum::<f32>() / mid.pixels.len() as f32;
    assert!(lit > 0.1, "green of the warm light over red and blue: {lit}");

    // torn: both layers jump in bands with their colours apart while it plays, and it cuts
    set(&mut s, "glitch");
    let layers = spec(&mut s)["layers"].clone();
    for l in [&layers[0], &layers[1]] {
        let ids: Vec<&str> = l["fx"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"distort.glitch@1") && ids.contains(&"distort.split@1"), "{ids:?}");
    }
    let (early, late) = (frame(&mut s, 1.9), frame(&mut s, 2.1));
    assert!(at(&early, 0.5, 0.5)[1] < 0.1 && at(&late, 0.5, 0.5)[0] < 0.1);
    // larger: the last clip has grown past the frame by the cut (pictures are blurred as well,
    // which the compiler's own test looks at)
    set(&mut s, "zoomBlur");
    let layers = spec(&mut s)["layers"].clone();
    let grown = layers[0]["transform"]["scale"].as_array().unwrap().last().unwrap()["v"].as_f64().unwrap();
    assert!(grown > 2.0, "{grown}");
}

#[test]
fn a_clip_is_smeared_along_its_way_when_it_is_asked_to_be() {
    let mut s = Session::default();
    // red all over the frame, pushed out to the left by the blue clip after it
    add_red(&mut s, 0.0, 2.0);
    let track = video_track(&s);
    let blue = s
        .execute(
            "clip.add",
            &json!({"track": track, "color": [0, 0, 1], "start": {"seconds": 2}, "duration": {"seconds": 2}}),
        )
        .unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("clip.setTransition", &json!({"clip": blue, "kind": "pushLeft", "duration": {"seconds": 0.5}})).unwrap();
    let spec = |s: &mut Session| -> reelforge_renderspec::RenderSpec {
        serde_json::from_value(s.execute("render.compile", &json!({})).unwrap()).unwrap()
    };
    // the middle row at the cut, a quarter the size: how blue each column is
    let row = |s: &mut Session, at: f64| -> Vec<f32> {
        let (img, _) = reelforge_render::render_frame(
            &spec(s),
            Tick::from_seconds(at),
            0.25,
            &reelforge_render::NoMedia,
            reelforge_render::Quality::Final,
        )
        .unwrap();
        let y = img.height / 2;
        (0..img.width).map(|x| img.pixels[(y * img.width + x) as usize][2]).collect()
    };
    let soft = |row: &[f32]| row.iter().filter(|b| **b > 0.02 && **b < 0.98).count();
    // without it the edge between the two is a column at most
    assert!(spec(&mut s).layers.iter().all(|l| l.motion_blur.is_none()));
    assert!(soft(&row(&mut s, 2.0)) <= 1);

    // on: a film camera's shutter unless a number says how long, on the clip and on its layer
    let set = |s: &mut Session, v: Value| s.execute("clip.set", &json!({"clip": blue, "motionBlur": v}));
    let on = set(&mut s, json!(true)).unwrap();
    assert_eq!(on["clip"]["transform"]["motion_blur"], 0.5);
    let compiled = spec(&mut s);
    assert_eq!(compiled.layers[1].motion_blur, Some(reelforge_renderspec::MotionBlur { shutter: 0.5 }));
    assert!(compiled.layers[0].motion_blur.is_none() && compiled.required_features().contains("motion.blur/1"));
    // the blue clip's leading edge is a ramp now, several columns wide, and wider for a longer shutter
    let half = soft(&row(&mut s, 2.0));
    assert!(half >= 3, "{half}");
    set(&mut s, json!(1.5)).unwrap();
    assert!(soft(&row(&mut s, 2.0)) > half);
    // where it stands still it is as it was
    set(&mut s, json!(false)).unwrap();
    let plain = row(&mut s, 3.0);
    set(&mut s, json!(true)).unwrap();
    assert_eq!(row(&mut s, 3.0), plain);
    // off again, by false or by 0; and a shutter is 0 to 2
    let off = set(&mut s, json!(0)).unwrap();
    assert!(off["clip"]["transform"].get("motion_blur").is_none());
    assert!(spec(&mut s).layers.iter().all(|l| l.motion_blur.is_none()));
    for bad in [json!(3), json!(-0.5), json!("on")] {
        assert!(matches!(set(&mut s, bad), Err(EngineError::Params(m)) if m.contains("motionBlur")));
    }
    // one undo for each set
    s.execute("clip.set", &json!({"clip": blue, "motionBlur": true})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().clip(&blue).unwrap().1.transform.motion_blur, 0.0);
}

#[test]
fn a_clip_lands_turned_in_space_in_one_step_and_the_outline_says_so() {
    let mut s = Session::default();
    let t = video_track(&s);
    let r = s
        .execute(
            "clip.add",
            &json!({"track": t, "color": [1, 0, 0], "rotationY": 30, "scaleX": -1, "motionBlur": true, "anchorX": 0}),
        )
        .unwrap();
    let id = r["clip"].as_str().unwrap();
    let tr = s.project().clip(id).unwrap().1.transform;
    assert_eq!((tr.rotation_y, tr.scale_x, tr.anchor[0]), (30.0, -1.0, 0.0));
    assert!(tr.motion_blur > 0.0);
    assert_eq!(s.history(), ["Add Clip"], "one undo step");
    let outline = s.execute("project.outline", &json!({})).unwrap();
    let space = &outline["tracks"][0]["clips"][0]["space"];
    assert_eq!(
        (&space["rotationY"], &space["scaleX"], &space["motionBlur"]),
        (&json!(30.0), &json!(-1.0), &json!(0.5))
    );
    // what clip.set refuses, clip.add refuses, and adds nothing
    assert!(s.execute("clip.add", &json!({"track": t, "color": [1, 0, 0], "perspective": 0})).is_err());
    assert_eq!(s.project().tracks[0].clips.len(), 1);
}

#[test]
fn clips_move_on_the_beat_and_are_cut_on_it() {
    let listed = Session::default().execute("beat.list", &json!({})).unwrap();
    let ids: Vec<&str> = listed["moves"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["punch", "pulse", "shake", "flash", "strobe", "tilt", "bounce", "squash"]);

    let mut s = Session::default();
    let fps = s.project().canvas.fps;
    let frame = fps.frame_duration();
    let video = add_video(&mut s, 10.0);
    let at = |secs: f64| fps.snap(Tick::from_seconds(secs));
    let keys =
        |s: &Session, id: &str, p: Prop| s.project().clip(id).unwrap().1.keys.get(&p).cloned().unwrap_or_default();
    let value = |s: &Session, id: &str, p: Prop, t: Tick| s.project().clip(id).unwrap().1.value_at(p, t);

    // with no beat said and none marked, it says what to do
    let ask = json!({"clips": [video], "move": "punch"});
    assert!(matches!(s.execute("beat.apply", &ask), Err(EngineError::Refused(m)) if m.contains("audio.beats")));

    // a tempo: 120 a minute is a beat every half second, the first at 1 s
    let out = s
        .execute("beat.apply", &json!({"clips": [video], "move": "punch", "bpm": 120, "first": {"seconds": 1}}))
        .unwrap();
    assert_eq!(out["beats"], 18, "{out}");
    assert_eq!(out["props"], json!(["scale"]));
    // at rest before the first beat, at its largest on the beat, at rest again before the next
    assert_eq!(value(&s, &video, Prop::Scale, at(0.9)), 1.0);
    assert!((value(&s, &video, Prop::Scale, at(1.0)) - 1.14).abs() < 1e-4);
    let after = value(&s, &video, Prop::Scale, at(1.0) + frame * 3);
    assert!(after > 1.0 && after < 1.14, "{after}");
    assert_eq!(value(&s, &video, Prop::Scale, at(1.4)), 1.0);
    assert!((value(&s, &video, Prop::Scale, at(1.5)) - 1.14).abs() < 1e-4);
    // every key is at the start of a frame
    assert!(keys(&s, &video, Prop::Scale).iter().all(|k| fps.snap(k.t) == k.t));

    // the same kind again replaces it: twice as strong, every second beat
    let stronger =
        json!({"clips": [video], "move": "punch", "bpm": 120, "first": {"seconds": 1}, "every": 2, "strength": 1});
    assert_eq!(s.execute("beat.apply", &stronger).unwrap()["beats"], 9);
    assert!((value(&s, &video, Prop::Scale, at(1.0)) - 1.28).abs() < 1e-4);
    assert_eq!(value(&s, &video, Prop::Scale, at(1.5)), 1.0);
    // another kind adds to it, and measures from the clip's own place
    s.execute("clip.set", &json!({"clip": video, "rotation": 10})).unwrap();
    s.execute("beat.apply", &json!({"clips": [video], "move": "tilt", "at": [{"seconds": 2}, {"seconds": 3}]}))
        .unwrap();
    assert_eq!(value(&s, &video, Prop::Rotation, at(2.0)), 13.5);
    assert_eq!(value(&s, &video, Prop::Rotation, at(3.0)), 6.5);
    assert_eq!(value(&s, &video, Prop::Rotation, at(2.9)), 10.0);
    assert!(!keys(&s, &video, Prop::Scale).is_empty());

    // the timeline's beat markers are the beat when nothing else is said; other markers are not
    for (secs, note) in [(0.5, "♪"), (1.25, "♪"), (2.0, "♪"), (2.6, "here")] {
        s.execute("marker.add", &json!({"at": {"seconds": secs}, "note": note})).unwrap();
    }
    for mv in ids.iter().filter(|m| **m != "punch" && **m != "tilt") {
        let out = s.execute("beat.apply", &json!({"clips": [video], "move": mv})).unwrap();
        assert_eq!(out["beats"], 3, "{mv}: {out}");
    }
    assert!(value(&s, &video, Prop::Exposure, at(1.25)) > 0.8 && value(&s, &video, Prop::Exposure, at(2.6)) == 0.0);
    assert!(value(&s, &video, Prop::Opacity, at(2.0)) < 0.6 && value(&s, &video, Prop::Opacity, at(1.9)) == 1.0);
    assert!(value(&s, &video, Prop::ScaleX, at(0.5)) > 1.05 && value(&s, &video, Prop::ScaleY, at(0.5)) < 0.95);
    // a hop goes up and comes down; a shake ends where it began
    assert!(value(&s, &video, Prop::Y, at(0.5) + frame * 3) < 0.0 && value(&s, &video, Prop::Y, at(1.2)) == 0.0);
    // what comes out is a project that compiles
    s.execute("render.compile", &json!({})).unwrap();
    // one undo takes a move off again
    let had = keys(&s, &video, Prop::ScaleX).len();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(had > 0 && keys(&s, &video, Prop::ScaleX).is_empty());

    // what cannot be: an unknown move, both a tempo and times, no beat within the clips, a
    // title asked to flash, a sound clip, a clip on a path asked to shake
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let title = s.execute("clip.add", &json!({"track": text, "text": "SALE", "duration": {"seconds": 2}})).unwrap();
    let title = title["clip"].as_str().unwrap().to_string();
    for bad in [
        json!({"clips": [video], "move": "wobble"}),
        json!({"clips": [video], "move": "punch", "bpm": 120, "at": [0]}),
        json!({"clips": [video], "move": "punch", "at": [{"seconds": 60}]}),
        json!({"clips": [video], "move": "punch", "first": 0}),
        json!({"clips": [video], "move": "punch", "every": 0}),
        json!({"clips": [video], "move": "punch", "strength": 2}),
        json!({"clips": [title], "move": "flash"}),
        json!({"clips": [video, video], "move": "punch"}),
        json!({"clips": ["c999"], "move": "punch"}),
    ] {
        assert!(s.execute("beat.apply", &bad).is_err(), "{bad}");
    }
    s.execute("beat.apply", &json!({"clips": [title], "move": "shake"})).unwrap();
    let path = json!({"clip": title, "points": [[-0.2, 0], [0.2, 0]]});
    s.execute("clip.setPath", &path).unwrap();
    assert!(
        matches!(s.execute("beat.apply", &json!({"clips": [title], "move": "bounce"})), Err(EngineError::Refused(m)) if m.contains("path"))
    );
    assert!(s.execute("beat.apply", &json!({"clips": [title], "move": "punch"})).is_ok());
}

#[test]
fn a_montage_is_cut_so_that_every_clip_ends_on_a_beat() {
    let mut s = Session::default();
    let fps = s.project().canvas.fps;
    let track = video_track(&s);
    let span = |s: &Session, id: &str| {
        let c = s.project().clip(id).unwrap().1.clone();
        (c.start.seconds(), c.duration.seconds())
    };
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02;
    // three stills end to end, 1.3 s each, and a video of 0.9 s
    let mut stills = vec![];
    for i in 0..3 {
        let params = json!({"track": track, "color": [1, 0, 0], "start": {"seconds": 1.3 * i as f64}, "duration": {"seconds": 1.3}});
        stills.push(s.execute("clip.add", &params).unwrap()["clip"].as_str().unwrap().to_string());
    }
    let m = s
        .execute("media.import", &json!({"path": "/nowhere/short.mp4", "kind": "video", "duration": {"seconds": 0.9}, "sha256": "ab".repeat(32)}))
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let short = s.execute("clip.add", &json!({"track": track, "media": m, "start": {"seconds": 3.9}})).unwrap();
    let short = short["clip"].as_str().unwrap().to_string();

    // 120 a minute, two beats a clip: every still is a second long, one after another
    let out = s.execute("beat.cut", &json!({"clips": stills, "bpm": 120, "every": 2})).unwrap();
    assert_eq!(out["onBeat"], 3, "{out}");
    for (i, id) in stills.iter().enumerate() {
        assert!(near(span(&s, id), (i as f64, 1.0)), "{:?}", span(&s, id));
    }
    assert_eq!(out["cuts"][1]["start"], json!(fps.snap(Tick::from_seconds(1.0))));
    // one undo puts them back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(span(&s, &stills[2]), (2.6, 1.3)));

    // a video too short for two beats ends on the one it reaches, and the next cut is on the
    // beat again; a still is made longer to reach its beat
    let order = [stills[0].clone(), short.clone(), stills[1].clone(), stills[2].clone()];
    // at 150 a minute a beat is 0.4 s: the short video takes two of them when asked for three
    let out = s.execute("beat.cut", &json!({"clips": order, "bpm": 150, "every": 3})).unwrap();
    assert!(near(span(&s, &stills[0]), (0.0, 1.2)), "{out}");
    assert!(near(span(&s, &short), (1.2, 0.8)), "{:?}", span(&s, &short));
    assert!(near(span(&s, &stills[1]), (2.0, 0.4)) && near(span(&s, &stills[2]), (2.4, 1.2)), "{out}");
    assert_eq!(out["onBeat"], 4);
    s.execute("edit.undo", &json!({})).unwrap();
    // and is refused when it reaches no beat at all, with nothing changed
    let slow = s.execute("beat.cut", &json!({"clips": order, "bpm": 60, "every": 2}));
    assert!(matches!(slow, Err(EngineError::Refused(m)) if m.contains("0.90 s") && m.contains("1.00 s")));
    assert!(near(span(&s, &stills[0]), (0.0, 1.3)));

    // times given: clips left over when the beats run out follow at their own length; each
    // arrives with a flash, once, at its cut
    let ask = json!({"clips": stills, "at": [{"seconds": 0.5}, {"seconds": 1.5}], "move": "punch", "from": 0});
    let out = s.execute("beat.cut", &ask).unwrap();
    assert_eq!(out["onBeat"], 2);
    assert!(near(span(&s, &stills[0]), (0.0, 0.5)) && near(span(&s, &stills[1]), (0.5, 1.0)));
    assert!(near(span(&s, &stills[2]), (1.5, 1.3)), "{:?}", span(&s, &stills[2]));
    for id in &stills {
        let c = s.project().clip(id).unwrap().1.clone();
        assert!(c.value_at(Prop::Scale, Tick::ZERO) > 1.1 && c.value_at(Prop::Scale, c.duration) == 1.0);
    }
    s.execute("render.compile", &json!({})).unwrap();

    // refused: clips of two tracks, a clip in the way, no beat after the start
    let other = s.execute("track.add", &json!({"kind": "video"})).unwrap()["track"].as_str().unwrap().to_string();
    let above =
        s.execute("clip.add", &json!({"track": other, "color": [0, 0, 1], "duration": {"seconds": 1}})).unwrap();
    let above = above["clip"].as_str().unwrap().to_string();
    let two = s.execute("beat.cut", &json!({"clips": [stills[0], above], "bpm": 120}));
    assert!(matches!(two, Err(EngineError::Refused(m)) if m.contains("one track")));
    let crowded = s.execute("beat.cut", &json!({"clips": [stills[0], stills[1]], "bpm": 20}));
    assert!(matches!(crowded, Err(EngineError::Refused(m)) if m.contains("in the way")), "{:?}", span(&s, &stills[2]));
    let late = s.execute("beat.cut", &json!({"clips": [above], "at": [0]}));
    assert!(matches!(late, Err(EngineError::Refused(m)) if m.contains("no beat")));
}

#[test]
fn a_new_canvas_shape_keeps_covering_pictures_covering() {
    let mut s = Session::default();
    let video = video_track(&s);
    let text = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text).unwrap().id.clone();
    let import = |s: &mut Session, path: &str, size: [u32; 2]| -> String {
        s.execute(
            "media.import",
            &json!({"path": path, "kind": "image", "size": size, "duration": {"seconds": 3}, "sha256": "ab".repeat(32)}),
        )
        .unwrap()["media"]["id"]
        .as_str()
        .unwrap()
        .to_string()
    };
    let wide = import(&mut s, "/nowhere/wide.png", [1920, 1080]);
    let tall = import(&mut s, "/nowhere/tall.png", [1080, 1920]);
    let add = |s: &mut Session, media: &str, start: i64| -> String {
        s.execute(
            "clip.add",
            &json!({"track": video, "media": media, "start": {"seconds": start}, "duration": {"seconds": 2}}),
        )
        .unwrap()["clip"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // a portrait picture fills the default 9:16 canvas; a landscape one is fitted inside, and a
    // second landscape one is scaled to cover it
    let filled = add(&mut s, &tall, 0);
    let fitted = add(&mut s, &wide, 2);
    let covered = add(&mut s, &wide, 4);
    let k = (1920.0f32 / 1080.0).powi(2);
    s.execute("clip.set", &json!({"clip": covered, "scale": k})).unwrap();
    let title = s.execute("clip.add", &json!({"track": text, "text": "hi", "size": 90})).unwrap()["clip"]
        .as_str()
        .unwrap()
        .to_string();

    let r = s.execute("project.setCanvas", &json!({"ratio": "16:9"})).unwrap();
    assert_eq!(r["canvas"]["width"], 1920);
    assert_eq!(r["canvas"]["height"], 1080);
    let scale = |s: &Session, id: &str| s.project().clip(id).unwrap().1.transform.scale;
    assert!((scale(&s, &filled) - k).abs() < 1e-3, "a filling picture goes on covering");
    assert_eq!(scale(&s, &fitted), 1.0, "a fitted one stays fitted");
    assert!((scale(&s, &covered) - 1.0).abs() < 1e-3, "a covering one now fills by its own shape");
    // the shorter side did not change, so neither did the title
    let size = |s: &Session| match &s.project().clip(&title).unwrap().1.content {
        ClipContent::Text { size, .. } => *size,
        _ => unreachable!(),
    };
    assert_eq!(size(&s), 90.0);

    // the same shape at a smaller size: pictures stay, titles shrink with it
    s.execute("project.setCanvas", &json!({"width": 1280, "height": 720})).unwrap();
    assert!((scale(&s, &filled) - k).abs() < 1e-3);
    assert!((size(&s) - 60.0).abs() < 1e-3);

    // without refitting nothing moves; a bad shape is refused
    s.execute("project.setCanvas", &json!({"width": 720, "height": 720, "refit": false})).unwrap();
    assert!((scale(&s, &filled) - k).abs() < 1e-3);
    assert!(s.execute("project.setCanvas", &json!({"ratio": "5:7"})).is_err());
    assert!(s.execute("project.setCanvas", &json!({"width": 8})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().canvas.width, 1280);
}

#[test]
fn a_script_s_nodes_carry_one_another_turn_about_an_origin_and_travel_paths() {
    let mut s = Session::default();
    let src = r#"export function build({ scene, timeline, canvas }) {
        const card = scene.group({ name: "card", x: 192, rotationY: 30, perspective: canvas.height });
        const bar = scene.rect({ name: "bar", parent: card, y: 108, width: 384, height: 54, transformOrigin: "left top" });
        const dot = scene.circle({ name: "dot", radius: 10, x: -192 });
        timeline()
          .from(bar, { scaleX: 0, x: "-=192", duration: 1, ease: "none" })
          .to(dot, { motionPath: { path: [[0, 108], [192, 0]], autoRotate: true }, duration: 1, ease: "none" }, 0.5);
    }"#;
    let out = s.execute("script.add", &json!({"source": src})).unwrap();
    assert_eq!(s.history().len(), 1);
    let id = |i: usize| out["clips"][i]["clip"].as_str().unwrap().to_string();
    let proj = s.project();
    let (w, h) = (proj.canvas.width as f32, proj.canvas.height as f32);
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let card = proj.clip(&id(0)).unwrap().1;
    assert_eq!(out["clips"][0]["kind"], json!("group"));
    assert!(
        near(card.base(Prop::Opacity), 0.0)
            && near(card.base(Prop::RotationY), 30.0)
            && near(card.base(Prop::X), 192.0 / w)
    );
    assert!(near(card.transform.distance, 1.0));
    // carried by the card, and turned about its own top left corner, which is where x and y now point
    let bar = proj.clip(&id(1)).unwrap().1;
    assert_eq!(bar.parent.as_deref(), Some(id(0).as_str()));
    assert_eq!(bar.transform.anchor, [0.0, 0.0]);
    assert!(near(bar.base(Prop::Y), (108.0 - 27.0) / h), "{}", bar.base(Prop::Y));
    let xs: Vec<f32> = bar.keys[&Prop::X].iter().map(|k| k.v * w).collect();
    assert!(near(xs[0], -192.0 - 192.0) && near(xs[1], -192.0), "{xs:?}");
    assert_eq!(bar.keys[&Prop::ScaleX].iter().map(|k| k.v).collect::<Vec<_>>(), [0.0, 1.0]);
    // from where it is through the points, setting off half a second in
    let dot = proj.clip(&id(2)).unwrap().1;
    let path = dot.path.as_ref().unwrap();
    assert_eq!(path.points.len(), 3);
    assert!(
        near(path.points[0][0], -192.0 / w) && near(path.points[1][1], 108.0 / h) && near(path.points[2][0], 192.0 / w)
    );
    assert_eq!(
        (path.start, path.duration, path.orient, path.ease),
        (Tick::from_seconds(0.5), Tick::from_seconds(1.0), true, reelforge_model::Ease::Linear)
    );
    assert!(reelforge_compile::compile(&proj, &reelforge_compile::CompileOptions { encode: false }).is_ok());

    // built again where it is, the same; undone, gone in one step
    let group = out["group"].as_str().unwrap();
    let again = s.execute("script.setParams", &json!({"group": group})).unwrap();
    let proj = s.project();
    let bar = proj.clip(again["clips"][1]["clip"].as_str().unwrap()).unwrap().1;
    assert_eq!(bar.parent.as_deref(), again["clips"][0]["clip"].as_str());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.project().scripts.is_empty());
}
