# ADR-0002: No Entity-Component-System (plain data + functions)

## Status

Accepted

## Date

2025-01-01

## Context

Desert City States is a **turn-based** game on a **small board** (≤ ~271 tiles, tens of entities) where the simulation advances a handful of entities a few times per *turn*, not per *frame*. Determinism and full `serde` serialization of game state are first-class priorities (Rule A). The natural data shape is a single `GameState` aggregate holding typed collections keyed by stable integer IDs, queried by explicit functions.

## Decision

**Do not use an ECS** (e.g., `bevy_ecs`, `specs`, `hecs`). The simulation is implemented with plain Rust data structures (structs/enums) and free functions operating over the `GameState` aggregate. Cross-entity relationships are resolved by stable-ID lookups through helper functions rather than component graphs.

## Alternatives

- **ECS (bevy_ecs/specs/hecs):** shines for thousands of per-frame entities with spatial queries in real-time loops. Its internal archetype storage and allocation order are a determinism/save-compatibility hazard we do not need, and add serialization complexity.
- **Separate `dcs-types` crate for entities:** rejected; entities are owned by `dcs-core` and `dcs-render` already depends on `dcs-core` to read them, so duplicating them adds sync burden with no benefit.

## Consequences

- Simpler, more readable simulation; far easier to make deterministic and serializable.
- No "free" structural scaffolding for very large entity counts — not needed at this board scale (spatial queries use `HashMap<HexCoord, TileId>`).
- The whole model stays in the "plain data + functions" paradigm, reinforcing Rule A.
