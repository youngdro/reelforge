//! Voice-over: titles read aloud by a local Kokoro model, each spoken line placed where its title
//! starts (`voiceover.add`). The desktop editor speaks itself, on a worker thread; an editor joined
//! to a server (the browser, or a desktop joined to one) asks the server to (`voice.speak`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use reelforge_model::ClipContent;
use serde_json::{Value, json};

use crate::EditorApp;
use crate::i18n::fill;

/// How a job ended: `voiceover.add` parameters to run here and the number of lines, or the
/// server's answer (it placed them).
enum Spoken {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Here(Value, usize),
    There(Value),
}

/// A voice-over in progress.
pub(crate) struct VoiceJob {
    /// Lines read so far, and how many there are (0 while a server is reading).
    done: Arc<AtomicU32>,
    total: u32,
    result: Arc<Mutex<Option<Result<Spoken, String>>>>,
}

impl VoiceJob {
    /// Asks the server to read `clips` (the captions when empty).
    fn on_server(remote: &crate::remote::Remote, clips: Vec<String>, ctx: &egui::Context) -> VoiceJob {
        let job = VoiceJob { done: Arc::default(), total: 0, result: Arc::default() };
        let (result, ctx) = (job.result.clone(), ctx.clone());
        let params = if clips.is_empty() { json!({}) } else { json!({ "clips": clips }) };
        remote.call("voice.speak", &params, move |r| {
            *result.lock().expect("result lock") = Some(r.map(Spoken::There));
            ctx.request_repaint();
        });
        job
    }

    /// Reads here, on a worker thread, writing the lines beside the model's other voice-overs.
    #[cfg(not(target_arch = "wasm32"))]
    fn here(app: &mut EditorApp, clips: Vec<String>, ctx: &egui::Context) -> Result<VoiceJob, String> {
        let lines = reelforge_engine::spoken_lines(&app.project(), &clips)?;
        let dir = reelforge_voice::default_model_dir();
        reelforge_voice::check(&dir)?;
        let model = app.voice.clone();
        let job = VoiceJob { done: Arc::default(), total: lines.len() as u32, result: Arc::default() };
        let (done, result, ctx) = (job.done.clone(), job.result.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| {
                let mut model = model.lock().expect("voice lock");
                if model.is_none() {
                    *model = Some(reelforge_voice::Kokoro::load(&dir)?);
                }
                let model = model.as_mut().expect("loaded");
                let out_dir = reelforge_voice::voiceover_dir();
                std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
                let mut spoken = Vec::new();
                for line in &lines {
                    let samples = model.speak(&line.text, reelforge_voice::DEFAULT_VOICE, 1.0)?;
                    let bytes = reelforge_voice::wav(&samples, reelforge_voice::SAMPLE_RATE);
                    use sha2::{Digest, Sha256};
                    let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                    let path = out_dir.join(format!("{}.wav", &sha[..16]));
                    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                    let name: String = line.text.chars().take(24).collect();
                    spoken.push(json!({"path": path, "start": line.start.0, "name": name, "sha256": sha}));
                    done.fetch_add(1, Ordering::Relaxed);
                    ctx.request_repaint();
                }
                Ok(Spoken::Here(json!({ "lines": spoken }), lines.len()))
            })();
            *result.lock().expect("result lock") = Some(r);
            ctx.request_repaint();
        });
        Ok(job)
    }
}

impl EditorApp {
    /// The Voice-over button, or the progress of a running voice-over.
    pub(crate) fn voice_controls(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        if let Some(job) = &self.voice_job {
            let finished = job.result.lock().expect("result lock").take();
            match finished {
                None => {
                    let bar = if job.total == 0 {
                        egui::ProgressBar::new(0.0).animate(true).text(s.speaking_remote)
                    } else {
                        let n = job.done.load(Ordering::Relaxed);
                        egui::ProgressBar::new(n as f32 / job.total as f32).text(fill(s.speaking, &[&n, &job.total]))
                    };
                    ui.add(bar.desired_width(150.0));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                    return;
                }
                Some(r) => {
                    self.voice_job = None;
                    self.status = match r {
                        Ok(Spoken::Here(params, n)) => match self.execute("voiceover.add", params) {
                            Ok(_) => fill(s.voiced, &[&n]),
                            Err(e) => fill(s.voice_failed, &[&e]),
                        },
                        Ok(Spoken::There(v)) => fill(s.voiced, &[&v["count"].as_u64().unwrap_or(0)]),
                        Err(e) => fill(s.voice_failed, &[&e]),
                    };
                }
            }
        }
        // the titles picked, or with none, the captions
        let p = self.project();
        let picked: Vec<String> = self
            .selection()
            .into_iter()
            .filter(|id| p.clip(id).is_some_and(|(_, c)| matches!(c.content, ClipContent::Text { .. })))
            .collect();
        let captions = p
            .tracks
            .iter()
            .any(|t| t.name == reelforge_engine::CAPTIONS_TRACK && t.kind == reelforge_model::TrackKind::Text);
        drop(p);
        let tip = if picked.is_empty() { s.voiceover_tip } else { s.voiceover_picked_tip };
        if crate::theme::tool(ui, crate::theme::icon::SOUND, s.voiceover, tip, !picked.is_empty() || captions).clicked()
        {
            let ctx = ui.ctx().clone();
            if let Some(remote) = &self.remote {
                self.voice_job = Some(VoiceJob::on_server(remote, picked, &ctx));
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            match VoiceJob::here(self, picked, &ctx) {
                Ok(job) => self.voice_job = Some(job),
                Err(e) => self.status = fill(s.voice_failed, &[&e]),
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.status = s.voice_needs_server.to_string();
            }
        }
    }
}
