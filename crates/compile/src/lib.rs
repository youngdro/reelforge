//! Project → RenderSpec.
//!
//! This is the one place where editing meaning becomes rendering instructions. The renderer never
//! reads a project, so anything a project can express has to be lowered here: clips become layers
//! bottom track first, picture clips with sound and sound clips become audio clips, hidden tracks
//! drop their layers and muted tracks their sound. As the model grows (transitions, templates,
//! captions, speed curves), each feature gets its lowering here and the RenderSpec stays flat.

use std::collections::BTreeMap;

use reelforge_model::{Clip, ClipContent, MediaKind, Project, Prop, TitleAlign, Track, TrackKind, TransitionKind};
use reelforge_renderspec::{
    Animated, Asset, AssetKind, AudioClip, AudioCodec, AudioEncoding, Blend, Color, Ease, Effect, Fit, Key, Layer,
    LayerContent, Output, Probe, RenderSpec, TextAlign, TextBackground, TextContent, TextShadow, TextStroke, TimeMap,
    Transform, VideoCodec, VideoEncoding, validate,
};
use reelforge_time::Tick;
use thiserror::Error;

mod motion;

use motion::{Overlay, Shape};

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
        fps: c.fps,
        duration,
        background: Color(c.background),
        video: options.encode.then_some(VideoEncoding { codec: VideoCodec::H264, crf: Some(20), bitrate_kbps: None }),
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
            let sha256 = m.sha256.clone().ok_or_else(|| CompileError::Unhashed(media_id.to_string()))?;
            assets.insert(
                media_id.to_string(),
                Asset {
                    kind: match m.kind {
                        MediaKind::Video => AssetKind::Video,
                        MediaKind::Image => AssetKind::Image,
                        MediaKind::Audio => AssetKind::Audio,
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
                let content = match &clip.content {
                    ClipContent::Media { media, source_in, speed } => LayerContent::Media {
                        asset: asset_for(media)?,
                        time: TimeMap { source_in: *source_in - shape.lead.scale(*speed), speed: *speed },
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
                    blend: Blend::Normal,
                    mask: None,
                    fx: clip
                        .effects
                        .iter()
                        .map(|e| Effect {
                            id: e.id.clone(),
                            params: e.params.iter().map(|(k, v)| (k.clone(), Animated::Constant(*v))).collect(),
                        })
                        .collect(),
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
                    });
                }
            }
            let sound = !track.muted
                && match &clip.content {
                    ClipContent::Media { media, .. } => project.media[media].kind != MediaKind::Image,
                    _ => false,
                };
            if sound && let ClipContent::Media { media, source_in, speed } = &clip.content {
                spec.audio.push(AudioClip {
                    id: clip.id.clone(),
                    asset: asset_for(media)?,
                    at: clip.start,
                    dur: clip.duration,
                    time: TimeMap { source_in: *source_in, speed: *speed },
                    gain_db: motion::lower(clip, Prop::VolumeDb, &Shape::of(clip), step),
                });
            }
        }
    }
    spec.assets = assets;
    spec.engine.features = spec.required_features().into_iter().collect();
    validate(&spec).map_err(|e| CompileError::Invalid(e.to_string()))?;
    Ok(spec)
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
        p.tracks[0].clips.push(clip(
            "c20",
            Tick::ZERO,
            Tick::SECOND * 2,
            ClipContent::Media { media: "m9".into(), source_in: Tick::SECOND, speed: 1.0 },
        ));
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
        p.tracks[0].clips.push(clip(
            "c20",
            Tick::ZERO,
            Tick::SECOND,
            ClipContent::Media { media: "m9".into(), source_in: Tick::ZERO, speed: 1.0 },
        ));
        assert_eq!(compile(&p, &CompileOptions::default()), Err(CompileError::Unhashed("m9".into())));
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
