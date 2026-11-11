//! What a new canvas size does to the clips already on it: a picture that covered the old canvas
//! goes on covering the new one, and titles keep their size against the picture.

use reelforge_model::{Canvas, Clip, ClipContent, Project, Prop, TrackKind};

/// The ready-made shapes the editor offers, `(id, width, height)` at a 1080-pixel shorter side.
pub const RATIOS: [(&str, u32, u32); 7] = [
    ("9:16", 1080, 1920),
    ("16:9", 1920, 1080),
    ("1:1", 1080, 1080),
    ("4:3", 1440, 1080),
    ("3:4", 1080, 1440),
    ("2.35:1", 2538, 1080),
    ("4:5", 1080, 1350),
];

/// How much larger than it is drawn at scale 1 (fitted inside the canvas) a picture of `size`
/// must be to cover the canvas.
pub(crate) fn cover(canvas: [u32; 2], size: [u32; 2]) -> f32 {
    if size[0] == 0 || size[1] == 0 || canvas[0] == 0 || canvas[1] == 0 {
        return 1.0;
    }
    let (w, h) = (canvas[0] as f32 / size[0] as f32, canvas[1] as f32 / size[1] as f32);
    w.max(h) / w.min(h).max(f32::MIN_POSITIVE)
}

/// What [`refit`] changed.
#[derive(Default)]
pub(crate) struct Refit {
    /// Pictures scaled to go on covering the canvas.
    pub covered: Vec<String>,
    /// Titles resized with the canvas.
    pub titles: Vec<String>,
}

/// Fits the clips of every timeline of `proj` to `new` from `old`. A picture whose scale made it
/// cover `old` (or fill it, when it has the canvas's shape) is scaled to cover `new`, its scale
/// keyframes with it; titles are resized by the change of the canvas's shorter side.
pub(crate) fn refit(proj: &mut Project, old: &Canvas, new: &Canvas) -> Refit {
    let mut out = Refit::default();
    let (was, now) = ([old.width, old.height], [new.width, new.height]);
    if was == now {
        return out;
    }
    let text = new.width.min(new.height) as f32 / old.width.min(old.height).max(1) as f32;
    let sizes: Vec<(String, [u32; 2])> =
        proj.media.iter().filter_map(|(id, m)| m.size.map(|s| (id.clone(), s))).collect();
    let size_of = |id: &str| sizes.iter().find(|(m, _)| m == id).map(|(_, s)| *s);
    let tracks = proj.tracks.iter_mut().chain(proj.timelines.iter_mut().flat_map(|t| t.tracks.iter_mut()));
    for track in tracks {
        for clip in &mut track.clips {
            match &mut clip.content {
                ClipContent::Media { media, .. } if track.kind == TrackKind::Video => {
                    let Some(size) = size_of(media) else { continue };
                    let before = cover(was, size);
                    let base =
                        clip.keys.get(&Prop::Scale).and_then(|k| k.first()).map_or(clip.transform.scale, |k| k.v);
                    if (base - before).abs() <= 0.01 * before {
                        scale(clip, cover(now, size) / before);
                        out.covered.push(clip.id.clone());
                    }
                }
                ClipContent::Text { size, .. } if (text - 1.0).abs() > 1e-4 => {
                    *size = (*size * text).max(1.0);
                    out.titles.push(clip.id.clone());
                }
                _ => {}
            }
        }
    }
    out
}

/// Multiplies a clip's scale, and its scale keyframes, by `k`.
fn scale(clip: &mut Clip, k: f32) {
    clip.transform.scale *= k;
    if let Some(keys) = clip.keys.get_mut(&Prop::Scale) {
        for key in keys {
            key.v *= k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_is_one_for_the_canvas_shape() {
        assert_eq!(cover([1080, 1920], [720, 1280]), 1.0);
        // a landscape picture on a portrait canvas
        let k = cover([1080, 1920], [1920, 1080]);
        assert!((k - (1920.0f32 / 1080.0).powi(2)).abs() < 1e-4, "{k}");
    }

    #[test]
    fn ratios_are_even() {
        for (id, w, h) in RATIOS {
            assert!(w % 2 == 0 && h % 2 == 0, "{id}");
        }
    }
}
