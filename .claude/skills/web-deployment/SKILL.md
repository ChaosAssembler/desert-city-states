---
name: web-deployment
description: Use when creating or maintaining web-facing source files for the Trunk-based macroquad wasm build — HTML, Trunk config, and static assets
---

# Web Deployment for Macroquad Games

## Why this doesn't look like a typical Trunk/wasm-bindgen setup

**macroquad does not use `wasm-bindgen`.** It ships its own JS runtime
(`mq_js_bundle.js`) that loads a plain `wasm32-unknown-unknown` binary
directly via `load("file.wasm")` — no `cdylib`, no `#[wasm_bindgen]`
exports, no `#[macroquad::main]` magic needed for wasm specifically. This
matches the official macroquad docs (`mq.agical.se` "Build for the web",
the project README): they use exactly this HTML shape, and explicitly
recommend hosting a local copy of `mq_js_bundle.js` rather than trusting a
CDN at runtime. Trunk's `rel="rust"` asset pipeline **always** runs
`wasm-bindgen` on the compiled output with no supported way to disable it —
using it on a macroquad binary either breaks or requires an unofficial,
fragile `sed`-patching community shim. Don't use `rel="rust"`.

Instead, Trunk is used only as a **dev server / file watcher / static asset
pipeline**. The actual wasm compile (and fetching `mq_js_bundle.js` — see
below) happens in a Trunk `pre_build` hook, and the results are brought
into `dist/` via `rel="copy-file"`.

## Preconditions

- A dedicated, minimal wasm entry point binary (e.g. `src/bin/dcs-web.rs`)
  that just calls the renderer's `run()` — no CLI parsing (`clap`), no
  networking/serving code. Cargo auto-discovers `src/bin/*.rs` as binary
  targets; no `[[bin]]` stanza needed.
- **`mq_js_bundle.js` fetched at build time from
  `https://not-fl3.github.io/miniquad-samples/mq_js_bundle.js`, cached
  locally, never committed.** This was not the first thing tried here —
  worth knowing why, since the alternative looks more correct on paper:
  - The `macroquad` crate ships its own copy of this exact file in its
    package source (`js/mq_js_bundle.js`), reachable via the local Cargo
    registry cache and exactly version-matched to `Cargo.lock`. That
    seemed strictly better (hard version pinning, no network dependency,
    no "which URL" ambiguity) — but it has a **real, reproducible bug**,
    confirmed present in both macroquad 0.4.15 and 0.4.16: its `quad_net`
    plugin section does a bare `register_plugin = ...` assignment with no
    prior declaration, which throws `ReferenceError: assignment to
    undeclared variable` under the bundle's own `"use strict"` the instant
    it loads. It doesn't actually break rendering here (`quad_net` is
    networking, unused; `load()` is a hoisted function declaration so it's
    still defined despite the throw) — but it's a real console error every
    single page load, which defeats the point of an agent watching for
    console errors as a test signal.
  - The CDN-hosted copy doesn't have this bug (different, correctly-scoped
    minification of the same `quad_net` section) and is what the whole
    macroquad ecosystem actually tests against in practice — official docs
    universally point there instead of at the published crate's copy.
    Unpinned ("whatever's currently hosted") is the tradeoff, but it's the
    one that's known to work.
  - `web/build-wasm.sh` fetches it into `web/vendor/mq_js_bundle.js` only
    if not already present (delete that file to force a refresh).
    `web/vendor/` is gitignored except a tracked `.gitkeep` placeholder
    (see Common Issues — Trunk needs the directory to already exist at
    startup).
- `.cargo/config.toml` sets `--allow-undefined` for the `wasm32-unknown-unknown`
  target (see Common Issues below) — without it, the build fails to link.

## Rules

- Never use `std::fs` or `std::process::exit` in wasm-targeted code — they
  panic at runtime in the browser.
- Don't commit generated/vendored JS blobs (`mq_js_bundle.js` above) —
  fetch/generate them at build time instead. If a "generate from a
  version-pinned local source" option and a "fetch from the URL everyone
  actually uses" option disagree, check whether the pinned option is
  actually correct before assuming pinning wins — see the `register_plugin`
  bug above for a concrete case where it didn't.

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
   - `[[hooks]]` with `stage = "pre_build"` running `sh build-wasm.sh` (see
     Preconditions) — a script rather than an inline `cargo build` because
     it also handles fetching `mq_js_bundle.js`.
   - `[watch] ignore = ["../target", "../dist", "../docs", "../.git", "vendor"]`
     — **required**, otherwise Trunk's watcher sees the hook's own output
     land in `target/`/`dist/` (paths outside `web/`) *and* in `web/vendor/`
     (**inside** the watched directory, since that's where `mq_js_bundle.js`
     gets written) and rebuild-loops forever. The last one is easy to miss
     because it looks nothing like the usual `target/`-loop symptom — watch
     for *any* directory a hook writes into, not just the obvious ones.
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
- **Trunk fails at startup with `error taking the canonical path to the
  watch ignore path`**: every `[watch] ignore` entry must already exist on
  disk — Trunk canonicalizes each one at startup, before any hook has run.
  If a hook generates a directory (e.g. `web/vendor/`, see Preconditions),
  track it in git via an empty `.gitkeep` placeholder (gitignore the
  generated contents, not the directory itself) so it survives a fresh
  clone.
- **Blank/black canvas with no console errors, `gl`/`wasm_exports` present**:
  before suspecting the build, check whether the browser tab is stale — a
  WebGL context reused across several rapid dev-server restarts in the same
  tab (common while iterating on `Trunk.toml`/hooks) can silently fail to
  (re-)render even though the module loaded correctly. Test in a fresh tab.
- **Canvas sizing**: `canvas { display: block; width: 100vw; height: 100vh; }`
  or call `request_screen_size()` in code.
- **No file:// access**: always serve over HTTP. Trunk's dev server handles
  this automatically.
