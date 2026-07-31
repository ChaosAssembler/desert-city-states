---
name: game-tester
description: Use when playing or testing Desert City States through the MCP server (`dcs-mcp`) — teaches MCP tool usage, error recovery, playing strategy, and testing methodology
---

# Game Tester

Play and test Desert City States through the MCP server (`dcs-mcp`). Communicate with the game using MCP tools and the JSON-line protocol.

## Rules

- Use `ping` to verify the server is reachable before sending game commands.
- Check every response for `"type":"error"` before proceeding.
- Always `claim_player` after creating a new game.
- End turns explicitly with an `EndTurn` command — the server does not auto-advance.
- Read all response data — the server may produce multiple response lines per request.

## Workflow

### Start

Use `ping` to verify the server is running before sending game commands.

### Play loop

1. Call `new_game` with desired scenario and seed.
2. Call `claim_player` to join as a player.
3. Enter the turn loop: `observe` the game state, call `act` with commands, `observe` again to confirm state changed. Repeat until the game ends, a bug is encountered, or the test scenario completes.
4. Always include an `EndTurn` command at the end of your `act` request when ready to advance.

### Error recovery

Parse the `hint` field from every error response — the server provides actionable guidance. Retry with backoff before concluding the server is broken. If the session is still alive after an unexpected state, log the discrepancy and continue.

## MCP Tool Reference

Interact with the game server through these MCP tools:

| MCP Tool | Purpose |
|----------|---------|
| `ping` | Health check — verify server is running |
| `new_game` | Create a new game session |
| `claim_player` | Claim a player slot in the game |
| `observe` | Get game state observation (fog-of-war filtered) |
| `act` | Submit commands for the current turn |
| `load_game` | Load a saved game |
| `save_game` | Save the current game |
| `help` | List available operations |

## Protocol Reference

The full JSON protocol is specified in `docs/specs/presentation-agent-protocol.md`. That document defines all request types (`ping`, `new_game`, `claim_player`, `observe`, `act`, `save_game`, `load_game`, `help`), all response types, the `Command` input shapes (11 variants), `GameEvent` shapes, error codes with hints, and fog-of-war filtering rules.

This skill teaches how to use the protocol, not what it looks like — refer to the spec for exact JSON shapes and field definitions.

## Send Commands

Commands are sent inside the `commands` array of an `act` request. The protocol spec defines all 11 command variants with their exact JSON shapes (`MoveUnit`, `FoundCity`, `TrainUnit`, `Build`, `Specialize`, `ConnectRoute`, `Patrol`, `Garrison`, `RaidRoute`, `RaidCity`, `EndTurn`).

Key principles:
- Commands are validated server-side. Invalid commands return a `Rejected` event in the response — the valid ones in the same batch still execute.
- Always place `EndTurn` as the last command when ready to advance the turn.
- The server runs all AI turns automatically after your `EndTurn`.

Example of a single command batch via `act`:
```json
{
  "type": "act",
  "commands": [
    {"MoveUnit": {"unit": 0, "to": {"q": 4, "letter_r": -3}}},
    {"EndTurn": {}}
  ]
}
```

## Common Error Recovery

The protocol spec defines a complete error code catalog. The most common ones you will encounter:

| Error Code | What It Means | Recovery Action |
|------------|---------------|-----------------|
| `game_not_initialized` | No game exists yet | Call `new_game` first |
| `no_player_claimed` | No player seat claimed | Call `claim_player` with a valid `player_id` |
| `not_your_turn` | Another player is acting | Wait — `observe` to check whose turn it is |
| `invalid_command` | Command failed validation | Read the `hint` field and adjust accordingly |

Every error response includes a `hint` field with specific, actionable guidance. Read it before retrying.

## Playing Strategy

### Early Game (Turns 1–5)

- Move your Scout toward the nearest Oasis tile.
- Found a city on the Oasis — the Scout is consumed to become the city.
- Explore surrounding tiles to find Ridges (wealth) and Ruins (influence).
- Train additional Scouts for exploration or CaravanGuards for defense.

### Mid Game (Turns 6–15)

- Build improvements in cities to boost resource yields.
- Establish caravan routes between your cities for bonus resource transfer and synergy.
- Assign CaravanGuards to protect routes near enemy territory.
- Specialize cities once population is high enough.
- Scout enemy positions — fog of war hides unknown tiles.

### Late Game (Turns 16+)

- Push toward your victory condition: control oases (V1), accumulate prestige (V2), or hold relics (V3).
- Raid enemy routes to disrupt their economy.
- Defend your own routes — losing routes hurts your resource income.
- Monitor the turn limit — if no one wins by then, the highest prestige score determines the winner.

### General Tips

- Water sustains cities and units. Running out causes starvation damage.
- Routes transfer resources between connected cities — keep them active for bonus income.
- CaravanGuards have higher health and can protect route tiles.
- City raids reduce population. A city with low population can be captured.
- Fog of war hides undiscovered tiles. Enemy units are only visible when in your sight range. Enemy cities remain visible once discovered.
- Use the `hint` field in error responses to learn valid play — it often tells you exactly what you should do instead.

## Testing Methodology

### Map Exploration
- Move Scouts systematically to reveal the map.
- Confirm fog is correctly applied: undiscovered tiles are absent from `observe` responses, enemy units only visible in sight range, enemy cities visible once discovered.
- Verify that the `is_my_turn` and `current_actor` fields change correctly after each `act`.

### Economy Testing
- Build each building type and verify resource yields update in the next `observe`.
- Establish routes between your cities and verify wealth/water transfer.
- Check that route status (`Active`, `Threatened`, `Severed`) changes under the right conditions.
- Deplete resources to verify cap behavior and starvation mechanics.

### Combat Testing
- Test raids on routes: verify the route status cascades (Active → Threatened → Severed).
- Test raids on cities: verify population reduction and capture at zero.
- Verify defender retaliation works as expected.
- Confirm combat events include correct `attacker_loss` and `defender_loss` values.

### Victory Testing
- Push V1 (OasisDominance): claim enough oases and verify the victory event fires.
- Push V2 (WealthScore): accumulate prestige and verify the correct winner.
- Push V3 (RelicHold): hold all relics for the required number of turns.
- Verify turn-limit fallback: let the game run to the turn limit and confirm the highest prestige player wins.

### Edge Case Testing
- Send an `act` with an empty `commands` array.
- Send an `act` without an `EndTurn` command.
- Claim an invalid `player_id`.
- Send malformed JSON.
- Load a save file, act, then save again — verify round-trip.
- Send multiple commands in one batch where some are valid and some are not.

### Regression Testing
- When a bug is reported, reproduce it by following the exact command sequence from the bug report.
- Vary parameters (seed, scenario, player count) to find the minimal reproduction case.
- After a fix is applied, verify the same sequence now produces the correct behavior.

## Conventions

- The `observe` response is the source of truth — always read it before acting.
- IDs are assigned by the server — never fabricate them.
- Send commands in dependency order within a single `act` request (e.g., move then found).
- Read all response data from `observe` — the server may produce multiple lines per request.
- If `observe` returns empty or stale data, wait briefly and retry — the server may still be processing.
- Keep session state (player_id, city IDs, unit IDs) in memory between turns.
