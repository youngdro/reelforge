//! Export: the settings dialog, and the job that writes the file. The RenderSpec is compiled on the
//! UI thread (a snapshot, so later edits do not change a running export).
//!
//! On the desktop a worker thread draws and encodes H.264 and writes an MP4 beside the project. In
//! the browser the job runs between frames of the page: frames are drawn on the CPU and encoded by
//! the browser's own encoder (WebCodecs, in hardware where there is one: H.264, HEVC or AV1, with
//! FilmCraft's H.264 when the browser has none), and the MP4 is downloaded.
//!
//! The same job writes the other kinds of file the export settings can ask for (an animated GIF,
//! the sound alone as M4A or WAV), a video's cover beside it, and the frame under the playhead as
//! a PNG.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use reelforge_model::export::FRAME_RATES;
use reelforge_model::export::Format;
use reelforge_model::{Codec, ExportSettings, Project, Quality, Resolution, export::PRESETS};
use reelforge_time::Tick;
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

/// What an export writes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum What {
    /// The timeline, as the project's export settings say.
    Timeline,
    /// The frame at this instant, as a PNG.
    Frame(Tick),
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;

    use reelforge_compile::{CompileOptions, compile};
    use reelforge_media::{ExportError, Footage, StillFormat, encode_still, still};
    use reelforge_model::Project;
    use reelforge_renderspec::RenderSpec;
    use reelforge_time::Tick;

    use super::{CANCELLED, ExportJob, What};

    /// The first `reelforge-N.<ext>` in `dir` that does not exist yet.
    fn free_name(dir: &Path, ext: &str) -> PathBuf {
        (1..).map(|n| dir.join(format!("reelforge-{n}.{ext}"))).find(|p| !p.exists()).expect("some name is free")
    }

    /// `path` with `.part` after its name: where a file is written until it is whole.
    fn part_of(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        path.with_file_name(name)
    }

    /// Writes the frame of `spec` at `t` to `path`.
    fn write_still(spec: &RenderSpec, t: Tick, format: StillFormat, path: &Path) -> Result<[u32; 2], String> {
        let img = still(spec, &Footage::new(), t, 1.0).map_err(|e| e.to_string())?;
        let bytes = encode_still(&img, format).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok([img.width, img.height])
    }

    fn supported(spec: &RenderSpec) -> Result<(), String> {
        let missing = reelforge_render::unsupported_features(spec);
        if missing.is_empty() { Ok(()) } else { Err(format!("cannot export yet: {}", missing.join(", "))) }
    }

    pub(super) fn start(p: &Project, dir: &Path, what: What) -> Result<ExportJob, String> {
        // the timeline as it is drawn, without encode settings: for a frame, and for the cover
        let frames = || compile(p, &CompileOptions { encode: false }).map_err(|e| e.to_string());
        if let What::Frame(t) = what {
            let spec = frames()?;
            supported(&spec)?;
            let out = free_name(dir, "png");
            let job = ExportJob::new(out.display().to_string());
            let (done, result) = (job.done.clone(), job.result.clone());
            std::thread::spawn(move || {
                let r = write_still(&spec, t, StillFormat::Png, &out).map(|[w, h]| format!("{w}×{h}"));
                done.store(1, Ordering::Relaxed);
                *result.lock().expect("result lock") = Some(r);
            });
            return Ok(job);
        }
        let spec = compile(p, &CompileOptions { encode: true }).map_err(|e| e.to_string())?;
        supported(&spec)?;
        // a video's cover goes beside it
        let cover = match p.cover.filter(|_| p.export.format.pictured()) {
            Some(at) => Some((frames()?, at)),
            None => None,
        };
        let out = free_name(dir, spec.output.format.extension());
        let job = ExportJob::new(out.display().to_string());
        let (done, total, cancel, result) =
            (job.done.clone(), job.total.clone(), job.cancel.clone(), job.result.clone());
        std::thread::spawn(move || {
            let part = part_of(&out);
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
                if let Some((frames, at)) = &cover {
                    write_still(frames, *at, StillFormat::Jpeg, &out.with_extension("cover.jpg"))?;
                }
                Ok(if stats.renderer.is_empty() {
                    stats.encoder
                } else {
                    format!("{} · {}", stats.encoder, stats.renderer)
                })
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
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use eframe::egui;
    use reelforge_compile::{CompileOptions, compile};
    use reelforge_media::{
        AudioWriter, ExportError, ExportWriter, Footage, GifWriter, SoftwareH264, StillFormat, VideoEncoder, Yuv420,
        draw_frame, encode_still, encoded_size, still, video_encoding,
    };
    use reelforge_model::Project;
    use reelforge_render::Quality;
    use reelforge_render::gpu::GpuCompositor;
    use reelforge_render::plan::{PlannedSource, plan_frame};
    use reelforge_renderspec::{AssetKind, OutputFormat, RenderSpec, VideoCodec};
    use wasm_bindgen::prelude::*;

    use super::{CANCELLED, ExportJob, What};
    use crate::webcodecs::{WebDecoder, WebEncoder, sleep};

    #[wasm_bindgen(inline_js = r#"
export function rf_save(name, bytes, mime) {
  const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
  const a = document.createElement('a');
  a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
"#)]
    extern "C" {
        fn rf_save(name: &str, bytes: &[u8], mime: &str);
    }

    /// Exports in the background. `gpu` draws a video's frames when given (a sibling of the
    /// preview's compositor, on WebGPU); the CPU renderer draws them otherwise, and always draws a
    /// GIF's frames and single frames.
    pub(super) fn start(
        p: &Project,
        media: Arc<Footage>,
        ctx: egui::Context,
        mut gpu: Option<GpuCompositor>,
        what: What,
    ) -> Result<ExportJob, String> {
        let supported = |spec: &RenderSpec| {
            let missing = reelforge_render::unsupported_features(spec);
            if missing.is_empty() { Ok(()) } else { Err(format!("cannot export yet: {}", missing.join(", "))) }
        };
        // the timeline as it is drawn, without encode settings: for a frame, and for the cover
        let frames = || compile(p, &CompileOptions { encode: false }).map_err(|e| e.to_string());
        if let What::Frame(t) = what {
            let spec = frames()?;
            supported(&spec)?;
            let job = ExportJob::new("reelforge-frame.png".into());
            let (done, result, name) = (job.done.clone(), job.result.clone(), job.name.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let r = picture(&spec, &media, t, StillFormat::Png, &name).await;
                done.store(1, Ordering::Relaxed);
                media.clear_prepared();
                *result.lock().expect("result lock") = Some(r);
                ctx.request_repaint();
            });
            return Ok(job);
        }
        let spec = compile(p, &CompileOptions { encode: true }).map_err(|e| e.to_string())?;
        supported(&spec)?;
        let cover = match p.cover.filter(|_| p.export.format.pictured()) {
            Some(at) => Some((frames()?, at)),
            None => None,
        };
        let format = spec.output.format;
        let job = ExportJob::new(format!("reelforge.{}", format.extension()));
        let (done, total, cancel, result) =
            (job.done.clone(), job.total.clone(), job.cancel.clone(), job.result.clone());
        let name = job.name.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let r = async {
                let how = match format {
                    OutputFormat::Mp4 => video(&spec, &media, &mut gpu, &done, &total, &cancel, &name).await?,
                    OutputFormat::Gif => gif(&spec, &media, &done, &total, &cancel, &name).await?,
                    OutputFormat::M4a | OutputFormat::Wav => {
                        sound(&spec, &media, &done, &total, &cancel, &name).await?
                    }
                };
                // a video's cover is downloaded after it
                if let Some((frames, at)) = &cover {
                    picture(frames, &media, *at, StillFormat::Jpeg, "reelforge.cover.jpg").await?;
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

    /// Draws the frame of `spec` at `t` and downloads it as a picture file. Returns its size.
    async fn picture(
        spec: &RenderSpec,
        media: &Footage,
        t: reelforge_time::Tick,
        format: StillFormat,
        name: &str,
    ) -> Result<String, String> {
        let mut decoders = Decoders::default();
        decoders.prepare(spec, media, t).await;
        let img = still(spec, media, t, 1.0).map_err(|e| e.to_string());
        decoders.let_go(spec, media);
        let img = img?;
        let bytes = encode_still(&img, format).map_err(|e| e.to_string())?;
        let mime = if format == StillFormat::Png { "image/png" } else { "image/jpeg" };
        rf_save(name, &bytes, mime);
        Ok(format!("{}×{}", img.width, img.height))
    }

    /// An animated GIF: every frame drawn on the CPU at the GIF's size.
    async fn gif(
        spec: &RenderSpec,
        media: &Footage,
        done: &AtomicU64,
        total: &AtomicU64,
        cancel: &AtomicBool,
        name: &str,
    ) -> Result<String, String> {
        let step = |e: ExportError| e.to_string();
        let mut w = GifWriter::new(spec, Cursor::new(Vec::new())).map_err(step)?;
        let n = w.frames();
        total.store(n, Ordering::Relaxed);
        let mut decoders = Decoders::default();
        for i in 0..n {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.to_string());
            }
            let (t, scale) = w.next(spec);
            decoders.prepare(spec, media, t).await;
            let img = reelforge_render::render_frame(spec, t, scale, media, Quality::Final).map_err(|e| e.to_string());
            decoders.let_go(spec, media);
            w.push(&img?.0).map_err(step)?;
            done.store(i + 1, Ordering::Relaxed);
            sleep(0).await;
        }
        let (out, stats) = w.finish().map_err(step)?;
        rf_save(name, &out.into_inner(), "image/gif");
        Ok(stats.encoder)
    }

    /// The sound alone, a second at a time.
    async fn sound(
        spec: &RenderSpec,
        media: &Footage,
        done: &AtomicU64,
        total: &AtomicU64,
        cancel: &AtomicBool,
        name: &str,
    ) -> Result<String, String> {
        let step = |e: ExportError| e.to_string();
        let mut w = AudioWriter::new(spec, Cursor::new(Vec::new())).map_err(step)?;
        total.store(w.steps(), Ordering::Relaxed);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.to_string());
            }
            let more = w.step(spec, media).map_err(step)?;
            done.fetch_add(1, Ordering::Relaxed);
            sleep(0).await;
            if !more {
                break;
            }
        }
        let (out, stats) = w.finish().map_err(step)?;
        let mime = if spec.output.format == OutputFormat::Wav { "audio/wav" } else { "audio/mp4" };
        rf_save(name, &out.into_inner(), mime);
        Ok(stats.encoder)
    }

    /// An MP4: frames from the GPU when there is one, encoded by the browser where it can.
    async fn video(
        spec: &RenderSpec,
        media: &Footage,
        gpu: &mut Option<GpuCompositor>,
        done: &AtomicU64,
        total: &AtomicU64,
        cancel: &AtomicBool,
        name: &str,
    ) -> Result<String, String> {
        let venc = video_encoding(spec);
        let size = encoded_size(spec);
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
        let mut w = ExportWriter::new(spec, Cursor::new(Vec::new()), encoder).map_err(|e| e.to_string())?;
        let encoder = w.encoder();
        let n = w.frames();
        total.store(n, Ordering::Relaxed);
        let mut yuv = Yuv420::default();
        let step = |e: ExportError| e.to_string();
        let mut decoders = Decoders::default();
        for i in 0..n {
            if cancel.load(Ordering::Relaxed) {
                decoders.let_go(spec, media);
                return Err(CANCELLED.to_string());
            }
            let t = spec.output.fps.tick_of(i as i64);
            decoders.prepare(spec, media, t).await;
            match gpu {
                Some(g) => {
                    let scale = video_encoding(spec).scale(spec.output.width);
                    g.render(spec, t, scale, media, Quality::Final).map_err(|e| e.to_string())?;
                    let back = g.start_yuv420().map_err(|e| e.to_string())?;
                    // the browser maps the copy when the GPU is done; give it the turn
                    while !back.ready() {
                        sleep(0).await;
                    }
                    yuv.width = back.size()[0] as usize;
                    back.finish(&mut yuv.y, &mut yuv.u, &mut yuv.v).map_err(|e| e.to_string())?;
                }
                None => draw_frame(spec, media, i, &mut yuv).map_err(step)?,
            }
            decoders.let_go(spec, media);
            w.push(spec, media, i, &yuv).map_err(step)?;
            // let the encoder catch up, and the page draw
            while w.backlog() > 3 {
                sleep(4).await;
                w.poll(spec, media).map_err(step)?;
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
        let (out, _) = w.finish(spec, media).map_err(step)?;
        rf_save(name, &out.into_inner(), "video/mp4");
        let mut how = encoder;
        if gpu.is_some() {
            how.push_str(" · GPU");
        }
        if decoders.used {
            how.push_str(" · WebCodecs");
        }
        Ok(how)
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
            let layers: Vec<_> = plan.media_layers().into_iter().map(|(_, l)| l).collect();
            self.open.retain(|id, _| layers.iter().any(|l| &l.id == id));
            for layer in layers {
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
        // the frame under the playhead; at the very end, the last one
        let last = c.fps.snap(Tick((p.duration().0 - 1).max(0)));
        let here = c.fps.snap(self.playhead).clamp(Tick::ZERO, last);
        let mut set: Option<serde_json::Value> = None;
        let mut cover: Option<serde_json::Value> = None;
        let mut go: Option<What> = None;
        let mut open = true;
        egui::Window::new(s.export_title)
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                let video = e.format == Format::Mp4;
                if video {
                    ui.horizontal_wrapped(|ui| {
                        for (i, (id, preset)) in PRESETS.iter().enumerate() {
                            if ui.selectable_label(e == *preset, s.export_presets[i]).clicked() {
                                set = Some(json!({"preset": id}));
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                egui::Grid::new("export settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    let weak = |t: &str| egui::RichText::new(t.to_string()).color(theme::color::WEAK);
                    ui.label(weak(s.export_format));
                    let f_name = |f: Format| s.export_formats[Format::ALL.iter().position(|x| *x == f).unwrap_or(0)];
                    egui::ComboBox::from_id_salt("export format").selected_text(f_name(e.format)).width(170.0).show_ui(
                        ui,
                        |ui| {
                            for f in Format::ALL {
                                if ui.selectable_label(e.format == f, f_name(f)).clicked() {
                                    set = Some(json!({"format": f.name()}));
                                }
                            }
                        },
                    );
                    ui.end_row();
                    if video {
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
                        let fps_name =
                            |f: u32| if f == 0 { s.export_as_canvas.to_string() } else { format!("{f} fps") };
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
                    }
                    if e.format.pictured() {
                        ui.label(weak(s.export_quality));
                        let q_name =
                            |q: Quality| s.export_qualities[Quality::ALL.iter().position(|x| *x == q).unwrap_or(1)];
                        // a GIF's quality is its size and frame rate, so the list says them
                        let q_label = |q: Quality| {
                            if video {
                                return q_name(q).to_string();
                            }
                            let as_q = ExportSettings { quality: q, ..e };
                            let [w, h] = as_q.size(c);
                            let fps = as_q.frame_rate(c);
                            format!("{}  ·  {w}×{h} · {} fps", q_name(q), fps.num as f64 / fps.den as f64)
                        };
                        egui::ComboBox::from_id_salt("export quality")
                            .selected_text(q_label(e.quality))
                            .width(if video { 170.0 } else { 220.0 })
                            .show_ui(ui, |ui| {
                                for q in Quality::ALL {
                                    if ui.selectable_label(e.quality == q, q_label(q)).clicked() {
                                        set = Some(json!({"quality": q.name()}));
                                    }
                                }
                            });
                        ui.end_row();
                    }
                    if video {
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
                    }
                    if e.format.pictured() {
                        ui.label(weak(s.export_cover)).on_hover_text(s.export_cover_tip);
                        ui.horizontal(|ui| {
                            match p.cover {
                                Some(at) => ui.monospace(theme::timecode(at.seconds())),
                                None => ui.label(weak(s.export_cover_none)),
                            };
                            if ui
                                .add_enabled(p.cover != Some(here), egui::Button::new(s.export_cover_set).small())
                                .on_hover_text(s.export_cover_tip)
                                .clicked()
                            {
                                cover = Some(json!({"at": here}));
                            }
                            if p.cover.is_some() && ui.small_button(s.export_cover_clear).clicked() {
                                cover = Some(json!({"at": null}));
                            }
                        });
                        ui.end_row();
                    }
                });
                let note = match e.format {
                    Format::Mp4 => None,
                    Format::Gif => Some(s.export_gif_note),
                    Format::M4a | Format::Wav => Some(s.export_sound_note),
                };
                if let Some(note) = note {
                    ui.label(egui::RichText::new(note).small().color(theme::color::WEAK));
                }
                let encodes = !video || encodes(e.codec);
                if !encodes {
                    ui.label(egui::RichText::new(s.export_codec_web_only).small().color(theme::color::WARN));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let any = !p.tracks.iter().all(|t| t.clips.is_empty());
                    if theme::primary(ui, theme::icon::EXPORT, s.export, encodes && any).clicked() {
                        go = Some(What::Timeline);
                    }
                    if ui
                        .add_enabled(any, egui::Button::new(s.export_frame))
                        .on_hover_text(s.export_frame_tip)
                        .clicked()
                    {
                        go = Some(What::Frame(here));
                    }
                });
            });
        if let Some(v) = set {
            let _ = self.execute("project.setExport", v);
        }
        if let Some(v) = cover {
            let _ = self.execute("project.setCover", v);
        }
        self.export_open = open && go.is_none();
        if let Some(what) = go {
            #[cfg(not(target_arch = "wasm32"))]
            let started = native::start(&p, &self.export_dir, what);
            // the preview's device draws the frames where it can convert them to 4:2:0 (WebGPU)
            #[cfg(target_arch = "wasm32")]
            let started = web::start(
                &p,
                self.media.clone(),
                ctx.clone(),
                self.gpu.as_ref().map(|g| g.compositor.sibling()).filter(|g| g.converts_yuv()),
                what,
            );
            match started {
                Ok(job) => self.export = Some(job),
                Err(e) => self.status = fill(self.s().export_failed, &[&e]),
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use reelforge_engine::Session;
    use reelforge_model::export::Format;
    use reelforge_time::Tick;
    use serde_json::json;

    use super::{ExportJob, What, native};

    fn wait(job: ExportJob) -> (String, Result<String, String>) {
        for _ in 0..3000 {
            if let Some(r) = job.result.lock().expect("result lock").take() {
                return (job.name, r);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the export never finished");
    }

    #[test]
    fn a_job_writes_the_kind_of_file_asked_for_with_the_cover_beside_pictures() {
        let dir = std::env::temp_dir().join(format!("reelforge-export-job-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::default();
        s.execute("project.setCanvas", &json!({"width": 64, "height": 64})).unwrap();
        let track = s.project().tracks[0].id.clone();
        s.execute("clip.add", &json!({"track": track, "color": [0, 1, 0], "duration": {"frame": 6}})).unwrap();
        s.execute("sound.add", &json!({"sound": "pop"})).unwrap();
        s.execute("project.setCover", &json!({"at": {"frame": 2}})).unwrap();

        // the frame under the playhead: a PNG at the canvas's size, whatever the format
        let (name, how) = wait(native::start(&s.project(), &dir, What::Frame(Tick::ZERO)).unwrap());
        assert_eq!((name.ends_with("reelforge-1.png"), how.as_deref()), (true, Ok("64×64")));
        assert_eq!(&std::fs::read(dir.join("reelforge-1.png")).unwrap()[1..4], b"PNG");

        let mut kinds = Vec::new();
        for f in [Format::Gif, Format::Wav, Format::M4a] {
            s.execute("project.setExport", &json!({"format": f.name()})).unwrap();
            let (name, how) = wait(native::start(&s.project(), &dir, What::Timeline).unwrap());
            assert!(name.ends_with(&format!("reelforge-1.{}", f.name())), "{name}");
            kinds.push(how.unwrap());
            assert!(!dir.join(format!("reelforge-1.{}.part", f.name())).exists());
        }
        assert_eq!(kinds, ["gif · cpu", "pcm 16-bit", "aac"]);
        assert_eq!(&std::fs::read(dir.join("reelforge-1.gif")).unwrap()[..6], b"GIF89a");
        assert_eq!(&std::fs::read(dir.join("reelforge-1.wav")).unwrap()[..4], b"RIFF");
        // the cover goes with pictures only: the GIF wrote it, the sound did not write another
        assert_eq!(&std::fs::read(dir.join("reelforge-1.cover.jpg")).unwrap()[..2], [0xff, 0xd8]);
        let covers = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".cover.jpg"))
            .count();
        assert_eq!(covers, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
