//! The desktop editor. `reelforge [project.reel.json]` opens a project, or a demo without one.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use reelforge_engine::Session;
    use reelforge_model::Project;
    use reelforge_ui::EditorApp;

    let app = match std::env::args().nth(1) {
        Some(path) => {
            let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
            let project =
                Project::from_json(&json).unwrap_or_else(|e| panic!("{path} is not a reelforge project: {e}"));
            EditorApp::new(Session::new(project))
        }
        None => EditorApp::demo(),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("reelforge"),
        ..Default::default()
    };
    eframe::run_native("reelforge", options, Box::new(|_cc| Ok(Box::new(app))))
}

#[cfg(target_arch = "wasm32")]
fn main() {}
