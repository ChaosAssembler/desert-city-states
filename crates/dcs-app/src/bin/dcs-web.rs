//! Wasm entry point: launches the renderer directly, skipping the native
//! CLI (`clap`) and `Serve` paths in `main.rs`, which don't apply in a browser.
//!
//! No `wasm-bindgen`-based panic hook here: this binary loads via miniquad's
//! own JS glue (`mq_js_bundle.js`), not `wasm-bindgen`-generated glue, so
//! anything depending on `wasm-bindgen` (e.g. `console_error_panic_hook`)
//! can't resolve its JS imports at runtime. Panics surface as a WebAssembly
//! trap in the browser console instead.

fn main() {
    dcs_app::orchestrate::run_gui();
}

/// Manual pointer+length buffer passing for `window.dcsAgent` (see
/// `web/index.html`) to call into `dcs_app::agent_bridge::dispatch_json`.
/// No `wasm-bindgen` here (see module doc above) — plain wasm32 linear
/// memory, matching how miniquad's own JS glue already passes mouse and
/// keyboard data across this exact boundary.
///
/// These exports must live in this binary crate, not `dcs-app`'s lib:
/// `#[no_mangle]` forces codegen but not link-time retention from a
/// statically-linked rlib archive — only symbols reachable from `main`
/// (i.e. defined right here) are guaranteed to survive into the wasm
/// export table.
#[cfg(feature = "dev-tools")]
mod agent_ffi {
    /// Allocates a buffer of `len` bytes for the caller to write request
    /// bytes into before calling `agent_dispatch`.
    #[unsafe(no_mangle)]
    pub extern "C" fn agent_alloc(len: usize) -> *mut u8 {
        let mut buf = Vec::<u8>::with_capacity(len);
        let ptr = buf.as_mut_ptr();
        std::mem::forget(buf);
        ptr
    }

    /// Frees a buffer previously returned by `agent_dispatch` (the response
    /// buffer) — call after reading it. The *input* buffer `agent_alloc`
    /// returns is consumed and freed automatically inside `agent_dispatch`.
    #[unsafe(no_mangle)]
    pub extern "C" fn agent_free(ptr: *mut u8, len: usize) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ptr`/`len` must come from a prior `agent_alloc`/
        // `agent_dispatch` call with matching length — the JS side (see
        // web/index.html) only ever calls this with values it received
        // directly from those, never constructed itself.
        unsafe {
            drop(Vec::from_raw_parts(ptr, len, len));
        }
    }

    /// Parses `len` bytes at `ptr` as a UTF-8 JSON `Request`, dispatches it
    /// against the live `GameState`, and returns a packed
    /// `(response_ptr << 32) | response_len` — wasm32 pointers fit in 32
    /// bits, and an `i64`/`u64` return value round-trips through JS as a
    /// `BigInt`, standard WebAssembly-JS interop behavior needing no
    /// multi-value-returns support.
    #[unsafe(no_mangle)]
    pub extern "C" fn agent_dispatch(ptr: *const u8, len: usize) -> u64 {
        // SAFETY: `ptr`/`len` must come from a prior `agent_alloc` call
        // with the same `len`, fully written by the caller before this
        // call — the JS side (web/index.html) upholds this.
        let input_bytes = unsafe { Vec::from_raw_parts(ptr as *mut u8, len, len) };
        let input = String::from_utf8_lossy(&input_bytes).into_owned();

        let response = dcs_app::agent_bridge::dispatch_json(&input);

        let mut out = response.into_bytes();
        out.shrink_to_fit();
        let out_ptr = out.as_mut_ptr();
        let out_len = out.len();
        std::mem::forget(out);

        ((out_ptr as u64) << 32) | out_len as u64
    }
}
