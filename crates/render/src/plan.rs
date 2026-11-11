//! What is on screen at an instant. Pure: no pixels, no IO.

use reelforge_renderspec::{
    Animated, Between, Blend, Fit, LayerContent, RenderSpec, ShapeContent, ShapeMask, TextContent, Trim, effects,
};
use reelforge_time::Tick;

use crate::image::premultiply_srgb;

/// Everything a compositor needs to draw one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FramePlan {
    /// The output instant.
    pub t: Tick,
    /// Target width in pixels (output width times the scale).
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Background, premultiplied linear.
    pub background: [f32; 4],
    /// Visible layers, bottom first.
    pub layers: Vec<PlannedLayer>,
}

/// One visible layer at the frame's instant, its animated values already evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedLayer {
    /// The layer id.
    pub id: String,
    /// What to draw.
    pub source: PlannedSource,
    /// Fit of the content into the frame.
    pub fit: Fit,
    /// Centre of the layer in target pixels.
    pub center: [f32; 2],
    /// Scale on top of the fit.
    pub scale: f32,
    /// Clockwise rotation, radians.
    pub rotation: f32,
    /// Opacity 0 to 1.
    pub opacity: f32,
    /// Blend mode.
    pub blend: Blend,
    /// Effects with their parameters at this instant: `(id, [(name, value)])`.
    pub fx: Vec<(String, Vec<(String, f32)>)>,
    /// The `lut` asset of the layer's `color.lut@1`, if it has one.
    pub lut: Option<String>,
    /// Fractions cut off the picture: left, top, right, bottom.
    pub crop: [f32; 4],
    /// The picture mirrored: left to right, top to bottom.
    pub flip: [bool; 2],
    /// A shape that shows part of the layer.
    pub shape: Option<ShapeMask>,
    /// Width and height scaled apart, on top of the scale.
    pub stretch: [f32; 2],
    /// The point of the picture that sits at `center`, as fractions of what shows of it.
    pub anchor: [f32; 2],
    /// Turned in space.
    pub turn: Option<Turned>,
    /// The layers that carry this one, nearest first, each as it is placed at this instant.
    pub carried: Vec<Carrier>,
    /// The frame it is drawn into, in target pixels.
    pub frame: [f32; 2],
    /// Where it is at the instants its shutter is open, when it moves between them
    /// (`motion.blur/1`): what it shows is the mean of what it shows placed at each. Empty for a
    /// layer that is drawn once, as it is placed above.
    pub smear: Vec<Placement>,
}

/// How a layer is placed at an instant: everything of it that [`PlannedLayer::matrix`] reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Centre of the layer in target pixels.
    pub center: [f32; 2],
    /// Scale on top of the fit.
    pub scale: f32,
    /// Clockwise rotation, radians.
    pub rotation: f32,
    /// Width and height scaled apart.
    pub stretch: [f32; 2],
    /// Turned in space.
    pub turn: Option<Turned>,
    /// The layers that carry it, nearest first.
    pub carried: Vec<Carrier>,
}

/// The most instants a layer's shutter is sampled at.
pub const SMEAR: usize = 32;

/// A layer turned out of the frame's plane, at an instant.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Turned {
    /// Radians about the horizontal axis: positive tips the top edge away.
    pub x: f32,
    /// Radians about the vertical axis: positive swings the right edge away.
    pub y: f32,
    /// How far the eye is, in target pixels.
    pub eye: f32,
}

/// A layer that carries another, as it is placed at an instant: what of it moves what it carries.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Carrier {
    /// Its anchor in target pixels.
    pub center: [f32; 2],
    /// Its scale.
    pub scale: f32,
    /// Its width and height scaled apart.
    pub stretch: [f32; 2],
    /// Its clockwise rotation, radians.
    pub rotation: f32,
    /// Its turn in space.
    pub turn: Option<Turned>,
}

/// What a layer draws.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedSource {
    /// A picture of an asset at a source instant.
    Media {
        /// The asset id.
        asset: String,
        /// The source instant.
        at: Tick,
        /// What shows when that instant is between two frames of a video.
        between: Between,
    },
    /// A colour filling the frame, premultiplied linear.
    Solid([f32; 4]),
    /// Text, laid out and drawn by the compositor.
    Text(Box<TextContent>),
    /// A shape, drawn by the compositor; its trim and fill are values of the instant.
    Shape(Box<ShapeContent>),
    /// A composition's frame, drawn first on a see-through frame of its own and then placed as a
    /// picture: its layers at its own instant, at the size of the frame it goes into.
    Comp(Box<FramePlan>),
    /// The shadow this layer casts (`shadow.drop@1`): made from its picture as it shows and
    /// placed where it is, by the layer that draws it, which comes just before it in the plan.
    Shadow(Box<PlannedLayer>),
}

impl FramePlan {
    /// Every layer that shows a picture of an asset, the ones inside compositions included, each
    /// with the frame it is drawn into.
    pub fn media_layers(&self) -> Vec<(&FramePlan, &PlannedLayer)> {
        let mut out = Vec::new();
        for layer in &self.layers {
            match &layer.source {
                PlannedSource::Media { .. } => out.push((self, layer)),
                PlannedSource::Comp(inner) => out.extend(inner.media_layers()),
                // a shadow is cast by the picture of the layer after it, which is listed
                PlannedSource::Solid(_)
                | PlannedSource::Text(_)
                | PlannedSource::Shape(_)
                | PlannedSource::Shadow(_) => {}
            }
        }
        out
    }
}

/// How deep compositions may show compositions: a spec that passed validation has no loop, and
/// one that did not stops here.
const DEPTH: usize = 8;

/// A map of the plane, `[a, b, c, d, e, f, g, h, i]`: `x' = (a x + b y + c) / w` and
/// `y' = (d x + e y + f) / w` with `w = g x + h y + i`. For a layer that is not turned in space
/// `w` is 1 (`g` and `h` are 0, `i` is 1) and the map is affine; for one that is, `w` is how far
/// the point is from the eye, as a share of the eye's distance: at 0 or less it is not seen.
pub type Map = [f32; 9];

/// Where `m` takes a point; `None` where it is beside or behind the eye.
pub fn apply(m: &Map, x: f32, y: f32) -> Option<[f32; 2]> {
    let w = m[6] * x + m[7] * y + m[8];
    (w > 1e-4).then(|| [(m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w])
}

/// Whether a map is affine: its `w` is 1 everywhere.
pub fn is_affine(m: &Map) -> bool {
    m[6] == 0.0 && m[7] == 0.0 && m[8] == 1.0
}

/// The box around where `m` takes a `w` by `h` picture's corners, `[x0, y0, x1, y1]`; the whole
/// of `frame` when a corner is not seen (the picture runs past the eye).
pub fn bounds(m: &Map, w: f32, h: f32, frame: [f32; 2]) -> [f32; 4] {
    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let Some([px, py]) = apply(m, x, y) else { return [0.0, 0.0, frame[0], frame[1]] };
        b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
    }
    b
}

/// A map of space that keeps lines parallel: rows for x, y and z, each over `[x, y, z, 1]`.
type Space = [[f32; 4]; 3];

const SPACE: Space = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

/// `a` after `b`.
fn after(a: &Space, b: &Space) -> Space {
    let mut out = [[0.0; 4]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + if c == 3 { a[r][3] } else { 0.0 };
        }
    }
    out
}

/// Moved to `center`, turned in the plane and in space, scaled by `sx` and `sy`: what a layer's
/// own transform does to points counted from its anchor.
fn placed(center: [f32; 2], rotation: f32, turn: Option<Turned>, sx: f32, sy: f32) -> Space {
    let (sin, cos) = rotation.sin_cos();
    let mut m: Space = [[cos, -sin, 0.0, center[0]], [sin, cos, 0.0, center[1]], [0.0, 0.0, 1.0, 0.0]];
    if let Some(t) = turn {
        // about the vertical axis the right edge goes away, about the horizontal the top edge
        let ((sy_, cy_), (sx_, cx_)) = (t.y.sin_cos(), t.x.sin_cos());
        m = after(&m, &[[cy_, 0.0, sy_, 0.0], [0.0, 1.0, 0.0, 0.0], [-sy_, 0.0, cy_, 0.0]]);
        m = after(&m, &[[1.0, 0.0, 0.0, 0.0], [0.0, cx_, -sx_, 0.0], [0.0, sx_, cx_, 0.0]]);
    }
    after(&m, &[[sx, 0.0, 0.0, 0.0], [0.0, sy, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]])
}

/// The plane `z = 0` of `m` as it is on the frame, seen flat.
fn flat(m: &Space) -> Map {
    [m[0][0], m[0][1], m[0][3], m[1][0], m[1][1], m[1][3], 0.0, 0.0, 1.0]
}

/// The plane `z = 0` of `m` as the eye sees it from `eye` in front of `at`.
fn seen(m: &Space, at: [f32; 2], eye: f32) -> Map {
    let (x, y, z) = ([m[0][0], m[0][1], m[0][3]], [m[1][0], m[1][1], m[1][3]], [m[2][0], m[2][1], m[2][3]]);
    let row = |r: [f32; 3], v: f32| [r[0] - v / eye * z[0], r[1] - v / eye * z[1], r[2] - v / eye * z[2]];
    let (rx, ry) = (row(x, at[0]), row(y, at[1]));
    [rx[0], rx[1], rx[2], ry[0], ry[1], ry[2], -z[0] / eye, -z[1] / eye, 1.0 - z[2] / eye]
}

/// `a` after `b`, as maps of the plane.
pub fn then(a: &Map, b: &Map) -> Map {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
        }
    }
    out
}

impl PlannedLayer {
    /// The part of a `w` by `h` picture that shows, after the crop.
    pub fn visible(&self, w: f32, h: f32) -> (f32, f32) {
        let [l, t, r, b] = self.crop;
        ((w * (1.0 - l - r)).max(1e-3), (h * (1.0 - t - b)).max(1e-3))
    }

    /// Where a point of the visible part (`x`, `y` in its pixels, `cw` by `ch`) is in the whole `w`
    /// by `h` picture: mirrored, then moved past the crop.
    pub fn in_picture(&self, x: f32, y: f32, cw: f32, ch: f32, w: f32, h: f32) -> (f32, f32) {
        let x = if self.flip[0] { cw - x } else { x };
        let y = if self.flip[1] { ch - y } else { y };
        (x + self.crop[0] * w, y + self.crop[1] * h)
    }

    /// How many times smaller (1, 2, 4 or 8 each way) a `w` by `h` picture can be and still have
    /// a pixel for every target pixel it covers: a preview drawn small, a layer scaled down.
    pub fn shrink(&self, w: f32, h: f32, target: [f32; 2]) -> u32 {
        let (cw, ch) = self.visible(w, h);
        let Some(per) = self.spread(cw, ch, target) else { return 1 };
        let mut k = 1;
        while k < 8 && per * (2 * k) as f32 <= 1.0 {
            k *= 2;
        }
        k
    }

    /// How many target pixels a pixel of the layer's visible picture (`cw` by `ch`) is spread
    /// over, along the axis it is spread most; `None` for a layer part of which is beside or
    /// behind the eye.
    pub fn spread(&self, cw: f32, ch: f32, target: [f32; 2]) -> Option<f32> {
        let m = self.matrix(cw, ch, target);
        if is_affine(&m) {
            return Some(m[0].hypot(m[3]).max(m[1].hypot(m[4])));
        }
        // seen in perspective: along each of its edges, the nearest being the largest
        let corner = |x: f32, y: f32| apply(&m, x, y);
        let (a, b, c, d) = (corner(0.0, 0.0)?, corner(cw, 0.0)?, corner(cw, ch)?, corner(0.0, ch)?);
        let far = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        Some((far(a, b).max(far(d, c)) / cw).max(far(a, d).max(far(b, c)) / ch))
    }

    /// Whether the layer is only moved, scaled evenly and turned in the plane about its middle,
    /// by itself: what every layer was before `transform.stretch/1` and the rest.
    pub fn plain(&self) -> bool {
        self.stretch == [1.0, 1.0] && self.anchor == [0.5, 0.5] && self.turn.is_none() && self.carried.is_empty()
    }

    /// The map to target pixels from points counted in target pixels from the layer's anchor,
    /// before it is stretched, turned or carried: its own `scale` is not in it.
    pub fn place(&self) -> Map {
        let own = placed(self.center, self.rotation, self.turn, self.stretch[0], self.stretch[1]);
        // what carries it, nearest first: each takes the frame's middle to its own anchor
        let middle: Space = [[1.0, 0.0, 0.0, -self.frame[0] / 2.0], [0.0, 1.0, 0.0, -self.frame[1] / 2.0], SPACE[2]];
        let carry = |c: &Carrier| {
            after(&placed(c.center, c.rotation, c.turn, c.scale * c.stretch[0], c.scale * c.stretch[1]), &middle)
        };
        // the outermost one turned in space is where the eye looks from: what it carries is one
        // object in space with it, and what carries it moves the picture the eye saw
        let turned = self.carried.iter().rposition(|c| c.turn.is_some());
        let inside = turned.map_or(0, |i| i + 1);
        let mut m = own;
        for c in &self.carried[..inside] {
            m = after(&carry(c), &m);
        }
        let mut map = match (turned, self.turn) {
            (Some(i), _) => {
                let c = &self.carried[i];
                seen(&m, c.center, c.turn.expect("found turned").eye)
            }
            (None, Some(t)) => seen(&m, self.center, t.eye),
            (None, None) => flat(&m),
        };
        for c in &self.carried[inside..] {
            map = then(&flat(&carry(c)), &map);
        }
        map
    }

    /// The layer as it is placed at another instant: only what places it, to ask for its
    /// [`matrix`](Self::matrix) there.
    fn moved(&self, p: &Placement) -> PlannedLayer {
        PlannedLayer {
            id: String::new(),
            source: PlannedSource::Solid([0.0; 4]),
            fit: self.fit,
            center: p.center,
            scale: p.scale,
            rotation: p.rotation,
            opacity: self.opacity,
            blend: self.blend,
            fx: Vec::new(),
            lut: None,
            crop: self.crop,
            flip: self.flip,
            shape: self.shape,
            stretch: p.stretch,
            anchor: self.anchor,
            turn: p.turn,
            carried: p.carried.clone(),
            frame: self.frame,
            smear: Vec::new(),
        }
    }

    /// The maps from content pixels to target pixels at the instants the shutter is open
    /// ([`matrix`](Self::matrix) at each); empty for a layer that is not smeared.
    pub fn smears(&self, w: f32, h: f32, target: [f32; 2]) -> Vec<Map> {
        self.smear.iter().map(|p| self.moved(p).matrix(w, h, target)).collect()
    }

    /// For a sprite drawn at this layer's scale and placed by `from` and [`place`](Self::place)
    /// (`from` takes its pixels to target pixels counted from the layer's anchor): its maps at
    /// the instants the shutter is open, each at the scale the layer has then.
    pub fn smears_placed(&self, from: &Map) -> Vec<Map> {
        self.smear
            .iter()
            .map(|p| {
                let k = if self.scale != 0.0 { p.scale / self.scale } else { 1.0 };
                then(&then(&self.moved(p).place(), &[k, 0.0, 0.0, 0.0, k, 0.0, 0.0, 0.0, 1.0]), from)
            })
            .collect()
    }

    /// The map from content pixels (a `w` by `h` picture) to target pixels.
    pub fn matrix(&self, w: f32, h: f32, target: [f32; 2]) -> Map {
        let (sx, sy) = match self.fit {
            Fit::Contain => {
                let s = (target[0] / w).min(target[1] / h);
                (s, s)
            }
            Fit::Cover => {
                let s = (target[0] / w).max(target[1] / h);
                (s, s)
            }
            Fit::Stretch => (target[0] / w, target[1] / h),
        };
        let (sx, sy) = (sx * self.scale, sy * self.scale);
        if !self.plain() {
            // scaled about the anchor, then placed
            let (ox, oy) = (-w * self.anchor[0], -h * self.anchor[1]);
            return then(&self.place(), &[sx, 0.0, sx * ox, 0.0, sy, sy * oy, 0.0, 0.0, 1.0]);
        }
        let (sin, cos) = self.rotation.sin_cos();
        // translate(center) · rotate · scale · translate(-w/2, -h/2)
        let (a, b, d, e) = (cos * sx, -sin * sy, sin * sx, cos * sy);
        let (ox, oy) = (-w / 2.0, -h / 2.0);
        [a, b, a * ox + b * oy + self.center[0], d, e, d * ox + e * oy + self.center[1], 0.0, 0.0, 1.0]
    }
}

/// Inverts a map; `None` when it collapses (scale 0, or a layer seen edge on).
pub fn invert(m: Map) -> Option<Map> {
    if is_affine(&m) {
        let det = m[0] * m[4] - m[1] * m[3];
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, d, e) = (m[4] / det, -m[1] / det, -m[3] / det, m[0] / det);
        return Some([a, b, -(a * m[2] + b * m[5]), d, e, -(d * m[2] + e * m[5]), 0.0, 0.0, 1.0]);
    }
    // the adjugate over the determinant, in doubles: the terms nearly cancel for a far eye
    let m = m.map(f64::from);
    let co = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    let det = m[0] * co[0] + m[1] * co[3] + m[2] * co[6];
    if det.abs() < 1e-12 {
        return None;
    }
    Some(co.map(|c| (c / det) as f32))
}

/// Plans the frame at `t`, drawn at `scale` times the output size.
pub fn plan_frame(spec: &RenderSpec, t: Tick, scale: f32) -> FramePlan {
    let o = &spec.output;
    let width = ((o.width as f32 * scale).round() as u32).max(1);
    let height = ((o.height as f32 * scale).round() as u32).max(1);
    let layers = plan_layers(spec, &spec.layers, t, [width, height], 0);
    FramePlan { t, width, height, background: premultiply_srgb(o.background.0), layers }
}

/// The layers of `all` on screen at `t`, for a frame of `size` pixels.
fn plan_layers(
    spec: &RenderSpec,
    all: &[reelforge_renderspec::Layer],
    t: Tick,
    size: [u32; 2],
    depth: usize,
) -> Vec<PlannedLayer> {
    let [width, height] = size;
    let mut layers = Vec::new();
    for layer in all {
        if t < layer.at || t >= layer.at + layer.dur {
            continue;
        }
        let local = t - layer.at;
        let (source, fit) = match &layer.content {
            LayerContent::Media { asset, time, fit, between } => {
                let at = time.source_at(local);
                (PlannedSource::Media { asset: asset.clone(), at, between: *between }, *fit)
            }
            LayerContent::Solid { color } => (PlannedSource::Solid(premultiply_srgb(color.0)), Fit::Stretch),
            LayerContent::Text(text) => {
                let mut text = text.clone();
                // how much shows is a value of this instant: the sprite is drawn for it
                if let Some(r) = &text.reveal {
                    text.reveal = Some(Animated::Constant(r.sample(local).clamp(0.0, 1.0)));
                }
                // and so is how far along its characters are
                for m in &mut text.chars {
                    m.progress = Animated::Constant(m.progress.sample(local));
                }
                // and how much of it has been said or written
                if let Some(h) = &mut text.highlight {
                    h.progress = Animated::Constant(h.progress.sample(local));
                }
                if let Some(d) = &mut text.draw {
                    d.progress = Animated::Constant(d.progress.sample(local).clamp(0.0, 1.0));
                }
                // and so is the number it shows: written into its words
                if let Some(n) = text.number.take() {
                    text.text = n.fill(&text.text, n.value.sample(local));
                }
                (PlannedSource::Text(text), Fit::Contain)
            }
            LayerContent::Shape(shape) => {
                let mut shape = shape.clone();
                if let Some(t) = &shape.trim {
                    let (a, b) = (t.start.sample(local).clamp(0.0, 1.0), t.end.sample(local).clamp(0.0, 1.0));
                    shape.trim = Some(Trim { start: Animated::Constant(a), end: Animated::Constant(b) });
                }
                if let Some(o) = &shape.fill_opacity {
                    shape.fill_opacity = Some(Animated::Constant(o.sample(local).clamp(0.0, 1.0)));
                }
                (PlannedSource::Shape(shape), Fit::Contain)
            }
            LayerContent::Comp { comp, from } => {
                let (Some(inner), true) = (spec.comps.get(comp), depth < DEPTH) else { continue };
                let at = *from + local;
                let layers = plan_layers(spec, &inner.layers, at, size, depth + 1);
                if layers.is_empty() {
                    // past its end, or between its clips: nothing to place
                    continue;
                }
                let frame = FramePlan { t: at, width, height, background: [0.0; 4], layers };
                (PlannedSource::Comp(Box::new(frame)), Fit::Contain)
            }
        };
        let frame = [width as f32, height as f32];
        // where it is at an output instant: itself on its own clock, and what carries it, up
        // the chain, each on its own
        let placement = |t: Tick| {
            let local = t - layer.at;
            let pos = layer.transform.position.sample(local);
            let mut carried = Vec::new();
            let mut up = layer.parent.as_deref();
            while let Some(parent) = up.and_then(|id| all.iter().find(|l| l.id == id)) {
                if carried.len() == reelforge_renderspec::PARENTS || parent.id == layer.id {
                    break;
                }
                let (tr, at) = (&parent.transform, t - parent.at);
                let pos = tr.position.sample(at);
                carried.push(Carrier {
                    center: [frame[0] * (0.5 + pos[0]), frame[1] * (0.5 + pos[1])],
                    scale: tr.scale.sample(at),
                    stretch: tr.stretch.as_ref().map_or([1.0, 1.0], |s| s.sample(at)),
                    rotation: tr.rotation.sample(at).to_radians(),
                    turn: tr.turn.as_ref().map(|turn| turned(turn, at, frame)),
                });
                up = parent.parent.as_deref();
            }
            Placement {
                center: [frame[0] * (0.5 + pos[0]), frame[1] * (0.5 + pos[1])],
                scale: layer.transform.scale.sample(local),
                rotation: layer.transform.rotation.sample(local).to_radians(),
                stretch: layer.transform.stretch.as_ref().map_or([1.0, 1.0], |s| s.sample(local)),
                // lying flat at this instant, it is drawn as a layer that is never turned
                turn: layer
                    .transform
                    .turn
                    .as_ref()
                    .map(|turn| turned(turn, local, frame))
                    .filter(|t| t.x != 0.0 || t.y != 0.0),
                carried,
            }
        };
        let now = placement(t);
        let mut planned = PlannedLayer {
            id: layer.id.clone(),
            source,
            fit,
            center: now.center,
            scale: now.scale,
            rotation: now.rotation,
            opacity: layer.opacity.sample(local).clamp(0.0, 1.0),
            blend: layer.blend,
            fx: layer
                .fx
                .iter()
                .map(|fx| (fx.id.clone(), fx.params.iter().map(|(k, v)| (k.clone(), v.sample(local))).collect()))
                .collect(),
            lut: layer.fx.iter().find(|fx| fx.id == effects::LUT).and_then(|fx| fx.asset.clone()),
            crop: match (&layer.content, layer.crop_keys.as_ref().map(|k| k.sample(local)).or(layer.crop)) {
                (LayerContent::Media { .. } | LayerContent::Comp { .. }, Some(c)) => [c.left, c.top, c.right, c.bottom],
                _ => [0.0; 4],
            },
            flip: match &layer.content {
                LayerContent::Media { .. } | LayerContent::Comp { .. } => [layer.flip_x, layer.flip_y],
                _ => [false; 2],
            },
            shape: layer.shape_mask.map(|m| layer.shape_mask_keys.as_ref().map_or(m, |k| k.sample(m, local))),
            stretch: now.stretch,
            anchor: layer.transform.anchor.unwrap_or([0.5, 0.5]),
            turn: now.turn,
            carried: now.carried,
            frame,
            smear: Vec::new(),
        };
        if let Some(blur) = layer.motion_blur {
            planned.smear = smear(&planned, blur.shutter, spec.output.fps.frame_duration(), |by| placement(t + by));
        }
        // its shadow lies under it
        if let Some(shadow) = crate::shadow::Shadow::of(&planned)
            && matches!(planned.source, PlannedSource::Media { .. } | PlannedSource::Comp(_))
            && planned.opacity > 0.0
        {
            layers.push(PlannedLayer {
                id: format!("{}~shadow", planned.id),
                source: PlannedSource::Shadow(Box::new(planned.clone())),
                opacity: planned.opacity * shadow.opacity,
                blend: Blend::Normal,
                fx: Vec::new(),
                lut: None,
                crop: [0.0; 4],
                flip: [false; 2],
                shape: None,
                ..planned.clone()
            });
        }
        layers.push(planned);
    }
    layers
}

/// Where `layer` is at the instants a shutter open for `shutter` of a frame of `frame` ticks sees
/// it, centred on the frame's own: as many as it takes for no two to be more than a couple of
/// target pixels apart, [`SMEAR`] at most, and none when it does not move a pixel (it is then
/// drawn once, as sharp as it is). `at` places it at an offset from the frame's instant.
fn smear(layer: &PlannedLayer, shutter: f32, frame: Tick, at: impl Fn(Tick) -> Placement) -> Vec<Placement> {
    let open = (frame.0 as f64 * f64::from(shutter.clamp(0.0, 2.0))) as i64;
    if open <= 0 {
        return Vec::new();
    }
    // how far the corners of a frame-sized picture on its anchor go while the shutter is open:
    // through the middle of that time as well, for a move that turns back
    let [w, h] = layer.frame;
    let corners = |p: &Placement| {
        let m = layer.moved(p).place();
        let (x, y) = (w / 2.0 * p.scale, h / 2.0 * p.scale);
        [(-x, -y), (x, -y), (-x, y), (x, y), (0.0, 0.0)].map(|(x, y)| apply(&m, x, y))
    };
    let far = |a: &Placement, b: &Placement| {
        corners(a).iter().zip(corners(b)).fold(0.0f32, |far, (p, q)| match (p, q) {
            (Some(p), Some(q)) => far.max((p[0] - q[0]).hypot(p[1] - q[1])),
            // past the eye at one end: as far as can be
            _ => f32::INFINITY,
        })
    };
    let (first, mid, last) = (at(Tick(-open / 2)), at(Tick::ZERO), at(Tick(open - open / 2)));
    let travel = far(&first, &mid) + far(&mid, &last);
    if travel.is_nan() || travel < 1.0 {
        return Vec::new();
    }
    let n = if travel.is_finite() { ((travel / 2.0).ceil() as usize).clamp(2, SMEAR) } else { SMEAR };
    // the middles of `n` equal parts of the time the shutter is open
    (0..n as i64).map(|j| at(Tick(open * (2 * j + 1 - n as i64) / (2 * n as i64)))).collect()
}

/// A turn in space at `t` from its layer's start, on a frame of `frame` target pixels.
fn turned(turn: &reelforge_renderspec::Turn, t: Tick, frame: [f32; 2]) -> Turned {
    Turned {
        x: turn.x.sample(t).to_radians(),
        y: turn.y.sample(t).to_radians(),
        eye: turn.distance.max(0.1) * frame[1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_inverts() {
        let l = PlannedLayer {
            id: "x".into(),
            source: PlannedSource::Solid([0.0; 4]),
            fit: Fit::Contain,
            center: [100.0, 200.0],
            scale: 0.5,
            rotation: 0.3,
            opacity: 1.0,
            blend: Blend::Normal,
            fx: vec![],
            lut: None,
            crop: [0.0; 4],
            flip: [false; 2],
            shape: None,
            stretch: [1.0, 1.0],
            anchor: [0.5, 0.5],
            turn: None,
            carried: vec![],
            frame: [1080.0, 1920.0],
            smear: Vec::new(),
        };
        let m = l.matrix(640.0, 360.0, [1080.0, 1920.0]);
        let inv = invert(m).unwrap();
        let (x, y) = (12.0f32, 34.0f32);
        let (px, py) = (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]);
        let (bx, by) = (inv[0] * px + inv[1] * py + inv[2], inv[3] * px + inv[4] * py + inv[5]);
        assert!((bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3);
        // The content's centre lands on the layer's centre.
        let (cx, cy) = (m[0] * 320.0 + m[1] * 180.0 + m[2], m[3] * 320.0 + m[4] * 180.0 + m[5]);
        assert!((cx - 100.0).abs() < 1e-3 && (cy - 200.0).abs() < 1e-3);
    }

    #[test]
    fn a_number_and_a_trim_are_planned_at_the_instant() {
        use reelforge_renderspec::{Color, Key, NumberText, Paint, ShapeKind, ShapeStroke};
        fn key(t: f64, v: f32) -> Key<f32> {
            Key { t: Tick::from_seconds(t), v, ease: Default::default() }
        }
        let mut spec = crate::tests::spec();
        let mut text = TextContent::new("¥{n}", 10.0, Color([1.0; 4]));
        text.number = Some(NumberText {
            value: Animated::Keys(vec![key(0.0, 0.0), key(1.0, 100.0)]),
            decimals: 0,
            group: false,
            clock: false,
        });
        spec.layers[0].content = LayerContent::Text(Box::new(text));
        let mut shape = spec.layers[0].clone();
        shape.id = "shape".into();
        shape.content = LayerContent::Shape(Box::new(ShapeContent {
            kind: ShapeKind::Ellipse,
            size: [10.0, 10.0],
            radius: 0.0,
            points: 5,
            inner: 0.5,
            fill: Some(Paint::Solid { color: Color([1.0; 4]) }),
            stroke: Some(ShapeStroke { color: Color([1.0; 4]), width: 2.0 }),
            shadow: None,
            trim: Some(Trim {
                start: Animated::Constant(0.0),
                end: Animated::Keys(vec![key(0.0, 0.0), key(1.0, 2.0)]),
            }),
            fill_opacity: Some(Animated::Keys(vec![key(0.0, 0.0), key(1.0, 1.0)])),
        }));
        spec.layers.push(shape);
        let plan = plan_frame(&spec, Tick::from_seconds(0.25), 1.0);
        let PlannedSource::Text(t) = &plan.layers[0].source else { panic!() };
        assert_eq!((t.text.as_str(), &t.number), ("¥25", &None));
        let PlannedSource::Shape(s) = &plan.layers[1].source else { panic!() };
        let trim = s.trim.as_ref().unwrap();
        assert_eq!((&trim.start, &trim.end), (&Animated::Constant(0.0), &Animated::Constant(0.5)));
        assert_eq!(s.fill_opacity, Some(Animated::Constant(0.25)));
        // a trim past the whole outline is the whole outline
        let plan = plan_frame(&spec, Tick::from_seconds(0.9), 1.0);
        let PlannedSource::Shape(s) = &plan.layers[1].source else { panic!() };
        assert_eq!(s.trim.as_ref().unwrap().end, Animated::Constant(1.0));
    }

    #[test]
    fn keyed_crops_and_masks_are_planned_at_the_instant() {
        use reelforge_renderspec::{CropKeys, Key, MaskShape, ShapeMaskKeys};
        let mut spec = crate::tests::spec();
        spec.layers[0].content = LayerContent::Media {
            asset: "a".into(),
            time: Default::default(),
            fit: Fit::Contain,
            between: Default::default(),
        };
        fn key<T>(t: f64, v: T) -> Key<T> {
            Key { t: Tick::from_seconds(t), v, ease: Default::default() }
        }
        let l = &mut spec.layers[0];
        l.crop_keys = Some(CropKeys {
            left: Animated::Keys(vec![key(0.0, 0.0), key(1.0, 0.8)]),
            top: Animated::Constant(0.1),
            right: Animated::Constant(0.4),
            bottom: Animated::Constant(0.0),
        });
        l.shape_mask = Some(ShapeMask {
            shape: MaskShape::Circle,
            center: [0.5, 0.5],
            size: [0.5, 0.5],
            rotation: 0.0,
            feather: 0.0,
            invert: true,
        });
        l.shape_mask_keys = Some(ShapeMaskKeys {
            center: Animated::Keys(vec![key(0.0, [0.0, 0.5]), key(1.0, [1.0, 0.5])]),
            size: Animated::Constant([0.2, 0.2]),
            rotation: Animated::Constant(0.0),
            feather: Animated::Constant(0.0),
        });
        let at = |t: f64| plan_frame(&spec, Tick::from_seconds(t), 1.0).layers.remove(0);
        let half = at(0.5);
        assert!((half.crop[0] - 0.4).abs() < 1e-6 && half.crop[1] == 0.1);
        let m = half.shape.unwrap();
        assert!((m.center[0] - 0.5).abs() < 1e-6 && m.size == [0.2, 0.2] && m.invert);
        // keys that would cut everything leave 1 % of the picture, shared between the two edges
        let end = at(0.999);
        assert!(end.crop[0] + end.crop[2] <= 0.99 + 1e-6);
    }
}
