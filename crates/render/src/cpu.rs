//! The CPU reference compositor: draws a [`FramePlan`] in linear light, premultiplied.
//!
//! Slow and simple on purpose. It is the oracle the wgpu compositor is tested against, and the
//! fallback where there is no GPU.

use std::sync::Arc;

use reelforge_renderspec::{Blend, RenderSpec, effects};

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
    /// One colour: stretched over the frame (`None`), or a `w` by `h` picture of it fitted by the
    /// layer (the offline slate).
    Flat([f32; 4], Option<(u32, u32)>),
    /// Drawn text, pixel-aligned to the target.
    Sprite(Sprite),
    /// Nothing lands in the frame.
    Nothing,
}

/// Fetches what `layer` needs and draws its text. Placeholders go in `report` (drafts) or fail
/// the frame (finals).
pub(crate) fn resolve(
    spec: &RenderSpec,
    plan: &FramePlan,
    layer: &PlannedLayer,
    media: &dyn MediaSource,
    quality: Quality,
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
            match media.picture(asset, def, *at).map_err(RenderError::Media)? {
                Some(pic) => Resolved::Picture(pic),
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
        let what = resolve(spec, plan, layer, media, quality, &mut report)?;
        let lut = resolve_lut(spec, layer, media, quality, &mut report)?;
        let lut = lut.as_deref();
        match what {
            Resolved::Flat(color, None) => draw(&mut out, layer, &Image::filled(1, 1, color), target, true, lut),
            Resolved::Flat(color, Some((w, h))) => {
                draw(&mut out, layer, &Image::filled(w, h, color), target, false, lut)
            }
            Resolved::Picture(pic) => draw(&mut out, layer, &pic, target, false, lut),
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
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (sx, sy) = (inv[0] * px + inv[1] * py + inv[2], inv[3] * px + inv[4] * py + inv[5]);
            if sx < 0.0 || sy < 0.0 || sx >= cw || sy >= ch {
                continue;
            }
            let mut s = if flat.is_some() {
                pic.pixels[0]
            } else {
                let (px, py) = layer.in_picture(sx, sy, cw, ch, pw, ph);
                pic.sample(px, py)
            };
            let uv = [sx / cw, sy / ch];
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
