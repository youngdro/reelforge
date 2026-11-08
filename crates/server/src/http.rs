//! HTTP: MCP's Streamable HTTP transport at `/mcp`, and a plain API at `/v1` for programs that
//! just want to call operations (the CLI's `--server` mode uses it).
//!
//! | Request | Does |
//! |---|---|
//! | `POST /mcp` | One JSON-RPC message (or batch); the answer comes back as JSON. |
//! | `DELETE /mcp` | Ends the MCP session named by `Mcp-Session-Id`. |
//! | `GET /v1/ops` | Every operation with its parameter schema. |
//! | `POST /v1/ops/{name}` | Runs one; the body is its parameters. JSON back, or `image/png` (with `X-Reelforge-Info`). |
//! | `GET /v1/project` | `project.inspect`. |
//! | `GET /v1/events?since=N` | Waits (up to `timeout` s, 25 by default) for a revision after `N`: `{revision, command, by}`. |
//! | `GET /health` | `ok`, without a token. |
//!
//! Safety: a server on a loopback address checks that browser requests come from a local page
//! (`Origin`), against DNS rebinding. Anywhere else it needs a token, sent as
//! `Authorization: Bearer <token>`; it refuses to start without one.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{CallError, Hub, Output, mcp, ops, tool_name};

/// A bound, not yet running, HTTP server.
pub struct HttpServer {
    server: Server,
    hub: Arc<Hub>,
    token: Option<String>,
    /// MCP sessions and their clients' names.
    sessions: Mutex<HashMap<String, String>>,
}

impl HttpServer {
    /// Binds `addr` (`127.0.0.1:7878`; port 0 picks one). A non-loopback address needs `token`.
    pub fn bind(hub: Arc<Hub>, addr: &str, token: Option<String>) -> Result<HttpServer, String> {
        let parsed: SocketAddr = addr.parse().map_err(|e| format!("{addr:?} is not an address: {e}"))?;
        if !parsed.ip().is_loopback() && token.as_deref().is_none_or(str::is_empty) {
            return Err(format!(
                "{addr} is reachable from other machines: set a token (--token or REELFORGE_TOKEN) so only your clients can edit"
            ));
        }
        let server = Server::http(parsed).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
        Ok(HttpServer { server, hub, token, sessions: Mutex::default() })
    }

    /// The address it listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.server.server_addr().to_ip().expect("bound to an IP address")
    }

    /// Answers requests until the process ends, each on its own thread.
    pub fn run(self) {
        let this = Arc::new(self);
        for req in this.server.incoming_requests() {
            let this = this.clone();
            std::thread::spawn(move || this.answer(req));
        }
    }

    fn answer(&self, mut req: Request) {
        let (url, query) = req.url().split_once('?').unwrap_or((req.url(), ""));
        let (url, query) = (url.trim_end_matches('/').to_string(), query.to_string());
        let method = req.method().clone();
        let header = |name: &str| {
            req.headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
                .map(|h| h.value.as_str().to_string())
        };
        let (origin, auth, session) = (header("Origin"), header("Authorization"), header("Mcp-Session-Id"));
        if url == "/health" {
            return reply(req, text(200, "ok"));
        }
        match &self.token {
            Some(t) if auth.as_deref() != Some(&format!("Bearer {t}")) => {
                return reply(req, json_response(401, &json!({"error": "missing or wrong bearer token"})));
            }
            None if origin.as_deref().is_some_and(|o| !local_origin(o)) => {
                return reply(req, json_response(403, &json!({"error": "requests from other sites are refused"})));
            }
            _ => {}
        }
        let mut body = String::new();
        if req.as_reader().read_to_string(&mut body).is_err() {
            return reply(req, json_response(400, &json!({"error": "the body is not UTF-8"})));
        }
        let response = match (method, url.as_str()) {
            (Method::Post, "/mcp") => self.mcp(&body, session),
            (Method::Delete, "/mcp") => {
                let known = session.is_some_and(|s| self.sessions.lock().expect("sessions").remove(&s).is_some());
                text(if known { 200 } else { 404 }, "")
            }
            (Method::Get, "/mcp") => {
                text(405, "this server sends no unprompted messages").with_header(h("Allow", "POST, DELETE"))
            }
            (Method::Get, "/v1/ops") => {
                let list: Vec<Value> = ops()
                    .iter()
                    .map(|o| {
                        json!({"name": o.name, "tool": tool_name(o.name), "title": o.title, "doc": o.doc,
                               "schema": o.schema, "readOnly": o.read_only, "destructive": o.destructive})
                    })
                    .collect();
                json_response(200, &Value::Array(list))
            }
            (Method::Get, "/v1/project") => self.op("project.inspect", "{}"),
            (Method::Get, "/v1/events") => {
                let arg = |k: &str| {
                    query.split('&').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=')?.parse::<u64>().ok())
                };
                let wait = std::time::Duration::from_secs(arg("timeout").unwrap_or(25).min(60));
                let c = self.hub.session().wait_for_change(arg("since").unwrap_or(0), wait);
                json_response(200, &json!({"revision": c.revision, "command": c.command, "by": c.by}))
            }
            (Method::Post, path) if path.starts_with("/v1/ops/") => self.op(&path["/v1/ops/".len()..], &body),
            _ => json_response(404, &json!({"error": format!("no route {url}")})),
        };
        reply(req, response)
    }

    fn mcp(&self, body: &str, session: Option<String>) -> Response<std::io::Cursor<Vec<u8>>> {
        let by = match &session {
            Some(s) => match self.sessions.lock().expect("sessions").get(s) {
                Some(name) => name.clone(),
                None => {
                    return json_response(404, &json!({"error": "unknown or ended MCP session; initialize again"}));
                }
            },
            None => "agent".to_string(),
        };
        let msg = serde_json::from_str::<Value>(body).ok();
        let initializing = msg.as_ref().is_some_and(|v| v.get("method").and_then(Value::as_str) == Some("initialize"));
        match mcp::handle_text(&self.hub, body, &by) {
            None => text(202, ""),
            Some(answer) => {
                let mut r = json_response(200, &answer);
                if initializing && answer.get("result").is_some() {
                    let id = new_session_id();
                    let name = msg.as_ref().and_then(mcp::client_name).unwrap_or_else(|| "agent".into());
                    self.sessions.lock().expect("sessions").insert(id.clone(), name);
                    r = r.with_header(h("Mcp-Session-Id", &id));
                }
                r
            }
        }
    }

    fn op(&self, name: &str, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        let params: Value = if body.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return json_response(400, &json!({"error": format!("the body is not JSON: {e}")})),
            }
        };
        match self.hub.call_as("http", name, &params) {
            Ok(Output::Json(v)) => json_response(200, &json!({"result": v})),
            Ok(Output::Png { png, info }) => Response::from_data(png)
                .with_status_code(200)
                .with_header(h("Content-Type", "image/png"))
                .with_header(h("X-Reelforge-Info", &info.to_string())),
            Err(CallError::Unknown(n)) => json_response(404, &json!({"error": format!("unknown operation {n:?}")})),
            Err(CallError::Failed(why)) => json_response(422, &json!({"error": why})),
        }
    }
}

fn local_origin(origin: &str) -> bool {
    let host = origin.split("://").nth(1).unwrap_or(origin);
    let host =
        host.rsplit_once(':').map_or(host, |(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { host });
    matches!(host, "localhost" | "127.0.0.1" | "[::1]") || origin == "null"
}

fn new_session_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    // two independently seeded hashers: unguessable enough to name a session, which is not a secret
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn h(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("ASCII header")
}

fn text(status: u16, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(body.as_bytes().to_vec())
        .with_status_code(status)
        .with_header(h("Content-Type", "text/plain; charset=utf-8"))
}

fn json_response(status: u16, v: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(serde_json::to_vec(v).expect("serialises"))
        .with_status_code(status)
        .with_header(h("Content-Type", "application/json"))
}

fn reply(req: Request, r: Response<std::io::Cursor<Vec<u8>>>) {
    // the client may have gone; nothing to do about it
    let _ = req.respond(r);
}
