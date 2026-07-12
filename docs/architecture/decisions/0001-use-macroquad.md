# 0001-use-macroquad

## Title
Rendering engine = macroquad (2D, immediate-mode)

## Status
Accepted

## Context
Desert City States is a small-scale, turn-based hex 4X. The presentation requirement is 2D graphical (not terminal/TUI). For the MVP we need to ship a fast, readable board quickly: low API churn, a built-in UI for HUD/menus, and easy pan/zoom camera control. The simulation advances only on End Turn (not per frame), so a retained-mode scene graph is unnecessary overhead. The architecture also requires rendering to be fully isolated (Rule B) so the engine can be swapped without touching the pure core.

## Decision
Adopt **macroquad** as the rendering engine: a 2D, immediate-mode Rust library with built-in UI primitives, minimal dependencies, and a simple camera/input model that fits our pan/zoom and HUD needs. **Bevy is kept only as a documented fallback** for the case where scope explodes (e.g., heavy retained-mode UI / scene-graph needs). Because all rendering is confined to `dcs-render`, switching engines would rewrite only that crate while `dcs-core` and `dcs-protocol` stay untouched.

## Alternatives
- **Bevy:** full ECS + retained-mode renderer. Powerful but heavier API churn and overkill for turn-based, frame-static rendering; retained as fallback only.
- **egui (standalone):** good immediate-mode UI but not a game renderer; noted as a HUD fallback if macroquad's built-in UI outgrows us.
- **Raw wgpu/graphics:** maximum control, far too much boilerplate for MVP.

## Consequences
- The render layer depends on macroquad; `dcs-render` is the only crate aware of it.
- The pure `dcs-core` and `dcs-protocol` crates remain engine-agnostic and swappable.
- A documented Bevy fallback path exists but is not pursued unless scope demands.
- The architecture's Rule B (rendering isolation) is what makes the fallback feasible.
