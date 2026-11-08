//! reelforge as a service.
//!
//! A [`Hub`] holds one editing session that any number of clients share: MCP agents (over stdio or
//! Streamable HTTP, [`mcp`]), scripts and other programs (the plain HTTP API, [`http`]), and the
//! CLI (`reelforge-cli call`, locally or against a server). They all call the same
//! [`operations`](ops): every engine command, plus the few things that need media or the disk
//! (render a frame, export an MP4, save), so an agent and a person see one vocabulary.
//!
//! Edits still go through `Session::execute`; this crate only carries them. With a project file,
//! every edit is saved as it commits, so the file is always the current state.

pub mod http;
pub mod mcp;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use reelforge_engine::{EngineError, Session, SharedSession, input_schema, parse_time, registry};
use reelforge_media::{Footage, Renderer};
use reelforge_model::Project;
use reelforge_render::{Quality, render_frame};
use reelforge_time::Tick;
use serde_json::{Value, json};

/// What an operation returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// A JSON value.
    Json(Value),
    /// A PNG picture, and facts about it.
    Png {
        /// The encoded file.
        png: Vec<u8>,
        /// Size, instant, placeholders.
        info: Value,
    },
}

/// Why an operation did not run.
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
    /// No operation has this name.
    Unknown(String),
    /// The operation ran and refused, or its parameters are wrong; nothing changed.
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Unknown(n) => write!(f, "unknown operation {n:?}"),
            CallError::Failed(m) => f.write_str(m),
        }
    }
}

impl From<EngineError> for CallError {
    fn from(e: EngineError) -> CallError {
        match e {
            EngineError::Unknown(id) => CallError::Unknown(id),
            e => CallError::Failed(e.to_string()),
        }
    }
}

/// One operation a client can call.
#[derive(Clone, Debug)]
pub struct Op {
    /// Dotted id, `clip.split`. MCP tools use it with `_` for `.` (`clip_split`); either is accepted.
    pub name: &'static str,
    /// Short human title.
    pub title: &'static str,
    /// What it does.
    pub doc: String,
    /// JSON Schema of its parameters.
    pub schema: Value,
    /// It changes nothing.
    pub read_only: bool,
    /// It can remove work (a person may want to confirm).
    pub destructive: bool,
}

const RENDER_FRAME: &str = "render.frame";
const RENDER_EXPORT: &str = "render.export";
const PROJECT_SAVE: &str = "project.save";
const CAPTIONS_TRANSCRIBE: &str = "captions.transcribe";

/// Every operation: the engine's commands, then the service's own.
pub fn ops() -> Vec<Op> {
    let mut out: Vec<Op> = registry()
        .iter()
        .map(|c| Op {
            name: c.id,
            title: c.label,
            doc: c.doc.to_string(),
            schema: input_schema(c.id).unwrap_or_else(|| json!({"type": "object"})),
            read_only: !c.edits,
            destructive: matches!(c.id, "clip.remove"),
        })
        .collect();
    let time = input_schema("clip.split").expect("clip.split has a schema")["properties"]["at"].clone();
    out.push(Op {
        name: RENDER_FRAME,
        title: "Render Frame",
        doc: "Draws the project at one instant and returns it as a PNG, to look at an edit.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "at": time,
                "scale": {"type": "number", "exclusiveMinimum": 0, "maximum": 2, "description": "Size relative to the canvas; 0.5 by default."},
                "draft": {"type": "boolean", "description": "Draw placeholders for missing media instead of failing; true by default."}
            },
            "additionalProperties": false
        }),
        read_only: true,
        destructive: false,
    });
    out.push(Op {
        name: RENDER_EXPORT,
        title: "Export MP4",
        doc: "Renders the whole project to an H.264 + AAC MP4 file on the server's disk.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Where to write the MP4 on the server; relative paths are beside the project."},
                "renderer": {"enum": ["auto", "cpu", "gpu"], "description": "What draws the frames; the GPU when there is one by default."}
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        read_only: true,
        destructive: false,
    });
    out.push(Op {
        name: CAPTIONS_TRANSCRIBE,
        title: "Transcribe Captions",
        doc: "Listens to the timeline's sound with the server's local Whisper model and places what is said as captions (captions.add), on a new Captions track unless `track` names one. Runs on the server's CPU; nothing leaves it. Takes about as long as the sound with a small model.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "language": {"type": "string", "description": "What is spoken, as a code (zh, en, ja, …); told from the first 30 s by default."},
                "prompt": {"type": "string", "description": "Words that set the style: names, punctuation, simplified or traditional characters. For Chinese a simplified-character prompt is used by default."},
                "from": time.clone(),
                "to": time.clone(),
                "track": {"type": "string", "description": "A text track to put the captions on; a new Captions track by default."},
                "template": {"type": "string", "description": "Title template id (title.templates); classic by default."}
            },
            "additionalProperties": false
        }),
        read_only: false,
        destructive: false,
    });
    out.push(Op {
        name: PROJECT_SAVE,
        title: "Save Project",
        doc: "Writes the project file (edits already save themselves when the server has one); `path` saves a copy elsewhere.".into(),
        schema: json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "Another file to write; the open project by default."}},
            "additionalProperties": false
        }),
        read_only: true,
        destructive: false,
    });
    out
}

/// Finds an operation by dotted id or MCP tool name.
pub fn find_op(name: &str) -> Option<Op> {
    ops().into_iter().find(|o| o.name == name || tool_name(o.name) == name)
}

/// An operation's MCP tool name: `clip.split` is `clip_split` (many clients refuse dots).
pub fn tool_name(op: &str) -> String {
    op.replace('.', "_")
}

/// One shared session, its media and its file.
pub struct Hub {
    shared: Arc<SharedSession>,
    footage: Footage,
    file: Option<PathBuf>,
    /// Where files clients send are kept: `media/` beside the project file, or a temporary folder.
    media_dir: PathBuf,
    speech: reelforge_speech::Recogniser,
}

impl Hub {
    /// A hub on a session kept in memory only.
    pub fn new(session: Session) -> Hub {
        Hub::shared(SharedSession::new(session), None).expect("nothing to write")
    }

    /// A hub on the project at `path`, created empty when there is no file yet. Every edit is
    /// written back.
    pub fn open(path: &Path) -> Result<Hub, String> {
        Hub::shared(SharedSession::new(Session::new(load(path)?)), Some(path.into()))
    }

    /// A hub on a session someone else also holds (the editor, when it hosts a server). With
    /// `file`, every change is written there, whoever made it.
    pub fn shared(shared: Arc<SharedSession>, file: Option<PathBuf>) -> Result<Hub, String> {
        if let Some(path) = &file {
            write_atomic(path, &shared.project().to_json())?;
            let path = path.clone();
            shared.on_change(move |_, project| {
                if let Err(e) = write_atomic(&path, &project.to_json()) {
                    eprintln!("reelforge: {e}");
                }
            });
        }
        let media_dir = match file.as_ref().and_then(|f| f.parent()) {
            Some(dir) => dir.join("media"),
            None => std::env::temp_dir().join("reelforge-media"),
        };
        let media_dir = std::path::absolute(&media_dir).unwrap_or(media_dir);
        let speech = reelforge_speech::Recogniser::new(reelforge_speech::default_model_dir());
        Ok(Hub { shared, footage: Footage::new(), file, media_dir, speech })
    }

    /// Listens with the Whisper model in `dir` instead of the default one.
    pub fn with_speech_model(mut self, dir: PathBuf) -> Hub {
        self.speech = reelforge_speech::Recogniser::new(dir);
        self
    }

    /// Keeps a file a client sent (a browser has no path the server can read) and says where it is,
    /// for `media.import`. `sha256` must be the bytes' hash; `name` is the file's name.
    pub fn upload(&self, sha256: &str, name: &str, bytes: &[u8]) -> Result<Value, CallError> {
        use sha2::{Digest, Sha256};
        let sha256 = sha256.to_ascii_lowercase();
        let actual: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        if actual != sha256 {
            return Err(CallError::Failed(format!("the file's SHA-256 is {actual}, not {sha256}")));
        }
        // the name only lends the file an extension and a readable stem
        let clean: String = Path::new(name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
            .collect();
        let clean = if clean.trim_matches('.').is_empty() { "file".to_string() } else { clean };
        let path = self.media_dir.join(format!("{}-{clean}", &sha256[..12]));
        let fail = |e: std::io::Error| CallError::Failed(format!("cannot keep {}: {e}", path.display()));
        if !path.is_file() {
            std::fs::create_dir_all(&self.media_dir).map_err(fail)?;
            let tmp = path.with_extension("part~");
            std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).map_err(fail)?;
        }
        Ok(json!({"path": path, "sha256": sha256, "bytes": bytes.len()}))
    }

    /// The bytes of the project's media with this hash, for a client to draw it.
    pub fn media_bytes(&self, sha256: &str) -> Result<Vec<u8>, CallError> {
        let project = self.shared.project();
        let media = project
            .media
            .values()
            .find(|m| m.sha256.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(sha256)))
            .ok_or_else(|| CallError::Unknown(format!("media {sha256}")))?;
        let path = self.resolve(media.uri.strip_prefix("file://").unwrap_or(&media.uri));
        std::fs::read(&path).map_err(|e| CallError::Failed(format!("cannot read {}: {e}", path.display())))
    }

    /// The project file, if there is one.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// The shared session.
    pub fn session(&self) -> &Arc<SharedSession> {
        &self.shared
    }

    /// Runs operation `name` (dotted id or tool name) with `params` (an object or null), as `cli`.
    pub fn call(&self, name: &str, params: &Value) -> Result<Output, CallError> {
        self.call_as("cli", name, params)
    }

    /// Runs operation `name` for client `by`, whose name goes with any change it makes.
    pub fn call_as(&self, by: &str, name: &str, params: &Value) -> Result<Output, CallError> {
        let op = find_op(name).ok_or_else(|| CallError::Unknown(name.to_string()))?;
        let empty = json!({});
        let params = if params.is_null() { &empty } else { params };
        if !params.is_object() {
            return Err(CallError::Failed("parameters must be a JSON object".into()));
        }
        match op.name {
            RENDER_FRAME => self.frame(params),
            RENDER_EXPORT => self.export(params),
            CAPTIONS_TRANSCRIBE => self.transcribe(params, by),
            PROJECT_SAVE => {
                let path = match params.get("path").and_then(Value::as_str) {
                    Some(p) => self.resolve(p),
                    None => self
                        .file
                        .clone()
                        .ok_or_else(|| CallError::Failed("this server has no project file; pass `path`".into()))?,
                };
                self.save_to(&path).map_err(CallError::Failed)?;
                Ok(Output::Json(json!({"saved": path})))
            }
            id => Ok(Output::Json(self.shared.execute(id, params, by)?)),
        }
    }

    /// A path from a client: relative ones are beside the project file.
    fn resolve(&self, p: &str) -> PathBuf {
        let p = Path::new(p);
        match self.file.as_ref().and_then(|f| f.parent()) {
            Some(dir) if p.is_relative() => dir.join(p),
            _ => p.to_path_buf(),
        }
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        write_atomic(path, &self.shared.project().to_json())
    }

    fn frame(&self, params: &Value) -> Result<Output, CallError> {
        let project = self.shared.project();
        let fps = project.canvas.fps;
        let at = match params.get("at") {
            Some(v) => parse_time(v, fps).map_err(|e| CallError::Failed(format!("`at`: {e}")))?,
            None => Tick::ZERO,
        };
        let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(0.5) as f32;
        if !(scale > 0.0 && scale <= 2.0) {
            return Err(CallError::Failed("`scale` must be above 0 and at most 2".into()));
        }
        let draft = params.get("draft").and_then(Value::as_bool).unwrap_or(true);
        let spec = reelforge_compile::compile(&project, &reelforge_compile::CompileOptions { encode: false })
            .map_err(|e| CallError::Failed(e.to_string()))?;
        let t = fps.snap(at);
        let quality = if draft { Quality::Draft } else { Quality::Final };
        let (img, report) =
            render_frame(&spec, t, scale, &self.footage, quality).map_err(|e| CallError::Failed(e.to_string()))?;
        let mut png = Vec::new();
        image::write_buffer_with_format(
            &mut Cursor::new(&mut png),
            &img.to_srgba8(),
            img.width,
            img.height,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| CallError::Failed(e.to_string()))?;
        let placeholders: Vec<Value> =
            report.placeholders.iter().map(|(l, why)| json!({"layer": l, "reason": why})).collect();
        Ok(Output::Png {
            png,
            info: json!({
                "width": img.width, "height": img.height, "at": t, "seconds": t.0 as f64 / reelforge_time::TICKS_PER_SECOND as f64,
                "placeholders": placeholders,
            }),
        })
    }

    fn transcribe(&self, params: &Value, by: &str) -> Result<Output, CallError> {
        self.speech.check().map_err(CallError::Failed)?;
        let project = self.shared.project();
        let fps = project.canvas.fps;
        let time = |name: &str| {
            params.get(name).map(|v| parse_time(v, fps).map_err(|e| CallError::Failed(format!("`{name}`: {e}")))).transpose()
        };
        let text = |name: &str| params.get(name).and_then(Value::as_str).map(str::to_string);
        let range = (time("from")?, time("to")?);
        let options = reelforge_speech::Options { language: text("language"), prompt: text("prompt") };
        let spec = reelforge_compile::compile(&project, &reelforge_compile::CompileOptions { encode: false })
            .map_err(|e| CallError::Failed(e.to_string()))?;
        let heard = self
            .speech
            .transcribe(&spec, &self.footage, range, &options, &mut |_| true)
            .map_err(CallError::Failed)?;
        if heard.segments.is_empty() {
            return Ok(Output::Json(json!({"language": heard.language, "count": 0, "clips": []})));
        }
        let add = reelforge_speech::captions_params(&heard.segments, text("track").as_deref(), text("template").as_deref());
        let placed = self.shared.execute("captions.add", &add, by)?;
        Ok(Output::Json(json!({
            "language": heard.language,
            "count": heard.segments.len(),
            "track": placed["track"],
            "clips": placed["clips"],
        })))
    }

    fn export(&self, params: &Value) -> Result<Output, CallError> {
        let path =
            params.get("path").and_then(Value::as_str).ok_or_else(|| CallError::Failed("`path` is required".into()))?;
        let renderer = match params.get("renderer").and_then(Value::as_str).unwrap_or("auto") {
            "auto" => Renderer::Auto,
            "cpu" => Renderer::Cpu,
            "gpu" => Renderer::Gpu,
            other => return Err(CallError::Failed(format!("`renderer` must be auto, cpu or gpu, not {other:?}"))),
        };
        let project = self.shared.project();
        let spec = reelforge_compile::compile(&project, &reelforge_compile::CompileOptions { encode: true })
            .map_err(|e| CallError::Failed(e.to_string()))?;
        let out = self.resolve(path);
        let tmp = out.with_extension("mp4.part");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| CallError::Failed(format!("cannot write {}: {e}", tmp.display())))?;
        let started = std::time::Instant::now();
        let result = reelforge_media::export_with(&spec, &self.footage, file, renderer, &mut |_| true);
        let stats = match result {
            Ok((_, stats)) => stats,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(CallError::Failed(e.to_string()));
            }
        };
        std::fs::rename(&tmp, &out).map_err(|e| CallError::Failed(format!("cannot write {}: {e}", out.display())))?;
        Ok(Output::Json(json!({
            "path": out,
            "frames": stats.frames,
            "audioSamples": stats.audio_samples,
            "renderer": stats.renderer,
            "seconds": started.elapsed().as_secs_f64(),
        })))
    }
}

/// Reads a project file, or an empty project when there is none yet.
pub fn load(path: &Path) -> Result<Project, String> {
    if !path.exists() {
        return Ok(Project::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Project::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a sibling file, then renames it over `path`, so readers never see half a file.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
