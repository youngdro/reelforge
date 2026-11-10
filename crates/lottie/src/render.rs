//! Drawing a frame: layers bottom first, each placed by its own and its parents' transforms,
//! cut by its masks and shown through its matte.

use crate::Animation;
use crate::geom::{Affine, Bez, Cap, Join, Line, P, add, mul, outline, rounded};
use crate::model::{Colour, Content, Item, Layer, MaskMode, Matte, Star, Text};
use crate::raster::{Coverage, Paint, Rule, Surface, cover, paint};

/// What a fill or stroke is painted with, at one frame.
enum Ink {
    Solid([f32; 4]),
    Gradient { radial: bool, start: P, end: P, stops: Vec<(f32, [f32; 4])> },
}

/// How paths are painted.
enum Style {
    Fill(Rule),
    Stroke { width: f32, cap: Cap, join: Join, miter_limit: f32, dashes: Vec<f32>, dash_offset: f32 },
}

/// One fill or stroke of some paths.
struct Draw {
    ink: Ink,
    style: Style,
    opacity: f32,
    /// The paths, by their place in [`Shapes::paths`].
    paths: Vec<usize>,
    /// From the space the style is in to the layer's.
    to_layer: Affine,
}

/// A path as it is drawn: straight pieces in its group's space.
struct Outline {
    lines: Vec<Line>,
    to_layer: Affine,
}

/// Something that changes the paths above it.
#[derive(Clone)]
enum Modifier {
    Round(f32),
    /// Each path trimmed by itself: where it starts and how much of it shows, as parts of its
    /// length.
    Trim(f32, f32),
    /// The paths trimmed as one line end to end: which of [`Shapes::together`].
    TrimTogether(usize),
}

/// The contents of a shape layer worked out for one frame.
struct Shapes {
    frame: f32,
    /// From the layer to the picture, to know how finely to cut curves.
    device: Affine,
    paths: Vec<Outline>,
    /// Trims of several paths as one: start, span, and the paths in order.
    together: Vec<(f32, f32, Vec<usize>)>,
}

const K: f32 = 0.5519;

fn rect(position: P, size: P, round: f32, reversed: bool) -> Bez {
    let (x, y, w, h) = (position[0], position[1], size[0] / 2.0, size[1] / 2.0);
    let r = round.min(w).min(h).max(0.0);
    let c = r * (1.0 - K);
    // from the top right corner, clockwise
    let mut pts: Vec<(P, P, P)> = if r > 0.0 {
        vec![
            ([x + w, y - h + r], [x + w, y - h + c], [x + w, y - h + r]),
            ([x + w, y + h - r], [x + w, y + h - r], [x + w, y + h - c]),
            ([x + w - r, y + h], [x + w - c, y + h], [x + w - r, y + h]),
            ([x - w + r, y + h], [x - w + r, y + h], [x - w + c, y + h]),
            ([x - w, y + h - r], [x - w, y + h - c], [x - w, y + h - r]),
            ([x - w, y - h + r], [x - w, y - h + r], [x - w, y - h + c]),
            ([x - w + r, y - h], [x - w + c, y - h], [x - w + r, y - h]),
            ([x + w - r, y - h], [x + w - r, y - h], [x + w - c, y - h]),
        ]
    } else {
        [[x + w, y - h], [x + w, y + h], [x - w, y + h], [x - w, y - h]].into_iter().map(|p| (p, p, p)).collect()
    };
    if reversed {
        pts.reverse();
        pts = pts.into_iter().map(|(v, i, o)| (v, o, i)).collect();
    }
    Bez::absolute(&pts, true)
}

fn ellipse(position: P, size: P, reversed: bool) -> Bez {
    let (x, y, w, h) = (position[0], position[1], size[0] / 2.0, size[1] / 2.0);
    let s = if reversed { -1.0 } else { 1.0 };
    let pts = [
        ([x, y - h], [x - s * w * K, y - h], [x + s * w * K, y - h]),
        ([x + s * w, y], [x + s * w, y - h * K], [x + s * w, y + h * K]),
        ([x, y + h], [x + s * w * K, y + h], [x - s * w * K, y + h]),
        ([x - s * w, y], [x - s * w, y + h * K], [x - s * w, y - h * K]),
    ];
    Bez::absolute(&pts, true)
}

fn star(s: &Star, frame: f32) -> Bez {
    let points = s.points.one(frame).floor().clamp(0.0, 512.0) as usize;
    let centre = s.position.two(frame);
    let dir = if s.reversed { -1.0 } else { 1.0 };
    let count = if s.star { points * 2 } else { points };
    if count < 3 {
        return Bez::default();
    }
    let step = std::f32::consts::TAU / count as f32;
    let (outer, inner) = (s.outer.one(frame), s.inner.one(frame));
    let (outer_round, inner_round) = (s.outer_round.one(frame) / 100.0, s.inner_round.one(frame) / 100.0);
    let mut angle = -std::f32::consts::FRAC_PI_2 + s.rotation.one(frame).to_radians();
    let mut pts = Vec::with_capacity(count);
    for k in 0..count {
        let long = !s.star || k % 2 == 0;
        let (radius, round) = if long { (outer, outer_round) } else { (inner, inner_round) };
        let arc = std::f32::consts::TAU * radius / (count as f32 * if s.star { 2.0 } else { 4.0 });
        let (x, y) = (radius * angle.cos(), radius * angle.sin());
        let l = x.hypot(y);
        let (ox, oy) = if l > 0.0 { (y / l, -x / l) } else { (0.0, 0.0) };
        let v = [x + centre[0], y + centre[1]];
        let t = [ox * arc * round * dir, oy * arc * round * dir];
        pts.push((v, add(v, t), add(v, mul(t, -1.0))));
        angle += step * dir;
    }
    Bez::absolute(&pts, true)
}

impl Shapes {
    fn tolerance(&self, to_layer: Affine) -> f32 {
        0.2 / self.device.then_after(to_layer).stretch().max(1e-3)
    }

    fn add_path(&mut self, bez: Bez, to_layer: Affine, mods: &[Modifier]) -> usize {
        let mut bez = bez;
        // the modifier nearest the path changes it first
        for m in mods.iter().rev() {
            if let Modifier::Round(r) = m {
                bez = rounded(&bez, *r);
            }
        }
        let mut lines = vec![bez.flatten(self.tolerance(to_layer))];
        let id = self.paths.len();
        for m in mods.iter().rev() {
            match m {
                Modifier::Trim(start, span) => {
                    lines = lines.iter().flat_map(|l| l.trimmed(*start, *span)).collect();
                }
                Modifier::TrimTogether(which) => self.together[*which].2.push(id),
                Modifier::Round(_) => {}
            }
        }
        self.paths.push(Outline { lines, to_layer });
        id
    }

    /// Works out what `items` draw. Returns the fills and strokes, top first, and the paths
    /// found, which the styles of the group around this one paint as well.
    fn walk(&mut self, items: &[Item], parent: Affine, opacity: f32, mods: &[Modifier]) -> (Vec<Draw>, Vec<usize>) {
        let frame = self.frame;
        let own = items.iter().find_map(|i| if let Item::Transform(t) = i { Some(t) } else { None });
        let m = own.map_or(parent, |t| parent.then_after(t.matrix(frame)));
        let opacity = opacity * own.map_or(1.0, |t| t.opacity(frame));
        let mut mods = mods.to_vec();
        // what each item gives, by its place: paths found, and fills and strokes to draw
        let mut found: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut blocks: Vec<(usize, Vec<Draw>)> = Vec::new();
        let mut styles: Vec<usize> = Vec::new();
        for i in (0..items.len()).rev() {
            match &items[i] {
                Item::Round(r) => mods.push(Modifier::Round(r.one(frame))),
                Item::Trim { start, end, offset, together } => {
                    let (s, e) = (start.one(frame) / 100.0, end.one(frame) / 100.0);
                    let (from, span) = (s.min(e) + offset.one(frame) / 360.0, (s - e).abs().min(1.0));
                    if *together {
                        self.together.push((from, span, Vec::new()));
                        mods.push(Modifier::TrimTogether(self.together.len() - 1));
                    } else {
                        mods.push(Modifier::Trim(from, span));
                    }
                }
                Item::Repeater(rp) => {
                    // everything above it is drawn once for each copy
                    let copies = rp.copies.one(frame).ceil().clamp(0.0, 512.0) as usize;
                    let (a, p, s) = (rp.anchor.two(frame), rp.position.two(frame), rp.scale.two(frame));
                    let (r, offset) = (rp.rotation.one(frame), rp.offset.one(frame));
                    let (first, last) = (rp.start_opacity.one(frame) / 100.0, rp.end_opacity.one(frame) / 100.0);
                    let copy = |k: f32| {
                        Affine::translate(p[0] * k + a[0], p[1] * k + a[1])
                            .then_after(Affine::rotate(r * k))
                            .then_after(Affine::scale((s[0] / 100.0).powf(k), (s[1] / 100.0).powf(k)))
                            .then_after(Affine::translate(-a[0], -a[1]))
                    };
                    let mut draws = Vec::new();
                    let mut ids = Vec::new();
                    let order: Vec<usize> = if rp.below { (0..copies).collect() } else { (0..copies).rev().collect() };
                    for k in order {
                        let part = if copies > 1 { k as f32 / (copies - 1) as f32 } else { 0.0 };
                        let (d, found) = self.walk(
                            &items[..i],
                            m.then_after(copy(k as f32 + offset)),
                            opacity * (first + (last - first) * part).clamp(0.0, 1.0),
                            &mods,
                        );
                        draws.extend(d);
                        ids.extend(found);
                    }
                    blocks.push((i, draws));
                    // (i is 0 for nothing above it; styles below still paint the copies)
                    found.push((i.saturating_sub(1), ids));
                    break;
                }
                Item::Group(inner) => {
                    let (draws, ids) = self.walk(inner, m, opacity, &mods);
                    blocks.push((i, draws));
                    found.push((i, ids));
                }
                Item::Path(p) => found.push((i, vec![self.add_path(p.at(frame), m, &mods)])),
                Item::Rect { position, size, round, reversed } => {
                    let bez = rect(position.two(frame), size.two(frame), round.one(frame), *reversed);
                    found.push((i, vec![self.add_path(bez, m, &mods)]));
                }
                Item::Ellipse { position, size, reversed } => {
                    let bez = ellipse(position.two(frame), size.two(frame), *reversed);
                    found.push((i, vec![self.add_path(bez, m, &mods)]));
                }
                Item::Star(s) => found.push((i, vec![self.add_path(star(s, frame), m, &mods)])),
                Item::Fill { .. } | Item::Stroke { .. } => styles.push(i),
                Item::Transform(_) | Item::Other => {}
            }
        }
        // a style paints every path above it
        for i in styles {
            let paths: Vec<usize> =
                found.iter().filter(|(at, _)| *at < i).flat_map(|(_, ids)| ids.iter().copied()).collect();
            if paths.is_empty() {
                continue;
            }
            let ink = |c: &Colour| match c {
                Colour::Solid(c) => {
                    let c = c.at(frame);
                    let part = |k: usize| c.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    Ink::Solid([part(0), part(1), part(2), 1.0])
                }
                Colour::Gradient(g) => Ink::Gradient {
                    radial: g.radial,
                    start: g.start.two(frame),
                    end: g.end.two(frame),
                    stops: g.stops(frame),
                },
            };
            let draw = match &items[i] {
                Item::Fill { colour, opacity: o, rule } => Draw {
                    ink: ink(colour),
                    style: Style::Fill(*rule),
                    opacity: opacity * (o.one(frame) / 100.0).clamp(0.0, 1.0),
                    paths,
                    to_layer: m,
                },
                Item::Stroke { colour, opacity: o, pen } => Draw {
                    ink: ink(colour),
                    style: Style::Stroke {
                        width: pen.width.one(frame),
                        cap: pen.cap,
                        join: pen.join,
                        miter_limit: pen.miter_limit,
                        dashes: pen.dashes.iter().map(|d| d.one(frame)).collect(),
                        dash_offset: pen.dash_offset.as_ref().map_or(0.0, |d| d.one(frame)),
                    },
                    opacity: opacity * (o.one(frame) / 100.0).clamp(0.0, 1.0),
                    paths,
                    to_layer: m,
                },
                _ => continue,
            };
            blocks.push((i, vec![draw]));
        }
        blocks.sort_by_key(|b| b.0);
        (blocks.into_iter().flat_map(|b| b.1).collect(), found.into_iter().flat_map(|f| f.1).collect())
    }

    /// Trims the paths that are trimmed as one line.
    fn trim_together(&mut self) {
        for (start, span, ids) in std::mem::take(&mut self.together) {
            // in the order the file lists them
            let ids: Vec<usize> = ids.into_iter().rev().collect();
            let lengths: Vec<f32> =
                ids.iter().map(|id| self.paths[*id].lines.iter().map(Line::length).sum::<f32>()).collect();
            let total: f32 = lengths.iter().sum();
            if total <= 0.0 || span >= 1.0 - 1e-6 {
                continue;
            }
            let start = start.rem_euclid(1.0);
            // one or two windows of the whole length
            let windows: Vec<(f32, f32)> = if start + span <= 1.0 {
                vec![(start * total, (start + span) * total)]
            } else {
                vec![(start * total, total), (0.0, (start + span - 1.0) * total)]
            };
            let mut at = 0.0;
            for (id, length) in ids.iter().zip(&lengths) {
                let mut kept = Vec::new();
                let mut line_at = at;
                for line in &self.paths[*id].lines {
                    let l = line.length();
                    for (from, to) in &windows {
                        let (a, b) = ((from - line_at).max(0.0), (to - line_at).min(l));
                        if b > a && l > 0.0 {
                            kept.extend(line.trimmed(a / l, (b - a) / l));
                        }
                    }
                    line_at += l;
                }
                self.paths[*id].lines = kept;
                at += length;
            }
        }
    }
}

/// Draws frames of one animation at one size.
pub(crate) struct Renderer<'a> {
    pub anim: &'a Animation,
    pub w: usize,
    pub h: usize,
    /// Pictures to draw layers apart on, cleared.
    spare: Vec<Surface>,
}

fn luma(p: [f32; 4]) -> f32 {
    0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]
}

impl<'a> Renderer<'a> {
    pub fn new(anim: &'a Animation, w: usize, h: usize) -> Renderer<'a> {
        Renderer { anim, w, h, spare: Vec::new() }
    }

    fn take(&mut self) -> Surface {
        self.spare.pop().unwrap_or_else(|| Surface::new(self.w, self.h))
    }

    fn give(&mut self, mut s: Surface) {
        s.clear();
        self.spare.push(s);
    }

    /// Where a layer is in its composition at `frame`: its own transform under its parents'.
    fn placed(layers: &[Layer], index: usize, frame: f32) -> Affine {
        let own = |l: &Layer| {
            if !l.auto_orient {
                return l.transform.matrix(frame);
            }
            // turned the way it is going
            let (a, b) = (l.transform.position(frame), l.transform.position(frame + 0.05));
            let turn = if a == b { 0.0 } else { (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees() };
            let anchor = l.transform.anchor.two(frame);
            l.transform
                .matrix(frame)
                .then_after(Affine::translate(anchor[0], anchor[1]))
                .then_after(Affine::rotate(turn))
                .then_after(Affine::translate(-anchor[0], -anchor[1]))
        };
        let mut m = own(&layers[index]);
        let mut at = &layers[index];
        for _ in 0..64 {
            let Some(parent) = at.parent.and_then(|p| layers.iter().find(|l| l.index == Some(p))) else { break };
            m = own(parent).then_after(m);
            at = parent;
        }
        m
    }

    /// Draws the layers of a composition, bottom first.
    pub fn comp(
        &mut self,
        layers: &[Layer],
        frame: f32,
        base: Affine,
        target: &mut Surface,
        depth: usize,
        clip: Option<&Coverage>,
    ) {
        for index in (0..layers.len()).rev() {
            let l = &layers[index];
            if !l.is_matte && frame >= l.from && frame < l.until {
                self.layer(layers, index, frame, base, target, depth, clip);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layer(
        &mut self,
        layers: &[Layer],
        index: usize,
        frame: f32,
        base: Affine,
        target: &mut Surface,
        depth: usize,
        clip: Option<&Coverage>,
    ) {
        let l = &layers[index];
        let opacity = l.transform.opacity(frame);
        if matches!(l.content, Content::Nothing) || opacity <= 0.0 {
            return;
        }
        let m = base.then_after(Self::placed(layers, index, frame));
        // the layer it is shown through: the one the file names, or the one above it
        let matte = l.matte.and_then(|kind| {
            let at = match l.matte_layer {
                Some(named) => layers.iter().position(|o| o.index == Some(named)),
                None => index.checked_sub(1),
            };
            at.map(|at| (kind, at))
        });
        // one fill or stroke fades as it is; anything more is drawn apart and faded as one picture
        let single = match &l.content {
            Content::Solid { .. } | Content::Image(_) => true,
            Content::Shapes(items) => simple(items),
            _ => false,
        };
        // its masks, as how much of each pixel they let show
        let masked = if l.masks.is_empty() {
            None
        } else {
            let Some(kept) = self.mask(l, frame, m) else { return };
            Some(kept)
        };
        if matte.is_none() && (single || opacity >= 0.999) {
            // drawn straight onto the picture, through its masks and whatever cuts the picture
            return match masked {
                Some(kept) => {
                    let Some(both) = kept.clipped(clip) else { return };
                    self.content(l, frame, m, opacity, target, depth, Some(&both))
                }
                None => self.content(l, frame, m, opacity, target, depth, clip),
            };
        }
        let mut own = self.take();
        self.content(l, frame, m, 1.0, &mut own, depth, masked.as_ref());
        if let Some((kind, at)) = matte {
            let mut through = self.take();
            let by = &layers[at];
            if frame >= by.from && frame < by.until {
                self.layer(layers, at, frame, base, &mut through, depth, None);
            }
            own.keep(&through, |p| match kind {
                Matte::Alpha => p[3],
                Matte::AlphaInverted => 1.0 - p[3],
                Matte::Luma => luma(p),
                Matte::LumaInverted => 1.0 - luma(p),
            });
            self.give(through);
        }
        target.draw(&own, opacity, clip);
        self.give(own);
    }

    /// How much of each pixel the layer's masks let show; `None` when that is nothing anywhere.
    fn mask(&self, l: &Layer, frame: f32, m: Affine) -> Option<Coverage> {
        let covers: Vec<Option<Coverage>> = l
            .masks
            .iter()
            .map(|mask| cover(&[mask.path.at(frame).map(m).flatten(0.2).pts], Rule::NonZero, self.w, self.h))
            .collect();
        // starting from nothing, only what a mask adds can show: the boxes of those that add
        let bounded =
            l.masks[0].mode == MaskMode::Add && !l.masks.iter().any(|k| k.mode == MaskMode::Add && k.inverted);
        let (x0, y0, x1, y1) = if bounded {
            let adding =
                l.masks.iter().zip(&covers).filter(|(k, _)| k.mode == MaskMode::Add).filter_map(|(_, c)| c.as_ref());
            adding.fold((self.w, self.h, 0, 0), |b, c| {
                (b.0.min(c.x0), b.1.min(c.y0), b.2.max(c.x0 + c.w), b.3.max(c.y0 + c.h))
            })
        } else {
            (0, 0, self.w, self.h)
        };
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        let mut kept = vec![if l.masks[0].mode == MaskMode::Add { 0.0f32 } else { 1.0 }; bw * bh];
        for (mask, cov) in l.masks.iter().zip(&covers) {
            let o = (mask.opacity.one(frame) / 100.0).clamp(0.0, 1.0);
            let step = |k: &mut f32, c: f32| {
                let c = if mask.inverted { 1.0 - c } else { c } * o;
                *k = match mask.mode {
                    MaskMode::Add => *k + c - *k * c,
                    MaskMode::Subtract => *k * (1.0 - c),
                    MaskMode::Intersect => *k * c,
                };
            };
            // where the mask's own box crosses the one kept
            let inner = cov
                .as_ref()
                .map_or((0, 0, 0, 0), |c| (c.x0.max(x0), c.y0.max(y0), (c.x0 + c.w).min(x1), (c.y0 + c.h).min(y1)));
            let inner = if inner.2 > inner.0 && inner.3 > inner.1 { inner } else { (x0, y0, x0, y0) };
            // outside its box a mask covers nothing, which leaves what is kept alone unless the
            // mask is inverted or intersects
            let mut probe = 0.5;
            step(&mut probe, 0.0);
            if probe != 0.5 {
                for y in y0..y1 {
                    let row = &mut kept[(y - y0) * bw..][..bw];
                    let within = y >= inner.1 && y < inner.3;
                    for (x, k) in row.iter_mut().enumerate() {
                        if !(within && x + x0 >= inner.0 && x + x0 < inner.2) {
                            step(k, 0.0);
                        }
                    }
                }
            }
            if let Some(c) = cov {
                for y in inner.1..inner.3 {
                    let from = &c.row(y)[inner.0 - c.x0..inner.2 - c.x0];
                    let to = &mut kept[(y - y0) * bw + inner.0 - x0..][..inner.2 - inner.0];
                    for (k, c) in to.iter_mut().zip(from) {
                        step(k, *c);
                    }
                }
            }
        }
        Some(Coverage { x0, y0, w: bw, h: bh, stride: bw, data: kept })
    }

    #[allow(clippy::too_many_arguments)]
    fn content(
        &mut self,
        l: &Layer,
        frame: f32,
        m: Affine,
        opacity: f32,
        target: &mut Surface,
        depth: usize,
        clip: Option<&Coverage>,
    ) {
        let boxed = |size: P| -> Vec<P> {
            [[0.0, 0.0], [size[0], 0.0], [size[0], size[1]], [0.0, size[1]]].into_iter().map(|p| m.apply(p)).collect()
        };
        match &l.content {
            Content::Nothing => {}
            Content::Solid { size, colour } => {
                if let Some(cov) = cover(&[boxed(*size)], Rule::NonZero, self.w, self.h).and_then(|c| c.clipped(clip)) {
                    paint(target, &cov, &Paint::Solid(*colour), opacity);
                }
            }
            Content::Image(id) => {
                let Some(picture) = self.anim.pictures.get(id) else { return };
                let (Some(bitmap), Some(back)) = (&picture.bitmap, m.inverse()) else { return };
                let cov = cover(&[boxed(picture.size)], Rule::NonZero, self.w, self.h).and_then(|c| c.clipped(clip));
                if let Some(cov) = cov {
                    let fit = Affine::scale(
                        bitmap.width as f32 / picture.size[0].max(1e-3),
                        bitmap.height as f32 / picture.size[1].max(1e-3),
                    );
                    paint(target, &cov, &Paint::Image { bitmap, to_local: fit.then_after(back) }, opacity);
                }
            }
            Content::Comp { id, size } => {
                let Some(inner) = self.anim.comps.get(id) else { return };
                if depth >= 16 {
                    return;
                }
                let at = match &l.remap {
                    Some(seconds) => seconds.one(frame) * self.anim.fps,
                    None => (frame - l.start) / l.stretch,
                };
                // a composition shows through a window of its own size
                let window = boxed(*size);
                let inside = m.inverse().is_some_and(|back| {
                    [[0.0, 0.0], [self.w as f32, 0.0], [self.w as f32, self.h as f32], [0.0, self.h as f32]]
                        .into_iter()
                        .all(|corner| {
                            let p = back.apply(corner);
                            p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= size[0] && p[1] <= size[1]
                        })
                });
                if inside || size[0] <= 0.0 || size[1] <= 0.0 {
                    return self.comp(inner, at, m, target, depth + 1, clip);
                }
                let Some(window) = cover(&[window], Rule::NonZero, self.w, self.h).and_then(|c| c.clipped(clip)) else {
                    return;
                };
                self.comp(inner, at, m, target, depth + 1, Some(&window));
            }
            Content::Shapes(items) => {
                let mut shapes = Shapes { frame, device: m, paths: Vec::new(), together: Vec::new() };
                let (draws, _) = shapes.walk(items, Affine::IDENTITY, 1.0, &[]);
                shapes.trim_together();
                self.draw(&shapes, &draws, m, opacity, target, clip);
            }
            Content::Text(text) => self.text(text, frame, m, opacity, target, clip),
        }
    }

    /// Paints fills and strokes, the last first.
    fn draw(
        &mut self,
        shapes: &Shapes,
        draws: &[Draw],
        m: Affine,
        opacity: f32,
        target: &mut Surface,
        clip: Option<&Coverage>,
    ) {
        for d in draws.iter().rev() {
            let opacity = opacity * d.opacity;
            if opacity <= 0.0 {
                continue;
            }
            let device = m.then_after(d.to_layer);
            let (polys, rule): (Vec<Vec<P>>, Rule) = match &d.style {
                Style::Fill(rule) => {
                    let polys = d
                        .paths
                        .iter()
                        .flat_map(|id| {
                            let o = &shapes.paths[*id];
                            let to_device = m.then_after(o.to_layer);
                            o.lines.iter().map(move |l| l.map(to_device).pts)
                        })
                        .collect();
                    (polys, *rule)
                }
                Style::Stroke { width, cap, join, miter_limit, dashes, dash_offset } => {
                    // widened in the space the stroke is in, so a squeezed group squeezes its line
                    let Some(back) = d.to_layer.inverse() else { continue };
                    let tolerance = 0.2 / device.stretch().max(1e-3);
                    let mut polys = Vec::new();
                    for id in &d.paths {
                        let o = &shapes.paths[*id];
                        let local = back.then_after(o.to_layer);
                        for line in &o.lines {
                            let line = line.map(local);
                            let pieces = if dashes.is_empty() { vec![line] } else { line.dashed(dashes, *dash_offset) };
                            for piece in &pieces {
                                for poly in outline(piece, *width, *cap, *join, *miter_limit, tolerance) {
                                    polys.push(poly.into_iter().map(|p| device.apply(p)).collect());
                                }
                            }
                        }
                    }
                    (polys, Rule::NonZero)
                }
            };
            let Some(cov) = cover(&polys, rule, self.w, self.h).and_then(|c| c.clipped(clip)) else { continue };
            match &d.ink {
                Ink::Solid(c) => paint(target, &cov, &Paint::Solid(*c), opacity),
                Ink::Gradient { radial, start, end, stops } => {
                    let Some(to_local) = device.inverse() else { continue };
                    let ink = Paint::Gradient { radial: *radial, start: *start, end: *end, stops, to_local };
                    paint(target, &cov, &ink, opacity);
                }
            }
        }
    }

    /// Sets a text layer from the glyph outlines the file carries.
    fn text(
        &mut self,
        text: &Text,
        frame: f32,
        m: Affine,
        opacity: f32,
        target: &mut Surface,
        clip: Option<&Coverage>,
    ) {
        let family = self.anim.families.get(&text.font).cloned().unwrap_or_else(|| text.font.clone());
        let mut shapes = Shapes { frame, device: m, paths: Vec::new(), together: Vec::new() };
        let scale = text.size / 100.0;
        let mut paths = Vec::new();
        for (row, line) in text.text.split(['\r', '\n', '\u{3}']).enumerate() {
            let glyphs: Vec<_> = line.chars().map(|c| self.anim.glyph(c, &family)).collect();
            let advance = |g: &Option<&crate::model::Glyph>| {
                g.map_or(0.0, |g| g.advance) * scale + text.tracking * text.size / 1000.0
            };
            let width: f32 = glyphs.iter().map(advance).sum();
            let mut x = match text.justify {
                1 => -width,
                2 => -width / 2.0,
                _ => 0.0,
            };
            for g in &glyphs {
                if let Some(glyph) = g {
                    let place =
                        Affine::translate(x, row as f32 * text.line_height).then_after(Affine::scale(scale, scale));
                    paths.extend(shapes.walk(&glyph.outline, place, 1.0, &[]).1);
                }
                x += advance(g);
            }
        }
        // the first is drawn over the ones after it
        let mut draws = Vec::new();
        if let Some(c) = text.fill {
            draws.push(Draw {
                ink: Ink::Solid(c),
                style: Style::Fill(Rule::NonZero),
                opacity: 1.0,
                paths: paths.clone(),
                to_layer: Affine::IDENTITY,
            });
        }
        if let Some((c, width)) = text.stroke.filter(|s| s.1 > 0.0) {
            let outline = Draw {
                ink: Ink::Solid(c),
                style: Style::Stroke {
                    width,
                    cap: Cap::Butt,
                    join: Join::Round,
                    miter_limit: 4.0,
                    dashes: Vec::new(),
                    dash_offset: 0.0,
                },
                opacity: 1.0,
                paths,
                to_layer: Affine::IDENTITY,
            };
            draws.insert(if text.stroke_over { 0 } else { draws.len() }, outline);
        }
        self.draw(&shapes, &draws, m, opacity, target, clip);
    }
}

/// Whether a shape layer's contents are one fill or stroke, which fades the same drawn straight
/// onto the picture as drawn apart first.
fn simple(items: &[Item]) -> bool {
    fn count(items: &[Item]) -> usize {
        items
            .iter()
            .map(|i| match i {
                Item::Group(inner) => count(inner),
                Item::Fill { .. } | Item::Stroke { .. } => 1,
                Item::Repeater(_) => 2,
                _ => 0,
            })
            .sum()
    }
    count(items) <= 1
}
