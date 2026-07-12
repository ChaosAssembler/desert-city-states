# Gameplay Spec: Caravan & Trade Routes (SIGNATURE SYSTEM)

> **Phase:** 3 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::caravan`)
> **Status:** Draft for review
> **Implements:** DD §8; ARCH §4.3, §5, §15; ADR-0004 (Command-only), ADR-0005 (axial hex in-core)

---

## 1. Purpose

This is the **heart of the game** ("routes > oases", DD §8.6). Specify how routes
are **established** (auto-routed via a threat-weighted Dijkstra), their exact
**yield formulas**, their **state machine** (Active / Threatened / Severed), how a
**Caravan Guard grants tile control**, **terrain-based raid difficulty**, the
**network effects** (redundancy + synergy + isolation penalty), and route
**vulnerability** to Raiders. All mechanics are precise and unit-testable so the
"route control beats oasis control" fantasy is mechanical, not flavor.

## 2. Scope

**In scope**
- `ConnectRoute` resolve (cost + auto-route via `safe_route`).
- `safe_route` threat-weighted Dijkstra over the hex graph.
- `CaravanRoute` `path`, `status`, `upkeep`, `consecutive_threatened`.
- Route yield formulas (Wealth + Water) and network synergy.
- Route state transitions (Active ↔ Threatened ↔ Severed) at `advance_turn`.
- Tile **control** (territory + patrolling Guard) and raid resolution hook.
- Terrain raid difficulty (Ridge safe / Salt Flats exposed).
- Redundancy + isolation penalty application.
- Route diplomacy / tolls — stretch note only.

**Out of scope**
- The actual combat *formula* (combat spec) — this spec defines the **raid contest
  trigger & odds inputs**, combat spec resolves HP.
- Economy *update order* (resources-economy spec applies the yields this spec computes).
- Unit movement (units-movement spec); founding cost (cities spec).

## 3. Responsibilities

- Own route establishment, control computation, yield math, and state recomputation.
- Provide `preview_cost` (render cost preview, DD §3.3) and `is_route_tile_controlled`.
- Never mutate outside the resolver / `advance_turn` (ADR-0004).

## 4. Core Data Structures / Additions

Reuses `CaravanRoute` (core-data-model §4.6):

```rust
#[derive(Serialize, Deserialize)]
pub struct CaravanRoute {
    pub id: RouteId,
    pub owner: PlayerId,
    pub endpoints: (CityId, CityId),  // A -> B, player-chosen (DD §8.1)
    pub path: Vec<TileId>,           // auto-computed shortest safe path (inclusive of both city tiles)
    pub status: RouteStatus,         // Active | Threatened | Severed
    pub length: u32,                // == path.len()
    pub upkeep: u8,                 // Water/turn drawn from network pool (DD §8.5)
    pub consecutive_threatened: u8, // 0/1/2 -> drives 2-turn Sever rule (DD §8.4)
}
```

### 4.1 Balance constants (tunable — DD §18 OQ-1)

```rust
pub const ROUTE_ESTABLISH_BASE_COST: i32 = 5;       // Wealth, DD §8.2
pub const ROUTE_ESTABLISH_PER_TILE: i32 = 1;        // Wealth per path tile, DD §8.2
pub const ROUTE_UPKEEP_WATER: i32        = 1;        // Water/turn, DD §8.5
pub const ROUTE_WEALTH_BASE: i32          = 2;        // per active route, DD §8.3
pub const ROUTE_DIST_BONUS: f32          = 0.25;     // per path tile beyond endpoints
pub const ROUTE_DIST_CAP: u32            = 8;         // distance bonus capped here
pub const TRADE_HUB_BONUS_PER_EP: i32   = 1;        // per TradeHub endpoint
pub const MARKET_BONUS: i32              = 2;         // per Market-endpoint on its routes
pub const TRADE_HUB_WEALTH_MULT: f32     = 1.5;      // +50% if either endpoint TradeHub
pub const NETWORK_SYNERGY_PER_CITY: f32  = 0.10;     // +10% Wealth per extra connected city
pub const WATER_TRANSFER_PER_ROUTE: i32  = 2;        // pipe from WellFort/surplus, §8.3
pub const ISOLATION_PENALTY_WATER: i32  = -2;        // §8.5

// safe_route threat weighting (DD §8.2 / §8.4)
pub const THREAT_ENEMY_TILE: f32   = 2.0;  // add to edge weight if tile enemy-owned/adjacent
pub const THREAT_EXPOSED_FLAT: f32 = 1.0;  // add if Salt Flats & uncontrolled (exposed)
// Ridges: high move_cost (3) but NO threat penalty -> safe-but-long; creates real corridor choice
```

## 5. Key Functions / API

```rust
/// Threat-weighted shortest safe path between two city tiles (DD §8.2).
/// Auto-route ONLY — player supplies endpoints, not tiles (DD #2 auto-route).
/// Returns the inclusive path (cityA tile ... cityB tile). Uses dcs-core::hex Dijkstra.
pub fn safe_route(state: &GameState, from: TileId, to: TileId) -> Vec<TileId>;

/// Cost preview for the UI (DD §3.3 route-planning mode).
pub fn preview_cost(state: &GameState, from: CityId, to: CityId) -> (i32, Vec<TileId>);

/// Resolve ConnectRoute in the resolution phase: validate, compute path, spend, store.
pub fn resolve_connect(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Is route-tile `t` controlled by `route.owner`? (territory ring OR patrolling Guard)
pub fn is_route_tile_controlled(state: &GameState, route: &CaravanRoute, t: TileId) -> bool;

/// Recompute every route's status at advance_turn (Active/Threatened/Severed + isolation).
pub fn recompute_routes(state: &mut GameState) -> Vec<GameEvent>;

/// Wealth produced by ONE active route (the §6.2 core formula), as f32 for synergy.
pub fn route_wealth(state: &GameState, route: &CaravanRoute) -> f32;

/// Network synergy multiplier for `player` (1 + 0.10*max(0, C-2)), C = connected cities.
pub fn network_synergy(state: &GameState, player: PlayerId) -> f32;

/// Count of distinct cities in `player`'s active-route connected component.
pub fn connected_city_count(state: &GameState, player: PlayerId) -> u32;

/// Water source/sink pair for a route's transfer (§6.3).
pub fn water_transfer(state: &GameState, route: &CaravanRoute) -> Option<(CityId, CityId)>;
```

## 6. Algorithms

### 6.1 `safe_route` — threat-weighted Dijkstra (DD §8.2, §8.4)

Edge weight from tile `a` to neighbor `b`:

```
w(a->b) = TERRAIN[b].move_cost                       // Oasis 1, Dunes 2, SaltFlats 1, Ridges 3, Ruins 1
         * (1.0 + threat_penalty(b))

threat_penalty(b) =
    if tile b is enemy-owned OR an enemy unit is on/adjacent:  +THREAT_ENEMY_TILE (2.0)
    else if TERRAIN[b] == SaltFlats AND NOT controlled-by-owner: +THREAT_EXPOSED_FLAT (1.0)
    else 0.0
```

- Run standard Dijkstra (binary heap) from `cityA.tile` to `cityB.tile`. Graph = all
  in-map tiles; tiny (≤271 nodes, ADR-0005). Deterministic tie-break by `TileId`
  to keep replays stable.
- **Output:** inclusive `Vec<TileId>` (both city tiles at the ends). `length = path.len()`.
- **Design consequence:** the path avoids enemy tiles and exposed Salt Flats, mildly
  prefers Ridges (safe, no penalty) over long Dune slogs, and never lets the player
  hand-edit tiles (DD #2 confirmed).

### 6.2 Route Wealth yield (per ACTIVE route, per turn) — DD §8.3

```
trade_endpoints = count of endpoints whose city.specialization == TradeHub           // 0,1,2
markets        = count of endpoints whose city.buildings contains Market             // 0,1,2
dist_factor    = min(route.length - 1, ROUTE_DIST_CAP) as f32 * ROUTE_DIST_BONUS  // capped

base = ROUTE_WEALTH_BASE(2)
     + trade_endpoints * TRADE_HUB_BONUS_PER_EP(1)
     + dist_factor
     + markets * MARKET_BONUS(2)

if either endpoint is TradeHub:  base *= TRADE_HUB_WEALTH_MULT(1.5)   // +50%
base *= network_synergy(owner)                                          // economy of scale
wealth = floor(base)  -> u32
```

> Longer *safe* routes pay more path cost but yield a small distance bonus
> (capped), rewarding defense-in-depth without infinite scaling.

### 6.3 Route Water transfer (DD §8.3) — "routes > oases"

```
(source, sink) = water_transfer(route):
    prodA = city_water_production(endpointA)   // terrain + Well + WellFort(+3)
    prodB = city_water_production(endpointB)
    source = the endpoint with GREATER production
    sink   = the other
    if sink.specialization == WellFort OR sink is self-sufficient (own prod >= need):
        transfer = 0          // surplus/surplus link -> Wealth only
    else:
        transfer = WATER_TRANSFER_PER_ROUTE(2)
```
The `+2` delivered to the dependent (non-WellFort) endpoint **offsets** the `−1`
route upkeep, so a connected dependent city nets **+1 Water/turn** from the route on
top of its own oasis, while an *isolated* city pays the isolation penalty (−2).
This is the mechanical expression of "owning an oasis means nothing if the road dies" (DD §8.6).

### 6.4 Network effects (DD §8.5)

- **Redundancy:** if two cities share **2+ independent routes**, the *link* between
  them survives one route being Severed — the connection is defined by "≥1 active
  route exists between the pair", so the Severed route yields 0 but the other keeps
  the city connected (no isolation penalty for either endpoint).
- **Synergy:** `network_synergy(player) = 1 + NETWORK_SYNERGY_PER_CITY(0.10) *
  max(0, C - 2)`, where `C = connected_city_count(player)` = number of distinct
  cities in the owner's active-route connected component (union-find over active
  routes). A 2-city network (baseline) ⇒ ×1.0; 3 cities ⇒ ×1.10; 4 ⇒ ×1.20;
  rewards a **web**, not spokes. Applied multiplicatively in §6.2.
- **Isolation penalty:** a city with **zero active routes** suffers
  `ISOLATION_PENALTY_WATER` (−2 Water/turn), applied in the economy upkeep
  (resources-economy spec §6.1 step 4). A Severed-but-redundant route means the city
  still has an active alternate ⇒ **not** isolated.

### 6.5 Route state machine (DD §8.4) — recomputed each `advance_turn`

For each route, let `exposed = path tiles not controlled` (see §6.6). A route is
**controlled** if every exposed tile has **no enemy unit on/adjacent**; otherwise it
is contestable.

```
enemy_adjacent_to_exposed =
    exists tile t in path where NOT controlled(t)
        AND (enemy unit on t OR enemy unit adjacent to t)

match route.status:
  Active:
    if enemy_adjacent_to_exposed:
        status = Threatened; consecutive_threatened = 1   // yields HALVED this turn
  Threatened:
    if enemy_adjacent_to_exposed:
        status = Severed;   consecutive_threatened = 2      // yields 0 until repatrolled/rebuilt
    else:
        status = Active;    consecutive_threatened = 0
  Severed:
    if NOT enemy_adjacent_to_exposed AND exists friendly Guard controlling an exposed tile:
        status = Active;    consecutive_threatened = 0      // re-patrolled / reinforced
    else:
        stays Severed                                       // must Patrol or rebuild (ConnectRoute)
```

- **Yield gating:** `Active` ⇒ full yield (§6.2/§6.3). `Threatened` ⇒ Wealth halved
  (Water transfer still applies, but reduced 50% — keeps a sliver of supply).
  `Severed` ⇒ Wealth 0 and Water transfer 0 until restored.
- **Raid trigger:** the Raider's `RaidRoute` command (units spec) sets/cascades this
  state immediately during resolution (combat spec §6.2) by placing an effective
  "enemy-adjacent" marker; `recompute_routes` at `advance_turn` then formalizes
  Threatened→Severed across the full turn cycle and handles auto-clearing when the
  Raider leaves.

### 6.6 Tile control (DD §8.4)

`is_route_tile_controlled(state, route, t)` is **true** when:

1. `t` is within the owner's **territory** — i.e. `t` is the worked ring (city tile
   + `ring(1)`) of any owner city (**or** `t.owner == Some(route.owner)`), **or**
2. a friendly **Caravan Guard** with `UnitAbility::Patrolling` is stationed **on `t`
   or on a tile adjacent to `t`** (DD §8.4, §9.3 "Patrol grants control").

A controlled tile **cannot be raided** (no `enemy_adjacent_to_exposed` for it). A
**Caravan Guard** therefore raises the route's effective threat weight / protection —
this is the "grants tile control along the path" rule. Multiple guards can cover a
long route (one guard covers itself + 6 neighbors).

### 6.7 Terrain-based raid difficulty (DD §8.4)

The Raider-vs-route contest (combat spec §6.3) uses terrain to flip odds:

- **Raider on a Ridge** (rough terrain) raids **better**: `raider_atk_eff *= 1.25`
  (ambush advantage in rough ground).
- **Guard on Salt Flats** is **exposed**: `guard_def_eff` uses `TERRAIN[SaltFlats].defense_mod = -1`, making the raid easier.
- **Guard on a Ridge** gets `defense_mod = +2`, making that tile very hard to sever.
- Corridor choice is thus real: a route over Ridges is safe but long/expensive; a
  route over Salt Flats is fast/cheap but easy to raid (DD §8.4 last para).

### 6.8 Establish flow (`resolve_connect`)

1. Validate: both cities owned by `current_actor`; `route_slots` available on **both**
   endpoints (else `Rejected(Blocked)`); `player.resources.wealth >= cost`.
2. `cost = ROUTE_ESTABLISH_BASE_COST(5) + path.len() * ROUTE_ESTABLISH_PER_TILE(1)`.
3. Spend Wealth; compute `path = safe_route(cityA.tile, cityB.tile)`; store
   `CaravanRoute{status:Active, upkeep:ROUTE_UPKEEP_WATER(1), consecutive_threatened:0}`.
4. Decrement `route_slots` on both endpoints (raised by Caravanserai/TradeHub, cities spec §4.1).
5. Emit `RouteCreated{route, from, to, path}`.

### 6.9 Stretch notes (excluded from MVP, DD §8.7)

- **Tolls / right-of-passage:** a route passing through a rival's claimed tile yields
  that rival Influence (negotiation tension). Out of MVP scope; data model already
  exposes `Tile.owner` and route `path` so this slots in later.

## 7. Edge Cases / Invariants

- **Auto-route only:** `ConnectRoute` accepts endpoints, never a tile list — no manual editing (DD #2).
- **Route slots:** a route consumes one slot on **each** endpoint; both must have capacity.
- **Captured city:** flips owner; its routes re-evaluate at next `recompute_routes` — a route now crossing enemy territory becomes contested/exposed.
- **Redundancy ≠ penalty relief incorrectly:** a city with a Severed route but another active route is **not** isolated (§6.4).
- **Determinism:** `safe_route` tie-break by `TileId`; no RNG in route math (path is
  deterministic given the map + ownership). Raid *contests* may draw RNG (combat spec) but never route *establishment*.
- **Self-loop / same city:** `ConnectRoute` with `from==to` rejected.
- **Both endpoints self-sufficient (both WellFort/oases):** Water transfer = 0, but full Wealth still flows.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `safe_route` returns inclusive path; avoids enemy tiles; prefers safe corridors; deterministic (fixed seed ⇒ identical path).
- [ ] `preview_cost` matches `5 + path.len()*1`.
- [ ] `ConnectRoute` spends correct Wealth, decrements both endpoints' slots, stores `Active` route with `upkeep=1`.
- [ ] Route Wealth = floor((2 + trade + dist + market) × (1.5 if TradeHub) × synergy).
- [ ] Synergy: 2 cities ×1.0, 3 cities ×1.10, 4 cities ×1.20.
- [ ] Water transfer delivers +2 only to a non-self-sufficient sink; both self-sufficient ⇒ 0.
- [ ] Threatened ⇒ Wealth halved; Severed ⇒ 0 Wealth & 0 Water.
- [ ] Active→Threatened on enemy-adjacent exposed tile; Threatened→Severed on 2nd consecutive.
- [ ] Severed→Active only when enemy gone AND a friendly Guard controls an exposed tile.
- [ ] `is_route_tile_controlled` true for territory ring + adjacent patrolling Guard; false for uncontrolled Salt Flats with enemy adjacent.
- [ ] City with ≥1 active route (even if another Severed) is NOT isolated (no −2 Water).
- [ ] Raid on Ridge-favored Raider / Salt-Flats Guard shifts contest odds per §6.7.
- [ ] Captured city's routes re-evaluate (become contested) at next `advance_turn`.

## 9. References

- Design: DD §8 (signature system) — §8.1 what a route is, §8.2 establish/auto-route, §8.3 yields, §8.4 vulnerability/defense/terrain, §8.5 upkeep/network/isolation, §8.6 why routes beat oases, §8.7 diplomacy (stretch).
- Architecture: ARCH §4.3 (threat-weighted Dijkstra / `safe_route`), §5.2/`ConnectRoute`, §15 (caravan module).
- ADRs: ADR-0004 (Command-only), ADR-0005 (axial hex in-core, Dijkstra), ADR-0003 (pure core).
- Related specs: `gameplay-resources-economy.md` (applies yields, isolation, upkeep), `gameplay-cities.md` (route_slots, WellFort water supply, TradeHub/Caravanserai bonuses), `gameplay-units-movement.md` (`RaidRoute`, `Patrol`/control), `gameplay-combat.md` (raid contest resolution, terrain mods), `foundation-core-data-model.md` (`CaravanRoute`), `foundation-turn-engine.md` (`RouteCreated`/`RouteRaided`, `recompute_routes` call site), `foundation-hex-grid-math.md` (`distance`, `ring`, Dijkstra).

## 10. Open Questions (carried, not resolved)

- **DD #3 / OQ-1:** `ROUTE_DIST_BONUS`, `NETWORK_SYNERGY_PER_CITY` (+10%), `ISOLATION_PENALTY_WATER` (−2) and `ROUTE_WEALTH_BASE` are first-pass; tunable tables for playtest.
- **DD #2 (auto-route):** confirmed auto-route only; this spec implements it. No open question remains on path editing.
- **DD #6 / OQ-3:** contested-raid *resolution order* within one turn cycle is handled by the turn engine (first-come in actor order); the raid *contest odds* here feed the combat spec. Carried as design sign-off pending (combat spec §6.3).
