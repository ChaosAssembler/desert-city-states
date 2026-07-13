# Foundation Spec: Core Data Model

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (pure sim) + `dcs-protocol` (shared contract)
> **Status:** Draft for review
> **Implements:** DD §5.3, §6, §7, §8, §9, §13; ARCH §3; ADR-0002 (no ECS), ADR-0003 (pure core)

---

## 1. Purpose

Defines the single in-memory aggregate `GameState` and every simulation entity
struct/enum that lives inside it. This is the canonical "plain data + functions"
model (ADR-0002) that the rest of the foundation specs (hex math, world gen, turn
engine, scenario, save/load) operate on. Everything here is `serde`-serializable
so that saving a game is identical to replaying it (ADR-0003, ADR-0007).

## 2. Scope

**In scope**
- `GameState` aggregate and its owned fields (map, players, cities, units, routes, relics, turn, RNG state, scenario, victory tracking).
- Entity structs: `Tile`, `City`, `Unit`, `CaravanRoute`, `Player`, `Relic`.
- ID newtypes and the catalog enums (`TerrainType`, `UnitKind`, `CitySpecialization`, `ResourceKind`, `VictoryKind`, …).
- ID-lookup helper functions (read-side accessors).
- Balance data tables (terrain defs, unit stats, yields) as `const` tables.

**Out of scope**
- Rendering, camera, HUD (lives in `dcs-render`).
- The resolver/command loop (see `foundation-turn-engine.md`).
- Map generation algorithm (see `foundation-world-generation.md`).
- Save format/migration (see `foundation-save-load.md`).
- AI planning logic (later spec).

## 3. Responsibilities

- Be the **only** root of mutable simulation state.
- Guarantee **stable integer IDs** (`u32` newtypes) for all entities so cross-references survive serialization and vec reordering.
- Keep all randomness funneled through the owned `SeededRng`.
- Be fully `serde` round-trippable (no `HashMap` with nondeterministic iteration order where iteration matters — use `indexmap`/`FxHashMap`).
- Expose deterministic read helpers so render/AI never reach into raw fields.

## 4. Core Data Structures

### 4.1 ID newtypes

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CityId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelicId(pub u32);
```

IDs are assigned monotonically from per-kind counters in `GameState` (see §6).
They are **never** the vec index; lookups go through helpers. This keeps
serialization order-independent.

### 4.2 `GameState` (the root aggregate)

```rust
#[derive(Serialize, Deserialize)]
pub struct GameState {
    pub version: u32,                 // mirrors protocol SAVE_VERSION
    pub scenario: ScenarioConfig,     // see foundation-scenario-config.md
    pub rng: SeededRng,               // ALL randomness; serialized (ADR-0006)

    pub turn: u32,
    pub current_actor: PlayerId,
    pub phase: TurnPhase,             // Order | Resolution | Income | EndOfTurn

    pub tiles: Vec<Tile>,
    pub tile_index: FxHashMap<HexCoord, TileId>,  // coord -> tile, fixed iter order
    pub cities: Vec<City>,
    pub units: Vec<Unit>,
    pub routes: Vec<CaravanRoute>,
    pub players: Vec<Player>,
    pub relics: Vec<Relic>,

    pub victory: VictoryTracker,
    pub log: Vec<GameEvent>,          // optional replay/UI history

    // monotonic id counters
    next_tile_id: u32,
    next_city_id: u32,
    next_unit_id: u32,
    next_route_id: u32,
    next_player_id: u32,
    next_relic_id: u32,
}
```

### 4.3 `Tile`

```rust
#[derive(Serialize, Deserialize)]
pub struct Tile {
    pub id: TileId,
    pub coord: HexCoord,              // axial (q, r); s = -q - r
    pub terrain: TerrainType,
    pub is_relic_site: bool,          // subset of Ruins (DD §5.6)
    pub owner: Option<PlayerId>,      // territory / worked-ring ownership
    pub improvement: Option<BuildingKind>,  // built on worked tile
}
```

> `Visibility`/fog is stored **per-player** on `Player::discovered` (ARCH §12), not
> on the tile, to keep the tile itself immutable per-coord and serializable cleanly.

### 4.4 `City`

```rust
#[derive(Serialize, Deserialize)]
pub struct City {
    pub id: CityId,
    pub owner: PlayerId,
    pub tile: TileId,                 // must be an Oasis (invariant)
    pub population: u32,              // starts 1 (DD §7.2)
    pub specialization: Option<CitySpecialization>,
    pub buildings: Vec<BuildingKind>,
    pub stockpiles: Stockpiles,       // Water/Wealth/Influence amounts + caps
    pub route_slots: u8,              // route capacity (DD §8.2)
    pub growth_timer: u32,            // turns of surplus elapsed
}
```

### 4.5 `Unit`

```rust
#[derive(Serialize, Deserialize)]
pub struct Unit {
    pub id: UnitId,
    pub owner: PlayerId,
    pub kind: UnitKind,
    pub tile: TileId,
    pub hp: u32,
    pub moves_left: u8,              // resets each turn (advance_turn)
    pub ability: UnitAbility,        // e.g. None | Patrolling | Garrisoned (no payload; Patrol command stores the tile separately)
}
```

`UnitKind` is the catalog enum of unit kinds (see §4.10).

### 4.6 `CaravanRoute`

```rust
#[derive(Serialize, Deserialize)]
pub struct CaravanRoute {
    pub id: RouteId,
    pub owner: PlayerId,
    pub endpoints: (CityId, CityId),  // A -> B, player-chosen (DD §8.1)
    pub path: Vec<TileId>,           // auto-computed shortest safe path
    pub status: RouteStatus,         // Active | Threatened | Severed
    pub length: u32,
    pub upkeep: u8,                  // Water/turn drawn from network pool
    pub consecutive_threatened: u8,  // for 2-turn Sever rule (DD §8.4)
}
```

### 4.7 `Player`

```rust
#[derive(Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub kind: PlayerKind,            // Human | Ai { personality, difficulty }
    pub color: PlayerColor,          // faction accent for render (data only)
    pub resources: Stockpiles,       // empire-wide stockpile (DD §6)
    pub discovered: FxHashSet<TileId>, // fog reveal (ARCH §12)
    pub defeated: bool,
}
```

### 4.8 `Relic`

```rust
#[derive(Serialize, Deserialize)]
pub struct Relic {
    pub id: RelicId,
    pub tile: TileId,
    pub holder: Option<PlayerId>,
    pub consecutive_turns_held: u32, // V3 timer (DD §13)
}
```

### 4.9 `Stockpiles` and shared value types

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Stockpiles {
    pub water: u32,
    pub wealth: u32,
    pub influence: u32,
    // caps are derived from buildings/spec (economy module), not stored per-tick
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceKind { Water, Wealth, Influence }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerColor { Sand, Crimson, Teal, Violet }  // up to 4 players
```

### 4.10 Catalog enums

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainType { Oasis, Dunes, SaltFlats, Ridges, Ruins }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitKind { Scout, CaravanGuard, Raider }     // DD §9.2

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildingKind {                               // DD §7.4
    Well, Market, Granary, Watchtower, Caravanserai, Temple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CitySpecialization {                          // DD §7.5
    TradeHub, WellFort, Fortress, ScholarOutpost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteStatus { Active, Threatened, Severed }   // DD §8.1

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VictoryKind { OasisDominance, WealthScore, RelicHold, TurnLimit }  // DD §13

// `TurnLimit` is a fourth, purely cosmetic-distinction variant. It is emitted by
// `check_victory` when `state.turn >= scenario.turn_limit` and no V1/V2/V3 condition
// is met: the fallback winner is the highest-prestige-score living player, tie-broken
// by oases → score → PlayerId (see behavior-victory-conditions.md §6.5). The mechanical
// winner determination is unchanged — `TurnLimit` only lets the UI label a timed-out
// conclusion distinctly ("Time's up — X wins on points") versus a true `WealthScore`
// victory.
//
// ⚠️ Serialization impact: adding an enum variant is a non-additive change to the
// serialized form (postcard is not self-describing). Any implementation that adds this
// variant MUST bump `SAVE_VERSION` (mirrored by `GameState.version`) and provide a
// migration in foundation-save-load.md — or accept that pre-`TurnLimit` saves become
// incompatible.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerKind {
    Human,
    Ai { personality: AiPersonality, difficulty: Difficulty },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiPersonality { Expansionist, Raider, Trader, Fortifier }  // DD §11.1

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty { Easy, Normal, Hard }             // DD §11.3

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitAbility { None, Patrolling, Garrisoned }  // DD §9.3 (extensible)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase { Order, Resolution, Income, EndOfTurn }
```

### 4.11 `VictoryTracker`

```rust
#[derive(Serialize, Deserialize, Default)]
pub struct VictoryTracker {
    pub oases_controlled: FxHashMap<PlayerId, u32>,   // V1 live count
    pub prestige_score: FxHashMap<PlayerId, u32>,     // V2 running Wealth×1 + Influence×2 + oases×8 + active_routes×4
    pub relic_timers: FxHashMap<RelicId, PlayerId>,   // V3 current holders
}
```

### 4.12 Balance data tables (DD §18 OQ-1 — tunable, not logic)

```rust
pub struct TerrainDef {
    pub move_cost: u8,
    pub defense_mod: i8,        // Dunes 0, SaltFlats -1, Ridges +2, ...
    pub water: u8, pub wealth: u8,   // base yields (Oasis 3/1, SaltFlats 0/1, ...)
}

pub const TERRAIN: &[TerrainDef; 5] = &[ /* indexed by TerrainType */ ];

pub struct UnitDef { pub moves: u8, pub atk: u8, pub def: u8, pub hp: u8,
                     pub upkeep: u8, pub sight: u8 }   // DD §9.2
pub const UNITS: &[UnitDef; 3] = &[ /* Scout, CaravanGuard, Raider */ ];
```

All balance numbers (DD §18 Open Question #3) live here so tuning is a table
edit, never a logic change. Building, specialization, unit, and tile/terrain
definitions are likewise **content data external to engine logic** — the engine
must iterate over these tables generically, never branching on a specific kind
(see `gameplay-cities.md` §4.2).

## 5. Key Functions / API

```rust
// id allocation (called by world-gen / turn resolver only)
fn alloc_tile_id(state: &mut GameState) -> TileId;
fn alloc_city_id(state: &mut GameState) -> CityId;
fn alloc_unit_id(state: &mut GameState) -> UnitId;
// ... route/player/relic analogues

// read helpers (no mutation; used by render/AI)
fn tile_at(state: &GameState, coord: HexCoord) -> Option<&Tile>;
fn city(state: &GameState, id: CityId) -> &City;        // panic (bug) if missing
fn unit(state: &GameState, id: UnitId) -> &Unit;
fn units_on(state: &GameState, tile: TileId) -> impl Iterator<Item = &Unit>;
fn routes_through(state: &GameState, tile: TileId) -> impl Iterator<Item = &CaravanRoute>;
fn cities_of(state: &GameState, player: PlayerId) -> impl Iterator<Item = &City>;
fn city_owner(state: &GameState, city: CityId) -> PlayerId;

// invariants
fn assert_city_on_oasis(c: &City, state: &GameState) -> bool;  // tile.terrain == Oasis
```

Behavior: allocators bump the matching `next_*_id` counter. Read helpers are
free functions returning references; missing-ID lookups are **invariant
violations** (panic in debug, bug to fix) — never a user-facing error (ARCH §16).

## 6. Algorithms / Invariants

- **ID strategy:** per-kind monotonic `u32` counters. No reuse after removal
  (entities are rarely deleted; defeat marks `Player::defeated` rather than
  removing). Routes/cities may be removed on capture → IDs stay reserved to keep
  references valid.
- **Invariant (city↔oasis):** `tile(state, city.tile).terrain == TerrainType::Oasis`.
- **Invariant (ownership):** `city.owner == player`, and `tile.owner` is set for
  the city's worked ring.
- **RNG ownership:** every random draw routes through `state.rng`; no other RNG
  exists in `dcs-core` (ADR-0006).

## 7. Edge Cases / Invariants (continuity)

- Defeated players remain in `players` (IDs stable); their entities may persist
  as neutral/abandoned but are not controlled.
- `Option<PlayerId>` on `Tile::owner` means neutral territory (unworked ring).
- Captured city flips `owner`; its routes are re-evaluated at next `advance_turn`.
- `PlayerColor` must be unique per player; world-gen assigns distinct colors.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `GameState` derives `Serialize`/`Deserialize`; round-trips via both json & postcard.
- [ ] Serializing then deserializing yields byte-identical iter-order for `tile_index`/`VictoryTracker` maps (determinism).
- [ ] ID allocators never collide; `next_*_id` survives serialization.
- [ ] `city_on_oasis` invariant holds for all cities after `new_game`.
- [ ] `tile_at` returns `None` for coords outside the map radius.
- [ ] `units_on`/`routes_through` return only entities referencing the given tile.
- [ ] Balance tables cover every `TerrainType`/`UnitKind` variant exactly once.
- [ ] `SeededRng` state is a field of `GameState` and round-trips identically.

## 9. References

- Design: DD §5.3 (tiles), §6 (economy), §7 (cities), §8 (routes), §9 (units), §13 (victory), §15 (catalogs), §18 (open questions).
- Architecture: ARCH §3 (core data model), §11 (scenario), §13 (victory), §16 (errors).
- ADRs: ADR-0002 (no ECS), ADR-0003 (pure core), ADR-0006 (RNG in state).
- Related specs: `foundation-hex-grid-math.md`, `foundation-world-generation.md`, `foundation-turn-engine.md`, `foundation-scenario-config.md`, `foundation-save-load.md`.

## 10. Open Questions (carried, not resolved)

- **DD #4 / OQ-2:** Scout-can-found vs dedicated Founder — data model already supports either via `FoundCity` + `UnitKind`; balance decision deferred.
- **DD #3 / OQ-1:** Exact isolation penalty (−2 Water) and network synergy (+10%) values are first-pass; kept in tables.
- **DD #10 / OQ-4:** Render screen/zoom is a render concern; data model unaffected.
