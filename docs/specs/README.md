# Desert City States — Technical Specs

> **Phase 3 (per-system specs) — Foundation + Gameplay + Behavior & Presentation groups**
> Source of truth: `docs/design/Desert-City-States.md` + `docs/architecture/ARCHITECTURE.md` + `docs/architecture/decisions/` (ADRs 0001–0008).

These specs describe the foundation systems that live in `dcs-core` (pure sim)
and `dcs-protocol` (Command/Event/VersionedSave contract). They are binding on
implementation and consistent with the design doc and the 8 ADRs. No `.rs` or
`Cargo.toml` files are created by this phase (specification only).

## Spec Index

| Spec | File | Crate | Implements | Status |
|---|---|---|---|---|
| Core Data Model | [foundation-core-data-model.md](./foundation-core-data-model.md) | dcs-core | DD §5–§9,§13; ARCH §3; ADR-0002/0003 | Draft |
| Hex Grid Math | [foundation-hex-grid-math.md](./foundation-hex-grid-math.md) | dcs-core (`hex`) | DD §5.1; ARCH §4; ADR-0005 | Draft |
| World Generation | [foundation-world-generation.md](./foundation-world-generation.md) | dcs-core (`map`) | DD §5.2–§5.7; ARCH §10; ADR-0006 | Draft |
| Turn Engine | [foundation-turn-engine.md](./foundation-turn-engine.md) | dcs-core (`turn`) + dcs-protocol | DD §4,§16.1; ARCH §5; ADR-0004 | Draft |
| Scenario Config | [foundation-scenario-config.md](./foundation-scenario-config.md) | dcs-core (`scenario`) | DD §5.2,§13,§16.2; ARCH §11 | Draft |
| Save / Load | [foundation-save-load.md](./foundation-save-load.md) | dcs-core (`serialize`) + dcs-protocol | ARCH §7; ADR-0007 | Draft |

### Gameplay group (Phase 3)

| Spec | File | Crate | Implements | Status |
|---|---|---|---|---|
| Resources & Economy | [gameplay-resources-economy.md](./gameplay-resources-economy.md) | dcs-core (`economy`) | DD §6; ARCH §3,§5 | Draft |
| Cities | [gameplay-cities.md](./gameplay-cities.md) | dcs-core (`world`) | DD §7; ARCH §3,§15 | Draft |
| Caravan & Trade Routes | [gameplay-caravan-routes.md](./gameplay-caravan-routes.md) | dcs-core (`caravan`) | DD §8; ARCH §4.3,§5,§15 | Draft |
| Units & Movement | [gameplay-units-movement.md](./gameplay-units-movement.md) | dcs-core (`world`/`hex`) | DD §9; ARCH §4.3,§5 | Draft |
| Combat | [gameplay-combat.md](./gameplay-combat.md) | dcs-core (`combat`) | DD §10; ARCH §5,§16 | Draft |
| Fog of War | [gameplay-fog-of-war.md](./gameplay-fog-of-war.md) | dcs-core (`fog`) | DD §12; ARCH §12 | Draft |

### Behavior & Presentation group (Phase 3)

| Spec | File | Crate | Implements | Status |
| --- | --- | --- | --- | --- |
| AI Opponents | [behavior-ai-opponents.md](./behavior-ai-opponents.md) | dcs-core (`ai`) | DD §11; ARCH §9; ADR-0003/0004/0006 | Draft |
| Victory Conditions | [behavior-victory-conditions.md](./behavior-victory-conditions.md) | dcs-core (`victory`) | DD §13; ARCH §13; ADR-0003/0004/0006 | Draft |
| Rendering & UI | [presentation-rendering-ui.md](./presentation-rendering-ui.md) | dcs-render + dcs-app | DD §3; ARCH §8; ADR-0001/0003/0004/0005 | Draft |

## Cross-cutting invariants (all Phase-3 specs)

- **Pure core:** `dcs-core` depends only on `dcs-protocol` + std/serde/rand. No macroquad/render/app (ADR-0003, ADR-0008).
- **No ECS:** plain data + free functions over `GameState` (ADR-0002).
- **Determinism:** all randomness from `state.rng` (owned by `GameState`, serialized), no `thread_rng`/`std::time` (ADR-0006). `FxHashMap`/`FxHashSet` for any map iterated in save/replay.
- **Axial hex in-core:** pointy-top `(q,r,s=-q-r)`; single auditable `cube_round` (ADR-0005).
- **Sequential turns + Command pattern:** only `Command`s mutate state via the resolver; illegal input → `Rejected`, never panic (ADR-0004).
- **Save = replay:** entire `GameState` (incl. RNG) serializes via `serde` behind a `VersionedSave<T>` envelope (ADR-0007).

## Open questions carried (not resolved here)

- **DD #4 / OQ-2:** Scout-can-found vs dedicated Founder — data model supports both.
- **DD #6 / OQ-3:** Contested-raid resolution order — proposed first-come in actor order.
- **DD #3 / OQ-1:** Balance numbers (isolation −2 Water, synergy +10%) kept as tunable tables.
- **DD #10 / OQ-4:** Single-screen vs zoom/pan — render-only; data model unaffected.
- **Route planning through fog (DD §12):** gameplay-fog-of-war specifies default = reject `ConnectRoute` if path crosses an unexplored tile; alt. possible.
- **ARCH OQ-5/OQ-6:** Ship format (postcard vs bincode) and PRNG crate (nanorand vs StdRng) — abstracted, resolved at impl.
