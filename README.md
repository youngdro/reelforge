# reelforge

An all-Rust short-video editor: the interface, the engine and the renderer are Rust, drawn with wgpu,
and run on the desktop and in the browser (WebAssembly). Every action is a command that people, scripts
and AI agents drive the same way, and rendering reads one versioned, self-contained description of the
edit (RenderSpec), so a render can happen in the editor or on a separate render service.

Status: early. Video, still images and sound import (drop files on the editor, or `media.import`) and
decode in pure Rust on the desktop and in the browser; solid colours, transforms, opacity, two colour
effects and undo work; projects export to MP4 from the desktop editor and the CLI (H.264 + AAC) and from the
browser (H.264, HEVC or AV1 through the browser's hardware encoder, WebCodecs), at a chosen
resolution (720p to 4K), frame rate and quality, or one of four presets (`project.setExport`). A built-in
sound library (twelve effects and four music beds, synthesised by reelforge itself, so free to
use) adds with `sound.add`, and voice-overs record from the microphone on the desktop and in the
browser. A project can hold several timelines (cuts) that share its media: `timeline.add` (empty or a
copy), `timeline.switch`, `timeline.rename`, `timeline.remove`. Titles
render with shaping, wrapping, outlines, shadows, gradients and boxes, Chinese included (a bundled
Noto Sans SC subset covering GB 2312), and ten title templates (花字) apply a look in one step
(`title.templates`, `"template"` on `clip.add` / `clip.set`). SubRip captions come in by dropping a
`.srt` on the editor or `captions.import`, and go out with `captions.export`. Frames are composited on the GPU (wgpu) in the editor's preview and in exports,
with the CPU compositor as the reference and the fallback. Agents edit through an MCP server (stdio
or HTTP), and the same operations are an HTTP API and `reelforge-cli call`. People in a browser or on
another machine join a server's session and edit live alongside agents. The preview plays sound,
mixed as the export mixes it, with the picture following the sound. In the preview, what you see is
selected, moved, scaled and turned by dragging it. The timeline shows pictures
along video clips and the waveform of sound. The editor speaks English and
Chinese: it follows the system (or browser) language, and the switch in the top bar is remembered.
See [ARCHITECTURE.md](ARCHITECTURE.md).

Video and audio codecs and containers are FilmCraft's clean-room, pure-Rust crates
(`filmcraft-codecs`, `filmcraft-h264enc`, `filmcraft-aac`, `filmcraft-isobmff`; MIT OR Apache-2.0):
no FFmpeg and no C libraries.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

## Build and run

Rust 1.95 or newer. On Linux the editor plays sound through ALSA: install `libasound2-dev` (Debian,
Ubuntu) or `alsa-lib-devel` (Fedora) to build it.

```sh
cargo run -p reelforge                      # the editor (opens a demo project)
cargo run -p reelforge-cli -- --help        # commands, scripts, compile, validate, frame, export, serve, call
cargo xtask ci                              # fmt, clippy, tests, layer rules, wasm build
```

In the browser:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # must match Cargo.lock
cargo build --release --target wasm32-unknown-unknown -p reelforge-web
wasm-bindgen --target web --no-typescript --out-dir dist target/wasm32-unknown-unknown/release/reelforge_web.wasm
cp apps/reelforge-web/index.html dist/ && python3 -m http.server -d dist 8765
```

The page exposes `window.reelforge.execute(commandId, paramsJson)`, the same commands as the CLI.

## A first session from the shell

```sh
cargo run -q -p reelforge-cli -- new demo.reel.json --demo
cargo run -q -p reelforge-cli -- exec demo.reel.json clip.split '{"clip": "c4", "at": {"seconds": 2}}' --save
cargo run -q -p reelforge-cli -- compile demo.reel.json -o demo.renderspec.json
cargo run -q -p reelforge-cli -- frame demo.renderspec.json --seconds 1.5 --out frame.png --draft
```

With your own footage and a subtitle:

```sh
cargo run -q -p reelforge-cli -- new cut.reel.json
cargo run -q -p reelforge-cli -- exec cut.reel.json media.import '{"path": "/path/to/clip.mp4"}' --save   # → m4
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.add '{"track": "t1", "media": "m4", "sourceIn": {"seconds": 1}, "duration": {"seconds": 4}}' --save
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.add '{"track": "t2", "text": "第一句字幕", "size": 72, "start": {"seconds": 0.5}, "duration": {"seconds": 2}}' --save
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.setAnimation '{"clip": "c6", "slot": "in", "kind": "pop"}' --save
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.setKey '{"clip": "c5", "prop": "scale", "at": {"seconds": 4}, "value": 1.3, "ease": "inOut"}' --save
cargo run -q -r -p reelforge-cli -- export cut.reel.json -o cut.mp4      # a project, or a RenderSpec
# frames are drawn on the GPU when there is one; --renderer cpu|gpu chooses
```

## Agents (MCP) and the HTTP API

`reelforge-cli serve` puts a project behind the Model Context Protocol: every command is a tool
(`clip_split`, `clip_add`…, each with a JSON Schema), plus `render_frame` (returns a PNG the agent can
look at), `render_export` and `project_save`. Every edit is saved to the project file as it commits.

```sh
cargo build -r -p reelforge-cli
# a local agent starts it over stdio, e.g. Claude Code:
claude mcp add reelforge -- ./target/release/reelforge-cli serve /path/to/cut.reel.json --stdio
# or run it over HTTP (Streamable HTTP at /mcp) for remote agents and other programs:
./target/release/reelforge-cli serve cut.reel.json --listen 127.0.0.1:7878
claude mcp add --transport http reelforge http://127.0.0.1:7878/mcp
```

To edit together with an agent, let the editor host the server: the agent's edits appear in the
window as they commit, the status bar says who made each one, and undo covers both.

```sh
cargo run -r -p reelforge -- cut.reel.json --serve        # MCP at http://127.0.0.1:7878/mcp
```

Opening a project in the editor saves every edit to it, whoever makes it.

People can join from a browser, or from another machine's editor. Build the web editor once (see
Build and run, into `dist/`), then serve it with the project; opening the address edits the same
session as the agents:

```sh
reelforge-cli serve cut.reel.json --web dist                 # open http://127.0.0.1:7878/
reelforge-cli serve cut.reel.json --listen 0.0.0.0:7878 --token SECRET --web dist
                                                             # open http://host:7878/?token=SECRET
cargo run -r -p reelforge -- --join http://host:7878 --token SECRET   # the desktop editor joins too
```

Each editor edits its own copy at once and sends the edit to the server; it follows everyone else's
edits as they commit, and fetches the media the project uses. Files dropped on a joined editor are
sent to the server and kept in `media/` beside the project.

On anything but loopback the server needs a token (`--token` or `REELFORGE_TOKEN`), sent as
`Authorization: Bearer …`.

The same operations are a plain HTTP API (`GET /v1/ops`, `POST /v1/ops/clip.split` with the
parameters as the body) and a shell command, on a file or on a running server:

```sh
reelforge-cli ops                                                   # every operation and its parameters
reelforge-cli call clip.split clip=c4 at=2s --project cut.reel.json  # name=value, or one JSON object
reelforge-cli call render.frame at=1.5s --server http://127.0.0.1:7878 -o frame.png
export REELFORGE_SERVER=http://127.0.0.1:7878                       # then --server can be left out
```
