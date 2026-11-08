//! Project → RenderSpec.
//!
//! This is the one place where editing meaning becomes rendering instructions. The renderer never
//! reads a project, so anything a project can express has to be lowered here: clips become layers
//! bottom track first, picture clips with sound and sound clips become audio clips, hidden tracks
//! drop their layers and muted tracks their sound. As the model grows (transitions, templates,
//! captions, speed curves), each feature gets its lowering here and the RenderSpec stays flat.

use std::collections::BTreeMap;

use reelforge_model::{ClipContent, MediaKind, Project, TrackKind};
use reelforge_renderspec::{
    Animated, Asset, AssetKind, AudioClip, AudioCodec, AudioEncoding, Blend, Color, Effect, Fit, Layer, LayerContent,
    Output, Probe, RenderSpec, TextContent, TimeMap, Transform, VideoCodec, VideoEncoding, validate,
};
use reelforge_time::Tick;
use thiserror::Error;

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
        for clip in &track.clips {
            let picture = track.kind != TrackKind::Audio && !track.hidden;
            if picture {
                let content = match &clip.content {
                    ClipContent::Media { media, source_in, speed } => LayerContent::Media {
                        asset: asset_for(media)?,
                        time: TimeMap { source_in: *source_in, speed: *speed },
                        fit: Fit::Contain,
                    },
                    ClipContent::Solid { color } => LayerContent::Solid { color: Color(*color) },
                    ClipContent::Text { text, size, color } => LayerContent::Text(TextContent {
                        text: text.clone(),
                        font: None,
                        size: *size,
                        color: Color(*color),
                    }),
                };
                spec.layers.push(Layer {
                    id: clip.id.clone(),
                    at: clip.start,
                    dur: clip.duration,
                    content,
                    transform: Transform {
                        position: Animated::Constant([clip.transform.x, clip.transform.y]),
                        scale: Animated::Constant(clip.transform.scale),
                        rotation: Animated::Constant(clip.transform.rotation),
                    },
                    opacity: Animated::Constant(clip.opacity),
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
                    gain_db: Animated::Constant(clip.volume_db),
                });
            }
        }
    }
    spec.assets = assets;
    spec.engine.features = spec.required_features().into_iter().collect();
    validate(&spec).map_err(|e| CompileError::Invalid(e.to_string()))?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelforge_model::{Clip, ClipTransform, Media};

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
            ClipContent::Text { text: "Hi".into(), size: 80.0, color: [1.0; 4] },
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
}
