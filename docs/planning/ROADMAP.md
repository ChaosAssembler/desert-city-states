# Desert City States — Implementation Roadmap

> **Status:** Planning doc 4 of 4 — phased implementation plan. Implementation status: Phase 0 complete, Phase 1 complete, Phase 2 complete, Phase 3 next.
> **Source of truth for behavior:** `docs/design/Desert-City-States.md` (DD)
> **Architecture:** `docs/architecture/ARCHITECTURE.md` + `docs/architecture/decisions/` (ADR-0001…0008)
> **Specs (binding):** `docs/specs/README.md` and the 15 spec files
> **Tooling-agnostic:** milestones are ordered, not calendar-dated. Map to time per your cadence.

This roadmap sequences work by the dependency order implied by the specs and
ADRs. The core invariant from ADR-0003/0008 governs everything: `dcs-core` is
pure/deterministic/serializable and depends only on `dcs-protocol`; rendering and
app live in separate crates and never feed back into core.

---

## Legend

- `- [ ]` planned · `- [~]` in progress · `- [x]` done · `- [-]` deferred
- Exit/DoD criteria state the concrete verification (test, CLI flag, build gate).
- Spec paths are relative to `docs/specs/`.

---

## Dependency-ordered milestone table

| Phase | Primary specs | Key exit criteria |
|---|---|---|
| **0 — Workspace scaffold** | ADR-0008, ADR-0003, ARCH §2 | 4-crate workspace builds empty; `cargo tree` CI gate green; mise tooling |
| **1 — Foundation (dcs-core)** | `foundation-core-data-model`, `foundation-hex-grid-math`¹, `foundation-scenario-config`, `foundation-world-generation`, `foundation-turn-engine`, `foundation-save-load`, `dcs-protocol` (ADR-0004/0006/0007) | From a seed, generate a map, step turns, serialize/deserialize; save == replay |
| **2 — Gameplay (dcs-core)** | `gameplay-fog-of-war`, `gameplay-cities`, `gameplay-units-movement`, `gameplay-combat`, `gameplay-caravan-routes`, `gameplay-resources-economy` | Headless full core loop works: economy, cities, units, combat, caravan network, isolation penalty |
| **3 — Behavior (dcs-core)** | `behavior-ai-opponents`, `behavior-victory-conditions` | AI plays a full game to a victory/elimination; all 3 victory meters tracked |
| **4 — Presentation (dcs-render + dcs-app)** | `presentation-rendering-ui`, ADR-0001/0003/0004/0005 | Visually playable (human vs AI) on a small map |
| **5 — MVP integration & playtest** | scenario `mvp_preset` (DD §17.1) | Fun, winnable MVP per DD §17.1; first balance pass |
| **6 — Full scope / stretch** | DD §17.2, ADR-0001 fallback | Full game per design |

¹ `foundation-hex-grid-math.md` is referenced by `docs/specs/README.md`, the
core-data-model, world-gen, turn-engine, caravan, and units specs, and ADR-0005,
and **does exist** under `docs/specs/`. Phase 1 implements its module
`dcs-core::hex` (from ARCH §4 + ADR-0005). See Phase 1 deliverables.

---

## Phase 0 — Workspace scaffold

- [x] **Done**

### Goal

Establish the Cargo workspace with the four member crates and the mechanical
guards that keep `dcs-core` engine-free, so all later phases build on enforced
boundaries rather than convention.

### In scope

- Virtual workspace manifest at repo root with members `crates/dcs-core`,
  `crates/dcs-protocol`, `crates/dcs-render`, `crates/dcs-app` (ADR-0008, ARCH §2.1).
- `dcs-protocol` defined as the shared `Command`/`GameEvent`/`VersionedSave`
  contract surface (ARCH §2.3) — crate stub is enough for Phase 0.
- CI `cargo tree` gate asserting `dcs-core` has **no** dependency on
  `dcs-render`/`dcs-app`/macroquad (ADR-0003, ARCH §14.2.1, ARCH §16).
- Dev tooling: consider **mise** for Rust toolchain + tasks; repo `.gitignore`
  already anticipates `mise.local.toml`. A `cargo test -p dcs-core` and a
  `--headless` smoke hook cited by ARCH §16 can be wired now or in Phase 1.

### Out of scope

- Any game logic, types, or rendering (Phase 1+).
- Choosing the PRNG crate or save format (decided Phase 1).

### Key deliverables

- Repo root `Cargo.toml` (virtual) + four crate skeletons (no sim code).
- `crates/dcs-protocol/Cargo.toml` declaring serde dependency.
- CI workflow running `cargo build --workspace` and `cargo tree -p dcs-core`
  (asserted clean of render/app/engine deps).
- `mise.toml` (or equivalent) for toolchain + common tasks; `.gitignore`
  leaves `mise.local.toml` ignored.

### Dependencies

- None (first milestone).

### Exit / Definition of Done

- [x] `cargo build --workspace` succeeds with four empty crates.
- [x] `cargo tree -p dcs-core` shows **no** macroquad / `dcs-render` / `dcs-app`
  edges (CI gate green).
- [x] `dcs-core` depends only on `dcs-protocol` + std/serde/rand-family crates.
- [x] Dev can run `mise run <task>` (or cargo equivalents) for build/test/tree.

---

## Phase 1 — Foundation (dcs-core)

- [x] **Done**

### Goal

Implement the deterministic, serializable simulation core: the data model,
hex math, scenario config, world generation, the turn engine (Command/Event
resolver + sequential phase machine), and save/load — enough to generate a map
from a seed, step turns headlessly, and round-trip state. Heavy unit testing.

### In scope

- **Implement the hex spec module** from `foundation-hex-grid-math.md` (exists
  under `docs/specs/`; from ARCH §4 + ADR-0005) → `dcs-core::hex` (axial
  `(q,r,s=-q-r)`,
  conversions, `neighbors`/`distance`/`ring`/`range`/`line`, in-core A*/Dijkstra,
  single auditable `cube_round`).
- `dcs-protocol`: `Command` + `GameEvent` + `RejectReason` (turn-engine spec §4)
  and `VersionedSave<T>` + `SAVE_VERSION` (save-load spec §4).
- `dcs-core::model`: `GameState` aggregate, entity structs, ID newtypes, catalog
  enums, `FxHashMap`/`FxHashSet` for iteration-stable maps, balance data tables
  (core-data-model spec §4).
- `dcs-core::scenario`: `ScenarioConfig`, `Default`, `mvp_preset()`,
  `scale_thresholds()`, `validate()`, `load()` (scenario-config spec §4–§6).
- `dcs-core::map` (`new_game`): deterministic world generation pipeline
  (world-gen spec §5–§6).
- `dcs-core::turn`: `step`/`advance_turn`/`validate`, sequential actor order,
  phase machine (Order→Resolution→Income→EndOfTurn), `Rejected` (never panic),
  RNG-usage rules; pluggable hooks for gameplay resolution filled in Phase 2
  (turn-engine spec §5–§6).
- `dcs-core::serialize`: json + postcard/bincode behind one API, migration
  registry, `load` auto-detect by extension (save-load spec §4–§6).

### Out of scope

- Gameplay *logic* (economy/cities/units/combat/caravan) — stubbed in the
  resolver, filled in Phase 2.
- AI, victory checks (Phase 3), rendering (Phase 4).

### Key deliverables (spec → module)

- [x] `docs/specs/foundation-hex-grid-math.md` **present** (gap closed) → `dcs-core::hex`
- [x] `docs/specs/foundation-core-data-model.md` → `dcs-core::model`
- [x] `docs/specs/foundation-scenario-config.md` → `dcs-core::scenario`
- [x] `docs/specs/foundation-world-generation.md` → `dcs-core::map`
- [x] `docs/specs/foundation-turn-engine.md` → `dcs-core::turn`
- [x] `docs/specs/foundation-save-load.md` → `dcs-core::serialize`
- [x] `docs/specs/README.md` has no missing-spec gap note
- [x] `dcs-protocol` crate → `Command`/`GameEvent`/`VersionedSave` (ADR-0004/0007)

### Dependencies

- Phase 0 (workspace + CI gate).

### Open questions resolved in this phase

- **PRNG crate (ARCH OQ-6):** choose `nanorand` vs `rand::StdRng`; recommend
  `nanorand` (ADR-0006). The RNG state lives in `GameState` and serializes.
- **Save format (ARCH OQ-5):** default **json for dev**, **postcard for ship**,
  both behind `serialize` (ADR-0007). Pick the concrete ship default now.
- **Contested-raid order (DD #6 / OQ-3):** codify the engine rule — **first-come
  in actor order** (turn-engine spec §6.6) — so all later phases rely on it.
  Final *design sign-off* is validated in Phase 5 playtest.

### Exit / Definition of Done

- [x] `new_game(scenario, seed)` is deterministic: `==` deep-equal for equal
  inputs; different seed → different valid map (world-gen spec §8).
- [x] `step(state, [EndTurn])` cycles actors and `advance_turn` increments
  `turn`, resets `moves_left`, runs victory hook (turn-engine spec §8).
- [x] Illegal command → `GameEvent::Rejected`, no panic, no partial state change.
- [x] `GameState` round-trips via **both** json and postcard; byte-stable
  `tile_index`/`VictoryTracker` (FxHashMap order) (core-data-model §8, save-load §9).
- [x] **Save == replay:** re-issuing a saved `Command` log against `new_game`
  reproduces the end state (turn-engine §8, save-load §7).
- [x] `mvp_preset()` passes `validate()` and yields `map_radius==4,
  player_count==3` (scenario-config spec §8).
- [x] `cargo test -p dcs-core` green; CI `cargo tree` gate still green.

---

## Phase 2 — Gameplay systems (dcs-core)

- [x] **Done**

### Goal

Implement the six gameplay modules that plug into the Phase 1 resolver so a
**full headless simulation of the core loop** works: resources/economy, cities,
units/movement, combat, fog of war, and the signature caravan/route system.

### In scope

- `dcs-core::fog` — visibility model, reveal sources/radii, reveal triggers,
  queries, serialization (fog-of-war spec §4–§6).
- `dcs-core::world` (cities + units entities) — `FoundCity` (Scout can found,
  DD #4 default), `Build`/`Specialize`, `TrainUnit`, worked ring, growth,
  building/specialization effects, unit stats `UNITS`, A* `MoveUnit`, `Patrol`/
  `Garrison`, `RaidRoute`/`RaidCity`, training/upkeep/cap, Zone-of-Control
  (cities spec, units-movement spec).
- `dcs-core::combat` — auto-resolution formula, terrain mods, positioning,
  local combat, city sieges, Raider-vs-route contest, RNG from `state.rng`
  (combat spec §5–§6).
- `dcs-core::caravan` — `safe_route` threat-weighted Dijkstra, `ConnectRoute`
  resolve, route yield + Water-transfer formulas, network synergy/redundancy,
  isolation penalty, route state machine, tile control (caravan-routes spec §5–§6).
- `dcs-core::economy` — per-turn `apply_income`/`apply_global_economy`, caps,
  isolation penalty, starvation/elimination, network wealth yield
  (resources-economy spec §5–§6); wire into `advance_turn`.

### Out of scope

- AI decisions (Phase 3), victory thresholds (Phase 3), rendering (Phase 4).

### Key deliverables (spec → module), in dependency order

- [x] `docs/specs/gameplay-fog-of-war.md` → `dcs-core::fog` (needed by units reveal + caravan fog rule)
- [x] `docs/specs/gameplay-cities.md` → `dcs-core::world` (FoundCity/Build/Specialize/Train) — **sets DD #4 default**
- [x] `docs/specs/gameplay-units-movement.md` → `dcs-core::world` (A*, MoveUnit, Patrol, Raid*)
- [x] `docs/specs/gameplay-combat.md` → `dcs-core::combat` (formula, siege, raid contest)
- [x] `docs/specs/gameplay-caravan-routes.md` → `dcs-core::caravan` (`safe_route`, yields, state machine)
- [x] `docs/specs/gameplay-resources-economy.md` → `dcs-core::economy` (income/upkeep/isolation/starvation)

### Dependencies

- Phase 1 (data model, hex math, turn engine, scenario, save/load, `dcs-protocol`).

### Open questions resolved / validated in this phase

- **DD #4 Scout-founding (OQ-2):** lock the recommended default — `is_founding_unit(Scout)==true`
  (cities spec §6.1). A `Founder` kind remains a later one-predicate flip.
- **Route planning through fog (fog spec §6.3):** implement the specified default —
  `ConnectRoute` is **rejected** if `safe_route` would cross an unexplored tile for
  the actor ("scout before you caravan").
- **DD #3 balance (OQ-1):** first-pass constants (isolation −2, synergy +10%,
  unit/terrain/building tables) are entered as tunable tables; active tuning is
  Phase 5.

### Exit / Definition of Done

- [x] A scripted headless game (scripted `Command` sequences) reaches a natural
  end state with economy, cities, units, combat, and caravan network all active.
- [x] Isolation penalty: city with zero active routes loses exactly 2 Water/turn;
  a city with an active alternate despite a Severed route is **not** isolated
  (economy §8, caravan §8).
- [x] `safe_route` returns inclusive, deterministic, enemy-avoiding path;
  `preview_cost == 5 + path.len()` (caravan §8).
- [x] Raid cascade: no-defender → Threatened (1st) → Severed (2nd consecutive);
  controlled by a patrolling Guard → contest (combat §8, caravan §8).
- [x] Combat: only on/adjacent units fight; each roll −1 HP; Ridge/Salt-Flats
  mods shift odds (combat §8).
- [x] Fog: Scout reveals r3; enemy units hidden in fog (incl. from AI); static
   cities/routes remembered but shown stale unless currently observed
   (memory-marker model); units leave no memory once out of sight (fog §8).
- [x] `cargo test -p dcs-core` green (per-spec acceptance checklists); CI
  `cargo tree` gate still green.

---

## Phase 3 — Behavior (dcs-core)

- [~] **In Progress**

### Goal

Add the pure AI (`ai_plan`) and victory tracking so the core can play itself to a
conclusion, exercising the signature route mechanic as opponents (DD §18 OQ-7).

### In scope

- `dcs-core::victory` — `VictoryTracker` update at `advance_turn`, V1/V2/V3
  checks, generalized prestige score, turn-limit fallback + elimination,
  `Victory` event (victory-conditions spec §5–§6).
- `dcs-core::ai` — `ai_plan(state, player, difficulty) -> Vec<Command>` pure
  function; fog-aware `assess`, weighted-utility `prioritize`, budgeted `emit`;
  **mandatory route awareness** (connect/defend/react); no-cheat visibility;
  MVP ships one Normal-ish profile (ai-opponents spec §5–§6).

### Out of scope

- Rendering/HUD (Phase 4); balance tuning of weights (Phase 5/6).

### Key deliverables

- [ ] `docs/specs/behavior-victory-conditions.md` → `dcs-core::victory`
- [ ] `docs/specs/behavior-ai-opponents.md` → `dcs-core::ai`

### Dependencies

- Phase 2 (all gameplay modules the AI reads/emits `Command`s for) + Phase 1
  (turn engine, scenario thresholds, save format).

### Open questions resolved / validated in this phase

- **V2 prestige formula (victory spec §6.2 / §10):** decide between the literal
  DD `Wealth×1+Influence×2` and the spec's generalized territory+route formula
  (recoverable via weights = 0). Recommended: keep the generalized form as
  implemented, with weights tunable in Phase 5.
- **`VictoryKind::TurnLimit` variant (victory spec §6.5 / §10):** decide whether
  the turn-limit fallback needs a distinct `VictoryKind` or reuses `WealthScore`.

### Exit / Definition of Done

- [ ] `ai_plan` is pure (`&GameState` → `Vec<Command>`), returns **no** `EndTurn`,
  and every emitted command passes `validate` (ai spec §8).
- [ ] AI emits `ConnectRoute` between unconnected own cities (fog-legal) and
  `Patrol` on exposed/threatened routes under Normal/Hard (`defend_core_routes`);
  Easy defends a bit, less reliably (OQ-7 resolved) (ai spec §8).
- [ ] AI never targets an enemy unit/city/route hidden by its own fog (no-cheat
  test) (ai spec §8).
- [ ] A full headless game (`--headless` flag) with 2–3 AI ends in a `Victory`
  event: V1 majority/elimination, or turn-limit fallback by prestige score.
- [ ] Victory meters (`state.victory`) update each end-of-turn; same
  `(scenario, seed, commands)` ⇒ identical outcome (victory spec §8).
- [ ] `cargo test -p dcs-core` green; CI `cargo tree` gate still green.

---

## Phase 4 — Presentation (dcs-render + dcs-app)

### Goal

Build the macroquad renderer and the glue app so the game is **visually
playable** (human vs AI) on a small map. Renderer reads `GameState`, emits
`Command`s only (Rule B / ADR-0003).

### In scope

- `dcs-render` — `Renderer` + `draw_frame(&self, &GameState, &UiView)`,
  `poll_input() -> Vec<Command>`, `screen_to_hex`, `Camera2D` pan/zoom reusing
  in-core `hex::to_pixel`/`from_pixel` (exact cube-round), `fit_map` (macroquad
  built-in UI; egui as fallback, ARCH §14.2.3).
- Drawing: terrain tints, routes (owner-colored, red when Threatened/Severed),
  cities (pop/spec growth), unit tokens, fog overlay (render-only, uses core fog
  queries), HUD (top bar resources+victory meters, left selected, right
  minimap+alerts, bottom End Turn/queue).
- `dcs-app` — `main()`, `run()` orchestration loop (poll→step→advance_turn→draw;
  sim advances only on `EndTurn`), save/load menu wiring to `dcs-protocol` +
  `dcs-core::serialize`.

### Out of scope

- Game rules (stay in core); audio/art pass (Phase 6); scenario editor (Phase 6).

### Key deliverables

- [ ] `docs/specs/presentation-rendering-ui.md` → `dcs-render` + `dcs-app`
- [ ] ADR-0001 (macroquad; Bevy/egui fallback path documented)

### Dependencies

- Phase 3 (core fully playable headless) + Phase 1 (`dcs-protocol` `Command`/
  `GameEvent`, save/load).

### Open questions resolved in this phase

- **DD #10 screen/zoom (OQ-4):** adopt the render spec's recommended default —
  **zoom/pan via `Camera2D`**, with `fit_map` giving an initial whole-map view
  (radius-4 MVP fits one screen by default; full-scope pans/zooms). Confirm
  whether MVP is *locked* to single-screen (data model unaffected either way).
- **egui vs macroquad built-in UI:** macroquad built-in UI for MVP; egui fallback
  if HUD complexity grows (implementation-time render call).

### Exit / Definition of Done

- [ ] `draw_frame` compiles with `state: &GameState` (no `&mut`); `poll_input`
  returns only `Command`s and never calls `step`/`advance_turn` (render spec §8).
- [ ] `screen_to_hex` inverts `hex::to_pixel` exactly (same cube-round as core).
- [ ] Camera pan/zoom works; zoom clamped; HUD in fixed screen space; fog always
  hides undiscovered tiles regardless of zoom.
- [ ] Human can: select, move/found/connect-route (A→B cost preview), patrol/raid,
  end turn; AI turns run synchronously and re-render.
- [ ] CI `cargo tree` confirms `dcs-render`/`dcs-app` depend on `dcs-core`/
  `dcs-protocol` only, never the reverse (ADR-0003).
- [ ] Save/load from the menu round-trips `GameState`; fresh `UiView` (no camera
  persisted) after load.

---

## Phase 5 — MVP integration & playtest

### Goal

Wire everything into the DESIGN-DOC MVP and run a balance tuning pass. Produce
the **first playable build** that is fun and winnable.

### In scope

- Drive the game from `ScenarioConfig::mvp_preset()` (radius 4, **3 players**, 30
  turns, V1 only but all meters built) — DD §17.1.
- Small hex map, fog of war, **3 resources**, **3 unit types**, **1 generic city**
  (specializations data-modeled, enabled as stretch in Phase 6), caravan system,
  basic AI (expand + raid), **V1 oasis dominance** with turn-limit fallback.
- First balance pass on DD §18 OQ-1 numbers (isolation −2, synergy +10%, all
  unit/terrain/building tables, AI weights), informed by playtest.
- Validate the Phase 1 DD #6 raid-order rule under real contention.

### Out of scope

- 4 specializations, V2/V3 as *active* win conditions, diplomacy/tolls, audio,
  full map sizes, scenario editor (Phase 6).

### Key deliverables

- [ ] `mvp_preset()` integration in `dcs-app` menu; human-vs-AI small-map build.
- [ ] Playtest sessions; balance-tables tuned and documented as first-pass → v1.
- [ ] DD #6 raid-order sign-off (or adjustment) from playtest evidence.

### Dependencies

- Phase 4 (visual playability) + Phase 3 (AI/victory) + Phase 2 (gameplay).

### Exit / Definition of Done

- [ ] A human can start the MVP, explore, found a city, lay caravan routes, train
  units, raid, and reach a V1 victory or turn-limit fallback — **winnable and fun**.
- [ ] AI provides real opposition (expands + raids; routes are defended at
  Normal/Hard so the signature mechanic reads as real).
- [ ] Balance numbers recorded as tuned v1 (still in tables, still adjustable).
- [ ] `cargo test -p dcs-core` + a `--headless` N-game smoke run green.

---

## Phase 6 — Full scope / stretch

### Goal

Complete the full game per DD §17.2, enabling deferred content and polish.

### In scope

- 4 city specializations enabled (TradeHub/WellFort/Fortress/ScholarOutpost —
  already data-modeled/specced; flip the MVP feature-flag).
- V2 (Wealth/Prestige) and V3 (Relic Hold) as **active** victory options using the
  already-tracked meters; `VictoryKind::TurnLimit` variant if chosen in Phase 3.
- Route diplomacy / tolls for routes through rival tiles (caravan spec §6.9).
- Full personality × difficulty AI matrix (data, not code — ai spec §10).
- Larger maps (radius 7–9), map symmetry toggle, drought events, tiered buildings.
- Art direction, audio, accessibility full pass (colorblind-safe + shape
  redundancy, text scaling, hotkeys — DD §3.6).
- Scenario editor; difficulty tuning; Bevy fallback path documented (ADR-0001).

### Out of scope

- Anything violating core invariants (pure core, ADR-0003/0008).

### Key deliverables

- [ ] Specializations + V2/V3 enabled and balanced.
- [ ] Diplomacy/tolls, symmetry, drought, tiered buildings.
- [ ] Full map sizes + difficulty tuning; art/audio/accessibility.
- [ ] Optional scenario editor.

### Dependencies

- Phase 5 (validated MVP) — everything before it.

### Exit / Definition of Done

- [ ] Full game per DD §17.2 runs on radius 7–9 maps with all 3 victory types,
  4 specializations, diplomacy, and the full AI matrix.
- [ ] Accessibility pass verified (colorblind-safe + redundant coding + hotkeys).
- [ ] `cargo test -p dcs-core`, `--headless` smoke, and CI `cargo tree` gate green.

---

## Open questions to resolve per phase

| Open question | Source | Phase | Decision / status |
|---|---|---|---|
| PRNG crate choice | ARCH OQ-6 / ADR-0006 | **1** | `nanorand` vs `rand::StdRng`; recommend `nanorand`. RNG state in `GameState`. |
| Save format (json vs postcard/bincode) | ARCH OQ-5 / ADR-0007 | **1** | json dev, postcard ship, behind `serialize`. |
| Contested-raid resolution order | DD #6 / OQ-3 | **1** (rule) → **5** (sign-off) | First-come in actor order (turn-engine §6.6); validated in playtest. |
| Hex-grid-math spec | specs index / ADR-0005 | **1** | Spec `foundation-hex-grid-math.md` exists under `docs/specs/`; implement `dcs-core::hex` from ARCH §4 + ADR-0005. |
| Scout-can-found vs Founder | DD #4 / OQ-2 | **2** | Recommended default: `Scout` can found (`is_founding_unit`). |
| Route planning through fog | fog spec §6.3 | **2** | Default: `ConnectRoute` rejected if path crosses unexplored tile. |
| Balance numbers (isolation −2, synergy +10%, all tables) | DD #3 / OQ-1 | **2** (enter) → **5** (tune) → **6** | Tunable tables; first pass entered Phase 2, tuned in playtest. |
| V2 prestige formula (generalized vs literal) | victory spec §6.2/§10 | **3** | Keep generalized; literal recoverable via weights=0. |
| `VictoryKind::TurnLimit` variant | victory spec §6.5/§10 | **3** (or **5**) | Reuses `WealthScore` unless UI wants distinct kind. |
| Single-screen vs zoom/pan | DD #10 / OQ-4 | **4** | Default: zoom/pan `Camera2D` + `fit_map`; MVP fits one screen by default. |
| egui vs macroquad built-in UI | render spec §10 / ARCH §14.2.3 | **4** | macroquad built-in for MVP; egui fallback if needed. |

---

## Configurable session length (data-driven scenarios)

The **configurable session length** decision (DD §5.2/§13/§16.2, Open Questions
#5/#9) is honored by `ScenarioConfig` (scenario-config spec): map size, player
count, turn limit, and all three victory thresholds are fields that **scale down**
for small maps / 2 players via `scale_thresholds`. The MVP uses `mvp_preset()`
(radius 4, 3 players, 30 turns, V1 focus); full scope uses `Default` (radius 7, 4
players, 60 turns). Nothing about session length is hard-coded in logic — it is
always read from the scenario, so Phases 5 and 6 merely select a preset/override.

---

## Cross-cutting risks & gates (carried from architecture)

- **Core purity gate:** the CI `cargo tree -p dcs-core` check (ADR-0003) runs
  green at the end of **every** phase — it is the mechanical enforcer of Rule A/B.
- **Save compatibility:** balance-table changes must distinguish breaking vs
  non-breaking; `VersionedSave` migrations stay deterministic (ADR-0007).
- **AI route-competence (DD #7 / OQ-7):** **RESOLVED** — AI route defense decided:
  Easy defends a bit, less reliably; Normal/Hard cover core routes reliably
  (baked into utility weights). Re-validated in Phase 5.
- **Spec gap:** `foundation-hex-grid-math.md` exists under `docs/specs/`; implement
  `dcs-core::hex` in Phase 1 (no longer missing).
