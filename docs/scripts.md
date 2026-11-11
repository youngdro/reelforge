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

The whole of a longer one is in [examples/price-card.js](examples/price-card.js).

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
replies use). Later nodes lie over earlier ones.

| Call | Options besides those |
| --- | --- |
| `scene.rect(opts)`, `scene.ellipse(opts)`, `scene.polygon(opts)`, `scene.star(opts)`, `scene.tag(opts)`, `scene.arrow(opts)` | `width`, `height`, `radius` (corners), `points` and `inner` (polygon, star), `fill` (a colour, `{type: "linear", from, to, angle?}`, `{type: "radial", from, to}` or `null`), `stroke: {color, width}`, `shadow: {color, offset: [x, y], blur}`, `trim: [start, end]` |
| `scene.circle(opts)` | the same, with `radius` its radius |
| `scene.line(opts)` | `width` (its length), `stroke` |
| `scene.text(words, opts)` | `size` (pixels), `color`, `weight`, `align`, `wrap`, `outline: {color, width}`, `outer`, `shadow`, `background`, `radius`, `gradient`, `font` (a media id), `tracking`, `leading`, `italic`, `underline`, `template`, and `counter: {from, to, duration?, ease?, decimals?, group?, clock?}`, which writes a running number where the words say `{n}`. Widths of outlines and shadows are in ems, as `clip.set` takes them |
| `scene.image(mediaId, opts)`, `scene.video(mediaId, opts)` | `sourceIn` (seconds into a video) |
| `scene.sticker(id, opts)` | (see `sticker.library`) |

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
- **Properties**: `x`, `y`, `scale`, `rotation`, `opacity` on every node; the colour adjustments
  `exposure`, `contrast`, `saturation`, `temperature`, `tint`, `highlights`, `shadows`, `fade`,
  `vignette`; `drawStart` and `drawEnd` on a shape with a stroke (0 to 1 round its outline: from 0
  to 1, `drawEnd` draws it on); `count` on a title with a counter; `cropLeft`, `cropTop`,
  `cropRight`, `cropBottom` on a picture. A property a node has not is refused, with those it has.
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

## Not yet

A script can only tween what the editor can key. GSAP properties with nothing behind them yet:
`scaleX` / `scaleY` and skew, `transformOrigin`, 3D turns, motion along a path, colours and
gradients, a shape's size and corners, blur and other filters, splitting a title into characters or
words, and moving a group as one. Nodes by the hundred (particles, charts) want a mode that draws
frames rather than clips, which is planned as `script/1`.
