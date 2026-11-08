# reelforge

An all-Rust short-video editor: the interface, the engine and the renderer are Rust, drawn with wgpu,
and run on the desktop and in the browser (WebAssembly). Every action is a command that people, scripts
and AI agents drive the same way, and rendering reads one versioned, self-contained description of the
edit (RenderSpec), so a render can happen in the editor or on a separate render service.

Status: skeleton. See [ARCHITECTURE.md](ARCHITECTURE.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
