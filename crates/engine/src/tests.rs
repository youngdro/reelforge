use reelforge_model::ClipContent;
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
