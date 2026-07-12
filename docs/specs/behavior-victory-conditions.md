# Behavior Spec: Victory Conditions

> **Phase:** 3 — Per-system behavior specs (group: Behavior & Presentation)
> **Crate:** `dcs-core` (module `dcs-core::victory`) — pure, deterministic
> **Status:** Draft for review
> **Implements:** DD §13 (Victory Conditions); ARCH §13 (Victory Tracking); ADR-0003 (pure core), ADR-0004 (Command-only mutation path)
> **Depends on:** `foundation-scenario-config.md` (thresholds scale from `ScenarioConfig`)

---

## 1. Purpose

Specify the three victory types from DD §13, the **`VictoryTracker`** that lives in
`GameState` and is updated every end-of-turn, the concrete thresholds (read from
`ScenarioConfig`, scaled down for small maps / 2 players), the **prestige score
formula** for V2 (generalized from DD §13's `Wealth×1 + Influence×2` proposal to also
reward territory and routes — the economic spine of "routes > oases"), the **turn-limit
fallback** (highest score wins), and elimination as an instant win. All checks run
through the turn engine's `advance_turn` (ADR-0004) — the victory module never mutates
except updating the tracker it owns.

## 2. Scope

**In scope**
- V1 Oasis Dominance (control ≥ `oasis_majority_pct%` of all oases, or eliminate all rivals).
- V2 Wealth/Prestige Score (prestige score ≥ `wealth_score_target`).
- V3 Relic Hold (hold ≥ `relic_count` relic sites for `relic_hold_turns` consecutive turns).
- `VictoryTracker` fields + update at `advance_turn`.
- Prestige score formula (Wealth / Influence / territory / routes).
- Threshold **scaling** from `ScenarioConfig` (small map / 2p ⇒ smaller N, M, score).
- Turn-limit fallback + elimination win.
- `Victory` event emission.

**Out of scope**
- The *display* of victory meters (render spec — HUD top bar).
- Relic *placement* (world-gen spec); this spec only tracks hold duration.
- Combat/capture mechanics that *cause* elimination (combat / cities specs).
- The resolver itself (turn-engine spec) — this module is called by it.

> **MVP scope:** MVP ships **V1 Oasis Dominance only**. V2 (Wealth/Prestige) and V3
> (Relic Hold) are **post-MVP**. The meters are still built for all three so the later
> victory types can be enabled without rework, but only V1 is active at launch.

## 3. Responsibilities

- Maintain live victory meters in `GameState.victory` (ARCH §13).
- Compute the prestige score for every living player each end-of-turn.
- Decide, deterministically, whether a victory condition is met (or the turn limit
  reached) and emit the single `Victory` `GameEvent` (or none).
- Never mutate anything outside the `VictoryTracker` and the emitted event; all
  underlying state changes happen in the resolver / other modules.

## 4. Data Structures / Additions

Reuses `VictoryTracker` (core-data-model §4.11) and `VictoryKind` (§4.10). No new
entity types; adds only **prestige-score balance weights** (tunable — DD §18 OQ-1):

```rust
// Prestige score weights (DD §13 generalized — see §6.2). Tunable tables.
pub const PRESTIGE_WEALTH_W:      f32 = 1.0;  // Wealth stockpile weight
pub const PRESTIGE_INFLUENCE_W:   f32 = 2.0;  // Influence stockpile weight
pub const PRESTIGE_OASIS_W:       f32 = 8.0;  // each controlled oasis (territory)
pub const PRESTIGE_ROUTE_W:       f32 = 4.0;  // each ACTIVE owned route (network)

// Re-exported from scenario (scenario-config spec) — the live thresholds:
//   scenario.oasis_majority_pct   (V1, default 50)
//   scenario.wealth_score_target  (V2, default 200)
//   scenario.relic_count          (V3 N, default 2; 1 on small/2p)
//   scenario.relic_hold_turns     (V3 M, default 10; 6 on small)
//   scenario.turn_limit           (fallback trigger)
```

`VictoryTracker` (from core-data-model §4.11), re-stated for convenience:

```rust
#[derive(Serialize, Deserialize, Default)]
pub struct VictoryTracker {
    pub oases_controlled: FxHashMap<PlayerId, u32>,   // V1 live count
    pub prestige_score:   FxHashMap<PlayerId, u32>,   // V2 running score
    pub relic_timers:     FxHashMap<RelicId, PlayerId>,// V3 current holders
}
```

> Note: `Relic.consecutive_turns_held` (core-data-model §4.8) is the per-relic hold
> counter incremented in `advance_turn` (turn-engine §6.3 step 7); `VictoryTracker`
> mirrors current holders for fast win-checking. Both derive from the same source.

## 5. Key Functions / API

```rust
/// Recompute all meters for the just-completed turn and update `state.victory`.
/// Called once from `advance_turn` (turn-engine §6.3 step 2 (victory check)) BEFORE the win check.
/// Pure over state except writing the tracker it owns.
pub fn update_victory_tracker(state: &mut GameState) -> Vec<GameEvent>;

/// Decide if a victory has occurred (V1/V2/V3 met, elimination, or turn limit).
/// Returns Some(Victory { kind, winner }) to be emitted by the engine, else None.
/// Pure read; no mutation.
pub fn check_victory(state: &GameState) -> Option<GameEvent>;

/// Prestige score for one player (V2 + turn-limit fallback). Encapsulated so the
/// HUD and the fallback can read it without duplicating the formula.
pub fn prestige_score(state: &GameState, player: PlayerId) -> u32;

/// Count of oases owned by `player` (V1 numerator).
pub fn oases_controlled_by(state: &GameState, player: PlayerId) -> u32;

/// Total oases on the map (V1 denominator) — derived from tiles (always known).
pub fn total_oases(state: &GameState) -> u32;

/// Does `player` currently hold `relic_count` relic sites? (V3 numerator/holder test)
pub fn holds_required_relics(state: &GameState, player: PlayerId) -> bool;
```

## 6. Algorithms

### 6.1 `update_victory_tracker` (per end-of-turn)

For each **living** (`!defeated`) player `p`:

1. `oases_controlled[p] = oases_controlled_by(p)` — oases whose `Tile.owner == Some(p)`
   (an oasis tile is one with `terrain == Oasis`; world-gen guarantees oases are
   city-foundable, so "control" = ownership of the oasis tile).
2. `prestige_score[p] = prestige_score(p)` (see §6.2).
3. `relic_timers`: for each `Relic` whose `holder == Some(p)`, ensure
   `relic_timers[relic.id] = p`; clear entries for relics no longer held by `p`.

`Relic.consecutive_turns_held` is incremented in the turn engine's relic step
(turn-engine §6.3 step 7): a relic held by `p` this turn ⇒ `+1`; a relic whose holder
changed or is `None` ⇒ reset to `0`.

### 6.2 Prestige score formula (V2 + fallback) — DD §13 generalized

DD §13 proposes `Score = Wealth×1 + Influence×2` (target 200). This spec **generalizes**
that proposal to also reward the economic spine — **territory (oases) and the active
route network** — because those are the mechanical expression of "routes > oases"
(DD §8.6) and the task requires the score to derive from Wealth/Influence/territory/
routes. The design's `Wealth×1+Influence×2` is the **stockpile core** of this formula;
the generalization keeps those exact weights and adds territory + route terms:

```
prestige_score(p) = floor(
      Wealth(p)            * PRESTIGE_WEALTH_W(1.0)
    + Influence(p)         * PRESTIGE_INFLUENCE_W(2.0)
    + oases_controlled(p)  * PRESTIGE_OASIS_W(8.0)
    + active_routes(p)     * PRESTIGE_ROUTE_W(4.0)
)
```

where `active_routes(p)` = count of `CaravanRoute` owned by `p` with `status == Active`
(Threatened/Severed routes contribute 0 — they are not "real" network). `Wealth(p)` /
`Influence(p)` are the player's empire stockpiles (`Player.resources`).

The **win threshold** is `scenario.wealth_score_target` (default 200; scaled per
scenario-config §6.2). Because territory + routes now add to the score, the default
200 from DD §13 still applies as the raw target and remains tunable; the weights are
first-pass (DD §18 OQ-1). A player wins V2 when `prestige_score(p) >=
wealth_score_target`.

> Consistency note: this does not contradict DD §13 — DD §13 explicitly marks the
> `Wealth×1+Influence×2` figure a *proposal*, and the V2 meter is "visible to all".
> The generalized formula is the implementation of that proposal; if a designer wants
> the literal stockpiles-only version, set `PRESTIGE_OASIS_W = PRESTIGE_ROUTE_W = 0`.

### 6.3 V1 — Oasis Dominance

```
threshold = ceil(oasis_majority_pct / 100 * total_oases)   // e.g. 50% of 5 -> 3
win if oases_controlled(p) >= threshold   // majority share
   OR  all other living players are defeated (elimination => instant win)
```

`oasis_majority_pct` stays ≥ 50 by scenario validation (scenario-config §7); the
*count* of oases is bounded by world-gen (≥ `player_count`). On a 2-player map the
winner simply needs the majority of the oases that exist.

### 6.4 V3 — Relic Hold

`relic_count` (= N) and `relic_hold_turns` (= M) come from `ScenarioConfig`
(scenario-config §6.2: N=1 for small/2p maps, N=2 for full; M=6 small, M=10 full).

```
win if exists player p such that:
    holds_required_relics(p)                        // p holds ALL relic sites
    AND for every Relic r with is_relic_site:
          r.holder == Some(p) AND r.consecutive_turns_held >= relic_hold_turns
```

World-gen creates exactly `relic_count` relic sites (scenario-config §7 validates
`relic_count <=` ruins budget), so "hold all relic sites" == "hold ≥ N relics". If
relic sites are shared between players, no one satisfies the sole-holder test and V3
does not trigger (a contested relic race continues).

> **MVP scope (V3):** MVP V3 = **sole-hold only** — a player must hold *all* relic
> sites for `relic_hold_turns` consecutive turns.
>
> **Post-MVP (V3):** make the relic-victory **mode configurable** so scenarios are not
> hard-coded to sole-hold. The mode may also allow a **majority-hold** option and/or a
> **"hold a minimum number of relics"** option (e.g. hold ≥ K of the N sites, K ≤ N).
> These parameters are **scenario/data-driven** (part of the configurable win settings
> — see the earlier "configurable session length / win settings" decision) rather than
> baked into the win-check logic.

### 6.5 Turn-limit fallback & elimination

- **Elimination:** if, after `update_victory_tracker`, exactly one living player
  remains (`living_players == 1`), that player wins immediately (kind `OasisDominance`
  — it is the "eliminate all rivals" branch of V1). If zero living players remain
  (mutual elimination — rare), the engine falls back to highest score among the last
  standing set.
- **Turn limit:** if `state.turn >= scenario.turn_limit` and no V1/V2/V3 threshold is
  met, the winner is the living player with the **highest `prestige_score`**; tiebreak
  order: (1) more oases controlled, (2) higher prestige score, (3) lower `PlayerId`
  (deterministic). `VictoryKind::TurnLimit` now exists (core-data-model §4.10, §10
  resolved), so this fallback is emitted as `Victory { kind: TurnLimit, winner }`
  — the mechanical winner determination (highest prestige score, tie-broken by oases
  → score → `PlayerId`) is unchanged, and only the UI label differs from a true
  `WealthScore` victory (core-data-model §4.10 note).

### 6.6 Where it runs

`update_victory_tracker` is called inside `advance_turn` (turn-engine §6.3 step 2 (victory check));
`check_victory` is called immediately after and its `Some` result is emitted as the
`Victory` event that stops the orchestration loop (turn-engine §7). Both are pure
except the tracker write they own; no RNG is drawn (deterministic).

## 7. Edge Cases / Invariants

- **Sole-holder requirement for V3:** a relic contested by two players never satisfies
  V3 (held timers keep resetting on holder change), so V3 rewards secure control.
- **Threatened/Severed routes** contribute 0 to V2 (only `Active` routes count) — a
  severed network weakens your score, reinforcing the route mechanic.
- **Majority vs. count:** V1 uses `ceil(pct/100 * total)` so a bare majority wins even
  when the count is odd; `oasis_majority_pct` is validated ≥ 50.
- **Scaling:** on radius-4 / 2-player, `relic_count == 1` and `relic_hold_turns == 6`
  (scenario-config §6.2) — V3 is still achievable but easier; `wealth_score_target`
  shrinks toward 50–100. Full map keeps defaults (2 / 10 / 200).
- **Defeated players excluded:** `update_victory_tracker` and the fallback consider
  only `!defeated` players; defeated players remain in `players` (ID stable) but never
  win.
- **Determinism:** no RNG, no `HashMap` iteration in the math (tracker maps use
  `FxHashMap` fixed order, ADR-0006); identical `(seed, commands)` ⇒ identical win.
- **Multiple conditions same turn:** if more than one player meets a threshold
  simultaneously, the check returns the player with the highest applicable score /
  earliest `PlayerId` (deterministic); in practice V1/V3 are sole-holder so collisions
  are rare.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `update_victory_tracker` fills `oases_controlled`, `prestige_score`, `relic_timers` for every living player each end-of-turn.
- [ ] V1: a player owning `>= ceil(pct/100 * total_oases)` oases triggers `Victory{OasisDominance}`.
- [ ] V1: last living player wins immediately (elimination branch).
- [ ] V2: `prestige_score(p) >= wealth_score_target` triggers `Victory{WealthScore}`.
- [ ] Prestige score = floor(Wealth·1 + Influence·2 + oases·8 + active_routes·4); Threatened/Severed routes count 0.
- [ ] V3: a player solely holding all `relic_count` relic sites, each with `consecutive_turns_held >= relic_hold_turns`, triggers `Victory{RelicHold}`.
- [ ] V3 does NOT trigger while a relic is contested (holder changed this turn).
- [ ] Scaling: radius-4 / 2p ⇒ `relic_count==1`, `relic_hold_turns < 10`, `wealth_score_target < 200`; radius-7 / 4p ⇒ defaults unchanged.
- [ ] Turn limit reached with no threshold ⇒ `Victory{TurnLimit}` for highest-score living player; tiebreak oases then score then PlayerId.
- [ ] `check_victory` is pure read; only `update_victory_tracker` writes the tracker.
- [ ] `Victory` event emitted exactly once and stops the loop (turn-engine §7).
- [ ] Same `(scenario, seed, commands)` ⇒ identical victory outcome (determinism).

## 9. References

- Design: DD §13 (Victory Conditions) — V1 oasis majority, V2 score (Wealth×1+Influence×2, target 200), V3 relic hold, threshold scaling, turn-limit fallback + tiebreak, MVP ships V1 only but meters built for all three.
- Architecture: ARCH §13 (Victory Tracking — `VictoryTracker`, the three meters, fallback), §3.1 (`GameState.victory`), §5.3 (victory check at `advance_turn`), §11 (`ScenarioConfig` thresholds).
- ADRs: ADR-0003 (pure core), ADR-0004 (victory check runs inside the resolver's `advance_turn`), ADR-0006 (determinism — no RNG in victory).
- Related specs: `foundation-scenario-config.md` (`oasis_majority_pct`, `wealth_score_target`, `relic_count`, `relic_hold_turns`, `turn_limit`, `scale_thresholds`), `foundation-core-data-model.md` (`VictoryTracker`, `VictoryKind`, `Relic`, `Player.resources`), `foundation-turn-engine.md` (`advance_turn` step 7/8, `Victory` event, loop stop), `gameplay-caravan-routes.md` (`RouteStatus::Active` for V2 route count), `gameplay-fog-of-war.md` (relic sites revealed once seen).

## 10. Open Questions (carried, not resolved)

- **DD #3 / OQ-1:** prestige-score weights (`PRESTIGE_OASIS_W`, `PRESTIGE_ROUTE_W`,
  and the `wealth_score_target` interaction) are first-pass; tunable. The design's
  literal `Wealth×1+Influence×2` is recoverable by zeroing the territory/route terms.
- **V2 formula scope:** this spec generalizes DD §13's proposal to include territory +
  routes (per the task). If the design team prefers the literal stockpiles-only score,
  it is a one-line table change — flagged here, not a contradiction.
- **`VictoryKind::TurnLimit` variant — RESOLVED:** the distinct
  `VictoryKind::TurnLimit` variant now exists in the core data model
  (foundation-core-data-model.md §4.10). The fallback winner determination (highest
  `prestige_score` living player, tiebreak oases → score → `PlayerId`) is unchanged and
  correct; the variant simply gives the UI a distinct label ("Time's up — X wins on
  points") versus a true `WealthScore` victory. Implementation must bump `SAVE_VERSION`
  (core-data-model §4.10 note) since adding an enum variant changes the serialized form.
- **DD #5 / OQ:** relic/win thresholds scale with map size & player count — implemented
  via `scale_thresholds` (scenario-config §6.2); exact constants are first-pass.
