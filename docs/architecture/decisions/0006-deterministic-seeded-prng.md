# ADR-0006: Deterministic seeded PRNG owned by GameState

## Status

Accepted

## Date

2025-01-01

## Context

Reproducibility, headless testing, and save/replay all require that **all** randomness be fully controlled and serialized. The architecture's Rule A forbids `std::time`, `rand::thread_rng`, system entropy, or nondeterministic hashing anywhere in `dcs-core`. A `(scenario, seed, command-history)` must fully determine a game.

## Decision

All randomness is drawn from a **single seeded PRNG stored inside `GameState`**. Recommended: `nanorand` (small, fast, no_std-friendly) or `rand::rngs::StdRng` seeded with a `u64`. The RNG state is **serialized as part of `GameState`** so a loaded save resumes the exact same random sequence. No `thread_rng` or `std::time` is permitted in core; map gen, combat rolls, AI tie-breaks, and ruin rewards all draw from `state.rng`. Nondeterministic hashing (e.g., iterating `HashMap` with default hasher) is avoided where iteration order matters — use `indexmap`/`FxHashMap` with fixed order.

## Alternatives

- **`thread_rng` / `std::time`-seeded RNG:** rejected — nondeterministic, breaks save/replay and headless reproducibility.
- **Per-call fresh RNG:** rejected — no single source of truth; cannot serialize or replay.

## Consequences

- Saves embed the RNG state, so a loaded game continues deterministically.
- Tests use fixed seeds and can assert exact end states and map-gen reproducibility.
- Replay = re-issue saved command log against a rebuilt `new_game(scenario, seed)`.
- Choice between `nanorand` and `rand::StdRng` remains an open question (recommend `nanorand` for minimal footprint).
