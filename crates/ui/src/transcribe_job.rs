//! Automatic captions: the timeline's sound is heard by a local Whisper model and what is said is
//! placed as captions (`captions.add`). The desktop editor listens itself, on a worker thread; an
//! editor joined to a server (the browser, or a desktop joined to one) asks the server to, since
//! the server holds the media and the model.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use serde_json::{Value, json};

use crate::EditorApp;
use crate::i18n::fill;

/// How a job ended: `captions.add` parameters to run here, or the server's answer (it placed them).
enum Heard {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Here(Value, usize),
    There(Value),
}

/// A transcription in progress.
pub(crate) struct TranscribeJob {
    /// Share done, as `f32` bits; unknown (a server is listening) when it stays 0.
    progress: Arc<AtomicU32>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Result<Heard, String>>>>,
    /// The work runs on a server: it cannot be stopped from here.
    remote: bool,
}

impl TranscribeJob {
    fn new(remote: bool) -> TranscribeJob {
        TranscribeJob { progress: Arc::default(), cancel: Arc::default(), result: Arc::default(), remote }
    }

    /// Asks the server to listen.
    fn on_server(remote: &crate::remote::Remote, ctx: &egui::Context) -> TranscribeJob {
        let job = TranscribeJob::new(true);
        let (result, ctx) = (job.result.clone(), ctx.clone());
        remote.call("captions.transcribe", &json!({}), move |r| {
            *result.lock().expect("result lock") = Some(r.map(Heard::There));
            ctx.request_repaint();
        });
        job
    }

    /// Listens here, on a worker thread.
    #[cfg(not(target_arch = "wasm32"))]
    fn here(app: &mut EditorApp, ctx: &egui::Context) -> Result<TranscribeJob, String> {
        let p = app.project();
        let spec = reelforge_compile::compile(&p, &reelforge_compile::CompileOptions { encode: false })
            .map_err(|e| e.to_string())?;
        let speech = app
            .speech
            .get_or_insert_with(|| Arc::new(reelforge_speech::Recogniser::new(reelforge_speech::default_model_dir())))
            .clone();
        speech.check()?;
        let job = TranscribeJob::new(false);
        let (progress, cancel, result, media, ctx) =
            (job.progress.clone(), job.cancel.clone(), job.result.clone(), app.media.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = speech
                .transcribe(&spec, &media, (None, None), &Default::default(), &mut |f| {
                    progress.store(f.to_bits(), Ordering::Relaxed);
                    ctx.request_repaint();
                    !cancel.load(Ordering::Relaxed)
                })
                .map(|t| Heard::Here(reelforge_speech::captions_params(&t.segments, None, None), t.segments.len()));
            *result.lock().expect("result lock") = Some(r);
            ctx.request_repaint();
        });
        Ok(job)
    }
}

impl EditorApp {
    /// The Auto captions button, or the progress of a running transcription.
    pub(crate) fn transcribe_controls(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        if let Some(job) = &self.transcribe {
            let finished = job.result.lock().expect("result lock").take();
            match finished {
                None => {
                    let f = f32::from_bits(job.progress.load(Ordering::Relaxed));
                    let bar = if job.remote {
                        egui::ProgressBar::new(0.0).animate(true).text(s.listening_remote)
                    } else {
                        egui::ProgressBar::new(f).text(fill(s.listening, &[&((f * 100.0).round() as u32)]))
                    };
                    ui.add(bar.desired_width(150.0));
                    #[cfg(not(target_arch = "wasm32"))]
                    if !job.remote && ui.button(s.cancel).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                    return;
                }
                Some(r) => {
                    self.transcribe = None;
                    self.status = match r {
                        Ok(Heard::Here(_, 0)) => s.no_speech_heard.to_string(),
                        Ok(Heard::Here(params, n)) => match self.execute("captions.add", params) {
                            Ok(_) => fill(s.captions_heard, &[&n]),
                            Err(e) => fill(s.transcribe_failed, &[&e]),
                        },
                        Ok(Heard::There(v)) => match v["count"].as_u64() {
                            Some(0) | None => s.no_speech_heard.to_string(),
                            Some(n) => fill(s.captions_heard, &[&n]),
                        },
                        Err(e) if e == "stopped" => s.transcribe_stopped.to_string(),
                        Err(e) => fill(s.transcribe_failed, &[&e]),
                    };
                }
            }
        }
        let has_sound = self.project().tracks.iter().any(|t| {
            !t.muted && t.clips.iter().any(|c| matches!(c.content, reelforge_model::ClipContent::Media { .. }))
        });
        if crate::theme::tool(ui, crate::theme::icon::CAPTIONS, s.auto_captions, s.auto_captions_tip, has_sound)
            .clicked()
        {
            let ctx = ui.ctx().clone();
            if let Some(remote) = &self.remote {
                self.transcribe = Some(TranscribeJob::on_server(remote, &ctx));
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            match TranscribeJob::here(self, &ctx) {
                Ok(job) => self.transcribe = Some(job),
                Err(e) => self.status = fill(s.transcribe_failed, &[&e]),
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.status = s.transcribe_needs_server.to_string();
            }
        }
    }
}
