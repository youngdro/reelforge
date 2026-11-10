//! The canvas settings (剪映's 比例): the shape and size of the picture, its frame rate and the
//! colour behind everything. Opened from the size in the top bar; every change is one
//! `project.setCanvas`.

use eframe::egui;
use reelforge_engine::RATIOS;
use reelforge_model::export::FRAME_RATES;
use reelforge_model::{ClipContent, Project, TrackKind};
use serde_json::{Value, json};

use crate::{EditorApp, theme};

/// The sizes offered, by the shorter side.
const SHORT_SIDES: [u32; 4] = [720, 1080, 1440, 2160];

/// The window's state.
pub(crate) struct CanvasEdit {
    /// Showing.
    pub(crate) open: bool,
    /// The size being typed, until it is applied.
    size: Option<[u32; 2]>,
    /// The clips follow a new size.
    refit: bool,
    /// A background colour being picked, applied once the picker closes.
    colour: Option<[f32; 3]>,
}

impl Default for CanvasEdit {
    fn default() -> CanvasEdit {
        CanvasEdit { open: false, size: None, refit: true, colour: None }
    }
}

/// `w:h` at `short` pixels on its shorter side, both sides even and at most 8192.
pub(crate) fn shaped(w: f64, h: f64, short: u32) -> [u32; 2] {
    let even = |v: f64| ((v.round() as u32).clamp(16, 8192)) & !1;
    if w >= h {
        [even(short as f64 * w / h), even(short as f64)]
    } else {
        [even(short as f64), even(short as f64 * h / w)]
    }
}

/// Whether `a` and `b` are the same shape, within half a percent.
fn same_shape(a: [u32; 2], b: [f64; 2]) -> bool {
    let (x, y) = (a[0] as f64 / a[1].max(1) as f64, b[0] / b[1].max(1.0));
    (x / y - 1.0).abs() < 0.005
}

/// The picture size of the first video or image on the main track (the lowest video track that
/// has one): what "original" means.
pub(crate) fn original(p: &Project) -> Option<[u32; 2]> {
    p.tracks.iter().filter(|t| t.kind == TrackKind::Video).find_map(|t| {
        t.clips
            .iter()
            .filter_map(|c| match &c.content {
                ClipContent::Media { media, .. } => Some((c.start, p.media.get(media)?.size?)),
                _ => None,
            })
            .min_by_key(|(start, _)| *start)
            .map(|(_, size)| size)
    })
}

/// The ratio's two numbers, from its id (`2.35:1`).
fn ratio(id: &str) -> [f64; 2] {
    let mut it = id.split(':').map(|v| v.parse::<f64>().unwrap_or(1.0));
    [it.next().unwrap_or(1.0), it.next().unwrap_or(1.0)]
}

impl EditorApp {
    /// The canvas settings, while open.
    pub(crate) fn canvas_window(&mut self, ctx: &egui::Context) {
        if !self.canvas.open {
            return;
        }
        let p = self.project();
        let s = self.s();
        let c = p.canvas.clone();
        let now = [c.width, c.height];
        let short = c.width.min(c.height);
        let mut set: Option<Value> = None;
        let mut open = true;
        let edit = &mut self.canvas;
        egui::Window::new(s.canvas_title)
            .open(&mut open)
            .anchor(egui::Align2::LEFT_TOP, [12.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                let weak = |t: &str| egui::RichText::new(t.to_string()).color(theme::color::WEAK);
                ui.label(weak(s.canvas_ratio));
                ui.horizontal_wrapped(|ui| {
                    if let Some(o) = original(&p) {
                        let size = shaped(o[0] as f64, o[1] as f64, short);
                        let r = ui.selectable_label(false, s.canvas_original).on_hover_text(s.canvas_original_tip);
                        if r.clicked() && size != now {
                            set = Some(json!({"width": size[0], "height": size[1]}));
                        }
                    }
                    for (id, ..) in RATIOS {
                        let [a, b] = ratio(id);
                        if ui.selectable_label(same_shape(now, [a, b]), id).clicked() {
                            let size = shaped(a, b, short);
                            if size != now {
                                set = Some(json!({"width": size[0], "height": size[1]}));
                            }
                        }
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("canvas settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(weak(s.canvas_resolution));
                    let name = |side: u32| match side {
                        1440 => "2K".to_string(),
                        2160 => "4K".to_string(),
                        _ => format!("{side}p"),
                    };
                    egui::ComboBox::from_id_salt("canvas resolution")
                        .selected_text(if SHORT_SIDES.contains(&short) { name(short) } else { format!("{short}p") })
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for side in SHORT_SIDES {
                                let size = shaped(c.width as f64, c.height as f64, side);
                                let label = format!("{}  ·  {}×{}", name(side), size[0], size[1]);
                                if ui.selectable_label(side == short, label).clicked() && size != now {
                                    set = Some(json!({"width": size[0], "height": size[1]}));
                                }
                            }
                        });
                    ui.end_row();

                    ui.label(weak(s.canvas_size));
                    ui.horizontal(|ui| {
                        let typed = edit.size.get_or_insert(now);
                        ui.add(egui::DragValue::new(&mut typed[0]).range(16..=8192).speed(2.0));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut typed[1]).range(16..=8192).speed(2.0));
                        let size = typed.map(|v| v & !1);
                        if ui.add_enabled(size != now, egui::Button::new(s.canvas_apply).small()).clicked() {
                            set = Some(json!({"width": size[0], "height": size[1]}));
                        }
                    });
                    ui.end_row();

                    ui.label(weak(s.canvas_fps));
                    let fps = c.fps.fps();
                    let fps_name = |f: f64| format!("{} fps", (f * 100.0).round() / 100.0);
                    egui::ComboBox::from_id_salt("canvas fps").selected_text(fps_name(fps)).width(170.0).show_ui(
                        ui,
                        |ui| {
                            for f in FRAME_RATES {
                                let on = (fps - f as f64).abs() < 1e-6;
                                if ui.selectable_label(on, fps_name(f as f64)).clicked() && !on {
                                    set = Some(json!({"fps": [f, 1]}));
                                }
                            }
                        },
                    );
                    ui.end_row();

                    ui.label(weak(s.canvas_background));
                    ui.horizontal(|ui| {
                        for colour in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.5, 0.5, 0.5], [0.07, 0.09, 0.16]] {
                            let shown = egui::Color32::from_rgb(
                                (colour[0] * 255.0) as u8,
                                (colour[1] * 255.0) as u8,
                                (colour[2] * 255.0) as u8,
                            );
                            let on = c.background[..3].iter().zip(colour).all(|(a, b)| (a - b).abs() < 1e-3);
                            let (rect, r) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                            ui.painter().rect_filled(rect, 4.0, shown);
                            let edge = if on { theme::color::ACCENT } else { theme::color::WEAK };
                            ui.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(if on { 2.0 } else { 1.0 }, edge),
                                egui::StrokeKind::Outside,
                            );
                            if r.clicked() && !on {
                                set = Some(json!({"background": [colour[0], colour[1], colour[2], 1.0]}));
                            }
                        }
                        // any other colour, sent once the picker is let go
                        let shown = edit.colour.unwrap_or([c.background[0], c.background[1], c.background[2]]);
                        let mut rgb = shown.map(|v| (v * 255.0).round() as u8);
                        let r = ui.color_edit_button_srgb(&mut rgb);
                        let picked = rgb.map(|v| f32::from(v) / 255.0);
                        if r.changed() {
                            edit.colour = Some(picked);
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.checkbox(&mut edit.refit, s.canvas_refit).on_hover_text(s.canvas_refit_tip);
            });
        // a colour being picked is applied once the picker closes
        if let Some(rgb) = self.canvas.colour.filter(|_| !ctx.any_popup_open()) {
            self.canvas.colour = None;
            set = Some(json!({"background": [rgb[0], rgb[1], rgb[2], 1.0]}));
        }
        if let Some(mut v) = set {
            v["refit"] = json!(self.canvas.refit);
            let resized = v.get("width").is_some();
            if self.execute("project.setCanvas", v).is_ok() && resized {
                self.canvas.size = None;
            }
        }
        // the typed size follows changes made elsewhere while it is not being typed
        if self.canvas.size == Some(now) {
            self.canvas.size = None;
        }
        self.canvas.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_keep_the_shorter_side() {
        assert_eq!(shaped(9.0, 16.0, 1080), [1080, 1920]);
        assert_eq!(shaped(16.0, 9.0, 720), [1280, 720]);
        assert_eq!(shaped(2.35, 1.0, 1080), [2538, 1080]);
        assert_eq!(shaped(1.0, 1.0, 2160), [2160, 2160]);
        assert_eq!(shaped(4.0, 3.0, 2160), [2880, 2160]);
        assert!(same_shape([1080, 1920], [9.0, 16.0]));
        assert!(!same_shape([1080, 1920], [3.0, 4.0]));
        for (id, w, h) in RATIOS {
            assert_eq!(shaped(ratio(id)[0], ratio(id)[1], 1080), [w, h], "{id}");
        }
    }
}
