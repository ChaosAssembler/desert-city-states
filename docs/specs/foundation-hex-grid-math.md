# Foundation Spec: Hex Grid Math

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (module `dcs-core::hex`)
> **Status:** Draft for review
> **Implements:** DD §5.1; ARCH §4; ADR-0005 (axial hex, implemented in-core)

---

## 1. Purpose

Define the minimal, deterministic hex-grid math module that every other system
depends on for **coordinates, conversion, neighborhood, distance, ranges, line
drawing, and pathfinding over the hex graph**. This module is the single source
of geometric truth in `dcs-core`; world generation, movement (A*), caravan
routing (threat-weighted Dijkstra), fog reveal, and rendering all build on it.

The module is intentionally tiny and **vendor-free** (ADR-0005): a hand-rolled
implementation gives us one auditable rounding rule (`cube_round`) so that map
generation, movement, route planning, saves, and replays are bit-for-bit
deterministic. No external hex crate is used (ARCH §4.4).

## 2. Scope

**In scope**

- The `HexCoord` type and the `s = -q - r` cube invariant.
- Axial↔cube↔pixel conversions (pointy-top) with a single `cube_round` rule.
- Neighbor direction table, `neighbors`, `distance`, `ring`, `range`, `line`.
- Map-membership test `in_map` for a hexagonal map of a given radius.
- Generic graph pathfinding primitives `astar` and `safe_route` over the hex graph.

**Out of scope**

- Terrain/move-cost tables and ownership lookups (those live in `GameState` /
  balance tables — caravan & units specs). The hex module is geometry-only and
  takes pure closures for passability / cost / threat.
- The `Vec<TileId>`-shaped gameplay wrappers (`world::astar`, `caravan::safe_route`)
  — those are defined in their own specs and *call into* this module (see §5.4).
- Camera, screen mapping, `Vec2` — macroquad types never enter `dcs-core`
  (ADR-0003). The renderer inverts camera + calls `from_pixel` itself.
- Map generation algorithm, turn loop, rendering (other specs).

## 3. Responsibilities

- Provide **pure** coordinate math: identical inputs → identical outputs, no RNG,
  no global state (ADR-0003, ADR-0005).
- Guarantee a **single, auditable `cube_round`** so pixel→hex is exact and matches
  pathfinding (render's `screen_to_hex` must use this same rounding — ADR-0005).
- Expose **generic** pathfinding that is parameterised by passability/cost/threat
  closures, so unit-movement A* and caravan safe-route share one implementation.
- Keep `dcs-core::hex` dependency-free: std + (optionally) serde only.

## 4. Core Data Structures

```rust
/// Pointy-top axial hex coordinate. The cube coordinate s is always derived as
/// s = -q - r; we never store it. This is the type referenced as `HexCoord`
/// throughout the data model, world-gen, and rendering specs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

/// Pointy-top layout parameters. Pure geometry — no macroquad dependency.
/// `size` is the hex "radius" in world pixels (ARCH §4.1 `hex_size`).
/// `origin` is the world-space pixel of hex (0,0); usually (0.0, 0.0) because the
/// renderer applies its own camera offset (ARCH §8, rendering spec §6.1).
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub size: f32,
    pub origin: (f32, f32),
}

/// Deterministic ordering helper: lexicographic (q, r). Used for tie-breaking
/// when multiple equal-cost neighbors are enqueued during pathfinding.
impl Ord for HexCoord { /* compares (q, r) */ }
```

> **Naming note.** Consumers (core-data-model, world-gen, rendering) call this
> type `HexCoord`; the architecture (ARCH §4.1) defines it identically. This spec
> uses `HexCoord` to match those references. The render spec calls the
> pixel→hex conversion `hex::from_pixel` (and refers to it loosely as
> `pixel_to_hex`); this spec defines it canonically as `from_pixel` and notes the
> alias. `to_pixel`/`from_pixel` take a scalar `size` exactly as ARCH §4.1 and the
> rendering spec do (`hex::to_pixel(coord, hex_size)`); `Layout` is a thin
> convenience bundling `size` + `origin` and is optional.

## 5. Key Functions / API

### 5.1 Coordinate conversions

```rust
/// Axial -> cube. s is derived, never stored.
pub fn to_cube(h: HexCoord) -> (i32, i32, i32);        // (x=q, y=-q-r, z=r)

/// Cube -> axial. Discards y (= -x - z); keeps q=x, r=z.
pub fn from_cube(x: i32, y: i32, z: i32) -> HexCoord;  // q=x, r=z

/// Axial -> world pixel (pointy-top). Returns raw (x, y) so core stays free of
/// macroquad Vec2 (ADR-0003). The renderer maps (x, y) through its Camera2D.
pub fn to_pixel(h: HexCoord, size: f32) -> (f32, f32);

/// Pixel -> axial with the single auditable cube-round (ADR-0005). This is the
/// inverse of `to_pixel` and the function the renderer calls (its `pixel_to_hex`
/// / `screen_to_hex` both delegate here after camera inversion).
pub fn from_pixel(p: (f32, f32), size: f32) -> HexCoord;

/// Convenience that also applies a non-origin layout offset. Equivalent to
/// translating `p` by `-origin` then calling `from_pixel(p, size)`.
pub fn pixel_to_hex(p: (f32, f32), layout: Layout) -> HexCoord;

/// Convenience wrapper: `to_pixel` plus `layout.origin`. Provided for callers
/// that bundle size+origin; the renderer usually calls the scalar `to_pixel`
/// and applies the camera itself.
pub fn to_pixel_layout(h: HexCoord, layout: Layout) -> (f32, f32);

/// The single rounding rule. Rounds fractional cube coords to the nearest hex;
/// fixes the component with the largest rounding error so x+y+z == 0 holds.
/// MUST be the only rounding used anywhere (pathfinding, line, input).
pub fn cube_round(x: f32, y: f32, z: f32) -> (i32, i32, i32);
```

### 5.2 Neighborhood, distance, ranges, lines

```rust
/// The 6 axial direction vectors for pointy-top hexes (constant table).
pub const AXIAL_DIRS: [(i32, i32); 6];

/// The 6 neighbor coordinates of `h`, in a fixed deterministic order
/// (matching AXIAL_DIRS).
pub fn neighbors(h: HexCoord) -> [HexCoord; 6];

/// Cube distance between two hexes (always u32; non-negative).
pub fn distance(a: HexCoord, b: HexCoord) -> u32;

/// All hexes at exactly `radius` steps from `center` (the worked ring is
/// `ring(center, 1)`). Ordered deterministically (start at `center + AXIAL_DIRS[0]*radius`,
/// walk each of the 6 edges).
pub fn ring(center: HexCoord, radius: u32) -> Vec<HexCoord>;

/// All hexes within `radius` steps of `center` (inclusive), used for fog reveal
/// and AoE. Size = 1 + 3*radius*(radius+1) for a full disc.
pub fn range(center: HexCoord, radius: u32) -> Vec<HexCoord>;

/// Line of hexes from `a` to `b` (inclusive) via lerp + cube_round.
/// `line(a, b).len()` == `distance(a, b) as usize + 1`.
pub fn line(a: HexCoord, b: HexCoord) -> Vec<HexCoord>;

/// Map membership for a hexagonal map centered at (0,0) with the given `radius`.
/// A hex is in-map iff cube distance from origin <= radius. Used by world-gen
/// step 0 and any map-bounded query.
pub fn in_map(h: HexCoord, radius: u32) -> bool;
```

### 5.3 Pathfinding primitives (generic, geometry-only)

These are the functions the gameplay modules wrap. They take pure closures so
the hex module knows nothing about `GameState`, terrain, or ownership.

```rust
/// A* over the hex graph between two coordinates.
/// - `passable(h)` returns false for tiles that cannot be entered (caller decides:
///   e.g. enemy-occupied, off-map). Neighbors failing `passable` are skipped.
/// - `cost(from, to)` is the step cost into neighbor `to` (caller supplies
///   move_cost / enemy-avoidance weighting).
/// - Returns the path EXCLUDING `start` and INCLUDING `goal`, or `None` if
///   unreachable. Tie-break by `HexCoord` Ord (lexicographic (q, r)) for
///   determinism (ADR-0005).
pub fn astar(
    start: HexCoord,
    goal: HexCoord,
    passable: impl Fn(HexCoord) -> bool,
    cost: impl Fn(HexCoord, HexCoord) -> f32,
) -> Option<Vec<HexCoord>>;

/// Threat-weighted Dijkstra (single-source shortest path) for caravan routing.
/// - `threat_fn(h)` returns the threat penalty of stepping onto `h`
///   (caller supplies: 0.0 safe, +THREAT_EXPOSED_FLAT for exposed Salt Flats,
///   +THREAT_ENEMY_TILE for enemy tiles, Ridges 0.0 — see caravan spec §4.1/§6.1).
/// - Edge weight into neighbor `to` = `base_cost * (1.0 + threat_fn(to))`,
///   so a safe tile costs `base_cost`, an enemy tile ~3x, exposed Salt Flats 2x.
/// - Returns `(path, total_cost)` with `path` INCLUSIVE of both endpoints, or
///   `None` if `goal` is unreachable. Deterministic tie-break by `HexCoord` Ord.
pub fn safe_route(
    start: HexCoord,
    goal: HexCoord,
    threat_fn: impl Fn(HexCoord) -> f32,
    base_cost: f32,
) -> Option<(Vec<HexCoord>, f32)>;
```

### 5.4 How the gameplay wrappers use these primitives

The gameplay specs define higher-level wrappers with `TileId` / `&GameState`
signatures; they call the primitives above. This keeps the *names* those specs
assume intact while the geometry lives here:

- `world::astar(state, from: TileId, to: TileId) -> Option<Vec<TileId>>`
  (gameplay-units-movement §5) builds on `hex::astar` with
  `passable = |h| in_map(h, radius) && !enemy_blocked(h)` and
  `cost = |_, to| TERRAIN[to].move_cost + ENEMY_AVOID_PENALTY(to)`.
  Edge cost table (Oasis 1, Dunes 2, SaltFlats 1, Ridges 3, Ruins 1) and the
  enemy-avoidance bump come from the units spec §6.2; determinism tie-break by
  `TileId` there reduces to `HexCoord` Ord here.

- `caravan::safe_route(state, from: TileId, to: TileId) -> Vec<TileId>`
  (gameplay-caravan-routes §5) builds on `hex::safe_route` with
  `base_cost = 1.0` and `threat_fn` implementing the caravan §6.1 formula:
  `threat_penalty(b) = THREAT_ENEMY_TILE (2.0)` if enemy-owned/adjacent,
  else `THREAT_EXPOSED_FLAT (1.0)` if `SaltFlats && !controlled`, else `0.0`.
  Ridges carry only their high `move_cost` (3) and **no** threat penalty, so they
  are safe-but-long (caravan §4.1, §6.7). Output is the inclusive `Vec<TileId>`.

> The renderer never calls `astar`/`safe_route`; it only uses `to_pixel` /
> `from_pixel` (via `screen_to_hex`) and `range` (not directly). Pathfinding is
> core-only.

## 6. Algorithms / Formulas

### 6.1 Coordinate system (pointy-top, axial `(q, r)`)

Cube coordinates are derived on demand: **x = q, z = r, y = -q - r**, so the
invariant `x + y + z == 0` (equivalently `s = -q - r`) holds for every valid hex.
Conversions are pure and lossless in integer space.

### 6.2 Axial ↔ pixel (pointy-top, size = hex radius in px)

```text
x_px = size * (SQRT3 * q + SQRT3/2 * r) + origin.x
y_px = size * (3/2 * r)                       + origin.y
```

where `SQRT3 = √3`. Inverse (before rounding):

```text
q_f = (SQRT3/3 * (x_px - origin.x) - 1/3 * (y_px - origin.y)) / size
r_f = (2/3 * (y_px - origin.y)) / size
```

then convert to cube (`x=q_f, z=r_f, y=-x-z`) and apply `cube_round`.

### 6.3 `cube_round` (the single rounding rule, Red Blob style)

```text
rx = round(x); ry = round(y); rz = round(z)
dx = abs(rx-x); dy = abs(ry-y); dz = abs(rz-z)
if dx > dy && dx > dz { rx = -ry - rz }
else if dy > dz       { ry = -rx - rz }
else                  { rz = -rx - ry }
return (rx, ry, rz)   // guarantees rx+ry+rz == 0
```

This is the ONLY rounding used anywhere (pixel→hex, `line`, pathfinding tie
ordering). It makes `screen_to_hex` land on the exact hex the resolver uses
(ADR-0005 determinism).

### 6.4 Neighbor directions (pointy-top axial)

```text
AXIAL_DIRS = [ ( 1, 0), ( 1, -1), ( 0, -1),
               (-1, 0), (-1,  1), ( 0,  1) ]
```
`neighbors(h)` = `[h + d for d in AXIAL_DIRS]` in this fixed order.

### 6.5 Distance (cube distance)

```rust
let (aq,_,ar) = to_cube(a); let (bq,_,br) = to_cube(b);
distance = ((aq-bq).abs().max((ar-br).abs()).max((aq+bq+ ... )))
         = (|aq-bq| + |ar-br| + |aq+ar - (bq+br)|) / 2   // == cube max-norm
```

Expressed via cube coordinates `(x,y,z)`: `max(|Δx|, |Δy|, |Δz|)`, returned as `u32`.

### 6.6 `ring` and `range`

- `ring(center, radius)`: for `radius == 0` returns `[center]`; otherwise start at
  `center + AXIAL_DIRS[4] * radius` and walk `radius` steps along each of the 6
  edges in `AXIAL_DIRS` order. Size is exactly `6 * radius` for `radius > 0`.
- `range(center, radius)`: union of `ring(center, k)` for `k in 0..=radius`
  (dedup), or equivalently all `h` with `distance(center, h) <= radius`. Size
  `1 + 3*radius*(radius+1)`.

### 6.7 `line` (lerp + cube_round)

```text
n = distance(a, b)
for i in 0..=n:
    t = if n == 0 { 0.0 } else { i as f32 / n as f32 }
    x = lerp(a.x, b.x, t); y = lerp(a.y, b.y, t); z = lerp(a.z, b.z, t)
    push cube_round(x, y, z)
```

Result includes both endpoints; `line(a, b).len() == n + 1`.

### 6.8 `in_map`

```text
in_map(h, radius) = distance(HexCoord{q:0,r:0}, h) <= radius
```

A hexagonal map of `radius` contains `1 + 3*radius*(radius+1)` hexes (radius 4 →
61; radius 9 → 271).

### 6.9 Pathfinding

- `astar`: standard A* with a binary min-heap keyed by `f = g + h`, heuristic
  `h = distance(current, goal)` (admissible for unit-cost movement). Graph = the
  in-map hex neighbors passing `passable`. Deterministic tie-break by `HexCoord`
  `Ord` when `f` ties, so repeated runs / replays are identical.
- `safe_route`: Dijkstra (no heuristic needed; cost is non-negative because
  `base_cost >= 0` and `threat_fn >= 0`). Edge weight into `to` =
  `base_cost * (1.0 + threat_fn(to))`. Binary heap; same deterministic tie-break.
  Returns the inclusive path plus accumulated `total_cost`.

## 7. Edge Cases / Invariants

- **Invariant (cube):** for every `HexCoord`, `q + r + (-q - r) == 0`. No function
  ever stores `s`; it is always derived.
- **Single rounding:** `cube_round` is the only rounding path. `screen_to_hex`,
  `line`, and any future pixel snapping all funnel through it — guaranteeing
  click↔tile↔pathfinding agreement (ADR-0005, rendering spec §6.1/§7).
- **Map bounds:** `to_pixel` never fails, but callers must use `in_map` before
  treating a coordinate as a real tile (world-gen, pathfinding `passable`).
- **Determinism:** no float comparison for equality of hexes, no iteration-order
  dependence; pathfinding tie-break is `HexCoord` `Ord` (lexicographic (q, r)),
  which the gameplay wrappers refine to `TileId` order (units/caravan specs).
- **`ring`/`range` size:** `ring(_, radius>0)` has exactly `6*radius` elements;
  `range` size is `1 + 3*radius*(radius+1)`.
- **`distance` symmetry & metric:** `distance(a,b) == distance(b,a)`; satisfies
  triangle inequality (used as A* heuristic).
- **Rounding of `to_pixel` round-trip:** `from_pixel(to_pixel(h, size), size) == h`
  exactly for integer hexes (no accumulation error), so rendering and logic agree.
- **No macroquad:** `to_pixel`/`from_pixel` return `(f32, f32)`; `dcs-render`
  converts to its `Vec2`/`Camera2D`. Core never imports macroquad (ADR-0003).
- **Ridges in safe-route:** `threat_fn` returns 0.0 for Ridges (caravan §4.1), so
  their only cost is the high `move_cost` (3) — "safe-but-long", never a threat.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `HexCoord` derives `Serialize`/`Deserialize`, `Ord` (lexicographic (q,r)), `Hash`.
- [ ] Cube invariant: for all sampled `h`, `to_cube(h)` sums to 0.
- [ ] `neighbors(h)` returns exactly 6 distinct coordinates; each is `distance 1` from `h`.
- [ ] `distance` is symmetric and non-negative; `distance(h, h) == 0`; triangle inequality holds.
- [ ] `ring(center, radius)` size == `6*radius` for `radius > 0`; `range` size == `1 + 3*radius*(radius+1)`.
- [ ] `line(a, b)` starts at `a`, ends at `b`, length `== distance(a,b)+1`, all intermediate hexes adjacent.
- [ ] `in_map` true for all `1 + 3*radius*(radius+1)` hexes; false outside.
- [ ] Pixel round-trip: `from_pixel(to_pixel(h, s), s) == h` for integer `h` (exact).
- [ ] `cube_round` returns coordinates summing to 0 for fractional inputs.
- [ ] `astar` returns the minimum move-cost path (excludes start, includes goal); `None` when `passable` blocks all routes; deterministic for fixed inputs.
- [ ] `safe_route` returns shortest threat-weighted path; avoids high-threat tiles when an alternative exists; cost = `base_cost * (1+threat)` sum; deterministic tie-break.
- [ ] Caravan `safe_route` wrapper avoids enemy tiles and exposed Salt Flats, mildly prefers Ridges (caravan §6.1/§6.7).
- [ ] Render `screen_to_hex` using `from_pixel` + same `cube_round` maps a click to the exact resolver hex (rendering spec §7).

## 9. References

- Design: DD §5.1 (pointy-top hex, axial coords, s invariant).
- Architecture: ARCH §4 (Hex Math — coords, conversions, neighbors/distance/ring/range/line, pathfinding, vendor-vs-implement decision §4.4).
- ADRs: ADR-0005 (axial hex coords, implemented in-core, single auditable cube-round, no external crate).
- Related specs (consumers — names reconciled above):
  - `foundation-core-data-model.md` (`HexCoord`, `tile_index: FxHashMap<HexCoord, TileId>`).
  - `foundation-world-generation.md` (`in_map`, `distance`, `ring`, `range` in step 0/5/7).
  - `foundation-turn-engine.md` (`astar`/`safe_route` invoked by `MoveUnit`/`ConnectRoute` resolution).
  - `gameplay-units-movement.md` (`astar` wrapper, `range` for fog reveal, §6.2 cost table).
  - `gameplay-caravan-routes.md` (`safe_route` wrapper, `THREAT_ENEMY_TILE=2.0`, `THREAT_EXPOSED_FLAT=1.0`, Ridges safe-but-long, `ring(1)` worked ring).
  - `presentation-rendering-ui.md` (`hex::to_pixel`, `hex::from_pixel` / `pixel_to_hex`, `screen_to_hex`, identical cube-round).

## 10. Open Questions (3 resolved)

- **RESOLVED — Return type of `to_pixel` / `from_pixel` / `pixel_to_hex`:**
  Decision: keep `(f32, f32)` (NOT a core-local `Vec2` type). Reason: keeps the
  pure core free of any math/vector crate dependency (ADR-0003). This spec
  returns `(f32, f32)` to keep core free of macroquad; if a core-local `Vec2` is
  desired later, it would be an internal alias only — render still owns
  `Camera2D`/`Vec2`.
- **RESOLVED — `Layout` vs scalar `size`:** Decision: keep `Layout { size, origin }`
  and the `pixel_to_hex` / `to_pixel_layout` convenience wrappers as optional
  conveniences; harmless redundancy. Consumers call the scalar `to_pixel(coord,
  size)` and apply the camera origin themselves; `Layout`/`pixel_to_hex`/
  `to_pixel_layout` are optional conveniences for callers that bundle origin. No
  consumer requires them today.
- **RESOLVED — Direction-table orientation:** Decision: keep current
  orientation; no change. Which of the 6 `AXIAL_DIRS` is index 0 is a fixed
  convention only — it affects deterministic ordering of `neighbors`/`ring`,
  not correctness.
