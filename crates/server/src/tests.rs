use std::sync::Arc;

use base64::Engine;
use reelforge_engine::Session;
use serde_json::{Value, json};

use crate::http::HttpServer;
use crate::{CallError, Hub, Output, mcp, ops};

fn rpc(hub: &Hub, id: i64, method: &str, params: Value) -> Value {
    mcp::handle(hub, &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).expect("a reply")
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
    assert!(mcp::handle(&hub, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());

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
