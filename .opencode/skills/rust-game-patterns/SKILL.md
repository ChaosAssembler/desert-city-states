---
name: rust-game-patterns
description: Use when implementing game systems in Rust — ECS architecture, hex grids, state machines, resource management, and event-driven patterns
---

# Rust Game Patterns

## Rules

- Model game state as data, not behavior — systems operate on component data
- Use newtype wrappers for domain values (WaterAmount, HexCoord) to prevent mixing
- Prefer enums for finite game states (TileType, UnitType, TurnPhase) over constants
- Keep systems pure where possible — side effects at system boundaries only
- Design for testability — game logic in pure functions, I/O at the edges

## ECS Architecture

- **Components:** Pure data structs — no logic. One concern per component.
- **Systems:** Functions that query components, compute, and write results. Stateless.
- **Resources:** Global state that systems need — turn counter, map, player list.
- **Events:** One-shot signals for things that happen — combat resolved, resource gained.

## Hex Grid Patterns

- Use axial coordinates (q, r) for storage, cube coordinates (q, r, s) for math
- Hex distance: `max(|dq|, |dr|, |ds|)` where `s = -q - r`
- Neighbors: 6 directional offsets, iterate with constant array
- Pathfinding: A* with hex-aware heuristics and movement costs per tile type

## State Machine Patterns

- Turn phases as an enum: Scout → Build → Trade → Combat → End
- Phase transitions validated — can't skip phases, can't go backwards
- Phase-specific systems run only during their phase
- Undo: snapshot state at phase boundary, restore on request

## Resource System Patterns

- Resources as named structs with checked arithmetic (`saturating_add`, `checked_sub`)
- Growth/maintenance calculated per turn from city improvements and population
- Trade routes transfer resources between cities — value depends on distance and safety
- Scarcity emergent from costs exceeding production, not artificial caps
