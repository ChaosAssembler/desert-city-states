# ADR-0007: Save format: serde with version envelope

## Status

Accepted

## Date

2025-01-01

## Context

The game requires save/load, and we want a save to be the same operation as a deterministic replay. The entire `GameState` (core only — never render/app state like camera or UI) must round-trip through serialization. We also need forward-compatible saves so future balance/schema changes don't invalidate old files.

## Decision

Serialize the **entire `GameState`** via `serde`. Use **`serde_json`** for development/debug saves (human-readable) and **`postcard` (or `bincode`)** for shipped, compact saves — both behind a `serialize` abstraction in `dcs-core::serialize`. Wrap the payload in a **`VersionedSave<T> { version: u32, payload: T }`** envelope defined in `dcs-protocol` (`SAVE_VERSION` const). On load: if `version < SAVE_VERSION`, run migration functions; if `> SAVE_VERSION`, reject. This makes the format pluggable and forward-compatible.

## Alternatives

- Single hard-coded format (json or bincode only): rejected — loses either debuggability or compactness, and no versioning.
- No version envelope: rejected — balance-table/schema changes break old saves with no migration path.
- Putting envelope types in `dcs-core`: rejected — the save contract belongs in `dcs-protocol` to keep core clean (see ADR-0003).

## Consequences

- Save and replay share the same purity guarantee; RNG state inside `GameState` keeps saves deterministic.
- Must implement and maintain version migrations for breaking changes (document breaking vs. non-breaking).
- The format choice (json dev vs. postcard/bincode ship) remains tunable behind the abstraction.
