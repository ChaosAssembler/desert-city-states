# Gameplay Spec: Resources & Economy

> **Phase:** 2 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::economy`)
> **Status:** Draft for review
> **Implements:** DD §6; ARCH §3, §5, §16; ADR-0004 (Command-only mutation)

---

## 1. Purpose

Define the empire-wide resource ledger and the **per-turn economy update** that runs
inside the turn engine's upkeep/income phase. This is the economic spine that makes
"routes > oases" mechanical: Wealth scales with your **active route network**, Water
is the survival constraint (and starves disconnected cities), and Influence gates
expansion. All stockpile mutation happens through `Command` resolution + the
`advance_turn` upkeep — never directly by render/AI.

## 2. Scope

**In scope**

- Three stockpiles: **Water**, **Wealth**, **Influence** (per `Player.resources`, DD §6).
- Per-turn **production** from cities, worked-ring tiles, specializations, and routes.
- Per-turn **sinks**: unit upkeep, training costs, route establish + upkeep, building costs, founding cost, specialization cost.
- **Stockpile caps** and overflow loss.
- **Isolation penalty** (−2 Water/turn for a city with zero active routes — DD §8.5).
- **Starvation** (Water → 0 with negative net flow ⇒ pop loss ⇒ abandonment ⇒ elimination).
- The exact **economy update order** within `advance_turn`'s income/upkeep phase.

**Out of scope**

- Route *establish* path computation (see `gameplay-caravan-routes.md`).
- City *workings* / building effects catalog detail (see `gameplay-cities.md`).
- Combat (see `gameplay-combat.md`); Ruin one-time rewards (world-gen / turn-engine).
- AI decision-making (later spec) — it merely emits the `Command`s consumed here.

## 3. Responsibilities

- Compute and apply the per-turn flow for every living `Player`.
- Enforce caps; drop overflow (no hoarding, DD §6.1).
- Apply the isolation penalty and starvation/elimination transitions.
- Keep **all randomness** on `state.rng` (none is required here except ruin rewards, which live in the resolver).

## 4. Core Data Structures / Additions

No new entity types. The economy reads/writes `Player.resources: Stockpiles`
(foundation-core-data-model §4.9) and `City.stockpiles`. Caps are **derived**,
not stored per-tick:

```rust
/// Derived cap for a player's empire-wide stockpile.
/// Base caps from DD §6.1; raised only by buildings (Granary) / scenario.
pub const CAP_BASE: Stockpiles = Stockpiles { water: 30, wealth: 50, influence: 30 };

/// Per-resource, per-turn production/flow accumulator (internal to the update).
struct Flow { water: i32, wealth: i32, influence: i32 }
```

Derived cap (called by the update before clamping):

```rust
fn empire_cap(state: &GameState, player: PlayerId) -> Stockpiles {
    let mut cap = CAP_BASE;
    for city in cities_of(state, player) {
        if city.buildings.contains(&BuildingKind::Granary) { cap.water += 5; } // DD §7.4
        // (Wealth/Influence caps are base-only in MVP; extensible here.)
    }
    cap
}
```

### 4.1 Balance constants (tunable tables — DD §18 OQ-1)

```rust
pub const ISOLATION_PENALTY_WATER: i32 = -2;     // DD §8.5
pub const ROUTE_UPKEEP_WATER: i32      = -1;      // DD §8.5
pub const WATER_TRANSFER_PER_ROUTE: i32 = 2;     // §8.3 pipe from WellFort/surplus
pub const FOUND_CITY_INFLUENCE: i32    = 10;     // DD §7.1
pub const SPECIALIZE_INFLUENCE: i32    = 10;     // DD §7.5
pub const UNIT_TRAIN_COST: [i32;3] = [4, 6, 5];   // Scout, CaravanGuard, Raider (Wealth)
pub const UNIT_UPKEEP:      [i32;3] = [0, 1, 1];   // Wealth/turn
pub const BUILDING_COST: [i32;6] = [8, 10, 6, 12, 10, 12]; // Well,Market,Granary,Watchtower,Caravanserai,Temple (Wealth)
pub const GROWTH_WATER_THRESHOLD: u32 = 5;       // DD §7.2
pub const GROWTH_PERIOD_TURNS: u32   = 3;        // DD §7.2
```

## 5. Key Functions / API

```rust
/// Run the full per-turn economy update for ONE actor (called from step's Income
/// phase) and return the net Income event. Pure over state; no RNG draws.
pub fn apply_income(state: &mut GameState, player: PlayerId) -> GameEvent;

/// Thin per-actor economy wrapper invoked during the **Income phase** (turn-engine
/// §6.1/§6.3) — NOT from `advance_turn`, which must NOT re-apply economy (turn-engine
/// §6.3). It does NOT re-run route upkeep, isolation penalty, starvation, or network
/// synergy — those run per-actor via `apply_income` (economy §6.1). This function is
/// kept as a victory-feed / no-op wrapper only. Returns the events it emits.
pub fn apply_global_economy(state: &mut GameState) -> Vec<GameEvent>;

/// Pure helper: wealth produced by the ACTIVE route network of `player` this turn
/// (used by income + victory V2). Encapsulates the route-yield formula so the
/// combat/AI modules can read it without duplicating math.
pub fn network_wealth_yield(state: &GameState, player: PlayerId) -> u32;

/// Is `city` isolated (zero active routes)? Drives the isolation penalty.
pub fn is_city_isolated(state: &GameState, city: CityId) -> bool;
```

Behavior:
- `apply_income` adds city base + worked-ring yields, route Wealth/Water transfer,
  building/specialization Influence, then **clamps to `empire_cap`** (overflow lost).
- Sponsoring costs (Train/Build/Found/Specialize/ConnectRoute) are **spent at
  `resolve_one` time** (resolution phase), not here — see turn-engine §6.2. Only
  *recurring* sinks (route upkeep, isolation, unit upkeep) run in upkeep.

## 6. Algorithms (formulas & update order)

### 6.1 Per-turn economy update order (within the per-actor Income phase)

The turn engine (foundation-turn-engine §6.3) runs the economy in this fixed order
so replays are deterministic:

1. **City base + worked-ring yields** → add to owner `resources`
   (Water from Oasis + Well; Wealth from Salt Flats; Influence 0 here).
2. **Route Wealth/Water transfer** → for each *active* route owned by the actor,
   add `network_wealth_yield` share + Water transfer to the dependent endpoint.
3. **Route upkeep** → subtract `ROUTE_UPKEEP_WATER` (−1 Water) per owned route.
4. **Isolation penalty** → for each owned city with **zero active routes**,
   subtract `ISOLATION_PENALTY_WATER` (−2 Water).
5. **Unit upkeep** → subtract `UNIT_UPKEEP[kind]` (Wealth) per owned unit.
6. **Building/specialization Influence** → Scholar Outpost (+2), Temple (+1) added.
7. **Cap & overflow** → clamp each resource to `empire_cap`; excess is discarded.
8. **Growth** → city with `water stockpile > GROWTH_WATER_THRESHOLD` and
   `growth_timer` reaching `GROWTH_PERIOD_TURNS` ⇒ `population += 1` (DD §7.2).
9. **Starvation** → city with `water == 0` and **negative net flow** ⇒
   `population -= 1`; at `population == 0` ⇒ city neutralized/abandoned (`Starved`).
   Well Fort cannot drop below Pop 1 (DD §7.5).
10. **Elimination** → a player with zero living cities ⇒ `defeated = true`.

> Net-flow sign (steps 4/5): "negative net flow" in step 9 means the city's own
> per-turn Water delta (post-isolation/upkeep) is `< 0`. A city at exactly 0 with
> zero flow is *not* starved that turn (it merely doesn't grow).

### 6.2 Worked-ring yield

A city works its tile + 6 neighbors (`ring(city_tile, 1)`). For each worked tile
`(terrain T)`:

```text
water   += TERRAIN[T].water          // Oasis 3, others 0
wealth  += TERRAIN[T].wealth         // SaltFlats 1, Oasis 1, others 0
```

Plus building bonuses on the city tile itself (see `gameplay-cities.md`):
Well +2 Water, Market +2 Wealth-from-routes, Temple +1 Influence, Scholar +2 Influence.

### 6.3 Route Wealth/Water transfer (formula — see caravan spec §6 for full detail)

For each **active** `Route` owned by the actor:

```text
wealth_route =
    ( ROUTE_WEALTH_BASE(2)
      + trade_endpoints * 1                 // # endpoints that are TradeHub
      + distance_factor                      // min(len-1, MAX)*DIST_BONUS, capped
      + markets * 2 )                       // # endpoints with Market
    * (1.5 if either endpoint is TradeHub else 1.0)   // TradeHub +50%
    * network_synergy(player)               // 1 + 0.10*max(0, C-2)
floor -> u32
```

Water transfer: the endpoint with the **greater** base Water production is the
*source*; the other (*sink*) receives `WATER_TRANSFER_PER_ROUTE` (+2) if it is
**not** a Well Fort / self-sufficient oasis. (Two self-sufficient oases ⇒ 0 transfer
but full Wealth still flows.) `network_synergy` is defined in the caravan spec §6.4.

> Empire Wealth scales with the **network** (number of connected cities `C`), not
> territory — this is the mechanical "routes > oases" rule.

### 6.4 Starvation detail

```text
for city in cities_of(player):
    flow = water_in_this_turn_for(city)   // steps 1-5 net, excluding growth
    if city.water == 0 && flow < 0:
        if city.specialization == WellFort: city.population = max(1, pop-1?) // floor 1; no loss
        else:
            city.population -= 1
            city.water = 0
            if city.population == 0: neutralize(city)   // abandoned, owner=None
```

## 7. Edge Cases / Invariants

- **Caps lose overflow**: adding past cap silently drops the excess (DD §6.1) — no
  error, no carry.
- **Isolation vs. territory**: a city sitting on an oasis *still* eats −2 Water if
  it has zero active routes (DD §8.5 — the core rule). A Severed route counts as
  **not active**, so a city whose only route is Severed is treated as isolated.
- **Redundancy protects the link, not the penalty**: if a city has 2 routes and one
  is Severed, it still has ≥1 active route ⇒ **no isolation penalty** (DD §8.5).
- **Well Fort floor**: never drops below Pop 1 from starvation (DD §7.5).
- **Elimination**: `defeated=true` only after all cities gone; defeated players are
  skipped in actor order but remain in `players` (ID stable, ARCH §3).
- **Determinism**: no RNG in the economy update itself; iteration over
  `cities`/`routes` uses stable `Vec` order (no `HashMap` iteration in the math).

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] A city with zero active routes loses exactly 2 Water/turn (isolation penalty).
- [ ] A city with one active + one Severed route is **not** isolated.
- [ ] Worked-ring yields match `TERRAIN` table (Oasis 3/1, SaltFlats 0/1).
- [ ] Stockpile clamps at `empire_cap`; Granary raises Water cap by exactly 5.
- [ ] Route Wealth uses base 2 + trade/market/distance/synergy + Trade Hub ×1.5.
- [ ] Water transfer delivers +2 only to a non-self-sufficient sink endpoint.
- [ ] Starvation: city at Water 0 with negative flow loses 1 pop; Pop 0 ⇒ neutralized.
- [ ] Well Fort city never drops below Pop 1 from starvation.
- [ ] Player with zero cities becomes `defeated == true`.
- [ ] Same `(scenario, seed, commands)` ⇒ identical end-of-turn stockpiles (determinism).
- [ ] Upkeep sinks (route −1 Water, unit Wealth) applied exactly once per turn.

## 9. References

- Design: DD §6 (resources/economy), §6.1 (caps/starvation), §6.2 (interdependencies), §7.2/§7.4/§7.5 (city yields/buildings/specializations), §8.3/§8.5 (route yield + isolation + upkeep).
- Architecture: ARCH §3 (data model), §5.3 (income phase), §16 (errors).
- ADRs: ADR-0004 (Command-only mutation), ADR-0003 (pure core), ADR-0006 (RNG).
- Related specs: `gameplay-cities.md`, `gameplay-caravan-routes.md`,
  `gameplay-units-movement.md`, `foundation-core-data-model.md`,
  `foundation-turn-engine.md`.

## 10. Open Questions (carried, not resolved)

- **DD #3 / OQ-1:** Exact `ISOLATION_PENALTY_WATER` (−2) and the synergy (+10%)
  are first-pass; kept as tunable constants for playtest.
- **DD #2 (auto-route):** route path is auto-computed; economy only consumes the
  resulting `path.len()` for cost — path choice is a caravan-spec concern.
