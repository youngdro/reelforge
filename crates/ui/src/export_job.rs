//! Export from the desktop editor: the RenderSpec is compiled on the UI thread (it is a snapshot,
//! so later edits do not change a running export), then drawn and encoded on a worker thread
//! that reports progress and stops when asked.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use reelforge_compile::{CompileOptions, compile};
use reelforge_media::{ExportError, Footage};
use reelforge_model::Project;

use crate::EditorApp;

/// An export in progress.
pub(crate) struct ExportJob {
    out: PathBuf,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}

/// The first `reelforge-N.mp4` in `dir` that does not exist yet.
fn free_name(dir: &Path) -> PathBuf {
    (1..).map(|n| dir.join(format!("reelforge-{n}.mp4"))).find(|p| !p.exists()).expect("some name is free")
}

impl ExportJob {
    fn start(p: &Project, dir: &Path) -> Result<ExportJob, String> {
        let spec = compile(p, &CompileOptions { encode: true }).map_err(|e| e.to_string())?;
        let missing = reelforge_render::unsupported_features(&spec);
        if !missing.is_empty() {
            return Err(format!("cannot export yet: {}", missing.join(", ")));
        }
        let out = free_name(dir);
        let job = ExportJob {
            out: out.clone(),
            done: Arc::default(),
            total: Arc::new(AtomicU64::new(1)),
            cancel: Arc::default(),
            result: Arc::default(),
        };
        let (done, total, cancel, result) =
            (job.done.clone(), job.total.clone(), job.cancel.clone(), job.result.clone());
        std::thread::spawn(move || {
            let part = out.with_extension("mp4.part");
            let r = (|| {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&part)
                    .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
                reelforge_media::export(&spec, &Footage::new(), file, &mut |p| {
                    done.store(p.done, Ordering::Relaxed);
                    total.store(p.total, Ordering::Relaxed);
                    !cancel.load(Ordering::Relaxed)
                })
                .map_err(|e| match e {
                    ExportError::Cancelled => "cancelled".to_string(),
                    e => e.to_string(),
                })?;
                std::fs::rename(&part, &out).map_err(|e| format!("cannot write {}: {e}", out.display()))
            })();
            if r.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            *result.lock().expect("result lock") = Some(r);
        });
        Ok(job)
    }
}

impl EditorApp {
    /// The Export button, or the progress of a running export.
    pub(crate) fn export_controls(&mut self, ui: &mut egui::Ui, p: &Project) {
        if let Some(job) = &self.export {
            let finished = job.result.lock().expect("result lock").take();
            match finished {
                Some(Ok(())) => {
                    self.status = format!("Exported {}", job.out.display());
                    self.export = None;
                }
                Some(Err(e)) => {
                    self.status = format!("export: {e}");
                    self.export = None;
                }
                None => {
                    let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed).max(1));
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total as f32)
                            .desired_width(140.0)
                            .text(format!("Exporting {done}/{total}")),
                    );
                    if ui.button("Cancel").clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    return;
                }
            }
        }
        let can = !p.tracks.iter().all(|t| t.clips.is_empty());
        if ui
            .add_enabled(can, egui::Button::new("⬇ Export MP4"))
            .on_hover_text(self.export_dir.display().to_string())
            .clicked()
        {
            match ExportJob::start(p, &self.export_dir) {
                Ok(job) => self.export = Some(job),
                Err(e) => self.status = format!("export: {e}"),
            }
        }
    }
}
