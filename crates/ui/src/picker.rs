//! The browser's file picker, for Import: a hidden `<input type="file" multiple>` clicked from
//! the button's frame (still within the click's user activation). Each picked file's bytes arrive
//! later, as dropped files do, and go into the project's media only.

use std::sync::{Arc, Mutex};

use eframe::egui;
use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;

use crate::Arrival;

#[wasm_bindgen(inline_js = r#"
export function reelforge_pick_files(accept, each) {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = accept;
    input.style.display = "none";
    input.addEventListener("change", () => {
        for (const file of Array.from(input.files || [])) {
            file.arrayBuffer().then(
                (b) => each(file.name, new Uint8Array(b), ""),
                (e) => each(file.name, new Uint8Array(0), String(e)),
            );
        }
        input.remove();
    });
    document.body.appendChild(input);
    input.click();
}
export function reelforge_save_file(name, bytes, type) {
    const url = URL.createObjectURL(new Blob([bytes], { type }));
    const a = document.createElement("a");
    a.href = url; a.download = name; document.body.appendChild(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60000);
}
"#)]
extern "C" {
    fn reelforge_save_file(name: &str, bytes: &[u8], mime: &str);
    #[wasm_bindgen(catch)]
    fn reelforge_pick_files(accept: &str, each: &Closure<dyn FnMut(String, Uint8Array, String)>)
    -> Result<(), JsValue>;
}

/// Opens the picker for files with these extensions; what is picked lands in `arrivals`.
pub(crate) fn pick_files(
    ctx: &egui::Context,
    arrivals: Arc<Mutex<Vec<Arrival>>>,
    extensions: &[&str],
) -> Result<(), String> {
    let accept = extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(",");
    let ctx = ctx.clone();
    let each =
        Closure::<dyn FnMut(String, Uint8Array, String)>::new(move |name: String, bytes: Uint8Array, err: String| {
            let got = if err.is_empty() { Ok(bytes.to_vec()) } else { Err(err) };
            arrivals.lock().expect("arrivals lock").push((name, got, false));
            ctx.request_repaint();
        });
    let picked = reelforge_pick_files(&accept, &each).map_err(|e| format!("{e:?}"));
    // called for every file picked, whenever that is
    each.forget();
    picked
}

/// Hands `bytes` to the person as a download named `name`.
pub(crate) fn save(name: &str, bytes: &[u8], mime: &str) {
    reelforge_save_file(name, bytes, mime);
}
