//! `reelforge-cli`: the engine and the renderer from a shell.
//!
//! ```sh
//! reelforge-cli new demo.reel.json --demo
//! reelforge-cli exec demo.reel.json clip.split '{"clip": "c4", "at": {"seconds": 2}}' --save
//! reelforge-cli compile demo.reel.json -o demo.renderspec.json
//! reelforge-cli frame demo.renderspec.json --seconds 1.5 --out frame.png
//! reelforge-cli export demo.reel.json -o demo.mp4 [--renderer auto|cpu|gpu]
//!
//! # the same operations an MCP agent has, on a file or on a running server
//! reelforge-cli call clip.split clip=c4 at=2s --project demo.reel.json
//! reelforge-cli serve demo.reel.json --listen 127.0.0.1:7878      # MCP at /mcp, API at /v1
//! reelforge-cli call render.frame at=1.5s --server http://127.0.0.1:7878 -o frame.png
//! reelforge-cli serve demo.reel.json --stdio                      # MCP for a local agent
//! ```

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use reelforge_engine::Session;
use reelforge_media::Footage;
use reelforge_model::Project;
use reelforge_render::{Quality, render_frame};
use reelforge_renderspec::RenderSpec;
use reelforge_server::{Hub, Output};
use reelforge_time::Tick;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "reelforge-cli", version, about = "reelforge without a window")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a new project (empty, or the demo with --demo).
    New {
        /// Where to write it.
        project: PathBuf,
        /// Start from the demo project.
        #[arg(long)]
        demo: bool,
    },
    /// List every command with its parameters, as JSON.
    Commands,
    /// Run one command on a project and print its result.
    Exec {
        /// The project file.
        project: PathBuf,
        /// Command id, such as clip.split.
        id: String,
        /// Parameters as a JSON object.
        #[arg(default_value = "{}")]
        params: String,
        /// Write the project back afterwards.
        #[arg(long)]
        save: bool,
    },
    /// Run a script of commands, one JSON object `{"id", "params"}` per line (`-` for stdin).
    Run {
        /// The project file.
        project: PathBuf,
        /// The script.
        script: PathBuf,
        /// Write the project back afterwards.
        #[arg(long)]
        save: bool,
    },
    /// Compile a project to a RenderSpec.
    Compile {
        /// The project file.
        project: PathBuf,
        /// Where to write the spec (stdout without it).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Check a RenderSpec and list the features it needs.
    Validate {
        /// The spec.
        spec: PathBuf,
    },
    /// Render one frame of a RenderSpec to PNG.
    Frame {
        /// The spec.
        spec: PathBuf,
        /// The instant, in seconds.
        #[arg(long, default_value_t = 0.0)]
        seconds: f64,
        /// The PNG to write.
        #[arg(long)]
        out: PathBuf,
        /// Size relative to the output (0.5 is half).
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Draw placeholders for what this build cannot draw instead of failing.
        #[arg(long)]
        draft: bool,
    },
    /// Render a project or a RenderSpec to an MP4 (H.264 + AAC).
    Export {
        /// A project, or a RenderSpec (`spec` field) as a render service would receive it.
        input: PathBuf,
        /// The MP4 to write.
        #[arg(short, long)]
        out: PathBuf,
        /// What draws the frames: the GPU when there is one (`auto`), or always `cpu` or `gpu`.
        #[arg(long, default_value = "auto", value_parser = ["auto", "cpu", "gpu"])]
        renderer: String,
    },
    /// Print the RenderSpec JSON Schema.
    Schema,
    /// Serve a project to MCP agents (stdio, or Streamable HTTP at /mcp) and HTTP clients (/v1).
    /// Every edit is saved to the project file as it commits.
    Serve {
        /// The project file (created when missing). Without one the project lives in memory.
        project: Option<PathBuf>,
        /// Speak MCP on stdin/stdout instead of HTTP: how a local agent starts the server.
        #[arg(long)]
        stdio: bool,
        /// Address to listen on. Anything but loopback needs --token.
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: String,
        /// Bearer token clients must send.
        #[arg(long, env = "REELFORGE_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Serve the browser editor from this folder (the built `reelforge-web`), so opening the
        /// server's address in a browser edits this project.
        #[arg(long, env = "REELFORGE_WEB")]
        web: Option<PathBuf>,
        /// The Whisper model folder for automatic captions (captions.transcribe); by default
        /// `.reelforge/models/whisper` in the home folder.
        #[arg(long, env = "REELFORGE_WHISPER")]
        whisper: Option<PathBuf>,
    },
    /// Download a Whisper model for automatic captions from Hugging Face (openai/whisper-SIZE):
    /// config.json, model.safetensors and tokenizer.json.
    FetchModel {
        /// tiny, base, small, medium, large-v3 or large-v3-turbo. small is a good start for Chinese.
        #[arg(long, default_value = "small")]
        size: String,
        /// Where to put it; the folder automatic captions look in by default.
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// List the operations `call`, MCP agents and the HTTP API share, with their parameters.
    Ops {
        /// Ask a running server instead.
        #[arg(long, env = "REELFORGE_SERVER")]
        server: Option<String>,
        /// Bearer token for the server.
        #[arg(long, env = "REELFORGE_TOKEN", hide_env_values = true)]
        token: Option<String>,
    },
    /// Call one operation, on a project file or a running server. Parameters are one JSON object or
    /// `name=value` pairs (`at=2s` is two seconds, `at=45f` frame 45; JSON values pass as they are).
    Call {
        /// Operation, such as clip.split (or its tool name, clip_split).
        op: String,
        /// Parameters.
        params: Vec<String>,
        /// The project file to edit (saved after every edit).
        #[arg(short, long, conflicts_with = "server")]
        project: Option<PathBuf>,
        /// A running `reelforge-cli serve`, such as http://127.0.0.1:7878.
        #[arg(long, env = "REELFORGE_SERVER")]
        server: Option<String>,
        /// Bearer token for the server.
        #[arg(long, env = "REELFORGE_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Where to write a picture result (render.frame).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
        return Ok(s);
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    // Atomic: write a sibling, then rename over the target.
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn open(path: &Path) -> Result<Session, String> {
    Ok(Session::new(Project::from_json(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?))
}

fn print(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).expect("serialises"));
}

fn run(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::New { project, demo } => {
            let mut s = Session::default();
            if demo {
                let p = s.project();
                let (video, text) = (p.tracks[0].id.clone(), p.tracks[1].id.clone());
                s.execute("edit.batch", &json!({"steps": [
                    {"id": "clip.add", "params": {"track": video, "color": [0.12, 0.10, 0.30], "duration": {"seconds": 6}}},
                    {"id": "clip.add", "params": {"track": video, "color": [0.85, 0.35, 0.20], "duration": {"seconds": 4}}},
                    {"id": "clip.add", "params": {"track": text, "text": "reelforge", "size": 140, "start": {"seconds": 1}, "duration": {"seconds": 4}}},
                ]}))
                .map_err(|e| e.to_string())?;
            }
            write(&project, &s.project().to_json())
        }
        Cmd::Commands => {
            print(&Session::default().execute("commands.list", &json!({})).map_err(|e| e.to_string())?);
            Ok(())
        }
        Cmd::Exec { project, id, params, save } => {
            let mut s = open(&project)?;
            let params: Value = serde_json::from_str(&params).map_err(|e| format!("params are not JSON: {e}"))?;
            print(&s.execute(&id, &params).map_err(|e| e.to_string())?);
            if save {
                write(&project, &s.project().to_json())?;
            }
            Ok(())
        }
        Cmd::Run { project, script, save } => {
            let mut s = open(&project)?;
            let text = read(&script)?;
            for (n, line) in text.as_bytes().lines().enumerate() {
                let line = line.map_err(|e| e.to_string())?;
                if line.trim().is_empty() {
                    continue;
                }
                let step: Value = serde_json::from_str(&line).map_err(|e| format!("line {}: {e}", n + 1))?;
                let id = step.get("id").and_then(Value::as_str).ok_or(format!("line {}: no \"id\"", n + 1))?;
                let out = s
                    .execute(id, step.get("params").unwrap_or(&Value::Null))
                    .map_err(|e| format!("line {} ({id}): {e}", n + 1))?;
                println!("{}", json!({"line": n + 1, "id": id, "result": out}));
            }
            if save {
                write(&project, &s.project().to_json())?;
            }
            Ok(())
        }
        Cmd::Compile { project, out } => {
            let mut s = open(&project)?;
            let spec = s.execute("render.compile", &json!({})).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&spec).expect("serialises") + "\n";
            match out {
                Some(path) => write(&path, &text),
                None => {
                    print!("{text}");
                    Ok(())
                }
            }
        }
        Cmd::Validate { spec } => {
            let spec = RenderSpec::from_json(&read(&spec)?).map_err(|e| e.to_string())?;
            let missing = reelforge_render::unsupported_features(&spec);
            print(&json!({"valid": true, "features": spec.required_features(), "unsupportedByThisRenderer": missing}));
            Ok(())
        }
        Cmd::Frame { spec, seconds, out, scale, draft } => {
            let spec = RenderSpec::from_json(&read(&spec)?).map_err(|e| e.to_string())?;
            if !(scale > 0.0 && scale <= 4.0) {
                return Err("--scale must be above 0 and at most 4".into());
            }
            let t = spec.output.fps.snap(Tick::from_seconds(seconds));
            let quality = if draft { Quality::Draft } else { Quality::Final };
            let (img, report) = render_frame(&spec, t, scale, &Footage::new(), quality).map_err(|e| e.to_string())?;
            image::save_buffer(&out, &img.to_srgba8(), img.width, img.height, image::ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
            for (layer, why) in report.placeholders {
                eprintln!("placeholder for {layer}: {why}");
            }
            Ok(())
        }
        Cmd::Export { input, out, renderer } => {
            let renderer = match renderer.as_str() {
                "cpu" => reelforge_media::Renderer::Cpu,
                "gpu" => reelforge_media::Renderer::Gpu,
                _ => reelforge_media::Renderer::Auto,
            };
            let text = read(&input)?;
            let is_spec = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?.get("spec").is_some();
            let spec = if is_spec {
                RenderSpec::from_json(&text).map_err(|e| e.to_string())?
            } else {
                let p = Project::from_json(&text).map_err(|e| e.to_string())?;
                reelforge_compile::compile(&p, &reelforge_compile::CompileOptions { encode: true })
                    .map_err(|e| e.to_string())?
            };
            let tmp = out.with_extension("mp4.part");
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
            let started = std::time::Instant::now();
            let mut shown = 0;
            let result = reelforge_media::export_with(&spec, &Footage::new(), file, renderer, &mut |p| {
                let pct = p.done * 100 / p.total;
                if pct >= shown + 10 || p.done == p.total {
                    shown = pct;
                    eprintln!("{pct:>3}%  frame {}/{}", p.done, p.total);
                }
                true
            });
            let stats = match result {
                Ok((_, stats)) => stats,
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e.to_string());
                }
            };
            std::fs::rename(&tmp, &out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
            print(&json!({
                "out": out,
                "frames": stats.frames,
                "audioSamples": stats.audio_samples,
                "renderer": stats.renderer,
                "seconds": started.elapsed().as_secs_f64(),
            }));
            Ok(())
        }
        Cmd::Schema => {
            print(&reelforge_renderspec::json_schema());
            Ok(())
        }
        Cmd::FetchModel { size, to } => fetch_model(&size, &to.unwrap_or_else(reelforge_speech::default_model_dir)),
        Cmd::Serve { project, stdio, listen, token, web, whisper } => {
            let mut hub = match &project {
                Some(path) => Hub::open(path)?,
                None => Hub::new(Session::default()),
            };
            if let Some(dir) = whisper {
                hub = hub.with_speech_model(dir);
            }
            if stdio {
                return reelforge_server::mcp::serve_stdio(&hub).map_err(|e| e.to_string());
            }
            let mut server =
                reelforge_server::http::HttpServer::bind(std::sync::Arc::new(hub), &listen, token.clone())?;
            let addr = server.local_addr();
            eprintln!("reelforge on http://{addr}: MCP at /mcp, operations at /v1/ops");
            if let Some(dir) = web {
                if !dir.join("index.html").is_file() {
                    return Err(format!("{} has no index.html: build reelforge-web into it first", dir.display()));
                }
                server = server.with_web(dir);
                let query = token.map(|t| format!("/?token={t}")).unwrap_or_default();
                eprintln!("edit in a browser at http://{addr}{query}");
            }
            if let Some(p) = &project {
                eprintln!("editing {} (every edit is saved)", p.display());
            }
            server.run();
            Ok(())
        }
        Cmd::Ops { server, token } => match server {
            Some(url) => {
                let (body, _) = remote(&url, token.as_deref(), "GET", "/v1/ops", "")?;
                print(&serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?);
                Ok(())
            }
            None => {
                let list: Vec<Value> = reelforge_server::ops()
                    .iter()
                    .map(|o| json!({"name": o.name, "tool": reelforge_server::tool_name(o.name), "title": o.title, "doc": o.doc, "schema": o.schema, "readOnly": o.read_only}))
                    .collect();
                print(&Value::Array(list));
                Ok(())
            }
        },
        Cmd::Call { op, params, project, server, token, out } => {
            let params = call_params(&params)?;
            let output = match (project, server) {
                (Some(path), _) => {
                    let hub = Hub::open(&path)?;
                    hub.call(&op, &params).map_err(|e| e.to_string())?
                }
                (None, Some(url)) => {
                    let (body, info) =
                        remote(&url, token.as_deref(), "POST", &format!("/v1/ops/{op}"), &params.to_string())?;
                    match info {
                        Some(info) => {
                            Output::Png { png: body, info: serde_json::from_str(&info).unwrap_or(Value::Null) }
                        }
                        None => {
                            let v: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                            Output::Json(v["result"].clone())
                        }
                    }
                }
                (None, None) => {
                    return Err("pass --project FILE, or --server URL (or set REELFORGE_SERVER)".into());
                }
            };
            match output {
                Output::Json(v) => print(&v),
                Output::Png { png, info } => {
                    let Some(out) = out else {
                        return Err("this result is a picture: pass -o FILE.png".into());
                    };
                    std::fs::write(&out, png).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
                    print(&json!({"out": out, "frame": info}));
                }
            }
            Ok(())
        }
    }
}

/// Parameters from the command line: one JSON object, or `name=value` pairs.
fn call_params(args: &[String]) -> Result<Value, String> {
    if let [one] = args
        && one.trim_start().starts_with('{')
    {
        return serde_json::from_str(one).map_err(|e| format!("params are not JSON: {e}"));
    }
    let mut obj = serde_json::Map::new();
    for a in args {
        let (k, v) =
            a.split_once('=').ok_or_else(|| format!("{a:?}: parameters are name=value, or one JSON object"))?;
        obj.insert(k.to_string(), param_value(v));
    }
    Ok(Value::Object(obj))
}

/// `2s` and `1.5s` are seconds, `45f` a frame; JSON (numbers, arrays, objects, true) passes
/// through; anything else is a string.
fn param_value(v: &str) -> Value {
    if let Some(n) = v.strip_suffix('s').and_then(|n| n.parse::<f64>().ok()) {
        return json!({"seconds": n});
    }
    if let Some(n) = v.strip_suffix('f').and_then(|n| n.parse::<i64>().ok()) {
        return json!({"frame": n});
    }
    serde_json::from_str(v).unwrap_or_else(|_| json!(v))
}

/// Downloads `openai/whisper-{size}` into `dir`, each file through a `.part` so a broken download
/// never looks complete.
fn fetch_model(size: &str, dir: &std::path::Path) -> Result<(), String> {
    if !size.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Err(format!("not a model size: {size:?}"));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(true).build().into();
    for file in ["config.json", "tokenizer.json", "model.safetensors"] {
        let url = format!("https://huggingface.co/openai/whisper-{size}/resolve/main/{file}");
        eprintln!("fetching {url}");
        let resp = agent.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
        let out = dir.join(file);
        let part = out.with_extension("part");
        let mut f = std::fs::File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        let n = std::io::copy(&mut resp.into_body().into_reader(), &mut f).map_err(|e| format!("{url}: {e}"))?;
        std::fs::rename(&part, &out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        eprintln!("  {} MB", n / 1_000_000);
    }
    eprintln!("Whisper {size} is in {}", dir.display());
    Ok(())
}

/// A request to a running server: the body, and for pictures their `X-Reelforge-Info`.
fn remote(
    base: &str,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(Vec<u8>, Option<String>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let auth = token.map(|t| format!("Bearer {t}"));
    let result = if method == "GET" {
        let mut r = agent.get(&url);
        if let Some(a) = &auth {
            r = r.header("Authorization", a);
        }
        r.call()
    } else {
        let mut r = agent.post(&url).header("Content-Type", "application/json");
        if let Some(a) = &auth {
            r = r.header("Authorization", a);
        }
        r.send(body)
    };
    let mut resp = result.map_err(|e| format!("cannot reach {base}: {e}"))?;
    let status = resp.status().as_u16();
    let info = resp.headers().get("X-Reelforge-Info").and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = resp.body_mut().read_to_vec().map_err(|e| e.to_string())?;
    if status >= 400 {
        let msg = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        return Err(format!("{status}: {msg}"));
    }
    Ok((bytes, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_parameters_read_like_a_shell() {
        let args = |a: &[&str]| call_params(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(
            args(&["clip=c4", "at=1.5s", "end=45f", "speed=2", "color=[1,0,0]", "text=你好", "draft=false"]),
            json!({"clip": "c4", "at": {"seconds": 1.5}, "end": {"frame": 45}, "speed": 2, "color": [1, 0, 0], "text": "你好", "draft": false})
        );
        assert_eq!(args(&[r#"{"clip": "c4"}"#]), json!({"clip": "c4"}));
        assert_eq!(args(&[]), json!({}));
        assert!(call_params(&["oops".to_string()]).is_err());
    }
}
