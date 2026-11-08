//! The editor interface: egui, drawn by eframe's wgpu renderer on the desktop and (WebGPU, or
//! WebGL2 as a fallback) in the browser.
//!
//! The interface decides nothing about an edit. Buttons, drags and the inspector all end in
//! [`EditorApp::execute`], which runs an engine command, exactly as the CLI or an agent would; the
//! interface only reads the project snapshot back to draw it. The preview is a RenderSpec drawn by
//! `reelforge-render`, the same path an export takes.
//!
//! Given eframe's wgpu device ([`EditorApp::with_gpu`]), the preview is drawn by the wgpu
//! compositor into a texture egui shows as it is, so a frame never leaves the GPU. Without one (or
//! on a WebGL2 fallback, which lacks what the compositor needs) it is drawn on the CPU and
//! uploaded.
//!
//! Files dropped on the window are imported (`media.import`) and placed at the playhead
//! (`clip.add`). On the desktop, Export writes an MP4 on a background thread.
//!
//! [`EditorApp::join`] edits a session a server holds instead, with agents and other editors: see
//! [`remote`].

mod audio;
#[cfg(not(target_arch = "wasm32"))]
mod export_job;
pub mod i18n;
pub mod remote;
mod theme;
mod timeline;

use std::sync::{Arc, Mutex};

use eframe::egui;
use i18n::{Lang, Strings, fill};
use reelforge_compile::{CompileOptions, compile};
use reelforge_engine::{Session, SharedSession};
use reelforge_media::Footage;
use reelforge_model::{ClipContent, MediaKind, Project, TrackKind};
use reelforge_render::{Quality, render_frame};
use reelforge_time::Tick;
use serde_json::{Value, json};
use theme::icon;

/// Longest side of the preview render, in pixels.
const PREVIEW_LONG_SIDE: f32 = 640.0;

/// A file the browser finished reading: its name and bytes.
type Arrival = (String, Result<Vec<u8>, String>);

/// The editor window's state.
pub struct EditorApp {
    session: Arc<SharedSession>,
    /// The last revision drawn, to notice changes other clients made.
    seen: u64,
    media: Arc<Footage>,
    /// Files dropped in the browser, read asynchronously: `(name, bytes)`.
    arrivals: Arc<Mutex<Vec<Arrival>>>,
    #[cfg(not(target_arch = "wasm32"))]
    export: Option<export_job::ExportJob>,
    #[cfg(not(target_arch = "wasm32"))]
    export_dir: std::path::PathBuf,
    playhead: Tick,
    playing: bool,
    /// Preview sound, and the project revision its mix was compiled from.
    sound: audio::Playback,
    sound_spec: Option<(u64, Arc<reelforge_renderspec::RenderSpec>)>,
    selected: Option<String>,
    status: String,
    preview: Option<egui::TextureHandle>,
    preview_key: Option<(u64, i64, u64)>,
    gpu: Option<GpuPreview>,
    /// Whether the bundled Chinese face has been given to egui.
    fonts: bool,
    timeline: timeline::TimelineView,
    inspector: Option<Inspector>,
    /// The server whose session this editor mirrors, when it joined one.
    remote: Option<remote::Remote>,
    /// The interface's language.
    lang: Lang,
    /// Whether the person picked the language (then it is remembered; otherwise the system's is used).
    lang_chosen: bool,
    /// A pick not written to storage yet (the window may sleep past eframe's autosave).
    lang_unsaved: bool,
    /// The window, once it is known, for a remote's answers to wake it.
    window: Arc<std::sync::OnceLock<egui::Context>>,
}

/// The wgpu compositor on eframe's device, and the egui texture showing its frames.
struct GpuPreview {
    compositor: reelforge_render::gpu::GpuCompositor,
    renderer: Arc<egui::mutex::RwLock<eframe::egui_wgpu::Renderer>>,
    device: eframe::wgpu::Device,
    /// The registered texture and the frame size it was registered at.
    texture: Option<(egui::TextureId, [u32; 2])>,
}

impl GpuPreview {
    /// Converts the frame for egui and (re)registers the texture when its size changed.
    fn show(&mut self) {
        let Some(tex) = self.compositor.display() else { return };
        let size = [tex.width(), tex.height()];
        let id = match self.texture {
            Some((id, old)) if old == size => id,
            known => {
                let view = tex.create_view(&Default::default());
                let filter = eframe::wgpu::FilterMode::Linear;
                let mut r = self.renderer.write();
                match known {
                    Some((id, _)) => {
                        r.update_egui_texture_from_wgpu_texture(&self.device, &view, filter, id);
                        id
                    }
                    None => r.register_native_texture(&self.device, &view, filter),
                }
            }
        };
        self.texture = Some((id, size));
    }
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
    /// Colour and title clips.
    color: Option<[f32; 4]>,
    /// Clips with sound, in dB.
    volume: Option<f32>,
    title: Option<Title>,
}

/// A title's own values in the inspector.
#[derive(Clone, PartialEq)]
struct Title {
    text: String,
    size: f32,
    bold: bool,
    outline: bool,
    boxed: bool,
}

impl EditorApp {
    /// An editor on `session`.
    pub fn new(session: Session) -> EditorApp {
        EditorApp::shared(SharedSession::new(session))
    }

    /// An editor on a session other clients edit too (agents through a server the app hosts). Their
    /// changes show as they commit.
    pub fn shared(session: Arc<SharedSession>) -> EditorApp {
        EditorApp {
            seen: session.revision(),
            session,
            media: Arc::new(Footage::new()),
            arrivals: Arc::default(),
            #[cfg(not(target_arch = "wasm32"))]
            export: None,
            #[cfg(not(target_arch = "wasm32"))]
            export_dir: std::env::current_dir().unwrap_or_default(),
            playhead: Tick::ZERO,
            playing: false,
            sound: audio::Playback::default(),
            sound_spec: None,
            selected: None,
            status: String::new(),
            preview: None,
            preview_key: None,
            gpu: None,
            fonts: false,
            timeline: timeline::TimelineView::default(),
            inspector: None,
            remote: None,
            lang: Lang::detect(),
            lang_chosen: false,
            lang_unsaved: false,
            window: Arc::default(),
        }
    }

    /// An editor on the session the server at `base` (`http://127.0.0.1:7878`) holds, shared with
    /// agents and other editors; `client` names it to them (`web`, `desktop`).
    pub fn join(base: &str, token: Option<String>, client: &str) -> EditorApp {
        let mut app = EditorApp::new(Session::default());
        let window = app.window.clone();
        app.remote =
            Some(remote::Remote::join(base, token, client, app.session.clone(), app.media.clone(), move || {
                if let Some(ctx) = window.get() {
                    ctx.request_repaint();
                }
            }));
        app.status = fill(app.s().joining, &[&base]);
        app
    }

    /// The interface's words.
    fn s(&self) -> &'static Strings {
        self.lang.strings()
    }

    /// Shows the interface in `lang`.
    pub fn with_lang(mut self, lang: Lang) -> EditorApp {
        self.lang = lang;
        self
    }

    /// The server this editor joined, if it did.
    pub fn remote(&self) -> Option<&remote::Remote> {
        self.remote.as_ref()
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
            {"id": "clip.add", "params": {"track": text, "text": "全 Rust 短视频剪辑", "size": 72, "start": {"seconds": 5}, "duration": {"seconds": 4}}},
        ]});
        s.execute("edit.batch", &steps).expect("the demo project is valid");
        EditorApp::new(s)
    }

    /// Starts with `status` in the status bar.
    pub fn with_status(mut self, status: String) -> EditorApp {
        self.status = status;
        self
    }

    /// Takes what eframe gives at start: the language the person picked last time, and eframe's
    /// device, to draw the preview with the wgpu compositor when it has what that needs (WebGPU,
    /// Vulkan, Metal, DX12; not WebGL2).
    pub fn with_gpu(mut self, cc: &eframe::CreationContext<'_>) -> EditorApp {
        if let Some(lang) = cc.storage.and_then(|s| s.get_string(LANG_KEY)).as_deref().and_then(Lang::from_code) {
            self.lang = lang;
            self.lang_chosen = true;
        }
        if let Some(rs) = &cc.wgpu_render_state {
            let info = rs.adapter.get_info();
            if info.backend != eframe::wgpu::Backend::Gl {
                let compositor = reelforge_render::gpu::GpuCompositor::new(rs.device.clone(), rs.queue.clone(), &info);
                self.gpu = Some(GpuPreview {
                    compositor,
                    renderer: rs.renderer.clone(),
                    device: rs.device.clone(),
                    texture: None,
                });
            }
        }
        self
    }

    /// Where Export writes its files (the desktop app passes the project's folder).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_export_dir(mut self, dir: std::path::PathBuf) -> EditorApp {
        self.export_dir = dir;
        self
    }

    /// The session, for hosts that drive the editor (the web page's API, a server).
    pub fn session(&self) -> &Arc<SharedSession> {
        &self.session
    }

    /// Runs a command and reports the outcome in the status bar.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        match self.run(id, &params) {
            Ok(v) => {
                self.status = fill(self.s().done, &[&self.lang.command(id)]);
                Ok(v)
            }
            Err(e) => {
                let what = match self.lang.command(id) {
                    "" => id,
                    name => name,
                };
                self.status = format!("{what}: {e}");
                Err(e.to_string())
            }
        }
    }

    /// Runs a command on this editor's session and, when it joined a server, sends it there.
    fn run(&self, id: &str, params: &Value) -> reelforge_engine::CmdResult {
        let Some(remote) = &self.remote else { return self.session.execute(id, params, "editor") };
        if matches!(id, "edit.undo" | "edit.redo") {
            // the server's history holds everyone's edits; what it undoes comes back as a change
            remote.send(id, params);
            return Ok(json!({}));
        }
        let edits = reelforge_engine::registry().iter().any(|c| c.id == id && c.edits);
        let out = self.session.execute(id, params, "editor")?;
        if edits {
            remote.send(id, params);
        }
        Ok(out)
    }

    fn project(&self) -> Arc<Project> {
        self.session.project()
    }

    /// Plays from the playhead (from the start, at the end), or pauses.
    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        if playing {
            if self.playhead >= self.project().duration() {
                self.playhead = Tick::ZERO;
            }
            self.sound.start(self.playhead);
            if let Some(why) = self.sound.take_problem() {
                self.status = fill(self.s().no_sound, &[&why]);
            }
        } else {
            self.sound.stop();
        }
    }

    /// Moves the playhead on while playing: by the sound the device took, or by `dt` seconds
    /// without a device; and mixes the sound ahead.
    fn advance(&mut self, dt: f64) {
        let p = self.project();
        let revision = self.session.revision();
        if self.sound_spec.as_ref().is_none_or(|(r, _)| *r != revision) {
            let stale = self.sound_spec.is_some();
            self.sound_spec =
                compile(&p, &CompileOptions { encode: false }).ok().map(|spec| (revision, Arc::new(spec)));
            if stale && self.sound.running() {
                // what was mixed ahead is of the old cut: mix again from here
                self.sound.start(self.playhead);
            }
        }
        self.playhead = match self.sound.clock() {
            Some(t) => t,
            None => self.playhead + Tick::from_seconds(dt),
        };
        let end = p.duration();
        if self.playhead >= end {
            self.playhead = end;
            self.set_playing(false);
            return;
        }
        if let Some((_, spec)) = &self.sound_spec {
            self.sound.fill(spec, &self.media);
        }
    }

    /// Imports a file on disk and places it at the playhead.
    #[cfg(not(target_arch = "wasm32"))]
    fn import_path(&mut self, path: &std::path::Path) {
        let path = path.to_string_lossy().into_owned();
        if let Ok(v) = self.execute("media.import", json!({ "path": path })) {
            self.place(&v);
        }
    }

    /// Imports a file the browser handed over and places it at the playhead.
    fn import_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        if let Some(remote) = &self.remote {
            // the server keeps the file; it is imported by the server's path when it is there
            self.status = match remote.upload(name, bytes) {
                Ok(()) => fill(self.s().sending, &[&name]),
                Err(e) => format!("{name}: {e}"),
            };
            return;
        }
        use sha2::{Digest, Sha256};
        let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let probe = match self.media.insert_bytes(&sha, name, bytes) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("{name}: {e}");
                return;
            }
        };
        let kind = match probe.kind {
            reelforge_renderspec::AssetKind::Video => "video",
            reelforge_renderspec::AssetKind::Image => "image",
            _ => "audio",
        };
        let mut params = json!({ "path": name, "kind": kind, "sha256": sha });
        if let Some(size) = probe.size {
            params["size"] = json!(size);
        }
        if let Some(d) = probe.duration {
            params["duration"] = json!(d);
        }
        if let Ok(v) = self.execute("media.import", params) {
            self.place(&v);
        }
    }

    /// Puts newly imported media at the playhead, on the lowest track of its kind with room there,
    /// or on a new track above the others.
    fn place(&mut self, imported: &Value) {
        let Ok(m) = serde_json::from_value::<reelforge_model::Media>(imported["media"].clone()) else { return };
        let want = if m.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
        let tracks: Vec<String> =
            self.project().tracks.iter().filter(|t| t.kind == want).map(|t| t.id.clone()).collect();
        let at = self.playhead;
        let add = |track: &str| json!({"track": track, "media": m.id, "start": at});
        // a track with no room refuses the clip and nothing changes, so trying is safe
        let mut placed = tracks.iter().find_map(|t| self.run("clip.add", &add(t)).ok());
        if placed.is_none() {
            let kind = if want == TrackKind::Audio { "audio" } else { "video" };
            if let Ok(t) = self.execute("track.add", json!({ "kind": kind })) {
                let id = t["track"].as_str().unwrap_or_default().to_string();
                placed = self.execute("clip.add", add(&id)).ok();
            }
        }
        if let Some(v) = placed {
            self.selected = v["clip"].as_str().map(str::to_string);
            self.status = fill(self.s().imported, &[&m.name]);
        }
    }

    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            if self.remote.is_some() {
                // the server may be another machine: send the file's bytes
                let name = f.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match std::fs::read(f.path()) {
                    Ok(bytes) => self.import_bytes(&name, bytes),
                    Err(e) => self.status = format!("{name}: {e}"),
                }
            } else {
                self.import_path(f.path());
            }
            #[cfg(target_arch = "wasm32")]
            {
                let arrivals = self.arrivals.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let name = f.path().to_string_lossy().into_owned();
                    let bytes = f.bytes_async().await;
                    arrivals.lock().expect("arrivals lock").push((name, bytes));
                    ctx.request_repaint();
                });
            }
        }
        let arrived = std::mem::take(&mut *self.arrivals.lock().expect("arrivals lock"));
        for (name, bytes) in arrived {
            match bytes {
                Ok(b) => self.import_bytes(&name, b),
                Err(e) => self.status = format!("{name}: {e}"),
            }
        }
    }

    fn track_of(&self, kind: TrackKind) -> Option<String> {
        self.project().tracks.iter().find(|t| t.kind == kind).map(|t| t.id.clone())
    }

    fn add_title(&mut self) {
        if let Some(t) = self.track_of(TrackKind::Text) {
            let _ = self.execute(
                "clip.add",
                json!({"track": t, "text": self.s().new_title, "start": self.playhead, "duration": {"seconds": 3}}),
            );
        }
    }

    fn add_colour(&mut self) {
        if let Some(t) = self.track_of(TrackKind::Video) {
            let _ = self.execute(
                "clip.add",
                json!({"track": t, "color": [0.2, 0.5, 0.9], "start": self.playhead, "duration": {"seconds": 3}}),
            );
        }
    }

    fn split(&mut self) {
        if let Some(sel) = self.selected.clone() {
            let _ = self.execute("clip.split", json!({"clip": sel, "at": self.playhead}));
        }
    }

    fn delete(&mut self) {
        if let Some(sel) = self.selected.clone()
            && self.execute("clip.remove", json!({"clips": sel})).is_ok()
        {
            self.selected = None;
            self.inspector = None;
        }
    }

    fn seek(&mut self, t: Tick) {
        let p = self.project();
        self.playhead = p.canvas.fps.snap(t.max(Tick::ZERO).min(p.duration()));
        if self.playing {
            self.sound.start(self.playhead);
        }
    }

    /// Keys, when no text field has them.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let fps = self.project().canvas.fps;
        if pressed(Modifiers::NONE, Key::Space) {
            self.set_playing(!self.playing);
        }
        if pressed(cmd | Modifiers::SHIFT, Key::Z) || pressed(cmd, Key::Y) {
            let _ = self.execute("edit.redo", json!({}));
        }
        if pressed(cmd, Key::Z) {
            let _ = self.execute("edit.undo", json!({}));
        }
        if pressed(Modifiers::NONE, Key::S) || pressed(cmd, Key::B) {
            self.split();
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.delete();
        }
        let frame = fps.frame_at(self.playhead);
        let second = fps.fps().round() as i64;
        for (m, k, step) in [
            (Modifiers::NONE, Key::ArrowLeft, -1),
            (Modifiers::NONE, Key::ArrowRight, 1),
            (Modifiers::SHIFT, Key::ArrowLeft, -second),
            (Modifiers::SHIFT, Key::ArrowRight, second),
        ] {
            if pressed(m, k) {
                self.seek(fps.tick_of(frame + step));
            }
        }
        if pressed(Modifiers::NONE, Key::Home) {
            self.seek(Tick::ZERO);
        }
        if pressed(Modifiers::NONE, Key::End) {
            self.seek(self.project().duration());
        }
    }

    /// The bar along the top: name, what the project is, who is here, undo, export.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.project();
        let s = self.s();
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("reelforge").strong().size(15.0).color(theme::color::ACCENT));
            ui.add_space(6.0);
            let c = &p.canvas;
            ui.label(
                egui::RichText::new(format!(
                    "{}×{} · {} fps · {}",
                    c.width,
                    c.height,
                    (c.fps.fps() * 100.0).round() / 100.0,
                    theme::timecode(p.duration().seconds())
                ))
                .color(theme::color::WEAK),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                self.export_controls(ui, &p);
                ui.add_space(4.0);
                if ui
                    .add(egui::Button::new(egui::RichText::new(s.switch_to).color(theme::color::WEAK)).frame(false))
                    .on_hover_text(s.switch_tip)
                    .clicked()
                {
                    if self.status == s.ready {
                        self.status.clear();
                    }
                    self.lang = if self.lang == Lang::Zh { Lang::En } else { Lang::Zh };
                    self.lang_chosen = true;
                    self.lang_unsaved = true;
                }
                if theme::tool(ui, icon::REDO, "", s.redo_tip, true).clicked() {
                    let _ = self.execute("edit.redo", json!({}));
                }
                if theme::tool(ui, icon::UNDO, "", s.undo_tip, true).clicked() {
                    let _ = self.execute("edit.undo", json!({}));
                }
                if let Some(remote) = &self.remote {
                    ui.add_space(8.0);
                    let host = remote.base().split("://").nth(1).unwrap_or(remote.base()).to_string();
                    let (tint, word) = if !remote.joined() {
                        (theme::color::WARN, s.connecting)
                    } else if remote.settled() {
                        (theme::color::ACCENT, s.live)
                    } else {
                        (theme::color::WARN, s.syncing)
                    };
                    ui.label(egui::RichText::new(format!("{}  {word} · {host}", icon::CONNECTED)).color(tint))
                        .on_hover_text(s.live_tip);
                }
            });
        });
    }

    /// Under the preview: where the playhead is, and play.
    fn transport(&mut self, ui: &mut egui::Ui) {
        let p = self.project();
        let fps = p.canvas.fps;
        let s = self.s();
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(theme::timecode(self.playhead.seconds())).monospace().color(theme::color::TEXT),
            );
            ui.label(
                egui::RichText::new(format!("/ {}", theme::timecode(p.duration().seconds())))
                    .monospace()
                    .color(theme::color::WEAK),
            );
            // the play controls, centred on the panel
            let w = 4.0 * 34.0 + 3.0 * 6.0 + 12.0;
            let room = ui.available_width();
            ui.add_space(((room - w) / 2.0 - 120.0).max(8.0));
            let frame = fps.frame_at(self.playhead);
            if theme::tool(ui, icon::TO_START, "", s.to_start, true).clicked() {
                self.seek(Tick::ZERO);
            }
            if theme::tool(ui, icon::FRAME_BACK, "", s.prev_frame, true).clicked() {
                self.seek(fps.tick_of(frame - 1));
            }
            let play = egui::Button::new(
                egui::RichText::new(if self.playing { icon::PAUSE } else { icon::PLAY })
                    .size(18.0)
                    .color(theme::color::ON_ACCENT),
            )
            .fill(theme::color::ACCENT)
            .min_size(egui::vec2(44.0, 30.0))
            .corner_radius(15.0);
            if ui.add(play).on_hover_text(s.play_tip).clicked() {
                self.set_playing(!self.playing);
            }
            if theme::tool(ui, icon::FRAME_ON, "", s.next_frame, true).clicked() {
                self.seek(fps.tick_of(frame + 1));
            }
            if theme::tool(ui, icon::TO_END, "", s.to_end, true).clicked() {
                self.seek(p.duration());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(fill(s.frame, &[&frame])).monospace().color(theme::color::WEAK));
            });
        });
    }

    /// Above the timeline: edit tools, snapping, zoom.
    fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let sel = self.selected.is_some();
        let s = self.s();
        ui.horizontal(|ui| {
            if theme::tool(ui, icon::SPLIT, s.split, s.split_tip, sel).clicked() {
                self.split();
            }
            if theme::tool(ui, icon::DELETE, s.delete, s.delete_tip, sel).clicked() {
                self.delete();
            }
            ui.separator();
            if theme::tool(ui, icon::TEXT, s.title, s.title_tip, true).clicked() {
                self.add_title();
            }
            if theme::tool(ui, icon::COLOUR, s.colour, s.colour_tip, true).clicked() {
                self.add_colour();
            }
            ui.label(
                egui::RichText::new(format!("{}  {}", icon::IMPORT, s.drop_hint)).color(theme::color::WEAK).small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::tool(ui, icon::FIT, "", s.fit_tip, true).clicked() {
                    let width = ui.ctx().content_rect().width();
                    self.timeline.fit(self.project().duration().seconds(), width);
                }
                if theme::tool(ui, icon::ZOOM_IN, "", s.zoom_in_tip, true).clicked() {
                    self.timeline.zoom(1.5);
                }
                if theme::tool(ui, icon::ZOOM_OUT, "", s.zoom_out_tip, true).clicked() {
                    self.timeline.zoom(1.0 / 1.5);
                }
                let snap = self.timeline.snap;
                let magnet = egui::Button::new(egui::RichText::new(icon::SNAP).size(14.0).color(if snap {
                    theme::color::ACCENT
                } else {
                    theme::color::WEAK
                }))
                .selected(snap);
                if ui.add(magnet).on_hover_text(if snap { s.snap_on } else { s.snap_off }).clicked() {
                    self.timeline.snap = !snap;
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        if self.status.is_empty() {
            self.status = s.ready.to_string();
        }
        ui.horizontal_centered(|ui| {
            let warn =
                ["cannot", "no sound", "failed", "refused", "error", ": "].iter().any(|w| self.status.contains(w))
                    && !self.status.ends_with(": done");
            let (glyph, tint) =
                if warn { (icon::WARNING, theme::color::WARN) } else { (icon::DONE, theme::color::WEAK) };
            ui.label(egui::RichText::new(format!("{glyph}  {}", self.status)).small().color(tint));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let shown = match &self.gpu {
                    Some(g) => format!("{}  {}", icon::SCREEN, fill(s.gpu_preview, &[&g.compositor.adapter()])),
                    None => format!("{}  {}", icon::SCREEN, s.cpu_preview),
                };
                ui.label(egui::RichText::new(shown).small().color(theme::color::WEAK));
            });
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let p = self.project();
        let s = self.s();
        let Some(id) = self.selected.clone() else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(icon::FILM).size(28.0).color(theme::color::LINE));
                ui.label(egui::RichText::new(s.select_hint).color(theme::color::WEAK));
            });
            ui.add_space(16.0);
            theme::section(ui, s.shortcuts, |ui| {
                egui::Grid::new("keys").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (k, what) in s.keys {
                        ui.label(egui::RichText::new(k).monospace().color(theme::color::TEXT));
                        ui.label(egui::RichText::new(what).color(theme::color::WEAK));
                        ui.end_row();
                    }
                });
            });
            return;
        };
        let Some((_, clip)) = p.clip(&id) else {
            self.selected = None;
            return;
        };
        let has_sound = match &clip.content {
            ClipContent::Media { media, .. } => p.media.get(media).is_some_and(|m| m.kind != MediaKind::Image),
            _ => false,
        };
        let current = Inspector {
            clip: id.clone(),
            x: clip.transform.x,
            y: clip.transform.y,
            scale: clip.transform.scale,
            rotation: clip.transform.rotation,
            opacity: clip.opacity,
            color: match &clip.content {
                ClipContent::Text { color, .. } | ClipContent::Solid { color } => Some(*color),
                ClipContent::Media { .. } => None,
            },
            volume: has_sound.then_some(clip.volume_db),
            title: match &clip.content {
                ClipContent::Text { text, size, style, .. } => Some(Title {
                    text: text.clone(),
                    size: *size,
                    bold: style.weight >= 600,
                    outline: style.outline.is_some(),
                    boxed: style.background.is_some(),
                }),
                _ => None,
            },
        };
        if self.inspector.as_ref().is_none_or(|i| i.clip != id) {
            self.inspector = Some(current.clone());
        }
        let (glyph, kind) = match &clip.content {
            ClipContent::Media { media, .. } => match p.media.get(media).map(|m| m.kind) {
                Some(MediaKind::Image) => (icon::IMAGE, s.kind_picture),
                Some(MediaKind::Audio) => (icon::MUSIC, s.kind_sound),
                _ => (icon::VIDEO, s.kind_video),
            },
            ClipContent::Text { .. } => (icon::TEXT, s.kind_title),
            ClipContent::Solid { .. } => (icon::COLOUR, s.kind_colour),
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(glyph).size(18.0).color(theme::color::ACCENT));
            ui.label(egui::RichText::new(kind).heading().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&id).monospace().color(theme::color::WEAK));
            });
        });
        ui.label(
            egui::RichText::new(format!(
                "{} → {}  ·  {}",
                theme::timecode(clip.start.seconds()),
                theme::timecode(clip.end().seconds()),
                theme::timecode(clip.duration.seconds())
            ))
            .monospace()
            .small()
            .color(theme::color::WEAK),
        );
        ui.add_space(6.0);
        let edit = self.inspector.as_mut().expect("set above");
        let mut commit = false;
        // a value is committed when its drag ends or it is typed in
        let row = |ui: &mut egui::Ui,
                   name: &str,
                   v: &mut f32,
                   speed: f64,
                   range: std::ops::RangeInclusive<f32>,
                   suffix: &str| {
            ui.label(egui::RichText::new(name).color(theme::color::WEAK));
            let r = ui.add_sized(
                [ui.available_width(), 24.0],
                egui::DragValue::new(v).speed(speed).range(range).suffix(suffix),
            );
            ui.end_row();
            r.drag_stopped() || (r.changed() && !r.dragged())
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(t) = &mut edit.title {
                theme::section(ui, s.sec_text, |ui| {
                    let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY));
                    commit |= r.lost_focus();
                    egui::Grid::new("title").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        commit |= row(ui, s.size, &mut t.size, 1.0, 8.0..=600.0, " px");
                    });
                    ui.horizontal(|ui| {
                        commit |= ui.toggle_value(&mut t.bold, s.bold).changed();
                        commit |= ui.toggle_value(&mut t.outline, s.outline).changed();
                        commit |= ui.toggle_value(&mut t.boxed, s.boxed).changed();
                    });
                });
                ui.add_space(6.0);
            }
            if let Some(c) = &mut edit.color {
                theme::section(ui, s.sec_colour, |ui| {
                    let mut rgba = egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                    let r =
                        egui::color_picker::color_edit_button_rgba(ui, &mut rgba, egui::color_picker::Alpha::OnlyBlend);
                    if r.changed() {
                        *c = rgba.to_rgba_unmultiplied();
                    }
                });
                ui.add_space(6.0);
            }
            theme::section(ui, s.sec_transform, |ui| {
                egui::Grid::new("transform").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    commit |= row(ui, s.pos_x, &mut edit.x, 0.005, -2.0..=2.0, "");
                    commit |= row(ui, s.pos_y, &mut edit.y, 0.005, -2.0..=2.0, "");
                    commit |= row(ui, s.scale, &mut edit.scale, 0.01, 0.0..=10.0, "×");
                    commit |= row(ui, s.rotation, &mut edit.rotation, 0.5, -360.0..=360.0, "°");
                    commit |= row(ui, s.opacity, &mut edit.opacity, 0.01, 0.0..=1.0, "");
                });
            });
            if let Some(v) = &mut edit.volume {
                ui.add_space(6.0);
                theme::section(ui, s.sec_sound, |ui| {
                    egui::Grid::new("sound").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        commit |= row(ui, s.volume, v, 0.2, -60.0..=12.0, " dB");
                    });
                });
            }
        });
        // a colour is committed once the pointer lets go of the picker
        let picked = self.inspector.as_ref().is_some_and(|e| e.color != current.color);
        commit |= picked && !ui.ctx().egui_is_using_pointer();
        if commit && self.inspector.as_ref() != Some(&current) {
            let e = self.inspector.clone().expect("set");
            let mut params =
                json!({"clip": id, "x": e.x, "y": e.y, "scale": e.scale, "rotation": e.rotation, "opacity": e.opacity});
            if let Some(c) = e.color {
                params["color"] = json!(c);
            }
            if let Some(v) = e.volume {
                params["volumeDb"] = json!(v);
            }
            if let Some(t) = e.title {
                let was = current.title.as_ref();
                params["text"] = json!(t.text);
                params["size"] = json!(t.size);
                params["weight"] = json!(if t.bold { 700 } else { 400 });
                if was.is_some_and(|w| w.outline != t.outline) {
                    params["outline"] = if t.outline { json!({}) } else { Value::Null };
                }
                if was.is_some_and(|w| w.boxed != t.boxed) {
                    params["background"] = if t.boxed { json!([0.0, 0.0, 0.0, 0.55]) } else { Value::Null };
                }
            }
            if self.execute("clip.set", params).is_err() {
                self.inspector = Some(current);
            }
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let p = self.project();
        let frame = p.canvas.fps.frame_at(self.playhead);
        let key = (self.session.revision(), frame, self.remote.as_ref().map_or(0, remote::Remote::media_epoch));
        let changed = self.preview_key != Some(key);
        self.preview_key = Some(key);
        let long = p.canvas.width.max(p.canvas.height) as f32;
        let scale = (PREVIEW_LONG_SIDE / long).min(1.0);
        if changed && let Some(g) = &mut self.gpu {
            // an empty timeline does not compile: the CPU path below draws the background
            g.texture = None;
            if let Ok(spec) = compile(&p, &CompileOptions { encode: false }) {
                match g.compositor.render(&spec, p.canvas.fps.snap(self.playhead), scale, &*self.media, Quality::Draft)
                {
                    Ok(_report) => g.show(),
                    Err(e) => self.status = fill(self.s().preview_failed, &[&e]),
                }
            }
        }
        if let Some((id, [w, h])) = self.gpu.as_ref().and_then(|g| g.texture) {
            let avail = ui.available_size();
            let (w, h) = (w as f32, h as f32);
            let s = (avail.x / w).min(avail.y / h);
            ui.centered_and_justified(|ui| {
                ui.add(egui::Image::new((id, egui::vec2(w * s, h * s))));
            });
            return;
        }
        if changed || self.preview.is_none() {
            let image = match compile(&p, &CompileOptions { encode: false }) {
                Ok(spec) => {
                    match render_frame(&spec, p.canvas.fps.snap(self.playhead), scale, &*self.media, Quality::Draft) {
                        Ok((img, _report)) => Some(img),
                        Err(e) => {
                            self.status = fill(self.s().preview_failed, &[&e]);
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

/// Where the picked language is kept between runs.
const LANG_KEY: &str = "reelforge.lang";

impl eframe::App for EditorApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.lang_chosen {
            storage.set_string(LANG_KEY, self.lang.code().to_string());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.lang_unsaved)
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
        }
        if !self.fonts {
            self.fonts = true;
            let _ = self.window.set(ctx.clone());
            #[cfg(not(target_arch = "wasm32"))]
            self.sound.prepare();
            // another client's edit wakes the window, which otherwise sleeps until input
            let repaint = ctx.clone();
            self.session.on_change(move |c, _| {
                if c.by != "editor" {
                    repaint.request_repaint();
                }
            });
            theme::install(&ctx);
        }
        let change = self.session.last_change();
        if change.revision != self.seen {
            self.seen = change.revision;
            if change.by != "editor" {
                let who = if change.by.starts_with("agent") {
                    icon::AGENT
                } else if change.by == "web" {
                    icon::WEB
                } else {
                    icon::CONNECTED
                };
                let what = match self.lang.command(&change.command) {
                    "" => change.command.as_str(),
                    name => name,
                };
                self.status = format!("{who}  {} · {what}", change.by);
                // a clip it removed may have been the selection
                if let Some(sel) = &self.selected
                    && !self.project().tracks.iter().any(|t| t.clips.iter().any(|c| &c.id == sel))
                {
                    self.selected = None;
                    self.inspector = None;
                }
            }
        }
        if let Some(remote) = self.remote.clone() {
            if let Some(wait) = remote.tick(ui.input(|i| i.time)) {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.05)));
            }
            for up in remote.take_uploaded() {
                if let Ok(v) = self.execute("media.import", up.import) {
                    self.place(&v);
                    self.status = fill(self.s().imported, &[&up.name]);
                }
            }
            if let Some(note) = remote.take_notes().pop() {
                self.status = note.text(self.lang);
            }
        }
        self.take_dropped_files(&ctx);
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx
                .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("reelforge drop overlay")));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                self.s().drop_overlay,
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
        }
        if self.playing {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            self.advance(dt as f64);
            ui.ctx().request_repaint();
        }
        self.shortcuts(&ctx);
        let bar = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6));
        egui::Panel::top("top").exact_size(42.0).frame(bar(theme::color::PANEL)).show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(bar(theme::color::STAGE))
            .show(ui, |ui| self.status_bar(ui));
        let p = self.project();
        let (playing, lang) = (self.playing, self.lang);
        let out = egui::Panel::bottom("timeline")
            .resizable(true)
            .default_size(280.0)
            .frame(egui::Frame::new().fill(theme::color::PANEL))
            .show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| self.timeline_bar(ui));
                self.timeline.show(ui, &p, self.playhead, self.selected.as_deref(), playing, lang)
            })
            .inner;
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(290.0)
            .frame(egui::Frame::new().fill(theme::color::PANEL).inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| self.inspector(ui));
        egui::Panel::bottom("transport")
            .exact_size(46.0)
            .frame(bar(theme::color::STAGE).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ui, |ui| self.transport(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::color::STAGE).inner_margin(egui::Margin::same(16)))
            .show(ui, |ui| self.preview(ui));

        if let Some(t) = out.seek {
            self.seek(t);
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
        if let Some((clip, start, end)) = out.trimmed {
            let mut params = json!({"clip": clip});
            if let Some(t) = start {
                params["start"] = json!(t);
            }
            if let Some(t) = end {
                params["end"] = json!(t);
            }
            let _ = self.execute("clip.trim", params);
        }
        if let Some((track, key, on)) = out.toggled {
            let _ = self.execute("track.set", json!({"track": track, key: on}));
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
