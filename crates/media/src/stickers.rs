//! The built-in sticker library: shapes drawn from distance functions, so they need no files, no
//! downloads and no licences. A project refers to one as `library:sticker/<id>`, an image like
//! any other; [`Footage`] draws it when it is first shown, the same picture everywhere.
//!
//! Every sticker is a few flat-coloured parts with a white border and a faint dark rim around the
//! lot, the way printed stickers are cut, so it reads on any footage.
//!
//! [`Footage`]: crate::Footage

/// The URI prefix of library stickers (within [`crate::library::SCHEME`]).
pub const PREFIX: &str = "library:sticker/";

/// The side of a sticker's picture, in pixels.
pub const SIZE: u32 = 512;

/// A library sticker.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LibrarySticker {
    /// The id commands use.
    pub id: &'static str,
    /// English name.
    pub en: &'static str,
    /// Chinese name.
    pub zh: &'static str,
}

const fn sticker(id: &'static str, en: &'static str, zh: &'static str) -> LibrarySticker {
    LibrarySticker { id, en, zh }
}

/// Every sticker, in panel order.
pub const STICKERS: [LibrarySticker; 20] = [
    sticker("heart", "Heart", "爱心"),
    sticker("star", "Star", "星星"),
    sticker("sparkle", "Sparkle", "闪光"),
    sticker("burst", "Burst", "爆炸"),
    sticker("arrow", "Arrow", "箭头"),
    sticker("ring", "Ring", "圈重点"),
    sticker("check", "Check", "对勾"),
    sticker("cross", "Cross", "叉"),
    sticker("exclaim", "Exclamation", "感叹号"),
    sticker("bubble", "Speech bubble", "对话气泡"),
    sticker("smile", "Smile", "笑脸"),
    sticker("bolt", "Lightning", "闪电"),
    sticker("sun", "Sun", "太阳"),
    sticker("moon", "Moon", "月亮"),
    sticker("cloud", "Cloud", "云"),
    sticker("drop", "Drop", "水滴"),
    sticker("flower", "Flower", "花"),
    sticker("note", "Music note", "音符"),
    sticker("crown", "Crown", "皇冠"),
    sticker("ribbon", "Ribbon", "彩带"),
];

/// The sticker with this id.
pub fn find(id: &str) -> Option<&'static LibrarySticker> {
    STICKERS.iter().find(|s| s.id == id)
}

/// The id in a `library:sticker/` URI.
pub fn id_of(uri: &str) -> Option<&str> {
    uri.strip_prefix(PREFIX).filter(|id| find(id).is_some())
}

type P = [f32; 2];
/// A signed distance: negative inside, in units where the picture spans -1 to 1.
type Sdf = Box<dyn Fn(P) -> f32 + Send + Sync>;

fn circle(c: P, r: f32) -> Sdf {
    Box::new(move |p| (p[0] - c[0]).hypot(p[1] - c[1]) - r)
}

/// A box with rounded corners: centre, half size, corner radius.
fn rbox(c: P, half: P, r: f32) -> Sdf {
    Box::new(move |p| {
        let q = [(p[0] - c[0]).abs() - half[0] + r, (p[1] - c[1]).abs() - half[1] + r];
        q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r
    })
}

/// A line from `a` to `b`, `r` thick each side, round at the ends.
fn stroke(a: P, b: P, r: f32) -> Sdf {
    Box::new(move |p| {
        let (pa, ba) = ([p[0] - a[0], p[1] - a[1]], [b[0] - a[0], b[1] - a[1]]);
        let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1])).clamp(0.0, 1.0);
        (pa[0] - ba[0] * h).hypot(pa[1] - ba[1] * h) - r
    })
}

/// A closed polygon.
fn poly(pts: Vec<P>) -> Sdf {
    Box::new(move |p| {
        let n = pts.len();
        let mut d = f32::INFINITY;
        let mut inside = false;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + n - 1) % n]);
            let (e, w) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
            let h = ((w[0] * e[0] + w[1] * e[1]) / (e[0] * e[0] + e[1] * e[1])).clamp(0.0, 1.0);
            let q = [w[0] - e[0] * h, w[1] - e[1] * h];
            d = d.min(q[0] * q[0] + q[1] * q[1]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
        if inside { -d.sqrt() } else { d.sqrt() }
    })
}

/// A star: `points` tips at `outer` from the centre, the notches between at `inner`, the first tip
/// up.
fn star(points: usize, outer: f32, inner: f32) -> Sdf {
    let pts = (0..points * 2)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / points as f32;
            let r = if i % 2 == 0 { outer } else { inner };
            [r * a.sin(), -r * a.cos()]
        })
        .collect();
    poly(pts)
}

fn any(parts: Vec<Sdf>) -> Sdf {
    Box::new(move |p| parts.iter().map(|f| f(p)).fold(f32::INFINITY, f32::min))
}

fn minus(a: Sdf, b: Sdf) -> Sdf {
    Box::new(move |p| a(p).max(-b(p)))
}

fn both(a: Sdf, b: Sdf) -> Sdf {
    Box::new(move |p| a(p).max(b(p)))
}

/// Grown by `r` (rounding its corners).
fn grown(a: Sdf, r: f32) -> Sdf {
    Box::new(move |p| a(p) - r)
}

/// The outline of `a`, `r` thick each side.
fn ring(a: Sdf, r: f32) -> Sdf {
    Box::new(move |p| a(p).abs() - r)
}

const RED: [u8; 3] = [240, 62, 78];
const YELLOW: [u8; 3] = [255, 204, 46];
const ORANGE: [u8; 3] = [255, 138, 40];
const GREEN: [u8; 3] = [46, 190, 110];
const BLUE: [u8; 3] = [64, 156, 255];
const PINK: [u8; 3] = [255, 122, 170];
const PURPLE: [u8; 3] = [140, 98, 245];
const WHITE: [u8; 3] = [255, 255, 255];
const INK: [u8; 3] = [38, 38, 46];
const PALE: [u8; 3] = [226, 240, 255];

/// A sticker's parts, drawn in order.
fn parts(id: &str) -> Option<Vec<(Sdf, [u8; 3])>> {
    Some(match id {
        "heart" => vec![(
            any(vec![
                circle([-0.31, -0.2], 0.37),
                circle([0.31, -0.2], 0.37),
                poly(vec![[-0.61, 0.02], [0.0, -0.2], [0.61, 0.02], [0.0, 0.7]]),
            ]),
            RED,
        )],
        "star" => vec![(grown(star(5, 0.74, 0.3), 0.05), YELLOW)],
        "sparkle" => vec![
            (grown(star(4, 0.78, 0.2), 0.03), YELLOW),
            (grown(Box::new(|p| star(4, 0.26, 0.07)([p[0] - 0.56, p[1] + 0.56])), 0.02), YELLOW),
        ],
        "burst" => vec![(star(12, 0.84, 0.58), ORANGE), (star(12, 0.56, 0.38), YELLOW)],
        "arrow" => vec![(
            grown(
                poly(vec![
                    [-0.72, -0.16],
                    [0.12, -0.16],
                    [0.12, -0.46],
                    [0.74, 0.0],
                    [0.12, 0.46],
                    [0.12, 0.16],
                    [-0.72, 0.16],
                ]),
                0.04,
            ),
            RED,
        )],
        "ring" => vec![(ring(circle([0.0, 0.0], 0.68), 0.07), RED)],
        "check" => vec![
            (circle([0.0, 0.0], 0.76), GREEN),
            (any(vec![stroke([-0.34, 0.04], [-0.1, 0.28], 0.085), stroke([-0.1, 0.28], [0.36, -0.24], 0.085)]), WHITE),
        ],
        "cross" => vec![
            (circle([0.0, 0.0], 0.76), RED),
            (
                any(vec![stroke([-0.28, -0.28], [0.28, 0.28], 0.085), stroke([-0.28, 0.28], [0.28, -0.28], 0.085)]),
                WHITE,
            ),
        ],
        "exclaim" => vec![(
            any(vec![
                poly(vec![[-0.17, -0.74], [0.17, -0.74], [0.08, 0.2], [-0.08, 0.2]]),
                circle([0.0, -0.66], 0.17),
                circle([0.0, 0.2], 0.08),
                circle([0.0, 0.58], 0.17),
            ]),
            RED,
        )],
        "bubble" => vec![
            (
                any(vec![rbox([0.0, -0.12], [0.76, 0.5], 0.26), poly(vec![[-0.42, 0.3], [-0.06, 0.3], [-0.5, 0.74]])]),
                WHITE,
            ),
            (any(vec![circle([-0.32, -0.12], 0.08), circle([0.0, -0.12], 0.08), circle([0.32, -0.12], 0.08)]), INK),
        ],
        "smile" => vec![
            (circle([0.0, 0.0], 0.76), YELLOW),
            (
                any(vec![
                    circle([-0.27, -0.2], 0.095),
                    circle([0.27, -0.2], 0.095),
                    // the lower part of a ring
                    both(ring(circle([0.0, 0.0], 0.42), 0.055), Box::new(|p| 0.16 - p[1])),
                ]),
                INK,
            ),
        ],
        "bolt" => vec![(
            grown(
                poly(vec![[0.16, -0.78], [-0.44, 0.1], [-0.04, 0.1], [-0.2, 0.78], [0.46, -0.16], [0.04, -0.16]]),
                0.03,
            ),
            YELLOW,
        )],
        "sun" => {
            let rays = (0..8)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 8.0;
                    stroke([0.58 * a.cos(), 0.58 * a.sin()], [0.78 * a.cos(), 0.78 * a.sin()], 0.065)
                })
                .collect();
            vec![(any(rays), ORANGE), (circle([0.0, 0.0], 0.42), YELLOW)]
        }
        "moon" => vec![(minus(circle([-0.06, 0.0], 0.72), circle([0.26, -0.16], 0.6)), YELLOW)],
        "cloud" => vec![(
            any(vec![
                circle([-0.38, 0.12], 0.3),
                circle([-0.08, -0.16], 0.38),
                circle([0.34, 0.06], 0.34),
                rbox([-0.02, 0.24], [0.56, 0.18], 0.18),
            ]),
            PALE,
        )],
        "drop" => vec![
            (any(vec![circle([0.0, 0.24], 0.46), poly(vec![[0.0, -0.78], [0.41, 0.03], [-0.41, 0.03]])]), BLUE),
            (stroke([-0.2, 0.2], [-0.12, 0.42], 0.05), PALE),
        ],
        "flower" => {
            let petals = (0..6)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 6.0;
                    circle([0.44 * a.sin(), -0.44 * a.cos()], 0.3)
                })
                .collect();
            vec![(any(petals), PINK), (circle([0.0, 0.0], 0.24), YELLOW)]
        }
        "note" => vec![(
            any(vec![
                circle([-0.3, 0.5], 0.2),
                circle([0.36, 0.36], 0.2),
                stroke([-0.15, 0.5], [-0.15, -0.5], 0.055),
                stroke([0.51, 0.36], [0.51, -0.64], 0.055),
                poly(vec![[-0.2, -0.56], [0.56, -0.7], [0.56, -0.44], [-0.2, -0.3]]),
            ]),
            PURPLE,
        )],
        "crown" => vec![
            (
                grown(
                    poly(vec![
                        [-0.7, 0.46],
                        [-0.76, -0.36],
                        [-0.34, 0.0],
                        [0.0, -0.56],
                        [0.34, 0.0],
                        [0.76, -0.36],
                        [0.7, 0.46],
                    ]),
                    0.04,
                ),
                YELLOW,
            ),
            (any(vec![circle([-0.4, 0.26], 0.07), circle([0.0, 0.26], 0.09), circle([0.4, 0.26], 0.07)]), RED),
        ],
        "ribbon" => vec![
            (
                any(vec![
                    poly(vec![[-0.88, -0.1], [-0.5, -0.1], [-0.5, 0.4], [-0.88, 0.4], [-0.72, 0.15]]),
                    poly(vec![[0.88, -0.1], [0.5, -0.1], [0.5, 0.4], [0.88, 0.4], [0.72, 0.15]]),
                ]),
                [196, 44, 62],
            ),
            (rbox([0.0, 0.0], [0.62, 0.27], 0.04), RED),
        ],
        _ => return None,
    })
}

/// How far the white border reaches past the shapes, and the dark rim past that.
const BORDER: f32 = 0.075;
const RIM: f32 = 0.022;

/// A sticker as `size × size` pixels of straight sRGB `[r, g, b, a]` bytes.
pub fn render(id: &str, size: u32) -> Option<Vec<u8>> {
    let parts = parts(id)?;
    let px = 2.0 / size as f32;
    // how much of a pixel a shape covers, from its distance at the pixel's centre
    let cover = |d: f32| (0.5 - d / px).clamp(0.0, 1.0);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // a margin all round keeps the rim inside the picture
            let p = [((x as f32 + 0.5) * px - 1.0) * 1.12, ((y as f32 + 0.5) * px - 1.0) * 1.12];
            let ds: Vec<f32> = parts.iter().map(|(f, _)| f(p) / 1.12).collect();
            let all = ds.iter().copied().fold(f32::INFINITY, f32::min);
            // premultiplied, bottom up: rim, border, parts
            let rim = cover(all - BORDER - RIM) * 0.3;
            let mut c = [0.0f32, 0.0, 0.0, rim];
            let mut over = |color: [u8; 3], k: f32| {
                for i in 0..3 {
                    c[i] = color[i] as f32 / 255.0 * k + c[i] * (1.0 - k);
                }
                c[3] = k + c[3] * (1.0 - k);
            };
            over(WHITE, cover(all - BORDER));
            for ((_, color), d) in parts.iter().zip(&ds) {
                over(*color, cover(*d));
            }
            let a = c[3];
            for v in &c[..3] {
                out.push(if a > 0.0 { (v / a * 255.0 + 0.5) as u8 } else { 0 });
            }
            out.push((a * 255.0 + 0.5) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sticker_draws_inside_its_picture() {
        for s in STICKERS {
            assert_eq!(id_of(&format!("{PREFIX}{}", s.id)), Some(s.id));
            let size = 64;
            let px = render(s.id, size).unwrap();
            assert_eq!(px.len(), (size * size * 4) as usize);
            let alpha = |x: u32, y: u32| px[((y * size + x) * 4 + 3) as usize];
            // something in the middle rows, nothing on the edge
            let solid = (0..size * size).filter(|i| alpha(i % size, i / size) == 255).count();
            assert!(solid > 300, "{}: {solid}", s.id);
            for i in 0..size {
                for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
                    assert_eq!(alpha(x, y), 0, "{} touches its edge at {x},{y}", s.id);
                }
            }
            // the same picture every time
            assert_eq!(render(s.id, size).unwrap(), px);
        }
        assert_eq!(render("nope", 8), None);
        assert_eq!(id_of("library:whoosh"), None);
        assert_eq!(crate::library::id_of("library:sticker/heart"), None);
    }
}
