//! The built-in decorations (`decor.list`, `decor.add`): small Lottie animations that come with
//! the program, placed over the picture as clips like any other and drawn in the brand's colours.
//! What there is, and how one is told its colours, is in [`reelforge_media::decor`].

use reelforge_media::decor::{self, DECOR, LibraryDecor};
use reelforge_model::{ClipContent, ClipTransform, Media, MediaKind, TrackKind};
use reelforge_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{fps, not_found};
use crate::params::P;
use crate::{CmdResult, EngineError, Session};

/// `decor.list`.
pub(crate) fn decor_list(_: &mut Session, _: &Value) -> CmdResult {
    let list: Vec<Value> = DECOR
        .iter()
        .map(|d| {
            let colours: Vec<&str> = ["color", "color2"]
                .into_iter()
                .zip(d.colours)
                .filter_map(|(name, used)| used.then_some(name))
                .collect();
            json!({
                "id": d.id,
                "name": d.en,
                "nameZh": d.zh,
                "seconds": d.seconds,
                "plays": if d.once { "once" } else { "round" },
                "scale": d.scale,
                "colours": colours,
            })
        })
        .collect();
    Ok(json!({ "decor": list }))
}

fn known(id: &str) -> Result<&'static LibraryDecor, EngineError> {
    decor::find(id).ok_or_else(|| {
        EngineError::Params(format!(
            "`decor` must be one of {}",
            DECOR.iter().map(|d| d.id).collect::<Vec<_>>().join(", ")
        ))
    })
}

/// `decor.add`.
pub(crate) fn decor_add(s: &mut Session, v: &Value) -> CmdResult {
    let p = P::new(v, fps(s));
    let d = known(p.str("decor")?)?;
    let start = p.opt_time("start")?.unwrap_or(Tick::ZERO);
    let duration = p.opt_time("duration")?.unwrap_or(Tick::from_seconds(f64::from(d.seconds)));
    if start < Tick::ZERO || duration <= Tick::ZERO {
        return Err(EngineError::Params("`start` must not be negative and `duration` must be positive".into()));
    }
    let place = |name: &str| p.opt_f64(name).map(|o| o.map(|x| x as f32));
    let (x, y, scale, rotation, opacity) =
        (place("x")?, place("y")?, place("scale")?, place("rotation")?, place("opacity")?);
    if opacity.is_some_and(|o| !(0.0..=1.0).contains(&o)) || scale.is_some_and(|k| k <= 0.0) {
        return Err(EngineError::Params("`opacity` must be between 0 and 1 and `scale` positive".into()));
    }
    let (track, name) = (p.opt_str("track")?.map(str::to_string), p.opt_str("name")?.unwrap_or(d.en).to_string());
    // the colours the command says, else the brand's, else the ones it is drawn in
    let brand = &s.project.brand;
    let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
    let primary = p.opt_color("color")?.or(brand.primary).map(rgb);
    let secondary = p.opt_color("color2")?.or(brand.secondary).map(rgb);
    let uri = decor::uri(d, primary, secondary);
    // read once here for its hash, length and size; the same bytes wherever it shows
    let known = s.project().media.values().find(|m| m.uri == uri).map(|m| m.id.clone());
    let measured = match known {
        Some(_) => None,
        None => {
            use sha2::{Digest, Sha256};
            let bytes = decor::file(&uri).expect("a library decoration");
            let a = decor::open(&uri).expect("a library decoration");
            let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            Some((sha, Tick::from_seconds(a.seconds()), decor::picture_size(d, &a)))
        }
    };
    s.edit("Add Decoration", |proj| {
        let span = TimeRange::new(start, duration);
        let ti = match &track {
            Some(t) => {
                let ti = proj.tracks.iter().position(|x| x.id == *t).ok_or_else(|| not_found("track", t))?;
                let tr = &proj.tracks[ti];
                if tr.kind != TrackKind::Video {
                    return Err(EngineError::Params(format!("{t} is not a video track")));
                }
                if tr.locked || tr.clips.iter().any(|c| c.range().overlaps(span)) {
                    return Err(EngineError::Refused(format!("{t} has no room there")));
                }
                ti
            }
            // over whatever shows then: a free video track above the last one with a clip there
            None => {
                let busy = |t: &reelforge_model::Track| {
                    t.kind == TrackKind::Video && t.clips.iter().any(|c| c.range().overlaps(span))
                };
                let above = proj.tracks.iter().rposition(busy).map_or(0, |i| i + 1);
                crate::promo::free_track(proj, TrackKind::Video, above, start, duration)
            }
        };
        let media = match (known, measured) {
            (Some(m), _) => m,
            (None, Some((sha, length, size))) => {
                let m = proj.mint("m");
                let item = Media {
                    id: m.clone(),
                    name,
                    uri,
                    sha256: Some(sha),
                    kind: MediaKind::Lottie,
                    duration: Some(length),
                    size: Some(size),
                    folder: None,
                };
                proj.media.insert(m.clone(), item);
                m
            }
            (None, None) => unreachable!("measured when not known"),
        };
        // 1 fits the canvas; one that should cover it is larger by how much their shapes differ
        let fit = || {
            let (c, size) = (&proj.canvas, proj.media[&media].size.unwrap_or([1, 1]));
            let (across, down) = (c.width as f32 / size[0] as f32, c.height as f32 / size[1] as f32);
            across.max(down) / across.min(down)
        };
        let scale = scale.unwrap_or_else(|| d.scale.unwrap_or_else(fit));
        let id = proj.mint("c");
        let mut clip =
            crate::promo::plain_clip(id.clone(), start, duration, ClipContent::media(media.clone(), Tick::ZERO));
        clip.transform = ClipTransform {
            x: x.unwrap_or(0.0),
            y: y.unwrap_or(0.0),
            scale,
            rotation: rotation.unwrap_or(0.0),
            ..ClipTransform::default()
        };
        clip.opacity = opacity.unwrap_or(1.0);
        let track = proj.tracks[ti].id.clone();
        proj.tracks[ti].clips.push(clip);
        proj.tracks[ti].clips.sort_by_key(|c| c.start);
        Ok(json!({ "clip": id, "media": media, "track": track, "start": start, "duration": duration }))
    })
}
