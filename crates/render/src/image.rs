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
