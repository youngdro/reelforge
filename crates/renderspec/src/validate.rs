//! Checks a spec before anything renders it, so a renderer can assume a well-formed edit.

use std::collections::BTreeSet;

use reelforge_time::Tick;
use thiserror::Error;

use crate::{Animated, AssetKind, LayerContent, RenderSpec, SPEC_VERSION, TimeMap};

/// Why a spec was refused. The message names the field.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    /// The JSON did not match the format.
    #[error("not a RenderSpec: {0}")]
    Parse(String),
    /// A version this crate does not read.
    #[error("unsupported spec version {0:?} (this build reads {SPEC_VERSION:?})")]
    Version(String),
    /// Anything else, with the path of the offending field.
    #[error("{path}: {message}")]
    Invalid {
        /// Where, such as `layers[2].dur`.
        path: String,
        /// What is wrong.
        message: String,
    },
}

fn bad(path: impl Into<String>, message: impl Into<String>) -> ValidationError {
    ValidationError::Invalid { path: path.into(), message: message.into() }
}

/// Validates a spec. Returns the first problem found.
pub fn validate(spec: &RenderSpec) -> Result<(), ValidationError> {
    if spec.spec != SPEC_VERSION {
        return Err(ValidationError::Version(spec.spec.clone()));
    }
    let o = &spec.output;
    for (name, v) in [("width", o.width), ("height", o.height)] {
        if v == 0 || v > 8192 || v % 2 != 0 {
            return Err(bad(format!("output.{name}"), "must be even and between 2 and 8192"));
        }
    }
    if let Some(v) = &o.video {
        if v.bitrate_kbps == Some(0) || v.crf.is_some_and(|c| c > 51) {
            return Err(bad("output.video", "crf must be 0 to 51 and a bitrate above 0"));
        }
        if let Some([w, h]) = v.size
            && (w == 0
                || h == 0
                || w > 8192
                || h > 8192
                || w % 2 != 0
                || h % 2 != 0
                || h != crate::scaled_height(o.width, o.height, w))
        {
            return Err(bad("output.video.size", "must be even, at most 8192 and keep the output's aspect"));
        }
    }
    if !o.fps.is_valid() {
        return Err(bad("output.fps", "frame rate must be non-zero and a whole number of ticks per frame"));
    }
    if o.duration <= Tick::ZERO {
        return Err(bad("output.duration", "must be positive"));
    }
    color_ok("output.background", o.background.0)?;

    for (id, asset) in &spec.assets {
        let path = format!("assets.{id}");
        if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(bad(format!("{path}.sha256"), "must be 64 lowercase hex digits"));
        }
        if asset.uri.is_empty() {
            return Err(bad(format!("{path}.uri"), "must not be empty"));
        }
        if let Some(of) = &asset.of
            && !spec.assets.contains_key(of)
        {
            return Err(bad(format!("{path}.of"), format!("unknown asset {of:?}")));
        }
    }

    layers_ok(spec, "", &spec.layers)?;
    audio_ok(spec, "", &spec.audio)?;
    for (id, comp) in &spec.comps {
        let at = format!("comps.{id}.");
        layers_ok(spec, &at, &comp.layers)?;
        audio_ok(spec, &at, &comp.audio)?;
        if comp_reaches(spec, id, id) {
            return Err(bad(format!("comps.{id}"), "a composition cannot show or play itself"));
        }
    }
    Ok(())
}

/// The compositions `comp` shows and plays.
fn comps_in(comp: &crate::Comp) -> impl Iterator<Item = &str> {
    let shown = comp.layers.iter().filter_map(|l| match &l.content {
        LayerContent::Comp { comp, .. } => Some(comp.as_str()),
        _ => None,
    });
    shown.chain(comp.audio.iter().filter_map(|a| a.comp.as_deref()))
}

/// Whether composition `from` shows or plays composition `to`, directly or through others.
fn comp_reaches(spec: &RenderSpec, from: &str, to: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<&str> = spec.comps.get(from).into_iter().flat_map(comps_in).collect();
    while let Some(id) = todo.pop() {
        if id == to {
            return true;
        }
        if seen.insert(id) {
            todo.extend(spec.comps.get(id).into_iter().flat_map(comps_in));
        }
    }
    false
}

/// Checks the layers of the spec (`at` empty) or of one composition (`at` its path and a dot).
fn layers_ok(spec: &RenderSpec, at: &str, layers: &[crate::Layer]) -> Result<(), ValidationError> {
    let mut ids = BTreeSet::new();
    for (i, layer) in layers.iter().enumerate() {
        let path = format!("{at}layers[{i}]");
        if !ids.insert(layer.id.as_str()) {
            return Err(bad(format!("{path}.id"), format!("duplicate layer id {:?}", layer.id)));
        }
        if layer.dur <= Tick::ZERO {
            return Err(bad(format!("{path}.dur"), "must be positive"));
        }
        match &layer.content {
            LayerContent::Media { asset, time, .. } => {
                asset_of_kind(spec, &format!("{path}.content.asset"), asset, &[AssetKind::Video, AssetKind::Image])?;
                time_ok(&format!("{path}.content.time"), time, layer.dur)?;
            }
            LayerContent::Solid { color } => color_ok(&format!("{path}.content.color"), color.0)?,
            LayerContent::Comp { comp, .. } => {
                if !spec.comps.contains_key(comp) {
                    return Err(bad(format!("{path}.content.comp"), format!("unknown composition {comp:?}")));
                }
            }
            LayerContent::Text(text) => {
                if !(text.size > 0.0 && text.size.is_finite()) {
                    return Err(bad(format!("{path}.content.size"), "must be positive"));
                }
                color_ok(&format!("{path}.content.color"), text.color.0)?;
                if !(100..=900).contains(&text.weight) {
                    return Err(bad(format!("{path}.content.weight"), "must be 100 to 900"));
                }
                if let Some(w) = text.width
                    && !(w > 0.0 && w.is_finite())
                {
                    return Err(bad(format!("{path}.content.width"), "must be positive"));
                }
                if !(text.tracking.is_finite() && (-0.5..=4.0).contains(&text.tracking)) {
                    return Err(bad(format!("{path}.content.tracking"), "must be -0.5 to 4 ems"));
                }
                if !(text.leading.is_finite() && (-0.5..=4.0).contains(&text.leading)) {
                    return Err(bad(format!("{path}.content.leading"), "must be -0.5 to 4 ems"));
                }
                if let Some(st) = &text.stroke {
                    color_ok(&format!("{path}.content.stroke.color"), st.color.0)?;
                    if !(st.width >= 0.0 && st.width <= text.size) {
                        return Err(bad(format!("{path}.content.stroke.width"), "must be 0 to the text size"));
                    }
                }
                if let Some(bg) = &text.background {
                    color_ok(&format!("{path}.content.background.color"), bg.color.0)?;
                    if !(bg.padding >= 0.0 && bg.padding <= 4.0) {
                        return Err(bad(format!("{path}.content.background.padding"), "must be 0 to 4 ems"));
                    }
                    if !(bg.radius >= 0.0 && bg.radius <= 4.0) {
                        return Err(bad(format!("{path}.content.background.radius"), "must be 0 to 4 ems"));
                    }
                }
                if let Some(g) = &text.gradient {
                    color_ok(&format!("{path}.content.gradient"), g.0)?;
                }
                if let Some(st) = &text.outer {
                    color_ok(&format!("{path}.content.outer.color"), st.color.0)?;
                    if !(st.width >= 0.0 && st.width <= text.size) {
                        return Err(bad(format!("{path}.content.outer.width"), "must be 0 to the text size"));
                    }
                }
                if let Some(sh) = &text.shadow {
                    color_ok(&format!("{path}.content.shadow.color"), sh.color.0)?;
                    if !(sh.offset.iter().all(|o| o.abs() <= 4.0) && (0.0..=4.0).contains(&sh.blur)) {
                        return Err(bad(format!("{path}.content.shadow"), "offset and blur must be within 4 ems"));
                    }
                }
                if let Some(font) = &text.font {
                    asset_of_kind(spec, &format!("{path}.content.font"), font, &[AssetKind::Font])?;
                }
                if let Some(n) = &text.number {
                    keys_ok(&format!("{path}.content.number.value"), &n.value, layer.dur)?;
                    if n.decimals > 4 {
                        return Err(bad(format!("{path}.content.number.decimals"), "must be 0 to 4"));
                    }
                }
            }
            LayerContent::Shape(sh) => shape_ok(&format!("{path}.content"), sh, layer.dur)?,
        }
        let pictured = matches!(layer.content, LayerContent::Media { .. } | LayerContent::Comp { .. });
        if let Some(c) = &layer.crop {
            let v = [c.left, c.top, c.right, c.bottom];
            if !pictured {
                return Err(bad(format!("{path}.crop"), "only media and composition layers are cropped"));
            }
            if v.iter().any(|x| !(0.0..1.0).contains(x)) || c.left + c.right > 0.99 || c.top + c.bottom > 0.99 {
                return Err(bad(
                    format!("{path}.crop"),
                    "each edge must be 0 to 1, leaving at least 1 % of the picture",
                ));
            }
        }
        if (layer.flip_x || layer.flip_y) && !pictured {
            return Err(bad(format!("{path}.flip_x"), "only media and composition layers are mirrored"));
        }
        if let Some(m) = &layer.shape_mask {
            let v = [m.center[0], m.center[1], m.size[0], m.size[1], m.rotation, m.feather];
            if v.iter().any(|x| !x.is_finite()) || m.size.iter().any(|x| *x < 0.0) || m.feather < 0.0 {
                return Err(bad(format!("{path}.shape_mask"), "needs finite values, a size and feather of 0 or more"));
            }
        }
        if let Some(c) = &layer.crop_keys {
            if !pictured || layer.crop.is_some() {
                return Err(bad(
                    format!("{path}.crop_keys"),
                    "only media and composition layers are cropped, and by `crop` or `crop_keys`",
                ));
            }
            for (name, edge) in [("left", &c.left), ("top", &c.top), ("right", &c.right), ("bottom", &c.bottom)] {
                keys_ok(&format!("{path}.crop_keys.{name}"), edge, layer.dur)?;
            }
        }
        if let Some(m) = &layer.shape_mask_keys {
            if layer.shape_mask.is_none() {
                return Err(bad(format!("{path}.shape_mask_keys"), "needs a `shape_mask` to move"));
            }
            keys_ok(&format!("{path}.shape_mask_keys.center"), &m.center, layer.dur)?;
            keys_ok(&format!("{path}.shape_mask_keys.size"), &m.size, layer.dur)?;
            keys_ok(&format!("{path}.shape_mask_keys.rotation"), &m.rotation, layer.dur)?;
            keys_ok(&format!("{path}.shape_mask_keys.feather"), &m.feather, layer.dur)?;
        }
        if let Some(mask) = &layer.mask {
            asset_of_kind(spec, &format!("{path}.mask"), mask, &[AssetKind::MaskSequence])?;
        }
        keys_ok(&format!("{path}.opacity"), &layer.opacity, layer.dur)?;
        if let LayerContent::Text(text) = &layer.content {
            if let Some(r) = &text.reveal {
                keys_ok(&format!("{path}.content.reveal"), r, layer.dur)?;
            }
            for (j, m) in text.chars.iter().enumerate() {
                let at = format!("{path}.content.chars[{j}]");
                keys_ok(&format!("{at}.progress"), &m.progress, layer.dur)?;
                if !(m.step.is_finite() && m.step >= 0.0) {
                    return Err(bad(format!("{at}.step"), "must be zero or more"));
                }
                let p = m.pose;
                let finite = p.offset.iter().chain([&p.scale, &p.rotation, &p.opacity]).all(|v| v.is_finite());
                if !finite || p.scale < 0.0 || !(0.0..=1.0).contains(&p.opacity) {
                    return Err(bad(
                        format!("{at}.pose"),
                        "needs numbers, a scale of zero or more and an opacity of 0 to 1",
                    ));
                }
            }
        }
        keys_ok(&format!("{path}.transform.position"), &layer.transform.position, layer.dur)?;
        keys_ok(&format!("{path}.transform.scale"), &layer.transform.scale, layer.dur)?;
        keys_ok(&format!("{path}.transform.rotation"), &layer.transform.rotation, layer.dur)?;
        for (j, fx) in layer.fx.iter().enumerate() {
            let fx_path = format!("{path}.fx[{j}]");
            let ok = fx.id.split_once('@').is_some_and(|(name, ver)| {
                !name.is_empty() && !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit())
            });
            if !ok {
                return Err(bad(format!("{fx_path}.id"), "must look like name@1"));
            }
            for (name, value) in &fx.params {
                keys_ok(&format!("{fx_path}.params.{name}"), value, layer.dur)?;
            }
            match (&fx.asset, fx.id == crate::effects::LUT) {
                (Some(a), true) => asset_of_kind(spec, &format!("{fx_path}.asset"), a, &[AssetKind::Lut])?,
                (None, true) => return Err(bad(format!("{fx_path}.asset"), "a lookup table needs its `lut` asset")),
                (Some(_), false) => return Err(bad(format!("{fx_path}.asset"), "this effect reads no asset")),
                (None, false) => {}
            }
        }
        if layer.fx.iter().filter(|f| f.id == crate::effects::LUT).count() > 1 {
            return Err(bad(format!("{path}.fx"), "one lookup table per layer"));
        }
        for id in crate::effects::DISTORT {
            if layer.fx.iter().filter(|f| f.id == id).count() > 1 {
                return Err(bad(format!("{path}.fx"), "one of each distortion per layer"));
            }
        }
    }
    Ok(())
}

/// Checks a shape layer's content.
fn shape_ok(path: &str, sh: &crate::ShapeContent, dur: Tick) -> Result<(), ValidationError> {
    let fine = |v: f32| v.is_finite() && (0.0..=16384.0).contains(&v);
    if !(sh.size.iter().all(|v| fine(*v)) && fine(sh.radius)) {
        return Err(bad(format!("{path}.size"), "size and radius must be 0 to 16384 pixels"));
    }
    if !(3..=64).contains(&sh.points) {
        return Err(bad(format!("{path}.points"), "must be 3 to 64"));
    }
    if !(sh.inner.is_finite() && (0.05..=1.0).contains(&sh.inner)) {
        return Err(bad(format!("{path}.inner"), "must be 0.05 to 1"));
    }
    match &sh.fill {
        Some(crate::Paint::Solid { color }) => color_ok(&format!("{path}.fill.color"), color.0)?,
        Some(crate::Paint::Linear { from, to, angle }) => {
            color_ok(&format!("{path}.fill.from"), from.0)?;
            color_ok(&format!("{path}.fill.to"), to.0)?;
            if !angle.is_finite() {
                return Err(bad(format!("{path}.fill.angle"), "must be a number"));
            }
        }
        Some(crate::Paint::Radial { from, to }) => {
            color_ok(&format!("{path}.fill.from"), from.0)?;
            color_ok(&format!("{path}.fill.to"), to.0)?;
        }
        None => {}
    }
    if let Some(st) = &sh.stroke {
        color_ok(&format!("{path}.stroke.color"), st.color.0)?;
        if !(st.width.is_finite() && (0.0..=2048.0).contains(&st.width)) {
            return Err(bad(format!("{path}.stroke.width"), "must be 0 to 2048 pixels"));
        }
    }
    if let Some(sh) = &sh.shadow {
        color_ok(&format!("{path}.shadow.color"), sh.color.0)?;
        if !(sh.offset.iter().all(|o| o.is_finite() && o.abs() <= 4096.0) && fine(sh.blur)) {
            return Err(bad(format!("{path}.shadow"), "offset and blur must be finite and within reach"));
        }
    }
    if let Some(t) = &sh.trim {
        keys_ok(&format!("{path}.trim.start"), &t.start, dur)?;
        keys_ok(&format!("{path}.trim.end"), &t.end, dur)?;
    }
    if let Some(o) = &sh.fill_opacity {
        keys_ok(&format!("{path}.fill_opacity"), o, dur)?;
    }
    Ok(())
}

/// Checks the audio clips of the spec or of one composition, as [`layers_ok`] does layers.
fn audio_ok(spec: &RenderSpec, at: &str, clips: &[crate::AudioClip]) -> Result<(), ValidationError> {
    let mut audio_ids = BTreeSet::new();
    for (i, clip) in clips.iter().enumerate() {
        let path = format!("{at}audio[{i}]");
        if !audio_ids.insert(clip.id.as_str()) {
            return Err(bad(format!("{path}.id"), format!("duplicate audio id {:?}", clip.id)));
        }
        if clip.dur <= Tick::ZERO {
            return Err(bad(format!("{path}.dur"), "must be positive"));
        }
        match &clip.comp {
            None => asset_of_kind(spec, &format!("{path}.asset"), &clip.asset, &[AssetKind::Video, AssetKind::Audio])?,
            Some(comp) => {
                if !spec.comps.contains_key(comp) {
                    return Err(bad(format!("{path}.comp"), format!("unknown composition {comp:?}")));
                }
                if !clip.asset.is_empty() {
                    return Err(bad(format!("{path}.asset"), "a clip plays an asset or a composition, not both"));
                }
                if clip.time.speed != 1.0 || !clip.time.ramp.is_empty() || clip.pitch != 0.0 {
                    return Err(bad(format!("{path}.time"), "a composition plays at speed 1 and its own pitch"));
                }
            }
        }
        time_ok(&format!("{path}.time"), &clip.time, clip.dur)?;
        keys_ok(&format!("{path}.gain_db"), &clip.gain_db, clip.dur)?;
        if !(clip.pitch.is_finite() && (-24.0..=24.0).contains(&clip.pitch)) {
            return Err(bad(format!("{path}.pitch"), "must be from -24 to 24 semitones"));
        }
        for (j, fx) in clip.fx.iter().enumerate() {
            audio_fx_ok(&format!("{path}.fx[{j}]"), fx)?;
        }
    }
    Ok(())
}

fn audio_fx_ok(path: &str, fx: &crate::AudioEffect) -> Result<(), ValidationError> {
    use crate::AudioEffect as E;
    let unit = |name: &str, v: f32| {
        if v.is_finite() && (0.0..=1.0).contains(&v) {
            Ok(())
        } else {
            Err(bad(format!("{path}.{name}"), "must be from 0 to 1"))
        }
    };
    match fx {
        E::Denoise { floor, amount } => {
            if floor.len() != E::NOISE_BANDS || floor.iter().any(|v| !v.is_finite()) {
                return Err(bad(format!("{path}.floor"), format!("must be {} numbers", E::NOISE_BANDS)));
            }
            unit("amount", *amount)
        }
        E::Band { low, high } => {
            if !(low.is_finite() && high.is_finite() && *low >= 0.0 && low < high) {
                return Err(bad(path, "`low` must be at least 0 and below `high`"));
            }
            Ok(())
        }
        E::Ring { hz, mix } => {
            if !(hz.is_finite() && *hz > 0.0 && *hz <= 20_000.0) {
                return Err(bad(format!("{path}.hz"), "must be above 0 and at most 20000"));
            }
            unit("mix", *mix)
        }
        E::Echo { delay, decay, repeats } => {
            if *delay <= Tick::ZERO || *delay > Tick(2 * reelforge_time::TICKS_PER_SECOND) {
                return Err(bad(format!("{path}.delay"), "must be above 0 and at most 2 seconds"));
            }
            if !(1..=8).contains(repeats) {
                return Err(bad(format!("{path}.repeats"), "must be from 1 to 8"));
            }
            unit("decay", *decay)
        }
    }
}

fn asset_of_kind(spec: &RenderSpec, path: &str, id: &str, kinds: &[AssetKind]) -> Result<(), ValidationError> {
    match spec.assets.get(id) {
        None => Err(bad(path, format!("unknown asset {id:?}"))),
        Some(a) if !kinds.contains(&a.kind) => {
            Err(bad(path, format!("asset {id:?} is {:?}, expected one of {kinds:?}", a.kind)))
        }
        Some(_) => Ok(()),
    }
}

fn speed_ok(path: &str, speed: f64) -> Result<(), ValidationError> {
    if speed.is_finite() && (-100.0..=100.0).contains(&speed) {
        Ok(())
    } else {
        Err(bad(path, "must be a number from -100 to 100"))
    }
}

fn time_ok(path: &str, time: &TimeMap, dur: Tick) -> Result<(), ValidationError> {
    speed_ok(&format!("{path}.speed"), time.speed)?;
    for (i, k) in time.ramp.iter().enumerate() {
        speed_ok(&format!("{path}.ramp[{i}].speed"), k.speed)?;
        if k.t < Tick::ZERO || k.t > dur {
            return Err(bad(format!("{path}.ramp[{i}].t"), "must lie within the owner's duration"));
        }
    }
    if time.ramp.windows(2).any(|w| w[1].t <= w[0].t) {
        return Err(bad(format!("{path}.ramp"), "keys must be in strictly increasing time"));
    }
    Ok(())
}

fn color_ok(path: &str, c: [f32; 4]) -> Result<(), ValidationError> {
    if c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
        Ok(())
    } else {
        Err(bad(path, "each channel must be between 0 and 1"))
    }
}

fn keys_ok<T>(path: &str, value: &Animated<T>, dur: Tick) -> Result<(), ValidationError> {
    let Animated::Keys(keys) = value else { return Ok(()) };
    if keys.is_empty() {
        return Err(bad(path, "a keyframe list must not be empty"));
    }
    for (i, pair) in keys.windows(2).enumerate() {
        if pair[1].t <= pair[0].t {
            return Err(bad(format!("{path}[{}].t", i + 1), "keyframes must be in strictly increasing time"));
        }
    }
    if keys.iter().any(|k| k.t < Tick::ZERO || k.t > dur) {
        return Err(bad(path, "keyframe times must lie within the owner's duration"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use reelforge_time::FrameRate;

    fn base() -> RenderSpec {
        let mut s = RenderSpec::new(Output {
            width: 1080,
            height: 1920,
            fps: FrameRate::FPS_30,
            duration: Tick::SECOND * 3,
            background: Color([0.0, 0.0, 0.0, 1.0]),
            video: None,
            audio: None,
            format: Default::default(),
        });
        s.layers.push(Layer {
            id: "c1".into(),
            at: Tick::ZERO,
            dur: Tick::SECOND,
            content: LayerContent::Solid { color: Color([1.0, 0.0, 0.0, 1.0]) },
            transform: Transform::default(),
            opacity: Animated::Constant(1.0),
            blend: Blend::Normal,
            mask: None,
            fx: vec![],
            crop: None,
            flip_x: false,
            flip_y: false,
            shape_mask: None,
            crop_keys: None,
            shape_mask_keys: None,
        });
        s
    }

    #[test]
    fn a_plain_spec_passes_and_round_trips() {
        let s = base();
        validate(&s).unwrap();
        assert_eq!(RenderSpec::from_json(&s.to_json()).unwrap(), s);
    }

    #[test]
    fn refusals_name_the_field() {
        let mut s = base();
        s.output.width = 1081;
        assert!(matches!(validate(&s), Err(ValidationError::Invalid { path, .. }) if path == "output.width"));

        let mut s = base();
        s.layers[0].content = LayerContent::Media {
            asset: "nope".into(),
            time: TimeMap::default(),
            fit: Fit::Contain,
            between: Default::default(),
        };
        assert!(
            matches!(validate(&s), Err(ValidationError::Invalid { path, .. }) if path == "layers[0].content.asset")
        );

        let mut s = base();
        s.layers[0].fx.push(Effect { id: "blur".into(), params: Default::default(), asset: None });
        assert!(validate(&s).is_err());

        let mut s = base();
        s.spec = "renderspec/9".into();
        assert!(matches!(validate(&s), Err(ValidationError::Version(_))));
    }

    #[test]
    fn text_and_media_layers_round_trip() {
        let mut s = base();
        let mut text = TextContent::new("你好 Hello", 96.0, Color([1.0, 1.0, 1.0, 1.0]));
        text.weight = 700;
        text.width = Some(900.0);
        text.stroke = Some(TextStroke { color: Color([0.0, 0.0, 0.0, 1.0]), width: 6.0 });
        text.background =
            Some(TextBackground { color: Color([0.0, 0.0, 0.0, 0.5]), padding: 0.25, radius: 0.0, lines: false });
        s.layers[0].content = LayerContent::Text(Box::new(text.clone()));
        assert_eq!(RenderSpec::from_json(&s.to_json()).unwrap(), s);
        // the optional styling has defaults, so a minimal text layer still reads
        let mut v = serde_json::to_value(&s).unwrap();
        let c = &mut v["layers"][0]["content"];
        for k in ["weight", "align", "width", "stroke", "background"] {
            c.as_object_mut().unwrap().remove(k);
        }
        let back = RenderSpec::from_json(&v.to_string()).unwrap();
        assert_eq!(
            back.layers[0].content,
            LayerContent::Text(Box::new(TextContent::new("你好 Hello", 96.0, text.color)))
        );
        text.weight = 950;
        s.layers[0].content = LayerContent::Text(Box::new(text));
        assert!(validate(&s).is_err());
        s.layers[0].content = LayerContent::Text(Box::new(TextContent::new("ok", 96.0, Color([1.0; 4]))));
        assert!(s.required_features().contains("text/1"));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut v = serde_json::to_value(base()).unwrap();
        v["surprise"] = serde_json::json!(1);
        assert!(RenderSpec::from_json(&v.to_string()).is_err());
    }
}
