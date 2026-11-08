//! `reelforge-cli`: the engine and the renderer from a shell.
//!
//! ```sh
//! reelforge-cli new demo.reel.json --demo
//! reelforge-cli exec demo.reel.json clip.split '{"clip": "c4", "at": {"seconds": 2}}' --save
//! reelforge-cli compile demo.reel.json -o demo.renderspec.json
//! reelforge-cli frame demo.renderspec.json --seconds 1.5 --out frame.png
//! reelforge-cli export demo.reel.json -o demo.mp4
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
    },
    /// Print the RenderSpec JSON Schema.
    Schema,
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
        Cmd::Export { input, out } => {
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
            let result = reelforge_media::export(&spec, &Footage::new(), file, &mut |p| {
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
                "seconds": started.elapsed().as_secs_f64(),
            }));
            Ok(())
        }
        Cmd::Schema => {
            print(&reelforge_renderspec::json_schema());
            Ok(())
        }
    }
}
