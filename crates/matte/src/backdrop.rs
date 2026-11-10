//! Cutting a thing out of a picture taken against a plain backdrop (a product on a studio
//! sweep, a white table, a wall of one colour).
//!
//! No model looks at the picture here. The backdrop is what the picture's edges show: a smooth
//! surface of colour is fitted to them (so a sweep that is lit from above, or darker towards its
//! corners, is still one backdrop), everything that is that colour and reached from the edges
//! without crossing something else goes, and what is left is the thing. A white label on the
//! thing is not reached from the edges and stays; the backdrop seen through a handle is not
//! reached either, and goes only if it is the backdrop's colour very nearly (`holes`).
//!
//! A thing standing on a backdrop shades it. The backdrop in shade is still the backdrop: where
//! the backdrop's own colour only darker is reached from the edges without a sharp step (a soft
//! shadow has none, the thing's outline is one), it goes too. A shadow too deep to be let go at
//! the tolerance asked for is kept as what it is, darkness and nothing else: see-through black,
//! which shades whatever the thing is put before as it shaded the backdrop.
//!
//! Along the outline a pixel is part thing and part backdrop. How much of each is worked out from
//! the colour of the thing just inside and the backdrop's there, and the backdrop's share is
//! taken out of the pixel's colour, so the cut-out has no rim of the old backdrop around it.

use rayon::prelude::*;

/// How a backdrop is told from what stands before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    /// How far from the backdrop's colour still counts as backdrop, 0 to 1: 0.5 suits a clean
    /// studio picture, more takes its soft shadows and creases too (and pale parts of the thing
    /// that touch the backdrop), less keeps more.
    pub tolerance: f32,
    /// How readily backdrop-coloured places that the edges do not reach go (the inside of a
    /// handle), 0 to 1: at 0 none do, at 1 they go as the backdrop outside does.
    pub holes: f32,
}

impl Default for Backdrop {
    fn default() -> Self {
        Backdrop { tolerance: 0.5, holes: 0.5 }
    }
}

/// A picture with its backdrop gone.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// The picture's pixels again: red, green, blue and alpha, linear light, the colours
    /// multiplied by the alpha.
    pub pixels: Vec<[f32; 4]>,
    /// The share of the picture's edge that is backdrop, 0 to 1. Little of it means the picture
    /// has no plain backdrop around its subject, and little was cut.
    pub edge: f32,
    /// The share of the picture that stays, 0 to 1.
    pub kept: f32,
}

/// The longer side of the copy the backdrop is found on.
const SIDE: usize = 320;

/// A colour as lightness and two of colourfulness (BT.601).
fn opposed(c: [f32; 3]) -> [f32; 3] {
    let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    [y, 0.564 * (c[2] - y), 0.713 * (c[0] - y)]
}

/// How far colour `c` is from the backdrop's colour `b` there (both sRGB-encoded). Darker counts
/// for less than lighter or another hue: a backdrop is shaded by what stands on it.
fn far(c: [f32; 3], b: [f32; 3]) -> f32 {
    let (c, b) = (opposed(c), opposed(b));
    let light = c[0] - b[0];
    let light = if light < 0.0 { light * 0.6 } else { light };
    let (u, v) = (2.0 * (c[1] - b[1]), 2.0 * (c[2] - b[2]));
    (light * light + u * u + v * v).sqrt()
}

/// sRGB-encoded to linear light.
fn decode(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// A shadow darker than this share of the backdrop's light is not followed: it may as well be a
/// dark thing.
const DEEP: f32 = 0.15;
/// A step in shade steeper than this from one small pixel to the next is an outline, not a
/// shadow's soft edge.
const STEEP: f32 = 0.1;

/// A colour (as on screen and in linear light) taken as the backdrop's colour there in shade:
/// the share of the backdrop's light it has, 0 to 1, and how far it is from the backdrop's
/// colour that much darker.
fn shaded(seen: [f32; 3], light: [f32; 3], behind: [f32; 3], encode: &(impl Fn(f32) -> f32 + Sync)) -> (f32, f32) {
    let b = behind.map(decode);
    let both: f32 = (0..3).map(|k| light[k] * b[k]).sum();
    let whole: f32 = (0..3).map(|k| b[k] * b[k]).sum();
    let share = (both / whole.max(1e-6)).clamp(0.0, 1.0);
    (share, far(seen, b.map(|v| encode(v * share))))
}

/// How much of a shadow's darkness is kept where the backdrop has `share` of its light, when
/// shadows down to `floor` of it are let go: none above that, all of it a little below.
fn darkness(share: f32, floor: f32) -> f32 {
    (1.0 - share) * (1.0 - smoothstep(floor - 0.2, floor, share))
}

/// A surface of colour over a picture: each of red, green and blue a polynomial of the second
/// degree in the place, which is counted from the picture's middle in shares of its sides.
#[derive(Clone, Copy, Debug)]
struct Surface([[f64; 6]; 3]);

impl Surface {
    fn flat(c: [f32; 3]) -> Surface {
        Surface(c.map(|v| [f64::from(v), 0.0, 0.0, 0.0, 0.0, 0.0]))
    }

    fn terms(u: f32, v: f32) -> [f64; 6] {
        let (u, v) = (f64::from(u), f64::from(v));
        [1.0, u, v, u * u, u * v, v * v]
    }

    fn at(&self, u: f32, v: f32) -> [f32; 3] {
        let t = Self::terms(u, v);
        self.0.map(|c| c.iter().zip(&t).map(|(a, b)| a * b).sum::<f64>().clamp(0.0, 1.0) as f32)
    }

    /// The surface nearest the colours at these places (least squares), if they are enough to
    /// say.
    fn through(samples: impl Iterator<Item = (f32, f32, [f32; 3])>) -> Option<Surface> {
        let mut m = [[0.0f64; 6]; 6];
        let mut rhs = [[0.0f64; 6]; 3];
        let mut n = 0usize;
        for (u, v, c) in samples {
            let t = Self::terms(u, v);
            for i in 0..6 {
                for j in 0..6 {
                    m[i][j] += t[i] * t[j];
                }
                for (k, r) in rhs.iter_mut().enumerate() {
                    r[i] += t[i] * f64::from(c[k]);
                }
            }
            n += 1;
        }
        if n < 24 {
            return None;
        }
        // places all on one line say nothing about the bend across it: lean towards flat
        for (i, row) in m.iter_mut().enumerate().skip(1) {
            row[i] += 1e-4 * n as f64;
        }
        let mut out = [[0.0f64; 6]; 3];
        for (k, r) in rhs.iter().enumerate() {
            out[k] = solve(m, *r)?;
        }
        Some(Surface(out))
    }
}

/// Solves `m x = r` by elimination.
fn solve(mut m: [[f64; 6]; 6], mut r: [f64; 6]) -> Option<[f64; 6]> {
    for i in 0..6 {
        let p = (i..6).max_by(|a, b| m[*a][i].abs().total_cmp(&m[*b][i].abs()))?;
        if m[p][i].abs() < 1e-12 {
            return None;
        }
        m.swap(i, p);
        r.swap(i, p);
        let (top, ri) = (m[i], r[i]);
        for (row, rj) in m.iter_mut().zip(r.iter_mut()).skip(i + 1) {
            let f = row[i] / top[i];
            for (a, b) in row.iter_mut().zip(top).skip(i) {
                *a -= f * b;
            }
            *rj -= f * ri;
        }
    }
    let mut x = [0.0f64; 6];
    for i in (0..6).rev() {
        let rest: f64 = (i + 1..6).map(|k| m[i][k] * x[k]).sum();
        x[i] = (r[i] - rest) / m[i][i];
    }
    Some(x)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What was found on the small copy: the backdrop's colours, how near them counts, and which
/// small pixels the edges reach through backdrop.
struct Found {
    w: usize,
    h: usize,
    surface: Surface,
    /// Up to the first it is backdrop, from the second it is not.
    near: (f32, f32),
    reached: Vec<bool>,
    /// The share of the backdrop's light each small pixel has, taken as backdrop in shade.
    lit: Vec<f32>,
    edge: f32,
}

fn find(width: usize, height: usize, rgb: &[[f32; 3]], how: Backdrop) -> Option<Found> {
    let k = (SIDE as f32 / width.max(height) as f32).min(1.0);
    let (w, h) = (((width as f32 * k).round() as usize).max(1), ((height as f32 * k).round() as usize).max(1));
    let small = if (w, h) == (width, height) { rgb.to_vec() } else { super::sized(width, height, rgb, w, h) };
    let place = |i: usize| (((i % w) as f32 + 0.5) / w as f32 - 0.5, ((i / w) as f32 + 0.5) / h as f32 - 0.5);
    let ring = ((w.min(h) as f32 * 0.04).round() as usize).max(2).min(w.min(h) / 2);
    let edge: Vec<usize> = (0..w * h)
        .filter(|i| {
            let (x, y) = (i % w, i / w);
            x < ring || y < ring || x >= w - ring || y >= h - ring
        })
        .collect();
    if edge.len() < 24 {
        return None;
    }
    // the colour most of the edge has, then the surface through the edge that is near it,
    // again and again: what stands at the edge (a hand, a table's corner) is left out
    let middle = |c: usize| {
        let mut v: Vec<f32> = edge.iter().map(|i| small[*i][c]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let mut surface = Surface::flat([middle(0), middle(1), middle(2)]);
    let mut reach = 0.12;
    for _ in 0..4 {
        let at = |i: &&usize| {
            let (u, v) = place(**i);
            far(small[**i], surface.at(u, v)) < reach
        };
        let fitted = Surface::through(edge.iter().filter(at).map(|i| {
            let (u, v) = place(*i);
            (u, v, small[*i])
        }));
        match fitted {
            Some(s) => surface = s,
            None => break,
        }
        reach = 0.08;
    }
    let off = |i: usize| {
        let (u, v) = place(i);
        far(small[i], surface.at(u, v))
    };
    let mut offs: Vec<f32> = edge.iter().map(|i| off(*i)).filter(|d| *d < reach).collect();
    let share = offs.len() as f32 / edge.len() as f32;
    if offs.len() < 24 {
        return None;
    }
    offs.sort_by(f32::total_cmp);
    // how much the backdrop itself varies about its surface (grain, the file's compression)
    let grain = (offs[offs.len() / 2] * 1.5).clamp(0.004, 0.05);
    let low = (4.0 * grain).max(0.03) * (how.tolerance.clamp(0.0, 1.0) / 0.5);
    let near = (low, low * 1.8 + 0.01);
    let encode = super::encoder();
    let light: Vec<[f32; 3]> = small.iter().map(|c| c.map(decode)).collect();
    let mut reached = vec![false; w * h];
    // for every small pixel: how far it is from the backdrop there, and taken as the backdrop
    // in shade, how much of its light it has and how far it is from that
    let mut cells: Vec<(f32, f32, f32)> = Vec::new();
    for pass in 0..2 {
        cells = (0..w * h)
            .map(|i| {
                let (u, v) = place(i);
                let behind = surface.at(u, v);
                let (lit, miss) = shaded(small[i], light[i], behind, &encode);
                (far(small[i], behind), lit, miss)
            })
            .collect();
        // from the edge inwards through what is near the backdrop's colour, or is that colour
        // in a shade that deepens gently
        reached.fill(false);
        let mut next: Vec<usize> = edge.iter().copied().filter(|i| cells[*i].0 < near.1).collect();
        for i in &next {
            reached[*i] = true;
        }
        while let Some(i) = next.pop() {
            let (x, y) = (i % w, i / w);
            let around =
                [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)];
            for (inside, j) in around {
                if !inside || reached[j] {
                    continue;
                }
                let (off, lit, miss) = cells[j];
                if off < near.1 || (miss < near.1 && lit >= DEEP && (cells[i].1 - lit).abs() < STEEP) {
                    reached[j] = true;
                    next.push(j);
                }
            }
        }
        if pass == 0 {
            // the backdrop is now known wherever it shows, not only at the edge: fit it there
            let sure: Vec<usize> = (0..w * h).filter(|i| reached[*i] && cells[*i].0 < near.0.max(0.03)).collect();
            if let Some(s) = Surface::through(sure.iter().map(|i| {
                let (u, v) = place(*i);
                (u, v, small[*i])
            })) {
                surface = s;
            }
        }
    }
    // places the edges do not reach that are the backdrop's colour all the same (seen through a
    // handle): each goes whole or stays whole, by how near the backdrop's colour it is in all
    let holes = how.holes.clamp(0.0, 1.0);
    let mut seen = reached.clone();
    for start in 0..w * h {
        if seen[start] || cells[start].0 >= near.1 {
            continue;
        }
        let mut place = vec![start];
        seen[start] = true;
        let mut at = 0;
        while at < place.len() {
            let i = place[at];
            at += 1;
            let (x, y) = (i % w, i / w);
            let around =
                [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)];
            for (inside, j) in around {
                if inside && !seen[j] && cells[j].0 < near.1 {
                    seen[j] = true;
                    place.push(j);
                }
            }
        }
        let mean = place.iter().map(|i| cells[*i].0).sum::<f32>() / place.len() as f32;
        if place.len() >= 4 && mean < near.1 * holes {
            for i in place {
                reached[i] = true;
            }
        }
    }
    let lit = cells.iter().map(|c| c.1).collect();
    Some(Found { w, h, surface, near, reached, lit, edge: share })
}

/// The least of `v` (`w × h`) within `r` each way of every value.
fn least(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut rows = vec![0.0f32; w * h];
    rows.par_chunks_mut(w).zip(v.par_chunks(w)).for_each(|(out, row)| {
        for (x, o) in out.iter_mut().enumerate() {
            *o = row[x.saturating_sub(r)..(x + r + 1).min(w)].iter().copied().fold(f32::INFINITY, f32::min);
        }
    });
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
        for (x, o) in o.iter_mut().enumerate() {
            *o = (y.saturating_sub(r)..(y + r + 1).min(h)).map(|y| rows[y * w + x]).fold(f32::INFINITY, f32::min);
        }
    });
    out
}

/// Takes the specks out of `stays` (how much of each pixel stays, `w` across): next to a sharp
/// outline a compressed picture rings, and bits of backdrop there are off its colour. A group of
/// staying pixels that touches nothing that is surely the thing (`inside` at 1) and is smaller
/// than `least` pixels goes, and so does a faint pixel with nothing staying next to it.
fn specks(stays: &mut [f32], inside: &[f32], w: usize, least: usize) {
    const FAINT: f32 = 0.25;
    let h = stays.len() / w;
    let around = |i: usize| {
        let (x, y) = (i % w, i / w);
        [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)]
    };
    let mut seen = vec![false; stays.len()];
    let mut group = Vec::new();
    for start in 0..stays.len() {
        if seen[start] || inside[start] >= 1.0 || stays[start] <= FAINT {
            continue;
        }
        group.clear();
        group.push(start);
        seen[start] = true;
        let (mut at, mut held) = (0, false);
        while at < group.len() {
            let i = group[at];
            at += 1;
            for (within, j) in around(i) {
                if !within {
                    continue;
                }
                if inside[j] >= 1.0 {
                    held = true;
                } else if !seen[j] && stays[j] > FAINT {
                    seen[j] = true;
                    group.push(j);
                }
            }
        }
        if !held && group.len() < least {
            for i in &group {
                stays[*i] = 0.0;
            }
        }
    }
    for i in 0..stays.len() {
        if stays[i] > 0.0 && stays[i] <= FAINT {
            let (x, y) = (i % w, i / w);
            let near = (y.saturating_sub(1)..(y + 2).min(h))
                .any(|y| (x.saturating_sub(1)..(x + 2).min(w)).any(|x| stays[y * w + x] > FAINT));
            if !near {
                stays[i] = 0.0;
            }
        }
    }
}

/// A picture with its plain backdrop made see-through. `rgba` is `width * height` pixels as a
/// compositor keeps them: red, green, blue and alpha, linear light, the colours multiplied by
/// the alpha. A picture with no backdrop to be found around its edges comes back as it is.
pub fn backdrop_linear(width: usize, height: usize, rgba: &[[f32; 4]], how: Backdrop) -> Result<Cut, String> {
    if width == 0 || height == 0 || rgba.len() != width * height {
        return Err("a picture needs a size and that many pixels".into());
    }
    let encode = super::encoder();
    // the colours themselves, linear and as on screen
    let mut straight = vec![[0.0f32; 3]; rgba.len()];
    let mut rgb = vec![[0.0f32; 3]; rgba.len()];
    straight.par_chunks_mut(width).zip(rgb.par_chunks_mut(width)).zip(rgba.par_chunks(width)).for_each(
        |((lin, enc), pixels)| {
            for ((lin, enc), [r, g, b, a]) in lin.iter_mut().zip(enc.iter_mut()).zip(pixels) {
                let alpha = if *a > 1e-4 { 1.0 / a } else { 0.0 };
                *lin = [r * alpha, g * alpha, b * alpha];
                *enc = lin.map(&encode);
            }
        },
    );
    let Some(found) = Some(how).filter(|h| h.tolerance > 0.0).and_then(|how| find(width, height, &rgb, how)) else {
        return Ok(Cut { pixels: rgba.to_vec(), edge: 0.0, kept: 1.0 });
    };
    let place = |x: usize, y: usize| ((x as f32 + 0.5) / width as f32 - 0.5, (y as f32 + 0.5) / height as f32 - 0.5);
    // shadows on the backdrop down to this share of its light are let go
    let floor = 1.0 - 0.9 * how.tolerance.clamp(0.0, 1.0);
    // how much of each pixel stays by its own colour alone, and for what is backdrop, how much of
    // the backdrop's light it has
    let mut stays = vec![0.0f32; width * height];
    let mut lit = vec![1.0f32; width * height];
    stays.par_chunks_mut(width).zip(lit.par_chunks_mut(width)).enumerate().for_each(|(y, (row, lights))| {
        let sy = (y * found.h / height).min(found.h - 1);
        for (x, (s, l)) in row.iter_mut().zip(lights.iter_mut()).enumerate() {
            let sx = (x * found.w / width).min(found.w - 1);
            let reached = found.reached[sy * found.w + sx];
            // a small pixel on the outline is not reached, and has backdrop in it all the same:
            // in the shade the reached ones beside it are in
            let mut beside = (0.0f32, 0usize);
            if !reached {
                for y in sy.saturating_sub(1)..(sy + 2).min(found.h) {
                    for x in sx.saturating_sub(1)..(sx + 2).min(found.w) {
                        if found.reached[y * found.w + x] {
                            beside = (beside.0 + found.lit[y * found.w + x], beside.1 + 1);
                        }
                    }
                }
                if beside.1 == 0 {
                    *s = 1.0;
                    continue;
                }
            }
            let (u, v) = place(x, y);
            let i = y * width + x;
            let behind = found.surface.at(u, v);
            // shade is followed only where the edges reached through it
            let (share, miss) = shaded(rgb[i], straight[i], behind, &encode);
            let in_shade = share >= DEEP && (reached || (share - beside.0 / beside.1 as f32).abs() < 1.5 * STEEP);
            *s = smoothstep(found.near.0, found.near.1, if in_shade { miss } else { far(rgb[i], behind) });
            if in_shade {
                *l = share;
            }
        }
    });
    // along the outline: the thing just inside says what colour it is there
    let r = ((width.min(height) as f32 / 500.0).round() as usize).max(2);
    let inside = least(&stays, width, height, r);
    specks(&mut stays, &inside, width, (2 * r + 1) * (2 * r + 1));
    let mut pixels = vec![[0.0f32; 4]; width * height];
    pixels.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * width + x;
            let [pr, pg, pb, pa] = rgba[i];
            if inside[i] >= 1.0 {
                *out = [pr, pg, pb, pa];
                continue;
            }
            if stays[i] <= 0.0 {
                // backdrop: gone, but for the darkness of a shadow too deep to let go
                *out = [0.0, 0.0, 0.0, darkness(lit[i], floor) * pa];
                continue;
            }
            let (u, v) = place(x, y);
            let c = straight[i];
            // the thing's colour: what is surely the thing nearby, the nearest counting most;
            // and the backdrop's shade there: that of what is surely backdrop nearby
            let reach = 2 * r as isize;
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            let (mut shade, mut shades) = (0.0f32, 0.0f32);
            for dy in -reach..=reach {
                let yy = y as isize + dy;
                if yy < 0 || yy >= height as isize {
                    continue;
                }
                for dx in -reach..=reach {
                    let xx = x as isize + dx;
                    if xx < 0 || xx >= width as isize {
                        continue;
                    }
                    let j = yy as usize * width + xx as usize;
                    let k = 1.0 / (dx * dx + dy * dy).max(1) as f32;
                    if inside[j] >= 1.0 {
                        let f = straight[j];
                        sum = [sum[0] + f[0] * k, sum[1] + f[1] * k, sum[2] + f[2] * k];
                        weight += k;
                    } else if stays[j] <= 0.0 {
                        shade += lit[j] * k;
                        shades += k;
                    }
                }
            }
            let shade = if shades > 0.0 { shade / shades } else { lit[i] };
            let behind = found.surface.at(u, v).map(|v| decode(v) * shade);
            let mut share = stays[i];
            if weight > 0.0 {
                let thing = sum.map(|v| v / weight);
                // a thing the backdrop's colour says nothing about the mix
                if far(thing.map(&encode), behind.map(&encode)) > 2.0 * found.near.1 {
                    let along: f32 = (0..3).map(|k| (c[k] - behind[k]) * (thing[k] - behind[k])).sum();
                    let whole: f32 = (0..3).map(|k| (thing[k] - behind[k]).powi(2)).sum();
                    let mixed = (along / whole.max(1e-9)).clamp(0.0, 1.0);
                    // only if the pixel is that mix: a dark seam on a pale thing is not
                    let made: [f32; 3] = std::array::from_fn(|k| behind[k] + (thing[k] - behind[k]) * mixed);
                    let (a, b) = (made.map(&encode), c.map(&encode));
                    let miss = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
                    if miss < 0.06 {
                        share = mixed;
                    }
                }
            }
            // the backdrop's share taken out of the colour; of its shadow the darkness stays
            let own: [f32; 3] = std::array::from_fn(|k| (c[k] - (1.0 - share) * behind[k]).clamp(0.0, share));
            let alpha = share + (1.0 - share) * darkness(shade, floor);
            *out = [own[0] * pa, own[1] * pa, own[2] * pa, alpha * pa];
        }
    });
    let (was, is): (f64, f64) = rgba
        .par_iter()
        .zip(pixels.par_iter())
        .map(|(a, b)| (f64::from(a[3]), f64::from(b[3])))
        .reduce(|| (0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
    Ok(Cut { pixels, edge: found.edge, kept: if was > 0.0 { (is / was) as f32 } else { 0.0 } })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A studio picture drawn by hand: a sweep lit from above and darker in its corners, a blue
    /// flask with a white label on it, and a ring (a handle) the sweep shows through. Under the
    /// flask's foot the sweep is in a soft shadow that takes `shade` of its light at the most,
    /// and with `bits` two orange squares lie on the sweep, of 2 and of 14 pixels. Each pixel is
    /// the mean of 4 × 4 places in it, as a camera's is.
    fn studio(w: usize, h: usize, shade: f32, bits: bool) -> Vec<[f32; 4]> {
        let sweep = |u: f32, v: f32| {
            let l = 0.86 - 0.18 * v - 0.25 * (u * u + v * v);
            let lit = 1.0 - shade * (-(u / 0.2).powi(2) - ((v - 0.3) / 0.06).powi(2)).exp();
            [l, l * 0.99, l * 0.96].map(|c| decode(c) * lit)
        };
        let at = |x: f32, y: f32| -> [f32; 3] {
            let (u, v) = (x / w as f32 - 0.5, y / h as f32 - 0.5);
            let flask = u.abs() < 0.16 && v.abs() < 0.3;
            let label = u.abs() < 0.1 && (v - 0.05).abs() < 0.08;
            let ring = ((u - 0.27).powi(2) + v.powi(2)).sqrt();
            let bit = bits
                && (((275.0..277.0).contains(&x) && (60.0..62.0).contains(&y))
                    || ((330.0..344.0).contains(&x) && (40.0..54.0).contains(&y)));
            if label {
                [0.97, 0.97, 0.97].map(decode)
            } else if flask || (0.07..0.11).contains(&ring) {
                [0.1, 0.25, 0.7].map(decode)
            } else if bit {
                [0.9, 0.5, 0.1].map(decode)
            } else {
                sweep(u, v)
            }
        };
        (0..w * h)
            .map(|i| {
                let mut sum = [0.0f32; 3];
                for s in 0..16 {
                    let c = at(
                        (i % w) as f32 + (s % 4) as f32 / 4.0 + 0.125,
                        (i / w) as f32 + (s / 4) as f32 / 4.0 + 0.125,
                    );
                    for k in 0..3 {
                        sum[k] += c[k] / 16.0;
                    }
                }
                [sum[0], sum[1], sum[2], 1.0]
            })
            .collect()
    }

    #[test]
    fn the_backdrop_in_shade_goes_too_and_a_deep_shadow_stays_as_darkness() {
        let (w, h) = (400usize, 300usize);
        let at = |cut: &Cut, u: f32, v: f32| {
            cut.pixels[((v + 0.5) * h as f32) as usize * w + ((u + 0.5) * w as f32) as usize]
        };
        // a light shadow under the flask's foot: the sweep there is the sweep, and goes
        let cut = backdrop_linear(w, h, &studio(w, h, 0.35, false), Backdrop::default()).expect("cut");
        for (u, v) in [(0.0, 0.33), (0.1, 0.32), (-0.15, 0.36), (0.0, 0.42)] {
            assert_eq!(at(&cut, u, v), [0.0; 4], "{u} {v}");
        }
        assert_eq!(at(&cut, 0.0, 0.25)[3], 1.0);
        let kept = cut.pixels.iter().filter(|p| p[3] > 0.5).count() as f32 / (w * h) as f32;
        assert!((kept - 0.2146).abs() < 0.005, "{kept}");
        // a deep one: its darkness stays, as black that is partly there, and thins out to
        // nothing where the shadow is light; no pale rim of sweep anywhere
        let cut = backdrop_linear(w, h, &studio(w, h, 0.8, false), Backdrop::default()).expect("cut");
        let deep = at(&cut, 0.0, 0.33);
        assert!(deep[..3] == [0.0; 3] && (0.4..0.65).contains(&deep[3]), "{deep:?}");
        let (thin, none) = (at(&cut, 0.0, 0.34), at(&cut, 0.0, 0.44));
        assert!(thin[3] > 0.0 && thin[3] < deep[3] && none == [0.0; 4], "{thin:?} {none:?}");
        for p in cut.pixels.iter().filter(|p| p[3] > 0.0 && p[3] < 0.9) {
            assert!(p[0] < 0.12 * p[3] + 1e-4, "{p:?}");
        }
        // asked to let more go, all of it goes; asked to keep more, the light one stays too
        let all =
            backdrop_linear(w, h, &studio(w, h, 0.8, false), Backdrop { tolerance: 1.0, holes: 0.5 }).expect("cut");
        assert_eq!(at(&all, 0.0, 0.33), [0.0; 4]);
        let few =
            backdrop_linear(w, h, &studio(w, h, 0.35, false), Backdrop { tolerance: 0.2, holes: 0.5 }).expect("cut");
        let light = at(&few, 0.0, 0.33);
        assert!(light[..3] == [0.0; 3] && (0.03..0.3).contains(&light[3]), "{light:?}");

        // a speck beside the outline goes; a thing of its own, however small, stays
        let cut = backdrop_linear(w, h, &studio(w, h, 0.0, true), Backdrop::default()).expect("cut");
        assert_eq!(cut.pixels[61 * w + 276], [0.0; 4]);
        assert_eq!(cut.pixels[47 * w + 337][3], 1.0);
    }

    #[test]
    fn a_plain_backdrop_goes_and_what_stands_before_it_stays() {
        let (w, h) = (400usize, 300usize);
        let picture = studio(w, h, 0.0, false);
        let cut = backdrop_linear(w, h, &picture, Backdrop::default()).expect("cut");
        let at = |u: f32, v: f32| cut.pixels[((v + 0.5) * h as f32) as usize * w + ((u + 0.5) * w as f32) as usize];
        // the sweep, light at the top and dark in a corner, is gone; the edge was all sweep
        for (u, v) in [(-0.4, -0.4), (0.0, -0.45), (0.45, 0.45), (-0.3, 0.2), (0.2, 0.4)] {
            assert_eq!(at(u, v), [0.0; 4], "{u} {v}");
        }
        assert!(cut.edge > 0.95, "{}", cut.edge);
        // the flask is whole and its colour its own; so is the label, white as it is
        assert_eq!(at(0.0, -0.2), picture[(0.3 * h as f32) as usize * w + w / 2]);
        assert_eq!(at(0.0, 0.05)[3], 1.0);
        assert_eq!(at(0.27, 0.09)[3], 1.0);
        // the sweep inside the ring is the sweep's colour to the grain: it goes
        assert_eq!(at(0.27, 0.0)[3], 0.0);
        let kept = cut.pixels.iter().filter(|p| p[3] > 0.5).count() as f32 / (w * h) as f32;
        let ring = std::f32::consts::PI * (0.11f32.powi(2) - 0.07f32.powi(2));
        assert!((kept - (0.32 * 0.6 + ring)).abs() < 0.005, "{kept}");
        assert!((cut.kept - kept).abs() < 0.01);
        // along the outline a pixel is partly there, and what is there of it is the flask's
        // blue, not blue washed with the sweep
        let mut partly = 0;
        for (i, p) in cut.pixels.iter().enumerate().filter(|(_, p)| p[3] > 0.2 && p[3] < 0.8) {
            let own = [p[0] / p[3], p[1] / p[3], p[2] / p[3]];
            assert!(own[2] > 2.0 * own[0] && own[0] < 0.15, "{own:?} at {} {}: {p:?}", i % w, i / w);
            partly += 1;
        }
        assert!(partly > 200, "{partly}");

        // holes left alone: the ring keeps the sweep it surrounds, and nothing else changes
        let whole = backdrop_linear(w, h, &picture, Backdrop { holes: 0.0, ..Backdrop::default() }).expect("cut");
        let i = h / 2 * w + (0.77 * w as f32) as usize;
        assert_eq!(whole.pixels[i], picture[i]);
        assert_eq!(whole.pixels[0], [0.0; 4]);
        // no tolerance: nothing goes
        let all = backdrop_linear(w, h, &picture, Backdrop { tolerance: 0.0, holes: 0.0 }).expect("cut");
        assert_eq!((all.kept, &all.pixels), (1.0, &picture));
    }

    #[test]
    fn a_picture_with_no_backdrop_at_its_edges_is_left_as_it_is() {
        // stripes of unrelated colours to every edge
        let (w, h) = (120usize, 90usize);
        let picture: Vec<[f32; 4]> = (0..w * h)
            .map(|i| match (i % w / 7 + i / w / 5) % 4 {
                0 => [0.8, 0.1, 0.1, 1.0],
                1 => [0.1, 0.6, 0.2, 1.0],
                2 => [0.05, 0.1, 0.7, 1.0],
                _ => [0.9, 0.8, 0.1, 1.0],
            })
            .collect();
        let cut = backdrop_linear(w, h, &picture, Backdrop::default()).expect("cut");
        assert!(cut.edge < 0.5 && cut.kept > 0.7, "{} {}", cut.edge, cut.kept);
        // a picture that is see-through already stays so where it was
        let mut flat = vec![[0.5, 0.5, 0.5, 1.0]; 64 * 64];
        for (i, p) in flat.iter_mut().enumerate() {
            if (20..44).contains(&(i % 64)) && (20..44).contains(&(i / 64)) {
                *p = [0.3, 0.0, 0.0, 0.5];
            }
        }
        let cut = backdrop_linear(64, 64, &flat, Backdrop::default()).expect("cut");
        assert_eq!(cut.pixels[32 * 64 + 32], [0.3, 0.0, 0.0, 0.5]);
        assert_eq!(cut.pixels[0], [0.0; 4]);
        assert!(backdrop_linear(0, 4, &[], Backdrop::default()).is_err());
        assert!(backdrop_linear(2, 2, &flat[..3], Backdrop::default()).is_err());
    }
}
