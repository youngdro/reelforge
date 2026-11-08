# Architecture

reelforge is an all-Rust short-video editor. The interface, the engine and the renderer are Rust; the
interface is egui drawn by wgpu; the same code runs on the desktop and in the browser (wasm32,
WebGPU with a WebGL2 fallback). Three ideas hold it together:

1. **Everything is a command.** Every change to a project is a registered command with a stable id
   (`clip.split`), documented parameters and an `enabled` check that explains itself. The interface,
   the CLI, the browser's `window.reelforge` API, MCP agents and the HTTP API all call the same
   dispatcher, `Session::execute`, and every command's parameters have a JSON Schema
   (`reelforge_engine::input_schema`).
2. **Rendering reads one document: RenderSpec.** The editor compiles a project into a RenderSpec, a
   versioned, flat, self-contained description of the finished edit: integer-tick time, assets by
   SHA-256, layers bottom first, audio clips, effects as versioned ids. The renderer depends on
   RenderSpec and nothing else, so it renders in the editor's process by default, or on a separate
   render service from the same JSON.
3. **Exact time.** All time is `Tick(i64)` at 254,016,000,000 per second, which divides every common
   frame rate and sample rate, so frames and samples are exact integers and edits never drift.

## Crates and layers

A crate depends only on lower layers; `cargo xtask layers` enforces it, plus two rules the layers
alone would allow: the renderer never depends on the project model or the compiler, and UI or
windowing crates (egui, eframe, winit, web-sys…) appear only from L5 up. The renderer and `reelforge-media` read RenderSpec only, so a render
service ships them without the editor.

| Layer | Crate | Role |
| --- | --- | --- |
| L0 | `reelforge-time` | `Tick`, `FrameRate`, `TimeRange` |
| L1 | `reelforge-renderspec` | RenderSpec types, validation, JSON Schema (`schema/renderspec-1.schema.json`) |
| L1 | `reelforge-model` | The project document: canvas, media, tracks, clips; invariants |
| L2 | `reelforge-compile` | Project → RenderSpec: all editing meaning resolved here |
| L2 | `reelforge-render` | Pure frame plan, CPU reference compositor, wgpu compositor (feature `gpu`); `MediaSource` trait |
| L3 | `reelforge-media` | Probing, video/audio decoding (`Footage`), MP4 export (H.264 + AAC); FilmCraft's pure-Rust codecs |
| L4 | `reelforge-engine` | `Session`, command registry, copy-on-write undo, batches; title templates (`templates`); SubRip captions (`captions`) |
| L5 | `reelforge-ui` | The egui editor: top bar, preview and transport, direct manipulation in the preview (select, move, scale, turn; `stage`), timeline (move, trim, snap; pictures along video clips and waveforms of sound, `strips`), inspector, shortcuts; its own dark theme and Phosphor icon subset (`theme`); preview sound (`audio`); English and Chinese, following the system and switched in the top bar (`i18n`); joins a server's session (`remote`); pictures of the title templates (`looks`) |
| L5 | `reelforge-server` | One shared session as a service: MCP (stdio, Streamable HTTP) and a plain HTTP API over the same operations; native only |
| L6 | `reelforge` | Desktop app (eframe, wgpu renderer) |
| L6 | `reelforge-cli` | Commands, scripts, compile, validate, render a frame, export; `serve`, and `call` on a file or a server |
| L6 | `reelforge-web` | Browser app (eframe web runner) + `window.reelforge.execute` |

```text
            interface (egui on wgpu)   CLI   browser API   MCP + HTTP
                         \              |        |        /
                          Session::execute  ── command registry, undo
                                   |
                               Project ──compile──► RenderSpec (JSON, versioned)
                                                        |
                                    plan_frame (pure) ──┴──► compositor: CPU reference │ wgpu
                                                                   │
                                          in-process (default)  or  render service (optional)
```

## RenderSpec rules

- A renderer refuses a `spec` version it does not know.
- Effects are `name@version`. `RenderSpec::required_features` lists what a spec needs; a final render
  refuses what the renderer lacks instead of drawing something else. A draft (preview) render draws
  a placeholder and reports it.
- Heavy analysis (cut-out masks, stabilisation, speech, TTS) is precomputed by the editor or an
  analysis worker and arrives as assets (`mask_sequence`, …), so renderers never run models and
  renders are reproducible.
- The JSON Schema is generated from the Rust types; a test fails when the checked-in copy is stale
  (`UPDATE_SCHEMA=1 cargo test -p reelforge-renderspec` refreshes it).

## What is not built yet

In rough priority order for short videos:

1. Faster media: hardware/WebCodecs decoders registered in front of the pure-Rust ones
   (`filmcraft_codecs::register_video_decoder`), decoding at preview size, export in the browser
   (WebCodecs encoder, or the pure-Rust one in a worker), pitch-preserving speed changes.
   Pure-Rust decoding and MP4 export (`reelforge-media`) are in place.
2. Text, next steps: per-line subtitle boxes, shadows, animated titles (typewriter, pop-in),
   importing font files in the editor (specs already take `font` assets), and a smaller web build
   (the two bundled CJK faces add 5 MB to the wasm). `text/1` itself is in place: FilmCraft's
   pure-Rust text engine (harfrust shaping, UAX #14 line breaking, its own rasteriser) with a
   bundled Noto Sans SC subset; system fonts are never consulted, so renders match everywhere.
3. GPU, next steps: decoded video uploaded as Y'CbCr planes and converted in a shader (today
   `Footage` converts to linear RGB on the CPU), effects beyond the two colour ones as WGSL, and the
   WebGL2 fallback (no float render targets there, so the browser preview without WebGPU stays on
   the CPU). The wgpu compositor itself is in place: the same frame plan, tested against the CPU
   reference to one 8-bit code, drawing the editor's preview on eframe's own device and exports on
   a headless one, with the sRGB and 4:2:0 conversions in shaders.
4. Audio, next steps: waveforms on the timeline, fades and volume keyframes in the inspector,
   scrubbing sound, pitch-preserving speed changes, and mixing on a worker thread (in the browser,
   an AudioWorklet). In place: the export's mix (`reelforge_media::export::mix`) plays in the
   preview (`reelforge-ui`'s `audio`, through cpal: ALSA, CoreAudio, WASAPI, WebAudio), mixed a
   quarter second ahead on the editor's thread; while it plays the device's sample count is the
   playhead, so picture follows sound, and an edit or a seek mixes again from the playhead.
5. Shared editing, next steps: MCP resources and change notifications, per-client undo, a push
   channel (WebSocket or SSE) in place of long polls, and presence (who is looking where). In
   place: `reelforge-server` exposes every command and render/export/save as MCP tools (stdio,
   Streamable HTTP) and at `/v1`; `reelforge-cli call` works on a file or a server; the desktop
   editor hosts a server on its own session (`--serve`); and an editor in a browser or on another
   machine joins a server's session (`reelforge_ui::remote`). A joined editor runs each edit on its
   own mirror `SharedSession` first (ids are minted from the project, so both sides agree), sends it
   to `/v1/ops`, and adopts the server's project whenever nothing is pending: after its own edits
   are answered and when `/v1/events` reports someone else's. Undo runs on the server. Media moves
   by hash (`PUT`/`GET /v1/media/{sha256}`), and the server serves the web build (`--web`).
6. A standalone render service that accepts a RenderSpec.
