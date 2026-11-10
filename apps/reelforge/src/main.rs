//! The desktop editor.
//!
//! ```sh
//! reelforge                                  # the last unsaved project, or a demo
//! reelforge cut.reel.json                    # open (or create) a project; every edit is saved
//! reelforge cut.reel.json --serve            # and let agents edit it too: MCP at http://127.0.0.1:7878/mcp
//! reelforge cut.reel.json --serve 0.0.0.0:7878 --token SECRET --web dist   # and people in a browser
//! reelforge --join http://studio:7878 --token SECRET   # edit a project another machine serves
//! ```
//!
//! Drop video, image or audio files on the window to import them; Export writes beside the project.
//! With `--serve`, agents and the editor share one session: an agent's edit shows in the window as
//! it commits, and the status bar says who made it.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use std::sync::Arc;

    use reelforge_engine::{Session, SharedSession};
    use reelforge_server::Hub;
    use reelforge_server::http::HttpServer;
    use reelforge_ui::EditorApp;

    // exports use the system's (hardware) encoder where there is one
    reelforge_hwenc::register();
    let fail = |msg: String| -> ! {
        eprintln!("reelforge: {msg}");
        std::process::exit(2)
    };
    let mut args = std::env::args().skip(1).peekable();
    let (mut path, mut serve, mut token) = (None, None, std::env::var("REELFORGE_TOKEN").ok());
    let (mut join, mut web) = (None, std::env::var_os("REELFORGE_WEB").map(std::path::PathBuf::from));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--serve" => {
                let addr = args.next_if(|n| !n.starts_with("--")).unwrap_or_else(|| "127.0.0.1:7878".into());
                serve = Some(addr);
            }
            "--token" => token = Some(args.next().unwrap_or_else(|| fail("--token needs a value".into()))),
            "--join" => join = Some(args.next().unwrap_or_else(|| fail("--join needs a server address".into()))),
            "--web" => web = Some(args.next().unwrap_or_else(|| fail("--web needs a folder".into())).into()),
            "-h" | "--help" => {
                println!("reelforge [project.reel.json] [--serve [addr]] [--web DIR] [--token TOKEN]");
                println!("reelforge --join http://host:7878 [--token TOKEN]");
                return Ok(());
            }
            flag if flag.starts_with("--") => fail(format!("unknown option {flag}")),
            p => path = Some(std::path::PathBuf::from(p)),
        }
    }

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("reelforge"),
        ..Default::default()
    };
    if let Some(base) = join {
        if path.is_some() || serve.is_some() {
            fail("--join edits the server's project: it takes no project file and no --serve".into());
        }
        let base = if base.contains("://") { base } else { format!("http://{base}") };
        let app = EditorApp::join(&base, token, "desktop");
        return eframe::run_native("reelforge", options, Box::new(|cc| Ok(Box::new(app.with_gpu(cc)))));
    }

    let mut app = match &path {
        Some(path) => {
            let project = reelforge_server::load(path).unwrap_or_else(|e| fail(e));
            let dir = path.parent().map(|d| d.to_path_buf()).filter(|d| !d.as_os_str().is_empty());
            EditorApp::shared(SharedSession::new(Session::new(project))).with_export_dir(dir.unwrap_or(".".into()))
        }
        None => EditorApp::demo(),
    };
    // the hub saves every change to the file, whoever makes it, and serves agents when asked
    let hub = Hub::shared(app.session().clone(), path.clone()).unwrap_or_else(|e| fail(e));
    if let Some(addr) = serve {
        let mut server = HttpServer::bind(Arc::new(hub), &addr, token).unwrap_or_else(|e| fail(e));
        let url = format!("http://{}/mcp", server.local_addr());
        eprintln!("reelforge: agents can edit this project over MCP at {url}");
        if let Some(dir) = web {
            eprintln!("reelforge: and people in a browser at http://{}/", server.local_addr());
            server = server.with_web(dir);
        }
        app = app.with_status(format!("Agents can edit over MCP at {url}"));
        std::thread::spawn(move || server.run());
    }

    // without a file, the project is kept in the app's storage and comes back next time
    let unsaved = path.is_none();
    eframe::run_native(
        "reelforge",
        options,
        Box::new(move |cc| {
            let app = app.with_gpu(cc);
            Ok(Box::new(if unsaved { app.with_recovery(cc) } else { app }))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
