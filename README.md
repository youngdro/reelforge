# reelforge

An all-Rust short-video editor: the interface, the engine and the renderer are Rust, drawn with wgpu,
and run on the desktop and in the browser (WebAssembly). Every action is a command that people, scripts
and AI agents drive the same way, and rendering reads one versioned, self-contained description of the
edit (RenderSpec), so a render can happen in the editor or on a separate render service.

Status: early. Video, still images and sound import (drop files on the editor, or `media.import`) and
decode in pure Rust on the desktop and in the browser; solid colours, transforms, opacity, two colour
effects, undo and a CPU preview work; projects export to H.264 + AAC MP4 from the desktop editor and the
CLI. Titles are placeholders until text rendering lands; the wgpu compositor and audio playback come
next. See [ARCHITECTURE.md](ARCHITECTURE.md).

Video and audio codecs and containers are FilmCraft's clean-room, pure-Rust crates
(`filmcraft-codecs`, `filmcraft-h264enc`, `filmcraft-aac`, `filmcraft-isobmff`; MIT OR Apache-2.0):
no FFmpeg and no C libraries.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

## Build and run

Rust 1.95 or newer.

```sh
cargo run -p reelforge                      # the editor (opens a demo project)
cargo run -p reelforge-cli -- --help        # commands, scripts, compile, validate, render a frame, export
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

With your own footage (export refuses titles until text rendering lands):

```sh
cargo run -q -p reelforge-cli -- new cut.reel.json
cargo run -q -p reelforge-cli -- exec cut.reel.json media.import '{"path": "/path/to/clip.mp4"}' --save   # → m4
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.add '{"track": "t1", "media": "m4", "sourceIn": {"seconds": 1}, "duration": {"seconds": 4}}' --save
cargo run -q -r -p reelforge-cli -- export cut.reel.json -o cut.mp4      # a project, or a RenderSpec
```
