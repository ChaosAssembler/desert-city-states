# Foundation Spec: Scenario Configuration

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (module `dcs-core::scenario`) + `dcs-protocol` (shared types)
> **Status:** Draft for review
> **Implements:** DD §5.2, §13, §16.2; ARCH §11; ADR-0003 / ADR-0007 (config lives in core, serialized)

---

## 1. Purpose

Define the `ScenarioConfig` struct that captures the **configurable session
length** decision (DD §5.2, §13, §16.2, Open Questions #5/#9): map size, player
count, turn limit, and the three victory thresholds — all of which **scale down**
for small maps / 2 players. Provides loading from file/defaults and validation
rules. Consumed by `new_game` (world-gen) and the turn engine's victory check.

## 2. Scope

**In scope:** `ScenarioConfig` fields + `Default` (full-scope starting point);
win-threshold scaling function; file load (TOML/JSON) + defaults; validation
(`validate()`).

**Out of scope:** generation algorithm (world-gen spec); victory *tracking*
(turn-engine spec); save envelope (save-load spec — `ScenarioConfig` is a field
of `GameState`, not separately versioned).

## 3. Responsibilities

- Be the single source of tunable session parameters.
- Derive scaled win thresholds from `(map_radius, player_count)` per DD §13.
- Validate inputs before `new_game` (reject impossible combos).
- Serialize cleanly as part of `GameState` (no extra versioning here).

## 4. Core Data Structures

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScenarioConfig {
    pub map_radius: u8,            // MVP 4 (61 tiles); full 7-9 (169-271)
    pub player_count: u8,          // 2-4
    pub turn_limit: u32,           // MVP 30; full 60 (default, scalable)
    pub ai_personalities: Vec<AiPersonality>,  // len must == player_count-1

    // victory thresholds (defaults; may be overridden, then scaled)
    pub oasis_majority_pct: u8,    // V1 (default >= 50)
    pub wealth_score_target: u32,  // V2 (default 200)
    pub relic_count: u8,           // V3 (derived from map if 0)
    pub relic_hold_turns: u32,     // V3 hold duration

    // which victory conditions are actually checked (data-driven; see note below)
    // default = all three; MVP = [OasisDominance]
    pub victories_enabled: Vec<VictoryKind>,

    pub symmetry: bool,            // mirror placement toggle (DD §5.7)
    pub seed: u64,                 // shown in menu (DD §5.4)
}
```

## 5. Key Functions / API

```rust
impl Default for ScenarioConfig {
    /// Full-scope defaults: radius 7, 4 players, 60 turns, thresholds as DD §13.
    fn default() -> Self;
}

/// MVP-friendly preset (radius 4, 3 players, 30 turns, V1 only but meters built).
pub fn mvp_preset() -> ScenarioConfig;

/// Load from a TOML/JSON file (or embedded string); falls back to Default on
/// missing file. Merges user overrides onto defaults.
pub fn load(path: &Path) -> Result<ScenarioConfig, ScenarioError>;

/// Scale win thresholds down for small maps / few players (DD §13).
/// Called by `load`/`default` after fields are set, OR explicitly by caller.
pub fn scale_thresholds(cfg: &mut ScenarioConfig);

/// Validate: returns Err if any constraint violated (see §7).
pub fn validate(cfg: &ScenarioConfig) -> Result<(), ScenarioError>;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("map_radius must be 4..=9")]
    BadRadius,
    #[error("player_count must be 2..=4")]
    BadPlayerCount,
    #[error("ai_personalities len ({0}) != player_count-1 ({1})")]
    PersonalityMismatch(usize, u8),
    #[error("oasis_majority_pct must be 50..=100")]
    BadMajority,
    #[error("relic_count exceeds available ruins budget for this map")]
    TooManyRelics,
    #[error("turn_limit must be > 0")]
    BadTurnLimit,
}
```

## 6. Algorithms

### 6.1 Defaults (full-scope starting point, DD §13)

`map_radius=7, player_count=4, turn_limit=60, oasis_majority_pct=50,
wealth_score_target=200, relic_count=2, relic_hold_turns=10, symmetry=false,
ai_personalities=[Expansionist, Raider, Trader]`. MVP preset: `radius=4,
player_count=3, turn_limit=30` with same thresholds (V1 focus).
`victories_enabled` for `Default` = `[OasisDominance, WealthScore, RelicHold]`
(all three); for `mvp_preset` = `[VictoryKind::OasisDominance]` (V1 only).

### 6.2 Threshold scaling (DD §13, §16.2 — scales *down*)

Applied by `scale_thresholds` so small boards / 2 players stay winnable:

```rust
let size_factor = clamp(map_radius as f32 / 7.0, 0.5, 1.0);  // radius 4 -> ~0.57
let pct_factor  = clamp(player_count as f32 / 4.0, 0.5, 1.0); // 2p -> 0.5

// V2 score: shrink with board+player scale (fewer oases/route income)
wealth_score_target = max(50, round(200 * size_factor * pct_factor));

// V3 relics: 1 relic for small maps / 2 players; 2 for full
relic_count = if map_radius < 6 || player_count <= 2 { 1 } else { 2 };

// V3 hold duration: shorter for small maps
relic_hold_turns = if map_radius < 6 { 6 } else { 10 };

// Turn limit: shrink toward MVP for small boards
turn_limit = max(20, round(turn_limit as f32 * lerp(size_factor, 1.0, 0.5)));
// (if user explicitly set turn_limit, scaling only tightens the *default* path;
//  explicit overrides are respected but still validated > 0)
```

`oasis_majority_pct` stays ≥50 (a majority is a majority regardless of size) but
the *count* of oases is bounded by generation (world-gen guarantees ≥ player_count).

### 6.3 Load & merge

1. Start from `Default::default()` (or `mvp_preset` if a `--mvp` flag).
2. Parse file; for each present key, override the default.
3. Auto-derive `relic_count` from `map_radius`/`player_count` if left at 0.
4. **`victories_enabled`:** if the loaded file does not specify it, keep the
   default (all three `[OasisDominance, WealthScore, RelicHold]`) — do **not**
   clear it to empty. Only an explicitly provided (non-empty) list overrides.
5. Run `scale_thresholds` then `validate`.

## 7. Edge Cases / Invariants

- `player_count ∈ [2,4]`; `map_radius ∈ [4,9]` (DD §5.2).
- `ai_personalities.len() == player_count - 1` (player 0 is Human).
- `oasis_majority_pct ∈ [50,100]`.
- `victories_enabled` must be **non-empty** and may contain only valid
  `VictoryKind` variants (`OasisDominance`, `WealthScore`, `RelicHold`). An empty
  list is rejected (a session must have at least one win condition). Note:
  `oasis_majority_pct` is only relevant when `OasisDominance` is enabled, but the
  validator does not remove it from config (don't over-constrain unrelated fields).
- `relic_count <=` max ruins budget for the map (world-gen can create up to ~2–3
  ruins; if `relic_count` exceeds that, validator rejects `TooManyRelics` — and
  world-gen will create extra ruins to satisfy, but cap defensively).
- Radius-4 with 4 players + ≥4 spacing is borderline (world-gen §7); validator
  warns but allows; balance tables should discourage this combo.
- `seed` is any `u64`; shown in menu for shareable maps (DD §5.4).

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `Default::default()` produces a valid config (passes `validate`).
- [ ] `mvp_preset()` `validate()`s and has `map_radius==4, player_count==3`.
- [ ] `scale_thresholds` on radius 4 / 2 players ⇒ `relic_count==1`, hold < 10, score < 200, turn_limit < 60.
- [ ] `scale_thresholds` on radius 7 / 4 players ⇒ unchanged (1.0 scale) from defaults.
- [ ] `load` from a partial file merges onto defaults without clobbering unset keys.
- [ ] `validate` rejects `player_count=1`, `map_radius=3`, `ai_personalities` length mismatch, `oasis_majority_pct=40`.
- [ ] `ScenarioConfig` round-trips via serde (as part of `GameState`).
- [ ] Explicit user `turn_limit` override respected but still `> 0`.

## 9. References

- Design: DD §5.2 (map size), §13 (victory thresholds + scaling), §16.2 (pacing),
  §18 (OQ #5/#9 configurable session length, #10 screen).
- Architecture: ARCH §11 (scenario struct), §3.1 (`GameState.scenario`).
- ADRs: ADR-0003 (config in pure core), ADR-0007 (serialized as part of state).
- Related specs: `foundation-world-generation.md` (`new_game(scenario, seed)`),
  `foundation-turn-engine.md` (victory check reads thresholds),
  `foundation-core-data-model.md` (`GameState.scenario` field),
  `foundation-save-load.md` (no separate versioning for scenario).

## 10. Open Questions (carried)

- **DD #5 / OQ:** Exact scaling constants are first-pass (tuned by playtest); kept
  in `scale_thresholds` as tunable expressions, not hardcoded magic per call site.
- **DD #10:** Single-screen vs zoom is render-only; scenario supplies `map_radius`
  and the renderer decides fit/zoom (data unaffected).
- Symmetry default (`false`) is a generation toggle; scenario just carries the flag.
- **Active victories are data-driven.** `victories_enabled` selects which
  `VictoryKind`s the turn engine's victory check evaluates. This lets the MVP ship
  with only `OasisDominance` (V1) while meters for V2/V3 are still built, and
  post-MVP configs enable `WealthScore` / `RelicHold` (V2/V3) simply by listing
  them — no code change. This directly serves the extensibility principle: new
  victory kinds are opt-in via data, not forced on every session.
