// The world a motion script runs in: `scene` makes nodes, `timeline` tweens them, and
// `__rf_run` answers with plain data. Lengths are canvas pixels, x and y from the centre of the
// canvas (y down), time is seconds from the script's start, angles are degrees clockwise.
"use strict";
(function () {
  const fail = (msg) => { throw new Error(msg); };
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const LIMIT = { nodes: 400, tweens: 20000, keys: 60000 };

  // ---- colours -------------------------------------------------------------------------
  const NAMED = {
    white: "#ffffff", black: "#000000", red: "#ff0000", green: "#008000", blue: "#0000ff",
    yellow: "#ffff00", orange: "#ffa500", purple: "#800080", pink: "#ffc0cb", gray: "#808080",
    grey: "#808080", cyan: "#00ffff", magenta: "#ff00ff", gold: "#ffd700", transparent: "#00000000",
  };
  function colour(v, where) {
    if (Array.isArray(v) && (v.length === 3 || v.length === 4) && v.every((x) => isNum(x) && x >= 0 && x <= 1)) {
      return v.length === 3 ? [v[0], v[1], v[2], 1] : v.slice();
    }
    if (typeof v === "string") {
      let s = v.trim().toLowerCase();
      if (NAMED[s]) s = NAMED[s];
      let m = /^#([0-9a-f]{3,8})$/.exec(s);
      if (m && [3, 4, 6, 8].includes(m[1].length)) {
        let h = m[1];
        if (h.length <= 4) h = h.split("").map((c) => c + c).join("");
        const n = (i) => parseInt(h.slice(i, i + 2), 16) / 255;
        return [n(0), n(2), n(4), h.length === 8 ? n(6) : 1];
      }
      m = /^rgba?\(([^)]+)\)$/.exec(s);
      if (m) {
        const p = m[1].split(/[\s,\/]+/).filter(Boolean).map(Number);
        if ((p.length === 3 || p.length === 4) && p.every(isNum)) {
          const c = (x) => Math.min(1, Math.max(0, x / 255));
          return [c(p[0]), c(p[1]), c(p[2]), p.length === 4 ? Math.min(1, Math.max(0, p[3])) : 1];
        }
      }
    }
    fail(`${where}: ${JSON.stringify(v)} is not a colour; write "#rrggbb", "#rrggbbaa", "rgb(r, g, b)", "rgba(r, g, b, a)", a name such as "white", or [r, g, b, a] with each from 0 to 1`);
  }
  function fill(v, where) {
    if (v === null) return null;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      if (v.type !== "linear" && v.type !== "radial") fail(`${where}: a gradient is {type: "linear" | "radial", from, to, angle?}`);
      const out = { type: v.type, from: colour(v.from, where + ".from"), to: colour(v.to, where + ".to") };
      if (v.type === "linear" && v.angle !== undefined) out.angle = v.angle;
      return out;
    }
    return colour(v, where);
  }
  const coloured = (o, where, extra) => {
    if (o === null || o === undefined) return o;
    if (typeof o !== "object" || Array.isArray(o)) fail(`${where} must be an object`);
    const out = Object.assign({}, o);
    if (out.color !== undefined) out.color = colour(out.color, where + ".color");
    return Object.assign(out, extra || {});
  };

  // ---- eases ---------------------------------------------------------------------------
  // An ease is {bezier: [x1, y1, x2, y2]} where a cubic Bézier says it (exactly for the
  // cubics, closely for the others), or {fn} to be sampled a key a frame.
  const B = (a, b, c, d) => ({ bezier: [a, b, c, d] });
  const TABLE = {
    power1: [B(1 / 3, 0, 2 / 3, 1 / 3), B(1 / 3, 2 / 3, 2 / 3, 1), B(0.45, 0, 0.55, 1)],
    power2: [B(1 / 3, 0, 2 / 3, 0), B(1 / 3, 1, 2 / 3, 1), B(0.65, 0, 0.35, 1)],
    power3: [B(0.5, 0, 0.75, 0), B(0.25, 1, 0.5, 1), B(0.76, 0, 0.24, 1)],
    power4: [B(0.64, 0, 0.78, 0), B(0.22, 1, 0.36, 1), B(0.83, 0, 0.17, 1)],
    sine: [B(0.12, 0, 0.39, 0), B(0.61, 1, 0.88, 1), B(0.37, 0, 0.63, 1)],
    expo: [B(0.7, 0, 0.84, 0), B(0.16, 1, 0.3, 1), B(0.87, 0, 0.13, 1)],
    circ: [B(0.55, 0, 1, 0.45), B(0, 0.55, 0.45, 1), B(0.85, 0, 0.15, 1)],
  };
  TABLE.quad = TABLE.power1; TABLE.cubic = TABLE.power2; TABLE.quart = TABLE.power3;
  TABLE.quint = TABLE.power4; TABLE.strong = TABLE.power4;
  const bounceOut = (t) => {
    const n = 7.5625, d = 2.75;
    if (t < 1 / d) return n * t * t;
    if (t < 2 / d) return n * (t -= 1.5 / d) * t + 0.75;
    if (t < 2.5 / d) return n * (t -= 2.25 / d) * t + 0.9375;
    return n * (t -= 2.625 / d) * t + 0.984375;
  };
  const dirs = (out) => [
    (t) => 1 - out(1 - t),
    out,
    (t) => (t < 0.5 ? (1 - out(1 - 2 * t)) / 2 : (1 + out(2 * t - 1)) / 2),
  ];
  const EASES = "none, linear, power1–4, quad, cubic, quart, quint, sine, expo, circ, back(overshoot), elastic(amplitude, period), bounce, each with .in, .out or .inOut; steps(n); hold; \"cubic-bezier(x1, y1, x2, y2)\"; [x1, y1, x2, y2]; or a function of 0..1";
  function ease(v) {
    if (v === undefined || v === null) return TABLE.power1[1];
    if (typeof v === "function") return { fn: v };
    if (Array.isArray(v)) {
      if (v.length === 4 && v.every(isNum) && v[0] >= 0 && v[0] <= 1 && v[2] >= 0 && v[2] <= 1) {
        return v[1] < -1 || v[1] > 2 || v[3] < -1 || v[3] > 2 ? { fn: bezierFn(v) } : { bezier: v.slice() };
      }
      fail(`ease ${JSON.stringify(v)}: a curve is [x1, y1, x2, y2] with x1 and x2 from 0 to 1`);
    }
    if (typeof v !== "string") fail(`ease must be one of: ${EASES}`);
    const s = v.replace(/\s+/g, "");
    if (s === "none" || s === "linear") return { linear: true };
    if (s === "hold") return { hold: true };
    let m = /^cubic-bezier\(([^)]*)\)$/.exec(s);
    if (m) return ease(m[1].split(",").map(Number));
    m = /^steps\((\d+)\)$/.exec(s);
    if (m && +m[1] > 0) { const n = +m[1]; return { fn: (t) => Math.min(1, Math.floor(t * n + 1e-9) / n), steps: true }; }
    m = /^([a-zA-Z0-9]+?)(?:\.(inOut|in|out))?(?:\(([^)]*)\))?$/.exec(s);
    if (!m) fail(`ease ${JSON.stringify(v)} is not known; use one of: ${EASES}`);
    const name = m[1].toLowerCase(), dir = m[2] === "in" ? 0 : m[2] === "inOut" ? 2 : 1;
    const args = m[3] ? m[3].split(",").map(Number) : [];
    if (args.some((a) => !isNum(a))) fail(`ease ${JSON.stringify(v)}: its numbers are not numbers`);
    if (TABLE[name]) return TABLE[name][dir];
    if (name === "back") {
      const k = args.length ? args[0] : 1.70158;
      // out and in are cubics, so the Bézier says them exactly
      if (dir === 1 && k <= 3) return B(1 / 3, 1 + k / 3, 2 / 3, 1);
      if (dir === 0 && k <= 3) return B(1 / 3, 0, 2 / 3, -k / 3);
      const out = (t) => 1 + (k + 1) * Math.pow(t - 1, 3) + k * Math.pow(t - 1, 2);
      if (dir === 2 && args.length === 0) return B(0.68, -0.6, 0.32, 1.6);
      return { fn: dirs(out)[dir] };
    }
    if (name === "elastic") {
      const amp = Math.max(1, args.length ? args[0] : 1), period = (args.length > 1 ? args[1] : 0.3) / (dir === 2 ? 1 / 1.5 : 1);
      const p3 = (period / (2 * Math.PI)) * Math.asin(1 / amp);
      const out = (t) => (t >= 1 ? 1 : amp * Math.pow(2, -10 * t) * Math.sin(((t - p3) * 2 * Math.PI) / period) + 1);
      return { fn: dirs(out)[dir] };
    }
    if (name === "bounce") return { fn: dirs(bounceOut)[dir] };
    fail(`ease ${JSON.stringify(v)} is not known; use one of: ${EASES}`);
  }
  // y of a cubic Bézier ease at time x, by bisection (x is monotonic for x1, x2 in 0..1)
  function bezierFn(b) {
    const at = (u, p1, p2) => 3 * (1 - u) * (1 - u) * u * p1 + 3 * (1 - u) * u * u * p2 + u * u * u;
    return (x) => {
      if (x <= 0) return 0;
      if (x >= 1) return 1;
      let lo = 0, hi = 1;
      for (let i = 0; i < 40; i++) { const mid = (lo + hi) / 2; if (at(mid, b[0], b[2]) < x) lo = mid; else hi = mid; }
      return at((lo + hi) / 2, b[1], b[3]);
    };
  }
  const easeFn = (e) => (e.fn ? e.fn : e.linear ? (t) => t : e.hold ? (t) => (t >= 1 ? 1 : 0) : bezierFn(e.bezier));
  // the same curve played backwards, as a yoyo's way back is
  const reversed = (e) => {
    if (e.linear) return e;
    if (e.bezier) { const b = e.bezier; return B(1 - b[2], 1 - b[3], 1 - b[0], 1 - b[1]); }
    const f = easeFn(e);
    return { fn: (t) => 1 - f(1 - t) };
  };

  // ---- nodes ---------------------------------------------------------------------------
  const PLACE = ["x", "y", "scale", "rotation", "opacity"];
  // width and height apart, turned in space about the horizontal and the vertical axis, leaning
  const SPACE = ["scaleX", "scaleY", "rotationX", "rotationY", "skewX", "skewY"];
  // a colour's parts, as the editor keys them
  const RGBA = ["colorR", "colorG", "colorB", "colorA"];
  const OUTLINE = ["strokeR", "strokeG", "strokeB", "strokeA"];
  // the parts of a colour a node's `name` tweens, by name
  const PARTS = { fill: RGBA, color: RGBA, stroke: OUTLINE };
  const tinted = (n, c, name) => { PARTS[name].forEach((part, i) => { n.base[part] = c[i]; }); n.props = n.props.concat([name]); };
  const MORE = ["perspective", "motionBlur", "transformOrigin", "origin", "parent", "mask", "animate"];
  // a mask's own numbers, as the editor keys them
  const MASK = { x: "maskX", y: "maskY", width: "maskWidth", height: "maskHeight", rotation: "maskRotation", feather: "maskFeather" };
  // GSAP's other names for what a node has
  const ALIAS = { rotate: "rotation", rotateZ: "rotation", rotationZ: "rotation", rotateX: "rotationX", rotateY: "rotationY",
    autoAlpha: "opacity", alpha: "opacity", borderRadius: "radius", borderColor: "stroke", borderWidth: "strokeWidth" };
  function named(o, what) {
    let out = o;
    for (const k of Object.keys(o)) {
      if (ALIAS[k] === undefined) continue;
      if (out === o) out = Object.assign({}, o);
      if (o[ALIAS[k]] !== undefined) fail(`${what}: \`${k}\` and \`${ALIAS[k]}\` are the same property; give one`);
      out[ALIAS[k]] = o[k];
      delete out[k];
    }
    return out;
  }
  // the point a node is placed, scaled and turned about, as shares of its own box from the top left
  function origin(v, label) {
    const bad = () => fail(`${label}: \`transformOrigin\` is "left top", "center bottom", "50% 100%" and the like, or [across, down] as shares of the node's own box (0.5, 0.5 is its middle)`);
    if (Array.isArray(v)) {
      if (v.length !== 2 || !v.every((x) => isNum(x) && x >= -4 && x <= 5)) bad();
      return v.slice();
    }
    if (typeof v !== "string") bad();
    const words = v.trim().toLowerCase().split(/\s+/);
    if (words.length < 1 || words.length > 2) bad();
    const out = [null, null];
    const rest = [];
    for (const w of words) {
      if (w === "left" || w === "right") { if (out[0] !== null) bad(); out[0] = w === "left" ? 0 : 1; }
      else if (w === "top" || w === "bottom") { if (out[1] !== null) bad(); out[1] = w === "top" ? 0 : 1; }
      else if (w === "center" || w === "centre") rest.push(0.5);
      else if (/^-?\d*\.?\d+%$/.test(w) && Math.abs(parseFloat(w)) <= 400) rest.push(parseFloat(w) / 100);
      else bad();
    }
    for (const r of rest) { const i = out[0] === null ? 0 : 1; if (out[i] !== null) bad(); out[i] = r; }
    return [out[0] === null ? 0.5 : out[0], out[1] === null ? 0.5 : out[1]];
  }
  const ADJUST = ["exposure", "contrast", "saturation", "temperature", "tint", "highlights", "shadows", "fade", "vignette"];
  const SHAPES = ["rectangle", "ellipse", "line", "polygon", "star", "tag", "arrow"];
  const SHAPE_OPTS = ["width", "height", "radius", "points", "inner", "fill", "stroke", "shadow", "trim"];
  const TEXT_OPTS = ["size", "color", "weight", "align", "wrap", "outline", "background", "radius", "gradient", "outer",
    "shadow", "lineBoxes", "font", "tracking", "leading", "italic", "underline", "template", "counter"];
  const BASE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1, scaleX: 1, scaleY: 1, rotationX: 0, rotationY: 0, skewX: 0, skewY: 0, drawStart: 0, drawEnd: 1,
    exposure: 0, contrast: 0, saturation: 0, temperature: 0, tint: 0, highlights: 0, shadows: 0, fade: 0, vignette: 0,
    cropLeft: 0, cropTop: 0, cropRight: 0, cropBottom: 0,
    maskX: 0.5, maskY: 0.5, maskWidth: 1, maskHeight: 1, maskRotation: 0, maskFeather: 0 };

  let state = null;

  function Node(kind, name) { this.kind = kind; this.name = name; this.base = {}; this.add = {}; this.props = []; this.more = {}; this.origin = null; this.parent = null; this.path = null; this.after = []; }

  function node(kind, opts, allowed, label) {
    if (opts === undefined) opts = {};
    if (opts === null || typeof opts !== "object" || Array.isArray(opts)) fail(`${label}: its options must be an object`);
    if (state.nodes.length >= LIMIT.nodes) fail(`a script makes at most ${LIMIT.nodes} nodes; draw more than that in frame mode`);
    opts = named(opts, label);
    const ok = PLACE.concat(SPACE, MORE, ["name"], allowed);
    for (const k of Object.keys(opts)) {
      if (!ok.includes(k)) fail(`${label}: no option \`${k}\`; it takes ${ok.join(", ")}`);
    }
    const n = new Node(kind, typeof opts.name === "string" && opts.name ? opts.name : `${label.replace("scene.", "")}${state.nodes.length + 1}`);
    for (const k of PLACE.concat(SPACE)) {
      const v = opts[k] === undefined ? BASE[k] : opts[k];
      if (!isNum(v)) fail(`${label}: \`${k}\` must be a number`);
      n.base[k] = v;
    }
    if (n.base.opacity < 0 || n.base.opacity > 1) fail(`${label}: \`opacity\` is from 0 to 1`);
    if (!(n.base.scale >= 0)) fail(`${label}: \`scale\` must not be negative`);
    if (Math.abs(n.base.skewX) > 89 || Math.abs(n.base.skewY) > 89) fail(`${label}: \`skewX\` and \`skewY\` are degrees within ±89`);
    if (opts.perspective !== undefined) {
      // the eye's distance, in pixels as CSS says it; the editor counts it in canvas heights
      if (!isNum(opts.perspective) || opts.perspective <= 0) fail(`${label}: \`perspective\` is the eye's distance in pixels, above 0`);
      n.more.perspective = Math.min(50, Math.max(0.2, opts.perspective / state.canvas[1]));
    }
    if (opts.motionBlur !== undefined) {
      const b = opts.motionBlur;
      if (typeof b !== "boolean" && !(isNum(b) && b >= 0 && b <= 2)) fail(`${label}: \`motionBlur\` is true, false, or the share of a frame the shutter is open (up to 2)`);
      n.more.motionBlur = b;
    }
    if (opts.transformOrigin !== undefined && opts.origin !== undefined) fail(`${label}: \`origin\` and \`transformOrigin\` are the same; give one`);
    const o = opts.transformOrigin !== undefined ? opts.transformOrigin : opts.origin;
    if (o !== undefined) n.origin = origin(o, label);
    if (opts.parent !== undefined && opts.parent !== null) {
      if (!(opts.parent instanceof Node)) fail(`${label}: \`parent\` is a node that scene made`);
      let depth = 1;
      for (let up = opts.parent; up.parent; up = up.parent) depth++;
      if (depth > 8) fail(`${label}: nodes carry one another at most 8 deep`);
      n.parent = opts.parent;
    }
    n.props = PLACE.concat(SPACE, ADJUST);
    if (opts.mask !== undefined && opts.mask !== null) {
      // what of the node shows: a shape over it, in the node's own measures (x and y as shares of
      // its box, 0.5 the middle; width and height in its shorter side)
      const m = typeof opts.mask === "string" ? { shape: opts.mask } : opts.mask;
      const ok = ["shape", "invert"].concat(Object.keys(MASK));
      if (!m || typeof m !== "object" || typeof m.shape !== "string") fail(`${label}: \`mask\` is a shape's name, or {shape, x?, y?, width?, height?, rotation?, feather?, invert?}`);
      for (const k of Object.keys(m)) if (!ok.includes(k)) fail(`${label}: mask has no \`${k}\`; it takes ${ok.join(", ")}`);
      const set = { shape: m.shape };
      if (m.invert !== undefined) set.invert = !!m.invert;
      for (const k of Object.keys(MASK)) {
        if (m[k] === undefined) continue;
        if (!isNum(m[k])) fail(`${label}: mask \`${k}\` must be a number`);
        set[k] = m[k];
        n.base[MASK[k]] = m[k];
      }
      n.after.push({ command: "clip.setMask", params: set });
      n.props = n.props.concat(Object.values(MASK));
    }
    if (opts.animate !== undefined && opts.animate !== null) {
      // the editor's own animations: in, out, and one that goes round (a title's, character by character)
      const a = opts.animate;
      if (typeof a !== "object" || Array.isArray(a)) fail(`${label}: \`animate\` is {in?, out?, loop?}, each a kind's name or {kind, duration?}`);
      for (const slot of Object.keys(a)) {
        if (!["in", "out", "loop"].includes(slot)) fail(`${label}: animate has no \`${slot}\`; it takes in, out, loop`);
        const v = typeof a[slot] === "string" ? { kind: a[slot] } : a[slot];
        if (!v || typeof v !== "object" || typeof v.kind !== "string") fail(`${label}: animate.${slot} is a kind's name or {kind, duration?}`);
        const set = { slot, kind: v.kind };
        if (v.duration !== undefined) {
          if (!isNum(v.duration) || v.duration <= 0) fail(`${label}: animate.${slot}.duration is seconds`);
          set.duration = { seconds: v.duration };
        }
        n.after.push({ command: "clip.setAnimation", params: set });
      }
    }
    state.nodes.push(n);
    return n;
  }

  function shape(kind) {
    return (opts) => {
      const label = kind === "rectangle" ? "scene.rect" : `scene.${kind}`;
      const n = node("shape", opts, SHAPE_OPTS, label);
      const o = opts || {};
      const s = { kind };
      const w = o.width === undefined ? 400 : o.width;
      const h = kind === "line" ? 0 : o.height === undefined ? (o.width === undefined ? 400 : o.width) : o.height;
      if (!isNum(w) || !isNum(h) || w < 0 || h < 0) fail(`${label}: \`width\` and \`height\` are pixels`);
      s.size = [w, h];
      for (const k of ["radius", "points", "inner", "trim"]) if (o[k] !== undefined) s[k] = o[k];
      if (o.fill !== undefined) s.fill = fill(o.fill, `${label} fill`);
      if (o.stroke !== undefined) s.stroke = coloured(o.stroke, `${label} stroke`);
      else if (kind === "line") s.stroke = { color: [1, 1, 1, 1], width: 6 };
      if (o.shadow !== undefined) {
        s.shadow = coloured(o.shadow, `${label} shadow`);
        if (s.shadow && s.shadow.color === undefined) s.shadow.color = [0, 0, 0, 0.35];
        if (s.shadow && s.shadow.offset === undefined) s.shadow.offset = [0, 0];
      }
      if (kind === "line" && o.fill === undefined) s.fill = null;
      n.add = { shape: s };
      // its box and corner move; and its outline, where it has one
      Object.assign(n.base, { width: w, height: h, radius: isNum(s.radius) ? s.radius : 0 });
      n.props = n.props.concat(kind === "line" ? ["width"] : ["width", "height"]);
      if (["rectangle", "tag", "arrow"].includes(kind)) n.props = n.props.concat(["radius"]);
      if (s.stroke) {
        n.props = n.props.concat(["drawStart", "drawEnd", "strokeWidth"]);
        n.base.strokeWidth = isNum(s.stroke.width) ? s.stroke.width : 0;
        tinted(n, s.stroke.color || [1, 1, 1, 1], "stroke");
      }
      // its fill's colour moves (a gradient's, the colour it starts from)
      if (kind !== "line" && s.fill) tinted(n, Array.isArray(s.fill) ? s.fill : s.fill.from, "fill");
      return n;
    };
  }

  const scene = {
    rect: shape("rectangle"),
    ellipse: shape("ellipse"),
    circle: (opts) => {
      const o = Object.assign({}, opts || {});
      if (o.radius !== undefined) { o.width = o.height = 2 * o.radius; delete o.radius; }
      return shape("ellipse")(o);
    },
    // nothing to see: a node that carries others (`parent: group`), so they move, grow and turn as one
    group(opts) {
      const o = opts || {};
      if (o && typeof o === "object" && o.opacity !== undefined) fail("scene.group: a group has no opacity (it passes on where it is, its size and its turn, nothing else); fade the nodes it carries");
      const n = node("shape", opts, ["width", "height"], "scene.group");
      const w = o.width === undefined ? 100 : o.width, h = o.height === undefined ? 100 : o.height;
      if (!isNum(w) || !isNum(h) || w <= 0 || h <= 0) fail("scene.group: `width` and `height` are pixels (they matter only to `transformOrigin`)");
      n.add = { shape: { kind: "rectangle", size: [w, h], fill: null } };
      n.base.opacity = 0;
      n.props = n.props.filter((k) => k !== "opacity" && !ADJUST.includes(k));
      n.group = true;
      return n;
    },
    line: shape("line"),
    polygon: shape("polygon"),
    star: shape("star"),
    tag: shape("tag"),
    arrow: shape("arrow"),
    text(text, opts) {
      if (typeof text !== "string" && typeof text !== "number") fail("scene.text: its first argument is the words");
      const n = node("text", opts, TEXT_OPTS, "scene.text");
      const o = opts || {};
      const add = { text: String(text) };
      for (const k of TEXT_OPTS) if (o[k] !== undefined && k !== "counter") add[k] = o[k];
      if (add.color !== undefined) add.color = colour(add.color, "scene.text color");
      for (const k of ["background", "gradient"]) if (add[k] !== undefined && add[k] !== null) add[k] = colour(add[k], `scene.text ${k}`);
      for (const k of ["outline", "outer", "shadow"]) if (add[k] !== undefined) add[k] = coloured(add[k], `scene.text ${k}`);
      // plain words unless the script says otherwise: no outline, no shadow
      for (const k of ["outline", "shadow"]) if (add[k] === undefined && add.template === undefined) add[k] = null;
      n.add = add;
      if (add.color !== undefined || add.template === undefined) tinted(n, add.color || [1, 1, 1, 1], "color");
      if (o.counter !== undefined) {
        const c = o.counter;
        if (c === null || typeof c !== "object") fail("scene.text: `counter` is {from, to, duration?, ease?, decimals?, group?, clock?}");
        const ok = ["from", "to", "duration", "ease", "decimals", "group", "clock"];
        for (const k of Object.keys(c)) if (!ok.includes(k)) fail(`scene.text counter: no option \`${k}\`; it takes ${ok.join(", ")}`);
        const out = {};
        for (const k of ["from", "to", "decimals", "group", "clock"]) if (c[k] !== undefined) out[k] = c[k];
        if (c.duration !== undefined) out.over = { seconds: c.duration };
        if (c.ease !== undefined) {
          const e = ease(c.ease);
          if (!e.bezier && !e.linear) fail("scene.text counter: `ease` must be one a Bézier can say (not elastic, bounce, steps or a function); tween `count` on a timeline for those");
          out.ease = e.linear ? "linear" : e.bezier;
        }
        n.counter = out;
        n.props = n.props.concat(["count"]);
        n.base.count = c.from === undefined ? 0 : c.from;
      }
      return n;
    },
    image(media, opts) { return picture("scene.image", media, opts); },
    video(media, opts) { return picture("scene.video", media, opts); },
    sticker(id, opts) {
      if (typeof id !== "string") fail("scene.sticker: its first argument is a sticker's id (sticker.library)");
      const n = node("sticker", opts, [], "scene.sticker");
      n.add = { sticker: id };
      return n;
    },
    setDuration(seconds) {
      if (!isNum(seconds) || seconds <= 0) fail("scene.setDuration: seconds, above 0");
      state.duration = seconds;
    },
  };
  function picture(label, media, opts) {
    if (typeof media !== "string") fail(`${label}: its first argument is the id of a file of the project (media.import)`);
    const n = node("media", opts, ["sourceIn"], label);
    n.add = { media };
    if (opts && opts.sourceIn !== undefined) n.add.sourceIn = { seconds: opts.sourceIn };
    n.props = n.props.concat(["cropLeft", "cropTop", "cropRight", "cropBottom"]);
    return n;
  }

  // ---- timelines -----------------------------------------------------------------------
  const SPECIAL = ["duration", "ease", "delay", "stagger", "repeat", "yoyo", "repeatDelay", "motionPath"];
  // GSAP properties with nothing behind them yet, and what to do meanwhile
  const NOT_YET = {
    skew: "write `skewX` and `skewY`",
    width: "only a shape has a width to tween; tween scaleX", height: "only a shape (not a line) has a height to tween; tween scaleY",
    fill: "this node has no fill to tween (give a shape its `fill` when making it; a title's colour is `color`)",
    color: "only a title has a `color` to tween (a shape's is `fill`)",
    backgroundColor: "tween `fill` on a shape, or `color` on a title", stroke: "only a shape made with a `stroke` has an outline colour to tween",
    strokeWidth: "only a shape made with a `stroke` has an outline to widen",
    radius: "only a rectangle, a tag or an arrow has corners to round",
    blur: "a blur is not tweened yet", filter: "filters are not tweened yet; exposure, contrast, saturation and the other adjustments are",
    z: "there is no depth yet; rotationX and rotationY turn a node in space", xPercent: "write pixels", yPercent: "write pixels",
    transformOrigin: "it does not move; give it to the node when making it", fontSize: "tween scale", text: "tween a counter's `count`, or make two titles",
  };

  // the points of a path, as a tween's `motionPath` gives them
  function pathOf(v, n, what) {
    const bad = () => fail(`${what}: \`motionPath\` is a list of points ([{x, y}, …] or [[x, y], …], pixels from the canvas centre), or {path, autoRotate?, closed?, fromCurrent?}`);
    const o = Array.isArray(v) ? { path: v } : v;
    if (!o || typeof o !== "object" || !Array.isArray(o.path)) bad();
    for (const k of Object.keys(o)) if (!["path", "autoRotate", "closed", "fromCurrent"].includes(k)) fail(`${what}: motionPath has no \`${k}\`; it takes path, autoRotate, closed, fromCurrent (a path passes through its points; there are no control points)`);
    const points = o.path.map((p) => {
      const q = Array.isArray(p) ? p : p && typeof p === "object" ? [p.x, p.y] : bad();
      if (q.length !== 2 || !q.every(isNum)) bad();
      return q;
    });
    // as GSAP does: from where the node is, unless told otherwise
    if (o.fromCurrent !== false) points.unshift([n.base.x, n.base.y]);
    if (points.length < 2 || points.length > 64) fail(`${what}: a path has 2 to 64 points`);
    return { points, orient: !!o.autoRotate, closed: !!o.closed };
  }

  function targets(t, what) {
    const list = Array.isArray(t) ? t.flat(Infinity) : [t];
    if (list.some((n) => !(n instanceof Node))) fail(`${what}: its first argument is a node, or a list of nodes, that scene made`);
    return list;
  }

  function Timeline(vars) {
    vars = vars || {};
    this.defaults = Object.assign({ duration: 0.5, ease: "power1.out" }, vars.defaults || {});
    this.offset = isNum(vars.delay) ? vars.delay : 0;
    this.end = 0; // where the next tween goes, in this timeline's own time
    this.prev = [0, 0]; // the start and end of the tween added last
    this.labels = {};
  }
  Timeline.prototype.at = function (pos) {
    if (pos === undefined || pos === null) return this.end;
    if (isNum(pos)) return pos;
    if (typeof pos !== "string") fail(`a position is seconds, "+=0.5", "-=0.5", "<", ">", "<0.2", or a label`);
    const s = pos.replace(/\s+/g, "");
    let m = /^([+-])=(\d*\.?\d+)$/.exec(s);
    if (m) return this.end + (m[1] === "+" ? 1 : -1) * +m[2];
    m = /^([<>])(?:([+-])=?)?(\d*\.?\d+)?$/.exec(s);
    if (m) return (m[1] === "<" ? this.prev[0] : this.prev[1]) + (m[2] === "-" ? -1 : 1) * (m[3] ? +m[3] : 0);
    m = /^([^+\-<>=]+)(?:([+-])=(\d*\.?\d+))?$/.exec(s);
    if (m && this.labels[m[1]] !== undefined) return this.labels[m[1]] + (m[2] ? (m[2] === "+" ? 1 : -1) * +m[3] : 0);
    fail(`position ${JSON.stringify(pos)}: seconds, "+=0.5", "-=0.5", "<", ">", "<0.2", or a label added with addLabel${m ? ` (there is no label ${JSON.stringify(m[1])})` : ""}`);
  };
  Timeline.prototype.addLabel = function (name, pos) {
    if (typeof name !== "string" || !name) fail("addLabel: a name");
    this.labels[name] = this.at(pos);
    return this;
  };
  Timeline.prototype.duration = function () { return this.end; };
  Timeline.prototype.tween = function (what, t, from, to, pos) {
    const list = targets(t, what);
    if (from) from = named(from, what);
    if (to) to = named(to, what);
    const vars = Object.assign({}, this.defaults, to || from);
    const dur = what === "set" ? 0 : vars.duration;
    if (!isNum(dur) || dur < 0) fail(`${what}: \`duration\` is seconds`);
    const e = ease(vars.ease);
    const repeat = vars.repeat === undefined ? 0 : vars.repeat;
    if (!Number.isInteger(repeat) || repeat < 0) fail(`${what}: \`repeat\` is a whole number of extra times (a tween that never ends has no length; repeat it as often as the clip is long)`);
    const gap = vars.repeatDelay === undefined ? 0 : vars.repeatDelay;
    const delay = vars.delay === undefined ? 0 : vars.delay;
    if (!isNum(gap) || gap < 0 || !isNum(delay)) fail(`${what}: \`delay\` and \`repeatDelay\` are seconds`);
    let each = 0, order = "start";
    if (isNum(vars.stagger)) each = vars.stagger;
    else if (vars.stagger && typeof vars.stagger === "object") {
      each = isNum(vars.stagger.each) ? vars.stagger.each
        : isNum(vars.stagger.amount) && list.length > 1 ? vars.stagger.amount / (list.length - 1) : 0;
      order = vars.stagger.from === undefined ? "start" : vars.stagger.from;
      if (!["start", "end", "center", "edges"].includes(order)) fail(`${what}: \`stagger.from\` is "start", "end", "center" or "edges"`);
    } else if (vars.stagger !== undefined) fail(`${what}: \`stagger\` is seconds, or {each | amount, from?}`);
    const start = this.at(pos) + delay;
    if (start < 0) fail(`${what}: it would start ${(-start).toFixed(3)} s before the script does`);
    const mid = (list.length - 1) / 2;
    const rank = (i) => order === "end" ? list.length - 1 - i : order === "center" ? Math.abs(i - mid) - (mid % 1)
      : order === "edges" ? Math.floor(mid) - Math.floor(Math.abs(i - mid)) : i;
    const one = dur * (repeat + 1) + gap * repeat;
    let last = start;
    list.forEach((n, i) => {
      const t0 = start + rank(i) * each;
      const names = new Set();
      for (const src of [from, to]) for (const k of Object.keys(src || {})) if (!SPECIAL.includes(k)) names.add(k);
      if (vars.motionPath !== undefined) {
        if (what !== "to") fail(`${what}: a path is travelled with \`to\``);
        if (n.path) fail(`${what}: ${n.name} already travels a path; a node has one (give it every point)`);
        if (repeat || vars.yoyo) fail(`${what}: a path is travelled once; \`closed: true\` brings it back to where it set off`);
        if (!(dur > 0)) fail(`${what}: a path takes time; give a \`duration\``);
        if (e.fn) fail(`${what}: a path's ease must be one a curve can say (not elastic, bounce, steps or a function)`);
        if (names.has("x") || names.has("y")) fail(`${what}: the path says where ${n.name} is; leave \`x\` and \`y\` out`);
        n.path = Object.assign(pathOf(vars.motionPath, n, what), { start: this.offset + t0, duration: dur,
          ease: e.linear ? "linear" : e.hold ? "hold" : e.bezier, what });
      } else if (!names.size) fail(`${what}: it changes nothing; name a property, such as opacity or x`);
      for (const k of names) {
        if (NOT_YET[k] !== undefined && !n.props.includes(k)) fail(`${what}: \`${k}\` cannot be tweened: ${NOT_YET[k]}`);
        if (PARTS[k] && n.props.includes(k)) {
          // a colour is four numbers, each tweened
          const a = from && from[k] !== undefined ? colour(from[k], `${what} ${k}`) : null;
          const b = to && to[k] !== undefined ? colour(to[k], `${what} ${k}`) : null;
          if (state.tweens.length + 4 > LIMIT.tweens) fail(`a script makes at most ${LIMIT.tweens} tweens`);
          PARTS[k].forEach((part, i) => state.tweens.push({ node: n, prop: part, t0: this.offset + t0, dur, ease: e, repeat, gap,
            yoyo: !!vars.yoyo, from: a ? a[i] : undefined, to: b ? b[i] : undefined, what, order: state.tweens.length }));
          continue;
        }
        if (!n.props.includes(k)) fail(`${what}: ${n.name} has no \`${k}\` to tween; it has ${n.props.join(", ")}`);
        if (state.tweens.length >= LIMIT.tweens) fail(`a script makes at most ${LIMIT.tweens} tweens`);
        state.tweens.push({ node: n, prop: k, t0: this.offset + t0, dur, ease: e, repeat, gap, yoyo: !!vars.yoyo,
          from: from ? from[k] : undefined, to: to ? to[k] : undefined, what, order: state.tweens.length });
      }
      last = Math.max(last, t0 + one);
    });
    this.prev = [start, last];
    this.end = Math.max(this.end, last);
    state.end = Math.max(state.end, this.offset + this.end);
    return this;
  };
  Timeline.prototype.to = function (t, vars, pos) { return this.tween("to", t, null, need(vars, "to"), pos); };
  Timeline.prototype.from = function (t, vars, pos) { return this.tween("from", t, need(vars, "from"), null, pos); };
  Timeline.prototype.fromTo = function (t, a, b, pos) { return this.tween("fromTo", t, need(a, "fromTo"), need(b, "fromTo"), pos); };
  Timeline.prototype.set = function (t, vars, pos) { return this.tween("set", t, null, need(vars, "set"), pos); };
  const need = (v, what) => (v && typeof v === "object" && !Array.isArray(v) ? v : fail(`${what}: the properties to tween must be an object`));

  // ---- from tweens to keys -------------------------------------------------------------
  function value(v, current, what) {
    if (isNum(v)) return v;
    if (typeof v === "string") {
      const m = /^([+-])=(-?\d*\.?\d+(?:e-?\d+)?)$/.exec(v.replace(/\s+/g, ""));
      if (m) return current + (m[1] === "+" ? 1 : -1) * +m[2];
    }
    if (typeof v === "function") fail(`${what}: a value is a number or "+=n" / "-=n"; work a value out per node in the script itself, with a loop`);
    fail(`${what}: ${JSON.stringify(v)} is not a number or "+=n" / "-=n"`);
  }

  function keysOf(fps) {
    const snap = (t) => Math.round(t * fps) / fps;
    // segments by node and property, in the order their tweens start
    const tracks = new Map();
    const sorted = state.tweens.slice().sort((a, b) => a.t0 - b.t0 || a.order - b.order);
    for (const tw of sorted) {
      const id = state.nodes.indexOf(tw.node) + ":" + tw.prop;
      if (!tracks.has(id)) tracks.set(id, { node: tw.node, prop: tw.prop, segs: [] });
      const tr = tracks.get(id);
      const base = tw.node.base[tw.prop] === undefined ? BASE[tw.prop] : tw.node.base[tw.prop];
      // where the property has got to when this tween starts
      let cur = base;
      for (const s of tr.segs) if (s.t0 <= tw.t0 + 1e-9) cur = tw.t0 >= s.t1 ? s.v1 : s.v0 + (s.v1 - s.v0) * easeFn(s.ease)((tw.t0 - s.t0) / (s.t1 - s.t0));
      const label = `${tw.what} ${tw.node.name}.${tw.prop}`;
      const v0 = tw.from === undefined ? cur : value(tw.from, cur, label);
      const v1 = tw.to === undefined ? cur : value(tw.to, cur, label);
      for (let i = 0; i <= tw.repeat; i++) {
        const back = tw.yoyo && i % 2 === 1;
        const t0 = tw.t0 + i * (tw.dur + tw.gap);
        tr.segs.push({ t0, t1: t0 + tw.dur, v0: back ? v1 : v0, v1: back ? v0 : v1, ease: back ? reversed(tw.ease) : tw.ease });
      }
    }
    const out = [];
    let count = 0;
    for (const tr of tracks.values()) {
      if (tr.node.path && (tr.prop === "x" || tr.prop === "y")) fail(`${tr.node.path.what}: ${tr.node.name} travels a path, which says where it is; do not tween its \`${tr.prop}\` as well`);
      const keys = new Map(); // frame number → {v, ease}
      const put = (t, v, e) => keys.set(Math.round(t * fps), { v, ease: e });
      const segs = tr.segs.slice().sort((a, b) => a.t0 - b.t0);
      segs.forEach((s, i) => {
        const a = snap(s.t0), b = snap(s.t1);
        const next = segs[i + 1];
        // a later tween that starts before this one ends takes over from there
        const cut = next && snap(next.t0) < b ? snap(next.t0) : b;
        const f = easeFn(s.ease);
        const at = (t) => (b <= a ? s.v1 : s.v0 + (s.v1 - s.v0) * f((t - a) / (b - a)));
        const prev = keys.get(Math.round(a * fps));
        if (prev && Math.abs(prev.v - s.v0) > 1e-6 && a > 0) {
          // the value jumps here: what came before holds from a frame earlier
          const before = Math.round(a * fps) - 1;
          if (!keys.has(before)) keys.set(before, { v: prev.v, ease: "hold" });
          else keys.get(before).ease = "hold";
        }
        if (b <= a) {
          // a `set` some way in: the node's own value holds until then
          const own = tr.node.base[tr.prop] === undefined ? BASE[tr.prop] : tr.node.base[tr.prop];
          if (!keys.size && a > 0 && Math.abs(own - s.v1) > 1e-6) keys.set(0, { v: own, ease: "hold" });
          put(a, s.v1, "linear");
          return;
        }
        if (s.ease.fn || cut < b) {
          const n = Math.round((cut - a) * fps);
          for (let k = 0; k <= n; k++) {
            const t = (Math.round(a * fps) + k) / fps;
            put(t, at(t), s.ease.steps || s.ease.hold ? "hold" : "linear");
          }
        } else {
          put(a, s.v0, s.ease.linear ? "linear" : s.ease.hold ? "hold" : s.ease.bezier);
          put(b, s.v1, "linear");
        }
      });
      const frames = Array.from(keys.keys()).sort((x, y) => x - y);
      // a value that starts later than the script and differs from the node's own: held till then
      const list = frames.map((fr) => ({ t: fr / fps, v: keys.get(fr).v, ease: keys.get(fr).ease }));
      // between two tweens the value stays where the first left it
      for (let i = 0; i + 1 < list.length; i++) {
        const gap = segs.every((s) => !(snap(s.t0) <= list[i].t + 1e-9 && snap(s.t1) >= list[i + 1].t - 1e-9));
        if (gap && Math.abs(list[i].v - list[i + 1].v) > 1e-6) list[i].ease = "hold";
      }
      count += list.length;
      if (count > LIMIT.keys) fail(`a script makes at most ${LIMIT.keys} keyframes; shorter tweens, or fewer sampled eases (elastic, bounce, steps, functions)`);
      out.push({ node: state.nodes.indexOf(tr.node), prop: tr.prop, keys: list });
    }
    return out;
  }

  // ---- running a script ----------------------------------------------------------------
  function resolve(declared, given) {
    const values = {};
    const specs = {};
    if (declared !== undefined && (declared === null || typeof declared !== "object" || Array.isArray(declared))) fail("`params` must be an object: {name: {type, default, label?, min?, max?, options?}}");
    const TYPES = ["number", "text", "color", "boolean", "list", "select"];
    for (const [name, spec] of Object.entries(declared || {})) {
      if (spec === null || typeof spec !== "object" || !TYPES.includes(spec.type)) fail(`params.${name}: {type: ${TYPES.map((t) => JSON.stringify(t)).join(" | ")}, default, label?, min?, max?, options?}`);
      if (spec.default === undefined) fail(`params.${name}: give a \`default\``);
      specs[name] = { type: spec.type, default: spec.default };
      for (const k of ["label", "min", "max", "step", "options"]) if (spec[k] !== undefined) specs[name][k] = spec[k];
      values[name] = spec.default;
    }
    for (const [name, v] of Object.entries(given || {})) {
      const spec = specs[name];
      if (!spec) fail(`the script has no parameter \`${name}\`; it has ${Object.keys(specs).join(", ") || "none"}`);
      const bad = (what) => fail(`parameter \`${name}\` must be ${what}`);
      if (spec.type === "number") {
        if (!isNum(v)) bad("a number");
        if ((isNum(spec.min) && v < spec.min) || (isNum(spec.max) && v > spec.max)) bad(`from ${spec.min ?? "-∞"} to ${spec.max ?? "∞"}`);
      } else if (spec.type === "text" && typeof v !== "string") bad("a string");
      else if (spec.type === "boolean" && typeof v !== "boolean") bad("true or false");
      else if (spec.type === "color") colour(v, `parameter \`${name}\``);
      else if (spec.type === "list" && !Array.isArray(v)) bad("a list");
      else if (spec.type === "select" && Array.isArray(spec.options) && !spec.options.includes(v)) bad(`one of ${spec.options.map((o) => JSON.stringify(o)).join(", ")}`);
      values[name] = v;
    }
    return { specs, values };
  }

  globalThis.__rf_params = function (declared) { return JSON.stringify(resolve(declared, {}).specs); };

  globalThis.__rf_run = function (build, declared, input) {
    const { canvas, fps, seed, params, duration } = JSON.parse(input);
    state = { nodes: [], tweens: [], end: 0, duration: null, log: [], canvas };
    // the same numbers every run: what a script makes depends on its source and parameters alone
    let a = seed >>> 0;
    Math.random = () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const say = (...args) => { if (state.log.length < 100) state.log.push(args.map((x) => (typeof x === "string" ? x : JSON.stringify(x))).join(" ").slice(0, 500)); };
    globalThis.console = { log: say, info: say, warn: say, error: say, debug: say };
    const { specs, values } = resolve(declared, params);
    if (typeof build !== "function") fail("the script must have `export function build({ params, canvas, scene, timeline })`");
    build({ params: values, canvas: { width: canvas[0], height: canvas[1], fps }, scene, timeline: (vars) => new Timeline(vars),
      random: Math.random });
    if (!state.nodes.length) fail("the script made nothing: call scene.rect, scene.text, … in build");
    const length = duration ?? state.duration ?? Math.max(state.end, 1 / fps);
    const keys = keysOf(fps);
    const nodes = state.nodes.map((n) => {
      const add = Object.assign({}, n.add, { x: n.base.x / canvas[0], y: n.base.y / canvas[1], scale: n.base.scale, rotation: n.base.rotation, opacity: n.base.opacity }, n.more);
      for (const k of SPACE) if (n.base[k] !== BASE[k]) add[k] = n.base[k];
      const path = n.path && { points: n.path.points.map((q) => [q[0] / canvas[0], q[1] / canvas[1]]), closed: n.path.closed,
        orient: n.path.orient, start: n.path.start, duration: n.path.duration, ease: n.path.ease };
      return { name: n.name, kind: n.group ? "group" : n.kind, add, counter: n.counter || null, keys: {},
        origin: n.origin, parent: n.parent ? state.nodes.indexOf(n.parent) : null, path: path || null, after: n.after };
    });
    for (const k of keys) {
      const per = k.prop === "x" ? canvas[0] : k.prop === "y" ? canvas[1] : 1;
      nodes[k.node].keys[k.prop] = k.keys.filter((key) => key.t <= length + 1e-9).map((key) => ({ t: key.t, v: key.v / per, ease: key.ease }));
    }
    const out = { duration: length, animated: state.end, nodes, params: specs, values, log: state.log };
    state = null;
    return JSON.stringify(out);
  };
  // nothing of the machine it runs on
  const noClock = function () { fail("a script has no clock (Date): what it builds depends on its source and parameters alone"); };
  noClock.now = noClock; noClock.parse = noClock; noClock.UTC = noClock;
  globalThis.Date = noClock;
})();
