# Behavior Spec: AI Opponents

> **Phase:** 3 — Per-system behavior specs (group: Behavior & Presentation)
> **Crate:** `dcs-core` (module `dcs-core::ai`) — pure, deterministic
> **Status:** Draft for review
> **Implements:** DD §11 (AI); ARCH §9 (AI architecture); ADR-0003 (pure core), ADR-0004 (Command-only / same enum as human), ADR-0006 (RNG in state)

---

## 1. Purpose

Define the opponent AI as a **pure function** that, given an immutable `GameState`
view, returns the `Command`s for one actor's turn. The AI emits the **same
`Command` enum** as the human and goes through the **same resolver** (ADR-0004), so
**cheating is impossible by construction** — it can only plan against what its own
fog-of-war allows it to see. The AI must treat the caravan route network as a
first-class concern (DD §18 Open Question #7 / ARCH §9): it plans, defends, and
attacks routes so the signature "routes > oases" mechanic is exercised by opponents,
not just the player. Difficulty tiers (Easy/Normal/Hard) are retained in the design and adjust how far the AI looks
ahead and how greedily it optimizes, without changing the algorithm shape — but they are
POST-MVP (the MVP ships one primitive profile; see §10). The `ai_plan` interface is the
stable extension point for adding those tiers and alternate AI implementations (see §9.1).

## 2. Scope

**In scope**
- `ai_plan(state, player, difficulty) -> Vec<Command>` — the single public entry.
- No-cheat visibility: AI reads only tiles/entities permitted by its `discovered` set.
- Personality-driven weighted utility (Expansionist / Raider / Trader / Fortifier, DD §11.1).
- Difficulty parameters (Easy / Normal / Hard, DD §11.3).
- Decision areas: expansion (found), city build-up & specialization, **route network
  planning & defense**, training, scouting, raiding, threat response.
- The **assess → prioritize → emit** planning pipeline.
- Route-security baked into utility (DD §18 OQ-7).

**Out of scope**
- Rendering / input (render spec). The AI only returns `Command`s.
- The resolver / turn engine (turn-engine spec) — it applies the returned `Command`s.
- Combat formula (combat spec), route establishment path (caravan spec), yield math
  (economy / caravan specs) — the AI only *emits* `ConnectRoute`/`Patrol`/etc.; the
  engine computes results.
- Learning / ML — explicitly heuristic & testable (DD §11.2).

## 3. Responsibilities

- Produce a **deterministic** `Vec<Command>` for the actor's turn from `&GameState`
  (no mutation, no I/O).
- Draw any tie-breaks / randomness **only** from `state.rng` (ADR-0006) — and only
  when truly needed (e.g. choosing among equal-utility candidates); the bulk of
  planning is a deterministic utility sort, so most games are reproducible without
  RNG draws.
- Never read enemy units/cities/routes hidden by the actor's fog (uses the fog
  queries from `gameplay-fog-of-war.md`).
- Respect resource/legality constraints — emit only `Command`s the resolver will
  accept; illegal commands waste the actor's turn, so the planner self-validates via
  `validate` (turn-engine §5) before emitting.

## 4. Data Structures / Additions

The AI module adds only **tunable planning tables** and a transient `Situation`
struct (not stored in `GameState` — it is scratch for one `ai_plan` call). No new
entity types.

```rust
/// Tunable per-personality / per-difficulty planning parameters (DD §11.1, §11.3).
/// All weights are relative; only their ratios matter.
pub struct AiParams {
    pub command_budget:    usize,   // max actions emitted this turn (greed/lookahead proxy)
    pub lookahead:         u8,      // how many future route/defense steps it reasons about
    pub expand_weight:     f32,     // found new cities / grab oases
    pub build_weight:      f32,     // buildings + specialization
    pub route_weight:      f32,     // establish + reinforce caravan network
    pub route_security:    f32,     // patrol/guard exposed routes (DD §18 OQ-7)
    pub raid_weight:       f32,     // attack enemy routes / cities
    pub scout_weight:      f32,     // reveal map
    pub defend_cities:     f32,     // garrison / fortify
    pub preemptive_raid:   bool,    // Hard: cut enemy weakest link before it develops
    pub defend_core_routes:bool,    // Normal/Hard: always cover threatened routes
}

/// Transient view of the world as the AI may legally perceive it (fog-aware).
pub struct Situation {
    pub own_cities:    Vec<CityId>,
    pub own_units:     Vec<UnitId>,
    pub own_routes:    Vec<RouteId>,
    pub own_oases:     u32,
    pub total_oases:   u32,                 // from map generation (always known: it's terrain)
    pub fog_frontier:  Vec<TileId>,         // nearest unexplored tiles to expand toward
    pub visible_enemy_units:   Vec<UnitId>, // is_unit_visible(player, u) only
    pub visible_enemy_cities:  Vec<CityId>, // is_city_visible(player, c) only
    pub visible_enemy_routes:  Vec<RouteId>,// is_route_visible(player, r) only
    pub exposed_own_routes:    Vec<RouteId>,// own routes with an uncontrolled tile
    pub threatened_own_routes: Vec<RouteId>,// status == Threatened / Severed
}
```

Base weight tables (first-pass, tunable — DD §18 OQ-1 analogue for AI):

| Personality   | expand | build | route | security | raid | scout | defend |
|---|---|---|---|---|---|---|---|
| Expansionist | 1.4 | 0.9 | 1.0 | 0.6 | 0.5 | 1.2 | 0.5 |
| Raider       | 0.7 | 0.6 | 0.8 | 0.5 | 1.6 | 1.0 | 0.4 |
| Trader       | 0.8 | 1.3 | 1.6 | 1.2 | 0.3 | 0.7 | 0.7 |
| Fortifier    | 0.6 | 1.2 | 1.0 | 1.4 | 0.4 | 0.5 | 1.5 |

Difficulty multiplies the *greed/competence* axis, not the personality:

```
Easy   = command_budget 4,  lookahead 0, security*0.4, route*0.7, preemptive_raid=false, defend_core_routes=false
Normal = command_budget 7,  lookahead 1, (weights as-is),        preemptive_raid=false, defend_core_routes=true
Hard   = command_budget 10, lookahead 2, security*1.2, raid*1.2,  preemptive_raid=true,  defend_core_routes=true
```

> **Difficulty tiers are POST-MVP.** Easy = "defend a bit, less reliably" — it performs
> occasional, unreliable route defense (resolves OQ-7), not a flat under-defend.
> Normal/Hard = reliable defense (`defend_core_routes` always covers threatened routes)
> plus the full eventual behavior. The primitive MVP profile (§10) does only occasional,
> unreliable route defense and does not yet use these tiers.

## 5. Key Functions / API

```rust
/// THE public entry. Pure: reads &GameState, returns the actor's action Commands.
/// `difficulty` is passed by the orchestrator (dcs-app) from player.kind; the
/// AI reads `player.kind`'s personality itself. The orchestrator appends `EndTurn`
/// before calling `step` (see §7 / ARCH §5.7) — ai_plan returns ACTIONS ONLY.
pub fn ai_plan(state: &GameState, player: PlayerId, difficulty: Difficulty) -> Vec<Command>;

/// Build the fog-aware Situation (§4). Uses fog queries from fog spec.
fn assess(state: &GameState, player: PlayerId) -> Situation;

/// Score candidate actions by weighted utility; returns them sorted desc.
fn prioritize(state: &GameState, player: PlayerId, sit: &Situation, p: &AiParams)
    -> Vec<ScoredAction>;

/// Emit a legal, budget-limited Command list from the prioritized candidates.
fn emit(state: &GameState, player: PlayerId, ranked: &[ScoredAction], p: &AiParams)
    -> Vec<Command>;

/// --- candidate generators (each returns scored actions; helpers in §6) ---
fn candidates_expand(state, player, sit)  -> Vec<ScoredAction>;
fn candidates_build(state, player, sit)   -> Vec<ScoredAction>;
fn candidates_routes(state, player, sit)  -> Vec<ScoredAction>;   // ConnectRoute + Patrol
fn candidates_raid(state, player, sit, p) -> Vec<ScoredAction>;   // RaidRoute / RaidCity
fn candidates_scout(state, player, sit)   -> Vec<ScoredAction>;   // MoveUnit toward fog
```

`ScoredAction` is an internal `(f32 score, Command cmd, u8 category)` tuple used only
to rank and budget; it is never stored.

## 6. Algorithms

### 6.1 Pipeline: assess → prioritize → emit

```
fn ai_plan(state, player, difficulty) -> Vec<Command> {
    let p = params_for(personality_of(state, player), difficulty);
    let sit = assess(state, player);                 // fog-aware snapshot
    let ranked = prioritize(state, player, &sit, &p); // weighted-utility sort
    emit(state, player, &ranked, &p)                 // budgeted, validated commands
}
```

- **assess:** enumerates own entities (always fully visible to self) and filters
  enemy entities through `is_unit_visible` / `is_city_visible` / `is_route_visible`
  (fog spec §5). Computes `fog_frontier` = own-discovered tiles adjacent to an
  undiscovered tile (expand targets). Flags own routes whose `path` has an
  uncontrolled tile via `is_route_tile_controlled` (caravan spec §6.6) and those
  whose `status` is `Threatened`/`Severed`.
- **prioritize:** for each candidate generator, compute a `score = base_utility *
  weight[category] * situation_modifier`. Example modifiers: a `ConnectRoute` between
  two *unconnected* own cities gets a bonus proportional to the resulting network
  synergy (`network_synergy`, caravan spec §6.4); a `Patrol` on an `exposed_own_route`
  gets a bonus scaled by `route_security`; a `RaidRoute` on a *visible enemy* route
  gets a bonus scaled by `raid_weight` (and extra if `preemptive_raid` and the route
  is the player's *weakest link*, i.e. fewest alternate routes).
- **emit:** walk `ranked` in order, `validate(state, &cmd)` each; keep it if legal and
  the per-turn `command_budget` is not exceeded; stop when budget is spent or the list
  is exhausted. No `EndTurn` is emitted here.

### 6.2 Expansion (found)

- Choose an unowned **Oasis** in/near `fog_frontier` reachable by a Scout (or found
  directly if a Scout is already on/adjacent to an oasis). Prefer oases that are
  (a) not visible as already-owned by an enemy and (b) closest to existing own cities
  for network cohesion.
- Emit `FoundCity { unit, tile }` (Scout can found — cities spec §6.1 / DD #4 default)
  if `player.resources.influence >= FOUND_CITY_INFLUENCE (10)`.
- Expansionist/Hard expand more (higher weight + budget); Fortifier/Raider expand less.

### 6.3 City build-up & specialization

- For each own city with `building_slots` free and affordable Wealth, emit `Build`
  for the building that best fits the city's intended role:
  - **WellFort role** → `Well` (water), then `Granary` (cap); if `population >= 3`
    and no specialization yet, `Specialize { spec: WellFort }`.
  - **TradeHub role** → `Market` (route wealth), `Caravanserai` (+slot/−upkeep);
    `Specialize { spec: TradeHub }` once eligible.
  - **Fortress role** → `Watchtower` (def/fog), `Specialize { spec: Fortress }`.
  - **Scholar role** → `Temple` (influence), `Specialize { spec: ScholarOutpost }`.
- Role assignment is heuristic: the first city is typically WellFort (survival), the
  second TradeHub (economy), and one Fortress if any enemy is visible; Trader
  personalities over-index on TradeHub, Fortifier on Fortress, etc.

### 6.4 Route network planning & defense (SIGNATURE — mandatory route awareness)

This is the make-or-break area (DD §18 OQ-7). The AI treats routes as primary:

1. **Connect:** for every pair of own cities not yet in the same active connected
   component (per `connected_city_count` / union-find in caravan spec §6.4), and where
   `ConnectRoute` is legal (both `route_slots` free, Wealth ≥ cost, and **the
   `safe_route` path crosses only discovered tiles for the actor** — fog spec §6.3),
   emit `ConnectRoute { from, to }`. Prefer pairs that maximize `network_synergy`
   (web, not spokes).
2. **Defend:** for every `exposed_own_route` (a route with an uncontrolled tile) and
   every `threatened_own_routes` entry, if a spare **Caravan Guard** exists (or can
   be trained — see §6.5), emit `Patrol { unit, tile }` on/adjacent to the most
   exposed uncontrolled tile (one Guard covers itself + 6 neighbors). This is gated by
    `route_security` weight: Easy AI defends a bit, but less reliably (OQ-7 resolved —
    occasional, unreliable route defense), Normal/Hard always cover threatened routes
    (`defend_core_routes`).
3. **Redundancy:** if a critical link has only one route, plan a second `ConnectRoute`
   between the same pair when slots/Wealth allow (Hard only) — redundancy prevents
   isolation (caravan/economy specs §6.4).
4. **React:** if a route became `Severed` last turn, prioritize re-patrol or a new
   `ConnectRoute` before any expansion this turn (threat-response, DD §11.2).

### 6.5 Training & scouting

- Train units within the empire cap (`2 + total Pop`, units spec §6.4) when Wealth
  allows: keep at least one Caravan Guard per exposed route (route-security), one
  Scout while `fog_frontier` is non-empty, and Raiders if `raid_weight` is high
  (Raider personality / Hard) or an enemy is visible.
- Scout moves: emit `MoveUnit { unit: scout, to: fog_frontier tile }` to reveal; the
  stop-tile radius reveal (fog spec §6.2) makes Scouts the discovery engine.

### 6.6 Raiding & threat response

- If a *visible enemy* route has an exposed tile (and the AI has a Raider adjacent or
  can move one there within `moves_left`), emit `RaidRoute` (gated by `raid_weight`).
  `preemptive_raid` (Hard) targets the enemy's **weakest-link** route — the one whose
  sever would isolate the most enemy cities (compute via `is_city_isolated` on a
  hypothetical sever).
- If a *visible enemy* city is weak (low population / no Garrison / not Fortress and
  adjacent to a Raider), emit `RaidCity`.
- If the AI itself is threatened (enemy unit adjacent to an own city or route), raise
  `defend_cities`/`route_security` priority this turn (garrison a Guard, or reroute).

### 6.7 Determinism & RNG (ADR-0006)

- The utility sort is deterministic; ties are broken by a stable `Command`/`entity ID`
  order so the same `state` yields the same plan **without** drawing RNG.
- RNG is drawn **only** if a genuine coin-flip is needed (e.g. two candidates with
  equal score and equal IDs — rare). Any such draw uses `state.rng.next_u32()` *inside*
  `ai_plan`, keeping the function pure and the replay reproducible. No `thread_rng` /
  `std::time`.

## 7. Edge Cases / Invariants

- **No cheating:** the AI never reads enemy units/cities/routes hidden by its fog —
  `assess` uses only the fog-spec visibility queries. Omniscience is impossible by
  construction (ADR-0004 purity + fog data).
- **Self-validation:** every emitted `Command` passes `validate(state, &cmd)`
  (turn-engine §5); an illegal command would waste the turn, so it is dropped.
- **EndTurn not emitted:** `ai_plan` returns actions only; the orchestrator appends
  `EndTurn` before `step` (ARCH §5.7). (If a future refactor wants the AI to end its
  own turn, appending `Command::EndTurn` last is allowed but not required.)
- **Budget cap:** `command_budget` limits emitted actions so the AI does not try to do
  everything in one turn; leftover intents carry to next turn naturally.
- **Route planning through fog:** `ConnectRoute` is only emitted when `safe_route`
  would cross discovered tiles for the actor (fog spec §6.3) — the same rule the human
  UI enforces.
- **Defeated / no cities:** if the actor has no cities, `ai_plan` returns an empty
  `Vec` (the engine skips defeated actors anyway).
- **Determinism:** identical `(seed, commands)` ⇒ identical AI plans; no hidden state.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `ai_plan` returns only `Command`s; the orchestrator appends `EndTurn`.
- [ ] `ai_plan` is callable as `ai_plan(state, player, difficulty)` and is pure (no `&mut GameState`).
- [ ] AI emits `ConnectRoute` between unconnected own cities when slots/Wealth allow and path is fog-legal.
- [ ] AI emits `Patrol` for an exposed/threatened own route under Normal/Hard (`defend_core_routes`), and mostly skips it under Easy (OQ-7 stress test).
- [ ] AI never references an enemy unit whose tile is not in its `discovered` set (no-cheat test: plant a hidden enemy, assert it is not targeted).
- [ ] AI prefers founding on an unowned visible oasis; spends Influence only if `>= 10`.
- [ ] AI specializes cities once `population >= 3`: at least one WellFort + one TradeHub appear for non-Raider personalities.
- [ ] Hard AI emits `RaidRoute` on the enemy's weakest-link route when a Raider can reach it; Easy does not prioritize pre-emptive raids.
- [ ] Every emitted `Command` passes `validate(state, &cmd)` for the current actor.
- [ ] `command_budget` is never exceeded.
- [ ] Same `state` + difficulty ⇒ identical plan with no RNG draw; with an RNG tie-break, same `state.rng` state ⇒ identical plan.
- [ ] Severed own route last turn → re-patrol / re-connect prioritized over expansion this turn.

## 9. References

- Design: DD §11 (AI) — §11.1 personalities, §11.2 decision-making + route awareness, §11.3 difficulty, §18 OQ-7 (route-competence risk).
- Architecture: ARCH §9 (AI architecture — pure `ai_plan`, utility-based, route awareness mandatory, difficulty tiers), §5.5 (AI → Command), §2.4 (crate API).
- ADRs: ADR-0003 (pure core — AI is core, deterministic), ADR-0004 (Command-only; same enum as human ⇒ no cheat), ADR-0006 (RNG lives in state; AI draws only `state.rng`).
- Related specs: `gameplay-fog-of-war.md` (visibility queries the AI must use), `gameplay-caravan-routes.md` (`safe_route`, `is_route_tile_controlled`, `network_synergy`, `connected_city_count`, route state), `gameplay-cities.md` (`FoundCity`, `Specialize`, `building_slots`), `gameplay-units-movement.md` (training cap, `Patrol`/`RaidRoute`/`RaidCity`), `gameplay-resources-economy.md` (`is_city_isolated`), `foundation-core-data-model.md` (`PlayerKind`, `Difficulty`, `AiPersonality`), `foundation-turn-engine.md` (`Command`/`EndTurn`, `validate`), `foundation-scenario-config.md` (personalities per player).

## Extensibility — `ai_plan` as the stable extension point

`ai_plan(state, player, difficulty) -> Vec<Command>` is the single, **stable
extension point** for all AI evolution. Because it is a **pure, fog-aware** function
that emits the **same `Command` enum/resolver as the human** (ADR-0004), new AI
implementations and new difficulty levels can be added **without changing the
interface or the core engine**:

- **Alternate implementations** (a smarter rational planner, a personality-driven
  variant, a scripted tutorial opponent) are drop-in replacements behind the same
  signature — the orchestrator simply calls a different planner for a given
  `player.kind`.
- **Difficulty tiers** (Easy/Normal/Hard, DD §11.3) and **personality matrices** are
  expressed as **data**, not code: the `AiParams` weight tables and the difficulty
  multipliers (§4) are content/strategy tables (per the earlier data-driven decision).
  Enabling the full matrix is a config change, not an algorithm rewrite.

The MVP ships only the one primitive profile (§10, MVP note); everything beyond it is
additive on top of this interface.

## 10. Open Questions (carried; OQ-7 resolved)

- **DD #7 / OQ-7 (route-competence) — RESOLVED:** the Easy tier does **not**
  under-defend. Decision: Easy = "defend a bit, less reliably" — it performs
  occasional, unreliable route defense (sometimes park a Caravan Guard on an exposed
  route), which resolves the earlier under-defend / "feels flat" risk (OQ-7). Normal/Hard
  retain reliable defense (`defend_core_routes`) plus the full eventual behavior. The
  primitive MVP profile also does only occasional, unreliable route defense (see MVP note
  below).
- **DD #6 / OQ-3:** contested-raid resolution order (actor order) is engine-level; the
  AI simply emits `RaidRoute` and the engine arbitrates (turn-engine §6.6).
- **DD #4 / OQ-2:** `FoundCity` by Scout is the default; the AI uses Scouts to found.
- **DD #3 / OQ-1:** AI weight tables are first-pass proposals, tunable like all
  balance numbers.
- **MVP ships ONE PRIMITIVE profile (not rational / not win-oriented):** the MVP AI is a
  single primitive implementation. It does NOT need to play rationally or optimize; at
  minimum it must (a) found cities via Scout on oases, (b) connect its cities into a
  caravan route network, and (c) perform OCCASIONAL, UNRELIABLE route defense (sometimes
  park a Caravan Guard on an exposed route) so the signature caravan mechanic is present
  and exercisable. The full rational pipeline (weighted assess → prioritize → emit,
  redundancy, reacting to Severed routes, raiding the weakest link, personalities) is the
  EVENTUAL POST-MVP target the user confirmed they want. The `ai_plan(state, player,
  difficulty)` interface is the EXTENSION POINT: future difficulty tiers and alternate AI
  implementations plug in WITHOUT changing the interface or core. The personality×
  difficulty matrix is data, not code, so enabling the rest is a config change.
