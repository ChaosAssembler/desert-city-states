# Foundation Spec: World Generation

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (module `dcs-core::map` / `world`)
> **Status:** Draft for review
> **Implements:** DD §5.2–§5.7; ARCH §10; ADR-0006 (deterministic seeded PRNG)

---

## 1. Purpose

Define the deterministic algorithm that builds a fully initialized `GameState`
ready for turn 1 from a `ScenarioConfig` + `seed`. All placement draws from
`state.rng` so the same `(scenario, seed)` always yields the identical map
(DD §5.4, ADR-0006). Returns a `GameState` whose turn=1, `current_actor`=player 0,
and all players have a fair starting oasis with their **Scout + Guard opening**
(the player founds the capital themselves via `FoundCity`; no pre-placed city).

## 2. Scope

**In scope:** terrain assignment (weight table → tiles), oasis clustering with
min spacing, ridge chains, salt-flat corridors, ruins/relic placement, fair
player start positions, initial `GameState` assembly (tiles, starting
Scout + Guard units and seeded resources per player, starting discovered set,
RNG state, victory tracker zeroed).

**Out of scope:** turn-by-turn economy/upkeep (turn engine); AI play (later
spec); fog *rendering* (render layer); scenario *validation* (scenario-config
spec).

## 3. Responsibilities

- Produce a hexagon-shaped map of `radius = scenario.map_radius` (DD §5.2).
- Honor all DD placement rules (§5.4–§5.7) and min-distance spacing.
- Be **pure**: identical `(scenario, seed)` → identical `GameState`.
- Respect the seeded PRNG exclusively (no `thread_rng`/`std::time`).

## 4. Core Data Structures

```rust
/// Terrain placement uses weighted random selection from this table.
pub struct TerrainWeight { pub terrain: TerrainType, pub weight: u32 }
// default (full-scope) base weights — tuned in balance tables, not logic:
//   Oasis: handled by cluster step (not free-weighted)
//   Dunes: high (filler), SaltFlats: medium (corridors), Ridges: low (chains),
//   Ruins: very low (1-2 per map)

/// Generation context — private to generate(); not stored in GameState.
struct GenCtx<'a> { scenario: &'a ScenarioConfig, rng: &'a mut SeededRng }
```

The output is a `GameState` (see `foundation-core-data-model.md`).

## 5. Key Functions / API

```rust
/// Build a complete GameState for turn 1 from scenario + seed.
/// Pure & deterministic: same inputs -> identical output.
pub fn new_game(scenario: &ScenarioConfig, seed: u64) -> GameState;

/// Internal pipeline steps (all draw from state.rng):
fn generate_tiles(state: &mut GameState);              // allocate all hexes
fn place_oases(state: &mut GameState);                 // clustered, min-spaced
fn place_ridges(state: &mut GameState);                // 1-2 chains
fn carve_salt_flats(state: &mut GameState);            // linear corridors
fn place_ruins_and_relics(state: &mut GameState);      // 1-2 ruins, relic subset
fn place_players(state: &mut GameState);               // fair starts on oases
fn init_players_and_starting_units(state: &mut GameState); // Player + Scout/Guard + seeded stockpiles
fn reveal_start_fog(state: &mut GameState);            // starting sight radius
```

## 6. Algorithm (steps)

All steps draw from `state.rng` (a single `nanorand`/`StdRng` seeded with
`seed`). Order is fixed for determinism.

**Step 0 — allocate tiles.** For every `HexCoord` with `in_map(coord, radius)`
(hex-grid math spec), create a `Tile { id, coord, terrain: Dunes, is_relic_site:
false, owner: None, improvement: None }`. Build `tile_index: FxHashMap<HexCoord,
TileId>`. (Dunes is the default filler; later steps overwrite.)

**Step 1 — oasis clusters.** Target density ~1 oasis per 12–18 tiles (DD §5.4.1).
Count `oasis_target = max(player_count, total_tiles / 15)` (ensure at least one
per player). Repeat until `oasis_target` placed: pick a random in-map coord;
**accept only if** (a) tile is still `Dunes`, (b) no existing oasis within
`distance < 2` (DD §5.5: ≥2-hex gap → never adjacent), (c) not on map border
(`distance(coord, center) <= radius-1` to leave a buffer). Set `terrain = Oasis`.
*Open fallback:* if spacing can't be met after N tries, relax (a) only — note as
invariant risk.

**Step 2 — ridge chains.** Place 1–2 linear chains (DD §5.4.2) as chokepoints:
pick a random start; walk `chain_len` steps choosing a random `AXIAL_DIR` biased
to continue roughly straight; set each stepped tile (if Dunes) to `Ridges`. Stop
at map edge.

**Step 3 — salt-flat corridors.** Carve 1–2 linear corridors of `SaltFlats`
(DD §5.4.3) similarly, but only over Dunes, leaving oases/ridges untouched.
These form fast-but-exposed caravan lanes (DD §8.4).

**Step 4 — ruins & relics.** Choose 1–2 random Dunes tiles → `Ruins`. Of those,
flag `is_relic_site = true` for exactly `scenario.relic_count` of them (DD §5.6,
V3). If `relic_count` exceeds available ruins, create extra ruins to satisfy it.

**Step 5 — player start positions.** Collect all oasis coords. Choose
`player_count` starts such that:
- each is a distinct oasis,
- pairwise `distance >= 4` (DD §5.4.5, fair spacing),
- (optional, `scenario.symmetry`) mirror across map center for 2–4 players (DD §5.7).
Selection: shuffle oasis list with `rng`, then greedily pick while satisfying
the ≥4 spacing; if symmetry on, pick one and mirror its coord. Assign distinct
`PlayerColor` per player.

**Step 6 — init players + starting units.** For each start, create `Player`
(`kind` from `scenario.ai_personalities` or `Human` for player 0). **Do NOT
create a `City`** — the capital is NOT pre-placed. Instead:

- **Spawn STARTING UNITS:** 1 `Scout` + 1 `CaravanGuard` per player, placed on or
  adjacent to the start oasis tile (place on the oasis tile if free, otherwise
  on an adjacent neutral/Dunes tile; choose deterministically via `rng`).
- **Seed STARTING RESOURCE STOCKPILES:** `Influence = FOUND_CITY_INFLUENCE`
  (10) so the player can found the capital on turn 1; `Wealth` seeded (e.g., 10,
  first-pass / DD #3) so the player can train further units soon.
- **Do NOT set `tile.owner` here** — ownership is established by the `FoundCity`
  command when the capital is placed.

> **Note:** The capital is NOT pre-placed. The player founds it via `FoundCity`
> (a Scout may found, consuming the Scout, cost 10 Influence). This realizes the
> core "scout → found" loop. DD #4 (Scout can found) now covers BOTH the capital
> and later expansions.

**Step 7 — reveal starting fog.** Add to `Player::discovered` the `range(start,
sight_radius)` tiles (sight = city sight, DD §12). Enemy units hidden; static
structures revealed permanently once seen (handled at reveal time by render
reading `discovered`).

**Step 8 — finalize.** `turn=1`, `current_actor = PlayerId(0)`, `phase =
Order`, `victory` zeroed (DD §13 trackers empty), `log=[]`. Return `GameState`.

## 7. Edge Cases / Invariants

- **Invariant:** every `City.tile` is an `Oasis` (core-data-model spec).
- **Invariant:** world-gen creates **no** `City`; capitals are founded later by
   players via `FoundCity` on their (distinct, ≥4-apart) start oases from Step 5.
- **Invariant:** `relic_count` relic sites exist (creates extra ruins if needed).
- Oasis count ≥ player_count guaranteed by `oasis_target` floor.
- If `map_radius` is too small for spacing (e.g., radius 4 with 4 players and
  ≥4 spacing may be tight), generator still succeeds but log a `GameEvent::Warn`
  (non-fatal) — see turn engine. Balance tables should prevent this via scenario
  validation.
- Deterministic: no `HashMap` iteration in generation order; all randomness via
  `state.rng`; `tile_index` is `FxHashMap` (fixed order) per ADR-0006.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `new_game(s, seed)` is deterministic: `new_game(s,seed) == new_game(s,seed)` (deep equal).
- [ ] Different `seed` with same `scenario` yields a *different* but valid map.
- [ ] Every oasis has no oasis neighbor (distance ≥ 2).
- [ ] No `City` exists at world-gen; exactly `player_count` start oases, pairwise
   distance ≥ 4 (capitals are founded later via `FoundCity`).
- [ ] `relic_count` relic sites present; each on a `Ruins` tile.
- [ ] All tiles within `map_radius`; no out-of-map coords in `tile_index`.
- [ ] `Player::discovered` for player 0 includes its start + revealed sight-radius fog.
- [ ] `GameState.turn == 1`, `current_actor == PlayerId(0)`, `phase == Order`.
- [ ] `GameState` round-trips through serde (save-load spec).
- [ ] Ridge/salt-flat corridors never overwrite oases.

## 9. References

- Design: DD §5.2 (map size), §5.3 (tile types), §5.4 (generation approach),
  §5.5 (oasis rules), §5.6 (ruins/relics), §5.7 (symmetry), §12 (fog), §13 (V3).
- Architecture: ARCH §10 (map generation pipeline), §3 (data model).
- ADRs: ADR-0006 (seeded PRNG in state).
- Related specs: `foundation-core-data-model.md`, `foundation-hex-grid-math.md`
  (`in_map`, `distance`, `ring`, `range`), `foundation-scenario-config.md`
   (`ScenarioConfig` fields), `foundation-turn-engine.md` (`GameEvent::Warn`).

## 10. Open Questions (carried)

- **DD #5 / OQ:** Relic-count & hold-duration scaling for small maps is owned by
  `ScenarioConfig` (scenario-config spec); generator just consumes `relic_count`.
- **DD #10:** Single-screen vs zoom is render-only; generation is zoom-agnostic.
- Map symmetry is a *toggle* (`scenario.symmetry`) — generator supports both;
  default left to scenario validation.
