# Gameplay Spec: Units & Movement

> **Phase:** 3 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::world` / entities + `dcs-core::hex`)
> **Status:** Draft for review
> **Implements:** DD §9; ARCH §4.3, §5, §15; ADR-0004 (Command-only), ADR-0005 (hex in-core)

---

## 1. Purpose

Specify the three unit kinds (Scout / Caravan Guard / Raider), their stats, how
`MoveUnit` resolves via **A\*** over passable tiles, **Zone-of-Control** rules,
the unit **actions** (FoundCity, GuardRoute/Patrol, RaidRoute, RaidCity, Attack),
**training cost/time** and **upkeep**, and how units **interact with routes**
(Guard protects, Raider severs). Tied to DD §9.

## 2. Scope

**In scope**
- `UnitKind` stats table (Move/Atk/Def/HP/Upkeep/Sight + abilities).
- `MoveUnit` resolution: A* path, `moves_left` consumption, fog reveal, combat-on-enter, ruin reward.
- Zone of Control (light, Fortress-projected).
- Unit actions: FoundCity, Patrol/GuardRoute, RaidRoute, RaidCity, Attack.
- Training (cost/time, unit cap) and upkeep (disband on non-payment).
- Unit ↔ route interaction (control grant, sever trigger).

**Out of scope**
- The combat *formula* itself (combat spec) — this spec triggers it.
- Route *establish*/yield/state machine (caravan spec).
- Founding cost details (cities spec); AI movement planning (later spec).

## 3. Responsibilities

- Own the `Unit` struct + `UnitAbility` and the stats table.
- Resolve `MoveUnit` (A*), `Patrol`, `Garrison`, `RaidRoute`, `RaidCity`, `FoundCity` (delegates founding to cities spec), `TrainUnit` (delegates to cities spec).
- Expose `passable`, `unit_sight`, `zone_of_control` helpers.

## 4. Core Data Structures / Additions

Reuses `Unit` (core-data-model §4.5) and `UnitAbility` (§4.10). Stats table:

```rust
pub struct UnitDef { pub moves: u8, pub atk: u8, pub def: u8,
                     pub hp: u8, pub upkeep: u8, pub sight: u8 }
// indexed by UnitKind: [Scout, CaravanGuard, Raider]  (DD §9.2)
pub const UNITS: [UnitDef;3] = [
    UnitDef { moves:3, atk:1, def:1, hp:2, upkeep:0, sight:3 }, // Scout
    UnitDef { moves:2, atk:3, def:4, hp:4, upkeep:1, sight:1 }, // CaravanGuard
    UnitDef { moves:3, atk:4, def:2, hp:3, upkeep:1, sight:2 }, // Raider
];

pub const UNIT_TRAIN_COST: [i32;3] = [4, 6, 5];     // Wealth (DD §9.4)
pub const FORTRESS_TRAIN_DISCOUNT: f32 = 0.75;        // -25% (DD §9.4)
pub const UNIT_CAP_BASE: u32 = 2;                      // cap = BASE + total Pop (DD §9.4)
pub const TRADE_HUB_MARKET_DISCOUNT: i32 = 4;         // not for units; cities spec
```

`UnitAbility` (core-data-model §4.10): `None | Patrolling | Garrisoned`.

## 5. Key Functions / API

```rust
/// A* over passable tiles; returns the tile path (excludes start, includes target).
/// Edge cost = TERRAIN[next].move_cost; enemy tiles weighted higher (avoid).
pub fn astar(state: &GameState, from: TileId, to: TileId) -> Option<Vec<TileId>>;

/// Validate + apply MoveUnit: consume moves, move, reveal fog, combat if entering enemy.
pub fn resolve_move(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Station a Guard on/adjacent to a route tile -> grants control (caravan spec §6.6).
pub fn resolve_patrol(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Raider raids an exposed route tile (caravan spec §6.5 / combat spec §6.3).
pub fn resolve_raid_route(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Raider attacks a weak/empty city (combat spec §6.4 siege-lite).
pub fn resolve_raid_city(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Unit sight radius for fog reveal.
pub fn unit_sight(state: &GameState, unit: UnitId) -> u32;

/// Tiles under this player's Zone of Control (Fortress-projected, §6.3).
pub fn zone_of_control(state: &GameState, player: PlayerId) -> FxHashSet<TileId>;
```

### 6.7 Unit roster — fixed at 3 kinds (design decision)

**The unit roster is FIXED at the 3 `UnitKind`s — Scout, CaravanGuard, Raider —
for both the MVP and the current full-scope design. No 4th unit kind is planned.**
Per the user's directive: *"stay with the 3 units we have."*

- **Counters are EMERGENT from stats**, not an explicit rock-paper-scissors rule.
  There is no defined RPS relationship; balance is first-pass (DD #3 / OQ-1).
- **If playtesting shows a unit (e.g. Guard) is too dominant, the response is to
  TUNE STATS, not to add a unit.** Adding a unit later remains possible via the
  data-driven content system (the `UNITS` table is content), but is **not planned**.

## 6. Algorithms

### 6.1 `MoveUnit` resolution (DD §9, ARCH §5.2)

1. Validate: `cmd.unit` owned by `current_actor` (`Rejected(NotYourUnit)`);
   `moves_left > 0` (`Rejected(OutOfMoves)`); `to` in-map & reachable.
2. `path = astar(unit.tile, to)`. If none → `Rejected(Blocked)`.
3. Walk path accumulating `TERRAIN[next].move_cost` until `moves_left` exhausted or
   reaching `to`. If `to` unreachable within `moves_left`, move as far as possible
   along the path (stop tile recorded; remaining `moves_left` deducted).
4. **Combat-on-enter:** if the stop tile (or any entered tile) holds an **enemy
   unit**, resolve combat (combat spec §6) against it; outcome may destroy/retreat
   one side. Movement into an enemy-occupied tile is allowed only as an attack.
5. **Reveal fog:** add `range(stop_tile, unit_sight)` to owner's `discovered`
   (`Revealed` event) — reveal-on-move (DD §12).
6. **Ruin reward:** if the unit *stops* on a `Ruins` tile not yet looted, draw a
   one-time reward from `state.rng` (DD §5.6) — e.g. Wealth/Influence burst or
   free building; mark looted. Emits `Warn`/`Income` as appropriate.
7. Decrement `moves_left`; `unit.tile = stop_tile`; emit `UnitMoved`.

### 6.2 `astar` (DD §9, ARCH §4.3)

- Graph = all in-map tiles. Edge `a→b` cost = `TERRAIN[b].move_cost`
  (Oasis 1, Dunes 2, SaltFlats 1, Ridges 3, Ruins 1).
- **Enemy-tile avoidance:** add a large constant to the cost of stepping onto a tile
  occupied by / owned by an enemy, so A* prefers friendly/neutral detours but *can*
  path through enemies if that's the only route (then combat triggers at step 4).
- Deterministic tie-break by `TileId` (ADR-0005); tiny graph ⇒ cheap.
- **Impassable:** none of the terrains are hard-blocked in MVP; blocking is by
  enemy ZoC only (§6.3). (Ruins enterable, DD §5.6.)

### 6.3 Zone of Control (light — DD §7.5 Fortress, §9.5)

- A **Fortress** city projects ZoC onto its city tile + 6 neighbors. Enemy units
  ending a move inside a Fortress ZoC may be attacked by a stationed Guard
  (`UnitAbility::Garrisoned`) — optional in MVP, data-modeled so it slots in.
- Friendly units inside their own ZoC gain the **Fortress +3 defense** (combat spec
  §6.2) when defending. This is the "Fortress projects zone control" rule (DD §7.5).
- ZoC is **not** a hard move-block in MVP (keeps movement legible); it modifies
  combat, not pathing.

### 6.4 Training & upkeep (DD §9.4)

- `TrainUnit{city,kind}` (or queued, cities spec §6.4): spend
  `UNIT_TRAIN_COST[kind]` Wealth × `FORTRESS_TRAIN_DISCOUNT` if city is Fortress;
  spawn `Unit{ hp: UNITS[kind].hp, moves_left: UNITS[kind].moves, ability:None }`
  on the city tile (or a free worked-ring tile). Emits `UnitTrained`.
- **Unit cap:** empire-wide `cap = UNIT_CAP_BASE(2) + sum(population over living
  cities)`. Training beyond cap → `Rejected(Blocked)`.
- **Upkeep:** each turn (economy spec) subtract `UNITS[kind].upkeep` Wealth; if the
  player cannot pay (Wealth would go negative and no stockpile), the unit is
  **disbanded** (`Rejected`/`Warn` + remove unit). Proposal per DD §9.4: disband.

### 6.5 Unit ↔ route interaction

- **Caravan Guard — `Patrol`:** `resolve_patrol` sets `UnitAbility::Patrolling`
  and the unit must be **on or adjacent to a tile of the target route**. This makes
  that tile `controlled` (caravan spec §6.6) ⇒ raids there are contested/repelled.
  Multiple guards cover a long route (each covers itself + neighbors).
- **Caravan Guard — `Garrison`:** `UnitAbility::Garrisoned` in a city; adds its
  `def` to the city siege defense (combat spec §6.2) and contributes to Fortress ZoC.
- **Raider — `RaidRoute`:** `resolve_raid_route` requires the Raider be **on or
  adjacent to an exposed (uncontrolled) route tile** (caravan spec §6.5). It sets
  the "enemy-adjacent" contest marker → route becomes Threatened, and on a 2nd
  consecutive turn with no controlling Guard → Severed (caravan spec §6.5 / combat
  spec §6.3). Emits `RouteRaided{severed}`.
- **Raider — `RaidCity`:** attacks a weak/empty city (combat spec §6.4); on success
  `population -= 1` (or capture at 0). Emits `CityRaided`.

### 6.6 `unit_sight`

`sight = UNITS[kind].sight` (Scout 3, Guard 1, Raider 2). Used for fog reveal on
move/found (fog spec). City/relic/Watchtower sight come from the cities/fog specs.

## 7. Edge Cases / Invariants

- `moves_left` resets to `UNITS[kind].moves` for all units at `advance_turn` (turn-engine §6.3 step 9).
- A unit cannot move onto an enemy tile without triggering combat (no silent swap).
- `Patrol`/`Garrison` require the unit be a **Caravan Guard** (other kinds → `Rejected(InvalidState)`).
- `RaidRoute` requires a **Raider** on/adjacent to an **exposed** route tile; a controlled tile ⇒ contest, not free Threatened (caravan spec §6.6).
- Disband on unpaid upkeep removes the unit (ID stays reserved, core-data-model §6).
- Determinism: A* tie-break by `TileId`; ruin reward draws from `state.rng` only.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] A* returns shortest move-cost path; avoids enemy tiles when alternative exists; deterministic.
- [ ] `MoveUnit` consumes `moves_left` = sum of entered tiles' move_cost; stops when exhausted.
- [ ] Moving onto an enemy unit triggers combat; outcome via combat spec.
- [ ] Reveal-on-move adds `range(stop, sight)` to owner's `discovered`.
- [ ] Stopping on an unlooted Ruins draws a reward from `state.rng` and marks looted.
- [ ] `Patrol` only valid for Caravan Guard on/adjacent to route tile; sets `Patrolling` + control.
- [ ] `RaidRoute` only valid for Raider on/adjacent to an exposed route tile; cascades Threatened→Severed.
- [ ] Training spends correct Wealth (Fortress −25%); unit cap `2+totalPop` enforced.
- [ ] Upkeep non-payment disbands the unit (Wealth floored at 0).
- [ ] `moves_left` reset at `advance_turn`.
- [ ] Same seed+commands ⇒ identical movement/reveal/ruin outcomes (determinism).

## 9. References

- Design: DD §9 (units) — §9.1 stats framework, §9.2 catalog, §9.3 actions, §9.4 training/upkeep, §9.5 counters.
- Architecture: ARCH §4.3 (A* pathfinding), §5.2 (`MoveUnit` etc.), §15 (world/hex modules).
- ADRs: ADR-0004 (Command-only), ADR-0005 (hex in-core A*), ADR-0003 (pure core, RNG).
- Related specs: `gameplay-combat.md` (combat triggered here), `gameplay-caravan-routes.md` (`RaidRoute`, `Patrol` control, route state), `gameplay-cities.md` (`FoundCity`, `TrainUnit`, Fortress discount/ZoC), `gameplay-fog-of-war.md` (reveal-on-move), `gameplay-resources-economy.md` (upkeep disband), `foundation-core-data-model.md` (`Unit`), `foundation-turn-engine.md` (resolver, moves reset).

## 10. Open Questions (carried, not resolved)

- **DD #4 / OQ-2:** Scout-can-found vs Founder — a Scout performs `FoundCity` (cities spec §6.1 default); a dedicated Founder kind would be added here + cities spec.
- **DD #6 / OQ-3:** contested-raid resolution order across actors is set by the turn engine (first-come in actor order); the raid *trigger* here feeds it. Carried for design sign-off.
- **DD #3 / OQ-1:** unit stats / costs / cap are first-pass tables.
- **ZoC strength:** exact ZoC combat magnitude is a combat-spec detail; this spec only defines *which* tiles are ZoC (Fortress-projected).
