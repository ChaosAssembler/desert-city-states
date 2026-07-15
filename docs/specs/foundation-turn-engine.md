# Foundation Spec: Turn Engine

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (module `dcs-core::turn`) + `dcs-protocol` (`Command`/`GameEvent`)
> **Status:** Draft for review
> **Implements:** DD §4, §16.1; ARCH §5; ADR-0004 (sequential turns + Command pattern)

---

## 1. Purpose

Define the deterministic, sequential turn engine: the `Command` enum (the only
mutation entry point), the `resolve(command, &mut GameState) -> Vec<GameEvent>`
resolver, the per-actor phase machine (Order → Resolution → Income), and the
end-of-turn `advance_turn` bookkeeping (relic timers, victory checks, turn
increment, moves reset). Per-actor economy (production/yields, route transfer &
upkeep, growth, starvation, synergy/isolation) runs in the Income phase (§6.1),
not in `advance_turn`. Tied to ADR-0004: actors act one at a time; all randomness
from `state.rng`; illegal commands rejected, never panic.

## 2. Scope

**In scope:** `Command`/`GameEvent` enums; `step(state, &commands)` resolution;
turn-phase state machine; `advance_turn` upkeep; RNG usage rules; route
status/raid resolution; victory-meter updates; resolution-order rule for
contested raids.

**Out of scope:** map generation (world-gen spec); AI *decision* logic
(`ai_plan` later spec — but it returns `Command`s consumed here); rendering;
save format (save-load spec); combat formula tuning (balance tables).

## 3. Responsibilities

- Be the **single** mutation path for `GameState` (ADR-0004, ADR-0003).
- Enforce sequential actor order and phase transitions.
- Emit `GameEvent`s for render/AI/UI after every applied command.
- Keep all randomness on `state.rng` (combat rolls, ruin rewards, AI tie-breaks).
- Reject illegal commands via `GameEvent::Rejected{..}` (ARCH §16) — never panic
  on player/AI input error.

## 4. Core Data Structures

```rust
// ---- dcs-protocol ----
#[derive(Serialize, Deserialize, Clone)]
pub enum Command {
    MoveUnit     { unit: UnitId, to: TileId },
    FoundCity    { unit: UnitId, tile: TileId },
    TrainUnit    { city: CityId, kind: UnitKind },
    Build        { city: CityId, building: BuildingKind },
    Specialize   { city: CityId, spec: CitySpecialization },
    ConnectRoute { from: CityId, to: CityId },   // auto-route; endpoints only (DD §8.2)
    Patrol       { unit: UnitId, tile: TileId }, // station guard on route
    Garrison     { unit: UnitId, city: CityId },
    RaidRoute    { unit: UnitId, route: RouteId },
    RaidCity     { unit: UnitId, city: CityId },
    EndTurn,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum GameEvent {
    UnitMoved    { unit: UnitId, from: TileId, to: TileId },
    CityFounded  { city: CityId, owner: PlayerId, tile: TileId },
    UnitTrained  { unit: UnitId, city: CityId },
    Built        { city: CityId, building: BuildingKind },
    Specialized  { city: CityId, spec: CitySpecialization },
    RouteCreated { route: RouteId, from: CityId, to: CityId, path: Vec<TileId> },
    RouteStatusChanged { route: RouteId, status: RouteStatus },
    UnitPatrolled{ unit: UnitId, tile: TileId },
    UnitGarrisoned { unit: UnitId, city: CityId },
    RouteRaided  { route: RouteId, by: PlayerId, severed: bool },
    CityRaided   { city: CityId, by: PlayerId, pop_lost: u32 },
    Combat       { attacker: UnitId, defender: UnitId, attacker_loss: u32, defender_loss: u32, retreated: bool },
    Income       { player: PlayerId, water: i32, wealth: i32, influence: i32 },
    Grown        { city: CityId, population: u32 },
    Starved      { city: CityId, population: u32 },
    Revealed     { player: PlayerId, tiles: Vec<TileId> },
    Victory      { kind: VictoryKind, winner: PlayerId },
    TurnAdvanced { turn: u32 },
    Rejected     { command: Command, reason: RejectReason },
    Warn         { message: String },   // non-fatal (e.g. gen spacing)
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum RejectReason { NotYourUnit, OffMap, NotOasis, IllegalTarget,
                        NoResource, Blocked, OutOfMoves, NotYourTurn, InvalidState }
```

## 5. Key Functions / API

```rust
/// Apply one actor's queued commands (Order+Resolution+Income for that actor).
/// Deterministic. Returns events; never panics on bad input (emits Rejected).
pub fn step(state: &mut GameState, commands: &[Command]) -> Vec<GameEvent>;

/// Global end-of-round bookkeeping after the last actor acted. Advances turn,
/// resets moves, updates victory meters, relic timers. Does NOT apply the
/// per-actor economy update (yields, route transfer/upkeep, isolation, synergy,
/// growth, starvation, unit upkeep) — those run per actor in the Income phase.
pub fn advance_turn(state: &mut GameState) -> Vec<GameEvent>;

/// Validate a single command against current state (used by step + render preview).
pub fn validate(state: &GameState, cmd: &Command) -> Result<(), RejectReason>;

/// Resolve one command (internal; called in order by step).
fn resolve_one(state: &mut GameState, cmd: Command) -> Vec<GameEvent>;
```

## 6. Algorithms

### 6.1 Phase state machine (per actor)

For the `current_actor` (DD §16.1 turn flow):
1. **Order** — commands accumulate (human via `poll_input` until `EndTurn`;
   AI via `ai_plan`). *Nothing commits.* (DD §3.3 "no commit until End Turn".)
2. **Resolution** — `step` applies each queued `Command` in submission order via
   `resolve_one`, emitting `GameEvent`s. Movement before combat before income is
   enforced by phase separation, not interleaving (ARCH §5.6).
 3. **Income** — for the acting player (the canonical order from economy spec
    §6.1), apply that actor's **full** per-actor economy update in this order:
    city + worked-ring yields (DD §6), route Wealth/Water transfer (DD §8.3),
    route upkeep (DD §8.5), isolation penalty (DD §8.5), network synergy
    (DD §8.5), population growth (DD §7.2), starvation (DD §7.2), and unit upkeep
    (DD §9.4). All of these economy steps are applied *per actor* here, at the end
    of that actor's turn — they are **not** re-applied globally by `advance_turn`.
 4. Advance `current_actor` to next non-defeated player. If all acted →
    `advance_turn` (§6.3), then begin next actor cycle with `current_actor =
    PlayerId(0)`.

### 6.2 `resolve_one` per command (behavior summary)

- **MoveUnit** — validate owner/moves; compute path via `astar` (hex spec);
  consume `moves_left`; move; reveal fog along path (`Revealed`); if target is
  enemy unit → resolve combat (§6.4); if tile is Ruins and unit stops → one-time
  reward (draw from `state.rng`, DD §5.6).
- **FoundCity** — validate `unit.kind` can found (DD #4 open: Scout for now),
  tile is Oasis & unowned; spend Influence (DD §7.1); create `City`;
  `unit` consumed. Emits `CityFounded`.
- **TrainUnit** — validate city owner, cost (Wealth), unit cap (`2 + total Pop`,
  DD §9.4); spend; spawn `Unit` on city tile; emit `UnitTrained`.
- **Build** — validate city owner, building cost, slot; spend Wealth; push to
  `buildings`; apply effect (e.g., Granary +cap). Emit `Built`.
- **Specialize** — validate `population >= 3` and Influence cost (DD §7.5);
  set `specialization`; apply signature bonuses.
- **ConnectRoute** — validate both cities owned by actor and `route_slots`
  available; compute `safe_route` (hex spec) between the two city tiles; store
  `CaravanRoute{path, status: Active, upkeep}`; decrement slots; emit
  `RouteCreated`. Cost = `5 + path.len()` Wealth (DD §8.2).
- **Patrol / Garrison** — set `UnitAbility::Patrolling`/`Garrisoned`; grants
  route-tile control / city defense (DD §9.3, §8.4).
- **RaidRoute** — Raider on/adjacent to an exposed route tile (DD §8.4). If no
  controlling Guard adjacent → route becomes `Threatened` (yields halved); if
  already `Threatened` and raided again with no defender → `Severed` (yields 0).
  Emit `RouteRaided{severed}`.
- **RaidCity** — Raider attacks weak/empty city (DD §9.3, §10.4): resolve combat
  vs garrison; on success `population -= 1` (or capture if 0). Emit `CityRaided`.
- **EndTurn** — marks the actor's order phase complete; `step` then runs income
  for that actor and returns control to the orchestrator (ARCH §5.7).

### 6.3 `advance_turn` (global end-of-round bookkeeping, once per turn cycle)

> **Timing note — economy is per-actor, not global.** All economy update steps
> (city + worked-ring yields, route Wealth/Water transfer, route upkeep, isolation
> penalty, network synergy, growth, starvation, unit upkeep) are applied during the
> acting player's **Income** phase (§6.1) when they end their turn, using the
> canonical economy §6.1 order. `advance_turn` is **global end-of-round
> bookkeeping only** and must **NOT** re-apply any of those economy steps — doing so
> would double-apply them. The sole purpose of `advance_turn` is relic timers,
> victory checks, the turn increment, and the moves reset.

 1. **Relic timers (V3):** for each relic site, if a player occupies it, increment
    `consecutive_turns_held`; reset on holder change (DD §13).
 2. **Victory check:** update `VictoryTracker` (V1 oasis count, V2 prestige score,
    V3 relic hold). If any threshold met or turn ≥ `turn_limit` ⇒ emit
    `Victory` (or highest-score fallback, DD §13).
 3. `turn += 1`.
 4. Reset `moves_left` for all units of all players; set `phase = Order`;
    `current_actor = PlayerId(0)`.
 5. Emit `TurnAdvanced`.

### 6.4 Combat resolution (auto, DD §10)

```text
attack_power  = atk * atk_pos;                                  // atk = Atk stat; atk_pos encodes attacker tile effect (flank x1.25 on Ridge / Salt-Flats-exposed x0.90); morale = 1.0 (MVP off)
defense_power = def * (1 + TERRAIN[defender_tile].defense_mod); // def = Def stat; defender terrain defense mod (Ridge +2, Salt Flats -1); no terrain_atk term
odds = attack_power / (attack_power + defense_power)
roll = state.rng.next_f32()            // DRAWS FROM state.rng (ADR-0006)
if roll < odds { defender takes HP loss; if hp<=0 retreat/destroy }
else           { attacker takes HP loss; may retreat }
```

Terrain defense mods from `TERRAIN` table (Oasis/Dunes 0, SaltFlats −1, Ridges +2, city
tile + Fortress +3). Attacker tile effect is captured in `atk_pos` (Ridge flank ×1.25, Salt-Flats-exposed ×0.90), NOT a `terrain_atk` defense multiplier. Morale = 1.0 (off for MVP, DD §10.3). **All randomness from `state.rng`.**

### 6.5 RNG usage rules

- Only `state.rng` is read. No `thread_rng`/`std::time` (ADR-0006).
- Each draw advances `state.rng` exactly once; draw order is fixed by command
  resolution order so replays match.
- AI tie-breaks also draw from `state.rng` inside `ai_plan` (pure function).

### 6.6 Contested-raid resolution order (DD #6 / OQ-3)

When a route tile is targeted by raids from different actors within one turn
cycle, the **first raid in actor order wins resolution**; subsequent raids in the
same cycle re-evaluate against the post-raid state. Carried as the proposed rule
(ADR-0004 open question) — flagged for design sign-off, not resolved here.

## 7. Edge Cases / Invariants

- `step` only mutates for `current_actor`; a `Command` owned by another player →
  `Rejected{NotYourTurn}`.
- Illegal commands never partially apply: validation rejects before mutation.
- `advance_turn` runs exactly once per full actor cycle; `EndTurn` from the last
  actor triggers it.
- Captured city flips owner; its routes re-evaluated at next `advance_turn`.
- Defeated players are skipped in actor order but remain in `players` (ID stable).
- Victory emit stops the loop; orchestrator checks `Victory` event.
- Determinism: no float, no `HashMap` iteration order in resolution; all RNG via
  `state.rng`.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] Sequential order: `current_actor` cycles 0→1→…→last→advance→0. `advance_turn` once per cycle.
- [ ] Illegal command yields `Rejected` (no panic, no state change).
- [ ] `step(state, [EndTurn])` for all actors then `advance_turn` ⇒ `turn` increments by 1, all `moves_left` reset.
- [ ] `ConnectRoute` stores `safe_route` path; cost deducted; slots decremented.
- [ ] `RaidRoute` with no defender: Threatened, then Severed on 2nd consecutive raid (DD §8.4).
- [ ] Starvation: city water 0 + negative flow ⇒ pop −1; Well Fort floor at 1.
- [ ] Combat uses `state.rng`: same seed+commands ⇒ identical outcome.
- [ ] `Victory` emitted when V1/V2/V3 threshold met or `turn_limit` reached.
- [ ] Replay: re-issuing saved command log against `new_game(scenario,seed)` reproduces end state.
- [ ] Contested raid resolves by actor order (first-come) deterministically.

## 9. References

- Design: DD §4 (loop), §7 (cities/growth), §8 (routes/raids/upkeep), §9 (units),
  §10 (combat), §13 (victory), §16.1 (sequential), §18 (OQ #3 balance, #6 raid order).
- Architecture: ARCH §5 (turn loop & command), §16 (errors).
- ADRs: ADR-0004 (sequential + Command), ADR-0003 (pure core), ADR-0006 (RNG).
- Related specs: `foundation-core-data-model.md`, `foundation-hex-grid-math.md`
  (`astar`/`safe_route`), `foundation-world-generation.md` (`new_game`),
  `foundation-save-load.md` (replay = command log).

## 10. Open Questions (carried)

- **DD #4 / OQ-2:** Scout-can-found vs Founder — `FoundCity` allows Scout for now;
  balance call deferred.
- **DD #6 / OQ-3:** Contested-raid order proposed = actor order (first-come);
  awaiting design sign-off.
- **DD #3 / OQ-1:** Isolation (−2) & synergy (+10%) are table values, tunable.
- **DD #7:** AI route-competence is a planning concern, not engine; engine just
  applies whatever `Command`s `ai_plan` emits.
