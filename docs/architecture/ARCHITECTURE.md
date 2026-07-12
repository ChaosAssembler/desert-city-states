# Desert City-States — Software Architecture

> **Status:** Draft (Phase 2 of planning — architecture)
> **Version:** 1.0
> **Source of truth for game behavior:** `docs/design/Desert-City-States.md` (the Design Document, "DD")
> **Companion docs:** `docs/planning/` (roadmap/technical specs — future), `docs/architecture/decisions/` (ADRs)
> **Scope of this document:** This is a *planning* artifact. It describes the **intended** structure precisely so a workspace can be scaffolded later. **No Cargo.toml, no `.rs` files, and no scaffolding are produced by this phase.**

---

## 0. Relationship to the Design Document

This document answers *how* the game in the DD is built. Every behavioral rule (tile yields, route upkeep, combat formula, victory thresholds, sequential turns, auto-route-only) is taken as given from the DD and mapped to a module in §15. Where the DD leaves a number or rule as a *proposal* or *open question*, this document carries that flag forward rather than resolving it (see §14).

Settled decisions honored from prior phases:

- **Engine: macroquad** (2D, immediate-mode). Confirmed in DD §3 and Open Question #8.
- **Sequential turns.** Confirmed in DD §4, §16.1.
- **Caravan auto-route only.** Confirmed in DD §3.3, §8.1–8.2, Open Question #2.
- **Configurable session length** (map size, player count, turn limit; win/relic thresholds scale with them). Confirmed in DD §5.2, §13, §16.2, Open Question #5/#9.
- **Small-scale 4X**, hex grid, 2D graphical presentation.

---

## 1. Architectural Principles

The architecture rests on three hard rules. They are non-negotiable for the reasons stated because they are what make the game *testable, replayable, and engine-swappable*.

### 1.1 Rule A — Pure, deterministic, serializable simulation core

`dcs-core` contains **all** game logic and **zero** rendering or engine dependencies. It must depend on nothing that touches the OS windowing, graphics, time-of-day, or input systems.

- **Pure:** given the same `GameState` + same ordered `Command` list + same seed, the resulting `GameState` is bit-for-bit identical. No hidden global state.
- **Deterministic:** all randomness flows from one seeded PRNG owned by `GameState` (§6). No `std::time`, no `rand::thread_rng`, no system entropy anywhere in core.
- **Serializable:** the entire `GameState` round-trips through `serde` so a save file and a deterministic replay are the same operation (§7).

**Why:** a pure core lets us run the *entire* game headlessly in unit/integration tests, fuzz the turn resolver, and reproduce balance bugs from a save. It also decouples balance tuning from rendering work.

### 1.2 Rule B — Rendering/presentation is a separate layer

`dcs-render` (macroquad) **reads** core state and **produces** `Command`s. It never mutates `GameState` directly. Draw calls, sprites, camera, HUD, and input live here only. Core has no knowledge that rendering exists.

### 1.3 Rule C — Thin application/glue layer

`dcs-app` is the only crate that knows about *both* core and render, owns the main loop, dispatches input→commands, steps the simulation, and drives save/load. It holds no game *rules* — only orchestration.

### 1.4 We will NOT use an ECS (Entity-Component-System)

**Decision: no ECS (e.g., `bevy_ecs`, `specs`, `hecs`).**

Rationale:

- This is a **turn-based** game on a **small board** (≤ ~271 tiles, tens of entities). ECS shines for thousands of per-frame entities with spatial queries in real-time loops. Our simulation advances a handful of entities a few times per *turn*, not per *frame*.
- "Plain data + functions" is simpler to reason about and **far easier to make deterministic and serializable**. An ECS's internal archetype storage and allocation order are a determinism/save-compatibility hazard we do not need.
- The natural data shape here is a **single `GameState` aggregate** holding typed collections keyed by stable integer IDs (§3), queried by explicit functions. This is closer to a "data-oriented structs" model than component-graph model.
- If entity count ever explodes (it won't, by DD §5.2), spatial queries are trivial on a hex grid via `HashMap<HexCoord, TileId>` rather than an ECS.

> See also [0002-no-ecs](decisions/0002-no-ecs.md) for the formal ADR capturing this (§1.4).

---

## 2. Workspace & Crate Layout

### 2.1 Proposed Cargo workspace

A single workspace at the repo root with four member crates:

```
desert-city-states/            (workspace root, virtual manifest)
├── Cargo.toml                 (workspace: members + shared profile)
├── crates/
│   ├── dcs-core/              (simulation — pure, deterministic, serde)
│   ├── dcs-render/            (macroquad presentation layer)
│   ├── dcs-app/               (main entry + glue)
│   └── dcs-protocol/          (save-file schema versioning + command/event envelopes)
└── docs/
    ├── design/
    ├── planning/
    └── architecture/
```

### 2.2 Dependency directions (arrows = "depends on")

```
        ┌─────────────┐
        │  dcs-app    │   main(), loop, save/load menu, input dispatch
        └──────┬──────┘
               │ depends on
       ┌───────┴────────┐
       │                │
 ┌─────▼─────┐    ┌─────▼──────┐
 │ dcs-render│    │ dcs-core   │   PURE: no deps on render/app/engine
 └─────┬─────┘    └─────┬──────┘
       │ reads state    │ owns sim
       │                │
       └───────┬────────┘
               │ depends on (for versioned save envelope / shared command types)
        ┌──────▼───────┐
        │ dcs-protocol │  serde schema, VersionedSave, Command/Event enums
        └──────────────┘
```

**Key invariant: the arrow `dcs-core → (render/app)` never exists.** `dcs-core` may depend only on `dcs-protocol` (for save envelope types if kept there) and standard/serde/rand crates — never on macroquad, never on `dcs-app`, never on `dcs-render`.

### 2.3 Why a separate `dcs-protocol` crate?

`dcs-protocol` holds the **serialization contract** that is shared between the pure core (it produces/consumes `GameState` + `Command`s) and the app (it persists them). It contains:

- The `Command` and `GameEvent` enums (the only messages passed across the core/app boundary),
- the `VersionedSave<T>` wrapper (`{ version: u32, payload: T }`) for forward-compatible saves (§7),
- any shared error/result types used by both.

This avoids leaking macroquad or app concerns into core and gives a single, reviewable surface for save compatibility. (We deliberately do **not** extract a separate `dcs-types` crate for entity structs: those are owned by `dcs-core` and `dcs-render` already depends on `dcs-core` to *read* them, so duplicating them in a third crate would add sync burden with no benefit. `dcs-protocol` is strictly the *wire/save* contract.)

### 2.4 Crate responsibilities & public API surface (high level)

#### `dcs-core` — simulation
- **Responsibility:** everything in the game model — hex math, map generation, entities, economy, caravan system, combat, fog of war, AI, turn engine, serialization, seeded RNG.
- **Public API (high level):**
  - `GameState` — the central aggregate (§3).
  - `fn new_game(scenario: &ScenarioConfig, seed: u64) -> GameState` — deterministic world build.
  - `fn step(state: &mut GameState, commands: &[Command]) -> Vec<GameEvent>` — apply one actor's commands, emit events (§5).
  - `fn advance_turn(state: &mut GameState) -> Vec<GameEvent>` — end-of-round bookkeeping (non-economy: relic timers, victory check, turn increment, moves reset — §5).
  - Module groups: `hex`, `map`, `world`/`entities`, `economy`, `caravan`, `combat`, `fog`, `ai`, `turn`, `rng`, `serialize`.
  - Pure AI entry: `fn ai_plan(state: &GameState, player_id: PlayerId, difficulty: Difficulty) -> Vec<Command>`.

#### `dcs-render` — presentation (macroquad)
- **Responsibility:** camera, drawing tiles/units/cities/routes, fog overlay, HUD/UI, translating pointer input into `Command`s. Holds **no** game rules.
- **Public API (high level):**
  - `struct Renderer { camera: Camera2D, ... }`
  - `fn draw_frame(&mut self, state: &GameState, view: &UiView)` — one immediate-mode frame.
  - `fn poll_input(&mut self) -> Vec<Command>` — convert clicks/keys to commands (selection, found, connect-route, end-turn).
  - `fn screen_to_hex(&self, screen: Vec2) -> HexCoord` — inverse mapping for input (§8).

#### `dcs-app` — glue
- **Responsibility:** `main()`, the frame/turn loop, save/load menu, wiring input→`dcs-core`→`dcs-render`. Owns orchestration only.
- **Public API (high level):**
  - `fn main()` — entry.
  - `struct App { state: GameState, renderer: Renderer, scenario: ScenarioConfig }`
  - `fn run(&mut self)` — orchestration loop (§5.7).
  - `fn save(state: &GameState, path: &Path) -> Result<()>`, `fn load(path: &Path) -> Result<GameState>` (delegating to `dcs-protocol` + `dcs-core::serialize`).

#### `dcs-protocol` — save/command contract
- **Responsibility:** versioned save envelope + the `Command`/`GameEvent` enums and shared errors.
- **Public API (high level):**
  - `enum Command { ... }` (§5)
  - `enum GameEvent { ... }`
  - `struct VersionedSave<T> { version: u32, payload: T }`
  - `const SAVE_VERSION: u32`

---

## 3. Core Data Model

### 3.1 Central aggregate: `GameState`

`GameState` is the single root struct. It owns every piece of simulation state and the RNG. It is `serde`'s top-level serializable object.

```rust
struct GameState {
    version: u32,                 // schema version (mirrors protocol)
    scenario: ScenarioConfig,      // map radius, player count, turn limit, thresholds (§11); type defined in foundation-scenario-config §4
    rng: SeededRng,               // ALL randomness (§6) — owned here, serialized
    turn: u32,
    current_actor: PlayerId,      // whose sequential turn it is (§5)
    phase: TurnPhase,             // order / resolution / income / EndOfTurn
    tiles: Vec<Tile>,             // indexed by TileId; layout fixed at gen time
    tile_index: FxHashMap<HexCoord, TileId>,  // coord -> tile lookup; deterministic (fixed-order) map used deliberately for reproducible saves/replays (ADR-0006)
    cities: Vec<City>,
    units: Vec<Unit>,
    routes: Vec<CaravanRoute>,
    players: Vec<Player>,
    relics: Vec<Relic>,           // relic-site hold tracking (§13 V3)
    victory: VictoryTracker,      // live meters for V1/V2/V3
    log: Vec<GameEvent>,          // optional event history for replay/UI
}
```

### 3.2 Identity & IDs

Entities use **stable integer IDs** (`TileId`, `CityId`, `UnitId`, `RouteId`, `PlayerId`, `RelicId` — `u32` newtypes). References between entities are by ID, never by embedding or index-into-vec-ordinal assumptions that could shift. Collections are `Vec<T>` keyed by ID; lookups go through small helpers (`city(state, id) -> &City`). This keeps serialization stable and avoids borrow-checker pain when mutating one entity while reading another.

### 3.3 Entities (plain structs/enums)

```rust
struct Tile {
    id: TileId,
    coord: HexCoord,             // axial (q, r)
    terrain: TerrainType,        // Oasis | Dunes | SaltFlats | Ridges | Ruins
    is_relic_site: bool,         // subset of Ruins
    owner: Option<PlayerId>,     // territory/worked-ring ownership
    improvement: Option<BuildingKind>,
    // Fog/visibility is NOT stored here. The source of truth is
    // Player::discovered: FxHashSet<TileId> (see §12, core-data-model §4.3).
}

struct City {
    id: CityId,
    owner: PlayerId,
    tile: TileId,                // must be an Oasis
    population: u32,
    specialization: Option<CitySpecialization>,
    buildings: Vec<BuildingKind>,
    stockpiles: Stockpiles,      // Water / Wealth / Influence caps + amounts
    route_slots: u8,             // capacity for routes
    growth_timer: u32,
}

struct Unit {
    id: UnitId,
    owner: PlayerId,
    kind: UnitKind,              // Scout | CaravanGuard | Raider
    tile: TileId,
    hp: u32,
    moves_left: u8,
    ability: UnitAbility,        // e.g. None | Patrolling | Garrisoned
}

struct CaravanRoute {
    id: RouteId,
    owner: PlayerId,
    endpoints: (CityId, CityId), // A -> B (player-chosen)
    path: Vec<TileId>,           // auto-computed shortest safe path (§5/§8)
    status: RouteStatus,         // Active | Threatened | Severed
    length: u32,
}

struct Player {
    id: PlayerId,
    kind: PlayerKind,            // Human | Ai { personality, difficulty }
    resources: Stockpiles,
    discovered: FxHashSet<TileId>, // fog reveal — the per-player source of truth (§12)
    defeated: bool,
}

struct Relic {
    id: RelicId,
    tile: TileId,
    holder: Option<PlayerId>,
    consecutive_turns_held: u32, // for V3
}
```

### 3.4 Enums (the catalog types from DD §15)

```rust
enum TerrainType       { Oasis, Dunes, SaltFlats, Ridges, Ruins }
enum UnitKind          { Scout, CaravanGuard, Raider }
enum BuildingKind      { Well, Market, Granary, Watchtower, Caravanserai, Temple }
enum CitySpecialization{ TradeHub, WellFort, Fortress, ScholarOutpost }   // DD §7.5
enum RouteStatus       { Active, Threatened, Severed }
enum VictoryKind       { OasisDominance, WealthScore, RelicHold, TurnLimit }  // DD §13 (TurnLimit: timed-out fallback label)
enum PlayerKind        { Human, Ai(AiPersonality, Difficulty) }
enum AiPersonality     { Expansionist, Raider, Trader, Fortifier }        // DD §11.1
enum Difficulty        { Easy, Normal, Hard }
enum ResourceKind      { Water, Wealth, Influence }
```

Move costs, defense mods, yields, and unit stats are **data tables** (e.g., a `const TERRAIN: &[TerrainDef]`) — not hardcoded at call sites — so balance tuning (DD §18, Open Question #3) is a table edit, not a logic change.

### 3.5 Aggregation approach

`GameState` aggregates the `Vec`s above. Cross-entity relationships are resolved by ID lookups through free functions (`units_on(state, tile_id)`, `routes_through(state, tile_id)`, `city_owner(state, city_id)`). This is the "plain data + functions" model (§1.4) and keeps the core free of an ECS.

---

## 4. Hex Math

### 4.1 Coordinates

Per DD §5.1: **pointy-top hexes**, **axial** coordinates `(q, r)` with the invariant `s = -q - r` (cube coordinate `s` derived). All world storage and pathfinding use axial; conversions are pure functions in `dcs-core::hex`.

```rust
struct HexCoord { q: i32, r: i32 }   // s = -q - r (derive on demand)

// axial <-> cube
fn to_cube(h: HexCoord) -> (i32,i32,i32) { (h.q, h.r, -h.q - h.r) }
fn from_cube(x:i32,y:i32,z:i32) -> HexCoord { HexCoord { q: x, r: z } }

// axial <-> pixel (pointy-top): size = hex "radius" in px
fn to_pixel(h: HexCoord, size: f32) -> (f32, f32) {
    let x = size * (f32::sqrt(3.0) * h.q as f32 + f32::sqrt(3.0)/2.0 * h.r as f32);
    let y = size * (3.0/2.0 * h.r as f32);
    (x, y)
}
fn from_pixel(p:(f32,f32), size:f32) -> HexCoord { /* rounded inverse, Red Blob rounding */ }
```

### 4.2 Neighbors, distance, rings

```rust
const AXIAL_DIRS: [(i32,i32);6] = [ /* the 6 axial direction vectors */ ];
fn neighbors(h: HexCoord) -> [HexCoord;6] { ... }
fn distance(a: HexCoord, b: HexCoord) -> u32 {
    let (aq,ar,as) = to_cube(a); let (bq,br,bs) = to_cube(b);
    ((aq-bq).abs().max((ar-br).abs()).max((as-bs).abs())) as u32
}
fn ring(center: HexCoord, radius: u32) -> Vec<HexCoord> { ... }   // worked ring = ring(_,1)
fn range(center: HexCoord, radius: u32) -> Vec<HexCoord> { ... }  // fog reveal, AoE
fn line(a: HexCoord, b: HexCoord) -> Vec<HexCoord> { /* lerp + cube-round */ }
```

### 4.3 Pathfinding

- **Unit movement / route planning** use **A\*** over the hex graph. Edge cost = tile `move_cost` (DD §5.3), with Ridge penalty and enemy-tile avoidance.
- **Caravan route "shortest safe path"** (DD §8.2) is a **threat-weighted Dijkstra/A\***: minimize path length while adding cost for enemy tiles, exposed Salt Flats, and Ridges (hard but safe), per DD §8.4. The player supplies only endpoints; the engine returns the tile `path` (§3.3).
- Because the graph is tiny (≤271 nodes), A\* with a binary-heap is effectively free; no spatial indexing needed.

### 4.4 Vendor vs. implement

**Recommendation: implement a minimal in-core `hex` module.** Rationale:

- Determinism: a hand-rolled implementation has a single, auditable rounding rule (cube-round). A third-party crate's internal ordering is an external determinism risk for saves/replays.
- Zero-dependency rule (§1.1): keeps `dcs-core` free of extra crates.
- The needed surface (conversions, neighbors, distance, ring/range, line, A\*/Dijkstra) is ~150 lines and stable.
- Crates like `hex2d`/`hexagonal` are viable *if* a future need arises, but for MVP they add a dependency for little gain. **Decision: in-core.** (Captured in [0005-axial-hex-in-core](decisions/0005-axial-hex-in-core.md).)

---

## 5. Turn Loop & Command Architecture

### 5.1 Sequential turns (DD §4, §16.1)

Actors act **one at a time**: `Player 0 (human) → AI 1 → AI 2 → … → end-of-round bookkeeping (`advance_turn`) → next turn`. No simultaneous resolution (DD Open Question #6 notes the resolution-order question; §14 carries it forward).

### 5.2 Command pattern

Players and AI do not mutate `GameState` directly. They emit **`Command`s**; a deterministic resolver in `dcs-core::turn` applies them and returns `GameEvent`s. This is the core's only mutation entry point and what makes replays/saves trivial.

```rust
enum Command {
    MoveUnit    { unit: UnitId, to: TileId },
    FoundCity   { unit: UnitId, tile: TileId },
    TrainUnit   { city: CityId, kind: UnitKind },
    Build       { city: CityId, building: BuildingKind },
    Specialize  { city: CityId, spec: CitySpecialization },
    ConnectRoute{ from: CityId, to: CityId },    // auto-route; endpoints only
    Patrol      { unit: UnitId, tile: TileId },  // station guard on route
    Garrison    { unit: UnitId, city: CityId },
    RaidRoute   { unit: UnitId, route: RouteId },
    RaidCity    { unit: UnitId, city: CityId },
    EndTurn,
}
```

### 5.3 Turn phases (DD §16.1 turn flow)

For each actor, in order:

1. **Order phase** — actor (human via UI, or AI via `ai_plan`) emits `Command`s. Commands are validated & queued; **nothing commits until resolution** (DD §3.3 "no commit until End Turn" → commands accumulate, then resolve).
2. **Resolution phase** — `step(state, &commands)` applies each command deterministically: movement, foundation, training, building, route creation (auto-route computed here), patrol/garrison, raids. Combat auto-resolves (§10) using the seeded RNG. Emits `GameEvent`s.
3. **Income phase** — for the acting player: apply per-turn yields (DD §6), route Wealth/Water transfer (§8.3), upkeep (§8.5, §9.4), population growth (§7.2), starvation/isolation penalties (§6.1, §8.5).
4. Advance `current_actor` to the next player; if all players acted, run `advance_turn` — **global end-of-round bookkeeping only**: relic-hold timers (V3), victory-meter updates (§13), `turn += 1`, and reset `moves_left` for all units, then begin next actor cycle. This is **non-economy bookkeeping**: `advance_turn` must **NOT** re-apply the per-actor economy (yields, route transfer/upkeep, growth, starvation — see phase 3 above); that runs per actor in the Income phase (foundation-turn-engine §6.1/§6.3).

> **Resolution-order open question (DD #6):** when a contested route tile is targeted by raids from different actors within one turn cycle, which raid resolves first must be defined. We propose *first-come in actor order* and flag it for design sign-off (§14).

### 5.4 Input → Command (human)

`dcs-render::poll_input` translates pointer/keyboard into `Command`s against the current `GameState` read-only view (selection, right-click action, route-planning A→B with a live cost preview from `dcs-core::caravan::preview_cost`). Commands accumulate in `dcs-app` until `EndTurn`, then are dispatched to `step`.

### 5.5 AI → Command

`dcs-core::ai::ai_plan(state, player_id, difficulty)` is a **pure function** over an immutable `GameState` view that returns the actor's `Command` list for the turn (§9). It uses the same `Command` enum and the same resolver — AI cheating is prevented by construction.

### 5.6 Order of resolution within `step`

Commands are applied **in the order submitted**; movement before combat before income is enforced by phase separation, not by command interleaving. Validation rejects illegal commands (e.g., move onto blocked tile, found off-oasis) with a `GameEvent::Rejected` rather than panicking.

### 5.7 Orchestration loop (`dcs-app::run`, pseudocode)

```
loop {
    match current actor {
        Human => { commands = render.poll_input() until EndTurn; }
        AI    => { commands = ai_plan(state, actor, diff); }
    }
    events = core::step(&mut state, &commands);   // order+resolution+income for actor
    render.draw_frame(&state, &view);
    if actor == last { core::advance_turn(&mut state); check_victory(); }
}
```

---

## 6. Determinism, RNG & Testing

- **One PRNG, owned by `GameState`.** Recommend `nanorand` (no_std-friendly, small, fast) or `rand::rngs::StdRng` seeded with a `u64`. The RNG state is **serialized as part of `GameState`** so a loaded save continues the exact same sequence.
- **No** `std::time`, `rand::thread_rng`, system entropy, or nondeterministic hashing (use `indexmap`/`FxHasher` with fixed iteration order where maps are iterated) anywhere in `dcs-core`.
- **All** randomness — map gen, combat rolls, AI tie-breaks, ruin rewards — draws from `state.rng`. Thus a `(scenario, seed, command-history)` fully determines a game.
- **Replay = re-issue saved command log** against a re-built `new_game(scenario, seed)`. **Save = serialize `GameState`** (which includes rng). Both rely on the same purity guarantee.
- **Testing:** core is unit-testable headlessly — `cargo test -p dcs-core` runs full games with scripted `Command` sequences and asserts exact end states. Fuzz the resolver with random valid command streams. Integration tests assert map-gen reproducibility for a fixed seed.

---

## 7. Save / Load

- Serialize the **entire `GameState`** (core only) via `serde`. Format: `serde_json` for human-debuggable dev saves, or `bincode`/`postcard` for compact shippable saves — both behind `dcs-core::serialize`.
- **Versioning:** wrap payload in `dcs-protocol::VersionedSave<GameState> { version: u32, payload }`. On load, if `version < SAVE_VERSION`, run migration functions; if `> `, reject. Forward-compat is the reason for the separate `dcs-protocol` crate (§2.3).
- Save file contains only core data — **never** render/app state (camera position, UI state). Those are ephemeral and rebuilt.
- RNG state is inside `GameState`, so a save resumes deterministically (§6).

---

## 8. Rendering Integration (macroquad)

`dcs-render` is the **only** crate aware of macroquad. It draws a frame from a read-only `&GameState` and feeds `Command`s back.

- **Camera:** `Camera2D` with pan (drag) and zoom (wheel); MVP can be single-screen-fit for radius-4 (DD Open Question #10), full scope supports pan/zoom.
- **Coord mapping:** `hex::to_pixel` (§4.1) maps `HexCoord → world`; `Camera2D` maps world → screen; `screen_to_hex` inverts for input.
- **Drawing:**
  - *Tiles* — simple filled polygons/sprites per `TerrainType` with tint cues (DD §3.1): oasis green-blue, dunes tan, salt flats pale grey, ridges dark, ruins marker.
  - *Units* — iconographic tokens (scout/caravan-guard/raider) per DD §3.4.
  - *Cities* — grow visually with population/specialization.
  - *Routes* — polyline through `CaravanRoute.path`; dashed, turning red when `Threatened`/`Severed` (DD §3.1, §8.4).
  - *Fog overlay* — dim/hide tiles outside `Player::discovered` (§12).
- **HUD:** top bar (turn, resources + deltas, victory meters), left panel (selected entity), right panel (minimap + alerts), bottom bar (End Turn, build/train queue) per DD §3.2. Use macroquad's built-in UI for MVP; **egui is a fallback** if HUD complexity grows (note in §14 risk).
- **Input → commands:** clicks/keys → `poll_input` → `Command`s (§5.4).
- **Loop style:** event-driven immediate-mode `draw_frame` each frame; simulation advances only on `EndTurn` (not per frame). No draw calls in core (Rule B).

---

## 9. AI Architecture

- `ai_plan(state, player_id, difficulty) -> Vec<Command>` is a **pure function** over an immutable `GameState` — no mutation, no RNG outside `state.rng`, no cheating (§5.5).
- **Utility-based:** score turn-level goals (expand / defend / raid / build) by a weighted function (DD §11.2). Weights shift by `AiPersonality` (Expansionist/Raider/Trader/Fortifier) and `Difficulty` (Easy/Normal/Hard) (DD §11.1, §11.3).
- **Route awareness is mandatory** (DD §18, Open Question #7): AI utility includes route-security — patrol exposed tiles, reroute/guard on Threatened/Severed. This is what makes the signature mechanic real for opponents.
- **Difficulty tiers:** Easy defends a bit, less reliably (occasional, unreliable route defense — OQ-7 resolved); Normal/Hard are reliable defenders (Normal defends core routes; Hard pre-emptively cuts player's weakest link).
- MVP ships a single Normal-ish personality (expand + basic raid) per DD §17.1; the architecture supports the full matrix without changes.

---

## 10. Map Generation

- **Deterministic from seed** (DD §5.4): `new_game(scenario, seed)` drives all placement through `state.rng` (§6). Same seed + scenario → identical map (shareable for balance testing).
- **Pipeline (`dcs-core::map::generate`):**
  1. Seed oasis clusters at density ~1 per 12–18 tiles (DD §5.4.1); oases ≥2 hex apart (DD §5.5).
  2. Scatter 1–2 Ridge chains as chokepoints (DD §5.4.2).
  3. Fill with Dunes; carve Salt Flats linear corridors (DD §5.4.3).
  4. Place 1–2 Ruins, a subset flagged `is_relic_site` (DD §5.4.4, §5.6).
  5. Place player starts on distinct oases, ≥4 hexes apart (DD §5.4.5) for fairness; optional mirror symmetry for 2–4p (DD §5.7, proposal).
- Terrain weights are **data tables** so distribution is tunable without logic changes.

---

## 11. Config / Scenario Settings

`ScenarioConfig` is a `serde` struct loaded from a TOML/JSON file or defaults, capturing the **configurable session length** decision (DD §5.2, §13, §16.2, Open Questions #5/#9). (Same type as `ScenarioConfig` defined in `docs/specs/foundation-scenario-config.md` §4 — name kept consistent with that authoritative spec.)

```rust
struct ScenarioConfig {
    map_radius: u8,            // MVP 4 (61 tiles); full 7–9 (169–271)
    player_count: u8,          // 2–4
    turn_limit: u32,           // MVP 30; full 60 (default, scalable)
    ai_personalities: Vec<AiPersonality>,
    // victory thresholds derived/scaled from the above:
    oasis_majority_pct: u8,    // V1 (default ≥50%)
    wealth_score_target: u32,  // V2 (default 200)
    relic_count: u8,           // V3
    relic_hold_turns: u32,     // V3 hold duration (scales down for small maps)
    victories_enabled: Vec<VictoryKind>,  // which conditions the turn engine actually checks (DD §13)
    symmetry: bool,            // mirror placement toggle
    seed: u64,                 // shown in menu (DD §5.4)
}

// `mvp_preset()` returns radius=4, player_count=3, turn_limit=30 with
// `victories_enabled = [VictoryKind::OasisDominance]` (V1 only; meters for V2/V3 still built).
```

Thresholds scale with map size & player count per DD §13 (smaller board → fewer relics, shorter hold, shorter limit). Defaults are full-scope starting points, tuned by playtest.

---

## 12. Fog of War (data, not rendering)

Fog is **data in core**, rendered as an overlay in `dcs-render`:

- `Player::discovered: FxHashSet<TileId>` is the source of truth for revealed tiles (per-player on `Player`, not stored per-tile — see core-data-model §4.3).
- Reveal sources: starting city radius, Scout sight (3), city sight, Scholar Outpost, Watchtower (radius 2) (DD §12, §7.4).
- Unexplored tiles yield nothing and block route planning (DD §12).
- Enemy **units** hidden in fog; enemy **cities/routes** permanently revealed once any of their tiles is seen (DD §12).
- Core exposes `is_visible(state, player, tile)`; render uses it. No fog logic in render.

---

## 13. Victory Tracking

`VictoryTracker` in `GameState` maintains live meters for all three conditions (DD §13), even though MVP ships V1 only (DD §17.1) — built forward-compatible:

- **V1 Oasis Dominance:** count owned oases; win at `≥ oasis_majority_pct%` or elimination.
- **V2 Wealth/Prestige:** running prestige score `Wealth×1 + Influence×2 + oases×8 + active_routes×4` vs `wealth_score_target` (DD §13 generalized; setting the oasis/route weights to 0 recovers the literal `Wealth×1 + Influence×2` form).
- **V3 Relic Hold:** `Relic::consecutive_turns_held` vs `relic_hold_turns` for `relic_count` sites.
- Turn limit fallback: highest score wins (tiebreak oases, then score). All thresholds from `ScenarioConfig` (§11).

---

## 14. Extensibility, Risks & Open Questions

### 14.1 Macroquad → Bevy fallback path

We are **committed to macroquad** for MVP/full scope. If scope explodes (e.g., need for retained-mode UI, scene graph, or heavy HUD), the **fallback is Bevy** — but because Rule B isolates all rendering in `dcs-render` and core is engine-agnostic, swapping the renderer means rewriting only `dcs-render` while `dcs-core` and `dcs-protocol` are untouched. This is the payoff of the architecture. **This is captured in [0001-use-macroquad](decisions/0001-use-macroquad.md).**

### 14.2 Architecture risks

1. **Keeping core pure as render needs grow** — temptation to push "just a little" query into core via app state. Mitigation: CI lint / review enforcing `dcs-core` has no macroquad/engine dep (cargo feature gate + `cargo tree` check).
2. **Save versioning** — balance table changes can break old saves. Mitigation: `VersionedSave` + migration functions; document breaking vs. non-breaking.
3. **UI complexity** — HUD/minimap/route-planning UI may outgrow macroquad's built-in UI; **egui fallback** noted.
4. **AI route-competence** (DD Open Question #7) — signature mechanic falls flat if AI ignores routes. Mitigated by baking route-security into utility weights; still a tuning risk.
5. **Resolution-order for contested raids** (DD Open Question #6) — needs a defined rule (proposed: actor order, first-come).

### 14.3 Open questions to surface to the user/team

- **OQ-1 (DD #3):** Balance numbers (isolation −2 Water, network synergy +10%) are first-pass — architecture must keep them as tunable tables (done in §3.4) but values need playtest.
- **OQ-2 (DD #4):** Scout-can-found vs. dedicated Founder — data model already supports either (`FoundCity` command + `UnitKind`); decision is a balance call.
- **OQ-3 (DD #6):** Contested-raid resolution order (§5.3 / §14.2.5).
- **OQ-4 (DD #10):** Single-screen MVP vs. zoom/pan full — both supported by `dcs-render` camera; confirm MVP is single-screen radius-4.
- **OQ-5:** Save format choice — `serde_json` (debuggable) vs `bincode`/`postcard` (compact). Recommend json for dev, bincode for ship; both behind `serialize`.
- **OQ-6:** Confirm `nanorand` vs `rand::StdRng` for the core PRNG (both deterministic & seedable; recommend `nanorand` for minimal footprint).

---

## 15. Mapping: Design Systems → Core Modules

| Design Doc system | DD section | Crate | Module (`dcs-core::…`) | Key types |
|---|---|---|---|---|
| Hex grid / coordinates | §5.1 | dcs-core | `hex` | `HexCoord`, conversions, pathfinding |
| Map & world generation | §5 | dcs-core | `map` | `generate()`, `TerrainType` |
| Resources & economy | §6 | dcs-core | `economy` | `Stockpiles`, yield/upkeep fns |
| Cities & specialization | §7 | dcs-core | `world` / `entities` | `City`, `BuildingKind`, `CitySpecialization` |
| Caravan / trade routes | §8 | dcs-core | `caravan` | `CaravanRoute`, `RouteStatus`, route solver |
| Units | §9 | dcs-core | `world` / `entities` | `Unit`, `UnitKind`, `UnitAbility` |
| Combat & sieges | §10 | dcs-core | `combat` | resolution fn, terrain mods |
| Fog of war | §12 | dcs-core | `fog` | `is_visible`, reveal fns |
| AI opponents | §11 | dcs-core | `ai` | `ai_plan`, `AiPersonality`, `Difficulty` |
| Victory conditions | §13 | dcs-core | `victory` | `VictoryTracker`, `VictoryKind` |
| Turn loop & commands | §4, §16.1 | dcs-core | `turn` | `step`, `advance_turn`, `Command` |
| Seeded RNG / determinism | §5.4 | dcs-core | `rng` | `SeededRng` |
| Scenario / config | §5.2, §13, §16.2 | dcs-core + dcs-protocol | `scenario` | `ScenarioConfig` |
| Save / load | — | dcs-core + dcs-protocol | `serialize` | `VersionedSave` |
| Rendering / camera / HUD | §3 | dcs-render | (crate root) | `Renderer`, `draw_frame`, `poll_input` |
| App glue / loop / menus | — | dcs-app | (crate root) | `App::run`, `save`/`load` |
| Save/command contract | — | dcs-protocol | (crate root) | `Command`, `GameEvent`, `VersionedSave` |

---

## 16. Error Handling, Logging & Tooling

- **`dcs-core`:** returns `Result<_, CoreError>` for fallible ops (invalid command, gen failure); panics **only** for violated invariants (e.g., ID not found — a bug). No user-facing error strings in core.
- **`dcs-app`:** uses `anyhow` for top-level orchestration errors and `thiserror` for typed errors surfaced to UI (save/load failures, bad scenario file).
- **Logging:** `tracing` across all crates (structured, leveled). Core logs only simulation-relevant events (turn boundaries, RNG draws in debug); render logs input/frame stats in debug.
- **Dev tooling:** `cargo test -p dcs-core` (headless sim tests), a `--headless` app flag to run N games to completion (balance/AI smoke tests), `cargo tree` CI gate asserting `dcs-core` has no macroquad/app deps (enforces Rule B/A).

---

## 17. Performance

- Board is small (≤ ~271 tiles, tens of entities) — **not perf-critical**.
- Core: keep allocations modest; reuse buffers in pathfinding; `HashMap`/`FxHashMap` for coord↔tile and ID lookups.
- Render: **batch draw calls** where macroquad allows (group tiles by terrain, draw routes as one polyline per route); avoid per-tile state changes in the hot path.
- No spatial indexing needed at this scale; hex distance/range are O(1)/O(ring) and cheap.

---

## 18. Architecture Decision Records (ADRs)

Eight ADRs (0001–0008) are **already authored** and live in `docs/architecture/decisions/`:

- [0001-use-macroquad](decisions/0001-use-macroquad.md) — macroquad as renderer; Bevy fallback if scope explodes (§14.1).
- [0002-no-ecs](decisions/0002-no-ecs.md) — No ECS; plain data + functions (§1.4).
- [0003-pure-deterministic-serializable-core](decisions/0003-pure-deterministic-serializable-core.md) — Pure, deterministic, serializable core (§1.1).
- [0004-sequential-turns-command-pattern](decisions/0004-sequential-turns-command-pattern.md) — Sequential turns + Command pattern (§5).
- [0005-axial-hex-in-core](decisions/0005-axial-hex-in-core.md) — In-core hex math, no external hex crate (§4.4).
- [0006-deterministic-seeded-prng](decisions/0006-deterministic-seeded-prng.md) — Deterministic seeded RNG owned by `GameState`; no system entropy in core (§6).
- [0007-save-format-serde-version-envelope](decisions/0007-save-format-serde-version-envelope.md) — `VersionedSave` + `dcs-protocol` separation for save compat (§2.3, §7).
- [0008-cargo-workspace-topology](decisions/0008-cargo-workspace-topology.md) — Cargo workspace topology / dependency directions (§2).

---

*End of Architecture Document v1.0. Forward references: `docs/design/Desert-City-States.md`, `docs/architecture/decisions/` (ADRs), `docs/planning/` (technical specs + roadmap — later phases).*
