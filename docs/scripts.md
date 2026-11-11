# Motion scripts

A motion script is a piece of JavaScript that makes nodes (shapes, titles, pictures) and tweens
them on a timeline, in the words GSAP uses. `script.add` runs it **once** and turns what it built
into ordinary clips with keyframes, grouped together, which a person can go on editing by hand.
The script and its parameters are kept with the group, so the group is built again when either
changes.

```js
export const params = {
  title:  { type: "text",   default: "Flash sale", label: "Title" },
  price:  { type: "number", default: 199, min: 0,  label: "Price" },
  accent: { type: "color",  default: "#ff3b5c",    label: "Accent" },
};

export function build({ params, canvas, scene, timeline }) {
  const card  = scene.rect({ width: 0.8 * canvas.width, height: 420, radius: 48, fill: "#fff",
                             shadow: { color: "rgba(0,0,0,0.35)", offset: [0, 18], blur: 30 } });
  const title = scene.text(params.title, { y: -130, size: 64, color: "#222" });
  const price = scene.text("¥{n}", { y: 30, size: 200, color: params.accent,
                                     counter: { from: 299, to: params.price, duration: 0.9 } });

  const tl = timeline({ defaults: { duration: 0.5, ease: "power3.out" } });
  tl.from(card,  { y: "+=600", opacity: 0, ease: "back.out(1.4)" })
    .from(title, { scale: 0.6, opacity: 0 }, "-=0.25")
    .from(price, { scale: 0, duration: 0.8, ease: "elastic.out(1, 0.5)" }, "<0.1")
    .to([card, title, price], { opacity: 0, duration: 0.3 }, "+=1.5");
}
```

```sh
reelforge-cli call script.check -p my.reel.json source=@card.js          # what it would build, or what is wrong
reelforge-cli call script.add -p my.reel.json source=@card.js start=2s   # → {"group": "g7", "clips": [...]}
reelforge-cli call script.setParams -p my.reel.json '{"group": "g7", "params": {"price": 99}}'
```

Over MCP the same commands are the tools `script_check`, `script_add`, `script_setParams`…

The whole of a longer one is in [examples/price-card.js](examples/price-card.js). [examples/flip-card.js](examples/flip-card.js)
turns a card in space with what it carries, grows a bar from its left end and sends a star along a path.

## Units

- Lengths are **canvas pixels**. `x` and `y` are from the **centre** of the canvas, `y` down.
  `canvas.width`, `canvas.height` and `canvas.fps` are given, so `0.8 * canvas.width` keeps a
  script good for any canvas.
- Time is **seconds** from the script's start. Keys land on frames.
- Angles are degrees clockwise. Opacity is 0 to 1. Scale 1 is as made.
- Colours are CSS strings (`"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, `"rgb(…)"`, `"rgba(…)"`, a few
  names) or `[r, g, b, a]` with each from 0 to 1.

## `scene`: nodes

Every node takes `x`, `y`, `scale`, `rotation`, `opacity` and a `name` (which error messages and
replies use). Later nodes lie over earlier ones. Every node also takes:

- `scaleX`, `scaleY`: width and height apart, on top of `scale` (negative mirrors);
- `rotationX`, `rotationY`: turned in space, in degrees (`rotationY` swings it like a door,
  `rotationX` tips it like a lid; past 90 it shows its back), seen from `perspective` pixels away
  (two canvas heights unless told; nearer is stronger);
- `transformOrigin` (or `origin`): the point it is scaled and turned about, `"left center"`,
  `"50% 100%"`, or `[across, down]` as shares of its own box. As in CSS the node does not move:
  `x` and `y` go on saying where its middle rests;
- `parent`: a node that carries it. Its `x` and `y` are then counted from the parent's origin along
  the parent's axes, so moving, scaling or turning the parent (in space too) takes it along as one
  object. Opacity is not passed on. Up to 8 deep;
- `motionBlur`: `true`, or the share of a frame the shutter is open, smears it along its own
  movement;
- `mask`: what of the node shows, `"rectangle"`, `"circle"`, `"linear"` or `"mirror"`, or
  `{shape, x?, y?, width?, height?, rotation?, feather?, invert?}` in the node's own measures (`x`
  and `y` shares of its box, 0.5 the middle; `width` and `height` in its shorter side). Tween
  `maskX`, `maskY`, `maskWidth`, `maskHeight`, `maskRotation`, `maskFeather` to wipe it on or open
  it like an iris, as CSS's `clip-path` does;
- `animate`: the editor's own animations, `{in?, out?, loop?}`, each a kind's name or
  `{kind, duration?}` (`clip.setAnimation` lists them). For a title these go character by
  character, word by word or line by line (`charRise`, `wordPop`, `lineRise`, `typewriter`,
  `wave`…), which is what a script has in place of `SplitText`. `in` plays from the script's start
  and `out` up to its end.

GSAP's names `rotate`, `rotateX`, `rotateY`, `autoAlpha` and `alpha` are understood.

| Call | Options besides those |
| --- | --- |
| `scene.rect(opts)`, `scene.ellipse(opts)`, `scene.polygon(opts)`, `scene.star(opts)`, `scene.tag(opts)`, `scene.arrow(opts)` | `width`, `height`, `radius` (corners), `points` and `inner` (polygon, star), `fill` (a colour, `{type: "linear", from, to, angle?}`, `{type: "radial", from, to}` or `null`), `stroke: {color, width}`, `shadow: {color, offset: [x, y], blur}`, `trim: [start, end]` |
| `scene.circle(opts)` | the same, with `radius` its radius |
| `scene.line(opts)` | `width` (its length), `stroke` |
| `scene.text(words, opts)` | `size` (pixels), `color`, `weight`, `align`, `wrap`, `outline: {color, width}`, `outer`, `shadow`, `background`, `radius`, `gradient`, `font` (a media id), `tracking`, `leading`, `italic`, `underline`, `template`, and `counter: {from, to, duration?, ease?, decimals?, group?, clock?}`, which writes a running number where the words say `{n}`. Widths of outlines and shadows are in ems, as `clip.set` takes them |
| `scene.image(mediaId, opts)`, `scene.video(mediaId, opts)` | `sourceIn` (seconds into a video) |
| `scene.sticker(id, opts)` | (see `sticker.library`) |
| `scene.group(opts)` | Nothing to see: a node to be the `parent` of others, so they move, grow and turn as one. `width`, `height` matter only to its `transformOrigin`. It has no opacity; fade what it carries |

`scene.setDuration(seconds)` says how long the piece is. Without it, it lasts until its last tween
ends; `script.add`'s own `duration` overrides both.

## `timeline`: tweens

`timeline({ defaults?, delay? })` makes a timeline; a script may make several, all starting with the
script.

- `tl.to(nodes, vars, position?)`, `tl.from(nodes, vars, position?)`,
  `tl.fromTo(nodes, fromVars, toVars, position?)`, `tl.set(nodes, vars, position?)`,
  `tl.addLabel(name, position?)`, `tl.duration()`.
- `nodes` is a node or a list of nodes.
- `vars` names the properties and, beside them, `duration` (0.5), `ease` (`"power1.out"`),
  `delay`, `stagger` (seconds, or `{each | amount, from: "start" | "end" | "center" | "edges"}`),
  `repeat` (a whole number of extra times), `yoyo`, `repeatDelay`.
- A value is a number, or `"+=n"` / `"-=n"` from where the property is.
- **Properties**: `x`, `y`, `scale`, `rotation`, `opacity`, `scaleX`, `scaleY`, `rotationX`,
  `rotationY` on every node; `maskX`, `maskY`, `maskWidth`, `maskHeight`, `maskRotation`,
  `maskFeather` on a node with a `mask`; `fill` on a shape made with a fill and `color` on a title
  (a colour as anywhere else; of a gradient, the colour it starts from; each of red, green, blue
  and alpha goes its own straight way, so fade with `opacity` rather than to a transparent black);
  the colour adjustments
  `exposure`, `contrast`, `saturation`, `temperature`, `tint`, `highlights`, `shadows`, `fade`,
  `vignette`; `drawStart` and `drawEnd` on a shape with a stroke (0 to 1 round its outline: from 0
  to 1, `drawEnd` draws it on); `count` on a title with a counter; `cropLeft`, `cropTop`,
  `cropRight`, `cropBottom` on a picture. A property a node has not is refused, with those it has.
- **Paths**: `tl.to(node, { motionPath: [{x, y}, …], duration, ease })` sends a node through those
  points on a smooth curve at an even pace, from where it is (`{path, fromCurrent: false}` starts
  at the first point instead), `autoRotate: true` turning it to face the way it goes and
  `closed: true` bringing it round to where it set off. A node travels one path, once; its ease is
  one a curve can say; `x` and `y` are not tweened on a node that has a path, everything else is.
- **Positions**: seconds (`1.5`), `"+=0.2"` and `"-=0.2"` from the timeline's end, `"<"` and `">"`
  for the start and end of the tween before (`"<0.1"`, `">-0.1"`), a label (`"intro"`,
  `"intro+=0.3"`). With none, a tween follows the last.
- **Eases**: `none`, `power1` to `power4` (`quad`, `cubic`, `quart`, `quint`), `sine`, `expo`,
  `circ`, `back(overshoot)`, `elastic(amplitude, period)`, `bounce`, each with `.in`, `.out` or
  `.inOut`; `steps(n)`; `hold`; `"cubic-bezier(x1, y1, x2, y2)"` or `[x1, y1, x2, y2]`; or a function
  of 0..1.

## What a tween becomes

A tween is keyframes on the node's clip. An ease a cubic Bézier can say is two keys with that curve
between them, which the inspector's curve editor shows and changes: exactly for `power1`, `power2`
and `back` in and out, closely for the others. `elastic`, `bounce`, `steps` and functions are
sampled, a key a frame. `stagger`, `repeat`, `yoyo`, labels and positions are worked out when the
script runs; only keys are left.

## Rules a script runs under

- One file. Nothing can be imported; there is no file system, network, timer or clock (`Date`).
- `Math.random()` (also given as `random`) is seeded by `script.add`'s `seed`, so the same source,
  parameters and canvas always build the same clips.
- A loop may go round 2 000 000 times and calls nest 256 deep. A script makes at most 400 nodes,
  20 000 tweens and 60 000 keys, and its source is at most 256 KB.
- `console.log` is kept (100 lines) and comes back in the reply as `log`.

## Commands

| Command | |
| --- | --- |
| `script.check {source, params?, duration?, seed?}` | Builds nothing; answers with the nodes it would make and the keys each gets, or with what is wrong. |
| `script.add {source, params?, start?, duration?, name?, seed?}` | Adds the group; one undo step. |
| `script.list`, `script.get {group}` | The scripts in the project; one whole, with its source. |
| `script.setParams {group, params?, reset?, duration?, seed?}` | Other values, and the group built again in the same place. `null` is a parameter's default again. |
| `script.setSource {group, source, params?, name?}` | Other source, built again in the same place; values are kept where the new source still declares them. |
| `script.detach {group}` | Forgets the script; the clips stay, an ordinary group. |

Building a group again replaces its clips, so changes made to them by hand are lost; detach first
to keep them. A script whose clips are all deleted is forgotten. Copies of its clips are ordinary
clips.

## Presets

A script with the values it was given can be kept to use again. In the editor, **Save as Preset**
under a script's parameters keeps it in **My Presets** (the Stickers tab, under Promo), which every
project on that computer or browser shows; click one to add it at the playhead, or drag it onto the
timeline. Agents keep presets with the project:

| Command | |
| --- | --- |
| `preset.save {group}` or `preset.save {source, params?, name?, seed?, duration?}` | Keeps the script that built a group, with its values, or the script given. |
| `preset.list`, `preset.get {preset}` | The project's presets; one whole, with its source. `preset` is an id (`p12`) or a name. |
| `preset.apply {preset, start?, params?, duration?, seed?}` | Builds it as `script.add` would; `params` go over the saved values, `null` takes the default. |
| `preset.set {preset, name}`, `preset.remove {preset}` | Renames or forgets one; what it built stays. |

Only the values given are kept: a parameter left at its default follows the script, so a colour
the script takes from elsewhere is not frozen at the one of the day it was saved.

## Not yet

A script can only tween what the editor can key. What GSAP can say and a script cannot yet, with
what to write meanwhile (a script that uses one is refused, and told so):

| GSAP | State | Meanwhile |
| --- | --- | --- |
| `skewX`, `skewY` | nothing skews | |
| `width`, `height` | a shape's size is not keyed | `scaleX`, `scaleY` with a `transformOrigin` |
| `borderRadius` | a corner is not keyed | |
| `stroke`, a shadow's colour, a gradient's second colour, a title's box | only a fill's colour and a title's are keyed | cross-fade two nodes |
| `filter: blur()` and other filters | not keyed | the colour adjustments are; `motionBlur` |
| `z`, `rotationX/Y` with depth sorting | nodes are drawn in the order made | order them yourself |
| `SplitText` (characters, words, lines as targets) | a title is one node | `animate` with the editor's character, word and line animations; or one node a word |
| `clip-path` with a path of its own | a mask is a rectangle, a circle, a line or a band | `mask` |
| `motionPath` with control points, `start` / `end`, several paths | a path passes through its points, once | |
| group opacity | a parent passes on place, size and turn only | fade the children |
| `repeat: -1` | a piece has a length | repeat as often as it is long |

Nodes by the hundred (particles, charts) want a mode that draws frames rather than clips, which is
planned as `script/1`.
