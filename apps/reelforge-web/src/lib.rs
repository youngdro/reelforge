//! The editor in the browser.
//!
//! `index.html` loads the module and calls [`start`] with a canvas id. The page also gets a small
//! API on `window.reelforge` for agents and tests: `execute(id, paramsJson)` runs any engine
//! command and returns its JSON result, exactly as the CLI and MCP do.

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
    }

    /// Starts the editor on the canvas with this id.
    #[wasm_bindgen]
    pub async fn start(canvas_id: &str) -> Result<Handle, JsValue> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let canvas =
            document.get_element_by_id(canvas_id).ok_or("no such canvas")?.dyn_into::<web_sys::HtmlCanvasElement>()?;
        let runner = eframe::WebRunner::new();
        runner
            .start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(EditorApp::demo().with_gpu(cc)))))
            .await?;
        Ok(Handle { runner })
    }
}
