# Gameplay Spec: Cities

> **Phase:** 2 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::world` / entities)
> **Status:** Draft for review
> **Implements:** DD §7; ARCH §3, §15; ADR-0002 (no ECS), ADR-0004 (Command-only)

---

## 1. Purpose

Specify the city entity: how cities are **founded**, how they **grow**, which tiles
they **work**, the **6-building catalog** and **4 specializations** with concrete
effects, the **production/upgrade queue**, and how cities **feed resources & train
units**. Cities are the anchors of the route network and the only Water/Wealth/Influence
source besides routes. Tied to DD §7.

## 2. Scope

**In scope**

- Founding rules + cost; the DD #4 open question with a recommended default.
- Population growth model and its gating of building slots / unit caps / defense.
- Worked-ring definition and what it yields.
- 6 buildings (Well, Market, Granary, Watchtower, Caravanserai, Temple) — effects.
- 4 specializations (Trade Hub, Well Fort, Fortress, Scholar Outpost) — distinct roles.
- Per-city production/upgrade queue (Build/Train/Specialize ordering).
- How a city supplies resources and trains units.

**Out of scope**

- Route *establish* (caravan spec); combat (combat spec); economy *update order*
  (resources-economy spec — this spec defines building/spec *effects*, the economy
  spec applies them).
- AI founding logic (later spec).

## 3. Responsibilities

- Own the `City` struct and its `buildings`/`specialization`/`population`/`stockpiles`/`route_slots`/`growth_timer`.
- Validate & apply `FoundCity`, `Build`, `Specialize`, `TrainUnit` `Command`s (via the resolver).
- Expose read helpers: `worked_tiles`, `city_water_production`, `city_sight`, etc.

## 4. Core Data Structures / Additions

Reuses `City` from foundation-core-data-model §4.4. Add a **production queue**
field used for multi-turn building/training:

```rust
#[derive(Serialize, Deserialize)]
pub struct City {
    pub id: CityId,
    pub owner: PlayerId,
    pub tile: TileId,                 // invariant: Oasis
    pub population: u32,              // starts 1 (DD §7.2)
    pub specialization: Option<CitySpecialization>,
    pub buildings: Vec<BuildingKind>, // max length = building_slots()
    pub stockpiles: Stockpiles,       // per-city (mostly mirrors owner pool; see §6.5)
    pub route_slots: u8,              // capacity; base 2, raised by Caravanserai/TradeHub
    pub growth_timer: u32,            // turns of surplus elapsed
    pub queue: Vec<QueuedOrder>,     // production/upgrade queue (§6.4)
}

#[derive(Serialize, Deserialize, Clone)]
pub enum QueuedOrder {
    Build(BuildingKind),
    Train(UnitKind),
    Specialize(CitySpecialization),
}
```

### 4.1 Building / specialization effect table (DD §7.4, §7.5)

```rust
pub const BUILD_COST: [i32;6] = [8, 10, 6, 12, 10, 12];      // Well,Market,Granary,Watchtower,Caravanserai,Temple
pub const SPECIALIZE_COST_INFLUENCE: i32 = 10;                  // DD §7.5
pub const FOUND_CITY_INFLUENCE: i32 = 10;                      // DD §7.1
pub const POP_FOR_SPECIALIZE: u32 = 3;                         // DD §7.5

// Building effects (combined with economy spec §6.2):
//   Well        -> +2 Water on this oasis (tile yield)
//   Market      -> +2 Wealth from THIS city's routes; TradeHub discount applies
//   Granary     -> +5 Water stockpile cap (empire cap helper, economy spec §4)
//   Watchtower  -> reveal fog radius 2 around city; +1 def to adjacent tiles
//   Caravanserai-> route_upkeep_from_this_city -1; +1 route_slots
//   Temple      -> +1 Influence/turn

// Specialization signature bonuses (mutually exclusive; one per city):
//   TradeHub     -> +50% Wealth from its routes; +1 route_slots; Market cost -4
//   WellFort     -> +3 Water/turn; cannot starve below Pop 1; supplies Water to connected cities (routes)
//   Fortress     -> +3 def to city tile + neighbors; Train cost/speed -25%; projects Zone of Control
//   ScholarOutpost-> +2 Influence/turn; +1 relic-hold progress; reveals more fog (sight +1)
```

### 4.2 Content Extensibility / Data-Driven Requirement

**Planning-phase directive (hard implementation requirement):** the user has
decided that the set of **buildings and city specializations may change later**.
The concrete **6-building catalog** (Well, Market, Granary, Watchtower,
Caravanserai, Temple) and **4 specializations** (Trade Hub, Well Fort, Fortress,
Scholar Outpost) are **CONTENT DATA, not engine/simulation logic**, and must be
implemented accordingly.

- Each entry's definition — name, build/specialize cost, and effects — lives in
  **externalized catalogs/tables keyed by id/enum** (e.g. `BUILDING`,
  `SPECIALIZATION` const tables indexed by `BuildingKind` /
  `CitySpecialization`) that the engine *reads*. Adding, removing, or re-tuning
  any entry must **NOT** require editing engine or simulation code.
- Effects must be expressed as **data the engine interprets generically** (in the
  spirit of the `TerrainDef` / `UnitDef` balance-data-table concept in
  `foundation-core-data-model.md` §4.12). The simulation iterates over catalog
  entries generically (look up the entry for a given id, apply its declared
  effects) rather than branching on each `BuildingKind`/`CitySpecialization`
  variant with hardcoded behavior. The content set is therefore freely editable.
- For consistency, the same data-driven principle applies by reference to
  **unit types** (`UnitKind`/`UnitDef`) and **tile/terrain types**
  (`TerrainType`/`TerrainDef`): their definitions are content data external to
  engine logic, not hardcoded branches.
- This is a direct consequence of the **pure-core** architecture (ADR-0003):
  keeping content out of the engine keeps `dcs-core` stable while the *game
  content* evolves independently. The hard rule is that the engine must never
  hardcode a specific building, specialization, unit, or tile kind.

## 5. Key Functions / API

```rust
/// Validate + apply FoundCity during resolution. Consumes the founding unit.
/// Returns CityFounded / Rejected.
pub fn resolve_found_city(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Validate + apply Build / Specialize during resolution (spends immediately).
pub fn resolve_build(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;
pub fn resolve_specialize(state: &mut GameState, cmd: &Command) -> Vec<GameEvent>;

/// Queue a Build/Train/Specialize for this city's next Income phase.
pub fn enqueue(state: &mut GameState, city: CityId, order: QueuedOrder);

/// Process a city's queue at Income (spends resources as available; stops on shortfall).
pub fn process_queue(state: &mut GameState, city: CityId) -> Vec<GameEvent>;

// read helpers
pub fn worked_tiles(state: &GameState, city: CityId) -> Vec<TileId>; // city tile + ring(1)
pub fn building_slots(city: &City) -> u8 { 2 + (city.population / 2) } // DD §7.2
pub fn city_sight(state: &GameState, city: CityId) -> u32;             // §6.3
pub fn is_founding_unit(kind: UnitKind) -> bool;                        // §6.1 (DD #4)
```

## 6. Algorithms

### 6.1 Founding (DD §7.1 — DD #4 OPEN, recommended default)

**Recommended DEFAULT (carried as open question DD #4):** a **Scout** may found a
city. The `FoundCity` command validates `is_founding_unit(unit.kind)` where
`is_founding_unit(Scout) == true`. A dedicated `Founder` `UnitKind` is **not**
part of the MVP data model; if introduced later, this single function flips to also
accept it — no other code changes. This honors the DD §7.1 *"Scout can found"*
proposal while keeping the open question explicit.

Validation (`resolve_found_city`):
1. `cmd.unit` owned by `current_actor`; `is_founding_unit(unit.kind)` true → else `Rejected(InvalidState)`.
2. `cmd.tile` terrain == `Oasis` (else `Rejected(NotOasis)`).
3. Tile unowned (`owner == None`) and not in enemy territory (else `Rejected(IllegalTarget)`).
4. `player.resources.influence >= FOUND_CITY_INFLUENCE` → spend 10 (else `Rejected(NoResource)`).
5. Create `City { population:1, specialization:None, buildings:[], route_slots:2, growth_timer:0, queue:[] }` on the oasis; set `tile.owner = player` for the city tile + worked ring; reveal fog (§6.3). Founding unit is consumed.
6. Emit `CityFounded`.

### 6.2 Growth & population (DD §7.2)

```text
if city.water_stockpile > GROWTH_WATER_THRESHOLD(5):
    city.growth_timer += 1
    if city.growth_timer >= GROWTH_PERIOD_TURNS(3):
        city.population += 1
        city.growth_timer = 0
else:
    city.growth_timer = 0     // surplus must be *consecutive*
```

Population drives `building_slots` (§5) and the empire unit cap (`2 + total Pop`,
units spec §6.4) and city defense strength (combat spec).

### 6.3 Worked ring & sight (DD §7.3, §12)

- Worked tiles = `ring(city_tile, 1)` (6 neighbors) **plus the city tile**. Their
  base `TERRAIN` yields flow to the owner each Income (economy spec §6.2).
- `city_sight`:
  - base city sight = **2** (range around city tile, DD §12),
  - +1 if `ScholarOutpost` (reveals more fog, DD §7.5),
  - Watchtower building extends reveal to **radius 2 around the Watchtower tile**
    (not the city) — handled in fog spec, but the +1 defense to adjacent tiles is a
    combat effect here.
- Reveal-on-found: founding reveals `range(city_tile, city_sight)` (fog spec).

### 6.4 Production / upgrade queue (DD §7.4–§7.6)

Two valid paths, both through `Command`s:
- **Immediate** (MVP default): a `Build`/`Specialize`/`TrainUnit` `Command` is
  applied at resolution and spends immediately (validated against current resources).
- **Queued** (multi-turn): `enqueue` appends a `QueuedOrder`; `process_queue` runs
  at Income, applying head-to-tail while resources suffice, emitting `Built`/
  `UnitTrained`/`Specialized`, and leaving unaffordable orders in the queue for next
  turn. Queue length is unbounded but gated by `building_slots` for `Build`.

Slot/eligibility:
- `Build` rejected if `city.buildings.len() >= building_slots(city)` (`Rejected(Blocked)`).
- `Specialize` rejected if `population < POP_FOR_SPECIALIZE(3)` or already specialized (`Rejected(InvalidState)`); spends `SPECIALIZE_COST_INFLUENCE` (10).
- MVP: single-tier buildings only (DD §7.6).

### 6.5 Feeding resources & training units (DD §6.2, §9.4)

- **Feeding:** a city's worked-ring + building yields are added to the **owner's**
  empire `Stockpiles` in the Income phase (economy spec §6.1–6.2). The per-city
  `stockpiles` field mirrors the relevant slice for display/debug; the canonical
  pool is `Player.resources`.
- **Training:** `TrainUnit{city,kind}` (or a queued `Train`) spends
  `UNIT_TRAIN_COST[kind]` Wealth (Fortress −25%; Trade Hub Market discount does not
  apply to units). Spawns the `Unit` on the city tile if unoccupied (else on a free
  worked-ring tile); respects the empire unit cap `2 + total Pop`. Emits `UnitTrained`.
  Fortress also grants **faster** training (same cost, −1 turn if a turn-counter is
  added in full scope — MVP applies only the cost discount).

## 7. Edge Cases / Invariants

- **Invariant (city↔oasis):** `tile(state, city.tile).terrain == Oasis` (core-data-model §6).
- **Invariant (ownership):** `city.owner == player`, and `tile.owner == Some(player)` for the city tile + worked ring.
- **Specialization is exclusive & permanent** in MVP (no un-specialize). Captured city **keeps** its specialization but flips `owner`; its routes re-evaluate at next `advance_turn`.
- **Founding consumes the unit** by default — a Scout cannot both found and keep scouting the same turn.
- **Building slot overflow** rejected, never silently dropped.
- **Well Fort starvation floor:** handled in economy spec §6.4 (Pop floored at 1).
- **Defeated owner:** cities are neutralized (owner=None) when population hits 0; the `City` struct may persist as abandoned (ID stable, core-data-model §7).

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `FoundCity` by a **Scout** on an unowned Oasis spends 10 Influence and creates a city; non-Scout rejected (DD #4 default).
- [ ] Found city sets `tile.owner` for city tile + 6 neighbors.
- [ ] Growth: 3 consecutive turns with water > 5 ⇒ +1 population; a sub-threshold turn resets the timer.
- [ ] `building_slots` = 2 + population/2; `Build` beyond slots rejected.
- [ ] `Specialize` requires Pop ≥ 3 + 10 Influence; once set, re-specialize rejected.
- [ ] Well +2 Water, Market +2 route Wealth, Granary +5 Water cap, Temple +1 Influence, Watchtower +1 adjacent def, Caravanserai −1 route upkeep & +1 slot, all applied.
- [ ] Trade Hub +50% route Wealth +1 slot + Market −4; Well Fort +3 Water & no-starve floor; Fortress +3 def + −25% train; Scholar +2 Influence +1 relic +1 sight — all distinct, non-overlapping.
- [ ] `TrainUnit` spends correct Wealth; Fortress discount applies; unit cap `2+totalPop` enforced.
- [ ] Captured city flips owner; routes re-evaluated next `advance_turn`.
- [ ] Queue processes in order, halts on insufficient resources, preserves remaining orders.

## 9. References

- Design: DD §7 (cities) — §7.1 founding, §7.2 growth, §7.3 worked ring, §7.4 buildings, §7.5 specializations, §7.6 upgrade paths; §9.4 training; §12 cities reveal fog.
- Architecture: ARCH §3 (City struct), §15 (world module), §16 (errors).
- ADRs: ADR-0002 (no ECS), ADR-0004 (Command pattern), ADR-0003 (pure core).
- Related specs: `gameplay-resources-economy.md` (applies building/spec yields + caps), `gameplay-caravan-routes.md` (route_slots, WellFort water supply), `gameplay-combat.md` (combat interactions: Fortress defense bonus, city sieges, Well Fort starvation floor), `gameplay-units-movement.md` (TrainUnit, unit cap), `gameplay-fog-of-war.md` (city_sight reveal), `foundation-core-data-model.md`, `foundation-turn-engine.md`.

## 10. Open Questions (carried, not resolved)

- **DD #4 / OQ-2:** Scout-can-found vs dedicated Founder — **recommended DEFAULT:
  Scout can found** (`is_founding_unit(Scout)=true`); data model supports adding a
  `Founder` kind later by flipping that one predicate. Balance call deferred.
- **DD #3 / OQ-1:** building/specialization magnitudes are first-pass; tables.
- **MVP specializations:** DD §17.1 says MVP ships a *generic* city with
  specializations "data-modeled so they slot in" — this spec defines them fully so
  they can be enabled; the MVP feature-flag is an implementation concern.
