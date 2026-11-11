//! Pictures in the compositor's working space: linear light, premultiplied alpha, `f32`.

/// An RGBA picture, linear light, premultiplied, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` pixels of `[r, g, b, a]`.
    pub pixels: Vec<[f32; 4]>,
}

impl Image {
    /// A picture filled with one premultiplied linear colour.
    pub fn filled(width: u32, height: u32, color: [f32; 4]) -> Image {
        Image { width, height, pixels: vec![color; (width * height) as usize] }
    }

    /// Reads straight sRGB 8-bit RGBA, as decoders and PNG files give it.
    pub fn from_srgba8(width: u32, height: u32, bytes: &[u8]) -> Image {
        // the same sums as premultiply_srgb, looked up: a frame is millions of them
        static LINEAR: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
        let linear = LINEAR.get_or_init(|| std::array::from_fn(|v| srgb_to_linear(v as f32 / 255.0)));
        let pixels = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                [linear[p[0] as usize] * a, linear[p[1] as usize] * a, linear[p[2] as usize] * a, a]
            })
            .collect();
        Image { width, height, pixels }
    }

    /// Writes straight sRGB 8-bit RGBA.
    pub fn to_srgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let a = p[3].clamp(0.0, 1.0);
            let un = |c: f32| if a > 0.0 { linear_to_srgb((c / a).clamp(0.0, 1.0)) } else { 0.0 };
            for v in [un(p[0]), un(p[1]), un(p[2]), a] {
                out.push((v * 255.0 + 0.5) as u8);
            }
        }
        out
    }

    /// The pixel at `(x, y)`, transparent outside the picture.
    pub fn get(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            [0.0; 4]
        } else {
            self.pixels[(y as u32 * self.width + x as u32) as usize]
        }
    }

    /// Bilinear sample at a continuous position in pixel units (pixel centres at `n + 0.5`).
    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.get(x0, y0), self.get(x0 + 1, y0), self.get(x0, y0 + 1), self.get(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            out[i] = top + (bottom - top) * ty;
        }
        out
    }
}

/// sRGB transfer to linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear to sRGB transfer.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A straight sRGB colour as a premultiplied linear one.
pub fn premultiply_srgb(c: [f32; 4]) -> [f32; 4] {
    let a = c[3];
    [srgb_to_linear(c[0]) * a, srgb_to_linear(c[1]) * a, srgb_to_linear(c[2]) * a, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_convert_as_premultiply_srgb_does() {
        let bytes: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 3, v ^ 0x5a]).collect();
        let img = Image::from_srgba8(256, 1, &bytes);
        for (p, b) in img.pixels.iter().zip(bytes.chunks(4)) {
            let f = |i: usize| b[i] as f32 / 255.0;
            assert_eq!(*p, premultiply_srgb([f(0), f(1), f(2), f(3)]));
        }
    }

    #[test]
    fn srgb8_round_trips() {
        let bytes: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
        let img = Image::from_srgba8(256, 1, &bytes);
        assert_eq!(img.to_srgba8(), bytes);
    }
}

/// `img` blurred with a standard deviation of `radius` times its shorter side, edges held. The
/// blur runs on a copy shrunk until the deviation is about two pixels (three box blurs there come
/// within a few per cent of a Gaussian), so the result is small: a compositor scales it back up
/// as it samples any picture, which smooths it further. CPU and GPU draw the same picture.
pub fn blurred(img: &Image, radius: f32) -> Image {
    let sigma = radius.max(0.0) * img.width.min(img.height) as f32;
    let f = if sigma > 2.0 { 2.0 / sigma } else { 1.0 };
    let (w, h) = (((img.width as f32 * f).round() as u32).max(4), ((img.height as f32 * f).round() as u32).max(4));
    let mut small = shrink(img, w.min(img.width), h.min(img.height));
    let s = sigma * small.width as f32 / img.width.max(1) as f32;
    // three boxes of half-width r: variance 3 * ((2r+1)^2 - 1) / 12
    let r = (((4.0 * s * s + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as i64;
    if sigma > 0.0 {
        for _ in 0..3 {
            box_pass(&mut small, r, true);
            box_pass(&mut small, r, false);
        }
    }
    small
}

/// How far a blur of standard deviation `sigma` (pixels) spreads what it blurs, as [`soft`]
/// does it.
pub fn soft_room(sigma: f32) -> u32 {
    3 * soft_box(sigma) as u32
}

/// The half-width of each of three box blurs that together blur by `sigma`.
fn soft_box(sigma: f32) -> i64 {
    (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as i64
}

/// `img` blurred with a standard deviation of `sigma` of its own pixels, with nothing around it:
/// the picture grown by `room` (left, top, right, bottom) for its edges to soften into
/// ([`soft_room`] holds all of it). Three box blurs, within a few per cent of a Gaussian.
pub fn soft(img: &Image, sigma: f32, room: [u32; 4]) -> Image {
    let r = soft_box(sigma);
    let (w, h) = (img.width + room[0] + room[2], img.height + room[1] + room[3]);
    let mut out = Image { width: w, height: h, pixels: vec![[0.0; 4]; w as usize * h as usize] };
    for y in 0..img.height as usize {
        let from = y * img.width as usize;
        let to = (y + room[1] as usize) * w as usize + room[0] as usize;
        out.pixels[to..to + img.width as usize].copy_from_slice(&img.pixels[from..from + img.width as usize]);
    }
    for _ in 0..3 {
        box_pass(&mut out, r, true);
        box_pass(&mut out, r, false);
    }
    out
}

/// `img` shrunk to `w × h`, each pixel the mean of the pixels it covers.
/// `img` averaged down to `w` by `h` (no larger than it).
pub(crate) fn shrink(img: &Image, w: u32, h: u32) -> Image {
    if (w, h) == (img.width, img.height) {
        return img.clone();
    }
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut pixels = Vec::with_capacity(w as usize * h as usize);
    for ty in 0..h as usize {
        let (y0, y1) = (ty * sh / h as usize, ((ty + 1) * sh / h as usize).max(ty * sh / h as usize + 1).min(sh));
        for tx in 0..w as usize {
            let (x0, x1) = (tx * sw / w as usize, ((tx + 1) * sw / w as usize).max(tx * sw / w as usize + 1).min(sw));
            let mut sum = [0.0f32; 4];
            for y in y0..y1 {
                for p in &img.pixels[y * sw + x0..y * sw + x1] {
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as f32;
            pixels.push(sum.map(|v| v / n));
        }
    }
    Image { width: w, height: h, pixels }
}

/// One running-sum box blur of half-width `r` along rows (`across`) or columns, edges held.
fn box_pass(img: &mut Image, r: i64, across: bool) {
    let (w, h) = (img.width as i64, img.height as i64);
    let (len, lines) = if across { (w, h) } else { (h, w) };
    let at = |line: i64, i: i64| -> usize {
        let i = i.clamp(0, len - 1);
        (if across { line * w + i } else { i * w + line }) as usize
    };
    let n = (2 * r + 1) as f32;
    let mut out = vec![[0.0f32; 4]; len as usize];
    for line in 0..lines {
        let mut sum = [0.0f32; 4];
        for i in -r..=r {
            let p = img.pixels[at(line, i)];
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        for i in 0..len {
            out[i as usize] = sum.map(|v| v / n);
            let (add, drop) = (img.pixels[at(line, i + r + 1)], img.pixels[at(line, i - r)]);
            for c in 0..4 {
                sum[c] += add[c] - drop[c];
            }
        }
        for i in 0..len {
            img.pixels[at(line, i)] = out[i as usize];
        }
    }
}

#[cfg(test)]
mod blur_tests {
    use super::*;

    #[test]
    fn a_blur_spreads_an_edge_keeps_the_mean_and_shrinks_the_picture() {
        // left half black, right half white, 400 × 200
        let mut img = Image::filled(400, 200, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..200 {
            for x in 200..400 {
                img.pixels[y * 400 + x] = [1.0; 4];
            }
        }
        let b = blurred(&img, 0.05);
        assert!(b.width < 400 && (b.width as f32 / b.height as f32 - 2.0).abs() < 0.1, "{}×{}", b.width, b.height);
        let row = |x: u32| b.pixels[(b.height / 2 * b.width + x) as usize][0];
        assert!(row(0) < 0.02 && row(b.width - 1) > 0.98, "edges held: {} {}", row(0), row(b.width - 1));
        let mid = row(b.width / 2);
        assert!((0.2..0.8).contains(&mid), "the edge is soft: {mid}");
        let mean: f32 = b.pixels.iter().map(|p| p[0]).sum::<f32>() / b.pixels.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        // no blur, no change
        assert_eq!(blurred(&img, 0.0), img);
    }
}
