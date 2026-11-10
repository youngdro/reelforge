//! `cargo xtask <check>`: repository rules that the compiler does not enforce.
//!
//! - `layers`: every crate has a layer and depends only on lower layers; the renderer never sees
//!   the project model or the engine; UI and windowing crates stay in the top layers.
//! - `wasm`: every crate below the apps (except the native-only server), and the web app, builds for
//!   `wasm32-unknown-unknown`.
//! - `ci`: fmt, clippy, tests, layers, wasm.

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

/// Layer of each workspace crate. A crate may depend only on crates in lower layers.
const LAYERS: &[(&str, u8)] = &[
    ("reelforge-time", 0),
    ("reelforge-matte", 0),
    ("reelforge-script", 0),
    ("reelforge-lottie", 0),
    ("reelforge-renderspec", 1),
    ("reelforge-model", 1),
    ("reelforge-compile", 2),
    ("reelforge-render", 2),
    ("reelforge-media", 3),
    ("reelforge-engine", 4),
    ("reelforge-hwenc", 4),
    ("reelforge-speech", 4),
    ("reelforge-voice", 4),
    ("reelforge-ui", 5),
    ("reelforge-server", 5),
    ("reelforge", 6),
    ("reelforge-cli", 6),
    ("reelforge-web", 6),
    ("xtask", 9),
];

/// Edges that are forbidden even though the layers would allow them.
const FORBIDDEN: &[(&str, &str, &str)] = &[
    ("reelforge-render", "reelforge-model", "the renderer reads RenderSpec only, never the project"),
    ("reelforge-render", "reelforge-compile", "the renderer reads RenderSpec only, never the project"),
    ("reelforge-media", "reelforge-model", "media and export read RenderSpec only, so a render service can ship them"),
    (
        "reelforge-media",
        "reelforge-compile",
        "media and export read RenderSpec only, so a render service can ship them",
    ),
    (
        "reelforge-renderspec",
        "reelforge-model",
        "RenderSpec must stand alone so renderers can be deployed without the editor",
    ),
];

/// Crates below the app layer that never run in the browser (they listen on sockets).
const NATIVE_ONLY: &[&str] = &["reelforge-server", "reelforge-speech", "reelforge-voice"];

/// External crates allowed only from this layer up.
const UI_ONLY: &[&str] = &["egui", "eframe", "egui-wgpu", "egui-winit", "winit", "wasm-bindgen", "web-sys", "rfd"];
const UI_LAYER: u8 = 5;

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let ok = match task.as_str() {
        "layers" => layers(),
        "wasm" => wasm(),
        "ci" => ci(),
        _ => {
            eprintln!("usage: cargo xtask <layers|wasm|ci>");
            false
        }
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn cargo(args: &[&str]) -> bool {
    eprintln!("$ cargo {}", args.join(" "));
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn layers() -> bool {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata runs");
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).expect("cargo metadata is JSON");
    let layer: BTreeMap<&str, u8> = LAYERS.iter().copied().collect();
    let mut problems = Vec::new();
    for pkg in meta["packages"].as_array().expect("packages") {
        let name = pkg["name"].as_str().expect("name");
        let Some(&mine) = layer.get(name) else {
            problems.push(format!("{name} has no layer: add it to LAYERS in xtask/src/main.rs"));
            continue;
        };
        for dep in pkg["dependencies"].as_array().expect("dependencies") {
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            let d = dep["name"].as_str().expect("dep name");
            if let Some(&theirs) = layer.get(d)
                && theirs >= mine
                && mine != 9
            {
                problems.push(format!("{name} (L{mine}) depends on {d} (L{theirs}): only lower layers are allowed"));
            }
            if let Some((_, _, why)) = FORBIDDEN.iter().find(|(a, b, _)| *a == name && *b == d) {
                problems.push(format!("{name} depends on {d}: {why}"));
            }
            if UI_ONLY.contains(&d) && mine < UI_LAYER {
                problems.push(format!(
                    "{name} (L{mine}) depends on {d}: UI and windowing crates belong in L{UI_LAYER} and up"
                ));
            }
        }
    }
    for p in &problems {
        eprintln!("layers: {p}");
    }
    if problems.is_empty() {
        eprintln!("layers: ok");
    }
    problems.is_empty()
}

fn wasm() -> bool {
    let mut args = vec!["check", "--target", "wasm32-unknown-unknown"];
    for (name, layer) in LAYERS {
        if (*layer < 6 || *name == "reelforge-web") && !NATIVE_ONLY.contains(name) {
            args.extend(["-p", name]);
        }
    }
    cargo(&args)
}

fn ci() -> bool {
    cargo(&["fmt", "--all", "--check"])
        && cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
        && cargo(&["test", "--workspace"])
        && layers()
        && wasm()
}
