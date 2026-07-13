# ADR-0008: Cargo workspace topology

## Status
Accepted

## Date

2025-01-01

## Context
We need clean, enforceable boundaries between the pure simulation, the rendering layer, the glue/orchestration, and the shared save/command contract. The architecture's rules (pure core, separated rendering, thin glue) are only as strong as the module ownership that backs them.

## Decision
Use a single Cargo **workspace** at the repo root with **four member crates**:

- **`dcs-core`** — the pure, deterministic, serde simulation (hex, map gen, entities, economy, caravan, combat, fog, AI, turn engine, RNG, serialization).
- **`dcs-render`** — the macroquad presentation layer (camera, drawing, HUD, input→`Command`).
- **`dcs-app`** — the glue crate (`main()`, loop, save/load menu, input dispatch). Owns orchestration only, no game rules.
- **`dcs-protocol`** — the shared contract: `Command`/`GameEvent` enums and the `VersionedSave<T>` envelope.

**Dependency rule:** `dcs-core → dcs-protocol` only (never render/app/engine). `dcs-render` and `dcs-app` depend on `dcs-core` + `dcs-protocol`; `dcs-app` additionally depends on `dcs-render`. The arrow `dcs-core → (render/app)` must never exist.

## Alternatives
- A single monolithic crate: rejected — no enforced boundary; the "pure core" Rule A would be unverifiable.
- A separate `dcs-types` crate for entities: rejected — entities are owned by `dcs-core` and `dcs-render` already reads them via that dependency; duplicating adds sync burden. `dcs-protocol` is strictly the wire/save contract.

## Consequences
- Clear module ownership and a single reviewable surface for save compatibility.
- The CI `cargo tree` gate (ADR-0003) can mechanically enforce that `dcs-core` stays engine-free.
- Minor overhead of maintaining a workspace, justified by the enforced layering and engine-swappability.
