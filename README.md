# reelforge

An all-Rust short-video editor: the interface, the engine and the renderer are Rust, drawn with wgpu,
and run on the desktop and in the browser (WebAssembly). Every action is a command that people, scripts
and AI agents drive the same way, and rendering reads one versioned, self-contained description of the
edit (RenderSpec), so a render can happen in the editor or on a separate render service.

Status: early. Video, still images and sound import (Import in the media panel on the left, dropping files on the editor, or `media.import`; `media.remove` takes an unused file out; `media.set` renames a file or files it under a folder of the media panel, which has a search box as the sound, sticker and effect lists do) and
decode in pure Rust on the desktop and in the browser; solid colours, transforms, opacity, two colour
effects and undo work; projects export to MP4 from the desktop editor and the CLI (H.264 + AAC) and from the
browser (H.264, HEVC or AV1 through the browser's hardware encoder, WebCodecs), at a chosen
resolution (720p to 4K), frame rate and quality, or one of four presets (`project.setExport`). The same
settings choose another kind of file (`"format"`): an animated GIF (small and slow by its quality: 240, 360
or 480 px at 10, 12.5 or 20 frames a second), or the sound alone as M4A or WAV. `project.setCover` picks a
frame as the cover, saved as a JPEG beside an exported video (`cut.cover.jpg`), and the editor's export
window also saves the frame under the playhead as a PNG. A built-in
sound library (twelve effects and four music beds, synthesised by reelforge itself, so free to
use) adds with `sound.add`, and voice-overs record from the microphone on the desktop and in the
browser. A project can hold several timelines (cuts) that share its media: `timeline.add` (empty or a
copy), `timeline.switch`, `timeline.rename`, `timeline.remove`. Clips are grouped to be selected and moved as one
(`clip.group`, Ctrl+G), or joined into a compound clip (`clip.compound`, Alt+G): one clip that plays a timeline of
their own, placed, faded, masked and given effects as one picture, opened with a double-click to edit what is inside,
and taken apart again with `clip.uncompound`. CapCut / 剪映 drafts come in as a new timeline
by dropping their `draft_content.json` (or `capcut.import`): video, photo, sound and text segments with
speed, volume, placement, mirroring, fades and transitions; files the draft names that are not at hand
show as offline and relink when dropped in. Titles
render with shaping, wrapping, outlines, shadows, gradients and boxes, Chinese included (a bundled
Noto Sans SC subset covering GB 2312), and ten title templates (花字) apply a look in one step
(`title.templates`, `"template"` on `clip.add` / `clip.set`). Between two keyframes a value travels along a curve: a named one
(linear, hold, in, out, inOut, overshoot, anticipate) or one drawn by hand as a cubic Bézier, with `clip.setEase` or by dragging
the two handles of the graph in the inspector's Keyframes panel. A green (or any one-colour) screen is keyed out with `clip.setChromaKey`, and
`clip.pickChromaKey` finds the backdrop's colour in the frame by itself; picture effects (`fx.list`) include
glitch, colour split, mosaic and wave next to the shakes, zooms and blurs. SubRip captions come in by dropping a
`.srt` on the editor or `captions.import`, and go out with `captions.export`. **Auto captions** hear
the timeline with a local Whisper model (pure Rust on candle, CPU; nothing leaves the machine): the
editor's Auto captions button, or `captions.transcribe` for agents on a server. Get a model with
`reelforge-cli fetch-model --size small` (into `~/.reelforge/models/whisper`, or `REELFORGE_WHISPER`). **Voice-over** (配音)
reads the captions, or the titles picked, aloud with a local Kokoro-82M model and puts each line where its title
starts, on Voice-over tracks: the editor's Voice-over button, or `voice.speak` on a server (`voiceover.add` places
lines from any other source). Mandarin is turned into Kokoro's phonemes in-tree (pinyin, tone sandhi, numbers read in
Chinese). Get the model with `reelforge-cli fetch-model --kokoro` (into `~/.reelforge/models/kokoro`, or
`REELFORGE_KOKORO`). Someone speaking is tightened up in one step (剪口播): `audio.cutPauses` hears the pauses,
`captions.fillers` finds the captions that are only a filler word (嗯, 那个, um, …), and `timeline.cut` takes those stretches out of picture, sound and captions together, leaving the music as it is. Frames are composited on the GPU (wgpu) in the editor's preview and in exports,
with the CPU compositor as the reference and the fallback. Agents edit through an MCP server (stdio
or HTTP), and the same operations are an HTTP API and `reelforge-cli call`. People in a browser or on
another machine join a server's session and edit live alongside agents. The preview plays sound,
mixed as the export mixes it, with the picture following the sound. In the preview, what you see is
selected, moved, scaled and turned by dragging it. The timeline shows pictures
along video clips and the waveform of sound. The editor speaks English, Simplified
Chinese and Japanese: it follows the system (or browser) language, and
the pick in the top bar is remembered. A language is one `Strings` table in `crates/ui/src/lang/`.
Opened without a project file (and in the browser without a server), the editor keeps what you make
as drafts on the device: the drafts page lists them with a picture of each, to open another, start a
new one, or rename, copy or delete one. A narrow window (a phone held upright) shows the preview
above and the timeline, the media or the inspector under it, one at a time.
A template (剪同款) is an edit with slots where your pictures go: four are built in (`template.list`: beat
cuts, a vlog opener, cinematic, a showcase), each with music, titles, transitions and effects.
`template.apply` builds one, `template.fill` puts files in its slots in order (each keeps its length and
everything set on it; a picture of another shape is scaled to cover the frame, a video too short is slowed
to fill), and `clip.replace` swaps the file of any one clip. In the editor they are on the drafts page and
in the Templates tab on the left: pick one, then drop your files in. Any timeline becomes a template by
marking its picture clips as slots (`template.mark`, or a clip's menu).
A storyboard is a whole marketing video planned ahead: six are built in (`storyboard.list`: a product
launch, a flash sale, buy one get one, before and after, a roundup of three, a shop to visit). Each is
scenes in order, with a slot for your picture and a marker saying what to film, ready-made pieces over
it (price tags, checklists, coupons, buy buttons), transitions, music and the line to say. Its words are
fields with names (`product`, `price`, `cta`), in English, Chinese or Japanese: `storyboard.apply` builds
one from your copy and `storyboard.fill` gives fields other words later, and every piece written from
them is made again to fit. In the editor they are under Storyboards in the Templates tab, with the copy
as a form to type over.
A shaky shot is held steady (防抖): `clip.stabilize` watches the frames a video clip shows, follows how the
camera moved from each to the next, and keeps that path with the clip; the picture is then moved and
turned against the shake and drawn just large enough that its edges stay out of sight. How much goes
(`clip.setStabilize`, or the Stabilize box by a clip's speed) is changed without watching again: a little
takes the tremble out and leaves a pan a pan, all of it holds the picture nearly still.
A slowed clip has instants to show that its file has no frame for (补帧): `"frames"` on `clip.setSpeed`
(Smoothing, by the clip's speed) holds the frame before (`repeat`), mixes the two around it (`blend`, 帧融合),
or makes a frame in between by following what moved from the one to the other (`flow`, 光流).
The people in a video or picture are cut out without a green screen (智能抠像): `clip.setPersonKey`, or the Cut out
people box in a clip's look, keeps them and makes the rest see-through, so the tracks below show around them. A small
segmentation model bundled with the editor (`reelforge-matte`) looks at each picture as it is drawn, in pure Rust on
the machine or in the browser; it is made for a person facing the camera, and its outlines are soft.
Lottie animations (Bodymovin `.json`, dotLottie `.lottie`) are imported like any other file and placed like a video:
see-through where they draw nothing, going round again while their clip lasts, drawn in pure Rust (`reelforge-lottie`)
on the machine or in the browser. Shapes, strokes, gradients, trim paths, repeaters, masks, mattes, precompositions,
pictures and glyph text are drawn; expressions, effects, layer styles, text animators, blend modes and 3D layers are
not, and importing a file that uses them says so (`unsupported` in `media.import`'s answer, a note in the status bar).
Twenty decorations drawn for the editor come with it as Lottie animations (`decor.list`: confetti, fireworks,
sparkles, a tap ring, a bouncing arrow, an underline and a ring drawn by hand, falling coins, a flame…), free to use
and needing no files. `decor.add` puts one over the picture in the brand's colours (or the ones given, or its own red
and yellow); in the editor they are under Animated in the Stickers tab, each playing while the pointer is over it.
A clip is placed in space as well as on the flat of the frame: it is stretched one way more than the other
(`scaleX`, `scaleY`), tipped and swung like a card (`rotationX`, `rotationY`, seen from a distance of `perspective`
frame heights), and placed, scaled and turned about any point of itself (`anchorX`, `anchorY`: a price that grows
from its foot, a badge that swings open on its edge). All of them are keyframed like the rest, and the `flipX`,
`flipY`, `flip` and `tilt` animations use them. One clip follows another (`clip.setParent`): a card's picture, title
and price go wherever the card goes, turn as it turns and are seen in the same perspective. And a clip travels along
a path instead of standing in one place (`clip.setPath`): a smooth curve through points, at an even speed, facing the
way it goes if asked. In the editor they are under 3D & anchor in a clip's inspector.
A title's words move one after another as its characters do (`wordFade`, `wordRise`, `wordPop`; a Chinese character
is a word of its own), and its lines (`lineRise`); characters spring up past their size and back (`charElastic`) or
fall and bounce (`charBounce`), and any clip does the same as a whole (`elastic`, `drop`). A title is written on
(`draw`): a pen follows each character's outline before its ink comes in. And what is being said takes another colour
(`karaoke` on `clip.set`, the Karaoke box by a title's style): everything said so far, the edge passing smoothly
through the character being said, or the word or line being said alone. Give one caption a karaoke and
`captions.restyle` gives it to the rest, each over its own length.
Between two clips that touch on a video track (`clip.setTransition`, Transition in a clip's inspector), besides the
dissolve, dips, pushes, slides and zoom: a soft edge wipes the next clip in from any side (`wipeLeft`, `wipeRight`,
`wipeUp`, `wipeDown`); it opens from the middle through a circle, a box or a diamond; the last clip turns away in
space like a card with the next on its back (`flip`, `flipUp`); the next is turned in like a page (`pageTurn`); warm
light drifts across as it fades in (`lightLeak`); the picture tears into bands with its colours apart and cuts
(`glitch`); or the camera goes into one clip out of focus and comes back out of the other (`zoomBlur`).
A clip can be seen through a shutter (`motionBlur` on `clip.set`, Motion blur under Transform in its inspector): it is
smeared along its own way while it moves, grows or turns, by keyframes, animations, transitions, a path or whatever
carries it, and is as sharp as ever where it rests.
See [ARCHITECTURE.md](ARCHITECTURE.md).

Video and audio codecs and containers are FilmCraft's clean-room, pure-Rust crates
(`filmcraft-codecs`, `filmcraft-h264enc`, `filmcraft-aac`, `filmcraft-isobmff`; MIT OR Apache-2.0):
no FFmpeg and no C libraries. The model that cuts people out is MediaPipe's selfie segmentation (Apache-2.0),
bundled as published; see [crates/matte/models](crates/matte/models/README.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

## Build and run

Rust 1.95 or newer. On Linux the editor plays sound through ALSA: install `libasound2-dev` (Debian,
Ubuntu) or `alsa-lib-devel` (Fedora) to build it.

```sh
cargo run -p reelforge                      # the editor (the draft edited last; a demo the first time)
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
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.setAnimation '{"clip": "c6", "slot": "loop", "kind": "wave"}' --save   # its characters, round and round
cargo run -q -p reelforge-cli -- exec cut.reel.json promo.add '{"kind": "priceTag", "value": 129.9, "y": -0.2}' --save   # a price tag whose price counts up (promo.list has the other 29: coupons, checklists, ratings, buy buttons, end cards…)
cargo run -q -p reelforge-cli -- exec cut.reel.json brand.set '{"primary": [0.05, 0.35, 0.95], "secondary": [1, 0.85, 0.3]}' --save   # ready-made pieces take the brand's colours from here on
cargo run -q -p reelforge-cli -- exec cut.reel.json promo.add '{"kind": "checklist", "lines": ["持久保湿 12 小时", "敏感肌适用"], "compound": true}' --save   # a list, folded into one clip (→ c9)
cargo run -q -p reelforge-cli -- exec cut.reel.json promo.set '{"clip": "c9", "lines": ["持久保湿 12 小时", "敏感肌适用", "无香精"]}' --save   # other words: the piece is made again to fit them
cargo run -q -p reelforge-cli -- exec ad.reel.json storyboard.apply '{"storyboard": "flashSale", "copy": {"price": 59, "oldPrice": 89}}' --save   # a whole flash-sale video planned ahead: scenes, pieces, music (storyboard.list has six)
cargo run -q -p reelforge-cli -- exec ad.reel.json storyboard.fill '{"copy": {"discount": "直降 30 元", "stock": 12}}' --save   # other copy: every piece written from it is made again
cargo run -q -p reelforge-cli -- exec ad.reel.json decor.add '{"decor": "confettiBurst", "start": {"seconds": 12}}' --save   # confetti over the closing scene, in the brand's colours (decor.list has twenty animated decorations)
cargo run -q -p reelforge-cli -- exec ad.reel.json decor.add '{"decor": "circleMark", "start": {"seconds": 6}, "x": 0.1, "y": 0.2, "color": [1, 0.8, 0.2]}' --save   # a ring drawn by hand round the price
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.add '{"track": "t1", "shape": {"kind": "star", "points": 16, "inner": 0.8, "size": [300, 300], "fill": {"type": "radial", "from": [1, 0.8, 0.2], "to": [1, 0.4, 0]}}}' --save   # a shape of your own
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.setKey '{"clip": "c5", "prop": "scale", "at": {"seconds": 4}, "value": 1.3, "ease": "inOut"}' --save
cargo run -q -p reelforge-cli -- exec cut.reel.json clip.setEase '{"clip": "c5", "at": {"seconds": 4}, "ease": [0.3, 1.5, 0.6, 1]}' --save   # on from that key: past the next one and back
cargo run -q -r -p reelforge-cli -- export cut.reel.json -o cut.mp4      # a project, or a RenderSpec
# frames are drawn on the GPU when there is one; --renderer cpu|gpu chooses
cargo run -q -r -p reelforge-cli -- export cut.reel.json -o cut.gif --format gif   # or m4a, wav: the sound alone
cargo run -q -p reelforge-cli -- exec cut.reel.json project.setCover '{"at": {"seconds": 2}}' --save   # cut.cover.jpg beside the next video
```

## Agents (MCP) and the HTTP API

`reelforge-cli serve` puts a project behind the Model Context Protocol: every command is a tool
(`clip_split`, `clip_add`…, each with a JSON Schema), plus `render_frame` (returns a PNG the agent can
look at), `render_export` and `project_save`. Every edit is saved to the project file as it commits.
[docs/agents.md](docs/agents.md) is the guide for agents: setting up Claude Code, Claude Desktop and
other clients, how the tools take time and ids, and which tools each feature uses.

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
