//! Pictures of the title templates (花字), for the inspector's picker: each drawn once by the
//! renderer, the way it will look on the canvas.

use eframe::egui;
use reelforge_compile::{CompileOptions, compile};
use reelforge_engine::{Session, title_templates};
use reelforge_model::TrackKind;
use reelforge_render::{NoMedia, Quality, render_frame};
use reelforge_time::Tick;
use serde_json::json;

/// The sample's size in pixels.
pub(crate) const SIZE: [u32; 2] = [240, 104];

/// Each template's id and its picture, in the picker's order.
pub(crate) fn previews(ctx: &egui::Context) -> Vec<(&'static str, egui::TextureHandle)> {
    title_templates()
        .into_iter()
        .filter_map(|t| {
            let mut s = Session::default();
            let bg = crate::theme::color::RAISED.to_normalized_gamma_f32();
            s.execute(
                "project.setCanvas",
                &json!({"width": SIZE[0], "height": SIZE[1], "background": [bg[0], bg[1], bg[2], 1.0]}),
            )
            .ok()?;
            let track = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text)?.id.clone();
            s.execute("clip.add", &json!({"track": track, "text": "花字 Aa", "size": 50, "template": t.id})).ok()?;
            let spec = compile(&s.project(), &CompileOptions { encode: false }).ok()?;
            let (img, _) = render_frame(&spec, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).ok()?;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_srgba8());
            Some((t.id, ctx.load_texture(format!("template-{}", t.id), image, egui::TextureOptions::LINEAR)))
        })
        .collect()
}
