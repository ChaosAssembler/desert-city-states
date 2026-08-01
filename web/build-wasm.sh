#!/bin/sh
# Builds the wasm binary, then vendors macroquad's own js/mq_js_bundle.js —
# the browser-side runtime the binary needs (WebGL setup, canvas/input
# glue, the load() entry point, and macroquad's audio/sapp_jsutils/quad_net
# plugin registration) — from the exact macroquad version pinned in
# Cargo.lock.
#
# macroquad is pinned to exactly 0.4.14 in crates/dcs-render/Cargo.toml
# specifically so this works: 0.4.15/0.4.16 (and the current git master)
# ship a js/mq_js_bundle.js with a known, unfixed regression
# (not-fl3/macroquad#1055) — a bare `register_plugin = ...` assignment in
# the quad_net plugin section throws under the bundle's own "use strict"
# on every page load. 0.4.14 predates the regressing commit and is
# byte-identical (confirmed via md5) to the known-good copy the official
# docs point at (https://not-fl3.github.io/miniquad-samples/mq_js_bundle.js).
#
# Pulling it from the already-fetched Cargo registry cache — rather than
# that CDN, or hand-vendoring a copy — keeps it byte-for-byte in sync with
# the exact macroquad version actually being built, with no extra network
# dependency and no large unreadable file sitting in git history. If you
# bump the macroquad version pin, re-verify the new bundle doesn't have
# this bug before trusting it: `grep register_plugin web/vendor/mq_js_bundle.js`
# should show it only as an object-property key (`register_plugin:`), never
# as a bare `register_plugin=` assignment.
set -eu
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown --bin dcs-web -p dcs-app

macroquad_version=$(awk '/^name = "macroquad"$/{getline; print; exit}' Cargo.lock | sed -E 's/version = "(.*)"/\1/')
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
bundle_js=$(find "$cargo_home/registry/src" -maxdepth 2 -type d -name "macroquad-${macroquad_version}" -print -quit)/js/mq_js_bundle.js

mkdir -p web/vendor
cp "$bundle_js" web/vendor/mq_js_bundle.js
