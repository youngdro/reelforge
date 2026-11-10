//! The CPU reference compositor: draws a [`FramePlan`] in linear light, premultiplied.
//!
//! Slow and simple on purpose. It is the oracle the wgpu compositor is tested against, and the
//! fallback where there is no GPU.

use std::sync::Arc;

use reelforge_renderspec::{Blend, RenderSpec, effects};

use crate::distort::Warp;
use crate::plan::{FramePlan, PlannedLayer, PlannedSource, invert};
use crate::text::Sprite;
use crate::{FrameReport, Image, Lut, MediaSource, Quality, RenderError};

/// Offline slate colour: a dark red, so a missing picture is obvious.
pub(crate) const SLATE: [f32; 4] = [0.18, 0.02, 0.03, 1.0];

/// What one planned layer draws, its pictures fetched and its text drawn. Both compositors start
/// from this, so they agree on what is missing and how it is reported.
pub(crate) enum Resolved {
    /// A picture fitted by the layer.
    Picture(Arc<Image>),
    /// A video picture still in Y'CbCr, fitted by the layer (only when asked for).
    Yuv(Arc<crate::Yuv>),
    /// One colour: stretched over the frame (`None`), or a `w` by `h` picture of it fitted by the
    /// layer (the offline slate).
    Flat([f32; 4], Option<(u32, u32)>),
    /// Drawn text, pixel-aligned to the target.
    Sprite(Sprite),
    /// Nothing lands in the frame.
    Nothing,
}

/// What a compositor can take from [`resolve`].
#[derive(Copy, Clone, Debug)]
pub(crate) struct Takes {
    /// Video pictures still in Y'CbCr.
    pub yuv: bool,
    /// The longest side a picture may have (a GPU's texture limit).
    pub side: u32,
}

impl Takes {
    /// The CPU compositor's: converted pictures of any size.
    pub const CPU: Takes = Takes { yuv: false, side: u32::MAX };
}

/// Fetches what `layer` needs and draws its text. Placeholders go in `report` (drafts) or fail
/// the frame (finals). A video picture may come back unconverted, when the compositor `takes`
/// that, the media source has it so and nothing has to be done to it on the CPU; one larger than
/// the compositor takes is asked for smaller.
pub(crate) fn resolve(
    spec: &RenderSpec,
    plan: &FramePlan,
    layer: &PlannedLayer,
    media: &dyn MediaSource,
    quality: Quality,
    takes: Takes,
    report: &mut FrameReport,
) -> Result<Resolved, RenderError> {
    let mut placeholder = |reason: &str| -> Result<(), RenderError> {
        if quality == Quality::Final {
            return Err(RenderError::Missing { layer: layer.id.clone(), reason: reason.to_string() });
        }
        report.placeholders.push((layer.id.clone(), reason.to_string()));
        Ok(())
    };
    Ok(match &layer.source {
        PlannedSource::Solid(color) => Resolved::Flat(*color, None),
        PlannedSource::Media { asset, at } => {
            let Some(def) = spec.assets.get(asset) else {
                return Err(RenderError::Media(format!("unknown asset {asset:?}")));
            };
            let size = def.probe.as_ref().and_then(|p| Some((p.width?, p.height?)));
            // a picture too big for the compositor is asked for smaller
            let mut fit = 1;
            if let Some((w, h)) = size {
                while fit < 64 && w.max(h).div_ceil(fit) > takes.side {
                    fit *= 2;
                }
            }
            if takes.yuv
                && fit == 1
                && blur_radius(layer).is_none()
                && let Some(y) = media.yuv(asset, def, *at).map_err(RenderError::Media)?
                && y.is_whole()
            {
                return Ok(Resolved::Yuv(y));
            }
            let shrink = if quality == Quality::Draft { draft_shrink(def, plan, layer).max(fit) } else { fit };
            let pic =
                if shrink > 1 { media.picture_shrunk(asset, def, *at, shrink) } else { media.picture(asset, def, *at) };
            match pic.map_err(RenderError::Media)? {
                Some(pic) => match blur_radius(layer) {
                    Some(r) => Resolved::Picture(std::sync::Arc::new(crate::image::blurred(&pic, r))),
                    None => Resolved::Picture(pic),
                },
                None => {
                    placeholder("media offline or not decodable yet")?;
                    let size = def
                        .probe
                        .as_ref()
                        .and_then(|p| Some((p.width?, p.height?)))
                        .unwrap_or((plan.width, plan.height));
                    Resolved::Flat(SLATE, Some(size))
                }
            }
        }
        PlannedSource::Comp(inner) => {
            // the composition's own frame, then a picture like any other
            let (pic, drawn) = composite(spec, inner, media, quality)?;
            report.placeholders.extend(drawn.placeholders);
            match blur_radius(layer) {
                Some(r) => Resolved::Picture(Arc::new(crate::image::blurred(&pic, r))),
                None => Resolved::Picture(Arc::new(pic)),
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
            match crate::text::rasterize(layer, text, &family, k, [plan.width, plan.height]) {
                Some(sp) => Resolved::Sprite(sp),
                None => Resolved::Nothing,
            }
        }
    })
}

/// How many times smaller a draft may fetch the picture of `asset` for `layer`
/// ([`MediaSource::picture_shrunk`]): from the size the asset was probed at, 1 when unknown.
pub fn draft_shrink(asset: &reelforge_renderspec::Asset, plan: &FramePlan, layer: &PlannedLayer) -> u32 {
    match asset.probe.as_ref().and_then(|p| Some((p.width?, p.height?))) {
        Some((w, h)) => layer.shrink(w as f32, h as f32, [plan.width as f32, plan.height as f32]),
        None => 1,
    }
}

/// The radius of `layer`'s `blur.gaussian@1`, as a fraction of its picture's shorter side.
pub(crate) fn blur_radius(layer: &PlannedLayer) -> Option<f32> {
    let (_, params) = layer.fx.iter().find(|(id, _)| id == effects::BLUR)?;
    let r = params.iter().find(|(k, _)| k == "radius").map_or(0.02, |(_, v)| *v);
    (r > 0.0).then_some(r)
}

/// The lookup table `layer` reads, if any. Missing, it is skipped and reported (drafts) or fails
/// the frame (finals).
pub(crate) fn resolve_lut(
    spec: &RenderSpec,
    layer: &PlannedLayer,
    media: &dyn MediaSource,
    quality: Quality,
    report: &mut FrameReport,
) -> Result<Option<Arc<Lut>>, RenderError> {
    let Some(id) = &layer.lut else { return Ok(None) };
    let Some(def) = spec.assets.get(id) else {
        return Err(RenderError::Media(format!("unknown asset {id:?}")));
    };
    match media.lut(id, def).map_err(RenderError::Media)? {
        Some(lut) => Ok(Some(lut)),
        None if quality == Quality::Final => {
            Err(RenderError::Missing { layer: layer.id.clone(), reason: "lookup table offline".into() })
        }
        None => {
            report.placeholders.push((layer.id.clone(), "lookup table offline; drawn without it".into()));
            Ok(None)
        }
    }
}

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
        let what = resolve(spec, plan, layer, media, quality, Takes::CPU, &mut report)?;
        let lut = resolve_lut(spec, layer, media, quality, &mut report)?;
        let lut = lut.as_deref();
        match what {
            Resolved::Flat(color, None) => draw(&mut out, layer, &Image::filled(1, 1, color), target, true, lut),
            Resolved::Flat(color, Some((w, h))) => {
                draw(&mut out, layer, &Image::filled(w, h, color), target, false, lut)
            }
            Resolved::Picture(pic) => draw(&mut out, layer, &pic, target, false, lut),
            Resolved::Yuv(y) => draw(&mut out, layer, &y.to_image(), target, false, lut),
            Resolved::Sprite(sp) => crate::text::draw_sprite(&mut out, layer, &sp, lut),
            Resolved::Nothing => {}
        }
    }
    Ok((out, report))
}

/// Draws `pic` fitted per the layer's fit. `fill` stretches a 1-pixel picture to the frame.
fn draw(out: &mut Image, layer: &PlannedLayer, pic: &Image, target: [f32; 2], fill: bool, lut: Option<&Lut>) {
    if fill {
        draw_sized(out, layer, pic, target, lut);
        return;
    }
    let (cw, ch) = layer.visible(pic.width as f32, pic.height as f32);
    let m = layer.matrix(cw, ch, target);
    blit(out, layer, pic, m, None, lut);
}

/// Draws a flat picture as a box of `size` target pixels placed by the layer.
fn draw_sized(out: &mut Image, layer: &PlannedLayer, pic: &Image, size: [f32; 2], lut: Option<&Lut>) {
    let mut l = layer.clone();
    l.fit = reelforge_renderspec::Fit::Stretch;
    // A 1x1 picture stretched to `size`: matrix for a `size` box, then scale content down.
    let m = l.matrix(1.0, 1.0, size);
    blit(out, &l, pic, m, Some(size), lut);
}

/// Draws `pic` mapped by `m`. `flat` is a 1-pixel picture stretched over a box of that size.
fn blit(out: &mut Image, layer: &PlannedLayer, pic: &Image, m: [f32; 6], flat: Option<[f32; 2]>, lut: Option<&Lut>) {
    let Some(inv) = invert(m) else { return };
    let (pw, ph) = (pic.width as f32, pic.height as f32);
    let (cw, ch) = if flat.is_some() { (1.0, 1.0) } else { layer.visible(pw, ph) };
    // a mask is shaped on the picture as it is seen
    let [mw, mh] = flat.unwrap_or([cw, ch]);
    // Bounding box of the placed content.
    let corners = [(0.0, 0.0), (cw, 0.0), (0.0, ch), (cw, ch)]
        .map(|(x, y)| (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]));
    let x0 = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let x1 = corners.iter().map(|c| c.0).fold(f32::NEG_INFINITY, f32::max).ceil().min(out.width as f32) as u32;
    let y0 = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min).floor().max(0.0) as u32;
    let y1 = corners.iter().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max).ceil().min(out.height as f32) as u32;
    let warp = if flat.is_some() { None } else { Warp::of(&layer.fx) };
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (sx, sy) = (inv[0] * px + inv[1] * py + inv[2], inv[3] * px + inv[4] * py + inv[5]);
            if sx < 0.0 || sy < 0.0 || sx >= cw || sy >= ch {
                continue;
            }
            let uv = [sx / cw, sy / ch];
            let read = |sx: f32, sy: f32| {
                let (px, py) = layer.in_picture(sx, sy, cw, ch, pw, ph);
                pic.sample(px, py)
            };
            let mut s = match &warp {
                _ if flat.is_some() => pic.pixels[0],
                Some(w) => w.sample(uv, [cw, ch], read),
                None => read(sx, sy),
            };
            apply_fx(&mut s, &layer.fx, uv, lut);
            let k = layer.opacity * layer.shape.map_or(1.0, |m| m.coverage(uv, mw, mh));
            for c in &mut s {
                *c *= k;
            }
            let i = (y * out.width + x) as usize;
            out.pixels[i] = blend(layer.blend, s, out.pixels[i]);
        }
    }
}

/// Applies a layer's effects to one premultiplied linear pixel at `uv` (0 to 1 across the layer's
/// picture). `lut` is the table of its `color.lut@1`.
pub(crate) fn apply_fx(p: &mut [f32; 4], fx: &[(String, Vec<(String, f32)>)], uv: [f32; 2], lut: Option<&Lut>) {
    for (id, params) in fx {
        let get = |name: &str, default: f32| params.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v);
        match id.as_str() {
            effects::BRIGHTNESS => {
                let amount = get("amount", 0.0);
                let a = p[3];
                for c in &mut p[..3] {
                    *c = (*c + amount * a).max(0.0);
                }
            }
            effects::SATURATION => {
                let amount = get("amount", 1.0);
                let luma = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                for c in &mut p[..3] {
                    *c = (luma + (*c - luma) * amount).max(0.0);
                }
            }
            effects::GRADE => grade(p, effects::GRADE_PARAMS.map(|n| get(n, 0.0)), uv),
            effects::LUT => {
                if let Some(lut) = lut {
                    lut.apply(p, get("amount", 1.0).clamp(0.0, 1.0));
                }
            }
            effects::CHROMA_KEY => chroma_key(
                p,
                [get("r", 0.0), get("g", 1.0), get("b", 0.0)],
                [get("tolerance", 0.3), get("softness", 0.1), get("spill", 0.5)],
            ),
            _ => {}
        }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `key.chroma@1` on a premultiplied linear pixel: `key` is sRGB-encoded, `how` is the tolerance,
/// the softness and the spill. `composite.wgsl` does the same, step for step.
pub(crate) fn chroma_key(p: &mut [f32; 4], key: [f32; 3], how: [f32; 3]) {
    let a = p[3];
    if a <= 0.0 {
        return;
    }
    let [tolerance, softness, spill] = how.map(|v| v.clamp(0.0, 1.0));
    let c = [p[0] / a, p[1] / a, p[2] / a];
    let chroma = |c: [f32; 3]| {
        [-0.168_736 * c[0] - 0.331_264 * c[1] + 0.5 * c[2], 0.5 * c[0] - 0.418_688 * c[1] - 0.081_312 * c[2]]
    };
    let e = c.map(|v| crate::image::linear_to_srgb(v.clamp(0.0, 1.0)));
    let (k, q) = (chroma(key), chroma(e));
    let d = ((q[0] - k[0]) * (q[0] - k[0]) + (q[1] - k[1]) * (q[1] - k[1])).sqrt();
    let keep = if softness > 0.0 {
        smoothstep(tolerance, tolerance + softness, d)
    } else if d > tolerance {
        1.0
    } else {
        0.0
    };
    // what stays near the key loses the key's tint: its chroma towards the key, not its lightness
    let near = 1.0 - ((d - tolerance) / (softness + 0.3)).clamp(0.0, 1.0);
    let len = (k[0] * k[0] + k[1] * k[1]).sqrt();
    let towards = if len > 1e-4 { (q[0] * k[0] + q[1] * k[1]) / len } else { 0.0 };
    let mut c = c;
    if spill * near > 0.0 && towards > 0.0 {
        let off = -towards * spill * near / len;
        let (cb, cr) = (k[0] * off, k[1] * off);
        let e = [e[0] + 1.402 * cr, e[1] - 0.344_136 * cb - 0.714_136 * cr, e[2] + 1.772 * cb];
        c = e.map(|v| crate::image::srgb_to_linear(v.clamp(0.0, 1.0)));
    }
    for i in 0..3 {
        p[i] = c[i] * a * keep;
    }
    p[3] = a * keep;
}

/// `color.grade@1` on a premultiplied linear pixel; `composite.wgsl` does the same, step for step.
pub(crate) fn grade(p: &mut [f32; 4], g: [f32; 9], uv: [f32; 2]) {
    let [exposure, contrast, saturation, temperature, tint, highlights, shadows, fade, vignette] = g;
    let a = p[3];
    if a <= 0.0 {
        return;
    }
    let mut c = [p[0] / a, p[1] / a, p[2] / a];
    // exposure and white balance
    let e = exposure.clamp(-5.0, 5.0).exp2();
    let wb = [1.0 + 0.25 * temperature + 0.1 * tint, 1.0 - 0.2 * tint, 1.0 - 0.25 * temperature + 0.1 * tint];
    for i in 0..3 {
        c[i] = (c[i] * e * wb[i]).max(0.0);
    }
    // the dark and bright parts, weighed on a perceptual lightness
    let q = luma(c).max(0.0).sqrt().min(1.0);
    let ws = (1.0 - q / 0.6).clamp(0.0, 1.0);
    let wh = ((q - 0.4) / 0.6).clamp(0.0, 1.0);
    let k = (1.0 + shadows * ws * ws + 0.8 * highlights * wh * wh).max(0.0);
    // contrast around mid grey
    let power = 1.0 + contrast.clamp(-0.9, 1.0);
    for v in &mut c {
        *v = 0.18 * (*v * k / 0.18).powf(power);
    }
    let l = luma(c);
    let s = 1.0 + saturation.clamp(-1.0, 1.0);
    let lift = 0.04 * fade.clamp(0.0, 1.0);
    let d = ((uv[0] - 0.5) * (uv[0] - 0.5) + (uv[1] - 0.5) * (uv[1] - 0.5)).sqrt() * std::f32::consts::SQRT_2;
    let v = 1.0 - vignette.clamp(0.0, 1.0) * smoothstep(0.3, 1.0, d);
    for i in 0..3 {
        let x = (l + (c[i] - l) * s).max(0.0);
        p[i] = (x * (1.0 - 3.0 * lift) + lift) * v * a;
    }
}

pub(crate) fn blend(mode: Blend, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    if mode.is_v2() {
        // the W3C separable blend on straight colours, composited premultiplied
        let (sa, da) = (s[3], d[3]);
        let mut o = [0.0; 4];
        for i in 0..3 {
            let cs = if sa > 0.0 { s[i] / sa } else { 0.0 };
            let cd = if da > 0.0 { d[i] / da } else { 0.0 };
            o[i] = s[i] * (1.0 - da) + d[i] * (1.0 - sa) + sa * da * mode.mix(cs, cd);
        }
        o[3] = sa + da * (1.0 - sa);
        return o;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::premultiply_srgb;

    fn keyed(srgb: [f32; 3], how: [f32; 3]) -> [f32; 4] {
        let mut p = premultiply_srgb([srgb[0], srgb[1], srgb[2], 1.0]);
        chroma_key(&mut p, [0.0, 1.0, 0.0], how);
        p
    }

    #[test]
    fn a_keyed_colour_goes_and_the_rest_stays() {
        let how = [0.3, 0.1, 0.5];
        // the key and its lighter and darker shades are gone
        for green in [[0.0, 1.0, 0.0], [0.1, 0.85, 0.15], [0.05, 0.7, 0.05]] {
            assert_eq!(keyed(green, how), [0.0; 4], "{green:?}");
        }
        // skin, white, grey, black, red and blue are as they were
        for other in
            [[0.9, 0.7, 0.6], [1.0, 1.0, 1.0], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0], [0.9, 0.1, 0.1], [0.1, 0.2, 0.9]]
        {
            let p = keyed(other, how);
            let was = premultiply_srgb([other[0], other[1], other[2], 1.0]);
            assert_eq!(p[3], 1.0, "{other:?}");
            for i in 0..3 {
                assert!((p[i] - was[i]).abs() < 1e-5, "{other:?}: {p:?} was {was:?}");
            }
        }
        // between the two the pixel is part see-through, more so with more tolerance
        let edge = [0.35, 0.75, 0.3];
        let a = keyed(edge, [0.2, 0.2, 0.0])[3];
        assert!(a > 0.0 && a < 1.0, "{a}");
        assert!(keyed(edge, [0.3, 0.2, 0.0])[3] < a);
        // with no softness it is one or the other
        assert!([0.0, 1.0].contains(&keyed(edge, [0.25, 0.0, 0.0])[3]));
    }

    #[test]
    fn spill_takes_the_keys_tint_off_what_stays_near_it() {
        // a greenish edge that is kept: with spill its green stands out less
        let edge = [0.45, 0.7, 0.4];
        let (plain, cleaned) = (keyed(edge, [0.3, 0.05, 0.0]), keyed(edge, [0.3, 0.05, 1.0]));
        assert!(plain[3] > 0.9 && cleaned[3] == plain[3]);
        let tint = |p: [f32; 4]| p[1] - (p[0] + p[2]) / 2.0;
        assert!(tint(cleaned) < tint(plain) * 0.6, "{} {}", tint(cleaned), tint(plain));
        // its lightness (of the encoded colour, as the eye has it) stays
        let light = |p: [f32; 4]| {
            let e = [p[0], p[1], p[2]].map(crate::image::linear_to_srgb);
            0.299 * e[0] + 0.587 * e[1] + 0.114 * e[2]
        };
        assert!((light(cleaned) - light(plain)).abs() < 1e-3, "{} {}", light(cleaned), light(plain));
        // half the spill takes half as much off
        let half = keyed(edge, [0.3, 0.05, 0.5]);
        assert!(tint(half) < tint(plain) && tint(half) > tint(cleaned));
        // far from the key nothing changes
        assert_eq!(keyed([0.9, 0.1, 0.1], [0.3, 0.05, 1.0]), keyed([0.9, 0.1, 0.1], [0.3, 0.05, 0.0]));
        // a see-through pixel keeps its share of light: premultiplied by what is left of it
        let mut half = premultiply_srgb([0.9, 0.1, 0.1, 0.5]);
        let was = half;
        chroma_key(&mut half, [0.0, 1.0, 0.0], [0.3, 0.1, 0.5]);
        for i in 0..4 {
            assert!((half[i] - was[i]).abs() < 1e-6);
        }
    }
}
