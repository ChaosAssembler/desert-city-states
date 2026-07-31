#!/bin/sh
# Builds the wasm binary, then vendors miniquad's js/gl.js — the browser-side
# runtime the binary needs (WebGL setup, canvas/input glue, the load()
# entry point) — from the exact miniquad version pinned in Cargo.lock.
#
# This is generated, not committed: pulling it from the already-fetched
# Cargo registry cache keeps it byte-for-byte in sync with the miniquad
# version actually being built, with no extra network dependency and no
# large unreadable file sitting in git history.
set -eu
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown --bin dcs-web -p dcs-app

miniquad_version=$(awk '/^name = "miniquad"$/{getline; print; exit}' Cargo.lock | sed -E 's/version = "(.*)"/\1/')
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
gl_js=$(find "$cargo_home/registry/src" -maxdepth 2 -type d -name "miniquad-${miniquad_version}" -print -quit)/js/gl.js

mkdir -p web/vendor
cp "$gl_js" web/vendor/gl.js
