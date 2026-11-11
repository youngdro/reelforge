//! What the timeline draws inside media clips: pictures along video and image clips, the waveform
//! of clips with sound.
//!
//! Both are made on the editor's thread (the browser has no other), a few milliseconds' worth per
//! frame and none while playing, so playback keeps the decoder to itself. Pictures are taken on a
//! grid of source times that halves as the timeline zooms in, so zooming reuses what was made;
//! until a picture is ready the nearest one made is shown. Waveforms are the peak of every
//! [`PEAKS_PER_SECOND`]th of a second, read from the start of the file onwards.

use std::collections::{BTreeMap, HashMap, VecDeque};

use eframe::egui::{self, Color32, Rect, Stroke, pos2};
use reelforge_media::Footage;
use reelforge_model::{Media, MediaKind, Timing};
use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::{TICKS_PER_SECOND, Tick};
use web_time::{Duration, Instant};

/// Waveform resolution.
pub(crate) const PEAKS_PER_SECOND: usize = 200;
/// The rate sound is read at for waveforms; peaks need no more.
const WAVE_RATE: u32 = 8_000;
/// Sound read per step, in seconds.
const WAVE_STEP_SECONDS: usize = 4;
/// Time spent making pictures and waveforms per frame.
const BUDGET: Duration = Duration::from_millis(12);
/// Pictures kept before the cache starts over.
const MAX_PICTURES: usize = 3_000;

/// Pictures and waveforms by file.
#[derive(Default)]
pub(crate) struct Strips {
    /// Pictures by file, by source time in ticks; `None` for one that could not be made.
    pictures: HashMap<String, BTreeMap<i64, Option<egui::TextureHandle>>>,
    count: usize,
    waves: HashMap<String, Wave>,
    /// Pictures asked for this frame and not made yet.
    wanted: VecDeque<(Asset, Tick)>,
    /// Files whose waveform was asked for this frame and is not complete.
    wanted_waves: Vec<(Asset, Tick)>,
    /// The picture height made, in pixels.
    height: u32,
    /// Files that were offline at [`Footage::generation`] `seen`; not asked for again until it
    /// changes (a joined editor's files arrive after the project).
    offline: std::collections::HashSet<String>,
    seen: u64,
}

#[derive(Default)]
struct Wave {
    peaks: Vec<f32>,
    done: bool,
}

/// The renderer's description of a model file, for [`Footage`].
pub(crate) fn asset(m: &Media) -> Asset {
    Asset {
        kind: match m.kind {
            MediaKind::Video => AssetKind::Video,
            MediaKind::Image => AssetKind::Image,
            MediaKind::Audio => AssetKind::Audio,
            MediaKind::Lut => AssetKind::Lut,
            MediaKind::Font => AssetKind::Font,
            MediaKind::Lottie => AssetKind::Lottie,
        },
        sha256: m.sha256.clone().unwrap_or_default(),
        uri: m.uri.clone(),
        of: None,
        probe: None,
    }
}

fn key(a: &Asset) -> &str {
    if a.sha256.is_empty() { &a.uri } else { &a.sha256 }
}

/// Where a clip's source is: how it runs through its media, from its start on the timeline.
#[derive(Clone)]
pub(crate) struct Source {
    pub timing: Timing,
    pub start: Tick,
}

impl Source {
    fn at(&self, t: f64) -> f64 {
        self.timing.source_at(Tick::from_seconds(t) - self.start).seconds()
    }
}

impl Strips {
    /// Lays pictures of `m` along `r`, a clip block. `t_of` gives the timeline second at an x.
    pub(crate) fn paint_pictures(
        &mut self,
        painter: &egui::Painter,
        r: Rect,
        m: &Media,
        src: Source,
        t_of: impl Fn(f32) -> f64,
        tint: Color32,
    ) {
        let h = r.height();
        let aspect = m.size.map_or(16.0 / 9.0, |[w, h]| w as f32 / h.max(1) as f32).clamp(0.2, 5.0);
        let w = (h * aspect).max(8.0);
        let ppp = painter.ctx().pixels_per_point();
        self.height = (h * ppp).round().max(8.0) as u32;
        let a = asset(m);
        let visible = painter.clip_rect().intersect(r);
        if visible.width() <= 0.0 {
            return;
        }
        // a grid of source times no finer than one picture's worth, so zooming reuses pictures
        let span = ((t_of(r.left() + w) - t_of(r.left())) * src.timing.speed.abs()).max(1e-3);
        let grid = 2f64.powf(span.log2().floor()).max(1.0 / 64.0);
        let duration = m.duration.map_or(f64::MAX, |d| d.seconds());
        let first = ((visible.left() - r.left()) / w).floor() as i32;
        let last = ((visible.right() - r.left()) / w).ceil() as i32;
        for i in first.max(0)..last {
            let x = r.left() + i as f32 * w;
            let at = if m.kind == MediaKind::Image { 0.0 } else { (src.at(t_of(x + w * 0.5)) / grid).floor() * grid };
            let at = Tick::from_seconds(at.clamp(0.0, duration));
            let texture = self.picture(&a, at);
            let Some(texture) = texture else { continue };
            let tile = Rect::from_min_size(pos2(x, r.top()), egui::vec2(w, h));
            painter.image(texture.id(), tile, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            painter
                .line_segment([tile.right_top(), tile.right_bottom()], Stroke::new(1.0, Color32::from_black_alpha(60)));
        }
    }

    /// The picture at `at`, or the nearest made so far (asking for this one).
    fn picture(&mut self, a: &Asset, at: Tick) -> Option<egui::TextureHandle> {
        let made = self.pictures.get(key(a));
        match made.and_then(|m| m.get(&at.0)) {
            Some(texture) => return texture.clone(),
            None if self.offline.contains(key(a)) => {}
            None if !self.wanted.iter().any(|(w, t)| *t == at && key(w) == key(a)) => {
                self.wanted.push_back((a.clone(), at));
            }
            None => {}
        }
        let made = made?;
        let before = made.range(..at.0).rev().find_map(|(t, x)| x.as_ref().map(|x| (at.0 - t, x)));
        let after = made.range(at.0..).find_map(|(t, x)| x.as_ref().map(|x| (t - at.0, x)));
        match (before, after) {
            (Some(b), Some(a)) => Some(if b.0 <= a.0 { b.1 } else { a.1 }.clone()),
            (Some((_, x)), None) | (None, Some((_, x))) => Some(x.clone()),
            (None, None) => None,
        }
    }

    /// Draws the waveform of `m` over `r`, a clip block, at `gain`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_wave(
        &mut self,
        painter: &egui::Painter,
        r: Rect,
        m: &Media,
        src: Source,
        t_of: impl Fn(f32) -> f64,
        gain: f32,
        color: Color32,
    ) {
        let a = asset(m);
        let k = key(&a).to_string();
        let wave = self.waves.entry(k).or_default();
        if !wave.done && !self.offline.contains(key(&a)) && !self.wanted_waves.iter().any(|(w, _)| key(w) == key(&a)) {
            self.wanted_waves.push((a.clone(), m.duration.unwrap_or(Tick::ZERO)));
        }
        let visible = painter.clip_rect().intersect(r);
        if wave.peaks.is_empty() || visible.width() <= 0.0 {
            return;
        }
        let mid = r.center().y;
        let half = r.height() * 0.42;
        let ppp = painter.ctx().pixels_per_point();
        let step = 1.0 / ppp;
        let mut x = visible.left();
        while x < visible.right() {
            let (s0, s1) = (src.at(t_of(x)), src.at(t_of(x + step)));
            // backwards, source time falls along the clip
            let (s0, s1) = (s0.min(s1), s0.max(s1));
            let (b0, b1) = (
                (s0 * PEAKS_PER_SECOND as f64).floor().max(0.0) as usize,
                ((s1 * PEAKS_PER_SECOND as f64).ceil().max(0.0) as usize)
                    .max((s0 * PEAKS_PER_SECOND as f64) as usize + 1),
            );
            if b0 >= wave.peaks.len() {
                break;
            }
            let peak = wave.peaks[b0..b1.min(wave.peaks.len())].iter().fold(0.0f32, |m, &p| m.max(p));
            let y = loudness(peak * gain) * half;
            if y > 0.25 {
                painter.line_segment([pos2(x, mid - y), pos2(x, mid + y)], Stroke::new(step, color));
            }
            x += step;
        }
    }

    /// Makes what was asked for, for up to the frame's budget (none while playing). Returns whether
    /// more is waiting.
    pub(crate) fn work(&mut self, ctx: &egui::Context, media: &Footage, playing: bool) -> bool {
        if playing {
            self.wanted.clear();
            self.wanted_waves.clear();
            return false;
        }
        if media.generation() != self.seen {
            self.seen = media.generation();
            self.offline.clear();
        }
        let begun = Instant::now();
        while begun.elapsed() < BUDGET {
            if let Some((a, at)) = self.wanted.pop_front() {
                self.make_picture(ctx, media, &a, at);
            } else if let Some((a, duration)) = self.wanted_waves.first().cloned() {
                if self.read_wave(media, &a, duration) {
                    self.wanted_waves.remove(0);
                }
            } else {
                break;
            }
        }
        let more = !self.wanted.is_empty() || !self.wanted_waves.is_empty();
        // asked again by the next frame's drawing
        self.wanted.clear();
        self.wanted_waves.clear();
        more
    }

    fn make_picture(&mut self, ctx: &egui::Context, media: &Footage, a: &Asset, at: Tick) {
        if self.count >= MAX_PICTURES {
            self.pictures.clear();
            self.count = 0;
        }
        let texture = match media.thumbnail(a, at, self.height.max(8)) {
            Ok(None) => {
                self.offline.insert(key(a).to_string());
                return;
            }
            made => made.ok().flatten(),
        };
        let texture = texture.map(|t| {
            let image = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
            ctx.load_texture(format!("strip-{}-{}", key(a), at.0), image, egui::TextureOptions::LINEAR)
        });
        self.pictures.entry(key(a).to_string()).or_default().insert(at.0, texture);
        self.count += 1;
    }

    /// Reads the next stretch of `a`'s sound into its waveform; true once it is complete.
    fn read_wave(&mut self, media: &Footage, a: &Asset, duration: Tick) -> bool {
        let wave = self.waves.entry(key(a).to_string()).or_default();
        let per_peak = WAVE_RATE as usize / PEAKS_PER_SECOND;
        let start = (wave.peaks.len() * per_peak) as i64;
        let total = (duration.0 as i128 * WAVE_RATE as i128 / TICKS_PER_SECOND as i128) as i64;
        if start >= total {
            wave.done = true;
            return true;
        }
        let frames = (WAVE_STEP_SECONDS * WAVE_RATE as usize).min((total - start) as usize);
        match media.audio(a, start, frames, WAVE_RATE) {
            Ok(Some(channels)) => {
                let n = channels.iter().map(Vec::len).min().unwrap_or(0).min(frames);
                for chunk in (0..n).step_by(per_peak) {
                    let end = (chunk + per_peak).min(n);
                    let peak = channels.iter().flat_map(|c| c[chunk..end].iter()).fold(0.0f32, |m, s| m.max(s.abs()));
                    wave.peaks.push(peak);
                }
                if n < frames {
                    wave.done = true;
                }
            }
            Ok(None) if wave.peaks.is_empty() => {
                // offline (or silent): asked again once files arrive
                self.offline.insert(key(a).to_string());
                return true;
            }
            // unreadable: nothing more to draw
            _ => wave.done = true,
        }
        wave.done
    }
}

/// How tall a peak is drawn, 0 to 1: on a decibel scale over the top [`RANGE_DB`], so quiet
/// passages still show.
fn loudness(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    (1.0 + 20.0 * peak.min(1.0).log10() / RANGE_DB).max(0.0)
}

/// The quietest sound a waveform shows, in dB below full scale.
const RANGE_DB: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_time_follows_the_clip() {
        let timing = Timing::of(Tick::SECOND * 2, 2.0, false, &[], Tick::SECOND * 4);
        let s = Source { timing, start: Tick::SECOND * 10 };
        assert_eq!(s.at(10.0), 2.0);
        assert_eq!(s.at(11.5), 5.0);
    }

    #[test]
    fn waveforms_are_drawn_in_decibels() {
        assert_eq!(loudness(1.0), 1.0);
        assert_eq!(loudness(2.0), 1.0);
        assert!((loudness(10f32.powf(-24.0 / 20.0)) - 0.5).abs() < 1e-5);
        assert_eq!(loudness(1e-4), 0.0);
        assert_eq!(loudness(0.0), 0.0);
    }
}
