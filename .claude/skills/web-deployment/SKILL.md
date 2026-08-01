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
the project README): they use exactly this HTML shape. Trunk's `rel="rust"`
asset pipeline **always** runs `wasm-bindgen` on the compiled output with no
supported way to disable it — using it on a macroquad binary either breaks
or requires an unofficial, fragile `sed`-patching community shim. Don't use
`rel="rust"`.

Instead, Trunk is used only as a **dev server / file watcher / static asset
pipeline**: the actual wasm compile happens in a Trunk `pre_build` hook,
brought into `dist/` via `rel="copy-file"`. `mq_js_bundle.js` itself is
loaded directly from its official CDN in `index.html` with a Subresource
Integrity (SRI) hash — nothing to vendor, copy, or gitignore for it at all.

## Preconditions

- A dedicated, minimal wasm entry point binary (e.g. `src/bin/dcs-web.rs`)
  that just calls the renderer's `run()` — no CLI parsing (`clap`), no
  networking/serving code. Cargo auto-discovers `src/bin/*.rs` as binary
  targets; no `[[bin]]` stanza needed.
- **`macroquad` is pinned to exactly `=0.4.14` in
  `crates/dcs-render/Cargo.toml`, not `"0.4"`.** 0.4.15, 0.4.16 (latest
  published), and the current `master` on GitHub all ship a
  `js/mq_js_bundle.js` with a real, reproducible, currently-unfixed
  upstream bug (not-fl3/macroquad#1055, open since 2026-07-21): its
  `quad_net` plugin section does a bare `register_plugin = ...` assignment
  with no prior declaration, which throws `ReferenceError: assignment to
  undeclared variable` under the bundle's own `"use strict"` the instant it
  loads. Doesn't break rendering (`quad_net` is unused networking glue;
  `load()` is a hoisted function declaration so it's still defined despite
  the throw) but it's a real console error on every single page load —
  exactly what an agent watching for console errors as a pass/fail signal
  would false-positive on. `0.4.14` predates the regressing commit
  (`fbc3f90`). **Keep this pin in lockstep with the SRI hash in
  `index.html`** (see below) — they identify the same known-good release;
  don't bump one without the other. Bump both once #1055 is fixed
  upstream, and re-verify (Common Issues below) before trusting a newer
  bundle.
- `.cargo/config.toml` sets `--allow-undefined` for the `wasm32-unknown-unknown`
  target (see Common Issues below) — without it, the build fails to link.

## Rules

- Never use `std::fs` or `std::process::exit` in wasm-targeted code — they
  panic at runtime in the browser.
- Don't commit or vendor `mq_js_bundle.js` — load it directly from the CDN
  with an SRI hash instead (see Workflow). Before pinning that hash to
  *any* build of the bundle — CDN, a crate's own package, or otherwise —
  verify the actual bytes, don't just assume a version-pinned source is
  correct because it's pinned: a locally available exact-version match can
  still ship a real bug nobody hit because nobody sources it that way (see
  the `register_plugin` bug above).

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
   - `[[hooks]]` with `stage = "pre_build"`, plain inline `cargo build
     --release --target wasm32-unknown-unknown --bin <name> -p <crate>` — no
     script needed; there's nothing left to vendor.
   - `[watch] ignore = ["../target", "../dist", "../docs", "../.git"]` —
     **required**, otherwise Trunk's watcher sees the hook's own `cargo
     build` output land in `target/` and rebuild-loops forever.
2. **`index.html`**, alongside `Trunk.toml` in `web/`:
   - `<canvas id="glcanvas" tabindex="1">` — the id must be exactly
     `glcanvas`, matching what `mq_js_bundle.js` queries for.
   - `<link data-trunk rel="copy-file" href="../target/wasm32-unknown-unknown/release/<bin>.wasm">`
     to bring the hook's output into `dist/`.
   - `<script src="https://not-fl3.github.io/miniquad-samples/mq_js_bundle.js" integrity="sha384-..." crossorigin="anonymous"></script>`
     — loaded directly, not `data-trunk`-managed. The `integrity` hash
     means the browser refuses to execute the script if the served bytes
     ever change (verified empirically: corrupting the hash produces a
     hard `SRI mismatch` console error and the script simply doesn't run —
     loud failure, not silent drift). This does mean the page needs
     network access to `not-fl3.github.io` at *runtime*, not just build
     time — acceptable here since the whole point is a browser-driven
     (Playwright) test target, but worth knowing if this ever needs to run
     fully offline.
   - `<script>load("<bin>.wasm");</script>`.
   - CSS: `body { margin:0; overflow:hidden; }`,
     `canvas { display:block; width:100vw; height:100vh; }`.
3. Build output: `cd web && trunk build` (or `--release`) produces
   repo-root `dist/` with just the `.wasm` and `index.html` — no JS file
   copied in, it's fetched by the browser at load time.  `trunk serve` runs
   a dev server with rebuild-on-change. Both must be run with `web/` as the
   working directory, per the quirks above.
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
- **Bumping the pinned `macroquad`/SRI-hash pair**: fetch the candidate
  version's `js/mq_js_bundle.js` (from its crates.io package or the CDN,
  whichever you're pinning to), confirm
  `grep register_plugin` shows it only as an object-property key
  (`register_plugin:`), never a bare `register_plugin=` assignment, then
  recompute the hash: `openssl dgst -sha384 -binary mq_js_bundle.js | openssl base64 -A`.
  Update both the `macroquad` version in `crates/dcs-render/Cargo.toml` and
  the `integrity` attribute in `index.html` together.
- **Blank/black canvas with no console errors, `gl`/`wasm_exports` present**:
  before suspecting the build, check whether the browser tab is stale — a
  WebGL context reused across several rapid dev-server restarts in the same
  tab (common while iterating on `Trunk.toml`/hooks) can silently fail to
  (re-)render even though the module loaded correctly. Test in a fresh tab.
- **Canvas sizing**: `canvas { display: block; width: 100vw; height: 100vh; }`
  or call `request_screen_size()` in code.
- **No file:// access**: always serve over HTTP. Trunk's dev server handles
  this automatically.
