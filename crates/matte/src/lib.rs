//! Cutting a person out of a picture (剪映's 智能抠像).
//!
//! A small segmentation model says, for a 256 × 256 copy of the picture, how likely each pixel is
//! part of a person. The model is MediaPipe's selfie segmentation (Apache-2.0, see `models/`),
//! bundled as it is published; [`tflite`] reads the file and [`run`](mod@run) does its sums, so
//! nothing but this crate is needed to use it, in a browser as well.
//!
//! The model's answer is coarse: a person's outline to within a few pixels of a small picture.
//! [`person`] fits it to the picture's own edges before giving it back at the picture's size.
//!
//! A thing photographed against a plain backdrop needs no model: [`backdrop_linear`] finds the
//! backdrop by the picture's edges and takes it away (see `backdrop`).

mod backdrop;
mod run;
mod tflite;

pub use backdrop::{Backdrop, Cut, backdrop_linear};

use rayon::prelude::*;
use std::sync::OnceLock;

/// The segmentation model, as MediaPipe publishes it.
pub const MODEL: &[u8] = include_bytes!("../models/selfie_segmentation.tflite");

fn model() -> Result<&'static tflite::Model, String> {
    static READ: OnceLock<Result<tflite::Model, String>> = OnceLock::new();
    READ.get_or_init(|| tflite::read(MODEL)).as_ref().map_err(Clone::clone)
}

/// How likely each pixel of a picture is part of a person, as the model answers for a copy of
/// the picture at its own size (256 × 256, the picture stretched to it): that many values, 0 to
/// 1, row by row. `rgb` is `width * height` pixels as on screen (sRGB-encoded, 0 to 1).
pub fn likely(width: usize, height: usize, rgb: &[[f32; 3]]) -> Result<Small, String> {
    if width == 0 || height == 0 || rgb.len() != width * height {
        return Err("a picture needs a size and that many pixels".into());
    }
    let model = model()?;
    let [h, w, _] = model.input.1;
    let picture = sized(width, height, rgb, w, h);
    let data = picture.iter().flatten().copied().collect();
    let answer = run::run(model, tflite::Tensor { h, w, c: 3, data })?;
    if (answer.h, answer.w, answer.c) != (h, w, 1) {
        return Err("the model's answer is not a picture of one channel".into());
    }
    Ok(Small { width: w, height: h, picture, person: answer.data })
}

/// The model's answer and the small picture it was given.
#[derive(Clone, Debug, PartialEq)]
pub struct Small {
    /// Columns.
    pub width: usize,
    /// Rows.
    pub height: usize,
    /// The picture as the model saw it.
    pub picture: Vec<[f32; 3]>,
    /// How likely each of its pixels is part of a person, 0 to 1.
    pub person: Vec<f32>,
}

/// How much of each pixel of a picture is a person: `width * height` values from 0 (backdrop) to
/// 1, row by row. `rgb` is the picture as on screen (sRGB-encoded, 0 to 1).
///
/// The model's small answer is carried to the picture's size along the picture's own edges: near
/// each pixel the answer is taken to follow the picture's brightness in a straight line, the
/// line found on the small picture and applied to the large one (a guided filter). An outline
/// that the model has to within a few pixels lands on the edge that is really there.
pub fn person(width: usize, height: usize, rgb: &[[f32; 3]]) -> Result<Vec<f32>, String> {
    let small = likely(width, height, rgb)?;
    let (w, h) = (small.width, small.height);
    let grey = |p: &[f32; 3]| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
    let guide: Vec<f32> = small.picture.iter().map(grey).collect();
    // the line `person ≈ gain * brightness + lift` around every small pixel
    let near = |v: &[f32]| boxed(v, w, h, REACH);
    let products = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).collect::<Vec<f32>>();
    let (mean_guide, mean_person) = (near(&guide), near(&small.person));
    let (mean_square, mean_both) = (near(&products(&guide, &guide)), near(&products(&guide, &small.person)));
    let mut gain = vec![0.0; w * h];
    let mut lift = vec![0.0; w * h];
    for i in 0..w * h {
        let spread = mean_square[i] - mean_guide[i] * mean_guide[i];
        gain[i] = (mean_both[i] - mean_guide[i] * mean_person[i]) / (spread + FLAT);
        lift[i] = mean_person[i] - gain[i] * mean_guide[i];
    }
    let (gain, lift) = (near(&gain), near(&lift));
    // read between the small pixels for every large one
    let mut alpha = vec![0.0; width * height];
    alpha.par_chunks_mut(width).zip(rgb.par_chunks(width)).enumerate().for_each(|(y, (row, pixels))| {
        let (y0, y1, fy) = between(y, height, h);
        for (x, (a, pixel)) in row.iter_mut().zip(pixels).enumerate() {
            let (x0, x1, fx) = between(x, width, w);
            let read = |v: &[f32]| {
                let (top, bottom) = (
                    v[y0 * w + x0] + (v[y0 * w + x1] - v[y0 * w + x0]) * fx,
                    v[y1 * w + x0] + (v[y1 * w + x1] - v[y1 * w + x0]) * fx,
                );
                top + (bottom - top) * fy
            };
            let fitted = (read(&gain) * grey(pixel) + read(&lift)).clamp(0.0, 1.0);
            // the model's "probably" is not a see-through person: firm up both ends
            let t = ((fitted - FIRM.0) / (FIRM.1 - FIRM.0)).clamp(0.0, 1.0);
            *a = t * t * (3.0 - 2.0 * t);
        }
    });
    Ok(alpha)
}

/// [`person`] for a picture as a compositor keeps it: `width * height` pixels of red, green, blue
/// and alpha, linear light, the colours already multiplied by the alpha.
pub fn person_linear(width: usize, height: usize, rgba: &[[f32; 4]]) -> Result<Vec<f32>, String> {
    if width == 0 || rgba.len() != width * height {
        return Err("a picture needs a size and that many pixels".into());
    }
    let encode = encoder();
    let mut rgb = vec![[0.0f32; 3]; rgba.len()];
    rgb.par_chunks_mut(width).zip(rgba.par_chunks(width)).for_each(|(row, pixels)| {
        for (out, [r, g, b, a]) in row.iter_mut().zip(pixels) {
            // what is see-through has no colour of its own: black
            let alpha = if *a > 1e-4 { 1.0 / a } else { 0.0 };
            *out = [r * alpha, g * alpha, b * alpha].map(&encode);
        }
    });
    person(width, height, &rgb)
}

/// Linear light to the sRGB encoding, by a table over the square root (which follows the
/// encoding's curve closely, so a small table read between its entries is exact to 1 in 10 000):
/// a power for every colour of every pixel is the slowest part of a cut-out otherwise.
fn encoder() -> impl Fn(f32) -> f32 + Sync {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    const STEPS: usize = 1024;
    let table = TABLE.get_or_init(|| {
        let exact = |v: f32| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (0..=STEPS + 1).map(|i| exact((i as f32 / STEPS as f32).powi(2))).collect()
    });
    move |v: f32| {
        let at = v.clamp(0.0, 1.0).sqrt() * STEPS as f32;
        let low = at as usize;
        table[low] + (table[low + 1] - table[low]) * (at - low as f32)
    }
}

/// How far around a small pixel the answer is fitted to the picture, in small pixels.
const REACH: usize = 3;
/// Brightness that varies less than this (as a variance) is flat: no edge to follow.
const FLAT: f32 = 4e-4;
/// Below the first the answer counts as backdrop, above the second as the person.
const FIRM: (f32, f32) = (0.15, 0.85);

/// For position `out` of `len`: the two of `of` it lies between, and how far towards the second.
fn between(out: usize, len: usize, of: usize) -> (usize, usize, f32) {
    let at = ((out as f32 + 0.5) * of as f32 / len as f32 - 0.5).clamp(0.0, (of - 1) as f32);
    let low = at.floor() as usize;
    (low, (low + 1).min(of - 1), at - low as f32)
}

/// The mean of `v` (`w × h`) over the square reaching `r` each way around every value, the
/// square cut at the picture's edges.
fn boxed(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    // along rows, then along columns: running sums, so the reach costs nothing
    let along = |v: &[f32], len: usize, step: usize, out: &mut [f32]| {
        let mut sums = vec![0.0f64; len + 1];
        for i in 0..len {
            sums[i + 1] = sums[i] + f64::from(v[i * step]);
        }
        for i in 0..len {
            let (from, to) = (i.saturating_sub(r), (i + r + 1).min(len));
            out[i * step] = ((sums[to] - sums[from]) / (to - from) as f64) as f32;
        }
    };
    let mut rows = vec![0.0; w * h];
    for (row, out) in v.chunks_exact(w).zip(rows.chunks_exact_mut(w)) {
        along(row, w, 1, out);
    }
    let mut out = vec![0.0; w * h];
    for x in 0..w {
        along(&rows[x..], h, w, &mut out[x..]);
    }
    out
}

/// `rgb` (`width × height`) as a picture of `w × h`, stretched: each new pixel is the mean of
/// the part of the picture it covers, or read between the pixels where the picture is smaller.
fn sized(width: usize, height: usize, rgb: &[[f32; 3]], w: usize, h: usize) -> Vec<[f32; 3]> {
    // for every new position: the old ones it takes from and how much of each
    let shares = |len: usize, of: usize| -> Vec<Vec<(usize, f32)>> {
        (0..len)
            .map(|i| {
                if of <= len {
                    let (a, b, f) = between(i, len, of);
                    return vec![(a, 1.0 - f), (b, f)];
                }
                let (from, to) = (i as f32 * of as f32 / len as f32, (i + 1) as f32 * of as f32 / len as f32);
                let cells = (from.floor() as usize)..(to.ceil() as usize).min(of);
                cells
                    .map(|c| (c, ((c + 1) as f32).min(to) - (c as f32).max(from)))
                    .map(|(c, s)| (c, s / (to - from)))
                    .collect()
            })
            .collect()
    };
    let (across, down) = (shares(w, width), shares(h, height));
    let mut out = vec![[0.0; 3]; w * h];
    out.par_chunks_mut(w).zip(&down).for_each(|(row, rows)| {
        for (pixel, columns) in row.iter_mut().zip(&across) {
            for (y, sy) in rows {
                for (x, sx) in columns {
                    let p = rgb[y * width + x];
                    for c in 0..3 {
                        pixel[c] += p[c] * sy * sx;
                    }
                }
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(name: &str) -> (usize, usize, Vec<[f32; 3]>) {
        let path = format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(path).expect("the picture").to_rgb8();
        let rgb = img.pixels().map(|p| p.0.map(|v| f32::from(v) / 255.0)).collect();
        (img.width() as usize, img.height() as usize, rgb)
    }

    /// The model's answer for a portrait is what TensorFlow Lite's own interpreter answers for
    /// the same file and picture (`tests/astronaut_person.png`, kept to 8 bits).
    #[test]
    fn the_model_answers_as_its_reference_interpreter_does() {
        let (w, h, rgb) = picture("astronaut.png");
        let small = likely(w, h, &rgb).expect("runs");
        assert_eq!((small.width, small.height, small.person.len()), (256, 256, 256 * 256));
        // the picture is already the model's size: it is given as it is
        assert_eq!(small.picture, rgb);
        let (_, _, want) = picture("astronaut_person.png");
        let worst = small.person.iter().zip(&want).map(|(a, b)| (a - b[0]).abs()).fold(0.0f32, f32::max);
        // half a step of the 8 bits the reference is kept in, and a little
        assert!(worst < 0.5 / 255.0 + 1e-3, "{worst}");
    }

    #[test]
    fn a_person_is_kept_and_the_backdrop_goes() {
        let (w, h, rgb) = picture("astronaut.png");
        let alpha = person(w, h, &rgb).expect("runs");
        let at = |x: usize, y: usize| alpha[y * w + x];
        // the face and the suit stay whole; the studio backdrop left and right of the head goes
        assert!(at(128, 60) > 0.99 && at(110, 170) > 0.99, "{} {}", at(128, 60), at(110, 170));
        assert!(at(20, 30) < 0.01 && at(235, 30) < 0.01, "{} {}", at(20, 30), at(235, 30));
        assert!(alpha.iter().all(|a| (0.0..=1.0).contains(a)));
        // mostly one or the other: an outline, not a haze
        let between = alpha.iter().filter(|a| (0.05..0.95).contains(*a)).count();
        assert!(between * 20 < alpha.len(), "{between}");

        // at another size the same parts stay and go, and the answer is the picture's size
        let (w2, h2) = (400, 300);
        let big = sized(w, h, &rgb, w2, h2);
        let alpha = person(w2, h2, &big).expect("runs");
        assert_eq!(alpha.len(), w2 * h2);
        let at = |x: usize, y: usize| alpha[(y * h2 / 256) * w2 + x * w2 / 256];
        assert!(at(128, 60) > 0.99 && at(20, 30) < 0.01);

        // as a compositor keeps pictures: linear light, and see-through where it already was
        let decode = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let rgba: Vec<[f32; 4]> = rgb.iter().map(|p| [decode(p[0]), decode(p[1]), decode(p[2]), 1.0]).collect();
        let linear = person_linear(w, h, &rgba).expect("runs");
        let direct = person(w, h, &rgb).expect("runs");
        let worst = linear.iter().zip(&direct).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.02, "{worst}");

        assert!(person(0, 4, &[]).is_err() && person(2, 2, &rgb[..3]).is_err() && person_linear(2, 2, &[]).is_err());
    }

    #[test]
    fn the_encoding_table_is_the_encoding() {
        let encode = encoder();
        let exact = |v: f32| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        for i in 0..=10_000 {
            let v = i as f32 / 10_000.0;
            assert!((encode(v) - exact(v)).abs() < 1e-4, "{v}");
        }
        assert_eq!((encode(-1.0), encode(7.0)), (0.0, encode(1.0)));
    }

    #[test]
    fn a_picture_is_sized_by_what_each_pixel_covers() {
        let px = |v: f32| [v, v, v];
        // halved: the mean of each pair; a third of the way: shares of the two it straddles
        let row = [px(0.0), px(1.0), px(2.0), px(3.0)];
        assert_eq!(sized(4, 1, &row, 2, 1), [px(0.5), px(2.5)]);
        let three = sized(3, 1, &[px(0.0), px(3.0), px(6.0)], 2, 1);
        assert!((three[0][0] - 1.0).abs() < 1e-5 && (three[1][0] - 5.0).abs() < 1e-5, "{three:?}");
        // larger: read between the pixels, the edges held
        assert_eq!(sized(2, 1, &[px(0.0), px(1.0)], 4, 1), [px(0.0), px(0.25), px(0.75), px(1.0)]);
        // means over a square stop at the picture's edges
        assert_eq!(boxed(&[0.0, 3.0, 6.0], 3, 1, 1), [1.5, 3.0, 4.5]);
    }
}
