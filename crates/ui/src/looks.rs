//! Pictures of the title templates (花字), for the inspector's picker: each drawn once by the
//! renderer, the way it will look on the canvas, and sorted onto their shelves.

use eframe::egui;
use reelforge_compile::{CompileOptions, compile};
use reelforge_engine::{Session, TitleGroup, title_templates};
use reelforge_model::TrackKind;
use reelforge_render::{NoMedia, Quality, render_frame};
use reelforge_time::Tick;
use serde_json::json;

/// The sample's size in pixels.
pub(crate) const SIZE: [u32; 2] = [240, 104];

/// A template's id and its picture.
pub(crate) type Look = (&'static str, egui::TextureHandle);

/// How many pictures are drawn in one frame: there are dozens, and drawing them all at once
/// would hold the window still.
const AT_ONCE: usize = 6;

/// Draws the next few templates' pictures onto the end of `looks` (which holds them in the
/// picker's order) until every template has one, asking for another frame while some are left.
pub(crate) fn more(ctx: &egui::Context, looks: &mut Vec<Look>) {
    let all = title_templates();
    if looks.len() >= all.len() {
        return;
    }
    for t in all.iter().skip(looks.len()).take(AT_ONCE) {
        let image = picture(t.id).unwrap_or_else(|| egui::ColorImage::filled([1, 1], crate::theme::color::RAISED));
        looks.push((t.id, ctx.load_texture(format!("template-{}", t.id), image, egui::TextureOptions::LINEAR)));
    }
    ctx.request_repaint();
}

/// The pictures drawn so far shelf by shelf: each group's place in [`TitleGroup::ALL`] and its
/// templates' pictures (a group's templates stand together in the list).
pub(crate) fn shelves(looks: &[Look]) -> Vec<(usize, &[Look])> {
    let all = title_templates();
    let mut out = vec![];
    let mut from = 0;
    for (g, group) in TitleGroup::ALL.into_iter().enumerate() {
        let n = all.iter().filter(|t| t.group == group).count();
        let to = (from + n).min(looks.len());
        if from < to {
            out.push((g, &looks[from..to]));
        }
        from += n;
    }
    out
}

/// A template's sample, as the renderer draws it.
fn picture(id: &str) -> Option<egui::ColorImage> {
    let mut s = Session::default();
    let bg = crate::theme::color::RAISED.to_normalized_gamma_f32();
    s.execute(
        "project.setCanvas",
        &json!({"width": SIZE[0], "height": SIZE[1], "background": [bg[0], bg[1], bg[2], 1.0]}),
    )
    .ok()?;
    let track = s.project().tracks.iter().find(|t| t.kind == TrackKind::Text)?.id.clone();
    s.execute("clip.add", &json!({"track": track, "text": "花字 Aa", "size": 50, "template": id})).ok()?;
    let spec = compile(&s.project(), &CompileOptions { encode: false }).ok()?;
    let (img, _) = render_frame(&spec, Tick::ZERO, 1.0, &NoMedia, Quality::Draft).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_srgba8()))
}
