//! Export: the settings dialog, and the job that writes the file. The RenderSpec is compiled on the
//! UI thread (a snapshot, so later edits do not change a running export).
//!
//! On the desktop a worker thread draws and encodes H.264 and writes an MP4 beside the project. In
//! the browser the job runs between frames of the page: frames are drawn on the CPU and encoded by
//! the browser's own encoder (WebCodecs, in hardware where there is one: H.264, HEVC or AV1, with
//! FilmCraft's H.264 when the browser has none), and the MP4 is downloaded.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use reelforge_model::export::FRAME_RATES;
use reelforge_model::{Codec, ExportSettings, Project, Quality, Resolution, export::PRESETS};
use serde_json::json;

use crate::i18n::fill;
use crate::{EditorApp, theme};

/// What a cancelled export reports.
const CANCELLED: &str = "cancelled";

/// An export in progress.
pub(crate) struct ExportJob {
    /// What the file is called.
    name: String,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    /// The outcome: what encoded, or why it stopped.
    result: Arc<Mutex<Option<Result<String, String>>>>,
}

impl ExportJob {
    fn new(name: String) -> ExportJob {
        ExportJob {
            name,
            done: Arc::default(),
            total: Arc::new(AtomicU64::new(1)),
            cancel: Arc::default(),
            result: Arc::default(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;

    use reelforge_compile::{CompileOptions, compile};
    use reelforge_media::{ExportError, Footage};
    use reelforge_model::Project;

    use super::{CANCELLED, ExportJob};

    /// The first `reelforge-N.mp4` in `dir` that does not exist yet.
    fn free_name(dir: &Path) -> PathBuf {
        (1..).map(|n| dir.join(format!("reelforge-{n}.mp4"))).find(|p| !p.exists()).expect("some name is free")
    }

    pub(super) fn start(p: &Project, dir: &Path) -> Result<ExportJob, String> {
        let spec = compile(p, &CompileOptions { encode: true }).map_err(|e| e.to_string())?;
        let missing = reelforge_render::unsupported_features(&spec);
        if !missing.is_empty() {
            return Err(format!("cannot export yet: {}", missing.join(", ")));
        }
        let out = free_name(dir);
        let job = ExportJob::new(out.display().to_string());
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
                let (_, stats) = reelforge_media::export(&spec, &Footage::new(), file, &mut |p| {
                    done.store(p.done, Ordering::Relaxed);
                    total.store(p.total, Ordering::Relaxed);
                    !cancel.load(Ordering::Relaxed)
                })
                .map_err(|e| match e {
                    ExportError::Cancelled => CANCELLED.to_string(),
                    e => e.to_string(),
                })?;
                std::fs::rename(&part, &out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
                Ok(format!("H.264 · {}", stats.renderer))
            })();
            if r.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            *result.lock().expect("result lock") = Some(r);
        });
        Ok(job)
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use eframe::egui;
    use reelforge_compile::{CompileOptions, compile};
    use reelforge_media::{
        ExportError, ExportWriter, Footage, SoftwareH264, VideoEncoder, Yuv420, draw_frame, encoded_size,
        video_encoding,
    };
    use reelforge_model::Project;
    use reelforge_render::Quality;
    use reelforge_render::gpu::GpuCompositor;
    use reelforge_render::plan::{PlannedSource, plan_frame};
    use reelforge_renderspec::{AssetKind, RenderSpec, VideoCodec};
    use wasm_bindgen::prelude::*;

    use super::{CANCELLED, ExportJob};
    use crate::webcodecs::{WebDecoder, WebEncoder, sleep};

    #[wasm_bindgen(inline_js = r#"
export function rf_save(name, bytes) {
  const url = URL.createObjectURL(new Blob([bytes], { type: 'video/mp4' }));
  const a = document.createElement('a');
  a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
"#)]
    extern "C" {
        fn rf_save(name: &str, bytes: &[u8]);
    }

    /// Exports in the background. `gpu` draws the frames when given (a sibling of the preview's
    /// compositor, on WebGPU); the CPU renderer draws them otherwise.
    pub(super) fn start(
        p: &Project,
        media: Arc<Footage>,
        ctx: egui::Context,
        mut gpu: Option<GpuCompositor>,
    ) -> Result<ExportJob, String> {
        let spec = compile(p, &CompileOptions { encode: true }).map_err(|e| e.to_string())?;
        let missing = reelforge_render::unsupported_features(&spec);
        if !missing.is_empty() {
            return Err(format!("cannot export yet: {}", missing.join(", ")));
        }
        let job = ExportJob::new("reelforge.mp4".into());
        let (done, total, cancel, result) =
            (job.done.clone(), job.total.clone(), job.cancel.clone(), job.result.clone());
        let name = job.name.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let r = async {
                let venc = video_encoding(&spec);
                let size = encoded_size(&spec);
                let fps = spec.output.fps;
                let kbps = venc.bitrate_kbps.unwrap_or(8000);
                let encoder: Box<dyn VideoEncoder> = match WebEncoder::open(venc.codec, size, fps, kbps).await {
                    Ok(e) => Box::new(e),
                    // without WebCodecs, H.264 still works in wasm, slowly
                    Err(_) if venc.codec == VideoCodec::H264 => {
                        Box::new(SoftwareH264::new(size[0], size[1], fps, &venc).map_err(|e| e.to_string())?)
                    }
                    Err(e) => return Err(e),
                };
                let mut w = ExportWriter::new(&spec, Cursor::new(Vec::new()), encoder).map_err(|e| e.to_string())?;
                let encoder = w.encoder();
                let n = w.frames();
                total.store(n, Ordering::Relaxed);
                let mut yuv = Yuv420::default();
                let step = |e: ExportError| e.to_string();
                let mut decoders = Decoders::default();
                for i in 0..n {
                    if cancel.load(Ordering::Relaxed) {
                        decoders.let_go(&spec, &media);
                        return Err(CANCELLED.to_string());
                    }
                    let t = spec.output.fps.tick_of(i as i64);
                    decoders.prepare(&spec, &media, t).await;
                    match &mut gpu {
                        Some(g) => {
                            let scale = video_encoding(&spec).scale(spec.output.width);
                            g.render(&spec, t, scale, &*media, Quality::Final).map_err(|e| e.to_string())?;
                            let back = g.start_yuv420().map_err(|e| e.to_string())?;
                            // the browser maps the copy when the GPU is done; give it the turn
                            while !back.ready() {
                                sleep(0).await;
                            }
                            yuv.width = back.size()[0] as usize;
                            back.finish(&mut yuv.y, &mut yuv.u, &mut yuv.v).map_err(|e| e.to_string())?;
                        }
                        None => draw_frame(&spec, &media, i, &mut yuv).map_err(step)?,
                    }
                    decoders.let_go(&spec, &media);
                    w.push(&spec, &media, i, &yuv).map_err(step)?;
                    // let the encoder catch up, and the page draw
                    while w.backlog() > 3 {
                        sleep(4).await;
                        w.poll(&spec, &media).map_err(step)?;
                    }
                    // the progress bar repaints on its own, ten times a second: a repaint per frame
                    // would cost as much as the frame
                    done.store(i + 1, Ordering::Relaxed);
                    sleep(0).await;
                }
                w.end().map_err(step)?;
                while w.backlog() > 0 {
                    sleep(10).await;
                }
                let (out, _) = w.finish(&spec, &media).map_err(step)?;
                rf_save(&name, &out.into_inner());
                let mut how = encoder;
                if gpu.is_some() {
                    how.push_str(" · GPU");
                }
                if decoders.used {
                    how.push_str(" · WebCodecs");
                }
                Ok(how)
            }
            .await;
            media.clear_prepared();
            *result.lock().expect("result lock") = Some(r);
            ctx.request_repaint();
        });
        Ok(job)
    }

    /// The browser's decoders, one per clip on screen, each handing the renderer its pictures for
    /// the frame about to be drawn. Footage they cannot decode is left to the software decoder.
    #[derive(Default)]
    struct Decoders {
        /// By layer; `None` once the browser could not take the clip on.
        open: HashMap<String, Option<WebDecoder>>,
        /// Assets with pictures handed over for this frame.
        prepared: Vec<String>,
        /// Whether any picture came from the browser.
        used: bool,
    }

    impl Decoders {
        async fn prepare(&mut self, spec: &RenderSpec, media: &Footage, t: reelforge_time::Tick) {
            let plan = plan_frame(spec, t, 1.0);
            // clips that ended give their decoders back
            self.open.retain(|id, _| plan.layers.iter().any(|l| &l.id == id));
            for layer in &plan.layers {
                let PlannedSource::Media { asset, at } = &layer.source else { continue };
                let Some(def) = spec.assets.get(asset).filter(|d| d.kind == AssetKind::Video) else { continue };
                if !self.open.contains_key(&layer.id) {
                    let dec = match media.video_bytes(def) {
                        Some(bytes) => WebDecoder::open(bytes).await,
                        None => None,
                    };
                    self.open.insert(layer.id.clone(), dec);
                }
                let Some(Some(dec)) = self.open.get_mut(&layer.id) else { continue };
                if let Some(picture) = dec.picture(*at).await {
                    media.prepare(def, *at, picture);
                    self.prepared.push(asset.clone());
                    self.used = true;
                }
            }
        }

        /// Drops this frame's pictures.
        fn let_go(&mut self, spec: &RenderSpec, media: &Footage) {
            for asset in self.prepared.drain(..) {
                if let Some(def) = spec.assets.get(&asset) {
                    media.unprepare(def);
                }
            }
        }
    }
}

/// A short name for a resolution.
fn resolution_name(r: Resolution, canvas: &'static str) -> &'static str {
    match r {
        Resolution::Canvas => canvas,
        Resolution::P720 => "720p",
        Resolution::P1080 => "1080p",
        Resolution::P1440 => "2K",
        Resolution::P2160 => "4K",
    }
}

fn codec_name(c: Codec) -> &'static str {
    match c {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC (H.265)",
        Codec::Av1 => "AV1",
    }
}

/// Whether this build encodes `c`.
fn encodes(c: Codec) -> bool {
    cfg!(target_arch = "wasm32") || c == Codec::H264
}

impl EditorApp {
    /// The Export button, or the progress of a running export.
    pub(crate) fn export_controls(&mut self, ui: &mut egui::Ui, p: &Project) {
        if let Some(job) = &self.export {
            let finished = job.result.lock().expect("result lock").take();
            match finished {
                Some(Ok(how)) => {
                    self.status = format!("{} · {how}", fill(self.s().exported, &[&job.name]));
                    self.export = None;
                }
                Some(Err(e)) if e == CANCELLED => {
                    self.status = self.s().export_cancelled.to_string();
                    self.export = None;
                }
                Some(Err(e)) => {
                    self.status = fill(self.s().export_failed, &[&e]);
                    self.export = None;
                }
                None => {
                    let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed).max(1));
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total as f32)
                            .desired_width(140.0)
                            .text(fill(self.s().exporting, &[&done, &total])),
                    );
                    if ui.button(self.s().cancel).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    return;
                }
            }
        }
        let can = !p.tracks.iter().all(|t| t.clips.is_empty());
        #[cfg(not(target_arch = "wasm32"))]
        let tip = fill(self.s().export_tip, &[&self.export_dir.display()]);
        #[cfg(target_arch = "wasm32")]
        let tip = self.s().export_tip_web.to_string();
        if theme::primary(ui, theme::icon::EXPORT, self.s().export, can).on_hover_text(tip).clicked() {
            self.export_open = !self.export_open;
        }
    }

    /// The export settings, while open.
    pub(crate) fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_open {
            return;
        }
        let p = self.project();
        let s = self.s();
        let e = p.export;
        let c = &p.canvas;
        let mut set: Option<serde_json::Value> = None;
        let mut go = false;
        let mut open = true;
        egui::Window::new(s.export_title)
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, (id, preset)) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(e == *preset, s.export_presets[i]).clicked() {
                            set = Some(json!({"preset": id}));
                        }
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("export settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    let weak = |t: &str| egui::RichText::new(t.to_string()).color(theme::color::WEAK);
                    ui.label(weak(s.export_resolution));
                    let [w, h] = e.size(c);
                    egui::ComboBox::from_id_salt("export resolution")
                        .selected_text(format!("{}  ·  {w}×{h}", resolution_name(e.resolution, s.export_as_canvas)))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for r in Resolution::ALL {
                                let [w, h] = ExportSettings { resolution: r, ..e }.size(c);
                                let label = format!("{}  ·  {w}×{h}", resolution_name(r, s.export_as_canvas));
                                if ui.selectable_label(e.resolution == r, label).clicked() {
                                    set = Some(json!({"resolution": r.name()}));
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(weak(s.export_fps));
                    let fps_name = |f: u32| if f == 0 { s.export_as_canvas.to_string() } else { format!("{f} fps") };
                    egui::ComboBox::from_id_salt("export fps").selected_text(fps_name(e.fps)).width(170.0).show_ui(
                        ui,
                        |ui| {
                            for f in std::iter::once(0).chain(FRAME_RATES) {
                                if ui.selectable_label(e.fps == f, fps_name(f)).clicked() {
                                    set = Some(json!({"fps": f}));
                                }
                            }
                        },
                    );
                    ui.end_row();
                    ui.label(weak(s.export_quality));
                    let q_name =
                        |q: Quality| s.export_qualities[Quality::ALL.iter().position(|x| *x == q).unwrap_or(1)];
                    egui::ComboBox::from_id_salt("export quality")
                        .selected_text(q_name(e.quality))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for q in Quality::ALL {
                                if ui.selectable_label(e.quality == q, q_name(q)).clicked() {
                                    set = Some(json!({"quality": q.name()}));
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(weak(s.export_codec));
                    egui::ComboBox::from_id_salt("export codec")
                        .selected_text(codec_name(e.codec))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for k in Codec::ALL {
                                let r = ui
                                    .add_enabled(encodes(k), egui::Button::selectable(e.codec == k, codec_name(k)))
                                    .on_disabled_hover_text(s.export_codec_web_only);
                                if r.clicked() {
                                    set = Some(json!({"codec": k.name()}));
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(weak(s.export_estimate));
                    let mb = e.bitrate_kbps(c) as f64 * p.duration().seconds() / 8.0 / 1000.0;
                    ui.label(format!(
                        "{} · {:.1} Mbps",
                        fill(s.export_megabytes, &[&format!("{mb:.1}")]),
                        e.bitrate_kbps(c) as f64 / 1000.0
                    ));
                    ui.end_row();
                });
                if !encodes(e.codec) {
                    ui.label(egui::RichText::new(s.export_codec_web_only).small().color(theme::color::WARN));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can = encodes(e.codec) && !p.tracks.iter().all(|t| t.clips.is_empty());
                    if theme::primary(ui, theme::icon::EXPORT, s.export, can).clicked() {
                        go = true;
                    }
                });
            });
        if let Some(v) = set {
            let _ = self.execute("project.setExport", v);
        }
        self.export_open = open && !go;
        if go {
            #[cfg(not(target_arch = "wasm32"))]
            let started = native::start(&p, &self.export_dir);
            // the preview's device draws the frames where it can convert them to 4:2:0 (WebGPU)
            #[cfg(target_arch = "wasm32")]
            let started = web::start(
                &p,
                self.media.clone(),
                ctx.clone(),
                self.gpu.as_ref().map(|g| g.compositor.sibling()).filter(|g| g.converts_yuv()),
            );
            match started {
                Ok(job) => self.export = Some(job),
                Err(e) => self.status = fill(self.s().export_failed, &[&e]),
            }
        }
    }
}
