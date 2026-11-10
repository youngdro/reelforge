//! A playing preview reads ahead: a worker thread decodes the frames the playhead reaches next and
//! makes their pictures ([`Footage::prefetch`]), so the editor's thread only composites. The
//! nearest frames get pictures; a few further ones are only decoded, which fills the decoder's own
//! cache. Each request replaces the last, so the worker follows the playhead and never runs behind
//! it after a seek.
//!
//! When the preview is drawn on the GPU, which converts video pictures itself, nothing is made into
//! pictures ahead: the frames are only decoded ([`Ahead::pictures`]).
//!
//! In the browser there are no threads to spare: there, [`Ahead`] does nothing and the preview
//! decodes as it draws, as before.

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};

use reelforge_media::Footage;
use reelforge_renderspec::RenderSpec;
use reelforge_time::Tick;

/// Frames past the playhead that get pictures made.
#[cfg(not(target_arch = "wasm32"))]
const PICTURES: i64 = reelforge_media::AHEAD_PICTURES as i64 - 1;
/// Frames past the playhead that are decoded (the first [`PICTURES`] of them made into pictures).
#[cfg(not(target_arch = "wasm32"))]
const DECODED: i64 = 12;

/// What to read ahead: the spec, the frame shown, the scale the preview draws at.
#[cfg(not(target_arch = "wasm32"))]
type Job = (Arc<RenderSpec>, Tick, f32);

/// The reading-ahead worker.
pub(crate) struct Ahead {
    #[cfg(not(target_arch = "wasm32"))]
    tx: std::sync::mpsc::Sender<Job>,
    #[cfg(not(target_arch = "wasm32"))]
    pictures: Arc<AtomicBool>,
}

impl Ahead {
    /// Starts the worker for `media`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(media: Arc<Footage>) -> Ahead {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let pictures = Arc::new(AtomicBool::new(true));
        let make = pictures.clone();
        let _ = std::thread::Builder::new().name("preview-ahead".into()).spawn(move || {
            let mut next = None;
            loop {
                let job = match next.take() {
                    Some(job) => job,
                    None => match rx.recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    },
                };
                // only the newest request matters
                let (spec, from, scale) = rx.try_iter().last().unwrap_or(job);
                next = read_ahead(&spec, from, scale, &media, make.load(Ordering::Relaxed), &rx);
            }
        });
        Ahead { tx, pictures }
    }

    /// Whether frames ahead are made into pictures (the CPU preview), or only decoded (the GPU
    /// preview converts them itself).
    pub(crate) fn pictures(&self, make: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pictures.store(make, Ordering::Relaxed);
        #[cfg(target_arch = "wasm32")]
        let _ = make;
    }

    /// In the browser: nothing.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(_: Arc<Footage>) -> Ahead {
        Ahead {}
    }

    /// The preview is playing `spec`, drawn `scale` times its size, and shows the frame at `at`:
    /// read ahead from there.
    pub(crate) fn follow(&self, spec: &Arc<RenderSpec>, at: Tick, scale: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.tx.send((spec.clone(), at, scale));
        #[cfg(target_arch = "wasm32")]
        let _ = (spec, at, scale);
    }
}

/// Decodes the frames after `from`; stops early, returning it, when a newer request arrives.
#[cfg(not(target_arch = "wasm32"))]
fn read_ahead(
    spec: &RenderSpec,
    from: Tick,
    scale: f32,
    media: &Footage,
    pictures: bool,
    rx: &std::sync::mpsc::Receiver<Job>,
) -> Option<Job> {
    let fps = spec.output.fps;
    let first = fps.frame_at(fps.snap(from));
    for step in 1..=DECODED {
        let t = fps.tick_of(first + step);
        if t >= spec.output.duration {
            break;
        }
        for (asset, at, shrink, between, cut) in sources_at(spec, t, scale) {
            let Some(def) = spec.assets.get(&asset) else { continue };
            if let Some(fx) = cut.filter(|_| step <= PICTURES) {
                // the picture a compositor will ask for, and what is cut out of it (its people,
                // or a thing off its backdrop): both are kept (by the footage, by the renderer)
                // for when it does. The GPU preview needs this too, as a cut-out is made from a
                // whole picture before it gets there
                use reelforge_render::MediaSource;
                let picture = if !between.is_nearest() {
                    media.picture_between(&asset, def, at, between, shrink)
                } else if shrink > 1 {
                    media.picture_shrunk(&asset, def, at, shrink)
                } else {
                    media.picture(&asset, def, at)
                };
                if let Ok(Some(picture)) = picture {
                    let _ = reelforge_render::cutout::of(&fx, &picture);
                }
            } else if pictures && step <= PICTURES && !between.is_nearest() {
                // a picture made between two frames is kept by the footage for when it is asked for
                let _ = reelforge_render::MediaSource::picture_between(media, &asset, def, at, between, shrink);
            } else if pictures && step <= PICTURES {
                media.prefetch(def, at, shrink);
            } else {
                media.warm(def, at);
            }
        }
        if let Ok(job) = rx.try_recv() {
            return Some(rx.try_iter().last().unwrap_or(job));
        }
    }
    None
}

/// A picture a frame shows, as [`sources_at`] lists it.
#[cfg(not(target_arch = "wasm32"))]
type Source = (String, Tick, u32, reelforge_renderspec::Between, Option<Vec<(String, Vec<(String, f32)>)>>);

/// The pictures of assets a frame at `t` drawn `scale` times its size shows: `(asset id, source
/// instant, how many times smaller the draft asks for it, what shows between its frames, the
/// layer's effects if they cut the picture out)`.
#[cfg(not(target_arch = "wasm32"))]
fn sources_at(spec: &RenderSpec, t: Tick, scale: f32) -> Vec<Source> {
    let plan = reelforge_render::plan_frame(spec, t, scale);
    // the ones a compound clip shows are drawn into a frame of their own
    plan.media_layers()
        .into_iter()
        .filter_map(|(frame, l)| match &l.source {
            reelforge_render::PlannedSource::Media { asset, at, between } => {
                let shrink = spec.assets.get(asset).map_or(1, |a| reelforge_render::cpu::draft_shrink(a, frame, l));
                let cut = reelforge_render::cutout::cuts(&l.fx).then(|| l.fx.clone());
                Some((asset.clone(), *at, shrink, *between, cut))
            }
            _ => None,
        })
        .collect()
}
