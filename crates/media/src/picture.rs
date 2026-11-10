//! Looking at pictures before editing: the frame a layer shows, and the colour of the backdrop
//! behind a subject (a green screen), so a tool can key it out without being told the colour.

use std::sync::Arc;

use reelforge_render::image::linear_to_srgb;
use reelforge_render::{Image, MediaSource, PlannedSource, plan_frame};
use reelforge_renderspec::RenderSpec;
use reelforge_time::Tick;

/// The picture media layer `layer` of `spec` shows at `t`, as it was shot: before its crop,
/// effects and placement.
pub fn layer_picture(spec: &RenderSpec, media: &dyn MediaSource, layer: &str, t: Tick) -> Result<Arc<Image>, String> {
    let plan = plan_frame(spec, t, 1.0);
    let planned = plan
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or_else(|| format!("clip {layer:?} shows nothing at that time (it is hidden, or not there then)"))?;
    let PlannedSource::Media { asset, at, .. } = &planned.source else {
        return Err(format!("clip {layer:?} is not a video or a picture"));
    };
    let def = spec.assets.get(asset).ok_or_else(|| format!("unknown asset {asset:?}"))?;
    media.picture(asset, def, *at)?.ok_or_else(|| format!("the picture of clip {layer:?} is offline or not ready yet"))
}

/// Chroma of an encoded colour: BT.601 Cb and Cr, as `key.chroma@1` measures it.
fn chroma(c: [f32; 3]) -> [f32; 2] {
    [-0.168_736 * c[0] - 0.331_264 * c[1] + 0.5 * c[2], 0.5 * c[0] - 0.418_688 * c[1] - 0.081_312 * c[2]]
}

/// The one colour most of a picture's border has, if it has one: the backdrop a subject stands
/// in front of. Returns it as on screen (sRGB, 0 to 1) with the share of the border that is
/// about that colour. Greys do not count: a white wall is no key.
pub fn backdrop_color(img: &Image) -> Option<([f32; 3], f32)> {
    const BINS: usize = 24;
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 8 || h < 8 {
        return None;
    }
    let (bw, bh) = ((w * 6 / 100).max(2), (h * 6 / 100).max(2));
    // about twenty thousand pixels of the border are plenty
    let stride = ((2 * (w * bh + h * bw)) / 20_000).max(1);
    let mut seen = 0usize;
    let mut total = 0usize;
    // per bin: how many, and the sum of their colours
    let mut bins = vec![(0usize, [0.0f32; 3]); BINS * BINS];
    for y in 0..h {
        for x in 0..w {
            if !(x < bw || x >= w - bw || y < bh || y >= h - bh) {
                continue;
            }
            seen += 1;
            if !seen.is_multiple_of(stride) {
                continue;
            }
            let p = img.pixels[y * w + x];
            if p[3] < 0.5 {
                continue;
            }
            total += 1;
            let c = [p[0], p[1], p[2]].map(|v| linear_to_srgb((v / p[3]).clamp(0.0, 1.0)));
            let q = chroma(c);
            if q[0].hypot(q[1]) < 0.08 {
                continue;
            }
            let bin = |v: f32| (((v + 0.5) * BINS as f32) as usize).min(BINS - 1);
            let slot = &mut bins[bin(q[1]) * BINS + bin(q[0])];
            slot.0 += 1;
            for (sum, v) in slot.1.iter_mut().zip(c) {
                *sum += v;
            }
        }
    }
    if total == 0 {
        return None;
    }
    // a bin with its neighbours: a screen's shades fall in a few bins side by side
    let around = |i: usize| {
        let (bx, by) = (i % BINS, i / BINS);
        let mut n = 0;
        let mut sum = [0.0f32; 3];
        for y in by.saturating_sub(1)..=(by + 1).min(BINS - 1) {
            for x in bx.saturating_sub(1)..=(bx + 1).min(BINS - 1) {
                let (count, colours) = bins[y * BINS + x];
                n += count;
                for (sum, v) in sum.iter_mut().zip(colours) {
                    *sum += v;
                }
            }
        }
        (n, sum)
    };
    // (the fullest neighbourhood, not the fullest bin: that one may sit at the edge of the shades)
    let (n, sum) = (0..BINS * BINS).map(around).max_by_key(|(n, _)| *n)?;
    let share = n as f32 / total as f32;
    (n > 0 && share >= 0.25).then(|| (sum.map(|v| v / n as f32), share))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture whose pixel at `(x, y)` is `f(x, y)` on screen.
    fn drawn(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Image {
        let mut bytes = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                bytes.extend([r, g, b, 255]);
            }
        }
        Image::from_srgba8(w, h, &bytes)
    }

    #[test]
    fn a_screens_colour_is_found_around_a_subject() {
        // an unevenly lit green screen, a subject in the middle that reaches the bottom edge
        let img = drawn(160, 120, |x, y| {
            if (60..100).contains(&x) && y >= 40 {
                [200, 150, 120]
            } else {
                [30 + (x / 8) as u8, 170 + (y / 4) as u8, 50 + ((x + y) % 16) as u8]
            }
        });
        let (c, share) = backdrop_color(&img).expect("a green backdrop");
        assert!(c[1] > 0.6 && c[0] < 0.25 && c[2] < 0.3, "{c:?}");
        assert!(share > 0.8, "{share}");
        // blue screens too
        let img = drawn(160, 120, |x, _| [20, 40 + (x / 16) as u8, 200]);
        let (c, _) = backdrop_color(&img).expect("a blue backdrop");
        assert!(c[2] > 0.7 && c[0] < 0.2, "{c:?}");
    }

    #[test]
    fn a_wall_or_a_busy_scene_is_no_backdrop() {
        // white, grey and black have no colour to key
        for v in [250, 128, 5] {
            assert_eq!(backdrop_color(&drawn(80, 60, |_, _| [v, v, v])), None);
        }
        // every hue around the border: none has a quarter of it
        let img = drawn(180, 120, |x, y| {
            let t = (x as f32 / 180.0 + y as f32 / 120.0) * std::f32::consts::PI * 3.0;
            let ch = |o: f32| (127.0 + 120.0 * (t + o).sin()) as u8;
            [ch(0.0), ch(2.1), ch(4.2)]
        });
        assert_eq!(backdrop_color(&img), None);
        // too small to tell
        assert_eq!(backdrop_color(&drawn(4, 4, |_, _| [0, 255, 0])), None);
    }
}
