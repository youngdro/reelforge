//! The desktop editor. `reelforge [project.reel.json]` opens a project, or a demo without one.
//! Drop video, image or audio files on the window to import them; Export writes beside the project.

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
            let dir = std::path::Path::new(&path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
            EditorApp::new(Session::new(project)).with_export_dir(if dir.as_os_str().is_empty() {
                ".".into()
            } else {
                dir
            })
        }
        None => EditorApp::demo(),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("reelforge"),
        ..Default::default()
    };
    eframe::run_native("reelforge", options, Box::new(|cc| Ok(Box::new(app.with_gpu(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {}
