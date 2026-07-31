---
name: web-deployment
description: Use when creating or maintaining web-facing source files for a Trunk-based macroquad wasm game — HTML, Trunk config, and static assets
---

# Web Deployment for Macroquad Games

## Preconditions

Before running this workflow, ensure:

- The binary crate's `Cargo.toml` has `[lib] crate-type = ["cdylib", "rlib"]`.
- The Rust source has `#[macroquad::main]` wrapping the entry point and wasm-compatible code paths.

## Rules

- Use Trunk as the build tool. It handles wasm-bindgen, produces a clean `dist/` output.
- Never use `std::fs` or `std::process::exit` in wasm-targeted code — they panic at runtime in the browser.

## Workflow

1. **Determine the binary crate**: Identify which crate has the `main()` function (e.g., `crates/dcs-app`).
2. **Create `index.html`** at the project root with minimal HTML shell: `<meta viewport>`, `<style>` for canvas sizing (`body { margin:0; overflow:hidden; }`), empty `<body>`. Trunk auto-injects the wasm script.
3. **Create `Trunk.toml`** pointing `[build].target` at the binary crate's Cargo.toml.

## Conventions

- `index.html` goes at the project root or next to the binary crate's `Cargo.toml`.
- `Trunk.toml` goes at the project root.
- Static assets (sprites, fonts, sounds) go in `assets/` and are loaded via macroquad's `load_*()` functions. Use Trunk's `data-trunk` attribute in `index.html` to embed assets.
- The `dist/` directory is gitignored (add to `.gitignore` if not already).
- Build output: `trunk build --release` produces `dist/` with `.wasm`, `.js` glue, and `index.html`.

## Common Issues on Wasm

- **Panics are silent**: Add `console_error_panic_hook` to log them to the browser console.
- **`getrandom` errors**: macroquad pulls in `getrandom` transitively with the correct wasm features. If a direct dependency (e.g., `nanorand`) needs it, add `getrandom = { version = "0.2", features = ["js"] }` under `[target.'cfg(target_arch = "wasm32")'.dependencies]`.
- **Canvas sizing**: Use CSS `canvas { display: block; width: 100vw; height: 100vh; }` or call `request_screen_size()` in code.
- **No file:// access**: Always serve over HTTP. Trunk's dev server handles this automatically.
