//! A playing preview reads ahead: a worker thread decodes the frames the playhead reaches next and
//! makes their pictures ([`Footage::prefetch`]), so the editor's thread only composites. The
//! nearest frames get pictures; a few further ones are only decoded, which fills the decoder's own
//! cache. Each request replaces the last, so the worker follows the playhead and never runs behind
//! it after a seek.
//!
//! In the browser there are no threads to spare: there, [`Ahead`] does nothing and the preview
//! decodes as it draws, as before.

use std::sync::Arc;

use reelforge_media::Footage;
use reelforge_renderspec::RenderSpec;
use reelforge_time::Tick;

/// Frames past the playhead that get pictures made.
#[cfg(not(target_arch = "wasm32"))]
const PICTURES: i64 = reelforge_media::AHEAD_PICTURES as i64 - 1;
/// Frames past the playhead that are decoded (the first [`PICTURES`] of them made into pictures).
#[cfg(not(target_arch = "wasm32"))]
const DECODED: i64 = 12;

/// The reading-ahead worker.
pub(crate) struct Ahead {
    #[cfg(not(target_arch = "wasm32"))]
    tx: std::sync::mpsc::Sender<(Arc<RenderSpec>, Tick)>,
}

impl Ahead {
    /// Starts the worker for `media`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(media: Arc<Footage>) -> Ahead {
        let (tx, rx) = std::sync::mpsc::channel::<(Arc<RenderSpec>, Tick)>();
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
                let (spec, from) = rx.try_iter().last().unwrap_or(job);
                next = read_ahead(&spec, from, &media, &rx);
            }
        });
        Ahead { tx }
    }

    /// In the browser: nothing.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(_: Arc<Footage>) -> Ahead {
        Ahead {}
    }

    /// The preview is playing `spec` and shows the frame at `at`: read ahead from there.
    pub(crate) fn follow(&self, spec: &Arc<RenderSpec>, at: Tick) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.tx.send((spec.clone(), at));
        #[cfg(target_arch = "wasm32")]
        let _ = (spec, at);
    }
}

/// Decodes the frames after `from`; stops early, returning it, when a newer request arrives.
#[cfg(not(target_arch = "wasm32"))]
fn read_ahead(
    spec: &RenderSpec,
    from: Tick,
    media: &Footage,
    rx: &std::sync::mpsc::Receiver<(Arc<RenderSpec>, Tick)>,
) -> Option<(Arc<RenderSpec>, Tick)> {
    let fps = spec.output.fps;
    let first = fps.frame_at(fps.snap(from));
    for step in 1..=DECODED {
        let t = fps.tick_of(first + step);
        if t >= spec.output.duration {
            break;
        }
        for (asset, at) in sources_at(spec, t) {
            let Some(def) = spec.assets.get(&asset) else { continue };
            if step <= PICTURES {
                media.prefetch(def, at);
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

/// The pictures of assets a frame at `t` shows: `(asset id, source instant)`.
#[cfg(not(target_arch = "wasm32"))]
fn sources_at(spec: &RenderSpec, t: Tick) -> Vec<(String, Tick)> {
    reelforge_render::plan_frame(spec, t, 1.0)
        .layers
        .into_iter()
        .filter_map(|l| match l.source {
            reelforge_render::PlannedSource::Media { asset, at } => Some((asset, at)),
            _ => None,
        })
        .collect()
}
