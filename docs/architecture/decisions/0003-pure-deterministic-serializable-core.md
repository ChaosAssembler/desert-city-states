# ADR-0003: Pure, deterministic, serde-serializable simulation core, isolated from rendering

## Status
Accepted

## Date

2025-01-01

## Context
Testability, reproducibility, and save/replay are core goals. We must be able to run the *entire* game headlessly in unit/integration tests, fuzz the turn resolver, and reproduce balance bugs from a save — all independently of the renderer. The architecture therefore rests on three hard rules: the core is pure, deterministic, and serializable (Rule A), rendering is a separate layer (Rule B), and a thin glue layer orchestrates (Rule C).

## Decision
`dcs-core` contains **all** game logic and **zero** rendering or engine dependencies. `dcs-render` (macroquad) **reads** core state and **emits** `Command`s only, never mutating `GameState` directly. A thin `dcs-app` crate owns the main loop and dispatches input→core→render. Strict layer separation is enforced: the dependency arrow `dcs-core → (render/app)` must never exist. The `Command`/`GameEvent`/`VersionedSave` contract lives in `dcs-protocol` so core and app share a single reviewable surface.

## Alternatives
- Tightly coupled core+render: rejected — destroys headless testability and engine-swappability.
- Putting the wire/save contract inside `dcs-core`: rejected — would leak serialization-versioning concerns and tempt core to depend on app concerns; a dedicated `dcs-protocol` crate keeps the boundary clean.

## Consequences
- Core is unit-testable headlessly (`cargo test -p dcs-core`) and reproducible from `(scenario, seed, command-history)`.
- Need a CI `cargo tree` gate asserting `dcs-core` has no macroquad/app dependencies (enforces Rules A/B).
- Save, replay, and balance testing all rely on the same purity guarantee.
- The Command/Event/`VersionedSave` contract is centralized in `dcs-protocol`.
