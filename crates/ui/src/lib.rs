//! The editor interface: egui, drawn by eframe's wgpu renderer on the desktop and (WebGPU, or
//! WebGL2 as a fallback) in the browser.
//!
//! The interface decides nothing about an edit. Buttons, drags and the inspector all end in
//! [`EditorApp::execute`], which runs an engine command, exactly as the CLI or an agent would; the
//! interface only reads the project snapshot back to draw it. The preview is a RenderSpec drawn by
//! `reelforge-render`, the same path an export takes.
//!
//! Given eframe's wgpu device ([`EditorApp::with_gpu`]), the preview is drawn by the wgpu
//! compositor into a texture egui shows as it is, so a frame never leaves the GPU; WebGL2 too, when
//! the browser can draw into float pictures (`EXT_color_buffer_float`, almost all can). Without
//! one it is drawn on the CPU and uploaded.
//!
//! Files dropped on the window are imported (`media.import`) and placed at the playhead
//! (`clip.add`); files picked with Import in the asset panel on the left (`assets`) only join the
//! project's media, to be dragged onto the timeline from there. Export writes an MP4: on the
//! desktop on a background thread, in the browser with its own (hardware) encoder, as a download.
//!
//! [`EditorApp::join`] edits a session a server holds instead, with agents and other editors: see
//! [`remote`].
//!
//! An editor with neither a file nor a server keeps its project itself, as one of its drafts
//! ([`EditorApp::with_recovery`], `drafts`). A narrow window (a phone held upright) shows one panel
//! at a time under the preview.

mod ahead;
mod assets;
mod audio;
mod captions;
mod curve;
mod drafts;
mod export_job;
pub mod i18n;
mod looks;
#[cfg(target_arch = "wasm32")]
mod picker;
mod record;
mod remix;
pub mod remote;
mod spoken;
mod stage;
mod strips;
mod theme;
mod timeline;
#[cfg(target_arch = "wasm32")]
mod webcodecs;

use std::sync::{Arc, Mutex};

use eframe::egui;
use i18n::{Lang, Strings, fill};
use reelforge_compile::{CompileOptions, compile};
use reelforge_engine::{Session, SharedSession};
use reelforge_media::Footage;
use reelforge_model::{
    Adjust, AnimKind, BlendMode, ClipContent, ClipTransform, CurvePreset, FilterKind, LoopKind, Mask, MaskKind,
    MediaKind, Project, Prop, TrackKind, TransitionKind, VoiceKind,
};
use reelforge_render::{Quality, render_frame};
use reelforge_time::Tick;
use serde_json::{Value, json};
use theme::icon;

/// Longest side of the preview render on the CPU (no usable GPU), in pixels: the CPU cannot
/// keep up with more.
const PREVIEW_LONG_SIDE: f32 = 640.0;
/// Longest side of a playing preview in Smooth quality, in pixels.
const SMOOTH_LONG_SIDE: f32 = 720.0;

/// The render scale a preview shown `on_screen` pixels long (its longest side) draws at, for a
/// canvas `long` pixels long: as sharp as the screen shows it, never above the canvas, and lower
/// while playing in Smooth quality. Rounded to 1/32 so that resizing the window by a few points
/// does not draw again.
fn preview_scale(long: f32, on_screen: f32, gpu: bool, playing: bool, full: bool) -> f32 {
    let mut side = on_screen;
    if !gpu {
        side = side.min(PREVIEW_LONG_SIDE);
    } else if playing && !full {
        side = side.min(SMOOTH_LONG_SIDE);
    }
    let scale = (side / long.max(1.0)).clamp(0.05, 1.0);
    ((scale * 32.0).ceil() / 32.0).min(1.0)
}

/// What the preview draws that the project does not have yet: a gesture's placement so far (`x`,
/// `y`, scale, rotation as bits), a mask being dragged (centre, size, rotation), and whether the
/// crop is lifted to show the whole picture (crop mode).
type Moving = Option<[u32; 10]>;

/// A compiled edit, shared.
type Spec = Arc<reelforge_renderspec::RenderSpec>;

/// What the preview was last drawn for: revision, frame, remote media epoch, gesture, playing,
/// render scale (bits).
type PreviewKey = (u64, i64, u64, Moving, bool, u32);

/// A file the browser finished reading: its name, its bytes, and whether it goes on the timeline
/// too (a drop) or only into the project's media (picked with Import).
pub(crate) type Arrival = (String, Result<Vec<u8>, String>, bool);

/// The editor window's state.
pub struct EditorApp {
    session: Arc<SharedSession>,
    /// The last revision drawn, to notice changes other clients made.
    seen: u64,
    media: Arc<Footage>,
    /// Files dropped in the browser, read asynchronously: `(name, bytes)`.
    arrivals: Arc<Mutex<Vec<Arrival>>>,
    export: Option<export_job::ExportJob>,
    /// The export settings are showing.
    export_open: bool,
    /// The panel on the left: media, sounds, titles.
    assets: assets::AssetPanel,
    /// The SubRip file being picked is a second language's (asked from the captions list).
    second_captions: bool,
    /// The timelines window is showing, and the name being typed for the current timeline.
    timelines_open: bool,
    /// The window that trims speech.
    spoken: spoken::Spoken,
    timeline_name: String,
    /// A voice-over being recorded, and how many were.
    recorder: Option<record::Recorder>,
    takes: u32,
    #[cfg(not(target_arch = "wasm32"))]
    export_dir: std::path::PathBuf,
    playhead: Tick,
    playing: bool,
    /// Preview sound, and the project revision its mix was compiled from.
    sound: audio::Playback,
    /// Whether moving the playhead while paused plays a moment of sound.
    scrub_sound: bool,
    sound_spec: Option<(u64, Arc<reelforge_renderspec::RenderSpec>)>,
    selected: Option<String>,
    /// The other selected clips, besides `selected` (which the inspector and preview show).
    also: Vec<String>,
    /// What Copy or Cut took (`clip.copy`'s clipboard).
    clipboard: Option<Value>,
    /// How fast, and which way, the preview plays: 1 is normal (with sound); J and L change it.
    rate: f64,
    /// The range set with I and O, and whether playing loops (over it, or the whole timeline).
    range_in: Option<Tick>,
    range_out: Option<Tick>,
    looping: bool,
    status: String,
    preview: Option<egui::TextureHandle>,
    preview_key: Option<PreviewKey>,
    /// The RenderSpec the preview draws, and the revision and gesture it was compiled for: a
    /// playing preview compiles once, not every frame.
    preview_spec: Option<((u64, Moving), Option<Spec>)>,
    /// Decodes the frames a playing preview reaches next.
    ahead: ahead::Ahead,
    /// Full quality: a playing preview stays as sharp as a paused one (else it draws smaller).
    full_quality: bool,
    /// The preview's zoom (1 fits the frame in the panel) and how far it is panned, in points.
    zoom: f32,
    pan: egui::Vec2,
    /// A title whose words are being edited in the preview, and the words so far.
    editing: Option<(String, String)>,
    /// Only the preview shows (the panels and timeline are hidden).
    preview_only: bool,
    /// The preview quality was switched and is not written to storage yet.
    quality_unsaved: bool,
    /// Where the visible layers land in the preview's frame, bottom first.
    outlines: Vec<(String, reelforge_render::Outline)>,
    /// A drag on the preview in progress.
    gesture: Option<stage::Gesture>,
    /// The clip whose crop is being set in the preview (crop mode).
    cropping: Option<String>,
    gpu: Option<GpuPreview>,
    /// Whether the bundled Chinese face has been given to egui.
    fonts: bool,
    timeline: timeline::TimelineView,
    inspector: Option<Inspector>,
    /// The project revision and playhead the inspector's values were read at.
    inspector_seen: (u64, Tick),
    /// Pictures of the title templates, made when first shown.
    looks: Option<Vec<(&'static str, egui::TextureHandle)>>,
    /// The server whose session this editor mirrors, when it joined one.
    remote: Option<remote::Remote>,
    /// The interface's language.
    lang: Lang,
    /// Whether the person picked the language (then it is remembered; otherwise the system's is used).
    lang_chosen: bool,
    /// A pick not written to storage yet (the window may sleep past eframe's autosave).
    lang_unsaved: bool,
    /// Whether the project lives only in this editor (no file, no server), so it is kept in
    /// eframe's storage as it changes and brought back after a crash or a reload
    /// ([`EditorApp::with_recovery`]).
    recovery: bool,
    /// The revision last kept there, and when (in the window's seconds) the next keep is due.
    kept: (u64, Option<f64>),
    /// The drafts of such an editor (the project is the one being edited), and the page that
    /// lists them.
    drafts: Option<drafts::Drafts>,
    /// What was asked of the drafts; done at the start of the next frame, where storage is at hand.
    draft_ask: Option<drafts::Ask>,
    /// Stands in for eframe's storage when it has none: drafts then last as long as the window.
    spare: std::collections::HashMap<String, String>,
    /// The window is too narrow for the panels to stand side by side ([`NARROW_BELOW`]); then one
    /// of them shows under the preview.
    narrow: bool,
    sheet: Sheet,
    /// The clip the next picture or video picked takes the place of (Replace in a clip's menu).
    into: Option<String>,
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
    /// Picture clips: the in and out animations, with their lengths in seconds.
    anim: Option<[Option<(AnimKind, f32)>; 2]>,
    /// With `anim`: the animation that repeats, with one round in seconds.
    looped: Option<(LoopKind, f32)>,
    /// Clips on a video track: the transition into it with its length in seconds, and whether a
    /// clip ends where it starts (so one can play).
    transition: Option<(Option<(TransitionKind, f32)>, bool)>,
    /// Clips with sound: fade-in and fade-out lengths in seconds.
    fades: Option<[f32; 2]>,
    /// Video and sound clips: speed, pitch, direction and curve.
    speed: Option<SpeedEdit>,
    /// Picture clips: filter, lookup table and adjustments.
    grade: Option<GradeEdit>,
    /// Picture clips: blend, mask, and for videos and pictures crop and mirror.
    look: Option<LookEdit>,
    /// Picture clips: the effects that play on them (or, on an adjustment clip, on what is below).
    fx: Option<Vec<reelforge_model::Fx>>,
    /// An adjustment clip: nothing to place, only effects and colour.
    adjustment: bool,
    /// Clips whose sound plays: a changed voice, a moved pitch, noise taken out.
    voice: Option<VoiceEdit>,
}

/// A sounding clip's voice, as the inspector edits it.
#[derive(Clone, PartialEq)]
struct VoiceEdit {
    kind: Option<VoiceKind>,
    /// Semitones, on top of the voice's own.
    pitch: f32,
    /// How much noise is taken out, in percent, when any is.
    denoise: Option<f32>,
}

/// A picture clip's crop, mirror, blend and mask, as the inspector edits it.
#[derive(Clone, PartialEq)]
struct LookEdit {
    /// Crop and mirror apply (a video or picture clip).
    media: bool,
    /// Left, top, right, bottom, in percent.
    crop: [f32; 4],
    flip: [bool; 2],
    blend: BlendMode,
    mask: Option<Mask>,
    /// The canvas blur behind it, when it can have one (a video or picture on a video track).
    backdrop: Option<Option<f32>>,
    /// The colour keyed out of it, when it can have one (the same clips).
    chroma: Option<Option<reelforge_model::ChromaKey>>,
}

/// A picture clip's colour, as the inspector edits it.
#[derive(Clone, PartialEq)]
struct GradeEdit {
    filter: Option<FilterKind>,
    filter_strength: f32,
    lut: Option<String>,
    lut_strength: f32,
    adjust: [f32; 9],
}

/// How a video or sound clip runs, as the inspector edits it.
#[derive(Clone, PartialEq)]
struct SpeedEdit {
    speed: f32,
    keep_pitch: bool,
    reversed: bool,
    /// `None`: steady; `Some(None)`: a curve that is not a preset.
    curve: Option<Option<CurvePreset>>,
    /// A freeze frame (speed 0).
    frozen: bool,
    /// Video, which can be frozen.
    video: bool,
}

/// Whether a property has a keyframe at the playhead.
#[derive(Copy, Clone, PartialEq)]
enum KeyMark {
    /// No keyframes.
    Off,
    /// Keyframes, none here.
    Elsewhere,
    /// A keyframe here.
    Here,
}

/// A title's own values in the inspector.
#[derive(Clone, PartialEq)]
struct Title {
    text: String,
    size: f32,
    bold: bool,
    outline: bool,
    boxed: bool,
    shadow: bool,
    line_boxes: bool,
    /// The font the title is set in (a media id); the built-in face when `None`.
    font: Option<String>,
    /// Space after each character and between lines, in hundredths of the size.
    tracking: f32,
    leading: f32,
    italic: bool,
    underline: bool,
}

impl EditorApp {
    /// An editor on `session`.
    pub fn new(session: Session) -> EditorApp {
        EditorApp::shared(SharedSession::new(session))
    }

    /// An editor on a session other clients edit too (agents through a server the app hosts). Their
    /// changes show as they commit.
    pub fn shared(session: Arc<SharedSession>) -> EditorApp {
        let media = Arc::new(Footage::new());
        EditorApp {
            seen: session.revision(),
            session,
            ahead: ahead::Ahead::new(media.clone()),
            media,
            arrivals: Arc::default(),
            export: None,
            export_open: false,
            assets: assets::AssetPanel::default(),
            second_captions: false,
            timelines_open: false,
            spoken: spoken::Spoken::default(),
            timeline_name: String::new(),
            recorder: None,
            takes: 0,
            #[cfg(not(target_arch = "wasm32"))]
            export_dir: std::env::current_dir().unwrap_or_default(),
            playhead: Tick::ZERO,
            playing: false,
            sound: audio::Playback::default(),
            scrub_sound: true,
            sound_spec: None,
            selected: None,
            also: Vec::new(),
            clipboard: None,
            rate: 1.0,
            range_in: None,
            range_out: None,
            looping: false,
            status: String::new(),
            preview: None,
            preview_key: None,
            preview_spec: None,
            full_quality: false,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            editing: None,
            preview_only: false,
            quality_unsaved: false,
            outlines: Vec::new(),
            gesture: None,
            cropping: None,
            gpu: None,
            fonts: false,
            timeline: timeline::TimelineView::default(),
            inspector: None,
            inspector_seen: (0, Tick::ZERO),
            looks: None,
            remote: None,
            lang: Lang::detect(),
            lang_chosen: false,
            lang_unsaved: false,
            recovery: false,
            kept: (0, None),
            drafts: None,
            draft_ask: None,
            spare: Default::default(),
            narrow: false,
            sheet: Sheet::default(),
            into: None,
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
    /// device, to draw the preview with the wgpu compositor when it has what that needs
    /// ([`GpuCompositor::supports`](reelforge_render::gpu::GpuCompositor::supports): WebGPU,
    /// Vulkan, Metal, DX12, and WebGL2 with float render targets).
    pub fn with_gpu(mut self, cc: &eframe::CreationContext<'_>) -> EditorApp {
        if let Some(lang) = cc.storage.and_then(|s| s.get_string(LANG_KEY)).as_deref().and_then(Lang::from_code) {
            self.lang = lang;
            self.lang_chosen = true;
        }
        self.full_quality = cc.storage.and_then(|s| s.get_string(QUALITY_KEY)).as_deref() == Some("full");
        if let Some(rs) = &cc.wgpu_render_state {
            let info = rs.adapter.get_info();
            if reelforge_render::gpu::GpuCompositor::supports(&rs.adapter) {
                let compositor = reelforge_render::gpu::GpuCompositor::new(rs.device.clone(), rs.queue.clone(), &info);
                // the GPU converts video pictures itself: read ahead only decodes
                self.ahead.pictures(false);
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

    /// Keeps the project in eframe's storage (a file in the app's data folder on the desktop, the
    /// page's local storage in the browser) a moment after each change, and takes back the one kept
    /// there last, if any: an editor whose project has no file and no server loses nothing to a
    /// crash, a closed window or a reload. Media the browser handed over is not kept; dropping the
    /// same files in again brings it back. Call after [`EditorApp::with_gpu`].
    ///
    /// What is kept is a draft, one of as many as the person starts: the drafts page (the button
    /// beside the name in the top bar) lists them, to open another or start a new one. The editor
    /// starts on the draft edited last.
    pub fn with_recovery(self, cc: &eframe::CreationContext<'_>) -> EditorApp {
        self.with_drafts(&|key| cc.storage.and_then(|s| s.get_string(key)))
    }

    /// [`EditorApp::with_recovery`] with what reads storage.
    fn with_drafts(mut self, get: &dyn Fn(&str) -> Option<String>) -> EditorApp {
        let drafts = drafts::Drafts::load(get(drafts::INDEX_KEY));
        // the draft being edited; from before there were drafts, the one project kept
        let kept = drafts.current().and_then(|d| get(&drafts::project_key(&d.id))).or_else(|| get(RECOVERY_KEY));
        self.drafts = Some(drafts);
        self.recover(kept)
    }

    /// [`EditorApp::with_recovery`] with the project storage held.
    fn recover(mut self, kept: Option<String>) -> EditorApp {
        self.recovery = true;
        self.drafts.get_or_insert_with(Default::default);
        if let Some(project) = kept.and_then(|text| serde_json::from_str::<reelforge_model::Project>(&text).ok())
            && project.check().is_ok()
        {
            let missing = recovered_missing(&project);
            // the same session (a server the desktop app hosts holds it too), now on the kept project
            self.session.adopt(project, "project.recover", "editor");
            self.seen = self.session.revision();
            let s = self.s();
            self.status = s.recovered.to_string();
            if missing > 0 {
                self.status.push_str(&fill(s.draft_offline, &[&missing]));
            }
        }
        self.kept = (self.session.revision(), None);
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
        // a call from outside the window (the browser API) shows at once, not at the next input
        if let Some(ctx) = self.window.get() {
            ctx.request_repaint();
        }
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

    /// Runs a tool that listens to the media before it edits (`reelforge_engine::listen`): what it
    /// plans is one edit, and what it found, or why it could not, goes to the status bar.
    pub fn listen(&mut self, tool: &str, params: &Value) {
        let name = self.lang.command(tool);
        let plan = match reelforge_engine::listen::plan(tool, &self.project(), &self.media, params) {
            Ok(plan) => plan,
            Err(e) => {
                self.status = format!("{name}: {e}");
                return;
            }
        };
        if !plan.steps.is_empty() && self.execute("edit.batch", json!({"steps": plan.steps, "label": name})).is_err() {
            return;
        }
        let s = self.s();
        let found = &plan.found;
        self.status = match tool {
            "audio.beats" if found.get("bpm").is_some() => fill(s.beats_found, &[&found["bpm"], &found["beats"]]),
            "audio.normalize" => {
                let n = found["clips"].as_array().map_or(0, |l| l.iter().filter(|c| !c["lufs"].is_null()).count());
                fill(s.loudness_done, &[&n])
            }
            "audio.duck" => fill(s.duck_done, &[&found["spans"]]),
            "audio.denoise" if found.get("noiseDb").is_some() => fill(s.denoise_done, &[&found["noiseDb"]]),
            _ => fill(s.done, &[&name]),
        };
    }

    /// Keys the backdrop out of clip `id`: the colour most of the frame's border has where the
    /// playhead is (the clip's first frame when it is elsewhere). When no colour is found, a clip
    /// with no key yet gets a green one to adjust, and one with a key keeps it.
    pub fn key_out(&mut self, id: &str) {
        let p = self.project();
        let Some((_, clip)) = p.clip(id) else { return };
        let at = p.canvas.fps.snap(self.playhead);
        let at = if at >= clip.start && at < clip.end() { at } else { clip.start };
        let name = self.lang.command("clip.setChromaKey");
        let s = self.s();
        match reelforge_engine::listen::plan("clip.pickChromaKey", &p, &self.media, &json!({"clip": id, "at": at})) {
            Ok(plan) => {
                if self.execute("edit.batch", json!({"steps": plan.steps, "label": name})).is_ok() {
                    self.status = s.chroma_found.to_string();
                }
            }
            Err(_) if clip.look.chroma.is_none() => {
                let green = reelforge_model::ChromaKey::GREEN;
                if self.execute("clip.setChromaKey", json!({"clip": id, "color": green})).is_ok() {
                    self.status = s.chroma_guess.to_string();
                }
            }
            Err(_) => self.status = s.chroma_none.to_string(),
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
            let end = self.project().duration();
            if self.rate > 0.0 && self.playhead >= end {
                self.playhead = Tick::ZERO;
            } else if self.rate < 0.0 && self.playhead <= Tick::ZERO {
                self.playhead = end;
            }
            if self.rate == 1.0 {
                self.sound.start(self.playhead);
                if let Some(why) = self.sound.take_problem() {
                    self.status = fill(self.s().no_sound, &[&why]);
                }
            } else {
                // faster or backwards plays without sound
                self.sound.stop();
            }
        } else {
            self.sound.stop();
            self.media.clear_ahead();
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
            None => self.playhead + Tick::from_seconds(dt * self.rate),
        };
        let end = p.duration();
        if self.looping && self.rate > 0.0 {
            let (a, b) = self.range().unwrap_or((Tick::ZERO, end));
            if self.playhead >= b {
                self.seek(a);
                return;
            }
        }
        if self.playhead >= end || self.playhead <= Tick::ZERO && self.rate < 0.0 {
            self.playhead = self.playhead.max(Tick::ZERO).min(end);
            self.set_playing(false);
            self.rate = 1.0;
            return;
        }
        if let Some((_, spec)) = &self.sound_spec {
            self.sound.fill(spec, &self.media);
        }
    }

    /// Keeps the project in `store` as the draft being edited; the first time, that starts the
    /// draft.
    fn keep_draft(&mut self, store: &mut dyn drafts::Store) {
        let (project, s) = (self.project(), self.s());
        let Some(drafts) = &mut self.drafts else { return };
        if drafts.current().is_none() {
            let name = drafts.free_name(s.draft_name);
            drafts.start(name, drafts::now());
        }
        drafts.keep(store, &project, drafts::now());
        // what an editor from before there were drafts kept is this draft now
        if store.get(RECOVERY_KEY).is_some() {
            store.remove(RECOVERY_KEY);
        }
        self.kept = (self.session.revision(), None);
    }

    /// A small picture of the project for its draft's card: the cover's frame, or the first one.
    fn draft_cover(&self) -> Option<Vec<u8>> {
        let p = self.project();
        if p.duration() <= Tick::ZERO {
            return None;
        }
        let spec = compile(&p, &CompileOptions { encode: false }).ok()?;
        let side = p.canvas.width.max(p.canvas.height).max(1);
        let scale = (drafts::COVER_SIDE as f32 / side as f32).min(1.0);
        let picture = reelforge_media::still(&spec, &self.media, p.cover.unwrap_or(Tick::ZERO), scale).ok()?;
        reelforge_media::encode_still(&picture, reelforge_media::StillFormat::Jpeg).ok()
    }

    /// Puts the editor on `project`, a draft just opened or started: at its beginning, with nothing
    /// selected and no history.
    fn edit_draft(&mut self, project: reelforge_model::Project) {
        self.set_playing(false);
        self.session.adopt(project, "project.open", "editor");
        self.seen = self.session.revision();
        self.kept = (self.seen, None);
        self.selected = None;
        self.also.clear();
        self.inspector = None;
        self.editing = None;
        self.cropping = None;
        self.gesture = None;
        (self.range_in, self.range_out) = (None, None);
        self.playhead = Tick::ZERO;
    }

    /// Does what was asked of the drafts, which are kept in `store`.
    fn drafts_do(&mut self, ask: drafts::Ask, store: &mut dyn drafts::Store) {
        use drafts::Ask;
        if self.drafts.is_none() {
            return;
        }
        let s = self.s();
        let now = drafts::now();
        // a draft just started, to edit
        let start = |app: &mut EditorApp, store: &mut dyn drafts::Store, name: &str| {
            let drafts = app.drafts.as_mut().expect("checked");
            let name = drafts.free_name(name);
            drafts.start(name.clone(), now);
            app.edit_draft((*Session::default().project()).clone());
            app.keep_draft(store);
            name
        };
        match ask {
            Ask::Home => {
                self.set_playing(false);
                self.keep_draft(store);
                // the card's picture is of the draft as it is left; one made while files are
                // missing does not replace a whole one
                let cover = self.draft_cover();
                let partial = self.assets.any_offline(&self.project());
                let drafts = self.drafts.as_mut().expect("checked");
                if !(partial && drafts.has_cover(store)) {
                    drafts.keep_cover(store, cover.as_deref());
                }
                drafts.home = true;
            }
            Ask::Back => self.drafts.as_mut().expect("checked").home = false,
            Ask::Open(id) => {
                self.keep_draft(store);
                let drafts = self.drafts.as_mut().expect("checked");
                let name = drafts.name_of(&id);
                match drafts.open(store, &id) {
                    Some(project) => {
                        drafts.home = false;
                        self.edit_draft(project);
                        self.status = fill(s.draft_opened, &[&name]);
                    }
                    None => self.status = fill(s.draft_unreadable, &[&name]),
                }
            }
            Ask::New => {
                self.keep_draft(store);
                let name = start(self, store, s.draft_name);
                self.drafts.as_mut().expect("checked").home = false;
                self.status = fill(s.draft_started, &[&name]);
            }
            Ask::Template(id) => {
                // a new draft named after the template, with the template built in it
                self.keep_draft(store);
                start(self, store, remix::name(id, s));
                self.drafts.as_mut().expect("checked").home = false;
                self.use_template(id);
                self.sheet = Sheet::Media;
                self.keep_draft(store);
            }
            Ask::Rename(id, name) => self.drafts.as_mut().expect("checked").rename(store, &id, &name),
            Ask::Duplicate(id) => {
                let drafts = self.drafts.as_mut().expect("checked");
                let name = fill(s.draft_copy_name, &[&drafts.name_of(&id)]);
                drafts.duplicate(store, &id, name, now);
            }
            Ask::Delete(id) => {
                let drafts = self.drafts.as_mut().expect("checked");
                let name = drafts.name_of(&id);
                let edited = drafts.current().is_some_and(|d| d.id == id);
                drafts.delete(store, &id);
                if edited {
                    // the editor is always on some draft: the one changed last, or a new one
                    let next = drafts.list().first().map(|d| d.id.clone());
                    match next.and_then(|id| drafts.open(store, &id)) {
                        Some(project) => self.edit_draft(project),
                        None => {
                            start(self, store, s.draft_name);
                        }
                    }
                }
                self.status = fill(s.draft_deleted, &[&name]);
            }
        }
    }

    /// Runs `f` with where drafts are kept: eframe's storage, written out afterwards, or, when it
    /// has none, a map that lasts as long as the window.
    fn with_store(&mut self, frame: &mut eframe::Frame, f: impl FnOnce(&mut EditorApp, &mut dyn drafts::Store)) {
        match frame.storage_mut() {
            Some(storage) => {
                f(self, &mut drafts::Kept(storage));
                storage.flush();
            }
            None => {
                let mut spare = std::mem::take(&mut self.spare);
                f(self, &mut spare);
                self.spare = spare;
            }
        }
    }

    /// The drafts page, in place of the editor.
    fn drafts_page(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let s = self.s();
        let Some(mut drafts) = self.drafts.take() else { return };
        let now = drafts::now();
        let margin = if self.narrow { 14 } else { 28 };
        let ask = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::color::STAGE).inner_margin(egui::Margin::same(margin)))
            .show(ui, |ui| match frame.storage_mut() {
                Some(storage) => drafts.show(ui, s, &drafts::Kept(storage), now),
                None => drafts.show(ui, s, &self.spare, now),
            })
            .inner;
        self.drafts = Some(drafts);
        if ask.is_some() {
            self.draft_ask = ask;
            ui.ctx().request_repaint();
        }
    }

    /// Empties the project (`project.new`; undo brings it back) and starts at its beginning.
    fn new_project(&mut self) {
        if self.execute("project.new", json!({})).is_ok() {
            self.set_playing(false);
            self.selected = None;
            self.also.clear();
            self.playhead = Tick::ZERO;
            self.status = self.s().new_project_done.to_string();
        }
    }

    /// Imports a file on disk and, when `place`, places it at the playhead.
    #[cfg(not(target_arch = "wasm32"))]
    fn import_path(&mut self, path: &std::path::Path, place: bool) {
        if is_subrip(&path.to_string_lossy()) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::read(path) {
                Ok(bytes) => self.import_captions(&name, &bytes),
                Err(e) => self.status = format!("{name}: {e}"),
            }
            return;
        }
        if is_draft(&path.to_string_lossy()) {
            let folder = path.parent().map(|f| f.to_string_lossy().into_owned());
            match std::fs::read(path) {
                Ok(bytes) => self.import_draft(&bytes, folder),
                Err(e) => self.status = format!("{}: {e}", path.display()),
            }
            return;
        }
        let path = path.to_string_lossy().into_owned();
        if let Ok(v) = self.execute("media.import", json!({ "path": path })) {
            self.imported(&v, place);
        }
    }

    /// Imports a file the browser handed over and, when `place`, places it at the playhead.
    fn import_bytes(&mut self, name: &str, bytes: Vec<u8>, place: bool) {
        if is_subrip(name) {
            self.import_captions(name, &bytes);
            return;
        }
        if is_draft(name) {
            self.import_draft(&bytes, None);
            return;
        }
        if let Some(remote) = &self.remote {
            // the server keeps the file; it is imported by the server's path when it is there
            self.status = match remote.upload(name, bytes, place) {
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
        if let Some(m) = self.project().media.values().find(|m| m.sha256.as_deref() == Some(sha.as_str())) {
            if m.kind == MediaKind::Font {
                // a font the project has: the selected title is set in it
                self.imported(&json!({ "media": m }), place);
                return;
            }
            // the project has it already (taken back after a reload): its pictures are back
            self.status = fill(self.s().relinked, &[&m.name]);
            return;
        }
        let kind = match probe.kind {
            reelforge_renderspec::AssetKind::Video => "video",
            reelforge_renderspec::AssetKind::Image => "image",
            reelforge_renderspec::AssetKind::Lut => "lut",
            reelforge_renderspec::AssetKind::Font => "font",
            _ => "audio",
        };
        let mut params = json!({ "path": name, "kind": kind, "sha256": sha });
        if let Some(family) = &probe.family {
            params["name"] = json!(family);
        }
        if let Some(size) = probe.size {
            params["size"] = json!(size);
        }
        if let Some(d) = probe.duration {
            params["duration"] = json!(d);
        }
        if let Ok(v) = self.execute("media.import", params) {
            self.imported(&v, place);
        }
    }

    /// Does what the captions list asked.
    fn captions_ask(&mut self, ask: captions::Ask, ctx: &egui::Context) {
        use captions::Ask;
        let set = |clip: &str, text: &str| json!({"id": "clip.set", "params": {"clip": clip, "text": text}});
        match ask {
            Ask::Focus { clip, start } => {
                self.select(timeline::Select::Only(clip));
                if !self.playing {
                    self.seek(start);
                }
            }
            Ask::SetText { clip, text } => {
                let _ = self.execute("clip.set", json!({"clip": clip, "text": text}));
            }
            Ask::Split { clip, text, at } => {
                let steps = json!([set(&clip, &text), {"id": "captions.split", "params": {"clip": clip, "at": at}}]);
                let label = self.lang.command("captions.split");
                let _ = self.execute("edit.batch", json!({"steps": steps, "label": label}));
            }
            Ask::Merge { first, second, typed } => {
                let mut steps = Vec::new();
                if let Some((clip, text)) = typed.filter(|(_, t)| !t.trim().is_empty()) {
                    steps.push(set(&clip, &text));
                }
                steps.push(json!({"id": "captions.merge", "params": {"clips": [first, second]}}));
                let label = self.lang.command("captions.merge");
                if self.execute("edit.batch", json!({"steps": steps, "label": label})).is_ok() {
                    self.select(timeline::Select::Only(first));
                }
            }
            Ask::Delete(clip) => {
                let _ = self.execute("clip.remove", json!({"clip": clip}));
            }
            Ask::Replace { track, find, with } => {
                if let Ok(v) = self.execute("captions.replace", json!({"track": track, "find": find, "replace": with}))
                {
                    self.status = fill(self.s().cap_replaced, &[&v["count"].as_u64().unwrap_or(0)]);
                }
            }
            Ask::Restyle(from) => {
                let _ = self.execute("captions.restyle", json!({"from": from}));
            }
            Ask::Shift { track, seconds } => {
                let _ = self.execute("captions.shift", json!({"track": track, "by": {"seconds": seconds}}));
            }
            Ask::Import { second } => {
                self.second_captions = second;
                self.pick(ctx, &["srt"], false);
            }
            Ask::Export(track) => self.export_captions(&track),
        }
    }

    /// Saves a track's captions as a SubRip file: where the person says on the desktop, as a
    /// download in the browser.
    fn export_captions(&mut self, track: &str) {
        let Ok(v) = self.run("captions.export", &json!({"track": track})) else { return };
        let srt = v["srt"].as_str().unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) =
                rfd::FileDialog::new().add_filter("SubRip", &["srt"]).set_file_name("captions.srt").save_file()
            else {
                return;
            };
            self.status = match std::fs::write(&path, srt) {
                Ok(()) => fill(self.s().cap_exported, &[&path.display()]),
                Err(e) => format!("{}: {e}", path.display()),
            };
        }
        #[cfg(target_arch = "wasm32")]
        {
            picker::save("captions.srt", srt.as_bytes(), "application/x-subrip");
            self.status = fill(self.s().cap_exported, &[&"captions.srt"]);
        }
    }

    /// Places SubRip captions on a captions track (a command, so a joined editor shares them); as
    /// a second language's line when the captions list asked for the file so.
    fn import_captions(&mut self, name: &str, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let line = if std::mem::take(&mut self.second_captions) { 2 } else { 1 };
        if let Ok(v) = self.execute("captions.import", json!({ "srt": text, "line": line })) {
            let n = v["clips"].as_array().map_or(0, Vec::len);
            self.status = fill(self.s().captions_imported, &[&n, &name]);
        }
    }

    /// Reads a CapCut / 剪映 draft into a new timeline (a command, so a joined editor shares it).
    fn import_draft(&mut self, bytes: &[u8], folder: Option<String>) {
        let text = String::from_utf8_lossy(bytes);
        let mut params = json!({ "draft": text });
        if let Some(f) = folder {
            params["folder"] = json!(f);
        }
        if let Ok(v) = self.execute("capcut.import", params) {
            self.selected = None;
            self.playhead = Tick::ZERO;
            let clips = v["clips"].as_u64().unwrap_or(0);
            let offline = v["offline"].as_array().map_or(0, Vec::len);
            let skipped: u64 = v["skipped"].as_object().map_or(0, |o| o.values().filter_map(Value::as_u64).sum());
            let s = self.s();
            let mut msg = fill(s.draft_imported, &[&clips]);
            if offline > 0 {
                msg.push_str(&fill(s.draft_offline, &[&offline]));
            }
            if skipped > 0 {
                msg.push_str(&fill(s.draft_skipped, &[&skipped]));
            }
            self.status = msg;
        }
    }

    /// The selected clip, when it is a title.
    fn selected_title(&self) -> Option<String> {
        let id = self.selected.clone()?;
        let p = self.project();
        matches!(p.clip(&id)?.1.content, ClipContent::Text { .. }).then_some(id)
    }

    /// After `media.import`: puts the file at the playhead when `place`, or says it is in the media.
    fn imported(&mut self, imported: &Value, place: bool) {
        let Ok(m) = serde_json::from_value::<reelforge_model::Media>(imported["media"].clone()) else { return };
        if imported["relinked"] == true {
            // it was offline on the timeline already
            self.status = fill(self.s().relinked, &[&m.name]);
            return;
        }
        if m.kind == MediaKind::Font {
            // a font is set on the selected title, or waits in a title's font list
            let wear =
                self.selected_title().is_some_and(|c| self.run("clip.set", &json!({"clip": c, "font": m.id})).is_ok());
            self.status = fill(if wear { self.s().font_worn } else { self.s().font_imported }, &[&m.name]);
            return;
        }
        if m.kind == MediaKind::Lut {
            // a lookup table is worn by the selected picture clip, or waits in the colour section
            let wear = place
                && self
                    .selected
                    .clone()
                    .is_some_and(|c| self.run("clip.setColor", &json!({"clip": c, "lut": m.id})).is_ok());
            self.status = fill(if wear { self.s().lut_worn } else { self.s().lut_imported }, &[&m.name]);
            return;
        }
        if place {
            if self.place(&m, self.playhead, None) {
                self.status = fill(self.s().imported, &[&m.name]);
            }
        } else {
            self.status = fill(self.s().added_to_media, &[&m.name]);
        }
    }

    /// Puts media `m` at `at`: on `track` if it has room there, else on the lowest track of its
    /// kind with room, else on a new track above the others. Selects the new clip.
    fn place(&mut self, m: &reelforge_model::Media, at: Tick, track: Option<String>) -> bool {
        if m.kind == MediaKind::Font {
            if let Some(c) = self.selected_title()
                && self.execute("clip.set", json!({"clip": c, "font": m.id})).is_ok()
            {
                self.status = fill(self.s().font_worn, &[&m.name]);
            }
            return false;
        }
        if m.kind == MediaKind::Lut {
            if let Some(c) = self.selected.clone()
                && self.execute("clip.setColor", json!({"clip": c, "lut": m.id})).is_ok()
            {
                self.status = fill(self.s().lut_worn, &[&m.name]);
            }
            return false;
        }
        if m.kind != MediaKind::Audio && self.fill_with(m, at, track.as_deref()) {
            return false;
        }
        let want = if m.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
        let id = m.id.clone();
        self.put(want, track, move |t| json!({"track": t, "media": id, "start": at}))
    }

    /// Puts picture or video `m` where one was asked for, and says so: in the clip Replace was
    /// picked on, else in the template slot it was dropped on (`track` at `at`), else, when it was
    /// not dropped anywhere in particular, in the first empty slot of the timeline's template.
    /// False when nothing asked for it.
    fn fill_with(&mut self, m: &reelforge_model::Media, at: Tick, track: Option<&str>) -> bool {
        let s = self.s();
        if let Some(clip) = self.into.take() {
            if self.execute("clip.replace", json!({"clip": clip, "media": m.id, "fit": "slow"})).is_ok() {
                self.selected = Some(clip);
                self.status = fill(s.replaced, &[&m.name]);
            }
            return true;
        }
        let p = self.project();
        let slot = match track {
            Some(t) => p
                .track(t)
                .and_then(|t| t.clips.iter().find(|c| c.start <= at && at < c.start + c.duration))
                .and_then(|c| c.slot),
            None => reelforge_engine::slot_states(&p).into_iter().find(|(_, _, filled)| !filled).map(|(n, ..)| n),
        };
        let Some(n) = slot else { return false };
        if self.execute("template.fill", json!({"media": [m.id], "from": n})).is_ok() {
            let p = self.project();
            self.selected =
                reelforge_engine::slot_states(&p).into_iter().find(|(k, ..)| *k == n).map(|(_, clip, _)| clip);
            self.also.clear();
            if let Some((_, c)) = self.selected.as_ref().and_then(|c| p.clip(c)) {
                self.playhead = c.start;
            }
            self.status = fill(s.remix_filled, &[&m.name, &n.to_string()]);
        }
        true
    }

    /// Builds built-in timeline template `id`: on this timeline when nothing is on it yet, else on
    /// a new one. Its slots wait for pictures and videos, counted in the templates tab.
    fn use_template(&mut self, id: &str) {
        let s = self.s();
        let name = remix::name(id, s);
        // its titles say what they say in the interface's language
        let texts = reelforge_engine::timeline_templates().iter().position(|t| t.id == id).map(|i| s.remix_texts[i]);
        if let Ok(v) = self.execute("template.apply", json!({"template": id, "name": name, "texts": texts})) {
            self.set_playing(false);
            self.selected = None;
            self.also.clear();
            self.playhead = Tick::ZERO;
            self.timeline_name.clear();
            self.assets.show_templates();
            let slots = v["slots"].as_array().map_or(0, Vec::len);
            self.status = fill(s.remix_used, &[&name, &slots.to_string()]);
        }
    }

    /// Asks for pictures and videos for the empty slots of the timeline's template.
    fn pick_fill(&mut self, ctx: &egui::Context) {
        self.into = None;
        self.pick(ctx, PICTURES, true);
    }

    /// Replace in a clip's menu: asks for the picture or video the selected clip shows instead.
    fn pick_replacement(&mut self, ctx: &egui::Context) {
        let Some(clip) = self.selected.clone() else { return };
        self.into = Some(clip);
        self.pick(ctx, PICTURES, true);
        // the desktop's dialog has answered by now; the browser's files arrive later
        #[cfg(not(target_arch = "wasm32"))]
        if self.remote.is_none() {
            self.into = None;
        }
    }

    /// Makes the selected picture clips slots of a template (numbered in the order they play), or
    /// no longer slots.
    fn mark_slots(&mut self, on: bool) {
        let clips = self.selection();
        let mut params = json!({ "clips": clips });
        if !on {
            params["slot"] = Value::Null;
        }
        let _ = self.execute("template.mark", params);
    }

    /// Adds a clip (`params` for a track id) to `track` when it takes it there, else to the lowest
    /// track of `kind` with room, else to a new track of that kind. Selects the new clip.
    fn put(&mut self, kind: TrackKind, track: Option<String>, params: impl Fn(&str) -> Value) -> bool {
        self.put_from(kind, track, false, params)
    }

    /// [`put`](Self::put), trying the highest track first when `over`: what goes over the picture
    /// (a sticker, an effect) belongs above it.
    fn put_from(&mut self, kind: TrackKind, track: Option<String>, over: bool, params: impl Fn(&str) -> Value) -> bool {
        let mut tracks: Vec<String> =
            self.project().tracks.iter().filter(|t| t.kind == kind).map(|t| t.id.clone()).collect();
        if over {
            tracks.reverse();
        }
        if let Some(t) = track {
            tracks.retain(|x| *x != t);
            tracks.insert(0, t);
        }
        // a track with no room refuses the clip and nothing changes, so trying is safe
        let mut placed = tracks.iter().find_map(|t| self.run("clip.add", &params(t)).ok());
        if placed.is_none() {
            let word = match kind {
                TrackKind::Audio => "audio",
                TrackKind::Text => "text",
                TrackKind::Video => "video",
            };
            if let Ok(t) = self.execute("track.add", json!({ "kind": word })) {
                let id = t["track"].as_str().unwrap_or_default().to_string();
                placed = self.execute("clip.add", params(&id)).ok();
            }
        }
        match placed {
            Some(v) => {
                self.selected = v["clip"].as_str().map(str::to_string);
                true
            }
            None => false,
        }
    }

    /// Puts something from the asset panel at `at` (on `track`, when it was dropped on one).
    fn add_item(&mut self, item: &assets::Item, at: Tick, track: Option<String>) {
        let s = self.s();
        match item {
            assets::Item::Media(id) => {
                let Some(m) = self.project().media.get(id).cloned() else { return };
                if self.place(&m, at, track) {
                    self.status = fill(s.imported, &[&m.name]);
                }
            }
            assets::Item::Sound { id, name, .. } => {
                let mut params = json!({"sound": id, "at": at, "name": name});
                if let Some(t) = track {
                    params["track"] = json!(t);
                }
                if let Ok(v) = self.execute("sound.add", params) {
                    self.selected = v["clip"].as_str().map(str::to_string);
                }
            }
            assets::Item::Title(template) => {
                let (text, template) = (s.new_title, *template);
                self.put(TrackKind::Text, track, move |t| {
                    let mut p = json!({"track": t, "text": text, "start": at, "duration": assets::TITLE_LENGTH});
                    if let Some(id) = template {
                        p["template"] = json!(id);
                    }
                    p
                });
            }
            assets::Item::Sticker(id) => {
                let (id, name) = (*id, item.label(&self.project(), s));
                self.put_from(TrackKind::Video, track, true, move |t| {
                    json!({"track": t, "sticker": id, "name": name, "start": at, "duration": assets::OVERLAY_LENGTH})
                });
            }
            assets::Item::Effect(kind) => {
                let fx: Vec<&str> = kind.iter().map(|k| k.name()).collect();
                self.put_from(TrackKind::Video, track, true, move |t| {
                    json!({"track": t, "adjustment": true, "fx": fx, "start": at, "duration": assets::OVERLAY_LENGTH})
                });
            }
        }
    }

    /// Asks for files to add to the project's media: the system's file dialog on the desktop, the
    /// browser's file picker on the web (its files arrive later, like dropped ones).
    fn pick_files(&mut self, ctx: &egui::Context) {
        self.second_captions = false;
        self.pick(ctx, IMPORTABLE, false);
    }

    /// Asks for files with these extensions and imports what is picked; when `place`, they go on
    /// the timeline as dropped files do.
    fn pick(&mut self, ctx: &egui::Context, extensions: &[&str], place: bool) {
        let s = self.s();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let mut dialog = rfd::FileDialog::new().add_filter(s.picker_media, extensions);
            if extensions.len() > 1 {
                dialog = dialog.add_filter(s.picker_all, &["*"]);
            }
            let picked = dialog.pick_files().unwrap_or_default();
            if picked.is_empty() {
                self.second_captions = false;
            }
            for path in picked {
                if self.remote.is_some() {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    match std::fs::read(&path) {
                        Ok(bytes) => self.import_bytes(&name, bytes, place),
                        Err(e) => self.status = format!("{name}: {e}"),
                    }
                } else {
                    self.import_path(&path, place);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Err(e) = picker::pick_files(ctx, self.arrivals.clone(), extensions, place) {
            self.status = format!("{}: {e}", s.import);
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
                    Ok(bytes) => self.import_bytes(&name, bytes, true),
                    Err(e) => self.status = format!("{name}: {e}"),
                }
            } else {
                self.import_path(f.path(), true);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let arrivals = self.arrivals.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let name = f.path().to_string_lossy().into_owned();
                    let bytes = f.bytes_async().await;
                    arrivals.lock().expect("arrivals lock").push((name, bytes, true));
                    ctx.request_repaint();
                });
            }
        }
        let arrived = std::mem::take(&mut *self.arrivals.lock().expect("arrivals lock"));
        for (name, bytes, place) in arrived {
            match bytes {
                Ok(b) => self.import_bytes(&name, b, place),
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

    /// Every selected clip that still exists, the inspector's first.
    fn selection(&self) -> Vec<String> {
        let Some(first) = &self.selected else { return Vec::new() };
        let p = self.project();
        std::iter::once(first)
            .chain(self.also.iter().filter(|id| *id != first))
            .filter(|id| p.clip(id).is_some())
            .cloned()
            .collect()
    }

    fn select(&mut self, change: timeline::Select) {
        use timeline::Select;
        let p = self.project();
        // a group is selected, and let go, as one: the clip itself first
        let with_mates = |id: String| -> Vec<String> {
            let group = p.clip(&id).and_then(|(_, c)| c.group.clone());
            let mates: Vec<String> = match &group {
                Some(g) => p
                    .tracks
                    .iter()
                    .flat_map(|t| &t.clips)
                    .filter(|c| c.group.as_ref() == Some(g) && c.id != id)
                    .map(|c| c.id.clone())
                    .collect(),
                None => Vec::new(),
            };
            std::iter::once(id).chain(mates).collect()
        };
        let mut all = self.selection();
        match change {
            Select::Only(id) => all = with_mates(id),
            Select::Toggle(id) => {
                let group = with_mates(id);
                if all.contains(&group[0]) {
                    all.retain(|c| !group.contains(c));
                } else {
                    all.retain(|c| !group.contains(c));
                    all.splice(0..0, group);
                }
            }
            Select::Set(ids) => {
                all = Vec::new();
                for id in ids.into_iter().flat_map(with_mates) {
                    if !all.contains(&id) {
                        all.push(id);
                    }
                }
            }
        }
        let mut all = all.into_iter();
        self.selected = all.next();
        self.also = all.collect();
    }

    /// The selected clips the playhead is strictly inside.
    fn under_playhead(&self) -> Vec<String> {
        let p = self.project();
        self.selection()
            .into_iter()
            .filter(|id| p.clip(id).is_some_and(|(_, c)| c.start < self.playhead && self.playhead < c.end()))
            .collect()
    }

    /// Runs one command per id as one undo step.
    fn each(&mut self, ids: Vec<String>, id: &str, params: impl Fn(&str) -> Value) {
        match ids.len() {
            0 => {}
            1 => {
                let _ = self.execute(id, params(&ids[0]));
            }
            _ => {
                let steps: Vec<Value> = ids.iter().map(|c| json!({"id": id, "params": params(c)})).collect();
                let _ = self.execute("edit.batch", json!({"steps": steps, "label": id}));
            }
        }
    }

    fn split(&mut self) {
        let at = self.playhead;
        self.each(self.under_playhead(), "clip.split", |c| json!({"clip": c, "at": at}));
    }

    /// Q and W: cuts off what is before (or after) the playhead.
    fn trim_to_playhead(&mut self, end: bool) {
        let at = self.playhead;
        let key = if end { "end" } else { "start" };
        self.each(self.under_playhead(), "clip.trim", |c| json!({"clip": c, key: at}));
    }

    fn delete(&mut self, ripple: bool) {
        let all = self.selection();
        if !all.is_empty() && self.execute("clip.remove", json!({"clips": all, "ripple": ripple})).is_ok() {
            self.selected = None;
            self.also.clear();
            self.inspector = None;
        }
    }

    fn copy(&mut self) -> bool {
        let all = self.selection();
        if all.is_empty() {
            return false;
        }
        match self.execute("clip.copy", json!({"clips": all})) {
            Ok(board) => {
                self.status = fill(self.s().copied, &[&all.len()]);
                self.clipboard = Some(board);
                true
            }
            Err(_) => false,
        }
    }

    /// Pastes at the playhead; the copies become the selection.
    fn paste(&mut self) {
        let Some(board) = self.clipboard.clone() else { return };
        if let Ok(v) = self.execute("clip.paste", json!({"clipboard": board, "at": self.playhead})) {
            self.select_placed(&v);
        }
    }

    /// Moves the sound of the selected video clips to clips of their own on audio tracks.
    fn detach_audio(&mut self) {
        let p = self.project();
        let ids: Vec<String> = self.selection().into_iter().filter(|c| timeline::detachable(&p, c)).collect();
        self.each(ids, "clip.detachAudio", |c| json!({"clip": c}));
    }

    fn duplicate(&mut self) {
        let all = self.selection();
        if !all.is_empty()
            && let Ok(v) = self.execute("clip.duplicate", json!({"clips": all}))
        {
            self.select_placed(&v);
        }
    }

    fn select_placed(&mut self, v: &Value) {
        let ids =
            v["clips"].as_array().map(|a| a.iter().filter_map(|c| c["clip"].as_str().map(str::to_string)).collect());
        self.select(timeline::Select::Set(ids.unwrap_or_default()));
        self.inspector = None;
    }

    fn select_all(&mut self) {
        let ids = self.project().tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.clone())).collect();
        self.select(timeline::Select::Set(ids));
    }

    fn act(&mut self, action: timeline::Action) {
        use timeline::Action;
        match action {
            Action::Split => self.split(),
            Action::Copy => {
                self.copy();
            }
            Action::Cut => {
                if self.copy() {
                    self.delete(false);
                }
            }
            Action::Paste => self.paste(),
            Action::Duplicate => self.duplicate(),
            Action::Delete => self.delete(false),
            Action::RippleDelete => self.delete(true),
            Action::TrimStart => self.trim_to_playhead(false),
            Action::TrimEnd => self.trim_to_playhead(true),
            Action::SelectAll => self.select_all(),
            Action::DetachAudio => self.detach_audio(),
            Action::Group => self.group(),
            Action::Ungroup => self.ungroup(),
            Action::Compound => self.compound(),
            Action::Uncompound => self.uncompound(),
            Action::OpenCompound => self.open_compound(),
            Action::Slot => self.mark_slots(true),
            Action::Unslot => self.mark_slots(false),
            // needs the window to ask for a file: see `pick_replacement`
            Action::Replace => {}
        }
    }

    /// Ctrl+G: the selected clips are selected and moved as one from now on.
    fn group(&mut self) {
        let all = self.selection();
        if all.len() >= 2 {
            let _ = self.execute("clip.group", json!({"clips": all}));
        }
    }

    fn ungroup(&mut self) {
        let p = self.project();
        let grouped: Vec<String> =
            self.selection().into_iter().filter(|id| p.clip(id).is_some_and(|(_, c)| c.group.is_some())).collect();
        if !grouped.is_empty() {
            let _ = self.execute("clip.ungroup", json!({"clips": grouped}));
        }
    }

    /// Alt+G: the selected clips become one clip that plays a timeline of their own.
    fn compound(&mut self) {
        let all = self.selection();
        if all.is_empty() {
            return;
        }
        if let Ok(v) = self.execute("clip.compound", json!({"clips": all})) {
            let made = v["clip"].as_str().map(str::to_string);
            self.select(timeline::Select::Set(made.into_iter().collect()));
            self.inspector = None;
            self.status = fill(self.s().compound_made, &[&all.len()]);
        }
    }

    /// Puts the clips of the selected compound clips back in their place.
    fn uncompound(&mut self) {
        let p = self.project();
        let ids: Vec<String> =
            self.selection().into_iter().filter(|id| p.clip(id).is_some_and(|(_, c)| c.compound().is_some())).collect();
        match ids.as_slice() {
            [] => {}
            [one] => {
                if let Ok(v) = self.execute("clip.uncompound", json!({"clip": one})) {
                    self.select_placed(&v);
                }
            }
            _ => {
                self.each(ids, "clip.uncompound", |c| json!({"clip": c}));
                self.select(timeline::Select::Set(Vec::new()));
                self.inspector = None;
            }
        }
    }

    /// Edits what is inside the selected compound clip: its timeline, at the instant the playhead
    /// shows of it.
    fn open_compound(&mut self) {
        let p = self.project();
        let Some((_, clip)) = self.selected.as_ref().and_then(|id| p.clip(id)) else { return };
        let Some((timeline, source_in)) = clip.compound() else { return };
        let inside = self.playhead.max(clip.start).min(clip.end()) - clip.start + source_in;
        let list = p.timeline_list();
        let name = list.iter().position(|t| t.id == timeline).map(|i| self.timeline_label(&list[i].name, i));
        if self.go_to_timeline(timeline, inside) {
            self.status = fill(self.s().compound_inside, &[&name.unwrap_or_default()]);
        }
    }

    /// Leaves the compound clip being edited for a timeline that plays it, with that clip
    /// selected and the playhead on the same picture.
    fn leave_compound(&mut self) {
        let p = self.project();
        let (inside, id) = (self.playhead, p.timeline.clone());
        match p.player_of(&id).map(|(timeline, c)| (timeline.to_string(), c.clone())) {
            Some((timeline, clip)) => {
                let source_in = clip.compound().map_or(Tick::ZERO, |(_, from)| from);
                let at = (clip.start + (inside - source_in)).max(clip.start).min(clip.end());
                if self.go_to_timeline(&timeline, at) {
                    self.select(timeline::Select::Set(vec![clip.id]));
                }
            }
            // nothing plays it any more: back to the first timeline that is not a compound clip's
            None => {
                if let Some(t) = p.timeline_list().into_iter().find(|t| !t.compound) {
                    self.go_to_timeline(&t.id, Tick::ZERO);
                }
            }
        }
    }

    /// Switches to timeline `id` with the playhead at `at`. The selection belonged to the old one.
    fn go_to_timeline(&mut self, id: &str, at: Tick) -> bool {
        if self.execute("timeline.switch", json!({"timeline": id})).is_err() {
            return false;
        }
        let p = self.project();
        self.selected = None;
        self.also.clear();
        self.inspector = None;
        self.timeline_name.clear();
        self.playhead = p.canvas.fps.snap(at.max(Tick::ZERO).min(p.duration()));
        true
    }

    /// The range set with I and O, when it has both ends the right way round.
    fn range(&self) -> Option<(Tick, Tick)> {
        match (self.range_in, self.range_out) {
            (Some(a), Some(b)) if a < b => Some((a, b)),
            (Some(a), None) if a < self.project().duration() => Some((a, self.project().duration())),
            (None, Some(b)) if b > Tick::ZERO => Some((Tick::ZERO, b)),
            _ => None,
        }
    }

    /// J and L: plays backwards or forwards, faster each time the same key is pressed again.
    fn shuttle(&mut self, forwards: bool) {
        let same = self.playing && (self.rate > 0.0) == forwards;
        self.rate = match (same, forwards) {
            (true, _) => (self.rate * 2.0).clamp(-4.0, 4.0),
            (false, true) => 1.0,
            (false, false) => -1.0,
        };
        self.set_playing(true);
    }

    /// Moves a track up or down among its kind, or deletes it.
    fn track_action(&mut self, track: &str, what: timeline::TrackAction) {
        let p = self.project();
        let Some(kind) = p.track(track).map(|t| t.kind) else { return };
        let at = p.tracks.iter().filter(|t| t.kind == kind).position(|t| t.id == track).unwrap_or(0);
        let _ = match what {
            timeline::TrackAction::Up => self.execute("track.move", json!({"track": track, "index": at + 1})),
            timeline::TrackAction::Down => {
                self.execute("track.move", json!({"track": track, "index": at.saturating_sub(1)}))
            }
            timeline::TrackAction::Delete => self.execute("track.remove", json!({"track": track})),
        };
    }

    /// The previous (or next) place a clip starts or ends, or a marker is.
    fn cut_point(&self, next: bool) -> Option<Tick> {
        let p = self.project();
        let points = p
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().flat_map(|c| [c.start, c.end()]))
            .chain(p.markers.iter().map(|m| m.at))
            .chain([Tick::ZERO]);
        if next { points.filter(|t| *t > self.playhead).min() } else { points.filter(|t| *t < self.playhead).max() }
    }

    /// Plays a moment of sound at the playhead, as it is dragged while paused.
    fn scrub(&mut self) {
        if let Some(spec) = self.sound_spec() {
            self.sound.scrub(&spec, &self.media, self.playhead);
        }
    }

    /// The project compiled for sound, compiled again after an edit.
    fn sound_spec(&mut self) -> Option<Spec> {
        let revision = self.session.revision();
        if self.sound_spec.as_ref().is_none_or(|(r, _)| *r != revision) {
            let p = self.project();
            self.sound_spec =
                compile(&p, &CompileOptions { encode: false }).ok().map(|spec| (revision, Arc::new(spec)));
        }
        self.sound_spec.as_ref().map(|(_, spec)| spec.clone())
    }

    fn seek(&mut self, t: Tick) {
        let p = self.project();
        self.playhead = p.canvas.fps.snap(t.max(Tick::ZERO).min(p.duration()));
        if self.playing && self.rate == 1.0 {
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
            self.rate = 1.0;
            self.set_playing(!self.playing);
        }
        if pressed(Modifiers::NONE, Key::L) {
            self.shuttle(true);
        }
        if pressed(Modifiers::NONE, Key::J) {
            self.shuttle(false);
        }
        if pressed(Modifiers::NONE, Key::K) {
            self.set_playing(false);
            self.rate = 1.0;
        }
        if pressed(cmd, Key::C) {
            self.copy();
        }
        if pressed(cmd, Key::X) {
            self.act(timeline::Action::Cut);
        }
        if pressed(cmd, Key::V) {
            self.paste();
        }
        if pressed(cmd, Key::D) {
            self.duplicate();
        }
        if pressed(Modifiers::ALT, Key::D) {
            self.detach_audio();
        }
        // a key with Shift is asked for first: the same key without it would take it too
        if pressed(cmd | Modifiers::SHIFT, Key::G) {
            self.ungroup();
        }
        if pressed(cmd, Key::G) {
            self.group();
        }
        if pressed(Modifiers::ALT | Modifiers::SHIFT, Key::G) {
            self.uncompound();
        }
        if pressed(Modifiers::ALT, Key::G) {
            self.compound();
        }
        if pressed(cmd, Key::A) {
            self.select_all();
        }
        if pressed(Modifiers::NONE, Key::Escape) {
            self.select(timeline::Select::Set(Vec::new()));
        }
        if pressed(Modifiers::NONE, Key::M) {
            let _ = self.execute("marker.add", json!({"at": self.playhead}));
        }
        if pressed(Modifiers::NONE, Key::I) {
            self.range_in = Some(self.playhead);
        }
        if pressed(Modifiers::NONE, Key::O) {
            self.range_out = Some(self.playhead);
        }
        if pressed(Modifiers::ALT, Key::X) {
            (self.range_in, self.range_out) = (None, None);
        }
        if pressed(Modifiers::NONE, Key::F) {
            self.preview_only = !self.preview_only;
        }
        if pressed(Modifiers::NONE, Key::Q) {
            self.trim_to_playhead(false);
        }
        if pressed(Modifiers::NONE, Key::W) {
            self.trim_to_playhead(true);
        }
        for (next, k) in [(false, Key::ArrowUp), (true, Key::ArrowDown)] {
            if pressed(Modifiers::NONE, k)
                && let Some(t) = self.cut_point(next)
            {
                self.seek(t);
            }
        }
        if pressed(Modifiers::NONE, Key::Plus) || pressed(Modifiers::NONE, Key::Equals) {
            self.timeline.zoom(1.5);
        }
        if pressed(Modifiers::NONE, Key::Minus) {
            self.timeline.zoom(1.0 / 1.5);
        }
        if pressed(Modifiers::SHIFT, Key::Z) {
            let width = ctx.content_rect().width();
            self.timeline.fit(self.project().duration().seconds(), width);
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
        if pressed(Modifiers::SHIFT, Key::Delete) || pressed(Modifiers::SHIFT, Key::Backspace) {
            self.delete(true);
        }
        if pressed(Modifiers::NONE, Key::Delete) || pressed(Modifiers::NONE, Key::Backspace) {
            self.delete(false);
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
            let narrow = self.narrow;
            ui.label(egui::RichText::new("reelforge").strong().size(15.0).color(theme::color::ACCENT));
            ui.add_space(6.0);
            if let Some(drafts) = &self.drafts {
                // to the drafts page, and which draft this is
                let name = drafts.current().map(|d| d.name.clone());
                if theme::tool(ui, icon::FRAME_BACK, s.drafts, s.drafts_tip, true).clicked() {
                    self.draft_ask = Some(drafts::Ask::Home);
                    ui.ctx().request_repaint();
                }
                if let Some(name) = name.filter(|_| !narrow) {
                    ui.label(egui::RichText::new(name).color(theme::color::TEXT));
                }
            }
            if !narrow {
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
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.export_controls(ui, &p);
                ui.add_space(4.0);
                let mut lang = self.lang;
                // a narrow bar has room for the language's code only
                let (shown, width) = if narrow { (self.lang.code(), 40.0) } else { (s.lang_name, 96.0) };
                egui::ComboBox::from_id_salt("lang")
                    .selected_text(egui::RichText::new(shown).color(theme::color::WEAK))
                    .width(width)
                    .show_ui(ui, |ui| {
                        for l in Lang::ALL {
                            ui.selectable_value(&mut lang, l, l.strings().lang_name);
                        }
                    })
                    .response
                    .on_hover_text(s.lang_tip);
                if lang != self.lang {
                    if self.status == s.ready {
                        self.status.clear();
                    }
                    if (lang == Lang::Ja) != (self.lang == Lang::Ja) {
                        theme::set_fonts(ui.ctx(), lang == Lang::Ja);
                    }
                    self.lang = lang;
                    self.lang_chosen = true;
                    self.lang_unsaved = true;
                }
                // an editor with drafts starts anew on the drafts page, which loses nothing
                if self.drafts.is_none()
                    && theme::tool(ui, icon::ADD, if narrow { "" } else { s.new_project }, s.new_project_tip, true)
                        .clicked()
                {
                    self.new_project();
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
            // a narrow bar keeps the playhead's time, the steps by a frame, play, loop and the
            // preview alone
            let narrow = self.narrow;
            if !narrow {
                ui.label(
                    egui::RichText::new(format!("/ {}", theme::timecode(p.duration().seconds())))
                        .monospace()
                        .color(theme::color::WEAK),
                );
            }
            // the play controls, centred on the panel
            let w = 4.0 * 34.0 + 3.0 * 6.0 + 12.0;
            let room = ui.available_width();
            ui.add_space(if narrow { 8.0 } else { ((room - w) / 2.0 - 120.0).max(8.0) });
            let frame = fps.frame_at(self.playhead);
            if !narrow && theme::tool(ui, icon::TO_START, "", s.to_start, true).clicked() {
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
            if !narrow && theme::tool(ui, icon::TO_END, "", s.to_end, true).clicked() {
                self.seek(p.duration());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !narrow {
                    ui.label(egui::RichText::new(fill(s.frame, &[&frame])).monospace().color(theme::color::WEAK));
                    ui.add_space(8.0);
                }
                let looping = egui::Button::new(egui::RichText::new(icon::LOOP).size(14.0).color(if self.looping {
                    theme::color::ACCENT
                } else {
                    theme::color::WEAK
                }));
                if ui.add(looping).on_hover_text(s.loop_tip).clicked() {
                    self.looping = !self.looping;
                }
                let tip = if self.preview_only { s.preview_exit_tip } else { s.preview_only_tip };
                if theme::tool(ui, icon::SCREEN, "", tip, true).clicked() {
                    self.preview_only = !self.preview_only;
                }
                let label = if self.full_quality { s.quality_full } else { s.quality_smooth };
                if !narrow
                    && ui
                        .add(egui::Button::new(egui::RichText::new(label).small()))
                        .on_hover_text(s.quality_tip)
                        .clicked()
                {
                    self.full_quality = !self.full_quality;
                    self.quality_unsaved = true;
                }
            });
        });
    }

    /// Above the timeline: edit tools, snapping, zoom.
    /// Record, or stop and place what was recorded at the playhead.
    fn record_button(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        if let Some(why) = self.recorder.as_ref().and_then(record::Recorder::failed) {
            self.recorder = None;
            self.status = fill(s.mic_failed, &[&why]);
        }
        match &self.recorder {
            None => {
                let r = ui.add(egui::Button::new(
                    egui::RichText::new(format!("●  {}", s.record)).color(theme::color::PLAYHEAD),
                ));
                if r.on_hover_text(s.record_tip).clicked() {
                    match record::Recorder::start() {
                        Ok(rec) => {
                            self.recorder = Some(rec);
                            self.status = s.recording.to_string();
                        }
                        Err(e) => self.status = fill(s.mic_failed, &[&e]),
                    }
                }
            }
            Some(rec) => {
                let label = format!("■  {}  {}", s.stop_recording, theme::timecode(rec.seconds()));
                let r = ui.add(
                    egui::Button::new(egui::RichText::new(label).color(theme::color::ON_ACCENT).strong())
                        .fill(theme::color::PLAYHEAD),
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                if r.clicked() {
                    let rec = self.recorder.take().expect("recording");
                    match rec.stop() {
                        Ok(take) if take.seconds() >= 0.2 => {
                            self.takes += 1;
                            let name = format!("{} {}.wav", s.recording_name, self.takes);
                            self.import_bytes(&name, take.wav(), true);
                        }
                        Ok(_) => self.status = s.recording_short.to_string(),
                        Err(e) => self.status = fill(s.mic_failed, &[&e]),
                    }
                }
            }
        }
    }

    /// A timeline's name as shown: the engine's English defaults ("Timeline 2", "… copy") in the
    /// interface language.
    fn timeline_label(&self, name: &str, index: usize) -> String {
        let s = self.s();
        if let Some(original) = name.strip_suffix(" copy") {
            return fill(s.timeline_copy, &[&self.timeline_label(original, index)]);
        }
        if let Some(n) = name.strip_prefix("Compound ").and_then(|n| n.parse::<u32>().ok()) {
            return fill(s.compound_n, &[&n.to_string()]);
        }
        match name.strip_prefix("Timeline ").and_then(|n| n.parse::<u32>().ok()) {
            Some(n) => fill(s.timeline_n, &[&n.to_string()]),
            None if name.is_empty() => fill(s.timeline_n, &[&(index + 1).to_string()]),
            None => name.to_string(),
        }
    }

    /// Switches to another timeline: the selection belonged to the old one.
    fn timeline_command(&mut self, id: &str, params: Value) {
        if self.execute(id, params).is_ok() {
            self.selected = None;
            self.playhead = self.playhead.min(self.project().duration());
            self.timeline_name.clear();
        }
    }

    /// The project's timelines, while open: switch, add, copy, rename, delete.
    fn timelines_window(&mut self, ctx: &egui::Context) {
        if !self.timelines_open {
            return;
        }
        let s = self.s();
        let p = self.project();
        let list = p.timeline_list();
        let current = list.iter().position(|t| t.current).unwrap_or(0);
        if self.timeline_name.is_empty() {
            self.timeline_name = self.timeline_label(&list[current].name, current);
        }
        let mut open = true;
        let mut todo: Option<(&str, Value)> = None;
        egui::Window::new(s.timelines)
            .open(&mut open)
            .anchor(egui::Align2::LEFT_TOP, [12.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(280.0)
            .show(ctx, |ui| {
                for (i, t) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let name = self.timeline_label(&t.name, i);
                        let r = ui.selectable_label(t.current, format!("{}  {name}", icon::FILM));
                        if r.clicked() && !t.current {
                            todo = Some(("timeline.switch", json!({"timeline": t.id})));
                        }
                        if t.compound {
                            ui.label(egui::RichText::new(s.kind_compound).small().color(theme::color::COMPOUND));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(theme::timecode(t.duration.seconds()))
                                    .small()
                                    .color(theme::color::WEAK),
                            );
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.timeline_name).desired_width(150.0));
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name = self.timeline_name.trim().to_string();
                    let changed = name != self.timeline_label(&list[current].name, current);
                    if (ui.add_enabled(changed && !name.is_empty(), egui::Button::new(s.timeline_rename)).clicked()
                        || (enter && changed && !name.is_empty()))
                        && todo.is_none()
                    {
                        todo = Some(("timeline.rename", json!({"name": name})));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  {}", icon::ADD, s.timeline_new)).clicked() {
                        let name = fill(s.timeline_n, &[&(list.len() + 1).to_string()]);
                        todo = Some(("timeline.add", json!({"name": name})));
                    }
                    if ui.button(s.timeline_duplicate).clicked() {
                        let name = fill(s.timeline_copy, &[&self.timeline_label(&list[current].name, current)]);
                        todo = Some(("timeline.add", json!({"copy": true, "name": name})));
                    }
                    if ui
                        .add_enabled(
                            list.len() > 1,
                            egui::Button::new(format!("{}  {}", icon::DELETE, s.timeline_delete)),
                        )
                        .clicked()
                    {
                        todo = Some(("timeline.remove", json!({})));
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(s.timeline_hint).small().color(theme::color::WEAK));
            });
        self.timelines_open = open;
        if let Some((id, params)) = todo {
            self.timeline_command(id, params);
        }
    }

    /// Looks for what could go from someone speaking: the pauses heard in the selected clips (or
    /// in the ones the tool takes by itself), and the captions that are only filler words.
    fn spoken_find(&mut self) {
        use spoken::{Found, Kind};
        let p = self.project();
        let sounds = |id: &String| {
            p.clip(id).is_some_and(|(_, c)| match &c.content {
                ClipContent::Media { media, silent, .. } => {
                    !silent && p.media.get(media).is_some_and(|m| m.kind != MediaKind::Image)
                }
                ClipContent::Compound { timeline, .. } => p.timeline_sounds(timeline),
                _ => false,
            })
        };
        let chosen: Vec<String> = self.selection().into_iter().filter(sounds).collect();
        let mut params = json!({"minPause": self.spoken.min_pause, "keep": self.spoken.keep, "apply": false});
        if !chosen.is_empty() {
            params["clips"] = json!(chosen);
        }
        let tick = |v: &Value| Tick(v.as_i64().unwrap_or(0));
        self.spoken.clear();
        match reelforge_engine::listen::plan("audio.cutPauses", &p, &self.media, &params) {
            Ok(plan) => {
                let ids = plan.found["clips"].as_array().into_iter().flatten();
                self.spoken.clips = ids.filter_map(|c| c.as_str().map(str::to_string)).collect();
                for r in plan.found["pauses"].as_array().into_iter().flatten() {
                    let (from, to) = (tick(&r["from"]), tick(&r["to"]));
                    self.spoken.found.push(Found { kind: Kind::Pause, from, to, text: String::new(), on: true });
                }
            }
            Err(_) => self.spoken.unheard = true,
        }
        if let Ok(v) = self.session.execute("captions.fillers", &json!({}), "editor") {
            for f in v["fillers"].as_array().into_iter().flatten() {
                let text = f["text"].as_str().unwrap_or_default().replace('\n', " ");
                let (from, to) = (tick(&f["from"]), tick(&f["to"]));
                self.spoken.found.push(Found { kind: Kind::Filler, from, to, text, on: true });
            }
        }
        self.spoken.found.sort_by_key(|f| (f.from, f.to));
        self.spoken.revision = Some(self.session.revision());
    }

    /// The tracks speech is cut from when not every unlocked one is: music on an audio track of
    /// its own plays on unbroken.
    fn spoken_tracks(&self, p: &Project) -> Option<Vec<String>> {
        if !self.spoken.leave_music {
            return None;
        }
        let spoken = &self.spoken.clips;
        let with_voice = |t: &&reelforge_model::Track| {
            !t.locked && (t.kind != TrackKind::Audio || t.clips.iter().any(|c| spoken.contains(&c.id)))
        };
        let cut: Vec<String> = p.tracks.iter().filter(with_voice).map(|t| t.id.clone()).collect();
        (cut.len() < p.tracks.iter().filter(|t| !t.locked).count()).then_some(cut)
    }

    /// Cuts the ticked stretches out of the timeline in one step.
    fn spoken_cut(&mut self) {
        let ranges = self.spoken.chosen();
        let (n, time) = self.spoken.total();
        if ranges.is_empty() {
            return;
        }
        let p = self.project();
        let listed: Vec<Value> = ranges.iter().map(|(a, b)| json!({"from": a.0, "to": b.0})).collect();
        let mut params = json!({ "ranges": listed });
        if let Some(tracks) = self.spoken_tracks(&p) {
            params["tracks"] = json!(tracks);
        }
        if self.execute("timeline.cut", params).is_ok() {
            // the playhead stays on what it showed
            let before = |(a, b): &(Tick, Tick)| (self.playhead.min(*b) - *a).max(Tick::ZERO);
            let gone = ranges.iter().map(before).fold(Tick::ZERO, |sum, t| sum + t);
            self.playhead = (self.playhead - gone).max(Tick::ZERO).min(self.project().duration());
            self.spoken.clear();
            self.status = fill(self.s().spoken_done, &[&n, &format!("{:.1}", time.seconds())]);
        }
    }

    /// The window that trims speech, while open: what to look for, what was found, and the cut.
    fn spoken_window(&mut self, ctx: &egui::Context) {
        // what was found is good for the project it was found in
        if self.spoken.revision.is_some_and(|r| r != self.session.revision()) {
            self.spoken.clear();
        }
        self.timeline.cuts = if self.spoken.open { self.spoken.chosen() } else { Vec::new() };
        self.timeline.cut_tracks = self.spoken_tracks(&self.project());
        if !self.spoken.open {
            return;
        }
        let s = self.s();
        let (count, time) = self.spoken.total();
        let sp = &mut self.spoken;
        let mut open = true;
        let (mut find, mut cut, mut seek) = (false, false, None);
        egui::Window::new(s.spoken)
            .open(&mut open)
            .default_pos([340.0, 96.0])
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::Grid::new("spoken options").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(s.spoken_min_pause);
                    let longer = egui::DragValue::new(&mut sp.min_pause).range(0.2..=5.0).speed(0.02);
                    ui.add(longer.fixed_decimals(1).suffix(" s"));
                    ui.end_row();
                    ui.label(s.spoken_keep);
                    let keep = egui::DragValue::new(&mut sp.keep).range(0.0..=0.5).speed(0.005);
                    ui.add(keep.fixed_decimals(2).suffix(" s"));
                    ui.end_row();
                });
                ui.checkbox(&mut sp.leave_music, s.spoken_leave_music);
                ui.add_space(2.0);
                ui.label(egui::RichText::new(s.spoken_hint).small().color(theme::color::WEAK));
                ui.add_space(4.0);
                if theme::tool(ui, icon::SEARCH, s.spoken_find, s.spoken_tip, true).clicked() {
                    find = true;
                }
                if sp.revision.is_none() {
                    return;
                }
                ui.separator();
                if sp.unheard {
                    ui.label(egui::RichText::new(s.spoken_unheard).small().color(theme::color::WARN));
                }
                if sp.found.is_empty() {
                    ui.label(egui::RichText::new(s.spoken_none).color(theme::color::WEAK));
                    return;
                }
                egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                    for f in &mut sp.found {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut f.on, "");
                            let at = egui::RichText::new(theme::timecode(f.from.seconds())).monospace();
                            if ui.link(at).clicked() {
                                seek = Some(f.from);
                            }
                            let (word, colour) = match f.kind {
                                spoken::Kind::Pause => (s.spoken_pause, theme::color::WEAK),
                                spoken::Kind::Filler => (s.spoken_filler, theme::color::WARN),
                            };
                            ui.label(egui::RichText::new(word).small().color(colour));
                            let length = format!("{:.1} s", (f.to - f.from).seconds());
                            ui.label(egui::RichText::new(length).small().color(theme::color::WEAK));
                            if !f.text.is_empty() {
                                ui.add(egui::Label::new(&f.text).truncate());
                            }
                        });
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let label = fill(s.spoken_cut, &[&count, &format!("{:.1}", time.seconds())]);
                    if theme::primary(ui, icon::SPLIT, &label, count > 0).clicked() {
                        cut = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(s.spoken_untick).clicked() {
                            sp.found.iter_mut().for_each(|f| f.on = false);
                        }
                        if ui.small_button(s.spoken_tick).clicked() {
                            sp.found.iter_mut().for_each(|f| f.on = true);
                        }
                    });
                });
            });
        self.spoken.open = open;
        if let Some(t) = seek {
            self.seek(t);
        }
        if find {
            self.spoken_find();
        }
        if cut {
            self.spoken_cut();
        }
    }

    fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let sel = self.selected.is_some();
        let s = self.s();
        let p = self.project();
        let list = p.timeline_list();
        let current = list.iter().position(|t| t.current).unwrap_or(0);
        let shown = self.timeline_label(&list[current].name, current);
        // inside a compound clip: the way back to the timeline that plays it
        let back = list[current].compound.then(|| {
            let outer = p.player_of(&p.timeline).map(|(id, _)| id.to_string());
            let outer = outer.and_then(|id| list.iter().position(|t| t.id == id));
            let outer = outer.or_else(|| list.iter().position(|t| !t.compound)).unwrap_or(0);
            fill(s.compound_back, &[&self.timeline_label(&list[outer].name, outer)])
        });
        let narrow = self.narrow;
        let tools = |ui: &mut egui::Ui| {
            if let Some(back) = &back
                && theme::tool(ui, icon::FRAME_BACK, back, s.compound_back_tip, true).clicked()
            {
                self.leave_compound();
            }
            let label = if list.len() > 1 { format!("{shown}  ({}/{})", current + 1, list.len()) } else { shown };
            if theme::tool(ui, icon::FILM, &label, s.timelines_tip, true).clicked() {
                self.timelines_open = !self.timelines_open;
                self.timeline_name.clear();
            }
            ui.separator();
            if theme::tool(ui, icon::SPLIT, s.split, s.split_tip, sel).clicked() {
                self.split();
            }
            if theme::tool(ui, icon::DELETE, s.delete, s.delete_tip, sel).clicked() {
                self.delete(false);
            }
            if theme::tool(ui, icon::DELETE, s.ripple_delete, s.ripple_delete_tip, sel).clicked() {
                self.delete(true);
            }
            ui.separator();
            if theme::tool(ui, icon::TEXT, s.title, s.title_tip, true).clicked() {
                self.add_title();
            }
            if theme::tool(ui, icon::COLOUR, s.colour, s.colour_tip, true).clicked() {
                self.add_colour();
            }
            self.record_button(ui);
            if theme::tool(ui, icon::SPARKLE, s.spoken, s.spoken_tip, true).clicked() {
                self.spoken.open = !self.spoken.open;
            }
            if narrow {
                ui.separator();
                self.timeline_view_tools(ui);
            } else {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.timeline_view_tools(ui));
            }
        };
        if narrow {
            // more tools than fit: the row scrolls sideways
            egui::ScrollArea::horizontal()
                .id_salt("timeline tools")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| ui.horizontal(tools));
        } else {
            ui.horizontal(tools);
        }
    }

    /// The tools of the timeline's view: snapping, the sound of scrubbing, zoom, row height.
    fn timeline_view_tools(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        if theme::tool(ui, icon::ROW_HEIGHT, "", s.row_height_tip, true).clicked() {
            let sizes = timeline::ROW_HEIGHTS;
            let at = sizes.iter().position(|h| *h == self.timeline.row).unwrap_or(1);
            self.timeline.row = sizes[(at + 1) % sizes.len()];
        }
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
        let scrub = self.scrub_sound;
        let ear = egui::Button::new(egui::RichText::new(icon::WAVEFORM).size(14.0).color(if scrub {
            theme::color::ACCENT
        } else {
            theme::color::WEAK
        }))
        .selected(scrub);
        if ui.add(ear).on_hover_text(if scrub { s.scrub_on } else { s.scrub_off }).clicked() {
            self.scrub_sound = !scrub;
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
    }

    /// The tabs of a narrow window: what shows under the preview.
    fn sheet_tabs(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        let tabs = [
            (Sheet::Timeline, icon::FILM, s.sheet_timeline),
            (Sheet::Media, icon::IMAGE, s.tab_media),
            (Sheet::Adjust, icon::SLIDERS, s.sheet_adjust),
        ];
        ui.horizontal_centered(|ui| {
            let gap = 6.0;
            ui.spacing_mut().item_spacing.x = gap;
            let w = ((ui.available_width() - gap * 2.0) / 3.0).floor();
            for (sheet, glyph, word) in tabs {
                let on = self.sheet == sheet;
                let tint = if on { theme::color::ACCENT } else { theme::color::TEXT };
                let text = egui::RichText::new(format!("{glyph}  {word}")).size(14.0).color(tint);
                if ui.add_sized([w, 28.0], egui::Button::new(text).selected(on)).clicked() {
                    self.sheet = sheet;
                }
            }
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
                    && !self.status.ends_with(s.done.trim_start_matches("{}"));
            let (glyph, tint) =
                if warn { (icon::WARNING, theme::color::WARN) } else { (icon::DONE, theme::color::WEAK) };
            ui.label(egui::RichText::new(format!("{glyph}  {}", self.status)).small().color(tint));
            if self.narrow {
                return;
            }
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
            // the list is longer than a short window: it scrolls, and the panel keeps its height
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                theme::section(ui, s.shortcuts, |ui| {
                    egui::Grid::new("keys").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                        for &(k, what) in s.keys {
                            ui.label(
                                egui::RichText::new(i18n::keys(ui.ctx(), k)).monospace().color(theme::color::TEXT),
                            );
                            ui.label(egui::RichText::new(what).color(theme::color::WEAK));
                            ui.end_row();
                        }
                    });
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
            ClipContent::Compound { timeline, .. } => p.timeline_sounds(timeline),
            _ => false,
        };
        let picture = p.clip(&id).is_some_and(|(ti, _)| p.tracks[ti].kind != TrackKind::Audio);
        let adjustment = clip.is_adjustment();
        // values are read, and keyframes set, where the playhead is (held inside the clip)
        let at = p.canvas.fps.snap(self.playhead);
        let local = (at - clip.start).max(Tick::ZERO).min(clip.duration);
        let half = p.canvas.fps.frame_duration().0 / 2;
        let mark = |prop: Prop| match clip.keys.get(&prop) {
            Some(keys) if keys.iter().any(|k| (k.t - (at - clip.start)).0.abs() <= half) => KeyMark::Here,
            Some(keys) if !keys.is_empty() => KeyMark::Elsewhere,
            _ => KeyMark::Off,
        };
        let marks: Vec<(Prop, KeyMark)> = Prop::ALL.into_iter().map(|p| (p, mark(p))).collect();
        let anim_of = |a: Option<reelforge_model::Animation>| a.map(|a| (a.kind, a.duration.seconds() as f32));
        let current = Inspector {
            clip: id.clone(),
            x: clip.value_at(Prop::X, local),
            y: clip.value_at(Prop::Y, local),
            scale: clip.value_at(Prop::Scale, local),
            rotation: clip.value_at(Prop::Rotation, local),
            opacity: clip.value_at(Prop::Opacity, local),
            color: match &clip.content {
                ClipContent::Text { color, .. } | ClipContent::Solid { color } => Some(*color),
                ClipContent::Media { .. } | ClipContent::Adjustment | ClipContent::Compound { .. } => None,
            },
            volume: has_sound.then(|| clip.value_at(Prop::VolumeDb, local)),
            fades: has_sound.then(|| [clip.fade.fade_in.seconds() as f32, clip.fade.fade_out.seconds() as f32]),
            speed: match &clip.content {
                ClipContent::Media { media, speed, reversed, curve, .. }
                    if p.media.get(media).is_some_and(|m| m.kind != MediaKind::Image) =>
                {
                    Some(SpeedEdit {
                        speed: *speed as f32,
                        keep_pitch: clip.keep_pitch,
                        reversed: *reversed,
                        curve: (!curve.is_empty()).then(|| CurvePreset::of(curve)),
                        frozen: *speed == 0.0,
                        video: p.media.get(media).is_some_and(|m| m.kind == MediaKind::Video)
                            && p.clip(&id).is_some_and(|(ti, _)| p.tracks[ti].kind == TrackKind::Video),
                    })
                }
                ClipContent::Media { speed, .. } if *speed == 0.0 => Some(SpeedEdit {
                    speed: 0.0,
                    keep_pitch: true,
                    reversed: false,
                    curve: None,
                    frozen: true,
                    video: false,
                }),
                _ => None,
            },
            grade: picture.then(|| GradeEdit {
                filter: clip.color.filter.map(|f| f.kind),
                filter_strength: clip.color.filter.map_or(1.0, |f| f.strength),
                lut: clip.color.lut.as_ref().map(|l| l.media.clone()),
                lut_strength: clip.color.lut.as_ref().map_or(1.0, |l| l.strength),
                adjust: clip.adjust_at(local).values(),
            }),
            fx: picture.then(|| clip.fx.clone()),
            adjustment,
            voice: (has_sound && matches!(clip.content, ClipContent::Media { silent: false, .. })).then(|| VoiceEdit {
                kind: clip.voice.kind,
                pitch: clip.voice.pitch,
                denoise: clip.voice.denoise.as_ref().map(|d| (d.amount * 100.0).round()),
            }),
            look: (picture && !adjustment).then(|| LookEdit {
                media: clip.compound().is_some()
                    || matches!(&clip.content, ClipContent::Media { media, .. }
                        if p.media.get(media).is_some_and(|m| m.kind != MediaKind::Audio)),
                crop: Prop::CROP.map(|p| clip.value_at(p, local) * 100.0),
                flip: [clip.look.flip_x, clip.look.flip_y],
                blend: clip.look.blend,
                mask: clip.mask_at(local),
                backdrop: (matches!(&clip.content, ClipContent::Media { media, .. }
                        if p.media.get(media).is_some_and(|m| m.kind != MediaKind::Audio))
                    && p.clip(&id).is_some_and(|(ti, _)| p.tracks[ti].kind == TrackKind::Video))
                .then_some(clip.look.backdrop),
                chroma: (matches!(&clip.content, ClipContent::Media { media, .. }
                        if p.media.get(media).is_some_and(|m| m.kind != MediaKind::Audio))
                    && p.clip(&id).is_some_and(|(ti, _)| p.tracks[ti].kind == TrackKind::Video))
                .then_some(clip.look.chroma),
            }),
            title: match &clip.content {
                ClipContent::Text { text, size, style, .. } => Some(Title {
                    text: text.clone(),
                    size: *size,
                    bold: style.weight >= 600,
                    outline: style.outline.is_some(),
                    boxed: style.background.is_some(),
                    shadow: style.shadow.is_some(),
                    line_boxes: style.background.is_some() && style.line_boxes,
                    font: style.font.clone(),
                    tracking: style.tracking * 100.0,
                    leading: style.leading * 100.0,
                    italic: style.italic,
                    underline: style.underline,
                }),
                _ => None,
            },
            anim: (picture && !adjustment).then(|| [anim_of(clip.anim.enter), anim_of(clip.anim.exit)]),
            looped: clip.anim.repeat.map(|l| (l.kind, l.period.seconds() as f32)),
            transition: p.clip(&id).filter(|(ti, _)| p.tracks[*ti].kind == TrackKind::Video && !adjustment).map(
                |(ti, _)| {
                    let touching = p.tracks[ti].clips.iter().any(|c| c.end() == clip.start && !c.is_adjustment());
                    (clip.transition.map(|t| (t.kind, t.duration.seconds() as f32)), touching)
                },
            ),
        };
        // reloaded when the project changes (a drag in the preview, undo, another editor) or the
        // playhead moves
        let seen = (self.session.revision(), at);
        if self.inspector.as_ref().is_none_or(|i| i.clip != id) || self.inspector_seen != seen {
            self.inspector = Some(current.clone());
            self.inspector_seen = seen;
        }
        let (glyph, kind) = match &clip.content {
            ClipContent::Media { media, .. } => match p.media.get(media).map(|m| m.kind) {
                Some(MediaKind::Image) => (icon::IMAGE, s.kind_picture),
                Some(MediaKind::Audio) => (icon::MUSIC, s.kind_sound),
                _ => (icon::VIDEO, s.kind_video),
            },
            ClipContent::Text { .. } => (icon::TEXT, s.kind_title),
            ClipContent::Solid { .. } => (icon::COLOUR, s.kind_colour),
            ClipContent::Adjustment => (icon::SLIDERS, s.kind_adjustment),
            ClipContent::Compound { .. } => (icon::FILM, s.kind_compound),
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
        // the template the title wears, if it wears one unchanged
        let worn = match &clip.content {
            ClipContent::Text { color, style, .. } => reelforge_engine::title_templates()
                .into_iter()
                .find(|t| t.color == *color && style.wearing(&t.style) == *style)
                .map(|t| t.id),
            _ => None,
        };
        let edit_is_title = matches!(clip.content, ClipContent::Text { .. });
        if edit_is_title && self.looks.is_none() {
            self.looks = Some(looks::previews(ui.ctx()));
        }
        let mut wear: Option<&'static str> = None;
        let looks = self.looks.as_deref().unwrap_or_default();
        // tools that work on several clips take the whole selection when this clip is in it
        let chosen = Some(self.selection()).filter(|all| all.contains(&id)).unwrap_or_else(|| vec![id.clone()]);
        let edit = self.inspector.as_mut().expect("set above");
        let mut commit = false;
        // commands the panel asks for beyond the values above
        let mut actions: Vec<(&'static str, Value)> = vec![];
        // the keyframes panel moves the playhead or shapes a curve
        let mut curve_ask: Option<curve::Ask> = None;
        // the title asks for a font file
        let mut pick_font = false;
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
        // a keyframe toggle for a property: filled where the playhead sits on one
        let key_button = |ui: &mut egui::Ui, m: KeyMark| -> bool {
            let (rect, r) = ui.allocate_exact_size(egui::vec2(18.0, 24.0), egui::Sense::click());
            let (fill, stroke) = match m {
                KeyMark::Here => (theme::color::ACCENT, theme::color::ACCENT),
                KeyMark::Elsewhere => (egui::Color32::TRANSPARENT, theme::color::ACCENT),
                KeyMark::Off => {
                    (egui::Color32::TRANSPARENT, if r.hovered() { theme::color::TEXT } else { theme::color::WEAK })
                }
            };
            timeline::diamond(ui.painter(), rect.center(), 5.0, fill, egui::Stroke::new(1.2, stroke));
            r.on_hover_text(if m == KeyMark::Here { s.key_remove } else { s.key_add }).clicked()
        };
        let mark_of = |prop: Prop| marks.iter().find(|(p, _)| *p == prop).map_or(KeyMark::Off, |(_, m)| *m);
        let toggle = |prop: Prop, m: KeyMark| -> (&'static str, Value) {
            let cmd = if m == KeyMark::Here { "clip.removeKey" } else { "clip.setKey" };
            (cmd, json!({"clip": id, "prop": prop.name(), "at": at}))
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(t) = &mut edit.title {
                theme::section(ui, s.sec_text, |ui| {
                    let r = ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY));
                    commit |= r.lost_focus();
                    egui::Grid::new("title").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new(s.font).color(theme::color::WEAK));
                        ui.horizontal(|ui| {
                            let fonts: Vec<(&str, &str)> = p
                                .media
                                .values()
                                .filter(|m| m.kind == MediaKind::Font)
                                .map(|m| (m.id.as_str(), m.name.as_str()))
                                .collect();
                            let shown = t
                                .font
                                .as_deref()
                                .and_then(|f| fonts.iter().find(|(id, _)| *id == f))
                                .map_or(s.font_default, |(_, name)| *name);
                            let was = t.font.clone();
                            egui::ComboBox::from_id_salt("font")
                                .selected_text(shown)
                                .width(ui.available_width() - 40.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut t.font, None, s.font_default);
                                    for (fid, name) in &fonts {
                                        ui.selectable_value(&mut t.font, Some(fid.to_string()), *name);
                                    }
                                });
                            commit |= t.font != was;
                            pick_font |= ui.button(icon::ADD).on_hover_text(s.font_import).clicked();
                        });
                        ui.end_row();
                        commit |= row(ui, s.size, &mut t.size, 1.0, 8.0..=600.0, " px");
                        let (lo, hi) = reelforge_model::TitleStyle::SPACING;
                        commit |= row(ui, s.tracking, &mut t.tracking, 1.0, lo * 100.0..=hi * 100.0, " %");
                        commit |= row(ui, s.leading, &mut t.leading, 1.0, lo * 100.0..=hi * 100.0, " %");
                    });
                    ui.horizontal(|ui| {
                        commit |= ui.toggle_value(&mut t.bold, s.bold).changed();
                        commit |= ui.toggle_value(&mut t.italic, s.italic).changed();
                        commit |= ui.toggle_value(&mut t.underline, s.underline).changed();
                    });
                    ui.horizontal(|ui| {
                        commit |= ui.toggle_value(&mut t.outline, s.outline).changed();
                        commit |= ui.toggle_value(&mut t.boxed, s.boxed).changed();
                        commit |= ui.toggle_value(&mut t.shadow, s.shadowed).changed();
                        if ui.toggle_value(&mut t.line_boxes, s.line_boxes).changed() {
                            // boxes per line need a box
                            t.boxed |= t.line_boxes;
                            commit = true;
                        }
                    });
                });
                ui.add_space(6.0);
                theme::section(ui, s.sec_templates, |ui| {
                    let names: std::collections::HashMap<&str, &str> =
                        reelforge_engine::title_templates().into_iter().map(|t| t.id).zip(s.template_names).collect();
                    let width = (ui.available_width() - 8.0) / 2.0;
                    let size = egui::vec2(width, width * looks::SIZE[1] as f32 / looks::SIZE[0] as f32);
                    egui::Grid::new("looks").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
                        for (i, (id, texture)) in looks.iter().enumerate() {
                            let on = worn == Some(*id);
                            let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
                            ui.painter().image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            let stroke = if on {
                                egui::Stroke::new(2.0, theme::color::ACCENT)
                            } else if r.hovered() {
                                egui::Stroke::new(1.0, theme::color::WEAK)
                            } else {
                                egui::Stroke::new(1.0, theme::color::LINE)
                            };
                            ui.painter().rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);
                            let r = r.on_hover_text(names.get(id).copied().unwrap_or(id));
                            if r.clicked() && !on {
                                wear = Some(id);
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
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
            if edit.adjustment {
                ui.label(egui::RichText::new(s.fx_adjustment_hint).small().color(theme::color::WEAK));
            }
            let adjustment = edit.adjustment;
            if let Some(list) = edit.fx.as_mut().filter(|_| adjustment) {
                ui.add_space(6.0);
                if let Some(a) = fx_section(ui, s, &id, list) {
                    actions.push(a);
                }
                ui.add_space(6.0);
            }
            if !adjustment {
                theme::section(ui, s.sec_transform, |ui| {
                    egui::Grid::new("transform").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
                        let rows = [
                            (Prop::X, s.pos_x, &mut edit.x, 0.005, -2.0..=2.0, ""),
                            (Prop::Y, s.pos_y, &mut edit.y, 0.005, -2.0..=2.0, ""),
                            (Prop::Scale, s.scale, &mut edit.scale, 0.01, 0.0..=10.0, "×"),
                            (Prop::Rotation, s.rotation, &mut edit.rotation, 0.5, -360.0..=360.0, "°"),
                            (Prop::Opacity, s.opacity, &mut edit.opacity, 0.01, 0.0..=1.0, ""),
                        ];
                        for (prop, name, v, speed, range, suffix) in rows {
                            ui.label(egui::RichText::new(name).color(theme::color::WEAK));
                            let r = ui.add_sized(
                                [ui.available_width() - 26.0, 24.0],
                                egui::DragValue::new(v).speed(speed).range(range).suffix(suffix),
                            );
                            commit |= r.drag_stopped() || (r.changed() && !r.dragged());
                            if key_button(ui, mark_of(prop)) {
                                actions.push(toggle(prop, mark_of(prop)));
                            }
                            ui.end_row();
                        }
                    });
                });
            }
            // how the values travel between the keyframes around the playhead
            if !clip.keys.is_empty() {
                ui.add_space(6.0);
                theme::section(ui, s.sec_keys, |ui| {
                    curve_ask = curve::show(ui, s, clip, local, half);
                });
            }
            if let Some(g) = &mut edit.grade {
                ui.add_space(6.0);
                let changed = |r: &egui::Response| r.drag_stopped() || (r.changed() && !r.dragged());
                theme::section(ui, s.sec_grade, |ui| {
                    egui::Grid::new("grade").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new(s.filter).color(theme::color::WEAK));
                        ui.horizontal(|ui| {
                            let name =
                                |k: FilterKind| s.filters[FilterKind::ALL.iter().position(|x| *x == k).unwrap_or(0)];
                            let mut picked = g.filter;
                            egui::ComboBox::from_id_salt("filter")
                                .selected_text(g.filter.map_or(s.none, name))
                                .width(92.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut picked, None, s.none);
                                    for k in FilterKind::ALL {
                                        ui.selectable_value(&mut picked, Some(k), name(k));
                                    }
                                });
                            if picked != g.filter {
                                actions.push((
                                    "clip.setColor",
                                    json!({"clip": id, "filter": picked.map(FilterKind::name)}),
                                ));
                            }
                            if g.filter.is_some() {
                                let r = ui.add(
                                    egui::DragValue::new(&mut g.filter_strength)
                                        .speed(0.01)
                                        .range(0.0..=1.0)
                                        .max_decimals(2),
                                );
                                if changed(&r) {
                                    actions.push((
                                        "clip.setColor",
                                        json!({"clip": id, "filterStrength": g.filter_strength}),
                                    ));
                                }
                            }
                        });
                        ui.end_row();
                        // an adjustment clip takes no lookup table
                        if !adjustment {
                            ui.label(egui::RichText::new(s.lut).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                let luts: Vec<(&str, &str)> = p
                                    .media
                                    .values()
                                    .filter(|m| m.kind == MediaKind::Lut)
                                    .map(|m| (m.id.as_str(), m.name.as_str()))
                                    .collect();
                                if luts.is_empty() {
                                    ui.label(egui::RichText::new(s.lut_hint).color(theme::color::WEAK).small());
                                    return;
                                }
                                let shown = g
                                    .lut
                                    .as_deref()
                                    .and_then(|l| luts.iter().find(|(id, _)| *id == l))
                                    .map_or(s.none, |(_, name)| *name);
                                let mut picked = g.lut.clone();
                                egui::ComboBox::from_id_salt("lut").selected_text(shown).width(92.0).show_ui(
                                    ui,
                                    |ui| {
                                        ui.selectable_value(&mut picked, None, s.none);
                                        for (lid, name) in &luts {
                                            ui.selectable_value(&mut picked, Some(lid.to_string()), *name);
                                        }
                                    },
                                );
                                if picked != g.lut {
                                    actions.push(("clip.setColor", json!({"clip": id, "lut": picked})));
                                }
                                if g.lut.is_some() {
                                    let r = ui.add(
                                        egui::DragValue::new(&mut g.lut_strength)
                                            .speed(0.01)
                                            .range(0.0..=1.0)
                                            .max_decimals(2),
                                    );
                                    if changed(&r) {
                                        actions.push((
                                            "clip.setColor",
                                            json!({"clip": id, "lutStrength": g.lut_strength}),
                                        ));
                                    }
                                }
                            });
                            ui.end_row();
                        }
                        for (i, (name, lo, hi)) in Adjust::RANGES.into_iter().enumerate() {
                            ui.label(egui::RichText::new(s.adjust_names[i]).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                let r = ui.add_sized(
                                    [ui.available_width() - 26.0, 22.0],
                                    egui::DragValue::new(&mut g.adjust[i]).speed(0.01).range(lo..=hi).max_decimals(2),
                                );
                                if changed(&r) {
                                    actions.push(("clip.setColor", json!({"clip": id, name: g.adjust[i], "at": at})));
                                }
                                let prop = Prop::ADJUST[i];
                                // an adjustment clip's colour steps in and out; it has no keyframes
                                if !adjustment && key_button(ui, mark_of(prop)) {
                                    actions.push(toggle(prop, mark_of(prop)));
                                }
                            });
                            ui.end_row();
                        }
                    });
                    let untouched = g.filter.is_none() && g.lut.is_none() && g.adjust.iter().all(|v| *v == 0.0);
                    if !untouched && ui.button(s.grade_reset).clicked() {
                        actions.push(("clip.setColor", json!({"clip": id, "reset": true})));
                    }
                });
            }
            if let Some(l) = &mut edit.look {
                ui.add_space(6.0);
                let changed = |r: &egui::Response| r.drag_stopped() || (r.changed() && !r.dragged());
                theme::section(ui, s.sec_look, |ui| {
                    egui::Grid::new("look").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new(s.blend).color(theme::color::WEAK));
                        let name = |m: BlendMode| s.blends[BlendMode::ALL.iter().position(|x| *x == m).unwrap_or(0)];
                        let mut picked = l.blend;
                        egui::ComboBox::from_id_salt("blend").selected_text(name(l.blend)).width(120.0).show_ui(
                            ui,
                            |ui| {
                                for m in BlendMode::ALL {
                                    ui.selectable_value(&mut picked, m, name(m));
                                }
                            },
                        );
                        if picked != l.blend {
                            actions.push(("clip.setBlend", json!({"clip": id, "mode": picked.name()})));
                        }
                        ui.end_row();
                        if let Some(backdrop) = &mut l.backdrop {
                            ui.label(egui::RichText::new(s.canvas_blur).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                let mut on = backdrop.is_some();
                                if ui.checkbox(&mut on, "").on_hover_text(s.canvas_blur_tip).changed() {
                                    let blur = if on { json!(0.5) } else { Value::Null };
                                    actions.push(("clip.setBackdrop", json!({"clip": id, "blur": blur})));
                                }
                                if let Some(amount) = backdrop {
                                    let r = ui.add(egui::Slider::new(amount, 0.0..=1.0).show_value(false));
                                    if changed(&r) {
                                        actions.push(("clip.setBackdrop", json!({"clip": id, "blur": *amount})));
                                    }
                                }
                                if ui.small_button(s.apply_all).on_hover_text(s.apply_all_tip).clicked() {
                                    let blur = backdrop.map_or(Value::Null, |b| json!(b));
                                    actions.push(("clip.setBackdrop", json!({"clip": id, "blur": blur, "all": true})));
                                }
                            });
                            ui.end_row();
                        }
                        if let Some(chroma) = &mut l.chroma {
                            ui.label(egui::RichText::new(s.chroma).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                let mut on = chroma.is_some();
                                if ui.checkbox(&mut on, "").on_hover_text(s.chroma_tip).changed() {
                                    if on {
                                        actions.push(("clip.pickChromaKey", json!({"clip": id})));
                                    } else {
                                        actions.push(("clip.setChromaKey", json!({"clip": id, "color": null})));
                                    }
                                }
                                if let Some(k) = chroma {
                                    // the colour as on screen; it is sent once the picker is let go
                                    let mut rgb = k.color.map(|v| (v * 255.0).round() as u8);
                                    if ui.color_edit_button_srgb(&mut rgb).on_hover_text(s.chroma_color).changed() {
                                        k.color = rgb.map(|v| f32::from(v) / 255.0);
                                    }
                                    if ui.small_button(s.chroma_pick).on_hover_text(s.chroma_pick_tip).clicked() {
                                        actions.push(("clip.pickChromaKey", json!({"clip": id})));
                                    }
                                }
                            });
                            ui.end_row();
                            if let Some(k) = chroma {
                                let rows = [
                                    (s.chroma_names[0], "tolerance", &mut k.tolerance),
                                    (s.chroma_names[1], "softness", &mut k.softness),
                                    (s.chroma_names[2], "spill", &mut k.spill),
                                ];
                                for (label, name, value) in rows {
                                    ui.label(egui::RichText::new(label).color(theme::color::WEAK));
                                    let mut percent = *value * 100.0;
                                    let r = ui.add_sized(
                                        [ui.available_width(), 22.0],
                                        egui::DragValue::new(&mut percent).speed(0.5).range(0.0..=100.0).suffix(" %"),
                                    );
                                    *value = percent / 100.0;
                                    if changed(&r) {
                                        actions.push(("clip.setChromaKey", json!({"clip": id, name: *value})));
                                    }
                                    ui.end_row();
                                }
                            }
                        }
                        if l.media {
                            ui.label(egui::RichText::new(s.crop).color(theme::color::WEAK));
                            let on = self.cropping.as_deref() == Some(id.as_str());
                            if ui
                                .add(egui::Button::new(format!("{}  {}", icon::CROP, s.crop_in_preview)).selected(on))
                                .on_hover_text(s.crop_in_preview_tip)
                                .clicked()
                            {
                                self.cropping = (!on).then(|| id.clone());
                            }
                            ui.end_row();
                            for (i, edge) in ["left", "top", "right", "bottom"].into_iter().enumerate() {
                                ui.label(egui::RichText::new(s.crop_edges[i]).color(theme::color::WEAK));
                                ui.horizontal(|ui| {
                                    let r = ui.add_sized(
                                        [ui.available_width() - 26.0, 22.0],
                                        egui::DragValue::new(&mut l.crop[i]).speed(0.2).range(0.0..=95.0).suffix(" %"),
                                    );
                                    if changed(&r) {
                                        actions.push((
                                            "clip.setCrop",
                                            json!({"clip": id, edge: l.crop[i] / 100.0, "at": at}),
                                        ));
                                    }
                                    let prop = Prop::CROP[i];
                                    if key_button(ui, mark_of(prop)) {
                                        actions.push(toggle(prop, mark_of(prop)));
                                    }
                                });
                                ui.end_row();
                            }
                            ui.label(egui::RichText::new(s.flip).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                if ui.selectable_label(l.flip[0], s.flip_x).clicked() {
                                    actions.push(("clip.setCrop", json!({"clip": id, "flipX": !l.flip[0]})));
                                }
                                if ui.selectable_label(l.flip[1], s.flip_y).clicked() {
                                    actions.push(("clip.setCrop", json!({"clip": id, "flipY": !l.flip[1]})));
                                }
                            });
                            ui.end_row();
                        }
                        ui.label(egui::RichText::new(s.mask).color(theme::color::WEAK));
                        ui.horizontal(|ui| {
                            let name = |k: MaskKind| s.masks[MaskKind::ALL.iter().position(|x| *x == k).unwrap_or(0)];
                            let kind = l.mask.map(|m| m.kind);
                            let mut picked = kind;
                            egui::ComboBox::from_id_salt("mask")
                                .selected_text(kind.map_or(s.none, name))
                                .width(92.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut picked, None, s.none);
                                    for k in MaskKind::ALL {
                                        ui.selectable_value(&mut picked, Some(k), name(k));
                                    }
                                });
                            if picked != kind {
                                actions
                                    .push(("clip.setMask", json!({"clip": id, "shape": picked.map(MaskKind::name)})));
                            }
                            if let Some(m) = &l.mask
                                && ui.selectable_label(m.invert, s.mask_invert).clicked()
                            {
                                actions.push(("clip.setMask", json!({"clip": id, "invert": !m.invert})));
                            }
                        });
                        ui.end_row();
                        if let Some(m) = &mut l.mask {
                            let [cx, cy] = &mut m.center;
                            let [w, h] = &mut m.size;
                            let rows: [(&str, &mut f32, f64, std::ops::RangeInclusive<f32>); 6] = [
                                ("x", cx, 0.005, -1.0..=2.0),
                                ("y", cy, 0.005, -1.0..=2.0),
                                ("width", w, 0.005, 0.0..=4.0),
                                ("height", h, 0.005, 0.0..=4.0),
                                ("rotation", &mut m.rotation, 0.5, -360.0..=360.0),
                                ("feather", &mut m.feather, 0.002, 0.0..=1.0),
                            ];
                            for (i, (key, v, speed, range)) in rows.into_iter().enumerate() {
                                ui.label(egui::RichText::new(s.mask_params[i]).color(theme::color::WEAK));
                                ui.horizontal(|ui| {
                                    let r = ui.add_sized(
                                        [ui.available_width() - 26.0, 22.0],
                                        egui::DragValue::new(v).speed(speed).range(range).max_decimals(3),
                                    );
                                    if changed(&r) {
                                        actions.push(("clip.setMask", json!({"clip": id, key: *v, "at": at})));
                                    }
                                    let prop = Prop::MASK[i];
                                    if key_button(ui, mark_of(prop)) {
                                        actions.push(toggle(prop, mark_of(prop)));
                                    }
                                });
                                ui.end_row();
                            }
                        }
                    });
                });
            }
            if let Some(v) = &mut edit.volume {
                ui.add_space(6.0);
                theme::section(ui, s.sec_sound, |ui| {
                    egui::Grid::new("sound").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new(s.volume).color(theme::color::WEAK));
                        let r = ui.add_sized(
                            [ui.available_width() - 26.0, 24.0],
                            egui::DragValue::new(v).speed(0.2).range(-60.0..=12.0).suffix(" dB"),
                        );
                        commit |= r.drag_stopped() || (r.changed() && !r.dragged());
                        if key_button(ui, mark_of(Prop::VolumeDb)) {
                            actions.push(toggle(Prop::VolumeDb, mark_of(Prop::VolumeDb)));
                        }
                        ui.end_row();
                        if let Some(f) = &mut edit.fades {
                            for (name, v) in [s.fade_in, s.fade_out].into_iter().zip(f.iter_mut()) {
                                ui.label(egui::RichText::new(name).color(theme::color::WEAK));
                                let r = ui.add_sized(
                                    [ui.available_width() - 26.0, 24.0],
                                    egui::DragValue::new(v).speed(0.02).range(0.0..=10.0).suffix(" s").max_decimals(2),
                                );
                                commit |= r.drag_stopped() || (r.changed() && !r.dragged());
                                ui.label("");
                                ui.end_row();
                            }
                        }
                        if let Some(voice) = &mut edit.voice {
                            ui.label(egui::RichText::new(s.voice).color(theme::color::WEAK));
                            let name = |k: Option<VoiceKind>| {
                                k.and_then(|k| VoiceKind::ALL.iter().position(|x| *x == k))
                                    .map_or(s.voice_none, |i| s.voice_names[i])
                            };
                            let mut picked = voice.kind;
                            egui::ComboBox::from_id_salt("voice").selected_text(name(voice.kind)).width(110.0).show_ui(
                                ui,
                                |ui| {
                                    ui.selectable_value(&mut picked, None, s.voice_none);
                                    for (k, label) in VoiceKind::ALL.into_iter().zip(s.voice_names.iter()) {
                                        ui.selectable_value(&mut picked, Some(k), *label);
                                    }
                                },
                            );
                            if picked != voice.kind {
                                actions
                                    .push(("clip.setVoice", json!({"clip": id, "voice": picked.map(VoiceKind::name)})));
                            }
                            ui.label("");
                            ui.end_row();
                            ui.label(egui::RichText::new(s.pitch).color(theme::color::WEAK));
                            let r = ui.add_sized(
                                [ui.available_width() - 26.0, 24.0],
                                egui::DragValue::new(&mut voice.pitch).speed(0.1).range(-12.0..=12.0).max_decimals(1),
                            );
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                actions.push(("clip.setVoice", json!({"clip": id, "pitch": voice.pitch})));
                            }
                            ui.label("");
                            ui.end_row();
                            ui.label(egui::RichText::new(s.denoise).color(theme::color::WEAK));
                            ui.horizontal(|ui| {
                                let mut on = voice.denoise.is_some();
                                if ui.checkbox(&mut on, "").on_hover_text(s.denoise_tip).changed() {
                                    actions.push(if on {
                                        ("audio.denoise", json!({"clip": id}))
                                    } else {
                                        ("clip.setDenoise", json!({"clip": id, "amount": 0}))
                                    });
                                }
                                if let Some(amount) = &mut voice.denoise {
                                    let r = ui.add(
                                        egui::DragValue::new(amount)
                                            .speed(1.0)
                                            .range(5.0..=100.0)
                                            .suffix(" %")
                                            .max_decimals(0),
                                    );
                                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                        actions
                                            .push(("clip.setDenoise", json!({"clip": id, "amount": *amount / 100.0})));
                                    }
                                }
                            });
                            ui.label("");
                            ui.end_row();
                        }
                    });
                    if edit.voice.is_some() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(s.loudness).on_hover_text(s.loudness_tip).clicked() {
                                actions.push(("audio.normalize", json!({"clips": chosen})));
                            }
                            ui.menu_button(s.beats, |ui| {
                                for (n, label) in [1, 2, 4].into_iter().zip(s.beats_every) {
                                    if ui.button(label).clicked() {
                                        actions.push(("audio.beats", json!({"clip": id, "every": n})));
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui.button(s.beats_clear).clicked() {
                                    actions.push(("audio.beats", json!({"clip": id, "clear": true})));
                                    ui.close();
                                }
                            })
                            .response
                            .on_hover_text(s.beats_tip);
                            if ui.button(s.duck).on_hover_text(s.duck_tip).clicked() {
                                actions.push(("audio.duck", json!({"clip": id})));
                            }
                        });
                    }
                });
            }
            if let Some(sp) = &mut edit.speed {
                ui.add_space(6.0);
                theme::section(ui, s.sec_speed, |ui| {
                    if sp.frozen {
                        ui.label(egui::RichText::new(s.frozen).color(theme::color::WEAK));
                        return;
                    }
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::DragValue::new(&mut sp.speed)
                                .speed(0.01)
                                .range(0.1..=10.0)
                                .suffix("×")
                                .max_decimals(2),
                        );
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            actions.push(("clip.setSpeed", json!({"clip": id, "speed": sp.speed})));
                        }
                        if edit.fades.is_some() && ui.toggle_value(&mut sp.keep_pitch, s.keep_pitch).changed() {
                            actions.push(("clip.setSpeed", json!({"clip": id, "keepPitch": sp.keep_pitch})));
                        }
                        if ui.toggle_value(&mut sp.reversed, s.reverse).changed() {
                            actions.push(("clip.setReverse", json!({"clip": id, "reversed": sp.reversed})));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(s.curve).color(theme::color::WEAK));
                        let name = |c: Option<CurvePreset>| {
                            c.and_then(|c| CurvePreset::ALL.iter().position(|x| *x == c))
                                .map_or(s.curve_custom, |i| s.curves[i])
                        };
                        let shown = sp.curve.map_or(s.curve_none, name);
                        let mut picked = sp.curve;
                        egui::ComboBox::from_id_salt("curve").selected_text(shown).width(110.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut picked, None, s.curve_none);
                            for (c, label) in CurvePreset::ALL.into_iter().zip(s.curves.iter()) {
                                ui.selectable_value(&mut picked, Some(Some(c)), *label);
                            }
                        });
                        if picked != sp.curve {
                            let curve = match picked {
                                None => Value::Null,
                                Some(c) => json!(c.map(CurvePreset::name)),
                            };
                            if !curve.is_null() || sp.curve.is_some() {
                                actions.push(("clip.setSpeedCurve", json!({"clip": id, "curve": curve})));
                            }
                        }
                        if sp.video && ui.button(s.freeze).on_hover_text(s.freeze_tip).clicked() {
                            actions.push(("clip.freeze", json!({"clip": id, "at": at})));
                        }
                    });
                });
            }
            let titled = edit.title.is_some();
            let looped = &mut edit.looped;
            if let Some(slots) = &mut edit.anim {
                ui.add_space(6.0);
                theme::section(ui, s.sec_motion, |ui| {
                    egui::Grid::new("motion").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
                        for (i, slot) in slots.iter_mut().enumerate() {
                            let (label, names, slot_id) = if i == 0 {
                                (s.anim_in, &s.anim_kinds, "in")
                            } else {
                                (s.anim_out, &s.anim_out_kinds, "out")
                            };
                            ui.label(egui::RichText::new(label).color(theme::color::WEAK));
                            let shown = slot.map_or(s.anim_none, |(k, _)| {
                                names[AnimKind::ALL.iter().position(|x| *x == k).unwrap_or(0)]
                            });
                            let mut picked = slot.map(|(k, _)| k);
                            egui::ComboBox::from_id_salt(("anim", i)).selected_text(shown).width(110.0).show_ui(
                                ui,
                                |ui| {
                                    ui.selectable_value(&mut picked, None, s.anim_none);
                                    for (k, name) in AnimKind::ALL.into_iter().zip(names.iter()) {
                                        if !k.titles_only() || titled {
                                            ui.selectable_value(&mut picked, Some(k), *name);
                                        }
                                    }
                                },
                            );
                            if picked != slot.map(|(k, _)| k) {
                                actions.push((
                                    "clip.setAnimation",
                                    json!({"clip": id, "slot": slot_id, "kind": picked.map(AnimKind::name)}),
                                ));
                            }
                            match slot {
                                Some((kind, secs)) => {
                                    let r = ui.add(
                                        egui::DragValue::new(secs)
                                            .speed(0.02)
                                            .range(0.1..=5.0)
                                            .suffix(" s")
                                            .max_decimals(2),
                                    );
                                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                        actions.push((
                                            "clip.setAnimation",
                                            json!({"clip": id, "slot": slot_id,
                                                "kind": kind.name(), "duration": {"seconds": *secs}}),
                                        ));
                                    }
                                    r.on_hover_text(s.anim_length);
                                }
                                None => {
                                    ui.label("");
                                }
                            }
                            ui.end_row();
                        }
                        // the one that goes round and round in between
                        ui.label(egui::RichText::new(s.anim_loop).color(theme::color::WEAK));
                        let name = |k: LoopKind| s.loop_kinds[LoopKind::ALL.iter().position(|x| *x == k).unwrap_or(0)];
                        let mut picked = looped.map(|(k, _)| k);
                        egui::ComboBox::from_id_salt("anim loop")
                            .selected_text(picked.map_or(s.anim_none, name))
                            .width(110.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut picked, None, s.anim_none);
                                for k in LoopKind::ALL {
                                    if !k.titles_only() || titled {
                                        ui.selectable_value(&mut picked, Some(k), name(k));
                                    }
                                }
                            });
                        if picked != looped.map(|(k, _)| k) {
                            actions.push((
                                "clip.setAnimation",
                                json!({"clip": id, "slot": "loop", "kind": picked.map(LoopKind::name)}),
                            ));
                        }
                        match looped {
                            Some((kind, secs)) => {
                                let r = ui.add(
                                    egui::DragValue::new(secs)
                                        .speed(0.02)
                                        .range(0.05..=20.0)
                                        .suffix(" s")
                                        .max_decimals(2),
                                );
                                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                    actions.push((
                                        "clip.setAnimation",
                                        json!({"clip": id, "slot": "loop",
                                            "kind": kind.name(), "duration": {"seconds": *secs}}),
                                    ));
                                }
                                r.on_hover_text(s.anim_round);
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    });
                });
            }
            if let Some(list) = edit.fx.as_mut().filter(|_| !adjustment) {
                ui.add_space(6.0);
                if let Some(a) = fx_section(ui, s, &id, list) {
                    actions.push(a);
                }
            }
            if let Some((current_t, touching)) = &mut edit.transition {
                ui.add_space(6.0);
                theme::section(ui, s.sec_transition, |ui| {
                    if !*touching {
                        ui.label(egui::RichText::new(s.transition_needs_neighbour).small().color(theme::color::WEAK));
                        return;
                    }
                    ui.horizontal(|ui| {
                        let shown = current_t.map_or(s.anim_none, |(k, _)| {
                            s.transition_kinds[TransitionKind::ALL.iter().position(|x| *x == k).unwrap_or(0)]
                        });
                        let mut picked = current_t.map(|(k, _)| k);
                        egui::ComboBox::from_id_salt("transition").selected_text(shown).width(130.0).show_ui(
                            ui,
                            |ui| {
                                ui.selectable_value(&mut picked, None, s.anim_none);
                                for (k, name) in TransitionKind::ALL.into_iter().zip(s.transition_kinds.iter()) {
                                    ui.selectable_value(&mut picked, Some(k), *name);
                                }
                            },
                        );
                        if picked != current_t.map(|(k, _)| k) {
                            actions.push((
                                "clip.setTransition",
                                json!({"clip": id, "kind": picked.map(TransitionKind::name)}),
                            ));
                        }
                        if let Some((kind, secs)) = current_t {
                            let r = ui.add(
                                egui::DragValue::new(secs).speed(0.02).range(0.1..=3.0).suffix(" s").max_decimals(2),
                            );
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                actions.push((
                                    "clip.setTransition",
                                    json!({"clip": id, "kind": kind.name(), "duration": {"seconds": *secs}}),
                                ));
                            }
                            r.on_hover_text(s.anim_length);
                        }
                    });
                });
            }
        });
        // a colour is committed once the pointer lets go of the picker
        let picked = self.inspector.as_ref().is_some_and(|e| e.color != current.color);
        commit |= picked && !ui.ctx().egui_is_using_pointer();
        for (cmd, params) in actions {
            if cmd == "clip.pickChromaKey" {
                self.key_out(&id);
            } else if cmd.starts_with("audio.") {
                self.listen(cmd, &params);
            } else {
                let _ = self.execute(cmd, params);
            }
        }
        match curve_ask {
            Some(curve::Ask::Jump(t)) => self.seek(clip.start + t),
            Some(curve::Ask::Shape(t, ease)) => {
                // a curve with a name is sent by it
                let ease = ease.name().map_or_else(|| json!(ease.points()), |n| json!(n));
                let _ = self.execute("clip.setEase", json!({"clip": id, "at": clip.start + t, "ease": ease}));
            }
            None => {}
        }
        // a key colour is sent once the pointer lets go of the picker
        let key_color = |i: &Inspector| i.look.as_ref().and_then(|l| l.chroma.flatten()).map(|k| k.color);
        if let (Some(now), Some(was)) = (self.inspector.as_ref().and_then(key_color), key_color(&current))
            && now != was
            && !ui.ctx().egui_is_using_pointer()
        {
            let _ = self.execute("clip.setChromaKey", json!({"clip": id, "color": now}));
        }
        if commit && self.inspector.as_ref() != Some(&current) {
            let e = self.inspector.clone().expect("set");
            // only what changed is sent, at the playhead, so keyframed values change only there
            let mut params = json!({"clip": id, "at": at});
            for (prop, now, was) in [
                (Prop::X, e.x, current.x),
                (Prop::Y, e.y, current.y),
                (Prop::Scale, e.scale, current.scale),
                (Prop::Rotation, e.rotation, current.rotation),
                (Prop::Opacity, e.opacity, current.opacity),
            ] {
                if now != was {
                    params[prop.name()] = json!(now);
                }
            }
            if let Some(c) = e.color
                && e.color != current.color
            {
                params["color"] = json!(c);
            }
            if let Some(v) = e.volume
                && e.volume != current.volume
            {
                params["volumeDb"] = json!(v);
            }
            if let (Some(f), Some(was)) = (e.fades, current.fades) {
                if f[0] != was[0] {
                    params["fadeIn"] = json!({"seconds": f[0]});
                }
                if f[1] != was[1] {
                    params["fadeOut"] = json!({"seconds": f[1]});
                }
            }
            if let Some(t) = &e.title
                && e.title != current.title
            {
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
                if was.is_some_and(|w| w.shadow != t.shadow) {
                    params["shadow"] = if t.shadow {
                        json!({"color": [0.0, 0.0, 0.0, 0.6], "offset": [0.03, 0.06], "blur": 0.08})
                    } else {
                        Value::Null
                    };
                }
                if was.is_some_and(|w| w.line_boxes != t.line_boxes) {
                    params["lineBoxes"] = json!(t.line_boxes);
                }
                if was.is_some_and(|w| w.font != t.font) {
                    params["font"] = json!(t.font);
                }
                if was.is_some_and(|w| w.tracking != t.tracking) {
                    params["tracking"] = json!(t.tracking / 100.0);
                }
                if was.is_some_and(|w| w.leading != t.leading) {
                    params["leading"] = json!(t.leading / 100.0);
                }
                if was.is_some_and(|w| w.italic != t.italic) {
                    params["italic"] = json!(t.italic);
                }
                if was.is_some_and(|w| w.underline != t.underline) {
                    params["underline"] = json!(t.underline);
                }
            }
            if self.execute("clip.set", params).is_err() {
                self.inspector = Some(current);
            }
        }
        if let Some(t) = wear {
            let _ = self.execute("clip.set", json!({"clip": id, "template": t}));
        }
        if pick_font {
            self.second_captions = false;
            self.pick(ui.ctx(), FONTS, false);
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let mut p = self.project();
        // a gesture in progress is drawn from a copy with the clip's new placement
        if let Some(g) = &self.gesture
            && let Some(clip) = Arc::make_mut(&mut p).clip_mut(&g.clip)
        {
            clip.transform = g.now;
            for prop in [Prop::X, Prop::Y, Prop::Scale, Prop::Rotation] {
                clip.keys.remove(&prop);
            }
        }
        // a mask being dragged is drawn where it is dragged to
        if let Some(g) = &self.gesture
            && let Some(m) = g.mask()
            && let Some(clip) = Arc::make_mut(&mut p).clip_mut(&g.clip)
        {
            clip.look.mask = Some(m);
            clip.keys.retain(|prop, _| !Prop::MASK.contains(prop));
        }
        // crop mode shows the whole picture, with the crop as a box over it
        if self.cropping.is_some() && self.cropping != self.selected {
            self.cropping = None;
        }
        if let Some(id) = &self.cropping
            && let Some(clip) = Arc::make_mut(&mut p).clip_mut(id)
        {
            clip.look.crop = Default::default();
            clip.keys.retain(|prop, _| !Prop::CROP.contains(prop));
        }
        let frame = p.canvas.fps.frame_at(self.playhead);
        let moving = (self.gesture.is_some() || self.cropping.is_some()).then(|| {
            let t = self.gesture.as_ref().map_or([0.0; 4], |g| [g.now.x, g.now.y, g.now.scale, g.now.rotation]);
            let m = self.gesture.as_ref().and_then(stage::Gesture::mask);
            let m = m.map_or([0.0; 5], |m| [m.center[0], m.center[1], m.size[0], m.size[1], m.rotation]);
            let mode = [self.cropping.is_some() as u32 + 2 * self.gesture.is_some() as u32];
            let mut all = [0u32; 10];
            for (slot, v) in all.iter_mut().zip(t.iter().chain(&m).map(|v| v.to_bits()).chain(mode)) {
                *slot = v;
            }
            all
        });
        let revision = self.session.revision();
        let long = p.canvas.width.max(p.canvas.height) as f32;
        // as many pixels as the frame covers on this screen
        let area = ui.available_rect_before_wrap();
        let fit = (area.width() / p.canvas.width.max(1) as f32).min(area.height() / p.canvas.height.max(1) as f32);
        let on_screen = long * fit * self.zoom * ui.ctx().pixels_per_point();
        let mut scale = preview_scale(long, on_screen, self.gpu.is_some(), self.playing, self.full_quality);
        if let Some(g) = &self.gpu {
            // no larger than the device draws (2048 on WebGL2)
            scale = scale.min((g.compositor.max_side() as f32 / long.max(1.0) * 32.0).floor() / 32.0);
        }
        let epoch = self.remote.as_ref().map_or(0, remote::Remote::media_epoch);
        let key = (revision, frame, epoch, moving, self.playing, scale.to_bits());
        let changed = self.preview_key != Some(key);
        self.preview_key = Some(key);
        let at = p.canvas.fps.snap(self.playhead);
        // an empty timeline does not compile: the background is drawn alone
        if changed && self.preview_spec.as_ref().is_none_or(|(k, _)| *k != (revision, moving)) {
            let spec = compile(&p, &CompileOptions { encode: false }).ok().map(Arc::new);
            self.preview_spec = Some(((revision, moving), spec));
        }
        let spec = if changed { self.preview_spec.as_ref().and_then(|(_, s)| s.clone()) } else { None };
        if changed && self.playing {
            // nothing can be picked while playing; read ahead instead
            if let Some(spec) = &spec {
                self.ahead.follow(spec, at, scale);
            }
        } else if changed {
            // layers that are not clips (a transition's colour dip) cannot be picked
            self.outlines = spec
                .as_ref()
                .map(|spec| reelforge_render::layer_outlines(spec, at, &*self.media))
                .unwrap_or_default()
                .into_iter()
                .filter(|(id, _)| !id.contains('~'))
                .collect();
        }
        if changed && let Some(g) = &mut self.gpu {
            g.texture = None;
            if let Some(spec) = &spec {
                match g.compositor.render(spec, at, scale, &*self.media, Quality::Draft) {
                    Ok(_report) => g.show(),
                    Err(e) => self.status = fill(self.s().preview_failed, &[&e]),
                }
            }
        }
        let shown = if let Some((id, [w, h])) = self.gpu.as_ref().and_then(|g| g.texture) {
            Some((id, [w as f32, h as f32]))
        } else {
            if self.gpu.is_none() && (changed || self.preview.is_none()) {
                let image = match &spec {
                    Some(spec) => match render_frame(spec, at, scale, &*self.media, Quality::Draft) {
                        Ok((img, _report)) => Some(img),
                        Err(e) => {
                            self.status = fill(self.s().preview_failed, &[&e]);
                            None
                        }
                    },
                    None => None,
                };
                let color = match image {
                    Some(img) => egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.to_srgba8(),
                    ),
                    None => {
                        let s = scale;
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
            self.preview.as_ref().map(|t| (t.id(), t.size().map(|v| v as f32)))
        };
        let Some((id, [w, h])) = shown else { return };
        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, egui::Sense::click_and_drag());
        // Ctrl + wheel (or a pinch) zooms around the pointer; the middle button pans
        if response.hovered() {
            let (z, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
            if z != 1.0 {
                let zoom = (self.zoom * z).clamp(0.25, 8.0);
                let at = pointer.unwrap_or(area.center()) - area.center();
                self.pan = at - (at - self.pan) * (zoom / self.zoom);
                self.zoom = zoom;
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            self.pan += response.drag_delta();
        }
        if self.zoom == 1.0 && self.pan.length() < 1.0 {
            self.pan = egui::Vec2::ZERO;
        }
        let s = (area.width() / w).min(area.height() / h) * self.zoom;
        let frame_rect = egui::Rect::from_center_size(area.center() + self.pan, egui::vec2(w * s, h * s));
        ui.painter_at(area).image(
            id,
            frame_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // back to fitting the panel
        if self.zoom != 1.0 || self.pan != egui::Vec2::ZERO {
            let label = format!("{}  {:.0}%", icon::FIT, self.zoom * 100.0);
            let r = egui::Rect::from_min_size(area.left_top() + egui::vec2(6.0, 6.0), egui::vec2(84.0, 22.0));
            if ui
                .put(r, egui::Button::new(egui::RichText::new(label).small()))
                .on_hover_text(self.s().fit_tip)
                .clicked()
            {
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
            }
        }
        if self.playing {
            return;
        }
        let view = stage::View { frame: frame_rect, k: frame_rect.width() / p.canvas.width as f32 };
        let committed = self.project();
        // the selected clip's mask, and in crop mode its crop, as the picture shows them (mirrored
        // with it)
        let chosen = self.selected.as_deref().and_then(|id| committed.clip(id)).map(|(_, c)| c);
        let flips = chosen.map_or([false; 2], |c| [c.look.flip_x, c.look.flip_y]);
        let shown = |[l, t, r, b]: [f32; 4]| {
            let (l, r) = if flips[0] { (r, l) } else { (l, r) };
            let (t, b) = if flips[1] { (b, t) } else { (t, b) };
            [l, t, r, b]
        };
        let shaping = stage::Shaping {
            mask: chosen.and_then(|c| c.mask_at(at - c.start)),
            crop: chosen.filter(|_| self.cropping.is_some()).map(|c| shown(c.crop_at(at - c.start))),
        };
        if self.cropping.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
        {
            self.cropping = None;
        }
        let out = stage::interact(
            ui,
            &response,
            view,
            [p.canvas.width as f32, p.canvas.height as f32],
            &self.outlines,
            self.selected.as_deref(),
            shaping,
            &mut self.gesture,
            // where the clip is now: its keyframed placement at the playhead
            |id| committed.clip(id).map(|(_, c)| placement_at(c, at)),
        );
        if let Some(sel) = out.select {
            self.selected = sel;
            self.also.clear();
        }
        if let Some(clip) = out.edit
            && let Some((_, c)) = committed.clip(&clip)
            && let ClipContent::Text { text, .. } = &c.content
        {
            self.selected = Some(clip.clone());
            self.also.clear();
            self.editing = Some((clip, text.clone()));
        }
        self.edit_words(ui, view);
        if out.leave_crop {
            self.cropping = None;
        }
        if let Some((clip, m)) = out.mask {
            // only what the drag changed, at the playhead, so keyframes change only there
            let was = shaping.mask.unwrap_or(m);
            let mut params = json!({"clip": clip, "at": at});
            for (key, now, before) in [
                ("x", m.center[0], was.center[0]),
                ("y", m.center[1], was.center[1]),
                ("width", m.size[0], was.size[0]),
                ("height", m.size[1], was.size[1]),
                ("rotation", m.rotation, was.rotation),
            ] {
                if now != before {
                    params[key] = json!(now);
                }
            }
            let _ = self.execute("clip.setMask", params);
        }
        if let Some((clip, c)) = out.crop {
            let was = shaping.crop.unwrap_or(c);
            let mut params = json!({"clip": clip, "at": at});
            // back from how the picture shows to its own edges
            let names = [
                if flips[0] { "right" } else { "left" },
                if flips[1] { "bottom" } else { "top" },
                if flips[0] { "left" } else { "right" },
                if flips[1] { "top" } else { "bottom" },
            ];
            for (i, name) in names.into_iter().enumerate() {
                if c[i] != was[i] {
                    params[name] = json!((c[i] * 1000.0).round() / 1000.0);
                }
            }
            let _ = self.execute("clip.setCrop", params);
        }
        if let Some((clip, t)) = out.commit {
            // only what the gesture changed, at the playhead, so keyframes change only there
            let was = committed.clip(&clip).map(|(_, c)| placement_at(c, at)).unwrap_or_default();
            let mut params = json!({"clip": clip, "at": at});
            for (prop, now, before) in [
                (Prop::X, t.x, was.x),
                (Prop::Y, t.y, was.y),
                (Prop::Scale, t.scale, was.scale),
                (Prop::Rotation, t.rotation, was.rotation),
            ] {
                if now != before {
                    params[prop.name()] = json!(now);
                }
            }
            let _ = self.execute("clip.set", params);
        }
    }
}

impl EditorApp {
    /// The box for a title's words, over the title in the preview: Enter keeps them (Shift+Enter
    /// starts a new line), Esc or a click elsewhere without changes leaves them as they were.
    fn edit_words(&mut self, ui: &mut egui::Ui, view: stage::View) {
        let Some((clip, mut words)) = self.editing.take() else { return };
        let Some((_, o)) = self.outlines.iter().find(|(id, _)| *id == clip) else { return };
        let mid = view.frame.min + egui::vec2((o[0][0] + o[2][0]) * 0.5, (o[0][1] + o[2][1]) * 0.5) * view.k;
        let width = (view.frame.width() * 0.8).max(160.0);
        let (mut done, mut keep) = (false, false);
        egui::Area::new(egui::Id::unique("edit-words")).fixed_pos(mid - egui::vec2(width * 0.5, 18.0)).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = ui.add(
                        egui::TextEdit::multiline(&mut words)
                            .desired_width(width)
                            .desired_rows(1)
                            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
                    );
                    if !edit.has_focus() && !edit.lost_focus() {
                        edit.request_focus();
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if enter || edit.lost_focus() {
                        (done, keep) = (true, !esc);
                    }
                });
            },
        );
        if !done {
            self.editing = Some((clip, words));
            return;
        }
        let before = self.project().clip(&clip).and_then(|(_, c)| match &c.content {
            ClipContent::Text { text, .. } => Some(text.clone()),
            _ => None,
        });
        let words = words.trim_end_matches('\n').to_string();
        if keep && !words.trim().is_empty() && before.as_deref() != Some(words.as_str()) {
            let _ = self.execute("clip.set", json!({"clip": clip, "text": words}));
        }
    }
}

/// A clip's placement at timeline time `at`, keyframes evaluated (held at the clip's ends).
fn placement_at(c: &reelforge_model::Clip, at: Tick) -> ClipTransform {
    let t = (at - c.start).max(Tick::ZERO).min(c.duration);
    ClipTransform {
        x: c.value_at(Prop::X, t),
        y: c.value_at(Prop::Y, t),
        scale: c.value_at(Prop::Scale, t),
        rotation: c.value_at(Prop::Rotation, t),
    }
}

/// Whether a file name is a SubRip caption file.
/// A CapCut / 剪映 draft: `draft_content.json` or `draft_info.json`.
/// What Import offers: video, pictures, sound, LUTs, fonts, captions and CapCut drafts.
const IMPORTABLE: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "png", "jpg", "jpeg", "webp", "gif", "mp3", "wav", "m4a", "aac", "flac", "ogg",
    "opus", "cube", "ttf", "otf", "ttc", "srt", "json",
];

/// What fills a template's slot or takes a clip's place: video and pictures.
const PICTURES: &[&str] = &["mp4", "mov", "m4v", "mkv", "webm", "png", "jpg", "jpeg", "webp", "gif"];

/// What a title's font button offers.
const FONTS: &[&str] = &["ttf", "otf", "ttc"];

/// The inspector's list of a clip's picture effects: each with its strength and pace, one to
/// remove it, and a menu to add another. Returns the command that makes the list what it now is.
fn fx_section(
    ui: &mut egui::Ui,
    s: &i18n::Strings,
    clip: &str,
    list: &mut Vec<reelforge_model::Fx>,
) -> Option<(&'static str, Value)> {
    use reelforge_model::{Fx, FxGroup, FxKind};
    let mut changed = false;
    theme::section(ui, s.sec_fx, |ui| {
        let mut gone = None;
        let done = |r: &egui::Response| r.drag_stopped() || (r.changed() && !r.dragged());
        egui::Grid::new("fx").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
            for (i, fx) in list.iter_mut().enumerate() {
                ui.label(assets::fx_name(fx.kind, s));
                let r = ui.add(
                    egui::DragValue::new(&mut fx.strength)
                        .speed(0.01)
                        .range(0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                        .custom_parser(|t| {
                            t.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)
                        }),
                );
                changed |= done(&r);
                r.on_hover_text(s.fx_strength);
                if fx.kind.has_speed() {
                    let r = ui.add(
                        egui::DragValue::new(&mut fx.speed).speed(0.01).range(0.25..=4.0).suffix("×").max_decimals(2),
                    );
                    changed |= done(&r);
                    r.on_hover_text(s.fx_speed);
                } else {
                    ui.label("");
                }
                if ui.small_button(icon::DELETE).on_hover_text(s.fx_remove).clicked() {
                    gone = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = gone {
            list.remove(i);
            changed = true;
        }
        ui.menu_button(format!("{}  {}", icon::ADD, s.fx_add), |ui| {
            for (title, group) in s.fx_groups.into_iter().zip(FxGroup::ALL) {
                ui.label(egui::RichText::new(title).small().color(theme::color::WEAK));
                for kind in FxKind::ALL.into_iter().filter(|k| k.group() == group) {
                    if ui.button(assets::fx_name(kind, s)).clicked() {
                        list.push(Fx::new(kind));
                        changed = true;
                        ui.close();
                    }
                }
            }
        });
    });
    changed.then(|| ("clip.setFx", json!({"clip": clip, "fx": list})))
}

fn is_draft(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    file.starts_with("draft_") && file.ends_with(".json") && (file.contains("content") || file.contains("info"))
}

fn is_subrip(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("srt"))
}

/// Where the picked language is kept between runs.
const LANG_KEY: &str = "reelforge.lang";
/// Where the preview quality is kept between runs (`full` or `smooth`).
const QUALITY_KEY: &str = "reelforge.preview";
/// Where a project with no file and no server is kept ([`EditorApp::with_recovery`]).
const RECOVERY_KEY: &str = "reelforge.project";
/// How long after a change the project is kept, in seconds.
const RECOVERY_DELAY: f64 = 1.0;
/// A window narrower than this many points shows one panel at a time under the preview (a phone
/// held upright, a narrow browser window).
const NARROW_BELOW: f32 = 860.0;

/// What shows under the preview of a narrow window.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum Sheet {
    #[default]
    Timeline,
    /// The panel of media, sounds, titles, stickers and effects.
    Media,
    /// The inspector.
    Adjust,
}

/// How many files of a project taken back from storage must be handed over again: in the browser,
/// every imported one (the page kept only the project); on the desktop, none (they are read from
/// their paths).
fn recovered_missing(p: &reelforge_model::Project) -> usize {
    if cfg!(target_arch = "wasm32") {
        p.media.values().filter(|m| m.sha256.is_some() && !m.uri.starts_with(reelforge_media::library::SCHEME)).count()
    } else {
        0
    }
}

impl eframe::App for EditorApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.lang_chosen {
            storage.set_string(LANG_KEY, self.lang.code().to_string());
        }
        storage.set_string(QUALITY_KEY, if self.full_quality { "full" } else { "smooth" }.to_string());
        if self.recovery {
            self.keep_draft(&mut drafts::Kept(storage));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if (std::mem::take(&mut self.lang_unsaved) | std::mem::take(&mut self.quality_unsaved))
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
        }
        if self.recovery && self.session.revision() != self.kept.0 {
            // a moment after the last change, so a drag is kept once, at its end
            let now = ctx.input(|i| i.time);
            let due = *self.kept.1.get_or_insert(now + RECOVERY_DELAY);
            if now >= due {
                self.with_store(frame, |app, store| app.keep_draft(store));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
            }
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
            theme::install(&ctx, self.lang == Lang::Ja);
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
                    self.imported(&v, up.place);
                }
            }
            if let Some(note) = remote.take_notes().pop() {
                self.status = note.text(self.lang);
            }
        }
        self.narrow = ctx.content_rect().width() < NARROW_BELOW;
        if self.into.is_some() && self.into != self.selected {
            // the clip waiting for a file to replace it was let go of
            self.into = None;
        }
        if let Some(ask) = self.draft_ask.take() {
            self.with_store(frame, |app, store| app.drafts_do(ask, store));
        }
        if self.drafts.as_ref().is_some_and(|d| d.home) {
            self.drafts_page(ui, frame);
            return;
        }
        self.take_dropped_files(&ctx);
        self.export_window(&ctx);
        self.timelines_window(&ctx);
        self.spoken_window(&ctx);
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
        let (playing, lang, media) = (self.playing, self.lang, self.media.clone());
        let (selection, can_paste, range) = (self.selection(), self.clipboard.is_some(), self.range());
        let shown = !self.preview_only;
        // a narrow window has the preview above and one of the timeline, the media and the
        // inspector under it, picked by the tabs at the bottom
        let narrow = self.narrow;
        let on = |sheet: Sheet| shown && (!narrow || self.sheet == sheet);
        let (timeline_on, media_on, adjust_on) = (on(Sheet::Timeline), on(Sheet::Media), on(Sheet::Adjust));
        // a sheet leaves the preview some of the window
        let tallest = (ctx.content_rect().height() * 0.6).max(200.0);
        let side = |id: &str, left: bool, width: f32| {
            if narrow {
                egui::Panel::bottom(format!("{id} sheet")).default_size(300.0).size_range(140.0..=tallest)
            } else if left {
                egui::Panel::left(id.to_string()).default_size(width).size_range(220.0..=520.0)
            } else {
                egui::Panel::right(id.to_string()).default_size(width)
            }
            .resizable(true)
        };
        if narrow {
            egui::Panel::bottom("sheets").exact_size(40.0).frame(bar(theme::color::PANEL)).show_collapsible(
                ui,
                &mut shown.clone(),
                |ui| self.sheet_tabs(ui),
            );
        }
        let timeline = egui::Panel::bottom("timeline").resizable(true).default_size(280.0);
        let timeline = if narrow { timeline.size_range(140.0..=tallest) } else { timeline };
        let out = timeline
            .frame(egui::Frame::new().fill(theme::color::PANEL))
            .show_collapsible(ui, &mut timeline_on.clone(), |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| self.timeline_bar(ui));
                self.timeline.show(ui, &p, self.playhead, &selection, playing, can_paste, range, lang, &media)
            })
            .map(|r| r.inner)
            .unwrap_or_default();
        if self.assets.wants_looks() && self.looks.is_none() {
            self.looks = Some(looks::previews(&ctx));
        }
        let looks = self.looks.clone().unwrap_or_default();
        let asked = side("assets", true, 300.0)
            .frame(egui::Frame::new().fill(theme::color::PANEL).inner_margin(egui::Margin::same(10)))
            .show_collapsible(ui, &mut media_on.clone(), |ui| {
                self.assets.show(ui, &p, &media, lang, &looks, playing, self.playhead, &selection)
            })
            .and_then(|r| r.inner);
        side("inspector", false, 290.0)
            .frame(egui::Frame::new().fill(theme::color::PANEL).inner_margin(egui::Margin::same(12)))
            .show_collapsible(ui, &mut adjust_on.clone(), |ui| self.inspector(ui));
        let edge = if narrow { 8 } else { 16 };
        egui::Panel::bottom("transport")
            .exact_size(46.0)
            .frame(bar(theme::color::STAGE).inner_margin(egui::Margin::symmetric(edge, 6)))
            .show(ui, |ui| self.transport(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::color::STAGE).inner_margin(egui::Margin::same(edge)))
            .show(ui, |ui| self.preview(ui));

        if let Some(t) = out.seek {
            let before = self.playhead;
            self.seek(t);
            if self.scrub_sound && !self.playing && self.playhead != before {
                self.scrub();
            }
        }
        if let Some(change) = out.select {
            self.select(change);
        }
        if let Some((clips, by)) = out.shifted {
            let _ = self.execute("clip.shift", json!({"clips": clips, "by": by}));
        }
        match out.action {
            Some(timeline::Action::Replace) => self.pick_replacement(&ctx),
            Some(action) => self.act(action),
            None => {}
        }
        if let Some((track, what)) = out.track_action {
            self.track_action(&track, what);
        }
        match out.marker {
            Some(timeline::MarkerAction::Add(at)) => {
                let _ = self.execute("marker.add", json!({"at": at}));
            }
            Some(timeline::MarkerAction::Remove(id)) => {
                let _ = self.execute("marker.remove", json!({"marker": id}));
            }
            None => {}
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
        if let Some((clip, by)) = out.rolled {
            let _ = self.execute("clip.roll", json!({"clip": clip, "by": by}));
        }
        if let Some((clip, by)) = out.slipped {
            let _ = self.execute("clip.slip", json!({"clip": clip, "by": by}));
        }
        match asked {
            Some(assets::Action::Import) => self.pick_files(&ctx),
            Some(assets::Action::Add(item)) => self.add_item(&item, self.playhead, None),
            Some(assets::Action::Remove(id)) => {
                let name = p.media.get(&id).map(|m| m.name.clone()).unwrap_or_default();
                if self.execute("media.remove", json!({"media": id})).is_ok() {
                    self.status = fill(self.s().media_removed, &[&name]);
                }
            }
            Some(assets::Action::Rename(id, name)) => {
                let _ = self.execute("media.set", json!({"media": id, "name": name}));
            }
            Some(assets::Action::File(ids, folder)) => {
                let _ = self.execute("media.set", json!({"media": ids, "folder": folder}));
            }
            Some(assets::Action::Captions(ask)) => self.captions_ask(ask, &ctx),
            Some(assets::Action::Template(id)) => self.use_template(id),
            Some(assets::Action::Fill) => self.pick_fill(&ctx),
            None => {}
        }
        if let Some((item, at, track)) = out.dropped {
            self.add_item(&item, at, track);
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
