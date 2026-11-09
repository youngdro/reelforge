//! Video pictures as the decoder made them: 8-bit planar Y'CbCr. The GPU compositor converts them
//! in a shader ([`crate::gpu`]), so the CPU neither converts nor uploads a float picture; the CPU
//! compositor never asks for them.

use std::sync::Arc;

use crate::Image;

/// An 8-bit planar Y'CbCr picture with a BT.709-like (display-referred) transfer.
#[derive(Clone, Debug)]
pub struct Yuv {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Y', Cb and Cr, each row tight: luma `width` wide, chroma [`chroma_size`](Self::chroma_size).
    pub planes: [Arc<Vec<u8>>; 3],
    /// How far chroma is subsampled, as shifts `(x, y)`: `(1, 1)` is 4:2:0, `(0, 0)` is 4:4:4.
    pub shift: (u32, u32),
    /// The matrix's Kr and Kb (BT.709: 0.2126, 0.0722).
    pub kr_kb: (f32, f32),
    /// Full range (0–255) rather than limited (16–235, 16–240).
    pub full_range: bool,
}

impl Yuv {
    /// The size of a chroma plane.
    pub fn chroma_size(&self) -> (u32, u32) {
        (self.width.div_ceil(1 << self.shift.0), self.height.div_ceil(1 << self.shift.1))
    }

    /// Whether the planes hold what the sizes say.
    pub fn is_whole(&self) -> bool {
        let (cw, ch) = self.chroma_size();
        let luma = (self.width * self.height) as usize;
        let chroma = (cw * ch) as usize;
        self.planes[0].len() >= luma && self.planes[1].len() >= chroma && self.planes[2].len() >= chroma
    }

    /// The scale factors of the conversion: `(luma scale, luma offset, chroma scale)`.
    pub fn range(&self) -> (f32, f32, f32) {
        if self.full_range { (1.0, 0.0, 1.0) } else { (255.0 / 219.0, 16.0, 255.0 / 224.0) }
    }

    /// The picture in the compositor's working form, converted on the CPU: the same sums as the
    /// shader (nearest chroma, rounded to 8 bits, then linearised).
    pub fn to_image(&self) -> Image {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, _) = self.chroma_size();
        let (kr, kb) = self.kr_kb;
        let kg = 1.0 - kr - kb;
        let (ys, yo, cs) = self.range();
        let (crr, cbb) = (2.0 * (1.0 - kr) * cs, 2.0 * (1.0 - kb) * cs);
        let (cgr, cgb) = (crr * kr / kg, cbb * kb / kg);
        let mut bytes = vec![0u8; w * h * 4];
        for y in 0..h {
            let cy = y >> self.shift.1;
            for x in 0..w {
                let c = cy * cw as usize + (x >> self.shift.0);
                let yy = (self.planes[0][y * w + x] as f32 - yo) * ys;
                let u = self.planes[1][c] as f32 - 128.0;
                let v = self.planes[2][c] as f32 - 128.0;
                let o = &mut bytes[(y * w + x) * 4..][..4];
                o[0] = (yy + crr * v).round().clamp(0.0, 255.0) as u8;
                o[1] = (yy - cgr * v - cgb * u).round().clamp(0.0, 255.0) as u8;
                o[2] = (yy + cbb * u).round().clamp(0.0, 255.0) as u8;
                o[3] = 255;
            }
        }
        Image::from_srgba8(self.width, self.height, &bytes)
    }
}
