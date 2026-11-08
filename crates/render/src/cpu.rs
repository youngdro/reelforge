//! The CPU reference compositor: draws a [`FramePlan`] in linear light, premultiplied.
//!
//! Slow and simple on purpose. It is the oracle the wgpu compositor is tested against, and the
//! fallback where there is no GPU.

use reelforge_renderspec::{Blend, RenderSpec};

use crate::plan::{FramePlan, PlannedLayer, PlannedSource, invert};
use crate::{FrameReport, Image, MediaSource, Quality, RenderError};

/// Offline slate colour: a dark red, so a missing picture is obvious.
const SLATE: [f32; 4] = [0.18, 0.02, 0.03, 1.0];

/// Draws `plan`. Media pictures come from `media`.
pub fn composite(
    spec: &RenderSpec,
    plan: &FramePlan,
    media: &dyn MediaSource,
    quality: Quality,
) -> Result<(Image, FrameReport), RenderError> {
    let mut out = Image::filled(plan.width, plan.height, plan.background);
    let mut report = FrameReport::default();
    let target = [plan.width as f32, plan.height as f32];
    for layer in &plan.layers {
        let mut placeholder = |reason: &str| -> Result<(), RenderError> {
            if quality == Quality::Final {
                return Err(RenderError::Missing { layer: layer.id.clone(), reason: reason.to_string() });
            }
            report.placeholders.push((layer.id.clone(), reason.to_string()));
            Ok(())
        };
        match &layer.source {
            PlannedSource::Solid(color) => {
                let pic = Image::filled(1, 1, *color);
                draw(&mut out, layer, &pic, target, true);
            }
            PlannedSource::Media { asset, at } => {
                let Some(def) = spec.assets.get(asset) else {
                    return Err(RenderError::Media(format!("unknown asset {asset:?}")));
                };
                match media.picture(asset, def, *at).map_err(RenderError::Media)? {
                    Some(pic) => draw(&mut out, layer, &pic, target, false),
                    None => {
                        placeholder("media offline or not decodable yet")?;
                        let (w, h) = def
                            .probe
                            .as_ref()
                            .and_then(|p| Some((p.width?, p.height?)))
                            .unwrap_or((plan.width, plan.height));
                        draw(&mut out, layer, &Image::filled(w, h, SLATE), target, false);
                    }
                }
            }
            PlannedSource::Text(text) => {
                let k = plan.width as f32 / spec.output.width as f32;
                let family = match &text.font {
                    None => crate::text::DEFAULT_FAMILY.to_string(),
                    Some(font) => {
                        let Some(def) = spec.assets.get(font) else {
                            return Err(RenderError::Media(format!("unknown asset {font:?}")));
                        };
                        match media.font(font, def).map_err(RenderError::Media)? {
                            Some(bytes) => crate::text::font_family(&def.sha256, &bytes).map_err(RenderError::Media)?,
                            None => {
                                placeholder("font offline; drawn with the default face")?;
                                crate::text::DEFAULT_FAMILY.to_string()
                            }
                        }
                    }
                };
                crate::text::draw_text(&mut out, layer, text, &family, k);
            }
        }
    }
    Ok((out, report))
}

/// Draws `pic` fitted per the layer's fit. `fill` stretches a 1-pixel picture to the frame.
fn draw(out: &mut Image, layer: &PlannedLayer, pic: &Image, target: [f32; 2], fill: bool) {
    if fill {
        draw_sized(out, layer, pic, target);
        return;
    }
    let m = layer.matrix(pic.width as f32, pic.height as f32, target);
    blit(out, layer, pic, m, false);
}

/// Draws a flat picture as a box of `size` target pixels placed by the layer.
fn draw_sized(out: &mut Image, layer: &PlannedLayer, pic: &Image, size: [f32; 2]) {
    let mut l = layer.clone();
    l.fit = reelforge_renderspec::Fit::Stretch;
    // A 1x1 picture stretched to `size`: matrix for a `size` box, then scale content down.
    let m = l.matrix(1.0, 1.0, size);
    blit(out, &l, pic, m, true);
}

fn blit(out: &mut Image, layer: &PlannedLayer, pic: &Image, m: [f32; 6], flat: bool) {
    let Some(inv) = invert(m) else { return };
    let (pw, ph) = (pic.width as f32, pic.height as f32);
    let (cw, ch) = if flat { (1.0, 1.0) } else { (pw, ph) };
    // Bounding box of the placed content.
    let corners = [(0.0, 0.0), (cw, 0.0), (0.0, ch), (cw, ch)]
        .map(|(x, y)| (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]));
    let x0 = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let x1 = corners.iter().map(|c| c.0).fold(f32::NEG_INFINITY, f32::max).ceil().min(out.width as f32) as u32;
    let y0 = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let y1 = corners.iter().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max).ceil().min(out.height as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (sx, sy) = (inv[0] * px + inv[1] * py + inv[2], inv[3] * px + inv[4] * py + inv[5]);
            if sx < 0.0 || sy < 0.0 || sx >= cw || sy >= ch {
                continue;
            }
            let mut s = if flat { pic.pixels[0] } else { pic.sample(sx, sy) };
            apply_fx(&mut s, &layer.fx);
            for c in &mut s {
                *c *= layer.opacity;
            }
            let i = (y * out.width + x) as usize;
            out.pixels[i] = blend(layer.blend, s, out.pixels[i]);
        }
    }
}

pub(crate) fn apply_fx(p: &mut [f32; 4], fx: &[(String, Vec<(String, f32)>)]) {
    for (id, params) in fx {
        let get = |name: &str, default: f32| params.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v);
        match id.as_str() {
            "color.brightness@1" => {
                let amount = get("amount", 0.0);
                let a = p[3];
                for c in &mut p[..3] {
                    *c = (*c + amount * a).max(0.0);
                }
            }
            "color.saturation@1" => {
                let amount = get("amount", 1.0);
                let luma = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                for c in &mut p[..3] {
                    *c = (luma + (*c - luma) * amount).max(0.0);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn blend(mode: Blend, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0; 4];
    for i in 0..4 {
        o[i] = match mode {
            Blend::Normal => s[i] + d[i] * (1.0 - s[3]),
            Blend::Add if i < 3 => s[i] + d[i],
            Blend::Multiply if i < 3 => s[i] * d[i] + s[i] * (1.0 - d[3]) + d[i] * (1.0 - s[3]),
            Blend::Screen if i < 3 => s[i] + d[i] - s[i] * d[i],
            _ => s[3] + d[3] * (1.0 - s[3]),
        };
    }
    o
}
