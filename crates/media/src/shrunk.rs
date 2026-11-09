//! Pictures made smaller as they are converted, for previews drawn small: a frame `s` times
//! smaller each way costs about `s²` times less to convert, and the compositor then has that much
//! less to sample. Each output pixel is the average of the `s × s` block it stands for.

use filmcraft_frame::{PixelData, VideoFrame};
use rayon::prelude::*;

/// `frame` as straight sRGB RGBA8, `s` times smaller each way (at least one pixel):
/// `(width, height, bytes)`.
pub(crate) fn srgba8(frame: &VideoFrame, s: usize) -> (u32, u32, Vec<u8>) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let (ow, oh) = ((w / s).max(1), (h / s).max(1));
    let out = match &frame.data {
        PixelData::Yuv8 { planes, chroma, alpha: None }
            if matches!(frame.color.transfer, filmcraft_color::Transfer::Bt709 | filmcraft_color::Transfer::Srgb) =>
        {
            let (sx, sy) = chroma.shifts();
            let cw = w.div_ceil(1 << sx);
            let (kr, kb) = frame.color.matrix.kr_kb();
            let kg = 1.0 - kr - kb;
            let (ys, yo, cs) = match frame.color.range {
                filmcraft_color::Range::Limited => (255.0 / 219.0, 16.0, 255.0 / 224.0),
                filmcraft_color::Range::Full => (1.0, 0.0, 1.0),
            };
            let (crr, cbb) = (2.0 * (1.0 - kr) * cs, 2.0 * (1.0 - kb) * cs);
            let (cgr, cgb) = (crr * kr / kg, cbb * kb / kg);
            // the block in the chroma planes, at least a sample
            let (bx, by) = ((s >> sx).max(1), (s >> sy).max(1));
            let mut out = vec![0u8; ow * oh * 4];
            out.par_chunks_mut(ow * 4).enumerate().for_each(|(oy, row)| {
                for ox in 0..ow {
                    let (x0, y0) = (ox * s, oy * s);
                    let mut yy = 0u32;
                    for y in y0..(y0 + s).min(h) {
                        yy += planes[0][y * w + x0..y * w + (x0 + s).min(w)].iter().map(|&v| v as u32).sum::<u32>();
                    }
                    let n = (((y0 + s).min(h) - y0) * ((x0 + s).min(w) - x0)) as f32;
                    let (cx0, cy0) = (x0 >> sx, y0 >> sy);
                    let (mut u, mut v, mut m) = (0u32, 0u32, 0u32);
                    for cy in cy0..(cy0 + by).min(h.div_ceil(1 << sy)) {
                        for cx in cx0..(cx0 + bx).min(cw) {
                            u += planes[1][cy * cw + cx] as u32;
                            v += planes[2][cy * cw + cx] as u32;
                            m += 1;
                        }
                    }
                    let yy = (yy as f32 / n - yo) * ys;
                    let u = u as f32 / m.max(1) as f32 - 128.0;
                    let v = v as f32 / m.max(1) as f32 - 128.0;
                    let o = &mut row[ox * 4..][..4];
                    o[0] = (yy + crr * v).round().clamp(0.0, 255.0) as u8;
                    o[1] = (yy - cgr * v - cgb * u).round().clamp(0.0, 255.0) as u8;
                    o[2] = (yy + cbb * u).round().clamp(0.0, 255.0) as u8;
                    o[3] = 255;
                }
            });
            out
        }
        _ => average(&frame.to_rgba8(), w, h, s, ow, oh),
    };
    (ow as u32, oh as u32, out)
}

/// RGBA8 `bytes` (`w × h`) averaged over `s × s` blocks into `ow × oh`.
fn average(bytes: &[u8], w: usize, h: usize, s: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0u8; ow * oh * 4];
    out.par_chunks_mut(ow * 4).enumerate().for_each(|(oy, row)| {
        for ox in 0..ow {
            let mut sum = [0u32; 4];
            let mut n = 0;
            for y in oy * s..(oy * s + s).min(h) {
                for x in ox * s..(ox * s + s).min(w) {
                    for (c, v) in sum.iter_mut().zip(&bytes[(y * w + x) * 4..][..4]) {
                        *c += *v as u32;
                    }
                    n += 1;
                }
            }
            for (o, c) in row[ox * 4..][..4].iter_mut().zip(sum) {
                *o = (c as f32 / n.max(1) as f32).round() as u8;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> VideoFrame {
        let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
        let luma = (0..(w * h) as usize).map(|i| (80 + i % 100) as u8).collect();
        let cb = (0..cw * ch).map(|i| (118 + i % 20) as u8).collect();
        let cr = (0..cw * ch).map(|i| (120 + i % 17) as u8).collect();
        VideoFrame {
            width: w,
            height: h,
            data: PixelData::Yuv8 {
                planes: [luma, cb, cr].map(std::sync::Arc::new),
                chroma: filmcraft_frame::Chroma::C420,
                alpha: None,
            },
            color: Default::default(),
            par: (1, 1),
            pts: Default::default(),
        }
    }

    #[test]
    fn a_shrunk_picture_is_the_average_of_the_full_one() {
        let f = frame(64, 36);
        let full = f.to_rgba8();
        let (w, h, small) = srgba8(&f, 4);
        assert_eq!((w, h), (16, 9));
        // the same as converting first and then averaging, give or take the rounding
        let want = average(&full, 64, 36, 4, 16, 9);
        let worst = small.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 2, "{worst}");
        // never smaller than a pixel
        assert_eq!(srgba8(&frame(3, 2), 8).0, 1);
    }
}
