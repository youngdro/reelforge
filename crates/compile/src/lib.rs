//! Project → RenderSpec.
//!
//! This is the one place where editing meaning becomes rendering instructions. The renderer never
//! reads a project, so anything a project can express has to be lowered here: clips become layers
//! bottom track first, picture clips with sound and sound clips become audio clips, hidden tracks
//! drop their layers and muted tracks their sound. As the model grows (transitions, templates,
//! captions, speed curves), each feature gets its lowering here and the RenderSpec stays flat.

use std::collections::BTreeMap;

use reelforge_model::{
    Adjust, BlendMode, Clip, ClipContent, Codec, Mask, MaskKind, MediaKind, Project, Prop, Timing, TitleAlign, Track,
    TrackKind, TransitionKind,
};
use reelforge_renderspec::{
    Animated, Asset, AssetKind, AudioClip, AudioCodec, AudioEncoding, Blend, Color, Crop, Ease, Effect, Fit, Key,
    Layer, LayerContent, MaskShape, Output, Probe, RenderSpec, ShapeMask, SpeedKey, TextAlign, TextBackground,
    TextContent, TextShadow, TextStroke, TimeMap, Transform, VideoCodec, VideoEncoding, effects, validate,
};
use reelforge_time::Tick;
use thiserror::Error;

mod motion;

use motion::{Overlay, Shape};

/// A clip's blend mode as RenderSpec spells it.
fn blend(m: BlendMode) -> Blend {
    match m {
        BlendMode::Normal => Blend::Normal,
        BlendMode::Add => Blend::Add,
        BlendMode::Multiply => Blend::Multiply,
        BlendMode::Screen => Blend::Screen,
        BlendMode::Overlay => Blend::Overlay,
        BlendMode::Darken => Blend::Darken,
        BlendMode::Lighten => Blend::Lighten,
        BlendMode::SoftLight => Blend::SoftLight,
        BlendMode::HardLight => Blend::HardLight,
        BlendMode::Difference => Blend::Difference,
        BlendMode::ColorDodge => Blend::ColorDodge,
        BlendMode::ColorBurn => Blend::ColorBurn,
    }
}

/// A clip's mask as RenderSpec spells it.
fn shape_mask(m: Mask) -> ShapeMask {
    ShapeMask {
        shape: match m.kind {
            MaskKind::Linear => MaskShape::Linear,
            MaskKind::Mirror => MaskShape::Mirror,
            MaskKind::Circle => MaskShape::Circle,
            MaskKind::Rectangle => MaskShape::Rectangle,
        },
        center: m.center,
        size: m.size,
        rotation: m.rotation,
        feather: m.feather,
        invert: m.invert,
    }
}

/// Why a project could not be lowered.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CompileError {
    /// The project broke one of its own invariants.
    #[error("the project is not valid: {0}")]
    Model(String),
    /// Media without a content hash cannot be addressed by a renderer.
    #[error("media {0:?} has no content hash; re-import it so it can be rendered")]
    Unhashed(String),
    /// The project has nothing on it.
    #[error("the timeline is empty")]
    Empty,
    /// The lowering produced an invalid spec: a bug in this crate.
    #[error("internal error, the compiled spec is invalid: {0}")]
    Invalid(String),
}

/// Options for a compile.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Include encode settings; `false` compiles a spec for frames only (previews, thumbnails).
    pub encode: bool,
}

impl Default for CompileOptions {
    fn default() -> CompileOptions {
        CompileOptions { encode: true }
    }
}

/// The content hash a frames-only spec gives media that has none: no file has it, so the renderer
/// draws the offline slate.
pub const OFFLINE_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Lowers `project` into a validated RenderSpec.
pub fn compile(project: &Project, options: &CompileOptions) -> Result<RenderSpec, CompileError> {
    project.check().map_err(|e| CompileError::Model(e.to_string()))?;
    let duration = project.duration();
    if duration <= Tick::ZERO {
        return Err(CompileError::Empty);
    }
    let c = &project.canvas;
    let mut spec = RenderSpec::new(Output {
        width: c.width,
        height: c.height,
        fps: if options.encode { project.export.frame_rate(c) } else { c.fps },
        duration,
        background: Color(c.background),
        video: options.encode.then(|| {
            let e = &project.export;
            let size = e.size(c);
            VideoEncoding {
                codec: match e.codec {
                    Codec::H264 => VideoCodec::H264,
                    Codec::Hevc => VideoCodec::Hevc,
                    Codec::Av1 => VideoCodec::Av1,
                },
                crf: Some(e.quality.crf()),
                bitrate_kbps: Some(e.bitrate_kbps(c)),
                size: (size != [c.width, c.height]).then_some(size),
            }
        }),
        audio: options.encode.then_some(AudioEncoding {
            codec: AudioCodec::Aac,
            sample_rate: 48_000,
            bitrate_kbps: 192,
        }),
    });

    let step = c.fps.frame_duration();
    let mut assets: BTreeMap<String, Asset> = BTreeMap::new();
    let mut asset_for = |media_id: &str| -> Result<String, CompileError> {
        let m = &project.media[media_id];
        if !assets.contains_key(media_id) {
            // a preview draws unmeasured (offline) media as the renderer's offline slate; an export
            // needs every file
            let sha256 = match &m.sha256 {
                Some(h) => h.clone(),
                None if !options.encode => OFFLINE_SHA256.to_string(),
                None => return Err(CompileError::Unhashed(media_id.to_string())),
            };
            assets.insert(
                media_id.to_string(),
                Asset {
                    kind: match m.kind {
                        MediaKind::Video => AssetKind::Video,
                        MediaKind::Image => AssetKind::Image,
                        MediaKind::Audio => AssetKind::Audio,
                        MediaKind::Lut => AssetKind::Lut,
                    },
                    sha256,
                    uri: m.uri.clone(),
                    of: None,
                    probe: Some(Probe {
                        width: m.size.map(|s| s[0]),
                        height: m.size.map(|s| s[1]),
                        duration: m.duration,
                    }),
                },
            );
        }
        Ok(media_id.to_string())
    };

    for track in &project.tracks {
        let cuts = transitions(track);
        for (i, clip) in track.clips.iter().enumerate() {
            let picture = track.kind != TrackKind::Audio && !track.hidden;
            if picture {
                let shape = shape(clip, cuts[i], cuts.get(i + 1).copied().flatten());
                let media = matches!(clip.content, ClipContent::Media { .. });
                let content = match &clip.content {
                    ClipContent::Media { media, .. } => LayerContent::Media {
                        asset: asset_for(media)?,
                        time: time_map(&clip.timing().expect("a media clip"), shape.lead),
                        fit: Fit::Contain,
                    },
                    ClipContent::Solid { color } => LayerContent::Solid { color: Color(*color) },
                    ClipContent::Text { text, size, color, style } => LayerContent::Text(TextContent {
                        weight: style.weight,
                        align: match style.align {
                            TitleAlign::Left => TextAlign::Left,
                            TitleAlign::Center => TextAlign::Center,
                            TitleAlign::Right => TextAlign::Right,
                        },
                        width: (style.wrap > 0.0).then_some(style.wrap * project.canvas.width as f32),
                        stroke: style
                            .outline
                            .as_ref()
                            .filter(|o| o.width > 0.0)
                            .map(|o| TextStroke { color: Color(o.color), width: o.width * size }),
                        background: style.background.map(|c| TextBackground {
                            color: Color(c),
                            padding: 0.25,
                            radius: style.radius,
                        }),
                        gradient: style.gradient.map(Color),
                        outer: style
                            .outer
                            .as_ref()
                            .filter(|o| o.width > 0.0)
                            .map(|o| TextStroke { color: Color(o.color), width: o.width * size }),
                        shadow: style.shadow.as_ref().map(|sh| TextShadow {
                            color: Color(sh.color),
                            offset: sh.offset,
                            blur: sh.blur,
                        }),
                        ..TextContent::new(text.clone(), *size, Color(*color))
                    }),
                };
                // a blurred copy filling the frame behind it
                if let (Some(amount), LayerContent::Media { asset, time, .. }) = (clip.look.backdrop, &content) {
                    let mut fx = color_fx(clip, &mut asset_for)?;
                    fx.push(Effect {
                        id: effects::BLUR.into(),
                        params: [(
                            "radius".to_string(),
                            Animated::Constant(reelforge_model::look::backdrop_radius(amount)),
                        )]
                        .into(),
                        asset: None,
                    });
                    spec.layers.push(Layer {
                        id: format!("{}~backdrop", clip.id),
                        at: clip.start - shape.lead,
                        dur: shape.len,
                        content: LayerContent::Media { asset: asset.clone(), time: time.clone(), fit: Fit::Cover },
                        transform: Transform::default(),
                        opacity: motion::lower(clip, Prop::Opacity, &shape, step),
                        blend: Blend::Normal,
                        mask: None,
                        crop: None,
                        flip_x: clip.look.flip_x,
                        flip_y: clip.look.flip_y,
                        shape_mask: None,
                        fx,
                    });
                }
                spec.layers.push(Layer {
                    id: clip.id.clone(),
                    at: clip.start - shape.lead,
                    dur: shape.len,
                    content,
                    transform: Transform {
                        position: motion::lower_position(clip, &shape, step),
                        scale: motion::lower(clip, Prop::Scale, &shape, step),
                        rotation: motion::lower(clip, Prop::Rotation, &shape, step),
                    },
                    opacity: motion::lower(clip, Prop::Opacity, &shape, step),
                    blend: blend(clip.look.blend),
                    mask: None,
                    crop: (media && !clip.look.crop.is_zero()).then(|| {
                        let c = clip.look.crop;
                        Crop { left: c.left, top: c.top, right: c.right, bottom: c.bottom }
                    }),
                    flip_x: media && clip.look.flip_x,
                    flip_y: media && clip.look.flip_y,
                    shape_mask: clip.look.mask.map(shape_mask),
                    fx: {
                        let mut fx = color_fx(clip, &mut asset_for)?;
                        fx.extend(clip.effects.iter().map(|e| Effect {
                            id: e.id.clone(),
                            params: e.params.iter().map(|(k, v)| (k.clone(), Animated::Constant(*v))).collect(),
                            asset: None,
                        }));
                        fx
                    },
                });
                // a dip passes through a colour layer over both clips
                if let Some((kind, half)) = cuts[i]
                    && let Some(color) = kind.dip()
                {
                    spec.layers.push(Layer {
                        id: format!("{}~dip", clip.id),
                        at: clip.start - half,
                        dur: half * 2,
                        content: LayerContent::Solid { color: Color(color) },
                        transform: Transform::default(),
                        opacity: Animated::Keys(vec![
                            Key { t: Tick::ZERO, v: 0.0, ease: Ease::Linear },
                            Key { t: half, v: 1.0, ease: Ease::Linear },
                            Key { t: half * 2, v: 0.0, ease: Ease::Linear },
                        ]),
                        blend: Blend::Normal,
                        mask: None,
                        fx: vec![],
                        crop: None,
                        flip_x: false,
                        flip_y: false,
                        shape_mask: None,
                    });
                }
            }
            let sound = !track.muted
                && match &clip.content {
                    ClipContent::Media { media, .. } => project.media[media].kind != MediaKind::Image,
                    _ => false,
                };
            // a freeze frame is silent
            if sound
                && let ClipContent::Media { media, .. } = &clip.content
                && let Some(timing) = clip.timing().filter(|t| t.speed != 0.0)
            {
                let time = time_map(&timing, Tick::ZERO);
                // backwards, sound plays like tape run the other way
                let keep_pitch = clip.keep_pitch && !time.reverses() && (time.speed != 1.0 || !time.ramp.is_empty());
                spec.audio.push(AudioClip {
                    id: clip.id.clone(),
                    asset: asset_for(media)?,
                    at: clip.start,
                    dur: clip.duration,
                    time,
                    gain_db: motion::lower(clip, Prop::VolumeDb, &Shape::of(clip), step),
                    keep_pitch,
                });
            }
        }
    }
    spec.assets = assets;
    spec.engine.features = spec.required_features().into_iter().collect();
    validate(&spec).map_err(|e| CompileError::Invalid(e.to_string()))?;
    Ok(spec)
}

/// A clip's colour as effects: its filter, its lookup table, then its own adjustments.
fn color_fx(
    clip: &Clip,
    asset_for: &mut impl FnMut(&str) -> Result<String, CompileError>,
) -> Result<Vec<Effect>, CompileError> {
    let grade = |a: Adjust| Effect {
        id: effects::GRADE.into(),
        params: Adjust::RANGES
            .iter()
            .zip(a.values())
            .filter(|(_, v)| *v != 0.0)
            .map(|((name, ..), v)| (name.to_string(), Animated::Constant(v)))
            .collect(),
        asset: None,
    };
    let c = &clip.color;
    let mut fx = Vec::new();
    if let Some(f) = c.filter.filter(|f| f.strength > 0.0) {
        fx.push(grade(f.kind.adjust().scaled(f.strength)));
    }
    if let Some(l) = c.lut.as_ref().filter(|l| l.strength > 0.0) {
        fx.push(Effect {
            id: effects::LUT.into(),
            params: [("amount".to_string(), Animated::Constant(l.strength))].into_iter().collect(),
            asset: Some(asset_for(&l.media)?),
        });
    }
    if !c.adjust.is_zero() {
        fx.push(grade(c.adjust));
    }
    Ok(fx)
}

/// A clip's timing as a layer or audio clip that starts `lead` before the clip.
fn time_map(t: &Timing, lead: Tick) -> TimeMap {
    let mut ramp: Vec<SpeedKey> = t.ramp.iter().map(|&(at, speed)| SpeedKey { t: at + lead, speed }).collect();
    // a very short clip can round two points onto one tick
    ramp.dedup_by_key(|k| k.t);
    TimeMap { source_in: t.source_at(-lead), speed: t.speed, ramp }
}

/// For each clip of `track`, the transition into it that plays, with half its length: a transition
/// plays on a picture track when the clip starts where the one before it ends, and lasts at most
/// as long as the shorter of the two.
fn transitions(track: &Track) -> Vec<Option<(TransitionKind, Tick)>> {
    track
        .clips
        .iter()
        .enumerate()
        .map(|(i, clip)| {
            let t = clip.transition?;
            let prev = track.clips.get(i.checked_sub(1)?)?;
            if track.kind != TrackKind::Video || prev.end() != clip.start {
                return None;
            }
            let half = Tick(t.duration.min(prev.duration).min(clip.duration).0 / 2);
            (half > Tick::ZERO).then_some((t.kind, half))
        })
        .collect()
}

/// A clip's layer, given the transitions into it and out of it (with half their lengths).
fn shape(clip: &Clip, into: Option<(TransitionKind, Tick)>, out: Option<(TransitionKind, Tick)>) -> Shape {
    let lead = into.map_or(Tick::ZERO, |(_, h)| h);
    let tail = out.map_or(Tick::ZERO, |(_, h)| h);
    let mut overlays = vec![];
    if let Some((kind, h)) = into {
        overlays.push(Overlay { from: Tick::ZERO, to: h * 2, kind, incoming: true });
    }
    if let Some((kind, h)) = out {
        let cut = lead + clip.duration;
        overlays.push(Overlay { from: cut - h, to: cut + h, kind, incoming: false });
    }
    Shape { lead, len: lead + clip.duration + tail, overlays }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_model::{Clip, ClipTransform, Media, TitleStyle};

    fn clip(id: &str, start: Tick, duration: Tick, content: ClipContent) -> Clip {
        Clip {
            id: id.into(),
            start,
            duration,
            content,
            transform: ClipTransform::default(),
            opacity: 1.0,
            volume_db: -3.0,
            effects: vec![],
            keys: Default::default(),
            anim: Default::default(),
            transition: None,
            fade: Default::default(),
            keep_pitch: true,
            color: Default::default(),
            look: Default::default(),
        }
    }

    #[test]
    fn layers_follow_tracks_bottom_first_and_sound_is_split_out() {
        let mut p = Project::default();
        p.media.insert(
            "m9".into(),
            Media {
                id: "m9".into(),
                name: "a.mp4".into(),
                uri: "file:///a.mp4".into(),
                sha256: Some("ab".repeat(32)),
                kind: MediaKind::Video,
                duration: Some(Tick::SECOND * 10),
                size: Some([1920, 1080]),
            },
        );
        p.tracks[0].clips.push(clip("c20", Tick::ZERO, Tick::SECOND * 2, ClipContent::media("m9", Tick::SECOND)));
        p.tracks[1].clips.push(clip(
            "c21",
            Tick::ZERO,
            Tick::SECOND,
            ClipContent::Text { text: "Hi".into(), size: 80.0, color: [1.0; 4], style: TitleStyle::subtitle() },
        ));
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert_eq!(spec.layers.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["c20", "c21"]);
        assert_eq!(spec.audio.len(), 1);
        assert_eq!(spec.output.duration, Tick::SECOND * 2);
        assert!(spec.engine.features.contains(&"text/1".to_string()));
    }

    #[test]
    fn unhashed_media_is_refused() {
        let mut p = Project::default();
        p.media.insert(
            "m9".into(),
            Media {
                id: "m9".into(),
                name: "a".into(),
                uri: "a.png".into(),
                sha256: None,
                kind: MediaKind::Image,
                duration: None,
                size: None,
            },
        );
        p.tracks[0].clips.push(clip("c20", Tick::ZERO, Tick::SECOND, ClipContent::media("m9", Tick::ZERO)));
        assert_eq!(compile(&p, &CompileOptions::default()), Err(CompileError::Unhashed("m9".into())));
        // a preview shows it as offline
        let spec = compile(&p, &CompileOptions { encode: false }).unwrap();
        assert_eq!(spec.assets.values().next().unwrap().sha256, OFFLINE_SHA256);
    }

    #[test]
    fn empty_is_refused() {
        assert_eq!(compile(&Project::default(), &CompileOptions::default()), Err(CompileError::Empty));
    }

    #[test]
    fn a_transition_overlaps_the_cut_and_dips_through_a_colour() {
        use reelforge_model::{Transition, TransitionKind};
        let solid = |id: &str, start: i64, secs: i64| {
            clip(id, Tick::SECOND * start, Tick::SECOND * secs, ClipContent::Solid { color: [1.0; 4] })
        };
        let mut p = Project::default();
        let mut b = solid("c11", 2, 2);
        b.transition = Some(Transition { kind: TransitionKind::FadeBlack, duration: Tick::SECOND });
        let mut c = solid("c12", 5, 1);
        // not touching c11: its transition is kept but does not play
        c.transition = Some(Transition { kind: TransitionKind::Dissolve, duration: Tick::SECOND });
        p.tracks[0].clips = vec![solid("c10", 0, 2), b, c];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let ids: Vec<&str> = spec.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["c10", "c11", "c11~dip", "c12"]);
        let half = Tick(Tick::SECOND.0 / 2);
        let (a, b, dip, c) = (&spec.layers[0], &spec.layers[1], &spec.layers[2], &spec.layers[3]);
        assert_eq!((a.at, a.dur), (Tick::ZERO, Tick::SECOND * 2 + half));
        assert_eq!((b.at, b.dur), (Tick::SECOND * 2 - half, Tick::SECOND * 2 + half));
        assert_eq!((dip.at, dip.dur), (Tick::SECOND * 2 - half, Tick::SECOND));
        assert_eq!(dip.opacity.sample(half), 1.0);
        // the incoming clip is hidden until the dip is full, then shown
        assert_eq!(b.opacity.sample(Tick::ZERO), 0.0);
        assert_eq!(b.opacity.sample(Tick::SECOND), 1.0);
        assert_eq!((c.at, c.dur), (Tick::SECOND * 5, Tick::SECOND));
        assert_eq!(c.opacity, Animated::Constant(1.0));
    }

    #[test]
    fn a_push_moves_both_clips_a_whole_frame() {
        use reelforge_model::{Transition, TransitionKind};
        let mut p = Project::default();
        let a = clip("c10", Tick::ZERO, Tick::SECOND * 2, ClipContent::Solid { color: [1.0; 4] });
        let mut b = clip("c11", Tick::SECOND * 2, Tick::SECOND * 2, ClipContent::Solid { color: [0.0; 4] });
        b.transition = Some(Transition { kind: TransitionKind::PushLeft, duration: Tick::SECOND * 10 });
        p.tracks[0].clips = vec![a, b];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        // the length is held to the shorter clip
        let (a, b) = (&spec.layers[0], &spec.layers[1]);
        assert_eq!(b.at, Tick::SECOND);
        assert_eq!(a.transform.position.sample(Tick::SECOND), [0.0, 0.0]);
        assert_eq!(a.transform.position.sample(Tick::SECOND * 3), [-1.0, 0.0]);
        assert_eq!(b.transform.position.sample(Tick::ZERO), [1.0, 0.0]);
        assert_eq!(b.transform.position.sample(Tick::SECOND * 2), [0.0, 0.0]);
        let mid = b.transform.position.sample(Tick::SECOND)[0] - a.transform.position.sample(Tick::SECOND * 2)[0];
        assert!((mid - 1.0).abs() < 1e-4, "{mid}");
    }
}
