# Working on reelforge

Read ARCHITECTURE.md first. Rules:

- Every change to a project is a command in `crates/engine/src/commands.rs`. Clients (UI, CLI, web,
  MCP) never mutate a `Project` directly.
- The renderer (`reelforge-render`) reads RenderSpec only. New editing features get a lowering in
  `reelforge-compile`; if the renderer must learn something new, add it to RenderSpec with a version
  or feature id, and regenerate the schema (`UPDATE_SCHEMA=1 cargo test -p reelforge-renderspec`).
- Time is `Tick`. No `f64` seconds in edit or render math; convert at the boundary.
- Keep `cargo xtask ci` green: fmt, clippy `-D warnings`, tests, layers, wasm build.
- Code that runs on the web must not call `std::thread::spawn`, `std::time::Instant::now` or the file
  system; gate native-only code with `#[cfg(not(target_arch = "wasm32"))]`.
