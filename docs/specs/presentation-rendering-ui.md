# Presentation Spec: Rendering & UI

> **Phase:** 4 — Per-system presentation specs (group: Behavior & Presentation)
> **Crate:** `dcs-render` (macroquad) + `dcs-app` (glue)
> **Status:** Draft for review
> **Implements:** DD §3 (Presentation & UI/UX); ARCH §8 (Rendering Integration); ADR-0001 (macroquad), ADR-0003 (render reads core, emits Commands only), ADR-0004 (Command pattern)
> **Reads:** `GameState` (read-only), `Command`/`GameEvent` (from `dcs-protocol`)

---

## 1. Purpose

Specify how `dcs-render` (macroquad, immediate-mode) draws the game from a **read-only
`GameState`** and how `dcs-app` orchestrates the human play loop: poll input → emit
`Command`s → dispatch to the core resolver → re-render from the updated state. The
renderer never mutates `GameState` (ADR-0003, Rule B); all state change flows through
`Command`s into `dcs-core::turn::step` / `advance_turn`. Covers the `Camera2D`
pan/zoom over the hex map, axial-hex → pixel mapping (reusing the in-core hex math),
drawing of tiles / units / cities / routes / fog overlay, the HUD panels, and the
input→`Command` translation (selection, move, found, connect-route, camera, end-turn).
The **recommended default is zoom/pan via `Camera2D`** (DD Open Question #10 carried).

## 2. Scope

**In scope**
- `Renderer` struct + `draw_frame(state, view)`, `poll_input() -> Vec<Command>`, `screen_to_hex`.
- `Camera2D` pan (drag) / zoom (wheel); axial-hex → world pixel (reuse `dcs-core::hex::to_pixel`); world → screen via camera.
- Drawing: tiles (terrain tints), units (icon tokens), cities (pop/spec growth), routes (owner/state-colored polyline, red when Threatened/Severed), fog-of-war overlay (hide undiscovered; enemy-in-fog hidden).
- HUD panels (top bar, left selected, right minimap+alerts, bottom action bar) via macroquad built-in UI; **egui as documented fallback** (ARCH §14.2.3).
- Input → `Command` mapping: click select, right-click action, route-planning A→B with live cost preview, camera drag/zoom, End Turn.
- `dcs-app` main loop (poll → step → advance_turn → draw), fixed draw cadence, event-driven logic.
- **Explicit read-only guarantee** for render.

**Out of scope**
- Any game *rules* / balance — those live in `dcs-core` (the renderer only reads them).
- Map generation, AI, combat, economy — consumed only as data/events.
- Save/load file format (save-load spec) — `dcs-app` may expose menu buttons but the
  serialization lives in core/protocol.

## 3. Responsibilities

- `dcs-render`: turn a `&GameState` + a transient `UiView` into pixels; translate
  pointer/keyboard into `Command`s. **No `&mut GameState`.** Holds only **ephemeral**
  view state (camera position, current selection, hover, route-plan A, alert list) —
  never simulation state.
- `dcs-app`: own `main()`, the loop, input→core→render wiring, and (optionally) save/
  load menu. Holds **no game rules** — only orchestration.
- Both depend on `dcs-core` (to *read* types) and `dcs-protocol` (for `Command`/
  `GameEvent`); `dcs-core` never depends on them (ADR-0003, Rule A/B).

## 4. Data Structures / Additions

```rust
// ---- dcs-render ----
pub struct Renderer {
    pub camera: Camera2D,           // macroquad camera; owns pan offset + zoom
    pub hex_size: f32,              // world px per hex "radius" (pointy-top, hex spec)
    pub ui: UiView,                 // transient interaction state (NOT sim state)
}

/// Transient, per-frame UI/interaction state. Rebuilt each frame; never serialized.
#[derive(Default)]
pub struct UiView {
    pub selected_unit:  Option<UnitId>,
    pub selected_city:  Option<CityId>,
    pub selected_tile:  Option<TileId>,
    pub hovered_tile:   Option<TileId>,
    pub route_plan_a:   Option<CityId>,   // first endpoint chosen in route mode
    pub route_preview:  Option<(Vec<TileId>, i32)>, // safe_route preview + cost
    pub alerts:         Vec<Alert>,        // derived from recent GameEvents
    pub view_player:    PlayerId,          // whose fog we render (the human)
}

pub struct Alert { pub kind: AlertKind, pub text: String }
pub enum AlertKind { RouteThreatened, CityStarving, EnemyNear, RelicHeld, Victory }

// ---- dcs-app ----
pub struct App {
    pub state: GameState,           // owned; mutated ONLY via core resolver
    pub renderer: Renderer,         // read-only consumer
    pub scenario: ScenarioConfig,
}
```

> **Read-only guarantee (ADR-0003):** `draw_frame(&self, state: &GameState, view: &UiView)`
> takes `state` by shared reference. The only `&mut GameState` in the whole program is
> inside `dcs-app`, and it is touched **exclusively** by `core::step` / `core::advance_turn`.
> `poll_input` returns `Command`s; it never calls the resolver itself.

## 5. Key Functions / API

```rust
// ---- dcs-render ----
impl Renderer {
    /// One immediate-mode frame. Reads &GameState only; draws camera, map, entities,
    /// routes, fog overlay, HUD. Never mutates state.
    pub fn draw_frame(&mut self, state: &GameState, view: &UiView);

    /// Convert pointer input this frame into zero or more Commands (selection,
    /// move/found/route/patrol, camera drag/zoom, End Turn). Read-only against state
    /// (calls core read helpers + caravan::preview_cost for cost preview).
    pub fn poll_input(&mut self, state: &GameState) -> Vec<Command>;

    /// Screen (mouse) position -> HexCoord via camera inverse + hex::from_pixel.
    pub fn screen_to_hex(&self, screen: Vec2) -> HexCoord;

    /// Center/zoom the camera so the whole radius-N map fits one viewport (used for
    /// the MVP single-screen fit OR the initial full-scope view).
    pub fn fit_map(&mut self, state: &GameState);
}

// ---- dcs-app ----
impl App {
    pub fn main();
    pub fn run(&mut self);                       // orchestration loop (§6)
    pub fn save(&self, path: &Path) -> Result<()>;   // -> dcs-protocol + core::serialize
    pub fn load(path: &Path) -> Result<GameState>;
}
```

`draw_frame` internally calls small draw helpers (`draw_tile`, `draw_unit`, `draw_city`,
`draw_route`, `draw_fog`, `draw_hud_*`) — each is pure read.

## 6. Algorithms

### 6.1 Coordinate mapping (reuse in-core hex math — ADR-0005)

- World position of a hex: `hex::to_pixel(coord, hex_size)` (pointy-top, ARCH §4.1).
- Screen position: `camera.world_to_screen(world)` (macroquad `Camera2D`).
- Inverse (input): `screen_to_hex(screen) = hex::from_pixel(camera.screen_to_world(screen), hex_size)`
  using the **same** cube-round as core so click↔tile is exact and matches pathfinding.
- **Camera2D pan/zoom:** drag with left/right-button-empty-space updates `camera.offset`;
  wheel updates `camera.zoom` (clamped to `[min_zoom, max_zoom]` so you cannot zoom
  past the map bounds or into a single tile). All world geometry is drawn in camera
  space; HUD is drawn in screen space (fixed).

### 6.2 Draw order (per frame)

1. **Camera transform push.** 2. **Tiles** — filled polygon per `TerrainType` tint
(oasis green-blue, dunes tan, salt flats pale grey, ridges dark, ruins marker). 3.
**Routes** — polyline through `CaravanRoute.path` world points; color by `owner`
(`PlayerColor`) with shape/icon redundancy (ADR: colorblind-safe, DD §3.6); dashed,
turns **red** when `status == Threatened`/`Severed`. 4. **Cities** — grows visually
with `population` / `specialization` icon. 5. **Units** — iconographic tokens
(scout/guard/raider, DD §3.4). 6. **Fog overlay** — see §6.3. 7. **HUD** (screen
space) — see §6.4. 8. **Camera transform pop.**

### 6.3 Fog-of-war overlay (render-only; data from `dcs-core::fog`)

- For tiles **not** in `Player(view_player).discovered`: draw dimmed/blacked-out (no
  terrain detail). Unexplored tiles yield nothing and cannot be clicked for planning.
- **Enemy units** hidden with **NO memory (MVP)** unless `is_unit_visible(view_player, unit)` (current tile discovered) — a unit not currently on a discovered tile is fully hidden with no last-known ghost (fog spec §6.3).
- **Enemy cities/routes** once any of their tiles is seen are drawn as a **MEMORY MARKER** (dimmed/stale) at their remembered location; their dynamic state (e.g., route Active/Threatened/Severed; city population/specialization) is shown **live only while currently observed** — when not currently observed, render shows them as remembered (dimmed/stale, state unknown). Relic-site markers shown once their tile is discovered.
- The renderer calls the **same fog queries** as the AI — it never re-implements
  visibility (ADR-0003: fog logic stays in core).

### 6.4 HUD layout (DD §3.2)

| Region | Content | Source (read-only) |
|---|---|---|
| Top bar | Turn #, Water/Wealth/Influence + per-turn deltas, **victory meters** (oases X/Y, prestige score / target, relic timers) | `state.turn`, `Player.resources`, `state.victory` |
| Left panel | Selected entity details + available actions (Found/Build/Train/Specialize/Connect/Patrol) | `state` lookups by `UiView.selection` |
| Right panel | Minimap (scaled tile overview) + **alerts** (route under attack, city starving, enemy near) | `UiView.alerts` (from `GameEvent`s) |
| Bottom bar | End Turn button, current build/train queue, specialization choice | `state` + `poll_input` |

- **macroquad built-in UI** for MVP (buttons, labels, simple panels). **egui is the
  documented fallback** if HUD complexity outgrows macroquad's immediate-mode widgets
  (ARCH §14.2.3 / ADR-0001) — switching is render-internal; core is untouched.

### 6.5 Input → Command mapping (`poll_input`)

- **Left-click tile:** `screen_to_hex` → if a friendly unit is there, select it
  (`UiView.selected_unit`); else select tile/city. Clicking an owned city with a Scout
  on it could arm `FoundCity`; with a Raider adjacent to an enemy route, arm `RaidRoute`.
- **Right-click / action click:** issue the armed action for the selected unit/city:
  - selected unit → `MoveUnit { unit, to: clicked_tile }` (validated via `validate`).
  - route-plan mode (button in left panel): first city click sets `route_plan_a`;
    second city click calls `caravan::preview_cost` to show cost + path, then confirm →
    `ConnectRoute { from, to }` (auto-route only, DD #2).
  - Caravan Guard selected + route tile clicked → `Patrol { unit, tile }`.
  - Raider selected + enemy city/route clicked → `RaidRoute`/`RaidCity`.
- **Camera:** drag on empty space → pan; wheel → zoom (§6.1). No `Command` emitted.
- **End Turn button** → `Command::EndTurn` (appended to the human's accumulated list).
- **Selection is ephemeral:** `UiView` is reset/rebuilt each frame; nothing about it
  is serialized (save spec §7: never save camera/UI state).

### 6.6 `dcs-app` orchestration loop (ARCH §5.7, event-driven)

```
fn run(&mut self) {
    loop {
        match self.state.current_actor {
            Human => {
                // accumulate commands across frames until EndTurn
                let cmds = self.renderer.poll_input(&self.state);
                self.pending.extend(cmds);
                if self.pending.contains(EndTurn) {
                    let events = core::step(&mut self.state, &self.pending);
                    self.renderer.ui.alerts = derive_alerts(&events);
                    self.pending.clear();
                }
            }
            Ai(..) => {
                let diff = difficulty_of(self.state.current_actor);
                let cmds = ai_plan(&self.state, self.state.current_actor, diff);
                let mut all = cmds; all.push(Command::EndTurn);
                let events = core::step(&mut self.state, &all);
                self.renderer.ui.alerts = derive_alerts(&events);
            }
        }
        // after the last actor acted, the engine ran advance_turn internally
        // (turn-engine §6.3); check for Victory event to stop the loop.
        if let Some(Victory{..}) = last_event { break; }
        self.renderer.draw_frame(&self.state, &self.renderer.ui); // fixed draw cadence
    }
}
```

- **Fixed draw cadence:** `draw_frame` runs every frame (e.g. vsync / `macroquad::next_frame()`);
  the **simulation advances only on `EndTurn`**, not per frame (DD §3 / ARCH §8) — no
  draw call mutates state.
- **Event-driven logic:** alerts, victory meters, and animations are driven by the
  `GameEvent`s returned from `step`/`advance_turn`; the renderer does not poll for
  state *changes* directly beyond reading current state.
- **AI turn is synchronous** inside the loop: `ai_plan` (pure) → `step`; the renderer
  draws the resulting state. No shared mutation.

## 7. Edge Cases / Invariants

- **Read-only:** `draw_frame` / `poll_input` take `&GameState`; the only `&mut` is in
  `dcs-app` and is passed solely to `core::step` / `core::advance_turn`. A `grep`/lint
  CI gate (ARCH §14.2.1) asserts `dcs-render` has **no** path to `&mut GameState`.
- **Camera never exposes hidden info:** panning/zooming does not reveal fog — undiscovered
  tiles are always dimmed regardless of zoom (§6.3).
- **Click precision:** `screen_to_hex` uses the identical cube-round as core pathfinding,
  so a click maps to the same `HexCoord` the resolver will use (ADR-0005 determinism of
  rounding).
- **Illegal input:** `poll_input` validates via `core::turn::validate` before emitting;
  an illegal action is shown as a rejected/disabled UI state, never a panic.
- **Route preview through fog:** `preview_cost` / `ConnectRoute` are rejected if the
  path would cross an undiscovered tile (fog spec §6.3) — the UI shows "scout first".
- **Ephemeral UI:** camera, selection, hover, `route_plan_a` are never serialized; a
  loaded save starts with a fresh `UiView` (save spec §7).
- **Determinism unaffected:** rendering is purely a function of `&GameState`; it draws
  nothing that changes simulation. Replays look identical because state is identical.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `draw_frame(&self, &GameState, &UiView)` compiles with `state: &GameState` (no `&mut`).
- [ ] `poll_input` returns only `Command`s; it never calls `core::step` / `advance_turn`.
- [ ] `screen_to_hex` inverts `hex::to_pixel` exactly (same cube-round as core).
- [ ] Camera pan/zoom updates `Camera2D`; zoom clamped; HUD drawn in fixed screen space.
- [ ] Tiles drawn with terrain tints; routes colored by owner, red when Threatened/Severed.
- [ ] Fog overlay hides undiscovered tiles; enemy units drawn only if `is_unit_visible`; enemy cities/routes drawn if `is_city_visible`/`is_route_visible`.
- [ ] HUD top bar shows turn, resources + deltas, and the three victory meters from `state.victory`.
- [ ] Route-plan mode: A→B shows `preview_cost` (path + Wealth cost) then emits `ConnectRoute`.
- [ ] End Turn button emits `Command::EndTurn`; loop dispatches to `core::step` then re-renders.
- [ ] AI turn: `ai_plan` → `step(&mut state, cmds+EndTurn)` → `draw_frame` (no render mutation).
- [ ] Simulation advances only on `EndTurn`, not per frame (fixed draw cadence).
- [ ] A save/load round-trip restores `GameState` but a *fresh* `UiView` (no camera/UI persisted).
- [ ] CI `cargo tree` / lint confirms `dcs-render` depends on `dcs-core`/`dcs-protocol` only, never the reverse (ADR-0003).

## 9. References

- Design: DD §3 (Presentation & UI/UX) — §3.1 camera/map view, §3.2 HUD layout, §3.3 interaction/route-planning, §3.4 art direction, §3.6 accessibility (colorblind-safe), §18 OQ-10 (single-screen vs zoom/pan).
- Architecture: ARCH §8 (Rendering Integration — camera, coord mapping, drawing, HUD, input, loop style), §2.4 (`Renderer`/`draw_frame`/`poll_input`/`screen_to_hex`), §5.4 (input→Command), §5.7 (orchestration loop), §14.1 (macroquad→Bevy fallback), §14.2.3 (egui fallback), §4.1 (hex `to_pixel`/`from_pixel`).
- ADRs: ADR-0001 (macroquad renderer; Bevy/egui fallbacks), ADR-0003 (render reads core, emits Commands, never mutates — Rule B), ADR-0004 (Commands are the only mutation entry point), ADR-0005 (reuse in-core hex math for exact pixel↔hex round-trip).
- Related specs: `foundation-core-data-model.md` (`GameState`, `PlayerColor`, `Player.discovered`), `foundation-turn-engine.md` (`Command`/`GameEvent`, `step`/`advance_turn`, `validate`), `gameplay-fog-of-war.md` (visibility queries render must use), `gameplay-caravan-routes.md` (`preview_cost`, route `status`/`path`), `behavior-ai-opponents.md` (`ai_plan` consumed by the loop), `behavior-victory-conditions.md` (`state.victory` meters drawn in HUD), `foundation-save-load.md` (UI/camera state never serialized).

## 10. Open Questions (carried, not resolved)

- **DD #10 / OQ-4 (OPEN, recommended default stated):** single-screen-MVP vs
  zoom/pan. **This spec recommends the DEFAULT = zoom/pan via `Camera2D`**, with
  `fit_map` providing an initial whole-map view (so radius-4 MVP still fits one screen
  by default, and full-scope maps pan/zoom). The open question (whether MVP should be
  *locked* to single-screen) is carried for design sign-off; the architecture supports
  both and the data model is unaffected. Either way, fog/visibility is zoom-agnostic.
- **egui vs macroquad built-in UI:** macroquad built-in UI is the MVP choice; egui is a
  documented fallback if HUD complexity grows (ARCH §14.2.3). Not resolved — a render
  decision at implementation time.
- **Animation/transitions:** immediate-mode redraw each frame; tweened combat/route
  animations are a full-scope polish item, not specified here (event-driven only).
- **Colorblind palette exact values:** DD §3.6 mandates shape/icon redundancy + a
  colorblind-safe palette; specific hex values are an art-pass decision.
