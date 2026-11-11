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
    /// List the engine's commands with their parameters, as JSON (`ops` adds rendering, exporting,
    /// listening to the media and speech, which `exec`, `run` and `call` run as well).
    Commands,
    /// Run one operation on a project and print its result: any command, or one `ops` lists (audio.beats,
    /// render.export…); `--save` writes the project back.
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
        /// The file to write: an MP4, or what the project's export settings (or `--format`) say.
        #[arg(short, long)]
        out: PathBuf,
        /// What draws the frames: the GPU when there is one (`auto`), or always `cpu` or `gpu`.
        #[arg(long, default_value = "auto", value_parser = ["auto", "cpu", "gpu"])]
        renderer: String,
        /// The kind of file, whatever the project's export settings say: a video, an animated
        /// GIF, or the sound alone. Projects only.
        #[arg(long, value_parser = ["mp4", "gif", "m4a", "wav"])]
        format: Option<String>,
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
        /// The Kokoro model folder for voice-over (voice.speak); by default
        /// `.reelforge/models/kokoro` in the home folder.
        #[arg(long, env = "REELFORGE_KOKORO")]
        kokoro: Option<PathBuf>,
    },
    /// Download a model from Hugging Face: Whisper for automatic captions (openai/whisper-SIZE:
    /// config.json, model.safetensors and tokenizer.json), or with --kokoro, Kokoro-82M for
    /// voice-over (hexgrad/Kokoro-82M: config.json, kokoro-v1_0.pth and the voices).
    FetchModel {
        /// tiny, base, small, medium, large-v3 or large-v3-turbo. small is a good start for Chinese.
        #[arg(long, default_value = "small")]
        size: String,
        /// Fetch the voice-over model instead.
        #[arg(long)]
        kokoro: bool,
        /// With --kokoro, the voices to fetch (repeat it); four Mandarin ones by default.
        #[arg(long = "voice")]
        voices: Vec<String>,
        /// Where to put it; the folder automatic captions (or voice-over) look in by default.
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
    /// `name=value` pairs (`at=2s` is two seconds, `at=45f` frame 45, `source=@card.js` the text of
    /// that file; JSON values pass as they are).
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
    // exports use the system's (hardware) encoder where there is one
    reelforge_hwenc::register();
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

/// Runs any operation (an engine command, or one that needs the media: the listening tools, a
/// transcription, an export) on a project held in memory, for `exec` and `run`.
fn exec(hub: &Hub, id: &str, params: &Value) -> Result<Value, String> {
    match hub.call(id, params).map_err(|e| e.to_string())? {
        Output::Json(v) => Ok(v),
        Output::Png { .. } => Err(format!("{id} draws a picture: use `call {id} … --project FILE -o out.png`")),
    }
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
            let hub = Hub::new(open(&project)?);
            let params: Value = serde_json::from_str(&params).map_err(|e| format!("params are not JSON: {e}"))?;
            print(&exec(&hub, &id, &params)?);
            if save {
                write(&project, &hub.session().project().to_json())?;
            }
            Ok(())
        }
        Cmd::Run { project, script, save } => {
            let hub = Hub::new(open(&project)?);
            let text = read(&script)?;
            for (n, line) in text.as_bytes().lines().enumerate() {
                let line = line.map_err(|e| e.to_string())?;
                if line.trim().is_empty() {
                    continue;
                }
                let step: Value = serde_json::from_str(&line).map_err(|e| format!("line {}: {e}", n + 1))?;
                let id = step.get("id").and_then(Value::as_str).ok_or(format!("line {}: no \"id\"", n + 1))?;
                let out = exec(&hub, id, step.get("params").unwrap_or(&Value::Null))
                    .map_err(|e| format!("line {} ({id}): {e}", n + 1))?;
                println!("{}", json!({"line": n + 1, "id": id, "result": out}));
            }
            if save {
                write(&project, &hub.session().project().to_json())?;
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
        Cmd::Export { input, out, renderer, format } => {
            let renderer = match renderer.as_str() {
                "cpu" => reelforge_media::Renderer::Cpu,
                "gpu" => reelforge_media::Renderer::Gpu,
                _ => reelforge_media::Renderer::Auto,
            };
            let text = read(&input)?;
            let is_spec = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?.get("spec").is_some();
            let mut project = None;
            let spec = if is_spec {
                if format.is_some() {
                    return Err("--format is for projects: a RenderSpec says its own (`output.format`)".into());
                }
                RenderSpec::from_json(&text).map_err(|e| e.to_string())?
            } else {
                let mut p = Project::from_json(&text).map_err(|e| e.to_string())?;
                if let Some(f) = format.as_deref().and_then(reelforge_model::export::Format::parse) {
                    p.export.format = f;
                }
                let spec = reelforge_compile::compile(&p, &reelforge_compile::CompileOptions { encode: true })
                    .map_err(|e| e.to_string())?;
                project = Some(p);
                spec
            };
            let tmp = reelforge_server::part_of(&out);
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
            let started = std::time::Instant::now();
            let mut shown = 0;
            // the sound alone is mixed a second at a time
            let unit = if spec.output.video.is_some() { "frame" } else { "second" };
            let result = reelforge_media::export_with(&spec, &Footage::new(), file, renderer, &mut |p| {
                let pct = p.done * 100 / p.total;
                if pct >= shown + 10 || p.done == p.total {
                    shown = pct;
                    eprintln!("{pct:>3}%  {unit} {}/{}", p.done, p.total);
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
            // a video's cover goes beside it
            let cover = match &project {
                Some(p) if p.export.format.pictured() => reelforge_server::write_cover(p, &Footage::new(), &out)?,
                _ => None,
            };
            print(&json!({
                "out": out,
                "format": spec.output.format.extension(),
                "cover": cover,
                "frames": stats.frames,
                "audioSamples": stats.audio_samples,
                "renderer": stats.renderer,
                "encoder": stats.encoder,
                "seconds": started.elapsed().as_secs_f64(),
            }));
            Ok(())
        }
        Cmd::Schema => {
            print(&reelforge_renderspec::json_schema());
            Ok(())
        }
        Cmd::FetchModel { kokoro: true, voices, to, .. } => {
            fetch_kokoro(&voices, &to.unwrap_or_else(reelforge_voice::default_model_dir))
        }
        Cmd::FetchModel { size, to, .. } => fetch_model(&size, &to.unwrap_or_else(reelforge_speech::default_model_dir)),
        Cmd::Serve { project, stdio, listen, token, web, whisper, kokoro } => {
            let mut hub = match &project {
                Some(path) => Hub::open(path)?,
                None => Hub::new(Session::default()),
            };
            if let Some(dir) = whisper {
                hub = hub.with_speech_model(dir);
            }
            if let Some(dir) = kokoro {
                hub = hub.with_voice_model(dir);
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
        // `source=@card.js`: the text of a file, as curl reads one
        let value = match v.strip_prefix('@') {
            Some(path) => json!(std::fs::read_to_string(path).map_err(|e| format!("{k}: cannot read {path}: {e}"))?),
            None => param_value(v),
        };
        obj.insert(k.to_string(), value);
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

/// Downloads `openai/whisper-{size}` into `dir`.
fn fetch_model(size: &str, dir: &std::path::Path) -> Result<(), String> {
    if !size.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Err(format!("not a model size: {size:?}"));
    }
    let files = ["config.json", "tokenizer.json", "model.safetensors"].map(String::from);
    download(&format!("https://huggingface.co/openai/whisper-{size}/resolve/main"), &files, dir)?;
    eprintln!("Whisper {size} is in {}", dir.display());
    Ok(())
}

/// Downloads `hexgrad/Kokoro-82M` with `voices` (four Mandarin ones when empty) into `dir`.
fn fetch_kokoro(voices: &[String], dir: &std::path::Path) -> Result<(), String> {
    let mandarin = ["zf_xiaoxiao", "zf_xiaoyi", "zm_yunxi", "zm_yunjian"].map(String::from);
    let voices = if voices.is_empty() { &mandarin[..] } else { voices };
    if let Some(v) = voices.iter().find(|v| v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return Err(format!("not a voice name: {v:?}"));
    }
    let mut files = vec!["config.json".to_string(), "kokoro-v1_0.pth".to_string()];
    files.extend(voices.iter().map(|v| format!("voices/{v}.pt")));
    download("https://huggingface.co/hexgrad/Kokoro-82M/resolve/main", &files, dir)?;
    eprintln!("Kokoro-82M is in {}", dir.display());
    Ok(())
}

/// Downloads `base/FILE` to `dir/FILE` for each of `files`, each through a `.part` so a broken
/// download never looks complete.
fn download(base: &str, files: &[String], dir: &std::path::Path) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(true).build().into();
    for file in files {
        let url = format!("{base}/{file}");
        eprintln!("fetching {url}");
        let resp = agent.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
        let out = dir.join(file);
        let folder = out.parent().expect("a file in dir");
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot make {}: {e}", folder.display()))?;
        let part = out.with_extension("part");
        let mut f = std::fs::File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        let n = std::io::copy(&mut resp.into_body().into_reader(), &mut f).map_err(|e| format!("{url}: {e}"))?;
        std::fs::rename(&part, &out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        eprintln!("  {} MB", n / 1_000_000);
    }
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
