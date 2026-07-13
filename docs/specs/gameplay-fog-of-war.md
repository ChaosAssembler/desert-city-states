# Gameplay Spec: Fog of War

> **Phase:** 2 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::fog`)
> **Status:** Draft for review
> **Implements:** DD §12; ARCH §12; ADR-0003 (pure core), ADR-0004 (Command-only)

---

## 1. Purpose

Specify the **per-player visibility** model: the `discovered` set over tiles, the
**reveal radius** from Scouts / cities / Scholar Outpost / Watchtower, **reveal on
move/found**, and exactly **what each player can and cannot see** (enemy units in fog
hidden — including from the AI; static enemy cities/routes permanently revealed
once any of their tiles is seen). Pure data in core; rendering is a separate overlay
(ARCH §12, render-only concerns noted but not specified here). Tied to DD §12.

## 2. Scope

**In scope**
- `Player::discovered: FxHashSet<TileId>` as the visibility source of truth (core-data-model §4.7).
- Reveal sources & radii (Scout 3, Caravan Guard 1, Raider 2, City 2, Scholar +1, Watchtower 2).
- Reveal-on-move / reveal-on-found / reveal-on-route-create.
- Query helpers: `is_tile_visible`, `is_unit_visible`, `is_city_visible`, `is_route_visible`.
- Serialization of visibility (it's just `Player.discovered`, already serde).
- Interaction contract with rendering (read-only; noted).

**Out of scope**
- Camera / draw calls / fog overlay rendering (render layer — DD §3.1, ARCH §8).
- Which visual tint/alpha to use (render-only).
- AI *decision* use of fog (later spec) — but the *data* it may read is defined here.

## 3. Responsibilities

- Own all fog *data* and reveal logic (called from the resolver on move/found/route).
- Provide deterministic visibility queries consumed by render + AI.
- Keep fog purely a function of `discovered` (no hidden global flags).

## 4. Core Data Structures / Additions

Reuses `Player::discovered: FxHashSet<TileId>` (core-data-model §4.7). Reveal
radii as a data table (DD §12, §9.2, §7.4, §7.5):

```rust
/// Reveal radius (hex range) per source. range(center, r) from hex spec.
pub const SIGHT_SCOUT: u32   = 3;   // DD §9.2 / §12
pub const SIGHT_GUARD: u32   = 1;   // DD §9.2
pub const SIGHT_RAIDER: u32  = 2;   // DD §9.2
pub const SIGHT_CITY_BASE: u32 = 2;  // DD §12
pub const SIGHT_SCHOLAR_BONUS: u32 = 1; // ScholarOutpost reveals more (DD §7.5)
pub const SIGHT_WATCHTOWER: u32  = 2; // around the Watchtower tile (DD §7.4)
pub const SIGHT_START: u32    = SIGHT_CITY_BASE; // initial fog reveal at gen (world-gen §7)
```

## 5. Key Functions / API

```rust
/// Reveal `range(center, r)` into `player.discovered`; returns newly revealed tiles.
pub fn reveal(state: &mut GameState, player: PlayerId, center: TileId, r: u32)
    -> Vec<TileId>;

/// Reveal-on-move: called by resolve_move after a unit stops (units spec §6.1).
pub fn reveal_from_unit(state: &mut GameState, unit: UnitId);

/// Query: is `tile` currently in `player`'s discovered set?
pub fn is_tile_visible(state: &GameState, player: PlayerId, tile: TileId) -> bool;

/// Query: a UNIT is visible only if its CURRENT tile is discovered
/// (enemy units in fog are HIDDEN — including from the AI, DD §12).
pub fn is_unit_visible(state: &GameState, viewer: PlayerId, unit: UnitId) -> bool;

/// Query: a CITY is visible if ANY of its tiles (city tile + worked ring)
/// OR any tile of ANY of its routes' paths is discovered. STATIC => permanent
/// once revealed (DD §12).
pub fn is_city_visible(state: &GameState, viewer: PlayerId, city: CityId) -> bool;

/// Query: a ROUTE is visible if ANY path tile is discovered (static => permanent).
pub fn is_route_visible(state: &GameState, viewer: PlayerId, route: RouteId) -> bool;
```

## 6. Algorithms

### 6.1 Reveal sources & radii

| Source | Radius | Notes |
|---|---|---|
| Scout | `SIGHT_SCOUT` (3) | most reveal; reveal-on-move (DD §12) |
| Caravan Guard | `SIGHT_GUARD` (1) | minimal |
| Raider | `SIGHT_RAIDER` (2) | |
| City (base) | `SIGHT_CITY_BASE` (2) | around city tile; `range(city_tile, 2)` |
| City = Scholar Outpost | + `SIGHT_SCHOLAR_BONUS` (→3) | reveals more fog (DD §7.5) |
| Watchtower building | `SIGHT_WATCHTOWER` (2) | around the **Watchtower tile**, +1 def to adjacent (cities spec) |

### 6.2 Reveal triggers

- **World-gen start (world-gen §7):** each player's start city reveals
  `range(start_tile, SIGHT_START)`.
- **Reveal-on-move (units spec §6.1):** after a unit stops on a tile, call
  `reveal_from_unit` → `reveal(player, stop_tile, sight_of(unit.kind))`. The path
  is *not* auto-revealed tile-by-tile beyond the stop radius (keeps Scouts
  valuable — you reveal where you *look*, DD §12 "Scout reveals the most").
- **Reveal-on-found (cities spec §6.1):** founding a city reveals
  `range(city_tile, city_sight)` (city_sight per cities spec §6.3).
- **Reveal-on-route-create (caravan spec §6.8):** optionally reveal the route's
  path endpoints' cities for the owner (the owner already sees its own cities).
- **Watchtower/Scholar** contribute their radius continuously: recomputed each
  `advance_turn` by re-revealing all owned cities' current sight (handles a city
   that *later* specializes into Scholar Outpost or builds a Watchtower).

### 6.2.1 Review decisions (planning review)

The following two MVP behaviors were **confirmed by the user in the planning
review** and are recorded here so they are not re-litigated during implementation:

- **Reveal-at-stop model (keep ping-at-stop, no path reveal):** the INTENDED MVP
  behavior is that fog reveals a radius around a unit's **STOP tile** (and on
  found / route-create / world-gen start) — **NOT** tile-by-tile along the
  movement path. The path between origin and destination is *not* auto-revealed;
  only the destination's sight radius is pinged. This was confirmed verbatim as
  "keep ping-at-stop, no path reveal." (See also §6.2 reveal-on-move bullet.)
- **No memory marker for enemy UNITS (MVP); cities & routes use a MEMORY MARKER:**
  MVP has **NO memory** of last-known enemy **unit** positions — a unit that leaves
  the viewer's `discovered` set is simply **hidden** with no fading "last seen"
  trace (a deliberate first-pass simplification). **Cities and routes**, by
  contrast, use a **MEMORY MARKER**: once any of their tiles is discovered they
  remain shown at their remembered location, dimmed/stale, with dynamic state
  live-updated only while currently observed (see §6.3).

### 6.3 What each player can / cannot see (DD §12)

- **Unexplored tiles:** yield **nothing** and **block route planning** through them
  (`safe_route`/`preview_cost` may only traverse discovered-or-owned tiles for the
  planning player — actually routes may traverse *unexplored* tiles but cannot be
  planned through tiles the player has never seen? DD §12: "no route planning
  through them" ⇒ a `ConnectRoute` is rejected if any path tile is unexplored for
  the actor, OR the path is forced to avoid them. **Specified default:** `ConnectRoute`
  is rejected (`Rejected(Blocked)`) if `safe_route` would need to traverse a tile
  not in the actor's `discovered` set. This keeps "you must scout before you can
  caravan there" true.)
- **Enemy units:** hidden unless their **current** tile is discovered by the viewer
  (`is_unit_visible` uses current position only). **This applies to the AI too** —
  AI planning (later spec) may only "see" units on tiles in its own `discovered`
  set, preventing AI cheating/omniscence by construction (ADR-0004 purity).
- **Enemy cities/routes (memory marker):** once **any** of their tiles is discovered,
  they become a **MEMORY MARKER** — they **remain displayed** on the player's map at
  their remembered location, but are shown as **memory/stale**. Their dynamic state
  (e.g., route `Active`/`Threatened`/`Severed`; city `population`/`specialization`)
  is **only live-updated while the entity is currently observed** (the entity is on a
  discovered tile). When **not currently observed**, the marker is shown as remembered
  (dimmed/stale, state unknown). This **replaces** the earlier "permanently visible
  with live state" rule: `is_city_visible`/`is_route_visible` still derive from the
  `discovered` set (a seen structure is never fully un-seen), but the *state* shown is
  stale unless currently observed.
- **Ruins / relic sites:** revealed like any tile; their `is_relic_site` flag is
  visible once the tile is discovered (V3 tracking, DD §13).

### 6.4 Determinism & serialization

- `Player::discovered` is an `FxHashSet<TileId>` (fixed iteration order, ADR-0006)
  and is fully serialized as part of `GameState` → saves/replays preserve fog exactly.
- Reveal order does not affect the *set*; `reveal` is idempotent (insert-only).
- No RNG in fog logic itself (reveal is deterministic given unit positions).

## 7. Edge Cases / Invariants

- **Memory marker (not live permanence):** a city/route once seen stays **displayed** as a memory marker even if the viewer's units later leave — `is_city_visible`/`is_route_visible` never "un-see" a previously discovered static structure. The marker persists at its remembered location, but its dynamic state is only refreshed while currently observed; when not observed it is shown dimmed/stale (state unknown). *(This replaces the earlier "permanently visible with live state" rule.)*
- **Unit hide-on-move:** an enemy unit that moves out of your discovered tiles
  becomes hidden again (current-position check).
- **AI parity:** the AI uses the *same* `is_unit_visible`/`is_city_visible`
  queries — no privileged sight. Omniscience is impossible by construction.
- **Watchtower/Scholar re-reveal:** handled at `advance_turn` re-scan so late
  building/specializing reveals correctly.
- **Route planning through fog:** `ConnectRoute` rejected if path crosses an
  unexplored tile for the actor (default per §6.3).
- **Defeated players:** keep their `discovered` (ID-stable); irrelevant once defeated.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `reveal` inserts `range(center, r)` tiles; idempotent on repeat.
- [ ] Scout reveals radius 3, Guard 1, Raider 2, City 2, Scholar-city 3, Watchtower radius 2 around its tile.
- [ ] Reveal-on-move reveals only the stop tile's radius (not the whole path).
- [ ] Found city reveals `range(city_tile, city_sight)`.
- [ ] `is_unit_visible` true only if unit's **current** tile discovered (enemy in fog hidden, incl. from AI).
- [ ] `is_city_visible` / `is_route_visible` true if ANY of their tiles discovered, and stay true thereafter (static permanence).
- [ ] `ConnectRoute` rejected if `safe_route` would cross an unexplored tile for the actor.
- [ ] Unexplored tiles yield nothing (economy skips them) and block planning.
- [ ] `Player::discovered` round-trips through serde; same seed+commands ⇒ identical fog (determinism).
- [ ] AI uses the same visibility queries (no privileged sight).

## 9. References

- Design: DD §12 (fog & exploration) — start reveal, Scout/Scholar reveal most, cities small radius, unexplored no yield/planning, enemy units hidden / static structures permanent.
- Architecture: ARCH §12 (fog is data in core, rendered as overlay; `is_visible` exposed; `Player::discovered`), §8 (render reads only), §3 (data model).
- ADRs: ADR-0003 (pure core — fog is core data), ADR-0004 (Command-only; AI can't cheat sight), ADR-0006 (FxHashSet determinism).
- Related specs: `gameplay-units-movement.md` (reveal-on-move, sight radii), `gameplay-cities.md` (city_sight, Scholar/Watchtower reveal, reveal-on-found), `gameplay-caravan-routes.md` (`ConnectRoute` fog constraint, route visibility), `foundation-core-data-model.md` (`Player.discovered`, `FxHashSet`), `foundation-world-generation.md` (start reveal), `foundation-turn-engine.md` (`Revealed` event, advance_turn re-scan).

## 10. Open Questions (carried, not resolved)

- **Route planning through fog (DD §12):** **specified default = reject `ConnectRoute`
  if the path crosses an unexplored tile.** Alternative (allow traverse but hide
  yield preview) is possible; chosen default keeps "scout before you caravan" clean.
  Flag if design prefers the alternative.
- **DD #10 / OQ-4:** single-screen vs zoom/pan is render-only; fog data is
  zoom-agnostic (render decides how much to show).
- **Memory of past enemy positions:** not modeled (only current unit position
  matters); a "last-seen" memory is a possible full-scope extension, not in MVP.
- **Terrain-based line-of-sight (FUTURE / post-MVP, NOT in MVP):** keep in mind we
  might want logic allowing some terrain types to **block sight** or **decrease
  visibility range** (e.g., mountains/hills occluding, or rough terrain reducing
  the effective reveal radius). This is explicitly **out of scope for MVP**; the
  current model uses flat radial reveal only. Flag if/when design wants it.
