//! Checks a spec before anything renders it, so a renderer can assume a well-formed edit.

use std::collections::BTreeSet;

use reelforge_time::Tick;
use thiserror::Error;

use crate::{Animated, AssetKind, LayerContent, RenderSpec, SPEC_VERSION};

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

    let mut ids = BTreeSet::new();
    for (i, layer) in spec.layers.iter().enumerate() {
        let path = format!("layers[{i}]");
        if !ids.insert(layer.id.as_str()) {
            return Err(bad(format!("{path}.id"), format!("duplicate layer id {:?}", layer.id)));
        }
        if layer.dur <= Tick::ZERO {
            return Err(bad(format!("{path}.dur"), "must be positive"));
        }
        match &layer.content {
            LayerContent::Media { asset, time, .. } => {
                asset_of_kind(spec, &format!("{path}.content.asset"), asset, &[AssetKind::Video, AssetKind::Image])?;
                speed_ok(&format!("{path}.content.time.speed"), time.speed)?;
            }
            LayerContent::Solid { color } => color_ok(&format!("{path}.content.color"), color.0)?,
            LayerContent::Text(text) => {
                if !(text.size > 0.0 && text.size.is_finite()) {
                    return Err(bad(format!("{path}.content.size"), "must be positive"));
                }
                color_ok(&format!("{path}.content.color"), text.color.0)?;
                if let Some(font) = &text.font {
                    asset_of_kind(spec, &format!("{path}.content.font"), font, &[AssetKind::Font])?;
                }
            }
        }
        if let Some(mask) = &layer.mask {
            asset_of_kind(spec, &format!("{path}.mask"), mask, &[AssetKind::MaskSequence])?;
        }
        keys_ok(&format!("{path}.opacity"), &layer.opacity, layer.dur)?;
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
        }
    }

    let mut audio_ids = BTreeSet::new();
    for (i, clip) in spec.audio.iter().enumerate() {
        let path = format!("audio[{i}]");
        if !audio_ids.insert(clip.id.as_str()) {
            return Err(bad(format!("{path}.id"), format!("duplicate audio id {:?}", clip.id)));
        }
        if clip.dur <= Tick::ZERO {
            return Err(bad(format!("{path}.dur"), "must be positive"));
        }
        asset_of_kind(spec, &format!("{path}.asset"), &clip.asset, &[AssetKind::Video, AssetKind::Audio])?;
        speed_ok(&format!("{path}.time.speed"), clip.time.speed)?;
        keys_ok(&format!("{path}.gain_db"), &clip.gain_db, clip.dur)?;
    }
    Ok(())
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
    if speed > 0.0 && speed.is_finite() && speed <= 100.0 {
        Ok(())
    } else {
        Err(bad(path, "must be above 0 and at most 100"))
    }
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
        s.layers[0].content = LayerContent::Media { asset: "nope".into(), time: TimeMap::default(), fit: Fit::Contain };
        assert!(
            matches!(validate(&s), Err(ValidationError::Invalid { path, .. }) if path == "layers[0].content.asset")
        );

        let mut s = base();
        s.layers[0].fx.push(Effect { id: "blur".into(), params: Default::default() });
        assert!(validate(&s).is_err());

        let mut s = base();
        s.spec = "renderspec/9".into();
        assert!(matches!(validate(&s), Err(ValidationError::Version(_))));
    }

    #[test]
    fn text_and_media_layers_round_trip() {
        let mut s = base();
        s.layers[0].content = LayerContent::Text(TextContent {
            text: "Hello".into(),
            font: None,
            size: 96.0,
            color: Color([1.0, 1.0, 1.0, 1.0]),
        });
        assert_eq!(RenderSpec::from_json(&s.to_json()).unwrap(), s);
        assert!(s.required_features().contains("text/1"));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut v = serde_json::to_value(base()).unwrap();
        v["surprise"] = serde_json::json!(1);
        assert!(RenderSpec::from_json(&v.to_string()).is_err());
    }
}
