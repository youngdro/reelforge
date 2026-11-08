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

#[test]
fn transcribing_without_a_model_says_where_to_put_one() {
    let hub = Hub::new(Session::default()).with_speech_model(temp("no-model"));
    let err = hub.call("captions.transcribe", &json!({"language": "zh"})).unwrap_err();
    assert!(matches!(&err, CallError::Failed(m) if m.contains("no speech model") && m.contains("no-model")), "{err}");
    // lines from elsewhere still go in
    let out =
        hub.call("captions.add", &json!({"cues": [{"start": 0, "end": {"seconds": 1}, "text": "你好"}]})).unwrap();
    assert!(matches!(out, Output::Json(v) if v["clips"].as_array().unwrap().len() == 1));
}

#[test]
fn an_agent_gets_captions_from_the_timeline_sound() {
    // a model shaped like Whisper (random, tiny): checks the chain, not the hearing
    let model = temp("tiny-whisper");
    reelforge_speech::tiny::write(&model).unwrap();
    let hub = Hub::new(Session::default()).with_speech_model(model);
    // three seconds of a warbling 16 kHz mono tone, on the timeline
    let rate = 16_000u32;
    let n = rate * 3;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + n * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 1, 0], rate.to_le_bytes(), (rate * 2).to_le_bytes(), [2, 0, 16, 0]] {
        wav.extend(v);
    }
    wav.extend(b"data");
    wav.extend((n * 2).to_le_bytes());
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let v = ((t * 300.0 * std::f32::consts::TAU).sin() * (t * 3.0).sin() * 12000.0) as i16;
        wav.extend(v.to_le_bytes());
    }
    let path = temp("speech.wav");
    std::fs::write(&path, &wav).unwrap();
    let Output::Json(m) = hub.call("media.import", &json!({"path": path.to_string_lossy()})).unwrap() else {
        panic!("json")
    };
    let audio = track(&hub, "audio");
    hub.call("clip.add", &json!({"track": audio, "media": m["media"]["id"]})).unwrap();

    let Output::Json(heard) = hub.call("captions.transcribe", &json!({"language": "zh"})).unwrap() else {
        panic!("json")
    };
    assert_eq!(heard["language"], "zh");
    let count = heard["count"].as_u64().unwrap() as usize;
    assert_eq!(heard["clips"].as_array().unwrap().len(), count);
    let p = hub.session().project();
    let captions = p.tracks.iter().find(|t| t.name == reelforge_engine::CAPTIONS_TRACK);
    assert_eq!(captions.map_or(0, |t| t.clips.len()), count);
    for c in captions.iter().flat_map(|t| &t.clips) {
        assert!(c.end() <= reelforge_time::Tick::from_seconds(3.0));
    }
    // parameters are checked before any listening
    let bad = hub.call("captions.transcribe", &json!({"from": "soon"})).unwrap_err();
    assert!(bad.to_string().contains("`from`"), "{bad}");
}
