//! The Model Context Protocol: every [operation](crate::ops) as a tool.
//!
//! [`handle`] answers one JSON-RPC message and knows nothing of transports; [`serve_stdio`] reads
//! newline-delimited messages from stdin (how desktop agents start a local server), and
//! [`crate::http`] carries the same messages over Streamable HTTP for remote agents.
//!
//! A command that refuses (a clip that does not exist, a cut outside a clip) is a tool result with
//! `isError: true` and the reason, so the model can read it and try again; JSON-RPC errors are kept
//! for malformed requests and unknown tools.

use std::io::{BufRead, Write};

use base64::Engine;
use serde_json::{Value, json};

use crate::{CallError, Hub, Output, ops, tool_name};

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "reelforge is a short-video editor. A project has tracks (video, text, audio) holding clips \
on a timeline. Call project_inspect first to see track and clip ids, then edit with the clip_* and track_* tools; \
times are {\"seconds\": s} (or {\"frame\": n}). Every edit is one undo step (edit_undo); edit_batch applies several \
as one. Look at the result with render_frame, and write the video with render_export (or a GIF or the sound \
alone: its `format`; project_setCover picks the cover). Titles render Chinese and Latin text with the bundled Noto \
Sans SC face.";

fn tool(op: &crate::Op) -> Value {
    json!({
        "name": tool_name(op.name),
        "title": op.title,
        "description": op.doc,
        "inputSchema": op.schema,
        "annotations": {
            "title": op.title,
            "readOnlyHint": op.read_only,
            "destructiveHint": op.destructive,
            "openWorldHint": false,
        },
    })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The client's name from an `initialize` request, to say who made each change.
pub fn client_name(msg: &Value) -> Option<String> {
    (msg.get("method")? == "initialize")
        .then(|| msg["params"]["clientInfo"]["name"].as_str().map(|n| format!("agent {n}")))
        .flatten()
}

/// Answers one JSON-RPC message from client `by`. Notifications and responses get `None`.
pub fn handle(hub: &Hub, msg: &Value, by: &str) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(rpc_error(&Value::Null, -32600, "expected a JSON-RPC request object"));
    };
    let Some(id) = obj.get("id").filter(|id| !id.is_null()) else {
        // a notification (initialized, cancelled) or a client's response: nothing to say
        return None;
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // a response to a request we never send
        return None;
    };
    let params = obj.get("params").cloned().unwrap_or(json!({}));
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSIONS[0]);
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "reelforge", "title": "reelforge video editor", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            // commands.list repeats what tools/list says
            let tools: Vec<Value> = ops().iter().filter(|o| o.name != "commands.list").map(tool).collect();
            rpc_result(id, json!({"tools": tools}))
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "tools/call needs a tool `name`"));
            };
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match hub.call_as(by, name, &args) {
                Ok(Output::Json(v)) => {
                    let text = serde_json::to_string_pretty(&v).expect("serialises");
                    let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
                    if v.is_object() {
                        result["structuredContent"] = v;
                    }
                    rpc_result(id, result)
                }
                Ok(Output::Png { png, info }) => rpc_result(
                    id,
                    json!({
                        "content": [
                            {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"},
                            {"type": "text", "text": info.to_string()},
                        ],
                        "structuredContent": info,
                        "isError": false,
                    }),
                ),
                Err(CallError::Unknown(n)) => rpc_error(id, -32602, &format!("unknown tool {n:?}")),
                Err(CallError::Failed(why)) => {
                    rpc_result(id, json!({"content": [{"type": "text", "text": why}], "isError": true}))
                }
            }
        }
        // asked for by some clients even when not advertised
        "resources/list" => rpc_result(id, json!({"resources": []})),
        "prompts/list" => rpc_result(id, json!({"prompts": []})),
        other => rpc_error(id, -32601, &format!("method {other:?} not found")),
    })
}

/// Answers a message as it arrived on the wire: a parse error, a single message, or (older
/// revisions) a batch.
pub fn handle_text(hub: &Hub, text: &str, by: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Err(e) => Some(rpc_error(&Value::Null, -32700, &format!("parse error: {e}"))),
        Ok(Value::Array(batch)) => {
            let out: Vec<Value> = batch.iter().filter_map(|m| handle(hub, m, by)).collect();
            (!out.is_empty()).then_some(Value::Array(out))
        }
        Ok(msg) => handle(hub, &msg, by),
    }
}

/// Serves MCP on stdin and stdout, one JSON message per line, until stdin closes.
pub fn serve_stdio(hub: &Hub) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut by = "agent".to_string();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(name) = serde_json::from_str(&line).ok().as_ref().and_then(client_name) {
            by = name;
        }
        if let Some(reply) = handle_text(hub, &line, &by) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
