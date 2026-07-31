#!/bin/sh
# Builds the wasm binary, then fetches mq_js_bundle.js — the browser-side
# runtime the binary needs (WebGL setup, canvas/input glue, the load()
# entry point, and macroquad's audio/sapp_jsutils/quad_net plugin
# registration) — from the URL the official macroquad docs point at.
#
# This is NOT sourced from the macroquad crate's own bundled copy
# (js/mq_js_bundle.js in the crates.io package), even though that would be
# exact-version-pinned via Cargo.lock and was the first thing tried here:
# that published copy has a real bug, confirmed present in both 0.4.15 and
# 0.4.16 — its quad_net plugin section does a bare `register_plugin = ...`
# assignment with no prior declaration, which throws
# `ReferenceError: assignment to undeclared variable` under the bundle's
# own "use strict" the moment it loads. Harmless in practice here (quad_net
# is networking, unused, and `load()` is hoisted so the game still renders)
# but it pollutes every console-error check an agent runs. The CDN-hosted
# copy is what the whole macroquad ecosystem actually tests against and
# doesn't have this bug, so that's the one to use despite being an
# unpinned "whatever's currently there" artifact rather than a Cargo.lock-
# exact one. Cached locally so it's still never committed and builds don't
# refetch every time; delete web/vendor/mq_js_bundle.js to force a refresh.
set -eu
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown --bin dcs-web -p dcs-app

mkdir -p web/vendor
if [ ! -f web/vendor/mq_js_bundle.js ]; then
  curl -fsSL -o web/vendor/mq_js_bundle.js \
    https://not-fl3.github.io/miniquad-samples/mq_js_bundle.js
fi
