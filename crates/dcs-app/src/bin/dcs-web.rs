//! Wasm entry point: launches the renderer directly, skipping the native
//! CLI (`clap`) and `Serve` paths in `main.rs`, which don't apply in a browser.
//!
//! No `wasm-bindgen`-based panic hook here: this binary loads via miniquad's
//! own JS glue (`mq_js_bundle.js`), not `wasm-bindgen`-generated glue, so
//! anything depending on `wasm-bindgen` (e.g. `console_error_panic_hook`)
//! can't resolve its JS imports at runtime. Panics surface as a WebAssembly
//! trap in the browser console instead.

fn main() {
    dcs_render::run(dcs_render::RenderConfig::default());
}
