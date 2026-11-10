//! Project → RenderSpec.
//!
//! This is the one place where editing meaning becomes rendering instructions. The renderer never
//! reads a project, so anything a project can express has to be lowered here: clips become layers
//! bottom track first, picture clips with sound and sound clips become audio clips, hidden tracks
//! drop their layers and muted tracks their sound. An adjustment clip becomes no layer: its effects
//! and colour are laid on the layers of the tracks below it, for as long as it covers them. As the
//! model grows (transitions, templates,
//! captions, speed curves), each feature gets its lowering here and the RenderSpec stays flat.

use std::collections::BTreeMap;

use reelforge_model::{
    Adjust, BlendMode, Clip, ClipContent, ClipTransform, Codec, Fill, FxChannel, Inbetween, Mask, MaskKind, MediaKind,
    Project, Prop, ShapeKind, Timing, TitleAlign, Track, TrackKind, TransitionKind, export::Format,
};
use reelforge_renderspec::{
    Animated, Asset, AssetKind, AudioClip, AudioCodec, AudioEncoding, Between, Blend, Color, Comp, Crop, CropKeys,
    Ease, Effect, Fit, Key, Layer, LayerContent, MaskShape, NumberText, Output, OutputFormat, Paint, Probe, RenderSpec,
    ShapeContent, ShapeKind as SpecShape, ShapeMask, ShapeMaskKeys, ShapeShadow, ShapeStroke, SpeedKey, TextAlign,
    TextBackground, TextContent, TextShadow, TextStroke, TimeMap, Transform, Trim, VideoCodec, VideoEncoding, effects,
    validate,
};
use reelforge_time::Tick;
use thiserror::Error;

mod motion;

use motion::{Frame, Overlay, Shape};

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
fn between(frames: Inbetween) -> Between {
    match frames {
        Inbetween::Repeat => Between::Nearest,
        Inbetween::Blend => Between::Blend,
        Inbetween::Flow => Between::Flow,
    }
}

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
    let e = &project.export;
    // what kind of file the encode settings are for
    let format = if options.encode { e.format } else { Format::Mp4 };
    let mut spec = RenderSpec::new(Output {
        width: c.width,
        height: c.height,
        fps: if options.encode { e.frame_rate(c) } else { c.fps },
        duration,
        background: Color(c.background),
        video: (options.encode && format.pictured()).then(|| {
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
        audio: (options.encode && format.sounds()).then_some(AudioEncoding {
            codec: AudioCodec::Aac,
            sample_rate: 48_000,
            bitrate_kbps: 192,
        }),
        format: match format {
            Format::Mp4 => OutputFormat::Mp4,
            Format::Gif => OutputFormat::Gif,
            Format::M4a => OutputFormat::M4a,
            Format::Wav => OutputFormat::Wav,
        },
    });

    let mut assets: BTreeMap<String, Asset> = BTreeMap::new();
    let mut comps: BTreeMap<String, Comp> = BTreeMap::new();
    let (layers, audio) = lower_tracks(project, &project.tracks, options, &mut assets, &mut comps)?;
    spec.layers = layers;
    spec.audio = audio;
    spec.comps = comps;
    spec.assets = assets;
    spec.engine.features = spec.required_features().into_iter().collect();
    validate(&spec).map_err(|e| CompileError::Invalid(e.to_string()))?;
    Ok(spec)
}

/// Lowers the clips of `tracks`, a timeline of `project`, into layers (bottom first) and audio
/// clips. The timelines its compound clips play are lowered first, into `comps` under their ids;
/// media goes into `assets`.
fn lower_tracks(
    project: &Project,
    tracks: &[Track],
    options: &CompileOptions,
    assets: &mut BTreeMap<String, Asset>,
    comps: &mut BTreeMap<String, Comp>,
) -> Result<(Vec<Layer>, Vec<AudioClip>), CompileError> {
    let c = &project.canvas;
    let step = c.fps.frame_duration();
    for clip in tracks.iter().flat_map(|t| &t.clips) {
        let Some((timeline, _)) = clip.compound() else { continue };
        if comps.contains_key(timeline) {
            continue;
        }
        let inner = project
            .tracks_of(timeline)
            .ok_or_else(|| CompileError::Model(format!("clip {:?} plays an unknown timeline", clip.id)))?;
        // held while it is lowered, so a timeline that played itself would end here
        comps.insert(timeline.to_string(), Comp::default());
        let (layers, audio) = lower_tracks(project, inner, options, assets, comps)?;
        comps.insert(timeline.to_string(), Comp { layers, audio });
    }
    let mut layers: Vec<Layer> = Vec::new();
    let mut audio: Vec<AudioClip> = Vec::new();
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
                        MediaKind::Font => AssetKind::Font,
                        MediaKind::Lottie => AssetKind::Lottie,
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

    for (ti, track) in tracks.iter().enumerate() {
        let cuts = transitions(track);
        // the adjustment clips above this track
        let above: Vec<&Clip> = tracks[ti + 1..]
            .iter()
            .filter(|t| t.kind == TrackKind::Video && !t.hidden)
            .flat_map(|t| t.clips.iter().filter(|c| c.is_adjustment()))
            .collect();
        for (i, clip) in track.clips.iter().enumerate() {
            let picture = track.kind != TrackKind::Audio && !track.hidden && !clip.is_adjustment();
            if picture {
                let mut shape = shape(clip, cuts[i], cuts.get(i + 1).copied().flatten());
                shape.canvas = [c.width as f32, c.height as f32];
                if let ClipContent::Media { media, .. } = &clip.content {
                    // a file whose size is not known is taken to fill the canvas
                    let size = project.media.get(media).and_then(|m| m.size).unwrap_or([c.width, c.height]);
                    shape.steady = motion::Fix::of(clip, size, shape.canvas);
                }
                let at = clip.start - shape.lead;
                // where each adjustment clip above covers this layer, in the layer's time
                let covers: Vec<(Tick, Tick, &Clip)> = above
                    .iter()
                    .filter_map(|a| {
                        let (from, to) = ((a.start - at).max(Tick::ZERO), (a.end() - at).min(shape.len));
                        (from < to).then_some((from, to, *a))
                    })
                    .collect();
                for (from, to, a) in &covers {
                    shape.frames.extend(a.fx.iter().map(|fx| Frame {
                        from: *from,
                        to: *to,
                        origin: a.start - at,
                        len: a.duration,
                        fx: *fx,
                    }));
                }
                let media = matches!(clip.content, ClipContent::Media { .. });
                // what is cropped, mirrored and put out of focus like a picture
                let pictured = clip.is_picture();
                let crop_keyed = Prop::CROP.iter().any(|p| clip.keyed(*p));
                let content = match &clip.content {
                    ClipContent::Media { media, frames, .. } => LayerContent::Media {
                        asset: asset_for(media)?,
                        time: time_map(&clip.timing().expect("a media clip"), shape.lead),
                        fit: Fit::Contain,
                        // a still has no frames to be between
                        between: match project.media.get(media).map(|m| m.kind) {
                            Some(MediaKind::Video) => between(*frames),
                            _ => Between::Nearest,
                        },
                    },
                    ClipContent::Solid { color } => LayerContent::Solid { color: Color(*color) },
                    ClipContent::Shape { shape: sh } => {
                        let stroked = sh.stroke.as_ref().is_some_and(|s| s.width > 0.0);
                        let (trim, fill_opacity) = motion::draw(clip, &shape, step, stroked);
                        LayerContent::Shape(Box::new(ShapeContent {
                            kind: match sh.kind {
                                ShapeKind::Rectangle => SpecShape::Rectangle,
                                ShapeKind::Ellipse => SpecShape::Ellipse,
                                ShapeKind::Line => SpecShape::Line,
                                ShapeKind::Polygon => SpecShape::Polygon,
                                ShapeKind::Star => SpecShape::Star,
                                ShapeKind::Tag => SpecShape::Tag,
                                ShapeKind::Arrow => SpecShape::Arrow,
                            },
                            size: sh.size,
                            radius: sh.radius,
                            points: sh.points,
                            inner: sh.inner,
                            fill: sh.fill.as_ref().map(|f| match f {
                                Fill::Solid { color } => Paint::Solid { color: Color(*color) },
                                Fill::Linear { from, to, angle } => {
                                    Paint::Linear { from: Color(*from), to: Color(*to), angle: *angle }
                                }
                                Fill::Radial { from, to } => Paint::Radial { from: Color(*from), to: Color(*to) },
                            }),
                            stroke: sh
                                .stroke
                                .as_ref()
                                .filter(|s| s.width > 0.0)
                                .map(|s| ShapeStroke { color: Color(s.color), width: s.width }),
                            shadow: sh.shadow.as_ref().map(|s| ShapeShadow {
                                color: Color(s.color),
                                offset: s.offset,
                                blur: s.blur,
                            }),
                            trim: trim.map(|[start, end]| Trim { start, end }),
                            fill_opacity,
                        }))
                    }
                    // its timeline's picture, from where the clip starts in it
                    ClipContent::Compound { timeline, source_in } => {
                        LayerContent::Comp { comp: timeline.clone(), from: *source_in - shape.lead }
                    }
                    ClipContent::Adjustment => unreachable!("an adjustment clip is not a picture"),
                    ClipContent::Text { text, size, color, style } => LayerContent::Text(Box::new(TextContent {
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
                            lines: style.line_boxes,
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
                        reveal: motion::reveal(clip, &shape),
                        chars: motion::chars(clip, &shape),
                        font: style.font.as_ref().map(|f| asset_for(f)).transpose()?,
                        tracking: style.tracking,
                        leading: style.leading,
                        italic: style.italic,
                        underline: style.underline,
                        number: style.counter.map(|c| NumberText {
                            value: motion::count(clip, &shape, step).expect("a counter"),
                            decimals: c.decimals.min(4),
                            group: c.group,
                            clock: c.clock,
                        }),
                        ..TextContent::new(text.clone(), *size, Color(*color))
                    })),
                };
                // a blurred copy filling the frame behind it
                if let (Some(amount), LayerContent::Media { asset, time, between, .. }) = (clip.look.backdrop, &content)
                {
                    let mut fx = color_fx(clip, &shape, step, &mut asset_for)?;
                    fx.extend(above_fx(clip, &shape, &covers, step));
                    fx.extend(distort_fx(clip, &shape, step));
                    // it fills the frame where it is, and moves only with the frame
                    let still = Clip {
                        transform: ClipTransform::default(),
                        keys: Default::default(),
                        anim: Default::default(),
                        fx: vec![],
                        ..clip.clone()
                    };
                    let frame = Shape { overlays: vec![], ..shape.clone() };
                    fx.push(Effect {
                        id: effects::BLUR.into(),
                        params: [(
                            "radius".to_string(),
                            Animated::Constant(reelforge_model::look::backdrop_radius(amount)),
                        )]
                        .into(),
                        asset: None,
                    });
                    layers.push(Layer {
                        id: format!("{}~backdrop", clip.id),
                        at: clip.start - shape.lead,
                        dur: shape.len,
                        content: LayerContent::Media {
                            asset: asset.clone(),
                            time: time.clone(),
                            fit: Fit::Cover,
                            // out of focus, a mix is as good as a frame made in between
                            between: if between.is_nearest() { Between::Nearest } else { Between::Blend },
                        },
                        transform: Transform {
                            position: motion::lower_position(&still, &frame, step),
                            scale: motion::lower(&still, Prop::Scale, &frame, step),
                            rotation: motion::lower(&still, Prop::Rotation, &frame, step),
                        },
                        opacity: motion::lower(clip, Prop::Opacity, &shape, step),
                        blend: Blend::Normal,
                        mask: None,
                        crop: None,
                        flip_x: clip.look.flip_x,
                        flip_y: clip.look.flip_y,
                        shape_mask: None,
                        crop_keys: None,
                        shape_mask_keys: None,
                        fx,
                    });
                }
                layers.push(Layer {
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
                    crop: (pictured && !crop_keyed && !clip.look.crop.is_zero()).then(|| {
                        let c = clip.look.crop;
                        Crop { left: c.left, top: c.top, right: c.right, bottom: c.bottom }
                    }),
                    flip_x: pictured && clip.look.flip_x,
                    flip_y: pictured && clip.look.flip_y,
                    shape_mask: clip.look.mask.map(shape_mask),
                    crop_keys: (pictured && crop_keyed).then(|| {
                        let [left, top, right, bottom] = Prop::CROP.map(|p| motion::lower(clip, p, &shape, step));
                        CropKeys { left, top, right, bottom }
                    }),
                    shape_mask_keys: (clip.look.mask.is_some() && Prop::MASK.iter().any(|p| clip.keyed(*p))).then(
                        || ShapeMaskKeys {
                            center: motion::lower_pair(clip, [Prop::MaskX, Prop::MaskY], &shape, step),
                            size: motion::lower_pair(clip, [Prop::MaskWidth, Prop::MaskHeight], &shape, step),
                            rotation: motion::lower(clip, Prop::MaskRotation, &shape, step),
                            feather: motion::lower(clip, Prop::MaskFeather, &shape, step),
                        },
                    ),
                    fx: {
                        // people are cut out of the picture as it was shot, and a keyed colour
                        // goes from that before anything changes the colours
                        let mut fx: Vec<Effect> = Vec::new();
                        if media && clip.look.person {
                            fx.push(Effect { id: effects::PERSON_KEY.into(), params: Default::default(), asset: None });
                        }
                        fx.extend(clip.look.chroma.iter().filter(|_| media).map(chroma_fx));
                        fx.extend(color_fx(clip, &shape, step, &mut asset_for)?);
                        fx.extend(clip.effects.iter().map(|e| Effect {
                            id: e.id.clone(),
                            params: e.params.iter().map(|(k, v)| (k.clone(), Animated::Constant(*v))).collect(),
                            asset: None,
                        }));
                        fx.extend(above_fx(clip, &shape, &covers, step));
                        fx.extend(distort_fx(clip, &shape, step));
                        // out of focus: pictures only, and one blur a layer
                        if pictured
                            && !fx.iter().any(|e| e.id == effects::BLUR)
                            && let Some(radius) = motion::fx_channel(clip, &shape, FxChannel::Blur, step)
                        {
                            fx.push(Effect {
                                id: effects::BLUR.into(),
                                params: [("radius".to_string(), radius)].into(),
                                asset: None,
                            });
                        }
                        fx
                    },
                });
                // a dip passes through a colour layer over both clips
                if let Some((kind, half)) = cuts[i]
                    && let Some(color) = kind.dip()
                {
                    layers.push(Layer {
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
                        crop_keys: None,
                        shape_mask_keys: None,
                    });
                }
            }
            let sound = !track.muted
                && match &clip.content {
                    ClipContent::Media { media, silent, .. } => !silent && project.media[media].kind.sounds(),
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
                let pitch = if time.reverses() { 0.0 } else { clip.voice.semitones() };
                audio.push(AudioClip {
                    id: clip.id.clone(),
                    asset: asset_for(media)?,
                    comp: None,
                    at: clip.start,
                    dur: clip.duration,
                    time,
                    gain_db: motion::lower(clip, Prop::VolumeDb, &Shape::of(clip), step),
                    keep_pitch,
                    pitch,
                    fx: sound_fx(&clip.voice),
                });
            }
            // a compound clip plays its timeline's mix, as loud as the clip is set
            if !track.muted
                && let Some((timeline, source_in)) = clip.compound()
                && comps.get(timeline).is_some_and(|c| !c.audio.is_empty())
            {
                audio.push(AudioClip {
                    id: clip.id.clone(),
                    asset: String::new(),
                    comp: Some(timeline.to_string()),
                    at: clip.start,
                    dur: clip.duration,
                    time: TimeMap { source_in, speed: 1.0, ramp: vec![] },
                    gain_db: motion::lower(clip, Prop::VolumeDb, &Shape::of(clip), step),
                    keep_pitch: false,
                    pitch: 0.0,
                    fx: vec![],
                });
            }
        }
    }
    Ok((layers, audio))
}

/// A clip's colour as effects: its filter, its lookup table, then its own adjustments.
fn color_fx(
    clip: &Clip,
    shape: &Shape,
    step: Tick,
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
    let keyed = Prop::ADJUST.iter().any(|p| clip.keyed(*p));
    if keyed {
        // the adjustments with keyframes move; the others are as constant as ever
        fx.push(Effect {
            id: effects::GRADE.into(),
            params: Adjust::RANGES
                .iter()
                .zip(Prop::ADJUST)
                .filter(|(_, p)| clip.keyed(*p) || clip.base(*p) != 0.0)
                .map(|((name, ..), p)| (name.to_string(), motion::lower(clip, p, shape, step)))
                .collect(),
            asset: None,
        });
    } else if !c.adjust.is_zero() {
        fx.push(grade(c.adjust));
    }
    Ok(fx)
}

/// What effects and the adjustment clips above (`covers`: where each lies on the layer) do to a
/// layer's light and colour: the effects' own grade, sampled while they play, then the adjustment
/// clips' filters and their adjustments, each as one grade that steps in and out. Where two
/// adjustment clips overlap, their values add up.
fn above_fx(clip: &Clip, shape: &Shape, covers: &[(Tick, Tick, &Clip)], step: Tick) -> Vec<Effect> {
    let mut fx = Vec::new();
    let played: Vec<(String, Animated<f32>)> =
        [("exposure", FxChannel::Exposure), ("saturation", FxChannel::Saturation), ("vignette", FxChannel::Vignette)]
            .into_iter()
            .filter_map(|(name, ch)| Some((name.to_string(), motion::fx_channel(clip, shape, ch, step)?)))
            .collect();
    if !played.is_empty() {
        fx.push(Effect { id: effects::GRADE.into(), params: played.into_iter().collect(), asset: None });
    }
    let filter = |a: &Clip| {
        a.color.filter.filter(|f| f.strength > 0.0).map_or(Adjust::default(), |f| f.kind.adjust().scaled(f.strength))
    };
    let adjust = |a: &Clip| a.color.adjust;
    let both: [&dyn Fn(&Clip) -> Adjust; 2] = [&filter, &adjust];
    for values in both {
        let params: Vec<(String, Animated<f32>)> = Adjust::RANGES
            .iter()
            .enumerate()
            .filter_map(|(i, (name, lo, hi))| {
                let spans: Vec<(Tick, Tick, f32)> =
                    covers.iter().map(|(from, to, a)| (*from, *to, values(a).values()[i])).collect();
                Some((name.to_string(), motion::stepped(&spans, shape.len, (*lo, *hi))?))
            })
            .collect();
        if !params.is_empty() {
            fx.push(Effect { id: effects::GRADE.into(), params: params.into_iter().collect(), asset: None });
        }
    }
    fx
}

/// A clip's keyed colour as `key.chroma@1`.
fn chroma_fx(k: &reelforge_model::ChromaKey) -> Effect {
    let values = [
        ("r", k.color[0]),
        ("g", k.color[1]),
        ("b", k.color[2]),
        ("tolerance", k.tolerance),
        ("softness", k.softness),
        ("spill", k.spill),
    ];
    Effect {
        id: effects::CHROMA_KEY.into(),
        params: values.into_iter().map(|(name, v)| (name.to_string(), Animated::Constant(v))).collect(),
        asset: None,
    }
}

/// How the clip's own effects and those of adjustment clips above distort a layer's picture:
/// one `distort.*` effect for each kind that plays, sampled while it does.
fn distort_fx(clip: &Clip, shape: &Shape, step: Tick) -> Vec<Effect> {
    let kinds: [(&str, &[(&str, FxChannel)]); 4] = [
        (effects::WAVE, &[("amount", FxChannel::Wave), ("phase", FxChannel::WavePhase)]),
        (effects::GLITCH, &[("amount", FxChannel::Glitch), ("seed", FxChannel::GlitchSeed)]),
        (effects::MOSAIC, &[("size", FxChannel::Mosaic)]),
        (effects::SPLIT, &[("x", FxChannel::Split)]),
    ];
    kinds
        .into_iter()
        .filter_map(|(id, channels)| {
            let params: BTreeMap<String, Animated<f32>> = channels
                .iter()
                .filter_map(|(name, ch)| Some((name.to_string(), motion::fx_channel(clip, shape, *ch, step)?)))
                .collect();
            (!params.is_empty()).then(|| Effect { id: id.into(), params, asset: None })
        })
        .collect()
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
            // an adjustment clip is no picture to cut from or to
            if track.kind != TrackKind::Video
                || prev.end() != clip.start
                || prev.is_adjustment()
                || clip.is_adjustment()
            {
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
    Shape { lead, len: lead + clip.duration + tail, overlays, frames: vec![], canvas: [1.0, 1.0], steady: None }
}

/// What a clip's changed voice does to its sound, in the order it is done: noise out first, then
/// the voice's own parts.
fn sound_fx(voice: &reelforge_model::Voice) -> Vec<reelforge_renderspec::AudioEffect> {
    use reelforge_model::VoicePart;
    use reelforge_renderspec::AudioEffect;
    let mut out = Vec::new();
    if let Some(d) = voice.denoise.as_ref().filter(|d| d.amount > 0.0) {
        out.push(AudioEffect::Denoise { floor: d.floor.clone(), amount: d.amount });
    }
    for part in voice.kind.map_or(&[][..], |k| k.parts()) {
        out.push(match *part {
            VoicePart::Band(low, high) => AudioEffect::Band { low, high },
            VoicePart::Ring(hz, mix) => AudioEffect::Ring { hz, mix },
            VoicePart::Echo(delay, decay, repeats) => AudioEffect::Echo { delay, decay, repeats },
        });
    }
    out
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
            fx: vec![],
            voice: Default::default(),
            group: None,
            slot: None,
            steady: None,
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
                folder: None,
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
                folder: None,
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

    #[test]
    fn a_clips_own_effects_are_sampled_while_it_lasts() {
        use reelforge_model::{Fx, FxKind};
        let mut p = Project::default();
        let mut a = clip("c10", Tick::ZERO, Tick::SECOND * 2, ClipContent::Solid { color: [1.0; 4] });
        a.fx = vec![Fx::new(FxKind::Shake), Fx::new(FxKind::Flash)];
        p.tracks[0].clips = vec![a.clone()];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let l = &spec.layers[0];
        let Animated::Keys(k) = &l.transform.position else { panic!("the shake moves it") };
        // a key a frame
        assert_eq!(k.len(), 61);
        assert!(k.iter().any(|k| k.v[0] != 0.0) && k.iter().all(|k| k.v[0].abs() <= 0.02));
        assert!((l.transform.scale.sample(Tick::SECOND) - 1.05).abs() < 1e-6);
        assert_eq!(l.opacity, Animated::Constant(1.0));
        // the flash is a grade whose exposure falls from each beat
        let grade = l.fx.iter().find(|e| e.id == effects::GRADE).expect("a grade");
        assert_eq!(grade.params.keys().collect::<Vec<_>>(), ["exposure"]);
        let e = &grade.params["exposure"];
        assert!((e.sample(Tick::ZERO) - 2.5).abs() < 1e-6 && e.sample(Tick(Tick::SECOND.0 / 2)) < 0.5);
        assert!((e.sample(Tick::SECOND) - 2.5).abs() < 1e-6);
        // a colour has no focus to lose
        a.fx = vec![Fx::new(FxKind::BlurIn)];
        p.tracks[0].clips = vec![a];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert!(spec.layers[0].fx.is_empty());
    }

    #[test]
    fn a_steadied_clip_is_moved_against_its_shake_frame_by_frame() {
        use reelforge_model::Steady;
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
                folder: None,
            },
        );
        // a tremble across and a wobble, watched from one second into the file
        let step = Tick(Tick::SECOND.0 / 30);
        let path: Vec<[f32; 3]> =
            (0..90).map(|i| [0.01 * (i as f32 * 1.9).sin(), 0.0, 0.3 * (i as f32 * 1.3).sin()]).collect();
        let steady = Steady { amount: 0.5, from: Tick::SECOND, step, path };
        let mut c = clip("c20", Tick::ZERO, Tick::SECOND * 2, ClipContent::media("m9", Tick::SECOND));
        c.steady = Some(steady.clone());
        p.tracks[0].clips.push(c);
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let l = &spec.layers[0];
        let fixed = steady.steadied(16.0 / 9.0);
        assert!(fixed.zoom > 1.0);
        // drawn larger by as much all the way, and a key a frame for where it is and how it is turned
        assert!([0, 20, 59].iter().all(|&i| l.transform.scale.sample(step * i) == fixed.zoom));
        let Animated::Keys(k) = &l.transform.position else { panic!("it is moved") };
        assert_eq!(k.len(), 61);
        // the wide picture fits the tall canvas across, so a move of a hundredth of the picture is
        // a hundredth of the canvas, as large as the picture is drawn and the way it is turned
        for i in [0usize, 7, 30, 59] {
            let [x, _, turn] = fixed.moves[i];
            let at = step * i as i64;
            assert!((l.transform.rotation.sample(at) - turn).abs() < 1e-5, "{i}");
            let [dx, dy] = l.transform.position.sample(at);
            let (sin, cos) = turn.to_radians().sin_cos();
            assert!((dx - cos * x * fixed.zoom).abs() < 1e-5, "{i}: {dx}");
            assert!((dy - sin * x * fixed.zoom * 1080.0 / 1920.0).abs() < 1e-5, "{i}: {dy}");
        }
        assert!(k.iter().any(|k| k.v[0].abs() > 0.004));
        // mirrored, the picture shook the other way; at half speed each frame of the file lasts two
        let c = &mut p.tracks[0].clips[0];
        c.look.flip_x = true;
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let m = &spec.layers[0];
        let [x, _, turn] = fixed.moves[7];
        assert!((m.transform.rotation.sample(step * 7) + turn).abs() < 1e-5);
        let (sin, cos) = (-turn).to_radians().sin_cos();
        let [dx, dy] = m.transform.position.sample(step * 7);
        assert!((dx + cos * x * fixed.zoom).abs() < 1e-5 && (dy + sin * x * fixed.zoom * 0.5625).abs() < 1e-5);
        // without it nothing is keyed
        p.tracks[0].clips[0].steady = None;
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert_eq!(spec.layers[0].transform.position, Animated::Constant([0.0, 0.0]));
        assert_eq!(spec.layers[0].transform.scale, Animated::Constant(1.0));
    }

    #[test]
    fn people_are_cut_out_of_a_clip_before_anything_else_is_done_to_it() {
        let mut p = Project::default();
        p.media.insert(
            "m9".into(),
            Media {
                id: "m9".into(),
                name: "m9".into(),
                uri: "file:///m9".into(),
                sha256: Some("ab".repeat(32)),
                kind: MediaKind::Video,
                duration: Some(Tick::SECOND * 10),
                size: Some([1920, 1080]),
                folder: None,
            },
        );
        let mut video = clip("c20", Tick::ZERO, Tick::SECOND * 2, ClipContent::media("m9", Tick::SECOND));
        video.look.person = true;
        video.look.chroma = Some(reelforge_model::ChromaKey::new([0.1, 0.8, 0.2]));
        video.look.backdrop = Some(0.5);
        // a colour has no people to cut out, whatever its look says
        let mut flat = clip("c21", Tick::SECOND * 2, Tick::SECOND, ClipContent::Solid { color: [1.0; 4] });
        flat.look.person = true;
        p.tracks[0].clips = vec![video, flat];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let fx = |id: &str| -> Vec<&str> {
            spec.layers.iter().find(|l| l.id == id).expect("a layer").fx.iter().map(|e| e.id.as_str()).collect()
        };
        // the people first, then the colour keyed out of what is left; the blurred copy behind
        // stays whole (the people are cut out of the clip, not out of the frame)
        assert_eq!(fx("c20"), ["key.person@1", "key.chroma@1"]);
        assert_eq!(fx("c20~backdrop"), ["blur.gaussian@1"]);
        assert!(fx("c21").is_empty());
        assert!(spec.engine.features.contains(&"key.person@1".to_string()));
        assert!(spec.layers.iter().find(|l| l.id == "c20").unwrap().fx[0].params.is_empty());
        // off: nothing is asked of the renderer
        p.tracks[0].clips[0].look.person = false;
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert!(!spec.engine.features.contains(&"key.person@1".to_string()));
    }

    #[test]
    fn a_slowed_video_asks_for_pictures_between_its_frames() {
        use reelforge_model::Inbetween;
        let mut p = Project::default();
        for (id, kind) in [("m9", MediaKind::Video), ("m10", MediaKind::Image)] {
            p.media.insert(
                id.into(),
                Media {
                    id: id.into(),
                    name: id.into(),
                    uri: format!("file:///{id}"),
                    sha256: Some(if kind == MediaKind::Video { "ab" } else { "cd" }.repeat(32)),
                    kind,
                    duration: (kind == MediaKind::Video).then_some(Tick::SECOND * 10),
                    size: Some([1920, 1080]),
                    folder: None,
                },
            );
        }
        let with = |frames: Inbetween, media: &str| {
            let mut content = ClipContent::media(media, Tick::SECOND);
            if let ClipContent::Media { frames: f, speed, .. } = &mut content {
                (*f, *speed) = (frames, 0.25);
            }
            content
        };
        let mut video = clip("c20", Tick::ZERO, Tick::SECOND * 2, with(Inbetween::Flow, "m9"));
        video.look.backdrop = Some(0.5);
        p.tracks[0].clips = vec![video, clip("c21", Tick::SECOND * 2, Tick::SECOND, with(Inbetween::Blend, "m10"))];
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let between = |id: &str| match &spec.layers.iter().find(|l| l.id == id).expect("a layer").content {
            LayerContent::Media { between, .. } => *between,
            _ => panic!("media"),
        };
        // the video's frames are made; behind it, out of focus, they are only mixed; a still has none
        assert_eq!(between("c20"), Between::Flow);
        assert_eq!(between("c20~backdrop"), Between::Blend);
        assert_eq!(between("c21"), Between::Nearest);
        let needs = &spec.engine.features;
        assert!(needs.contains(&"frames.flow/1".to_string()) && needs.contains(&"frames.blend/1".to_string()));
        // as the file has them: nothing is asked of the renderer
        if let ClipContent::Media { frames, .. } = &mut p.tracks[0].clips[0].content {
            *frames = Inbetween::Repeat;
        }
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert!(!spec.engine.features.iter().any(|f| f.starts_with("frames.")));
        assert!(!spec.to_json().contains("between"));
    }

    #[test]
    fn an_adjustment_clip_works_on_what_is_below_it_while_it_lasts() {
        use reelforge_model::{Filter, FilterKind, Fx, FxKind, Transition, TransitionKind};
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
                size: Some([1080, 1920]),
                folder: None,
            },
        );
        let mut below = clip("c10", Tick::ZERO, Tick::SECOND * 4, ClipContent::media("m9", Tick::ZERO));
        below.transform.x = 0.25;
        p.tracks[0].clips = vec![below];
        let mut adj = clip("c11", Tick::SECOND, Tick::SECOND * 2, ClipContent::Adjustment);
        adj.fx = vec![Fx::new(FxKind::ZoomOut), Fx::new(FxKind::BlurIn)];
        adj.color.adjust.exposure = 1.0;
        adj.color.filter = Some(Filter { kind: FilterKind::ALL[0], strength: 1.0 });
        let mut over = p.tracks[0].clone();
        over.id = "t9".into();
        over.clips = vec![adj];
        p.tracks.insert(1, over);
        // a title above it is left alone
        let text = p.tracks.iter().position(|t| t.kind == TrackKind::Text).unwrap();
        p.tracks[text].clips.push(clip(
            "c12",
            Tick::ZERO,
            Tick::SECOND * 4,
            ClipContent::Text { text: "Hi".into(), size: 80.0, color: [1.0; 4], style: TitleStyle::subtitle() },
        ));
        p.check().unwrap();
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert_eq!(spec.layers.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["c10", "c12"]);
        assert_eq!(spec.output.duration, Tick::SECOND * 4);
        let (l, title) = (&spec.layers[0], &spec.layers[1]);
        // outside the adjustment clip nothing changes; inside, the frame is pulled back about its
        // centre, so what is off-centre moves out with it
        assert_eq!(l.transform.scale.sample(Tick(Tick::SECOND.0 / 2)), 1.0);
        assert!((l.transform.scale.sample(Tick::SECOND) - 1.3).abs() < 1e-6);
        assert!((l.transform.position.sample(Tick::SECOND)[0] - 0.25 * 1.3).abs() < 1e-6);
        assert!((l.transform.scale.sample(Tick::SECOND * 2) - 1.15).abs() < 1e-4);
        assert_eq!(l.transform.position.sample(Tick::SECOND * 3), [0.25, 0.0]);
        assert_eq!(title.transform.scale, Animated::Constant(1.0));
        assert!(title.fx.is_empty());
        // its filter and its adjustments step in and out, one grade each
        let grades: Vec<&Effect> = l.fx.iter().filter(|e| e.id == effects::GRADE).collect();
        assert_eq!(grades.len(), 2);
        let exposure = &grades[1].params["exposure"];
        assert_eq!(
            [0, 1, 2, 3].map(|s| exposure.sample(Tick::SECOND * s + Tick(1))),
            [0.0, 1.0, 1.0, 0.0],
            "{exposure:?}"
        );
        // out of focus when it starts, sharp a second later
        let blur = &l.fx.iter().find(|e| e.id == effects::BLUR).expect("a blur").params["radius"];
        assert_eq!(blur.sample(Tick(Tick::SECOND.0 / 2)), 0.0);
        assert!((blur.sample(Tick::SECOND) - 0.05).abs() < 1e-6);
        assert_eq!(blur.sample(Tick::SECOND * 2), 0.0);

        // a hidden track's adjustment clips do nothing, and none takes part in a transition
        p.tracks[1].hidden = true;
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        assert!(spec.layers[0].fx.is_empty());
        assert_eq!(spec.layers[0].transform.scale, Animated::Constant(1.0));
        p.tracks[1].hidden = false;
        let mut next = clip("c13", Tick::SECOND * 3, Tick::SECOND, ClipContent::Solid { color: [1.0; 4] });
        next.transition = Some(Transition { kind: TransitionKind::Dissolve, duration: Tick::SECOND });
        p.tracks[1].clips.push(next);
        let spec = compile(&p, &CompileOptions::default()).unwrap();
        let c13 = spec.layers.iter().find(|l| l.id == "c13").unwrap();
        assert_eq!((c13.at, c13.dur), (Tick::SECOND * 3, Tick::SECOND));
    }
}
