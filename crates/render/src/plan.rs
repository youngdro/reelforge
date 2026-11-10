//! What is on screen at an instant. Pure: no pixels, no IO.

use reelforge_renderspec::{Animated, Between, Blend, Fit, LayerContent, RenderSpec, ShapeMask, TextContent, effects};
use reelforge_time::Tick;

use crate::image::premultiply_srgb;

/// Everything a compositor needs to draw one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FramePlan {
    /// The output instant.
    pub t: Tick,
    /// Target width in pixels (output width times the scale).
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Background, premultiplied linear.
    pub background: [f32; 4],
    /// Visible layers, bottom first.
    pub layers: Vec<PlannedLayer>,
}

/// One visible layer at the frame's instant, its animated values already evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedLayer {
    /// The layer id.
    pub id: String,
    /// What to draw.
    pub source: PlannedSource,
    /// Fit of the content into the frame.
    pub fit: Fit,
    /// Centre of the layer in target pixels.
    pub center: [f32; 2],
    /// Scale on top of the fit.
    pub scale: f32,
    /// Clockwise rotation, radians.
    pub rotation: f32,
    /// Opacity 0 to 1.
    pub opacity: f32,
    /// Blend mode.
    pub blend: Blend,
    /// Effects with their parameters at this instant: `(id, [(name, value)])`.
    pub fx: Vec<(String, Vec<(String, f32)>)>,
    /// The `lut` asset of the layer's `color.lut@1`, if it has one.
    pub lut: Option<String>,
    /// Fractions cut off the picture: left, top, right, bottom.
    pub crop: [f32; 4],
    /// The picture mirrored: left to right, top to bottom.
    pub flip: [bool; 2],
    /// A shape that shows part of the layer.
    pub shape: Option<ShapeMask>,
}

/// What a layer draws.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedSource {
    /// A picture of an asset at a source instant.
    Media {
        /// The asset id.
        asset: String,
        /// The source instant.
        at: Tick,
        /// What shows when that instant is between two frames of a video.
        between: Between,
    },
    /// A colour filling the frame, premultiplied linear.
    Solid([f32; 4]),
    /// Text, laid out and drawn by the compositor.
    Text(Box<TextContent>),
    /// A composition's frame, drawn first on a see-through frame of its own and then placed as a
    /// picture: its layers at its own instant, at the size of the frame it goes into.
    Comp(Box<FramePlan>),
}

impl FramePlan {
    /// Every layer that shows a picture of an asset, the ones inside compositions included, each
    /// with the frame it is drawn into.
    pub fn media_layers(&self) -> Vec<(&FramePlan, &PlannedLayer)> {
        let mut out = Vec::new();
        for layer in &self.layers {
            match &layer.source {
                PlannedSource::Media { .. } => out.push((self, layer)),
                PlannedSource::Comp(inner) => out.extend(inner.media_layers()),
                PlannedSource::Solid(_) | PlannedSource::Text(_) => {}
            }
        }
        out
    }
}

/// How deep compositions may show compositions: a spec that passed validation has no loop, and
/// one that did not stops here.
const DEPTH: usize = 8;

/// A 2-D affine map `[a, b, c, d, e, f]`: `x' = a x + b y + c`, `y' = d x + e y + f`.
pub type Affine = [f32; 6];

impl PlannedLayer {
    /// The part of a `w` by `h` picture that shows, after the crop.
    pub fn visible(&self, w: f32, h: f32) -> (f32, f32) {
        let [l, t, r, b] = self.crop;
        ((w * (1.0 - l - r)).max(1e-3), (h * (1.0 - t - b)).max(1e-3))
    }

    /// Where a point of the visible part (`x`, `y` in its pixels, `cw` by `ch`) is in the whole `w`
    /// by `h` picture: mirrored, then moved past the crop.
    pub fn in_picture(&self, x: f32, y: f32, cw: f32, ch: f32, w: f32, h: f32) -> (f32, f32) {
        let x = if self.flip[0] { cw - x } else { x };
        let y = if self.flip[1] { ch - y } else { y };
        (x + self.crop[0] * w, y + self.crop[1] * h)
    }

    /// How many times smaller (1, 2, 4 or 8 each way) a `w` by `h` picture can be and still have
    /// a pixel for every target pixel it covers: a preview drawn small, a layer scaled down.
    pub fn shrink(&self, w: f32, h: f32, target: [f32; 2]) -> u32 {
        let (cw, ch) = self.visible(w, h);
        let m = self.matrix(cw, ch, target);
        // target pixels per picture pixel, along each of the picture's axes
        let per = m[0].hypot(m[3]).max(m[1].hypot(m[4]));
        let mut k = 1;
        while k < 8 && per * (2 * k) as f32 <= 1.0 {
            k *= 2;
        }
        k
    }

    /// The map from content pixels (a `w` by `h` picture) to target pixels.
    pub fn matrix(&self, w: f32, h: f32, target: [f32; 2]) -> Affine {
        let (sx, sy) = match self.fit {
            Fit::Contain => {
                let s = (target[0] / w).min(target[1] / h);
                (s, s)
            }
            Fit::Cover => {
                let s = (target[0] / w).max(target[1] / h);
                (s, s)
            }
            Fit::Stretch => (target[0] / w, target[1] / h),
        };
        let (sx, sy) = (sx * self.scale, sy * self.scale);
        let (sin, cos) = self.rotation.sin_cos();
        // translate(center) · rotate · scale · translate(-w/2, -h/2)
        let (a, b, d, e) = (cos * sx, -sin * sy, sin * sx, cos * sy);
        let (ox, oy) = (-w / 2.0, -h / 2.0);
        [a, b, a * ox + b * oy + self.center[0], d, e, d * ox + e * oy + self.center[1]]
    }
}

/// Inverts an affine map; `None` when it collapses (scale 0).
pub fn invert(m: Affine) -> Option<Affine> {
    let det = m[0] * m[4] - m[1] * m[3];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, d, e) = (m[4] / det, -m[1] / det, -m[3] / det, m[0] / det);
    Some([a, b, -(a * m[2] + b * m[5]), d, e, -(d * m[2] + e * m[5])])
}

/// Plans the frame at `t`, drawn at `scale` times the output size.
pub fn plan_frame(spec: &RenderSpec, t: Tick, scale: f32) -> FramePlan {
    let o = &spec.output;
    let width = ((o.width as f32 * scale).round() as u32).max(1);
    let height = ((o.height as f32 * scale).round() as u32).max(1);
    let layers = plan_layers(spec, &spec.layers, t, [width, height], 0);
    FramePlan { t, width, height, background: premultiply_srgb(o.background.0), layers }
}

/// The layers of `all` on screen at `t`, for a frame of `size` pixels.
fn plan_layers(
    spec: &RenderSpec,
    all: &[reelforge_renderspec::Layer],
    t: Tick,
    size: [u32; 2],
    depth: usize,
) -> Vec<PlannedLayer> {
    let [width, height] = size;
    let mut layers = Vec::new();
    for layer in all {
        if t < layer.at || t >= layer.at + layer.dur {
            continue;
        }
        let local = t - layer.at;
        let (source, fit) = match &layer.content {
            LayerContent::Media { asset, time, fit, between } => {
                let at = time.source_at(local);
                (PlannedSource::Media { asset: asset.clone(), at, between: *between }, *fit)
            }
            LayerContent::Solid { color } => (PlannedSource::Solid(premultiply_srgb(color.0)), Fit::Stretch),
            LayerContent::Text(text) => {
                let mut text = text.clone();
                // how much shows is a value of this instant: the sprite is drawn for it
                if let Some(r) = &text.reveal {
                    text.reveal = Some(Animated::Constant(r.sample(local).clamp(0.0, 1.0)));
                }
                // and so is how far along its characters are
                for m in &mut text.chars {
                    m.progress = Animated::Constant(m.progress.sample(local));
                }
                (PlannedSource::Text(text), Fit::Contain)
            }
            LayerContent::Comp { comp, from } => {
                let (Some(inner), true) = (spec.comps.get(comp), depth < DEPTH) else { continue };
                let at = *from + local;
                let layers = plan_layers(spec, &inner.layers, at, size, depth + 1);
                if layers.is_empty() {
                    // past its end, or between its clips: nothing to place
                    continue;
                }
                let frame = FramePlan { t: at, width, height, background: [0.0; 4], layers };
                (PlannedSource::Comp(Box::new(frame)), Fit::Contain)
            }
        };
        let pos = layer.transform.position.sample(local);
        layers.push(PlannedLayer {
            id: layer.id.clone(),
            source,
            fit,
            center: [width as f32 * (0.5 + pos[0]), height as f32 * (0.5 + pos[1])],
            scale: layer.transform.scale.sample(local),
            rotation: layer.transform.rotation.sample(local).to_radians(),
            opacity: layer.opacity.sample(local).clamp(0.0, 1.0),
            blend: layer.blend,
            fx: layer
                .fx
                .iter()
                .map(|fx| (fx.id.clone(), fx.params.iter().map(|(k, v)| (k.clone(), v.sample(local))).collect()))
                .collect(),
            lut: layer.fx.iter().find(|fx| fx.id == effects::LUT).and_then(|fx| fx.asset.clone()),
            crop: match (&layer.content, layer.crop_keys.as_ref().map(|k| k.sample(local)).or(layer.crop)) {
                (LayerContent::Media { .. } | LayerContent::Comp { .. }, Some(c)) => [c.left, c.top, c.right, c.bottom],
                _ => [0.0; 4],
            },
            flip: match &layer.content {
                LayerContent::Media { .. } | LayerContent::Comp { .. } => [layer.flip_x, layer.flip_y],
                _ => [false; 2],
            },
            shape: layer.shape_mask.map(|m| layer.shape_mask_keys.as_ref().map_or(m, |k| k.sample(m, local))),
        });
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_inverts() {
        let l = PlannedLayer {
            id: "x".into(),
            source: PlannedSource::Solid([0.0; 4]),
            fit: Fit::Contain,
            center: [100.0, 200.0],
            scale: 0.5,
            rotation: 0.3,
            opacity: 1.0,
            blend: Blend::Normal,
            fx: vec![],
            lut: None,
            crop: [0.0; 4],
            flip: [false; 2],
            shape: None,
        };
        let m = l.matrix(640.0, 360.0, [1080.0, 1920.0]);
        let inv = invert(m).unwrap();
        let (x, y) = (12.0f32, 34.0f32);
        let (px, py) = (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]);
        let (bx, by) = (inv[0] * px + inv[1] * py + inv[2], inv[3] * px + inv[4] * py + inv[5]);
        assert!((bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3);
        // The content's centre lands on the layer's centre.
        let (cx, cy) = (m[0] * 320.0 + m[1] * 180.0 + m[2], m[3] * 320.0 + m[4] * 180.0 + m[5]);
        assert!((cx - 100.0).abs() < 1e-3 && (cy - 200.0).abs() < 1e-3);
    }

    #[test]
    fn keyed_crops_and_masks_are_planned_at_the_instant() {
        use reelforge_renderspec::{CropKeys, Key, MaskShape, ShapeMaskKeys};
        let mut spec = crate::tests::spec();
        spec.layers[0].content = LayerContent::Media {
            asset: "a".into(),
            time: Default::default(),
            fit: Fit::Contain,
            between: Default::default(),
        };
        fn key<T>(t: f64, v: T) -> Key<T> {
            Key { t: Tick::from_seconds(t), v, ease: Default::default() }
        }
        let l = &mut spec.layers[0];
        l.crop_keys = Some(CropKeys {
            left: Animated::Keys(vec![key(0.0, 0.0), key(1.0, 0.8)]),
            top: Animated::Constant(0.1),
            right: Animated::Constant(0.4),
            bottom: Animated::Constant(0.0),
        });
        l.shape_mask = Some(ShapeMask {
            shape: MaskShape::Circle,
            center: [0.5, 0.5],
            size: [0.5, 0.5],
            rotation: 0.0,
            feather: 0.0,
            invert: true,
        });
        l.shape_mask_keys = Some(ShapeMaskKeys {
            center: Animated::Keys(vec![key(0.0, [0.0, 0.5]), key(1.0, [1.0, 0.5])]),
            size: Animated::Constant([0.2, 0.2]),
            rotation: Animated::Constant(0.0),
            feather: Animated::Constant(0.0),
        });
        let at = |t: f64| plan_frame(&spec, Tick::from_seconds(t), 1.0).layers.remove(0);
        let half = at(0.5);
        assert!((half.crop[0] - 0.4).abs() < 1e-6 && half.crop[1] == 0.1);
        let m = half.shape.unwrap();
        assert!((m.center[0] - 0.5).abs() < 1e-6 && m.size == [0.2, 0.2] && m.invert);
        // keys that would cut everything leave 1 % of the picture, shared between the two edges
        let end = at(0.999);
        assert!(end.crop[0] + end.crop[2] <= 0.99 + 1e-6);
    }
}
