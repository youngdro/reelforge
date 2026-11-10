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
| L0 | `reelforge-matte` | A person cut out of a picture: MediaPipe's selfie segmentation model (bundled, Apache-2.0) read from its TensorFlow Lite file (`tflite`: a FlatBuffers reader for the few operations it uses) and run in plain Rust (`run`), its 256 × 256 answer fitted to the picture's own edges at the picture's size (a guided filter, `person`). No dependency on the rest of the workspace |
| L1 | `reelforge-renderspec` | RenderSpec types, validation, JSON Schema (`schema/renderspec-1.schema.json`) |
| L1 | `reelforge-model` | The project document: canvas, media, tracks (magnetic or not) of the timeline being edited and the other timelines parked beside it (`timeline`), clips, the groups they are in (selected and moved as one), the template slot they are (`slot`: a picture clip that stands for a file of the person's own) and compound clips, which play another timeline of the project as one clip, and their colour (filters, LUTs, adjustments; `color`), their crop, mirror, blend mode, shape mask, chroma key (a green screen keyed out) and whether only their people are kept (`look`), keyframes (on placement, opacity and volume, and on the crop's edges, the mask and the colour adjustments; each travels on to the next along a named curve or a cubic Bézier drawn by hand, `Ease`, which compile follows frame by frame) and in, out and looping animations (`anim`), transitions (`transition`), picture effects on a clip and adjustment clips that lay their effects and colour on the tracks below (`fx`), speed curves, reverse and freeze frames, and what a video shows between two frames of its file (`timing`: `Inbetween`), and for a steadied shot the camera's path as it was measured and how much of the shake goes (`steady`: what that does to the picture is worked out from the two each time, so how much is changed without watching the file again); invariants |
| L2 | `reelforge-compile` | Project → RenderSpec: all editing meaning resolved here (keyframes, animations and transitions become RenderSpec keys and overlapping layers, `motion`; picture effects are sampled into those keys and into keyed `color.grade@1` and `blur.gaussian@1` parameters, so the renderer needs nothing new for them, except the distortions, which become keyed `distort.*@1` effects; a chroma key becomes `key.chroma@1`, first among a layer's effects but for `key.person@1`, which a clip whose people are cut out gets before it; an adjustment clip becomes no layer of its own; and a timeline that compound clips play is lowered once into `comps`, each such clip becoming a layer that shows it and an audio clip that plays its mix; a steadied clip's layer is moved and turned against its shake a key a frame and drawn larger by one scale, in the picture's own frame whatever the clip's placement, crop, mirror and speed, so the renderer needs nothing new for it) |
| L2 | `reelforge-render` | Pure frame plan, CPU reference compositor, wgpu compositor (feature `gpu`; it converts video pictures from Y'CbCr in a shader, `yuv`); drafts ask for pictures only as big as they are drawn (`picture_shrunk`); `MediaSource` trait (pictures, and pictures made between a video's frames, `picture_between`: a layer whose `between` is not `nearest` asks for its instant as it is, and a source that makes none answers with the frame of that instant); `key.person@1` is done to a layer's whole picture before it is placed, as a blur is, by `reelforge-matte` (`cutout`, which keeps the last few so that a paused frame or one read ahead is not cut out twice), so CPU and GPU draw the same cut-out |
| L3 | `reelforge-media` | Probing, video/audio decoding (`Footage`), MP4 export (`ExportWriter` over any `VideoEncoder`, AAC sound), GIF, sound-only and still export (`formats`), the synthesised sound library (`library:` URIs) and the drawn sticker library (`library:sticker/` URIs, `stickers`); pictures between a video's frames (`between`: the two mixed, or one made by following what moved, cell by cell on small grey pictures, coarse to fine, both frames drawn along that way and mixed, the nearer one alone where they do not agree; `frames.blend/1`, `frames.flow/1`); how a camera moved between two frames (`steady`: patches of a small grey picture followed to a fraction of a pixel, a shift and a turn fitted to those that agree, so something crossing the shot is outvoted and a cut or a flat picture says nothing); FilmCraft's pure-Rust codecs |
| L4 | `reelforge-hwenc` | The operating system's video encoders behind `reelforge_media::VideoEncoder` (VideoToolbox on macOS, Media Foundation transforms on Windows: H.264 and HEVC, in hardware where the machine has it); the apps register them in front of the software encoder (`register_video_encoder`), which stays the fallback. The one crate that may use `unsafe` |
| L4 | `reelforge-engine` | `Session`, command registry, copy-on-write undo, batches; title templates (`templates`); SubRip captions and their editing as a list (`captions`: split, merge, replace, restyle, shift, a second language); CapCut / 剪映 draft import (`capcut`); timeline templates (`remix`: 剪同款. A template is an ordinary timeline in which some picture clips are slots, `Clip::slot`; a file takes a slot's place and the slot keeps its length and everything set on it, `template.fill`, `clip.replace`; the built-in ones are built by the same commands anyone runs, in one undo step, `template.apply`; any timeline becomes one with `template.mark`) |
| L5 | `reelforge-ui` | The egui editor: top bar, the asset panel on the left (the project's media with pictures, lengths, use and offline marks, the sound library, titles and templates, the stickers, and picture effects that land as adjustment clips; dragged onto the timeline or added at the playhead; `assets`; and the captions of a text track as a list: words edited in place, Enter to split at the cursor, Backspace to join, find and replace, one caption's look for all, a shift in time, SubRip in and out with a second language's line; `captions`), preview and transport (drawn as sharp as the screen shows it, smaller while playing in Smooth quality; a playing preview compiles once and reads ahead on a worker thread on the desktop, `ahead`), direct manipulation in the preview (select, move, scale, turn, sticking to the frame, the safe area and other layers; Alt-click for the layer below; double-click a title to edit its words; zoom, pan and preview-only; a mask's own handles on the picture to move, size and turn it, and a crop mode that shows the whole picture with the crop as a box to drag; `stage`), timeline (move, trim, snap; select several by Shift/Ctrl-click or a box and move them together; a group is chosen by any of its clips; a compound clip opens on a double-click, with a way back beside the timeline's name; a right-click menu; markers on the ruler, an I/O range that can loop, locked tracks, track order and height; Alt-drag a cut to roll it or a clip to slip it, detach a video's audio; pictures along video clips and waveforms of sound, `strips`), inspector (with a canvas-blur backdrop for videos and pictures), shortcuts; its own dark theme and Phosphor icon subset (`theme`); preview sound, and a moment of it where the playhead is dragged while paused (`audio`); English and Chinese, following the system and switched in the top bar (`i18n`); joins a server's session (`remote`); a project with no file and no server is a draft, kept in the app's storage (the page's, in the browser) a moment after each change, so it comes back after a crash, a closed window or a reload (`with_recovery`); the drafts page lists every draft with a picture of it, to open another, start a new one, or rename, copy or delete one (`drafts`; opening a draft replaces the session's project, which is not an edit and leaves no history); an editor on a file or a server has New instead, which empties the project (`project.new`, undoable); a window narrower than 860 points shows the preview above and one of the timeline, the media and the inspector under it, picked by tabs at the bottom, with shorter bars; pictures of the title templates (`looks`); timeline templates as cards that draw the row of their slots, on the drafts page (a new draft from one) and in the asset panel's templates tab (on this timeline when it is empty, else a new one), where the slots still empty are counted: while there are some, a picture or video added without a place goes into the next one, one dropped on a slot goes into that slot, and a clip's menu has Replace and makes or unmakes a slot (`remix`) |
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
- A composition (`comps`, `comp/1`) is a set of layers and audio clips of its own. A layer that
  shows one has it drawn first, on a see-through picture the size of the frame, and is then placed,
  faded, cropped, masked and given effects and a blend mode as one picture; an audio clip that
  plays one plays its mix under the clip's own gain. A composition never shows or plays itself,
  however far down (`validate` refuses the loop).
- Heavy analysis (cut-out masks, stabilisation, speech, TTS) is precomputed by the editor or an
  analysis worker and arrives as assets (`mask_sequence`, …), so renderers never run models and
  renders are reproducible.
- The JSON Schema is generated from the Rust types; a test fails when the checked-in copy is stale
  (`UPDATE_SCHEMA=1 cargo test -p reelforge-renderspec` refreshes it).

## What is not built yet

In rough priority order for short videos:

1. Faster media: hardware/WebCodecs decoders registered in front of the pure-Rust ones
   (`filmcraft_codecs::register_video_decoder`), decoding at preview size, drawing browser exports
   on the GPU (today the CPU draws them), and Linux's system encoder
   (VA-API) in `reelforge-hwenc`, beside macOS's VideoToolbox and Windows's Media Foundation.
   In place: pure-Rust decoding, MP4 export through `ExportWriter`, which takes any
   `VideoEncoder` (FilmCraft's H.264 on the desktop and CLI; WebCodecs H.264, HEVC and AV1 in the
   browser, usually in hardware, with an `av1C` built from the sequence header when the browser
   gives none), and export settings kept with the project (`ExportSettings`: size by the shorter
   side, frame rate, quality, codec, presets), lowered to `output.video` (`encode.size/1`).
   The settings also say what kind of file (`ExportSettings.format`, lowered to `output.format`):
   an animated GIF (`encode.gif/1`, `GifWriter`: the frames drawn on the CPU at the GIF's size, a
   palette per frame) or the sound alone (`encode.audio/1`, `AudioWriter`: AAC in an MP4, or
   16-bit PCM in a WAV). `reelforge_media::export` writes whichever the spec asks for, so every
   client exports the same way; the step-wise writers are for the browser, which has to yield.
   A cover is an instant of a timeline (`Project.cover`, `project.setCover`); exports write that
   frame as a JPEG beside a video (`still` + `encode_still`), which is also how the editor saves
   the frame under the playhead.
2. Text, next steps: a smaller web build (the two bundled CJK faces
   add 5 MB to the wasm). `text/1` itself is in place: FilmCraft's pure-Rust text engine (harfrust
   shaping, UAX #14 line breaking, its own rasteriser) with a bundled Noto Sans SC subset; system
   fonts are never consulted, so renders match everywhere. A font file is project media of kind
   `font` (`media.import`; like a LUT it is used by clips, not placed), a title names one with
   `font`, and it reaches the renderer as a `font` asset. `text/4` adds letter spacing, line
   spacing (both in ems), italic (slanted when the family has no italic face) and underline.
   `text/5` moves the characters one after another (`chars`): each movement is a progress over
   time, a step from one character to the next, a curve (`settle`, `spring`, `wave`, `hop`) and the
   pose furthest from rest, so the renderer knows no preset. The compiler lowers the titles-only
   animations to it (`charFade`, `charRise`, `charDrop`, `charPop` in and out, the loops `wave` and
   `jump`); a loop of the whole clip (`breathe`, `swing`, `blink`…) is sampled into the layer's
   ordinary keys like any other animation.
3. GPU, next steps: blurs on the GPU (the colour effects, `color.grade@1`, `color.lut@1` and
   `key.chroma@1` included, are per-pixel WGSL already, as are crops, mirrors, shape masks and the
   `blend/2` modes, which read the layers below from a copy; the distortions, `distort.wave@1`,
   `distort.glitch@1`, `distort.mosaic@1` and `distort.split@1`, move where a layer's picture is
   read, by the same arithmetic in `render::distort` and in WGSL, the glitch's bands from an
   integer hash so every backend cuts them alike; a blurred layer's picture is still made on the
   CPU). A distortion on an adjustment clip bends each layer below in that layer's own picture,
   not the frame they make together. A composition is drawn in a pass of its own into a picture
   the next pass reads (a blurred one is still made on the CPU). The wgpu compositor itself is in place: the same
   frame plan, tested against the CPU reference to one 8-bit code (on Vulkan, and on GL with
   WebGL2's limits), drawing the editor's preview on eframe's own device (WebGL2 included, where
   the browser can draw into float pictures) and exports on a headless one, with decoded video
   converted from Y'CbCr and the sRGB and 4:2:0 conversions in shaders.
4. Audio, next steps: mixing on a worker thread (in the browser, an
   AudioWorklet). In place: fades and volume keyframes (lowered to `gain_db` keys), speed changes
   that keep the pitch (`audio.stretch/1`, WSOLA in `reelforge_media::stretch`, chained in fixed
   blocks so a mix made in slices matches one made whole), sound played backwards or along a speed curve
   (`time.reverse/1` read like tape; `time.ramp/1` through the same stretch), silent freeze frames (`time.hold/1`), a clip's pitch moved without
   changing its length (`audio.pitch/1`: read faster or slower, stretched back) and its sound changed before its gain (`audio.fx/1`,
   `reelforge_media::sound`: noise taken out against a measured floor, a band kept, a ring, an echo, each a function of the clip's
   own samples so slices still match the whole; a clip's `voice` in the model is lowered to these). Tools that listen before they
   edit (`reelforge_engine::listen`, over `reelforge_media::analysis`: BS.1770 loudness, beats, where a voice is heard, a noise
   floor) read a project and its media and answer with ordinary commands to run as one `edit.batch`; the server exposes them as
   the operations `audio.normalize`, `audio.beats`, `audio.duck`, `audio.denoise` and `audio.cutPauses` (the pauses in someone speaking, heard as levels a fiftieth of a second at a time and cut with `timeline.cut`, which takes stretches of time out of every track at once; `captions.fillers` finds the captions that are only a filler word, which are such stretches too), and the editor calls them with the media it
   holds. Two look instead of listening: `clip.pickChromaKey` reads the frame a clip shows (`reelforge_media::picture`) and keys
   out the colour most of its border has, and `clip.stabilize` watches every frame a video clip shows (`reelforge_media::steady`)
   and answers with the camera's path for `clip.setStabilize` to keep. A composition's own mix is made the same way and added under the gain of the clip
   that plays it. The export's mix (`reelforge_media::export::mix`) plays in the
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
