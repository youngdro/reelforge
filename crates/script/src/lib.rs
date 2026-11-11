//! Motion scripts.
//!
//! A motion script is a piece of JavaScript that makes nodes (shapes, titles, pictures) and
//! tweens them on a timeline, in the words GSAP uses (`to`, `from`, `fromTo`, `stagger`, position
//! parameters, ease names):
//!
//! ```js
//! export const params = { price: { type: "number", default: 199 } };
//! export function build({ params, canvas, scene, timeline }) {
//!   const card = scene.rect({ width: 0.8 * canvas.width, height: 300, radius: 40, fill: "#fff" });
//!   const price = scene.text(`¥${params.price}`, { size: 140, color: "#ff3b5c" });
//!   timeline().from(card, { y: "+=400", opacity: 0, ease: "back.out(1.4)" })
//!             .from(price, { scale: 0, ease: "elastic.out(1, 0.5)" }, "<0.1");
//! }
//! ```
//!
//! [`run`] runs one once and answers with plain data, [`Built`]: the nodes as `clip.add` would
//! take them and their keyframes. The engine turns that into ordinary clips, so nothing of a
//! script reaches the renderer, and a script's arithmetic never has to agree between machines.
//!
//! A script is given no file system, network, clock or module to import; `Math.random` is seeded,
//! so the same source and parameters always build the same thing; loops and recursion are bounded
//! ([`LOOP_LIMIT`], [`RECURSION_LIMIT`]) and so are the nodes, tweens and keys it may make.

use std::collections::BTreeMap;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::{Context, JsError, JsValue, Module, Source, js_string};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// How many times one loop of a script may go round.
pub const LOOP_LIMIT: u64 = 2_000_000;
/// How deep a script's calls may nest.
pub const RECURSION_LIMIT: usize = 256;
/// The longest source taken, in bytes.
pub const SOURCE_LIMIT: usize = 256 * 1024;

const PRELUDE: &str = include_str!("prelude.js");

/// Why a script built nothing.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScriptError {
    /// The source is not JavaScript, or is too long.
    #[error("the script does not parse: {0}")]
    Syntax(String),
    /// It threw, or asked for something there is not.
    #[error("{0}")]
    Run(String),
}

/// What a script is run with.
#[derive(Clone, Debug, PartialEq)]
pub struct Input<'a> {
    /// The values given for its parameters; the rest take their defaults.
    pub params: &'a Value,
    /// The canvas, in pixels.
    pub canvas: [u32; 2],
    /// Frames a second: keys land on frames.
    pub fps: f64,
    /// What `Math.random` starts from.
    pub seed: u32,
    /// How long the piece is to be, in seconds, when the caller says; else the script's own
    /// `scene.setDuration`, else where its last tween ends.
    pub duration: Option<f64>,
}

/// One keyframe a script made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyOut {
    /// Seconds from the script's start, on a frame.
    pub t: f64,
    /// The value: for `x` and `y` a share of the canvas, as clips keep them.
    pub v: f64,
    /// `"linear"`, `"hold"` or `[x1, y1, x2, y2]`.
    pub ease: Value,
}

/// One node a script made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Its name: the script's, or its kind and number.
    pub name: String,
    /// `shape`, `text`, `media`, `sticker`, or `group` (a shape nobody sees, which carries others).
    pub kind: String,
    /// What `clip.add` takes to make it, but for the track and the times.
    pub add: Map<String, Value>,
    /// A title's counter, as `clip.setCounter` takes it.
    #[serde(default)]
    pub counter: Option<Value>,
    /// Its keyframes by property name.
    pub keys: BTreeMap<String, Vec<KeyOut>>,
    /// The point it is placed, scaled and turned about, as shares of its own box from the top
    /// left, where the script moved it from the middle. The box stays where it lies, as CSS's
    /// `transform-origin` leaves it: `x` and `y` still say where its middle is at rest.
    #[serde(default)]
    pub origin: Option<[f64; 2]>,
    /// The node that carries it, by its place in the list: its `x` and `y` are then counted
    /// from that node's origin, along that node's axes.
    #[serde(default)]
    pub parent: Option<usize>,
    /// The path it travels.
    #[serde(default)]
    pub path: Option<PathOut>,
    /// What else is set on its clip once it is there: a mask, the editor's own animations.
    #[serde(default)]
    pub after: Vec<Step>,
}

/// A command run on a node's clip, which it names itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// `clip.setMask` or `clip.setAnimation`.
    pub command: String,
    /// Its parameters, but for the clip.
    pub params: Map<String, Value>,
}

/// A path a node travels: through its points at an even pace, eased as a whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathOut {
    /// The points, as shares of the canvas from its centre.
    pub points: Vec<[f64; 2]>,
    /// Goes on from the last point round to the first.
    pub closed: bool,
    /// Turns the node to face the way it goes.
    pub orient: bool,
    /// When it sets off, in seconds from the script's start.
    pub start: f64,
    /// How long the journey takes, in seconds.
    pub duration: f64,
    /// `"linear"`, `"hold"`, or a cubic Bézier `[x1, y1, x2, y2]`.
    pub ease: Value,
}

/// What a script built.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Built {
    /// How long the piece is, in seconds.
    pub duration: f64,
    /// Where its last tween ends.
    pub animated: f64,
    /// The nodes, bottom first.
    pub nodes: Vec<Node>,
    /// The parameters it declares: `{name: {type, default, label?, min?, max?, options?}}`.
    pub params: Map<String, Value>,
    /// The value each parameter had in this run.
    pub values: Map<String, Value>,
    /// What it wrote with `console.log`.
    pub log: Vec<String>,
}

fn told(e: &JsError, ctx: &mut Context) -> String {
    // an Error's message alone, where there is one: the prelude's own refusals read as sentences
    if let Some(obj) = e.as_opaque().and_then(JsValue::as_object)
        && let Ok(m) = obj.get(js_string!("message"), ctx)
        && let Some(s) = m.as_string()
    {
        let name = obj.get(js_string!("name"), ctx).ok().and_then(|n| n.as_string().map(|n| n.to_std_string_escaped()));
        let text = s.to_std_string_escaped();
        return plain(&match name.as_deref() {
            Some("Error") | None => text,
            Some(n) => format!("{n}: {text}"),
        });
    }
    told_plain(e)
}

/// An error in a line: what went wrong, without the interpreter's trace of where (its places are
/// those of functions, not of the mistake, and half of them are in the prelude).
fn told_plain(e: &JsError) -> String {
    plain(&e.to_string())
}

fn plain(all: &str) -> String {
    let first = all.lines().next().unwrap_or_default();
    let first = first.split(" (unknown at").next().unwrap_or(first).trim();
    match first {
        "RuntimeLimitError: reached the maximum number of iteration loops on this execution" => {
            format!("a loop went round more than {LOOP_LIMIT} times; the script is stopped")
        }
        f if f.starts_with("RuntimeLimitError") => {
            format!("calls nested deeper than {RECURSION_LIMIT}; the script is stopped")
        }
        "TypeError: module resolution is disabled for this context" => {
            "a script cannot import anything: it is one file, given `scene` and `timeline`".to_string()
        }
        f => f.to_string(),
    }
}

/// A script loaded: the world it runs in, and what it exports.
struct Loaded {
    ctx: Context,
    build: JsValue,
    declared: JsValue,
}

fn load(source: &str) -> Result<Loaded, ScriptError> {
    if source.len() > SOURCE_LIMIT {
        return Err(ScriptError::Syntax(format!("it is longer than {} KB", SOURCE_LIMIT / 1024)));
    }
    // no module can be imported: the default loader would read files
    let mut ctx = Context::builder()
        .module_loader(std::rc::Rc::new(boa_engine::module::IdleModuleLoader))
        .build()
        .map_err(|e| ScriptError::Run(told_plain(&e)))?;
    ctx.runtime_limits_mut().set_loop_iteration_limit(LOOP_LIMIT);
    ctx.runtime_limits_mut().set_recursion_limit(RECURSION_LIMIT);
    ctx.eval(Source::from_bytes(PRELUDE)).map_err(|e| ScriptError::Run(format!("prelude: {e}")))?;

    let module = Module::parse(Source::from_bytes(source), None, &mut ctx)
        .map_err(|e| ScriptError::Syntax(told(&e, &mut ctx)))?;
    let promise = module.load_link_evaluate(&mut ctx);
    ctx.run_jobs().map_err(|e| ScriptError::Run(told(&e, &mut ctx)))?;
    match promise.state() {
        PromiseState::Fulfilled(_) => {}
        PromiseState::Rejected(e) => {
            let e = JsError::from_opaque(e);
            return Err(ScriptError::Run(told(&e, &mut ctx)));
        }
        PromiseState::Pending => return Err(ScriptError::Run("the script never finished loading".into())),
    }
    let exports = module.namespace(&mut ctx);
    let mut get = |name: &str| {
        exports.get(boa_engine::JsString::from(name), &mut ctx).map_err(|e| ScriptError::Run(told(&e, &mut ctx)))
    };
    let (build, declared) = (get("build")?, get("params")?);
    Ok(Loaded { ctx, build, declared })
}

/// Calls the prelude's `name` and reads the JSON it answers with.
fn call<T: serde::de::DeserializeOwned>(ctx: &mut Context, name: &str, args: &[JsValue]) -> Result<T, ScriptError> {
    let f = ctx
        .global_object()
        .get(boa_engine::JsString::from(name), ctx)
        .ok()
        .and_then(|v| v.as_callable())
        .ok_or_else(|| ScriptError::Run("the script took the runner away".into()))?;
    let out = f.call(&JsValue::undefined(), args, ctx).map_err(|e| ScriptError::Run(told(&e, ctx)))?;
    let text = out.as_string().map(|s| s.to_std_string_escaped()).unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| ScriptError::Run(format!("what the script built cannot be read: {e}")))
}

/// Runs `source` once and answers with what it built.
pub fn run(source: &str, input: &Input) -> Result<Built, ScriptError> {
    let Loaded { mut ctx, build, declared } = load(source)?;
    let arg = json!({
        "canvas": input.canvas,
        "fps": input.fps,
        "seed": input.seed,
        "params": input.params,
        "duration": input.duration,
    });
    call(&mut ctx, "__rf_run", &[build, declared, JsValue::from(boa_engine::JsString::from(arg.to_string()))])
}

/// The parameters `source` declares, `{name: {type, default, label?, …}}`, without building.
pub fn declared(source: &str) -> Result<Map<String, Value>, ScriptError> {
    let Loaded { mut ctx, declared, .. } = load(source)?;
    call(&mut ctx, "__rf_params", &[declared])
}

#[cfg(test)]
mod tests;
