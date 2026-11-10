use std::sync::Arc;

use base64::Engine;
use reelforge_engine::Session;
use serde_json::{Value, json};

use crate::http::HttpServer;
use crate::{CallError, Hub, Output, mcp, ops};

fn rpc(hub: &Hub, id: i64, method: &str, params: Value) -> Value {
    mcp::handle(hub, &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}), "mcp:test")
        .expect("a reply")
}

fn call(hub: &Hub, tool: &str, args: Value) -> Value {
    rpc(hub, 9, "tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("reelforge-server-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// The project's first track of `kind`.
fn track(hub: &Hub, kind: &str) -> String {
    let p = hub.session().project();
    let json = serde_json::to_value(&*p).unwrap();
    json["tracks"].as_array().unwrap().iter().find(|t| t["kind"] == kind).unwrap()["id"].as_str().unwrap().into()
}

#[test]
fn every_operation_is_a_tool_with_a_schema() {
    let hub = Hub::new(Session::default());
    let init = rpc(
        &hub,
        1,
        "initialize",
        json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // notifications get no answer
    assert!(mcp::handle(&hub, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), "t").is_none());

    let tools = rpc(&hub, 2, "tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), ops().len() - 1, "all but commands.list");
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
    }
    let split = tools.iter().find(|t| t["name"] == "clip_split").unwrap();
    assert_eq!(split["inputSchema"]["required"], json!(["clip", "at"]));
    assert_eq!(tools.iter().find(|t| t["name"] == "project_inspect").unwrap()["annotations"]["readOnlyHint"], true);
    assert_eq!(tools.iter().find(|t| t["name"] == "clip_remove").unwrap()["annotations"]["destructiveHint"], true);
}

#[test]
fn an_agent_edits_looks_and_recovers_from_mistakes() {
    let hub = Hub::new(Session::default());
    let video = track(&hub, "video");
    let added = call(&hub, "clip_add", json!({"track": video, "color": [1, 0, 0], "duration": {"seconds": 2}}));
    assert_eq!(added["isError"], false);
    let clip = added["structuredContent"]["clip"].as_str().unwrap().to_string();

    // a refusal is a result the model reads, not a protocol error
    let bad = call(&hub, "clip_split", json!({"clip": clip, "at": {"seconds": 5}}));
    assert_eq!(bad["isError"], true);
    assert!(bad["content"][0]["text"].as_str().unwrap().len() > 5);
    let unknown = rpc(&hub, 3, "tools/call", json!({"name": "clip_explode", "arguments": {}}));
    assert_eq!(unknown["error"]["code"], -32602);

    assert_eq!(call(&hub, "clip_split", json!({"clip": clip, "at": {"seconds": 1}}))["isError"], false);
    let frame = call(&hub, "render_frame", json!({"at": {"seconds": 0.5}, "scale": 0.25}));
    assert_eq!(frame["content"][0]["type"], "image");
    let png = base64::engine::general_purpose::STANDARD.decode(frame["content"][0]["data"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (270, 480));
    assert_eq!(img.get_pixel(135, 240).0, [255, 0, 0, 255]);

    assert_eq!(call(&hub, "edit_undo", json!({}))["isError"], false);
    let p = call(&hub, "project_inspect", json!({}));
    let clips: usize = p["structuredContent"]["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["clips"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(clips, 1);
}

#[test]
fn a_project_file_follows_every_edit() {
    let path = temp("follow.reel.json");
    let _ = std::fs::remove_file(&path);
    let hub = Hub::open(&path).expect("opens");
    assert!(path.exists(), "created on open");
    let text = track(&hub, "text");
    hub.call("clip.add", &json!({"track": text, "text": "字幕", "duration": {"seconds": 1}})).expect("adds");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("字幕"));
    // queries do not write
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    hub.call("project_inspect", &json!({})).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
    // a reopened hub sees it
    let again = Hub::open(&path).unwrap();
    assert_eq!(again.session().project().tracks.iter().map(|t| t.clips.len()).sum::<usize>(), 1);
    assert!(matches!(again.call("clip.fly", &json!({})), Err(CallError::Unknown(_))));
}

#[test]
fn export_writes_beside_the_project() {
    let path = temp("export.reel.json");
    let _ = std::fs::remove_file(&path);
    let hub = Hub::open(&path).unwrap();
    let video = track(&hub, "video");
    hub.call("clip.add", &json!({"track": video, "color": [0, 0, 1], "duration": {"frame": 3}})).unwrap();
    hub.call("project.setCanvas", &json!({"width": 64, "height": 64})).unwrap();
    let Output::Json(out) = hub.call("render.export", &json!({"path": "out.mp4", "renderer": "cpu"})).unwrap() else {
        panic!("json")
    };
    assert_eq!(out["frames"], 3);
    assert!(path.with_file_name("out.mp4").exists());
}

fn post(base: &str, path: &str, body: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.post(format!("{base}{path}")).header("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut r = req.send(body).expect("request");
    let heads = r.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
    (r.status().as_u16(), heads, r.body_mut().read_to_vec().expect("body"))
}

fn serve(token: Option<&str>) -> String {
    let hub = Arc::new(Hub::new(Session::default()));
    let server = HttpServer::bind(hub, "127.0.0.1:0", token.map(str::to_string)).expect("binds");
    let addr = server.local_addr();
    std::thread::spawn(move || server.run());
    format!("http://{addr}")
}

#[test]
fn streamable_http_carries_mcp_sessions() {
    let base = serve(None);
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let (status, heads, _) = post(&base, "/mcp", init, &[("Accept", "application/json, text/event-stream")]);
    assert_eq!(status, 200);
    let session = heads.iter().find(|(k, _)| k.eq_ignore_ascii_case("mcp-session-id")).expect("a session").1.clone();

    let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert_eq!(post(&base, "/mcp", note, &[("Mcp-Session-Id", &session)]).0, 202);
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let (status, _, body) = post(&base, "/mcp", list, &[("Mcp-Session-Id", &session)]);
    assert_eq!(status, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "render_frame"));
    assert_eq!(post(&base, "/mcp", list, &[("Mcp-Session-Id", "nope")]).0, 404);
    // a page on another site cannot drive a local server
    assert_eq!(post(&base, "/mcp", list, &[("Origin", "https://evil.example")]).0, 403);
    assert_eq!(post(&base, "/mcp", list, &[("Origin", "http://localhost:5173")]).0, 200);
}

#[test]
fn the_plain_api_runs_operations() {
    let base = serve(Some("s3cret"));
    assert_eq!(post(&base, "/v1/ops/project.inspect", "{}", &[]).0, 401);
    let auth = [("Authorization", "Bearer s3cret")];
    let (status, _, body) = post(&base, "/v1/ops/project.inspect", "", &auth);
    assert_eq!(status, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    let video =
        v["result"]["project"]["tracks"].as_array().unwrap().iter().find(|t| t["kind"] == "video").unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    let add = json!({"track": video, "color": [0, 1, 0]}).to_string();
    // tool names work too
    assert_eq!(post(&base, "/v1/ops/clip_add", &add, &auth).0, 200);
    let (status, _, body) = post(&base, "/v1/ops/clip.split", r#"{"clip": "nope", "at": 0}"#, &auth);
    assert_eq!(status, 422);
    assert!(String::from_utf8_lossy(&body).contains("nope"));
    assert_eq!(post(&base, "/v1/ops/clip.fly", "{}", &auth).0, 404);
    let (status, heads, png) = post(&base, "/v1/ops/render.frame", r#"{"scale": 0.1}"#, &auth);
    assert_eq!(status, 200);
    assert!(heads.iter().any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "image/png"));
    assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(5, 5).0, [0, 255, 0, 255]);
}

#[test]
fn other_machines_need_a_token() {
    let hub = Arc::new(Hub::new(Session::default()));
    let Err(e) = HttpServer::bind(hub.clone(), "0.0.0.0:0", None) else { panic!("bound without a token") };
    assert!(e.contains("token"));
    assert!(HttpServer::bind(hub, "0.0.0.0:0", Some("t".into())).is_ok());
}

#[test]
fn remote_clients_hear_who_changed_what() {
    let hub = Arc::new(Hub::new(Session::default()));
    let server = HttpServer::bind(hub.clone(), "127.0.0.1:0", None).unwrap();
    let base = format!("http://{}", server.local_addr());
    std::thread::spawn(move || server.run());
    // an MCP client names itself, and its edits carry the name
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude-code","version":"1"}}}"#;
    let (_, heads, _) = post(&base, "/mcp", init, &[]);
    let session = heads.iter().find(|(k, _)| k.eq_ignore_ascii_case("mcp-session-id")).unwrap().1.clone();
    let since = hub.session().revision();
    let poll = {
        let base = base.clone();
        std::thread::spawn(move || {
            let mut r = ureq::get(format!("{base}/v1/events?since={since}&timeout=10")).call().unwrap();
            serde_json::from_slice::<Value>(&r.body_mut().read_to_vec().unwrap()).unwrap()
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(100));
    let video = track(&hub, "video");
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "clip_add", "arguments": {"track": video, "color": [1, 1, 0]}}});
    post(&base, "/mcp", &call.to_string(), &[("Mcp-Session-Id", &session)]);
    let heard = poll.join().unwrap();
    assert_eq!(heard, json!({"revision": since + 1, "command": "clip.add", "by": "agent claude-code"}));
    assert_eq!(hub.session().last_change().by, "agent claude-code");
}

#[test]
fn browsers_send_media_and_fetch_it_back() {
    let dir = temp("web");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html>editor").unwrap();
    std::fs::create_dir_all(dir.join("snippets/ui-1")).unwrap();
    std::fs::write(dir.join("snippets/ui-1/inline0.js"), "export {}").unwrap();
    let file = temp("browser.reel.json");
    let _ = std::fs::remove_file(&file);
    let hub = Arc::new(Hub::open(&file).unwrap());
    let server = HttpServer::bind(hub.clone(), "127.0.0.1:0", Some("k".into())).unwrap().with_web(dir);
    let base = format!("http://{}", server.local_addr());
    std::thread::spawn(move || server.run());
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();

    // the page itself needs no token; everything it does does
    let mut r = agent.get(format!("{base}/")).call().unwrap();
    assert_eq!(r.body_mut().read_to_string().unwrap(), "<!doctype html>editor");
    assert_eq!(agent.get(format!("{base}/..%2Fsecret")).call().unwrap().status(), 404);
    // scripts wasm-bindgen keeps in folders, but nothing hidden
    let mut r = agent.get(format!("{base}/snippets/ui-1/inline0.js")).call().unwrap();
    assert_eq!(r.body_mut().read_to_string().unwrap(), "export {}");
    assert_eq!(agent.get(format!("{base}/snippets/.hidden/x.js")).call().unwrap().status(), 404);
    assert_eq!(agent.get(format!("{base}/snippets//x.js")).call().unwrap().status(), 404);
    assert_eq!(agent.get(format!("{base}/v1/project")).call().unwrap().status(), 401);

    let png = {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect();
    let put = |sha: &str| {
        let mut r = agent
            .put(format!("{base}/v1/media/{sha}?name=my%20red%20dot.png"))
            .header("Authorization", "Bearer k")
            .send(&png[..])
            .unwrap();
        (r.status().as_u16(), serde_json::from_slice::<Value>(&r.body_mut().read_to_vec().unwrap()).unwrap())
    };
    assert_eq!(put(&"0".repeat(64)).0, 422, "the hash is checked");
    let (status, kept) = put(&sha);
    assert_eq!(status, 200);
    let path = kept["path"].as_str().unwrap().to_string();
    assert!(path.ends_with("-my_red_dot.png") && std::path::Path::new(&path).is_absolute(), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), png);

    // imported by that path, by a client that names itself
    let import = json!({"path": path, "name": "my red dot.png", "sha256": sha}).to_string();
    let (status, _, _) =
        post(&base, "/v1/ops/media.import", &import, &[("Authorization", "Bearer k"), ("X-Reelforge-Client", "web")]);
    assert_eq!(status, 200);
    assert_eq!(hub.session().last_change().by, "web");
    let m = hub.session().project().media.values().next().unwrap().clone();
    assert_eq!((m.name.as_str(), m.size), ("my red dot.png", Some([4, 2])));

    let mut r = agent.get(format!("{base}/v1/media/{sha}")).header("Authorization", "Bearer k").call().unwrap();
    assert_eq!(r.body_mut().read_to_vec().unwrap(), png);
    let other = "f".repeat(64);
    let r = agent.get(format!("{base}/v1/media/{other}")).header("Authorization", "Bearer k").call().unwrap();
    assert_eq!(r.status(), 404);
}

/// A 48 kHz mono WAV of `seconds` of `sample(i)`.
fn wav(seconds: f32, sample: impl Fn(usize) -> f32) -> Vec<u8> {
    let n = (48_000.0 * seconds) as u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + n * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(48_000u32.to_le_bytes());
    b.extend(96_000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((n * 2).to_le_bytes());
    for i in 0..n as usize {
        b.extend(((sample(i).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

/// A number from -1 to 1, the same every run.
fn hiss(i: usize) -> f32 {
    let mut z = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 23) as f32) - 1.0
}

#[test]
fn tools_listen_and_then_edit() {
    let tone = |hz: f32, i: usize| (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin();
    // a voice from 1 s to 2 s and from 4 s to 5 s, over hiss
    let voice = temp("voice.wav");
    std::fs::write(
        &voice,
        wav(6.0, |i| {
            let t = i as f32 / 48_000.0;
            let speaking = (1.0..2.0).contains(&t) || (4.0..5.0).contains(&t);
            0.01 * hiss(i) + if speaking { 0.1 * tone(220.0, i) } else { 0.0 }
        }),
    )
    .unwrap();
    // a song at 120 beats a minute: a kick every half second from a quarter second in
    let song = temp("song.wav");
    std::fs::write(
        &song,
        wav(8.0, |i| {
            let since = (i + 36_000) % 24_000;
            0.6 * (-(since as f32) / 900.0).exp() * tone(60.0, since) + 0.002 * hiss(i)
        }),
    )
    .unwrap();

    let hub = Hub::new(Session::default());
    let json = |out: Result<Output, CallError>| match out.expect("ok") {
        Output::Json(v) => v,
        Output::Png { .. } => panic!("json"),
    };
    let import = |path: &std::path::Path| {
        json(hub.call("media.import", &json!({"path": path})))["media"]["id"].as_str().unwrap().to_string()
    };
    let (voice_media, song_media) = (import(&voice), import(&song));
    let a1 = json(hub.call("track.add", &json!({"kind": "audio"})))["track"].as_str().unwrap().to_string();
    let a2 = json(hub.call("track.add", &json!({"kind": "audio"})))["track"].as_str().unwrap().to_string();
    let add = |track: &str, media: &str| {
        json(hub.call("clip.add", &json!({"track": track, "media": media, "start": 0})))["clip"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (voice_clip, song_clip) = (add(&a1, &voice_media), add(&a2, &song_media));
    let clip = |id: &str| serde_json::to_value(hub.session().project().clip(id).unwrap().1).unwrap();

    // the tools are operations like any other, with schemas
    for name in ["audio.normalize", "audio.beats", "audio.duck", "audio.denoise"] {
        let op = crate::find_op(name).expect(name);
        assert_eq!(op.schema["type"], "object");
        assert!(!op.read_only);
    }

    // noise: measured from the gaps, kept on the clip, and heard to be gone
    let before = json(hub.call("audio.denoise", &json!({"clip": voice_clip})));
    assert!((before["noiseDb"].as_f64().unwrap() - -47.8).abs() < 3.0, "{before}");
    let kept = clip(&voice_clip)["voice"]["denoise"].clone();
    assert!((kept["amount"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    assert_eq!(kept["floor"].as_array().unwrap().len(), 24);
    let spec = reelforge_compile::compile(&hub.session().project(), &Default::default()).unwrap();
    let cleaned = reelforge_media::export::mix(&spec, &hub.footage, 48_000, 2 * 48_000 + 4800, 48_000).unwrap();
    let level = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    // of the hiss between the words (0.01 / √3, with the song muted here) little is left
    let mut alone = spec.clone();
    alone.audio.retain(|a| a.id == voice_clip);
    let quiet = reelforge_media::export::mix(&alone, &hub.footage, 48_000, 2 * 48_000 + 4800, 48_000).unwrap();
    assert!(level(&quiet[0]) < 0.01 / 3f32.sqrt() * 0.2, "{}", level(&quiet[0]));
    assert!(level(&cleaned[0]) > 0.01, "the song still plays");
    // turned off again, and back with one undo
    json(hub.call("audio.denoise", &json!({"clip": voice_clip, "amount": 0})));
    assert!(clip(&voice_clip).get("voice").is_none());
    hub.call("edit.undo", &json!({})).unwrap();
    assert!((clip(&voice_clip)["voice"]["denoise"]["amount"].as_f64().unwrap() - 0.7).abs() < 1e-6);

    // beats: 120 a minute, a marker on each, replaced when asked again and gone when cleared
    let found = json(hub.call("audio.beats", &json!({"clip": song_clip})));
    assert!((found["bpm"].as_f64().unwrap() - 120.0).abs() < 2.0, "{found}");
    let markers = || hub.session().project().markers.clone();
    assert_eq!(markers().len() as u64, found["beats"].as_u64().unwrap());
    assert!((14..=16).contains(&markers().len()), "{}", markers().len());
    for m in markers() {
        let beat = (m.at.0 as f64 / reelforge_time::TICKS_PER_SECOND as f64 - 0.25) / 0.5;
        assert!((beat - beat.round()).abs() * 0.5 < 0.04 && m.note == "♪", "{:?}", m.at);
    }
    let half = json(hub.call("audio.beats", &json!({"clip": song_clip, "every": 2})));
    assert_eq!(markers().len() as u64, half["beats"].as_u64().unwrap());
    assert!((7..=8).contains(&markers().len()));
    json(hub.call("audio.beats", &json!({"clip": song_clip, "clear": true})));
    assert!(markers().is_empty());
    assert!(matches!(hub.call("audio.beats", &json!({"clip": voice_clip})), Err(CallError::Failed(_))));

    // ducking: the song goes down 12 dB under each of the two stretches of voice, and asking
    // twice does not take it down twice
    for _ in 0..2 {
        let ducked = json(hub.call("audio.duck", &json!({"clip": song_clip})));
        assert_eq!((ducked["spans"].as_u64(), ducked["under"][0].as_str()), (Some(2), Some(voice_clip.as_str())));
        let keys = clip(&song_clip)["keys"]["volumeDb"].as_array().unwrap().clone();
        assert_eq!(keys.len(), 8, "{keys:?}");
        let at = |i: usize| keys[i]["t"].as_i64().unwrap() as f64 / reelforge_time::TICKS_PER_SECOND as f64;
        let values: Vec<f64> = keys.iter().map(|k| k["v"].as_f64().unwrap()).collect();
        assert_eq!(values, [0.0, -12.0, -12.0, 0.0, 0.0, -12.0, -12.0, 0.0]);
        assert!((at(1) - 0.9).abs() < 0.06 && (at(2) - 2.1).abs() < 0.06, "{} {}", at(1), at(2));
        assert!((at(0) - (at(1) - 0.2)).abs() < 0.01 && (at(3) - (at(2) + 0.5)).abs() < 0.01);
    }
    assert!(matches!(hub.call("audio.duck", &json!({"clip": song_clip, "under": []})), Err(CallError::Failed(_))));

    // loudness: both reach the target as they are heard, the ducked one by moving its keys
    let levelled = json(hub.call("audio.normalize", &json!({"clips": [voice_clip], "target": -16})));
    let change = levelled["clips"][0]["changeDb"].as_f64().unwrap();
    assert!(change > 3.0, "{levelled}");
    assert!((clip(&voice_clip)["volume_db"].as_f64().unwrap() - change).abs() < 0.11, "{}", clip(&voice_clip));
    let again = json(hub.call("audio.normalize", &json!({"clips": [voice_clip], "target": -16})));
    assert!(again["clips"][0]["changeDb"].as_f64().unwrap().abs() < 0.3, "{again}");
    assert!((again["clips"][0]["lufs"].as_f64().unwrap() - -16.0).abs() < 0.3);
    let all = json(hub.call("audio.normalize", &json!({})));
    assert_eq!(all["clips"].as_array().unwrap().len(), 2);
    let keys = clip(&song_clip)["keys"]["volumeDb"].as_array().unwrap().clone();
    let song_change =
        all["clips"].as_array().unwrap().iter().find(|c| c["clip"] == song_clip.as_str()).unwrap()["changeDb"]
            .as_f64()
            .unwrap();
    assert!((keys[1]["v"].as_f64().unwrap() - (song_change - 12.0)).abs() < 0.11 && keys.len() == 8);

    // what cannot be heard is said, and nothing changes
    let revision = hub.session().revision();
    assert!(matches!(hub.call("audio.denoise", &json!({"clip": "nope"})), Err(CallError::Failed(_))));
    assert!(matches!(hub.call("audio.normalize", &json!({"target": 3})), Err(CallError::Failed(_))));
    assert_eq!(hub.session().revision(), revision);
}

#[test]
fn a_green_screen_is_found_and_keyed_out() {
    // a presenter (a skin-coloured box) before an unevenly lit green screen
    let shot = temp("presenter.png");
    image::RgbaImage::from_fn(160, 90, |x, y| {
        if (60..100).contains(&x) && y >= 30 {
            image::Rgba([210, 160, 130, 255])
        } else {
            image::Rgba([25 + (x / 8) as u8, 175 + (y / 3) as u8, 55, 255])
        }
    })
    .save(&shot)
    .unwrap();
    // and a holiday snap with no backdrop to speak of
    let snap = temp("snap.png");
    image::RgbaImage::from_fn(160, 90, |x, y| {
        let t = (x as f32 / 160.0 + y as f32 / 90.0) * 9.0;
        let ch = |o: f32| (127.0 + 120.0 * (t + o).sin()) as u8;
        image::Rgba([ch(0.0), ch(2.1), ch(4.2), 255])
    })
    .save(&snap)
    .unwrap();

    let hub = Hub::new(Session::default());
    let json = |out: Result<Output, CallError>| match out.expect("ok") {
        Output::Json(v) => v,
        Output::Png { .. } => panic!("json"),
    };
    let id = |v: Value, key: &str| v[key].as_str().unwrap().to_string();
    json(hub.call("project.setCanvas", &json!({"width": 160, "height": 90})));
    let below =
        hub.session().project().tracks.iter().find(|t| t.kind == reelforge_model::TrackKind::Video).unwrap().id.clone();
    let above = id(json(hub.call("track.add", &json!({"kind": "video"}))), "track");
    // blue fills the frame behind the presenter
    json(hub.call("clip.add", &json!({"track": below, "color": [0, 0, 1], "start": 0, "duration": {"seconds": 4}})));
    let media = |path: &std::path::Path| json(hub.call("media.import", &json!({"path": path})))["media"]["id"].clone();
    let host = id(json(hub.call("clip.add", &json!({"track": above, "media": media(&shot), "start": 0}))), "clip");
    let other = id(
        json(hub.call("clip.add", &json!({"track": above, "media": media(&snap), "start": {"seconds": 10}}))),
        "clip",
    );

    let op = crate::find_op("clip.pickChromaKey").expect("an operation");
    assert!(!op.read_only && op.schema["required"] == json!(["clip"]));
    let frame = || match hub.call("render.frame", &json!({"at": {"seconds": 1}, "scale": 1})).expect("renders") {
        Output::Png { png, .. } => image::load_from_memory(&png).unwrap().to_rgba8(),
        Output::Json(_) => panic!("a picture"),
    };
    let before = frame();
    assert!(before.get_pixel(10, 10)[1] > 150, "green at first");

    let found = json(hub.call("clip.pickChromaKey", &json!({"clip": host})));
    let c: Vec<f64> = found["color"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(c[1] > 0.65 && c[0] < 0.25 && c[2] < 0.3, "{found}");
    assert!(found["share"].as_f64().unwrap() > 0.8, "{found}");
    let key = hub.session().project().clip(&host).unwrap().1.look.chroma.expect("keyed");
    assert!((f64::from(key.color[1]) - c[1]).abs() < 1e-6);
    // the screen is gone, lit evenly or not: blue shows; the presenter is as before
    let after = frame();
    for (x, y) in [(10, 10), (150, 5), (20, 80), (140, 85), (80, 10)] {
        let p = after.get_pixel(x, y);
        assert!(p[2] > 240 && p[1] < 12 && p[0] < 12, "({x}, {y}): {p:?}");
    }
    for (x, y) in [(80, 60), (62, 32), (98, 88)] {
        assert_eq!(after.get_pixel(x, y), before.get_pixel(x, y), "({x}, {y})");
    }
    // one undo takes the key off
    hub.call("edit.undo", &json!({})).unwrap();
    assert!(hub.session().project().clip(&host).unwrap().1.look.chroma.is_none());

    // no backdrop, no key; and a time the clip is not on screen
    for bad in [json!({"clip": other, "at": {"seconds": 11}}), json!({"clip": host, "at": {"seconds": 30}})] {
        assert!(matches!(hub.call("clip.pickChromaKey", &bad), Err(CallError::Failed(_))), "{bad}");
    }
    assert!(hub.session().project().clip(&other).unwrap().1.look.chroma.is_none());
}
