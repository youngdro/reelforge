//! The editor interface: egui, drawn by eframe's wgpu renderer on the desktop and (WebGPU, or
//! WebGL2 as a fallback) in the browser.
//!
//! The interface decides nothing about an edit. Buttons, drags and the inspector all end in
//! [`EditorApp::execute`], which runs an engine command, exactly as the CLI or an agent would; the
//! interface only reads the project snapshot back to draw it. The preview is a RenderSpec drawn by
//! `reelforge-render`, the same path an export takes.
//!
//! Today the preview is composited on the CPU and uploaded as a texture. The wgpu compositor will
//! draw into a texture on eframe's own device instead, so the frame never leaves the GPU.

mod timeline;

use std::sync::Arc;

use eframe::egui;
use reelforge_compile::{CompileOptions, compile};
use reelforge_engine::Session;
use reelforge_model::{ClipContent, Project, TrackKind};
use reelforge_render::{Quality, StillImages, render_frame};
use reelforge_time::Tick;
use serde_json::{Value, json};

/// Longest side of the preview render, in pixels.
const PREVIEW_LONG_SIDE: f32 = 640.0;

/// The editor window's state.
pub struct EditorApp {
    session: Session,
    media: StillImages,
    playhead: Tick,
    playing: bool,
    selected: Option<String>,
    status: String,
    preview: Option<egui::TextureHandle>,
    preview_key: Option<(u64, i64)>,
    timeline: timeline::TimelineView,
    inspector: Option<Inspector>,
}

/// Values being edited in the inspector, committed as one command when the gesture ends.
#[derive(Clone, PartialEq)]
struct Inspector {
    clip: String,
    x: f32,
    y: f32,
    scale: f32,
    rotation: f32,
    opacity: f32,
    text: Option<String>,
}

impl EditorApp {
    /// An editor on `session`.
    pub fn new(session: Session) -> EditorApp {
        EditorApp {
            session,
            media: StillImages::new(),
            playhead: Tick::ZERO,
            playing: false,
            selected: None,
            status: "Ready".into(),
            preview: None,
            preview_key: None,
            timeline: timeline::TimelineView::default(),
            inspector: None,
        }
    }

    /// An editor on a small demo project: a colour background and a title.
    pub fn demo() -> EditorApp {
        let mut s = Session::default();
        let p = s.project();
        let (video, text) = (p.tracks[0].id.clone(), p.tracks[1].id.clone());
        let steps = json!({"label": "Demo", "steps": [
            {"id": "clip.add", "params": {"track": video, "color": [0.12, 0.10, 0.30], "duration": {"seconds": 6}}},
            {"id": "clip.add", "params": {"track": video, "color": [0.85, 0.35, 0.20], "start": {"seconds": 6}, "duration": {"seconds": 4}}},
            {"id": "clip.add", "params": {"track": text, "text": "reelforge", "size": 140, "start": {"seconds": 1}, "duration": {"seconds": 4}}},
        ]});
        s.execute("edit.batch", &steps).expect("the demo project is valid");
        EditorApp::new(s)
    }

    /// The session, for hosts that drive the editor (the web page's API, a control channel).
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Runs a command and reports the outcome in the status bar.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        match self.session.execute(id, &params) {
            Ok(v) => {
                self.status = format!("{id}: done");
                Ok(v)
            }
            Err(e) => {
                self.status = format!("{id}: {e}");
                Err(e.to_string())
            }
        }
    }

    fn project(&self) -> Arc<Project> {
        self.session.project()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let p = self.project();
            let track_of = |kind| p.tracks.iter().find(|t| t.kind == kind).map(|t| t.id.clone());
            if ui.button(if self.playing { "⏸ Pause" } else { "▶ Play" }).clicked() {
                self.playing = !self.playing;
                if self.playing && self.playhead >= p.duration() {
                    self.playhead = Tick::ZERO;
                }
            }
            ui.separator();
            if let Some(t) = track_of(TrackKind::Video)
                && ui.button("+ Colour").clicked()
            {
                let _ =
                    self.execute("clip.add", json!({"track": t, "color": [0.2, 0.5, 0.9], "duration": {"seconds": 3}}));
            }
            if let Some(t) = track_of(TrackKind::Text)
                && ui.button("+ Title").clicked()
            {
                let _ = self.execute(
                    "clip.add",
                    json!({"track": t, "text": "Title", "start": self.playhead, "duration": {"seconds": 3}}),
                );
            }
            let sel = self.selected.clone();
            if ui.add_enabled(sel.is_some(), egui::Button::new("✂ Split")).clicked() {
                let _ = self.execute("clip.split", json!({"clip": sel, "at": self.playhead}));
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("🗑 Delete")).clicked()
                && self.execute("clip.remove", json!({"clips": sel})).is_ok()
            {
                self.selected = None;
            }
            ui.separator();
            if ui.button("↶ Undo").clicked() {
                let _ = self.execute("edit.undo", json!({}));
            }
            if ui.button("↷ Redo").clicked() {
                let _ = self.execute("edit.redo", json!({}));
            }
            ui.separator();
            let fps = p.canvas.fps;
            ui.monospace(format!("{:.2}s · frame {}", self.playhead.seconds(), fps.frame_at(self.playhead)));
            ui.separator();
            ui.label(egui::RichText::new(&self.status).weak());
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let p = self.project();
        let Some(id) = self.selected.clone() else {
            ui.label("Select a clip on the timeline.");
            return;
        };
        let Some((_, clip)) = p.clip(&id) else {
            self.selected = None;
            return;
        };
        let current = Inspector {
            clip: id.clone(),
            x: clip.transform.x,
            y: clip.transform.y,
            scale: clip.transform.scale,
            rotation: clip.transform.rotation,
            opacity: clip.opacity,
            text: match &clip.content {
                ClipContent::Text { text, .. } => Some(text.clone()),
                _ => None,
            },
        };
        if self.inspector.as_ref().is_none_or(|i| i.clip != id) {
            self.inspector = Some(current.clone());
        }
        ui.label(format!("{id} · {:.2}s → {:.2}s", clip.start.seconds(), clip.end().seconds()));
        let edit = self.inspector.as_mut().expect("set above");
        let mut commit = false;
        egui::Grid::new("inspector").num_columns(2).show(ui, |ui| {
            let mut row =
                |ui: &mut egui::Ui, name: &str, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>| {
                    ui.label(name);
                    let r = ui.add(egui::DragValue::new(v).speed(speed).range(range));
                    commit |= r.drag_stopped() || (r.changed() && !r.dragged());
                    ui.end_row();
                };
            row(ui, "X", &mut edit.x, 0.005, -2.0..=2.0);
            row(ui, "Y", &mut edit.y, 0.005, -2.0..=2.0);
            row(ui, "Scale", &mut edit.scale, 0.01, 0.0..=10.0);
            row(ui, "Rotation", &mut edit.rotation, 0.5, -360.0..=360.0);
            row(ui, "Opacity", &mut edit.opacity, 0.01, 0.0..=1.0);
            if let Some(text) = &mut edit.text {
                ui.label("Text");
                let r = ui.text_edit_singleline(text);
                commit |= r.lost_focus();
                ui.end_row();
            }
        });
        if commit && self.inspector.as_ref() != Some(&current) {
            let e = self.inspector.clone().expect("set");
            let mut params =
                json!({"clip": id, "x": e.x, "y": e.y, "scale": e.scale, "rotation": e.rotation, "opacity": e.opacity});
            if let Some(t) = e.text {
                params["text"] = json!(t);
            }
            if self.execute("clip.set", params).is_err() {
                self.inspector = Some(current);
            }
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let p = self.project();
        let frame = p.canvas.fps.frame_at(self.playhead);
        let key = (self.session.revision(), frame);
        if self.preview_key != Some(key) {
            self.preview_key = Some(key);
            let long = p.canvas.width.max(p.canvas.height) as f32;
            let scale = (PREVIEW_LONG_SIDE / long).min(1.0);
            let image = match compile(&p, &CompileOptions { encode: false }) {
                Ok(spec) => {
                    match render_frame(&spec, p.canvas.fps.snap(self.playhead), scale, &self.media, Quality::Draft) {
                        Ok((img, _report)) => Some(img),
                        Err(e) => {
                            self.status = format!("preview: {e}");
                            None
                        }
                    }
                }
                Err(_) => None, // an empty timeline: show the background
            };
            let color = match image {
                Some(img) => egui::ColorImage::from_rgba_unmultiplied(
                    [img.width as usize, img.height as usize],
                    &img.to_srgba8(),
                ),
                None => {
                    let s = PREVIEW_LONG_SIDE / p.canvas.width.max(p.canvas.height) as f32;
                    let (w, h) = ((p.canvas.width as f32 * s) as usize, (p.canvas.height as f32 * s) as usize);
                    let bg = p.canvas.background.map(|c| (c * 255.0) as u8);
                    egui::ColorImage::new(
                        [w.max(1), h.max(1)],
                        vec![egui::Color32::from_rgb(bg[0], bg[1], bg[2]); w.max(1) * h.max(1)],
                    )
                }
            };
            match &mut self.preview {
                Some(t) => t.set(color, egui::TextureOptions::LINEAR),
                None => self.preview = Some(ui.ctx().load_texture("preview", color, egui::TextureOptions::LINEAR)),
            }
        }
        if let Some(tex) = &self.preview {
            let avail = ui.available_size();
            let [w, h] = tex.size().map(|v| v as f32);
            let s = (avail.x / w).min(avail.y / h);
            ui.centered_and_justified(|ui| {
                ui.add(egui::Image::new((tex.id(), egui::vec2(w * s, h * s))));
            });
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.playing {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            self.playhead += Tick::from_seconds(dt as f64);
            let end = self.project().duration();
            if self.playhead >= end {
                self.playhead = end;
                self.playing = false;
            }
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().egui_wants_keyboard_input() {
            self.playing = !self.playing;
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("inspector").resizable(true).default_size(240.0).show(ui, |ui| self.inspector(ui));
        let p = self.project();
        let out = egui::Panel::bottom("timeline")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| self.timeline.show(ui, &p, self.playhead, self.selected.as_deref()))
            .inner;
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));

        if let Some(t) = out.seek {
            self.playhead = p.canvas.fps.snap(t.max(Tick::ZERO));
        }
        if let Some(sel) = out.select {
            self.selected = sel;
        }
        if let Some((clip, start, track)) = out.moved {
            let _ = self.execute(
                "clip.move",
                json!({"clip": clip, "start": p.canvas.fps.snap(start.max(Tick::ZERO)), "track": track}),
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
