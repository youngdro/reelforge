//! The editor in the browser.
//!
//! `index.html` loads the module and calls [`start`] with a canvas id. The page also gets a small
//! API on `window.reelforge` for agents and tests: `execute(id, paramsJson)` runs any engine
//! command and returns its JSON result, exactly as the CLI and MCP do.
//!
//! Given a server's address, the editor joins the session that server holds (see
//! `reelforge_ui::remote`): a person in the browser, agents over MCP and other editors edit one
//! project. A server started with `--web` serves this page, which then joins it by itself.

#[cfg(target_arch = "wasm32")]
mod web {
    use eframe::wasm_bindgen::{self, JsCast, prelude::*};
    use reelforge_ui::EditorApp;

    /// The running app, so `execute` can reach its session.
    #[wasm_bindgen]
    pub struct Handle {
        runner: eframe::WebRunner,
    }

    #[wasm_bindgen]
    impl Handle {
        /// Runs an engine command; `params` is JSON. Returns the result as JSON, or throws.
        pub fn execute(&self, id: &str, params: &str) -> Result<String, JsValue> {
            let params: serde_json::Value =
                serde_json::from_str(params).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let mut app =
                self.runner.app_mut::<EditorApp>().ok_or_else(|| JsValue::from_str("the editor is not running"))?;
            let out = app.execute(id, params).map_err(|e| JsValue::from_str(&e))?;
            drop(app);
            Ok(out.to_string())
        }

        /// Whether every edit reached the server and the editor holds what the server holds (always,
        /// without a server).
        pub fn settled(&self) -> bool {
            self.runner.app_mut::<EditorApp>().is_some_and(|app| app.remote().is_none_or(|r| r.settled()))
        }
    }

    /// Starts the editor on the canvas with this id: on the session of the server at `server`
    /// (`http://127.0.0.1:7878`, with its `token` if it has one), or on a demo project.
    #[wasm_bindgen]
    pub async fn start(canvas_id: &str, server: Option<String>, token: Option<String>) -> Result<Handle, JsValue> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let canvas =
            document.get_element_by_id(canvas_id).ok_or("no such canvas")?.dyn_into::<web_sys::HtmlCanvasElement>()?;
        let runner = eframe::WebRunner::new();
        runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    // without a server, the project is kept in the page's storage and survives a reload
                    Ok(Box::new(match &server {
                        Some(base) => EditorApp::join(base, token, "web").with_gpu(cc),
                        None => EditorApp::demo().with_gpu(cc).with_recovery(cc),
                    }))
                }),
            )
            .await?;
        Ok(Handle { runner })
    }
}
