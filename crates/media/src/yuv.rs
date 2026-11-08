//! Compositor pictures to the encoder's input: 8-bit limited-range BT.709 4:2:0.

use reelforge_render::Image;
use reelforge_render::image::linear_to_srgb;

/// A planar 4:2:0 picture, 8-bit limited-range BT.709: what encoders take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Yuv420 {
    /// Luma, `width × height`.
    pub y: Vec<u8>,
    /// Blue difference, half each way (rounded up).
    pub u: Vec<u8>,
    /// Red difference, as `u`.
    pub v: Vec<u8>,
    /// Width in pixels.
    pub width: usize,
}

impl Yuv420 {
    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.y.len().checked_div(self.width).unwrap_or(0)
    }

    /// Chroma width in samples.
    pub fn chroma_width(&self) -> usize {
        self.width.div_ceil(2)
    }
}

/// Converts a linear, premultiplied picture, flattened onto black, to Y'CbCr. The transfer is
/// sRGB's, as the preview shows it; chroma is the average of each 2×2 block.
pub(crate) fn to_yuv420(img: &Image, out: &mut Yuv420) {
    let (w, h) = (img.width as usize, img.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    out.width = w;
    out.y.resize(w * h, 0);
    out.u.resize(cw * ch, 0);
    out.v.resize(cw * ch, 0);
    let mut cb = vec![0f32; cw];
    let mut cr = vec![0f32; cw];
    let mut n = vec![0f32; cw];
    for cy in 0..ch {
        cb.fill(0.0);
        cr.fill(0.0);
        n.fill(0.0);
        for row in cy * 2..(cy * 2 + 2).min(h) {
            for x in 0..w {
                let p = img.pixels[row * w + x];
                let [r, g, b] = [p[0], p[1], p[2]].map(|c| linear_to_srgb(c.clamp(0.0, 1.0)));
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                out.y[row * w + x] = (16.0 + 219.0 * luma).round() as u8;
                cb[x / 2] += (b - luma) / 1.8556;
                cr[x / 2] += (r - luma) / 1.5748;
                n[x / 2] += 1.0;
            }
        }
        for cx in 0..cw {
            out.u[cy * cw + cx] = (128.0 + 224.0 * cb[cx] / n[cx]).round().clamp(16.0, 240.0) as u8;
            out.v[cy * cw + cx] = (128.0 + 224.0 * cr[cx] / n[cx]).round().clamp(16.0, 240.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_black_and_red_land_on_bt709_codes() {
        let mut img = Image::filled(4, 2, [0.0, 0.0, 0.0, 1.0]);
        img.pixels[0] = [1.0, 1.0, 1.0, 1.0];
        img.pixels[1] = [1.0, 1.0, 1.0, 1.0];
        img.pixels[4] = [1.0, 1.0, 1.0, 1.0];
        img.pixels[5] = [1.0, 1.0, 1.0, 1.0];
        for i in [2, 3, 6, 7] {
            img.pixels[i] = [1.0, 0.0, 0.0, 1.0];
        }
        let mut out = Yuv420 { y: vec![], u: vec![], v: vec![], width: 0 };
        to_yuv420(&img, &mut out);
        assert_eq!(out.y[0], 235);
        assert_eq!((out.u[0], out.v[0]), (128, 128));
        assert_eq!(out.y[2], 63); // 16 + 219 × 0.2126
        assert_eq!((out.u[1], out.v[1]), (102, 240));
    }
}
