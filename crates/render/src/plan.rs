//! What is on screen at an instant. Pure: no pixels, no IO.

use reelforge_renderspec::{Blend, Fit, LayerContent, RenderSpec, TextContent};
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
    },
    /// A colour filling the frame, premultiplied linear.
    Solid([f32; 4]),
    /// Text (not drawn by this build yet: a placeholder in drafts).
    Text(TextContent),
}

/// A 2-D affine map `[a, b, c, d, e, f]`: `x' = a x + b y + c`, `y' = d x + e y + f`.
pub type Affine = [f32; 6];

impl PlannedLayer {
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
    let mut layers = Vec::new();
    for layer in &spec.layers {
        if t < layer.at || t >= layer.at + layer.dur {
            continue;
        }
        let local = t - layer.at;
        let (source, fit) = match &layer.content {
            LayerContent::Media { asset, time, fit } => {
                (PlannedSource::Media { asset: asset.clone(), at: time.source_at(local) }, *fit)
            }
            LayerContent::Solid { color } => (PlannedSource::Solid(premultiply_srgb(color.0)), Fit::Stretch),
            LayerContent::Text(text) => (PlannedSource::Text(text.clone()), Fit::Contain),
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
        });
    }
    FramePlan { t, width, height, background: premultiply_srgb(o.background.0), layers }
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
}
