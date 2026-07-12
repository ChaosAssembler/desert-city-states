# 0004-sequential-turns-command-pattern

## Title
Sequential turns with the Command pattern

## Status
Accepted

## Context
Classic 4X feel calls for players and AI acting one after another. Simultaneous resolution introduces fairness/race-condition complexity that adds little value for a turn-based game. We also need a single, auditable mutation path so that saves and replays are trivial and deterministic, and so the AI cannot cheat by reaching into state it shouldn't.

## Decision
Actors act **one at a time** in order (`Player 0 → AI 1 → AI 2 → … → end-of-round bookkeeping → next turn`). All state changes go through a **deterministic resolver** in `dcs-core::turn` that applies `Command`s to `GameState` and returns `GameEvent`s. `Command`s are the **only** mutation entry point; neither human UI nor AI mutate `GameState` directly. Each actor runs order → resolution → income phases, and a final `advance_turn` handles global end-of-round bookkeeping (relic timers, victory check, turn increment, and resetting per-actor move budgets). `advance_turn` does **not** re-apply the per-actor economy — economy (upkeep/yields) is applied during each actor's Income phase as it acts. Illegal commands are rejected via `GameEvent::Rejected` rather than panicking.

## Alternatives
- **Simultaneous turns:** rejected (DD Open Question #6 resolved against it) — added fairness/ordering complexity, no benefit for this design.
- Direct state mutation by actors: rejected — breaks determinism, replay, and prevents AI anti-cheat by construction.

## Consequences
- Clean, debuggable turn phases; the resolver is the single source of mutation truth.
- The AI emits the same `Command` enum through the same resolver as the human — AI cheating is prevented by construction.
- One open question carried forward: resolution order when a contested route tile is targeted by raids from different actors in one turn cycle (proposed: first-come in actor order).
