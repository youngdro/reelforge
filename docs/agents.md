# Editing with an agent (MCP)

reelforge exposes every editing command to AI agents through the Model Context Protocol. The editor's
buttons, the CLI, the browser's `window.reelforge` API and MCP all run the same commands
(`crates/engine/src/commands.rs`), so an agent can do anything a person can do in the editor, except
for the drafts page, which only manages the editor's own drafts. An agent can also watch its own work:
`render_frame` returns a PNG of any instant.

## Set up

Build the CLI once; it holds the server:

```sh
cargo build -r -p reelforge-cli          # target/release/reelforge-cli
```

### Claude Code

```sh
# the agent starts its own server over stdio, on one project file (created if missing)
claude mcp add reelforge -- /abs/path/target/release/reelforge-cli serve /abs/path/cut.reel.json --stdio

# or run one server and connect to it over HTTP (Streamable HTTP at /mcp)
reelforge-cli serve cut.reel.json --listen 127.0.0.1:7878
claude mcp add --transport http reelforge http://127.0.0.1:7878/mcp
```

### Claude Desktop and other MCP clients

Most clients take a command to start a stdio server. In Claude Desktop's
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "reelforge": {
      "command": "/abs/path/target/release/reelforge-cli",
      "args": ["serve", "/abs/path/cut.reel.json", "--stdio"]
    }
  }
}
```

Clients that speak Streamable HTTP connect to `http://host:7878/mcp`. On any address other than
loopback the server refuses to start without a token (`--token SECRET` or `REELFORGE_TOKEN`), and
clients send `Authorization: Bearer SECRET`.

### Editing alongside a person

To have the agent edit while someone watches, let the desktop editor host the server; the agent's
edits appear in the window as they commit, and undo covers both:

```sh
cargo run -r -p reelforge -- cut.reel.json --serve          # MCP at http://127.0.0.1:7878/mcp
reelforge-cli serve cut.reel.json --web dist                # or a browser editor on the same session
```

### Without MCP

The same operations are a plain HTTP API and a shell command:

```sh
curl -s localhost:7878/v1/ops                                         # every operation and its schema
curl -s -X POST localhost:7878/v1/ops/clip.split -d '{"clip":"c4","at":{"seconds":2}}'
reelforge-cli call project.outline --project cut.reel.json            # on a file, no server
reelforge-cli call render.frame at=1.5s --server http://127.0.0.1:7878 -o frame.png
```

## How the tools work

- **Names.** Command `clip.setKey` is the tool `clip_setKey` (many clients refuse dots). Every tool has
  a JSON Schema with a description of each parameter; `tools/list` returns all of them.
- **Read first.** `project_outline` gives the timeline in brief: tracks and their clips (start and end
  in seconds, what each shows, and what is set on it), media with their lengths, markers and cover.
  `project_inspect` is the full project document, with times in ticks.
- **Time.** Pass `{"seconds": 2.5}` or `{"frame": 75}`. A bare integer is ticks (254,016,000,000 per
  second); replies from editing commands give ticks, and `project_outline` gives seconds.
- **Ids.** Tracks are `t…`, clips `c…`, media `m…`, groups `g…`, timelines `s…`. A clip's id stays the
  same when it is moved or trimmed; splitting keeps the left half's id and replies with the right
  half's id.
- **Mistakes are refused, not guessed.** A command that cannot apply (no such clip, a cut outside the
  clip, an overlap) replies with `isError: true`, says why, and changes nothing. A parameter the tool
  does not take is refused the same way and lists the parameters it does take.
- **Undo.** Every edit is one undo step (`edit_undo`, `edit_redo`). `edit_batch` runs several
  commands as one step and applies none of them if one fails. A step can use what an earlier step
  replied: `"${0.clip}"` is the clip that step 0 added, and `"${2.clips.0.clip}"` is the first of a
  list.
- **Files.** `media_import` takes a path on the server's disk. A client on another machine first
  sends the bytes with `PUT /v1/media/{sha256}?name=clip.mp4`, then imports the path it gets back.
  Fonts (`.ttf`, `.otf`), LUTs (`.cube`) and Lottie animations (`.json`, `.lottie`) are media as well.
- **Saving.** With a project file, every edit is written to it as it commits. `project_save` with
  `path` writes a copy.

## A first video

```text
project_outline                                   → canvas 1080×1920, empty tracks t1 (video), t2 (text), t3 (audio)
media_import {"path": "/clips/talk.mp4"}          → m4
clip_add {"track": "t1", "media": "m4"}           → c5
audio_cutPauses {"apply": false}                  → the pauses it would cut; then {} to cut them
clip_add {"track": "t2", "text": "三个剪辑小技巧", "y": -0.3, "template": "candy", "duration": {"seconds": 3}}
clip_setAnimation {"clip": "c9", "slot": "in", "kind": "charPop"}
sound_add {"sound": "…"} or media_import + clip_add on t3, then audio_duck {"clip": …}
render_frame {"at": {"seconds": 1}, "scale": 0.3}  → look, then fix
render_export {"path": "out.mp4"}
```

## A marketing video from a storyboard

```text
storyboard_list {"lang": "zh"}                    → launch, flashSale, bogo, beforeAfter, roundup, storeVisit
                                                    painSolution, unboxing, reviews, liveTeaser, holidaySale, howTo
brand_set {"primary": [0.05, 0.35, 0.95]}         → the pieces and backdrops take the brand's colours
storyboard_apply {"storyboard": "launch", "copy": {"product": "云朵拖鞋", "price": 59, "oldPrice": 89}}
                                                  → slots 1–4, scenes (each with `shot`: what to film, and
                                                    `voice`: the line to say), pieces, copy
media_import … then template_fill {"media": ["m4", "m5", "m6", "m7"]}
storyboard_fill {"copy": {"cta": "马上带一双回家", "price": 49.9}}
                                                  → the button is as wide as its new words, the tag counts to 49.9
voiceover_add with the scenes' `voice` lines, or storyboard_apply with "captions": true
decor_add {"decor": "circleMark", "start": {"seconds": 14}, "x": 0.13, "y": 0.2, "scale": 0.6}
                                                  → a ring drawn by hand round the price; decor_list has confetti,
                                                    sparkles, a tap ring and the rest, in the brand's colours
render_frame {"at": {"seconds": 14}, "scale": 0.3} → look, then fix: promo_set moves or rewrites one piece
```

A product photograph on a white or studio backdrop goes over any scene: `clip_setCutout {"clip": …}`
takes the backdrop away, `clip_setShadow {"clip": …, "kind": "ground"}` stands the product on the
floor of the scene, and `promo_add {"kind": "productStage", "media": "m4", "text": "…"}` does both
in a pool of light with the name under it.

Two shots of the same thing, before and after, go one over the other and are told apart by a
divider: `clip_add` the second on a track above the first for the same stretch of time, then
`layout_apply {"layout": "compare", "clips": ["c1", "c2"]}`. `"layout": "pip"` makes the second a
small picture in a corner instead (a presenter over the product), `"columns"` or `"grid"` puts
two to six shots side by side.

A montage that keeps time with its music: put the song on an audio track and mark its beats
(`audio_beats`, a listening tool, needs the file), or skip that when the tempo is known and pass
`"bpm": 120` (and `"first"`, the time of the first beat) to the two commands below.
`beat_cut {"clips": ["c1", "c2", "c3"], "every": 2, "move": "punch"}` lays the clips of one track end
to end, each two beats long, every one arriving with a punch; `beat_apply {"clips": ["c9"], "move":
"squash", "every": 4}` then makes a price tag or a title squash on the first beat of each bar. The
moves are keyframes: `clip_clearKeys` takes one off, and a second `beat_apply` with the same move
replaces the first.

A storyboard is laid out for an upright 9:16 frame, its pieces clear of the bottom fifth where
apps put their own buttons and captions. On a wider canvas the pieces keep to a column down the
middle, smaller. A field a piece counts to (`price`, `stock`, `rating`) must be a number. A piece
told its words directly (`promo_set` with `text`) no longer follows its field. Captions written by
`captions: true` follow the copy too, until one is changed by hand. The built-in font
covers Chinese and Latin; for Japanese copy import a font and make it the brand's (`brand_set`)
before building.

## A product card that turns in space

```text
clip_add {"track": "t2", "shape": {"kind": "rectangle", "size": [780, 1040], "radius": 56, "fill": [1, 1, 1, 1]}}
                                                  → c7, the card
clip_set {"clip": "c7", "perspective": 1.4}
clip_setAnimation {"clip": "c7", "slot": "in", "kind": "flipY"}     → it turns in like a door
clip_setAnimation {"clip": "c7", "slot": "loop", "kind": "tilt"}    → then sways, seen in perspective
clip_add the product picture, the name and the price on tracks above, placed where they belong on the card
clip_setParent {"clip": "c9", "parent": "c7"}     → for each: it stays where it shows and now goes with the card
clip_set {"clip": "c13", "anchorX": 0.5, "anchorY": 1}              → the price stands on its foot
clip_setKey {"clip": "c13", "prop": "scaleY", "at": {"seconds": 1}, "value": 0, "ease": "overshoot"}
clip_setKey {"clip": "c13", "prop": "scaleY", "at": {"seconds": 1.5}, "value": 1}
clip_add a star, then clip_setPath {"clip": "c20", "points": [[0.4, -0.06], [0, 0.26], [-0.4, -0.06], [0, -0.4]],
                                    "closed": true, "orient": true, "duration": {"seconds": 4.5}}
                                                  → it goes round the card, facing the way it goes
render_frame {"at": {"seconds": 1.2}, "scale": 0.3} → look, then fix
```

A clip carried by another is placed in its parent's space: its `x`, `y` are from the parent's anchor,
in canvas units before the parent's scale and turn, and `project_outline` shows them so. Give a clip
to its parent after placing it and it stays where it was. A parent passes on where it is, not how it
looks: its opacity, effects and its start and end stay its own, so keep the clips the same length. A
parent that is hidden or removed carries nothing. Layers are drawn in track order whatever their
turn: a card turned past 90° still has the tracks above drawn over it. Keeping a picture in place
under a new anchor or parent is exact while nothing in the chain is turned in space, so set parents
at an instant when the card lies flat (`at`). A clip on a path is not moved by `x`, `y` until the
path is taken away (`points: null`).

## What each feature calls

| Feature | Tools |
| --- | --- |
| Cutting and arranging | `clip_add`, `clip_split`, `clip_trim`, `clip_move`, `clip_remove` (`ripple`), `clip_roll`, `clip_slip`, `clip_shift`, `clip_copy` / `clip_paste` / `clip_duplicate`, `timeline_cut`, `track_add` / `track_set` / `track_move`, `marker_*` |
| Placement and keyframes | `clip_set` (x, y, scale, rotation, opacity, volume; at `at` once keyframed), `clip_setKey`, `clip_setEase` (named curves or a cubic Bézier), `clip_removeKey`, `clip_clearKeys` |
| 3D, anchor, following and paths | `clip_set` with `scaleX` / `scaleY` (stretch one way), `rotationX` / `rotationY` (degrees: tipped back, swung like a door; keyframable like the rest with `clip_setKey`), `perspective` (how far the eye is, in frame heights: 2 by default, less is stronger), `anchorX` / `anchorY` (the point of the clip that `x`, `y` place and that it scales and turns about, as fractions from its top left; the picture stays where it was unless `keep: false`), `clip_setParent` (`clip`, `parent` or null: the clip is placed inside its parent and goes, scales and turns with it, in its perspective; it stays where it shows unless `keep: false`), `clip_setPath` (`points` in canvas coordinates or null, `closed`, `orient`, `start`, `duration`, `ease`: the clip travels the smooth curve through them at an even speed instead of standing at x, y), `clip_setAnimation` kinds `flipX` / `flipY` (in and out) and loops `flip` / `tilt` |
| Motion blur | `clip_set` with `motionBlur`: true (a film camera's shutter, half of each frame's time), a number 0 to 2 for a shorter or longer smear, false to turn it off. The clip is smeared along its own movement (keyframes, animations, transitions, a path, a parent's movement) and sharp at rest; set it on the clips that move fast, a title flying in, a product sliding across, a badge spinning |
| Speed and smoothness | `clip_setSpeed` (`frames`: repeat, blend or flow for frames in between), `clip_setSpeedCurve` (`curve`: a preset or points), `clip_setReverse`, `clip_freeze`, `clip_stabilize` (measures the camera's path) and `clip_setStabilize` |
| Crop, mask, blend | `clip_setCrop`, `clip_setMask`, `clip_setBlend`, `clip_setBackdrop` (canvas blur); crop and mask edges are keyframable (`cropLeft`…, `maskX`…) |
| Colour | `clip_setColor` (filters, LUT media, adjustments; adjustments are keyframable) |
| Green screen, cutout and shadow | `clip_pickChromaKey` (finds the backdrop colour and keys it), `clip_setChromaKey`, `clip_setPersonKey` (cuts a person out without a green screen), `clip_setCutout` (lifts a product off the plain backdrop of its photograph, white and grey included: `tolerance` up when the backdrop's shadows or creases stay, down when pale parts of the product go; `holes` for the backdrop seen through a handle; it needs one colour all round the subject, so look with `render_frame` after), `clip_setShadow` (a shadow under a video, picture or compound clip, of its outline as it shows: `kind` drop, ground, long or glow, then `color`, `opacity`, `x`, `y`, `blur`, `flat`, `lean` one at a time) |
| Effects and stickers | `fx_list`, `clip_setFx` (including the distortions), `clip_add` with `fx` or `adjustment: true` (an effect layer over the tracks below), `sticker_library`, `clip_add` with `sticker` |
| Titles and fonts | `clip_add` / `clip_set` with `text` and style (`font` is an imported font's media id, `tracking`, `leading`, `italic`, `underline`…), `title_templates` and `template` |
| Shapes, numbers and ready-made pieces | `clip_add` with `shape` (rectangle, ellipse, line, polygon, star, tag, arrow; gradient fill, outline, shadow), `clip_setShape` (`drawStart` / `drawEnd` and the `draw` animation draw the outline on), `clip_setCounter` (a title's `{n}` runs from one number to another: a price, a sales count, a countdown), `promo_list` (37 pieces by `category`: price, product, titles, action, rhythm; `about` says what each one's parameters mean, `media` whether it shows a picture), `promo_add` (`text`, `lines`, `value`, `values`, `color`, `color2`, `side`, `media` for the picture a `productCard` carries or a `productStage` cuts out and stands in the light, placement; grouped clips, or one compound clip with `compound: true`; the parts of a card are carried by it, so moving, scaling or turning the card's own clip takes them along), `promo_set` (gives a folded piece other words, lines, numbers, colours, another picture or another place and makes its inside again to fit; `promo_get` reads what it was made as), `brand_set` / `brand_get` (the colours, font and logo new pieces take after) |
| Lottie animations | `media_import` a `.json` or `.lottie` file (kind `lottie`; `unsupported` in the reply lists what the file uses that is not drawn: expressions, effects…), then `clip_add` on a video track like a video. It is see-through where it draws nothing and goes round again when its clip is longer than it is |
| Animated decorations | `decor_list` (twenty that come with the editor, free to use: confettiBurst, confettiRain, fireworks, sparkles, starBurst, heartBurst, tapRing, pulse, arrowBounce, swipeUp, emphasis, underline, circleMark, checkDraw, coinDrop, flame, bolt, giftShake, clockTick, loadingDots; for each its usual length, whether it plays once or goes round, its usual size and which colours it takes), `decor_add` (`decor`, `start`, `duration`, `color`, `color2`, `x`, `y`, `scale`, `rotation`, `opacity`, `track`: a Lottie clip on a free video track over whatever shows then, in the colours given, else the brand's, else its own red and yellow). The clip is like any other afterwards: `clip_set`, `clip_setAnimation`, keyframes |
| Animations | `clip_setAnimation` (`in`, `out`, and `loop` for one that repeats; the `char*` kinds animate each character) |
| Transitions (into a clip from the one before it on a video track, while they touch) | `clip_setTransition`: `dissolve`, `fadeBlack`, `fadeWhite`, `push*`, `slide*`, `zoom`; `wipeLeft`/`wipeRight`/`wipeUp`/`wipeDown` (a soft edge uncovers the clip); `circle`, `box`, `diamond` (it opens from the middle; a clip with a mask of its own dissolves instead); `flip`, `flipUp` (a card turned over in space); `pageTurn`; `lightLeak`; `glitch`; `zoomBlur` (the blur shows on pictures and video) |
| Text that moves and karaoke | `clip_setAnimation` kinds for titles: `wordFade`, `wordRise`, `wordPop` (word by word; a Chinese character is a word of its own, with the punctuation after it), `lineRise` (line by line), `charElastic` (each character springs up past its size and back), `charBounce` (each falls and bounces), `draw` (written on: a pen follows each character's outline, then its ink comes in; give it a second or two). For any clip: `elastic` (grows in on a spring), `drop` (falls in and bounces). `clip_set` with `karaoke` (`color`, yellow unless given; `mode`: `sweep` colours everything said so far, `word` or `line` the one being said alone; `start` and `duration`, the whole clip unless given; null removes it). The voice goes through the characters at an even pace. `captions_restyle` copies a caption's karaoke to the others, each over its own length |
| Captions | `captions_transcribe` (local Whisper on the server), `captions_add`, `captions_import` / `captions_export` (SubRip; `line: 2` for a second language), `captions_list`, `captions_split`, `captions_merge`, `captions_replace`, `captions_restyle`, `captions_shift` |
| Talking-head cleanup | `audio_cutPauses`, `captions_fillers`, `captions_cutFillers` |
| Sound | `sound_library` / `sound_add`, `clip_setVoice` / `voice_list`, `clip_setDenoise`, `audio_denoise`, `audio_normalize`, `audio_beats` (a marker on each beat), `audio_duck`, `clip_detachAudio` |
| Groups and compound clips | `clip_group`, `clip_ungroup`, `clip_compound`, `clip_uncompound`, `timeline_list` / `timeline_switch` |
| Timelines and templates (剪同款) | `timeline_add` / `timeline_switch` / …, `template_list`, `template_apply`, `template_slots`, `template_fill`, `template_mark`, `clip_replace` |
| Storyboards (whole marketing videos planned ahead) | `storyboard_list` (launch, flashSale, bogo, beforeAfter, roundup, storeVisit: scenes with what to film and the line to say, the fields of the copy; `lang` en, zh or ja), `storyboard_apply` (`copy` by field name, `media` for the scenes' pictures in order, `seconds`, `captions`; builds slots, pieces, transitions, markers and music in one step), `storyboard_fill` (other words or numbers for fields: every piece written from them is made again), `storyboard_copy`; pictures go in with `template_fill` |
| Layouts (split screen, picture in picture, compare) | `layout_list` (columns, rows, grid, focus, pip, compare: how many clips each takes and which parameters it reads), `layout_apply` (`layout`, `clips` in order, on tracks one above another and showing at the same time; `gap`, `margin`; for `pip` `corner`, `size`, `round`; for `compare` `from`, `to`, `axis`, `divider`; `border` outlines the cells. Each clip is cropped about its middle to its cell and scaled to fill it; its place, scale, turn, crop, mask, path and carrier become the layout's. Outlines and the divider are shape clips carried by their clip; applying again replaces them. Reframe inside a cell with `clip_setCrop`) |
| On the beat (卡点) | `beat_list` (punch, pulse, shake, flash, strobe, tilt, bounce, squash: what each looks like and the properties it keys), `beat_apply` (`clips`, `move`, `strength`; the beat is the timeline's ♪ markers from `audio_beats`, or `bpm` with `first`, or `at`; `every`, `offset`), `beat_cut` (`clips` of one track in order, each ending on the next beat that counts; `every`, `offset`, `from`, `move` for a move at each cut) |
| Media | `media_import`, `media_set` (rename, folders), `media_remove` |
| CapCut / 剪映 drafts | `capcut_import` |
| Looking and exporting | `render_frame`, `render_compile`, `project_setCanvas`, `project_setExport`, `project_setCover`, `render_export` (`format`: mp4, gif, m4a, wav), `render_jobs` |

## Exporting

`render_export {"path": "out.mp4"}` replies when the file is written. An export takes roughly as long
as the video on the CPU renderer, and many MCP clients give up on a tool call after about 60 seconds,
so for anything longer pass `"wait": false`: the reply is a job id at once, and `render_jobs` reports
frames done out of the total and, at the end, what the export wrote. Relative paths are beside the
project file. A video's cover (`project_setCover`) is written beside it as `<name>.cover.jpg`.
Background jobs belong to a running server: `reelforge-cli call` on a file exits when the call
returns, so use `wait: false` only with `serve`.

## Limits

- Paths are on the server's disk; a remote client uploads first (see Files above).
- The listening tools (`audio_*`, `clip_pickChromaKey`) analyse at most the first five minutes of a
  clip's sound.
- Mid-sentence filler words are not cut: `captions_cutFillers` only removes captions that contain
  nothing but filler words, because captions carry no per-word times yet.
