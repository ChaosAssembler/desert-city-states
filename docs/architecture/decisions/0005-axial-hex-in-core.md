# 0005-axial-hex-in-core

## Title
Axial hex coordinates, implemented in-core

## Status
Accepted

## Context
The game uses a **pointy-top hex grid** (DD §5.1). We need a small, stable set of coordinate operations — conversions to cube/pixel, neighbors, distance, rings/ranges, line-draw, and A*/Dijkstra pathfinding — and these operations must be perfectly deterministic because they feed map generation, movement, route planning, and serialized saves/replays.

## Decision
Use **axial coordinates** `(q, r)` with the invariant `s = -q - r` (cube coordinate `s` derived on demand), and implement a **minimal hex math module inside `dcs-core`** (`dcs-core::hex`). No external hex crate is used. The module provides: axial↔cube↔pixel conversions (pointy-top), the 6 axial direction vectors, `neighbors`, cube-distance, `ring`/`range`, `line` (lerp + cube-round), and A*/threat-weighted Dijkstra pathfinding over the hex graph (used for unit movement and caravan route planning).

## Alternatives
- **External hex crate (`hex2d`/`hexagonal`):** viable, but introduces a dependency into `dcs-core` (violates the zero-extra-dependency spirit of Rule A) and an external, unaudited rounding/ordering rule that is a determinism risk for saves/replays. The needed surface (~150 lines) is small and stable, so we own it.
- **Offset coordinates:** rejected — distance/neighbor math is more error-prone than axial/cube.

## Consequences
- Full control over a single, auditable rounding rule (cube-round), guaranteeing deterministic saves/replays.
- Keeps `dcs-core` dependency-free for hex math, consistent with Rule A.
- We must maintain and test the hex math ourselves (low cost at this surface).
