---
name: web-deployment
description: Use when creating or maintaining web-facing source files for the Trunk-based macroquad wasm build — HTML, Trunk config, and static assets
---

# Web Deployment for Macroquad Games

## Why this doesn't look like a typical Trunk/wasm-bindgen setup

**macroquad does not use `wasm-bindgen`.** It ships its own JS runtime
(miniquad's `mq_js_bundle.js`) that loads a plain `wasm32-unknown-unknown`
binary directly via `load("file.wasm")` — no `cdylib`, no `#[wasm_bindgen]`
exports, no `#[macroquad::main]` magic needed for wasm specifically. Trunk's
`rel="rust"` asset pipeline **always** runs `wasm-bindgen` on the compiled
output with no supported way to disable it — using it on a macroquad binary
either breaks or requires an unofficial, fragile `sed`-patching community
shim. Don't use `rel="rust"`.

Instead, Trunk is used only as a **dev server / file watcher / static asset
pipeline**. The actual wasm compile happens in a Trunk `pre_build` hook
running a plain `cargo build --target wasm32-unknown-unknown`, and the
resulting `.wasm` is brought into `dist/` via `rel="copy-file"`.

## Preconditions

- A dedicated, minimal wasm entry point binary (e.g. `src/bin/dcs-web.rs`)
  that just calls the renderer's `run()` — no CLI parsing (`clap`), no
  networking/serving code. Cargo auto-discovers `src/bin/*.rs` as binary
  targets; no `[[bin]]` stanza needed.
- `mq_js_bundle.js` vendored locally (e.g. `web/vendor/mq_js_bundle.js`) —
  don't rely on the external CDN at build/runtime; browser tests need this
  to work offline/reliably.
- `.cargo/config.toml` sets `--allow-undefined` for the `wasm32-unknown-unknown`
  target (see Common Issues below) — without it, the build fails to link.

## Rules

- Never use `std::fs` or `std::process::exit` in wasm-targeted code — they
  panic at runtime in the browser.

## Workflow

1. **`Trunk.toml`** goes in `web/` — a **plain directory with no
   `Cargo.toml` of its own** (not the workspace root, not a crate
   directory). This dodges two Trunk 0.21 quirks discovered the hard way:
   - Running from the workspace root fails with "could not find the root
     package of the target crate" — Trunk always resolves a root package via
     `cargo_metadata` against the cwd, which fails against a virtual
     workspace manifest (no `[package]`), even when the Rust pipeline isn't
     used at all (trunk-rs/trunk#909, unfixed as of 0.21.14/0.22.0-beta.2).
   - Running from a directory that **does** have a real `Cargo.toml` (e.g.
     a crate directory) makes Trunk implicitly add its own `wasm-bindgen`
     Rust build target even with no `rel="rust"` link in `index.html` —
     exactly what this setup avoids. `web/` has none, so `trunk build`
     correctly logs "no rust project found" and does only the copy-file
     work we asked for.

   Contents:
   - `[build] target = "index.html"`, `dist = "../dist"` (paths are
     relative to `Trunk.toml`'s own directory, so this lands the build
     output at the repo-root `dist/`, matching `.gitignore`) — no `[build]`
     target pointing at a `Cargo.toml`; we don't use Trunk's Rust pipeline.
   - `[[hooks]]` with `stage = "pre_build"` running
     `cargo build --release --target wasm32-unknown-unknown --bin <name> -p <crate>`.
   - `[watch] ignore = ["../target", "../dist", "../docs", "../.git"]` —
     **required**, otherwise Trunk's watcher sees the hook's own `cargo build`
     output land in `target/` and rebuild-loops forever.
2. **`index.html`**, alongside `Trunk.toml` in `web/`:
   - `<canvas id="glcanvas" tabindex="1">` — the id must be exactly
     `glcanvas`, matching what `mq_js_bundle.js` queries for.
   - `<link data-trunk rel="copy-file" href="../target/wasm32-unknown-unknown/release/<bin>.wasm">`
     to bring the hook's output into `dist/`.
   - `<link data-trunk rel="copy-file" href="vendor/mq_js_bundle.js">`.
   - `<script src="mq_js_bundle.js"></script>` then
     `<script>load("<bin>.wasm");</script>`.
   - CSS: `body { margin:0; overflow:hidden; }`,
     `canvas { display:block; width:100vw; height:100vh; }`.
3. Build output: `cd web && trunk build` (or `--release`) produces
   repo-root `dist/` with the `.wasm`, `mq_js_bundle.js`, and `index.html`.
   `trunk serve` runs a dev server with rebuild-on-change. Both must be run
   with `web/` as the working directory, per the quirks above.
4. **Verify no `wasm-bindgen` residue actually made it into the linked
   binary** before trusting a build (`--gc-sections` should strip anything
   unreachable, e.g. `getrandom`'s wasm-bindgen path when the game only ever
   uses seeded/deterministic RNG constructors, but don't assume — check):
   `strings dist/<bin>.wasm | grep -i wbindgen` should print nothing.

## Conventions

- Static assets (sprites, fonts, sounds) go in `assets/` and are loaded via
  macroquad's `load_*()` functions at runtime — these are not Trunk
  `data-trunk` assets, since macroquad fetches them itself over HTTP.
- The `dist/` directory (repo root) is gitignored.

## Common Issues on Wasm

- **Link fails with `undefined symbol: glBindTexture` / `init_webgl` / etc.**:
  miniquad declares JS-host functions (WebGL calls, canvas/window glue) as
  `extern "C"` with no glue generated at compile time — they resolve as wasm
  imports at instantiation, supplied by `mq_js_bundle.js`, not at link time.
  Add to `.cargo/config.toml`:
  ```toml
  [target.wasm32-unknown-unknown]
  rustflags = ["-C", "link-args=--allow-undefined"]
  ```
- **Don't use `console_error_panic_hook` (or anything else depending on
  `wasm-bindgen`)**: it needs `wasm-bindgen`-generated JS glue to resolve its
  own imports (e.g. `console.error`), which this build deliberately never
  generates (see "Why this doesn't look like a typical Trunk/wasm-bindgen
  setup" above) — adding it produces a binary that fails to link with
  `--export __wbindgen_describe___wbg_*` symbols, or fails to instantiate if
  it does link. Panics surface as a WebAssembly trap in the browser console
  instead; less friendly, but functional.
- **`getrandom`/nanorand errors**: any crate depending on `nanorand` or
  `getrandom` needs the `js` backend enabled explicitly for
  `wasm32-unknown-unknown` — neither can autodetect the browser. Add, in the
  dependent crate's `Cargo.toml`:
  ```toml
  [target.'cfg(target_arch = "wasm32")'.dependencies]
  getrandom = { version = "0.2", features = ["js"] }
  nanorand = { version = "0.7", features = ["getrandom"] }  # if used directly
  ```
  This is target-scoped, so native builds are unaffected.
- **Canvas sizing**: `canvas { display: block; width: 100vw; height: 100vh; }`
  or call `request_screen_size()` in code.
- **No file:// access**: always serve over HTTP. Trunk's dev server handles
  this automatically.
