# Presentation Spec: Agent Communication Protocol — Phase 4 Agent Interface

> **Phase:** 4 — Per-system presentation specs (group: Behavior & Presentation)
> **Crate:** `dcs-app` (stdin/stdout JSON protocol)
> **Status:** Draft for review
> **Implements:** DD §3 (Presentation); ARCH §8 (Rendering Integration); ADR-0003 (pure core), ADR-0004 (Command pattern)
> **Reads:** `GameState` (read-only), `Command`/`GameEvent` (from `dcs-protocol`)

---

## 1. Objective

Define the **agent communication protocol** — a line-delimited JSON protocol over
stdin/stdout that allows an external agent (human or AI) to control a Desert City
States game session. The agent sends requests on stdin; the server processes them
against the in-memory `GameState` and writes JSON responses to stdout. stderr is
reserved for server logging.

**Success criteria:**

- An agent can start a new game, claim a player, observe state, issue commands, and
  play through to victory using only stdin/stdout JSON messages.
- All game state visible to the player is accurately transmitted, respecting fog-of-war.
- Error responses are helpful, actionable, and include hints for correction.
- The protocol is simple enough to implement with a single `dcs-app --serve` binary
  and any language that can read/write newline-delimited JSON.

**User:** Any external process that wants to play or administer a DCS game —
a Python bot, a web frontend relay, a test harness, or a human using a
terminal REPL. The protocol is the *only* interface for Phase 4 agent play.

## 2. Scope

**In scope**

- Line-delimited JSON over stdin (requests) and stdout (responses).
- Eight request types: `ping`, `new_game`, `claim_player`, `observe`, `act`,
  `load_game`, `save_game`, `help`.
- Response envelopes: success types and a uniform error type with code + hint.
- Fog-of-war filtering of game state in `observe` responses.
- Command validation and event emission for `act` requests.
- Full round processing: after the player's commands, the server automatically
  processes all AI turns and returns the combined event stream.
- Save/load via file paths (the server reads/writes the serialized save file).

**Out of scope**

- HTTP/REST API (planned post-MCP; identical JSON schemas).
- Multi-process concurrency (one game per process).
- MCP protocol compliance — this is a simpler custom protocol.
- Binary output — everything is human-readable JSON.

## 3. Responsibilities

- **Server (`dcs-app --serve`):** own the `GameState`, process requests, validate
  commands, run AI turns, apply fog-of-war filtering, emit responses. On stdin
  EOF, clean up and exit. Never expose raw `GameState` — only filtered observations.
- **Agent:** send well-formed JSON requests on stdin, read JSON responses from
  stdout, and implement the session workflow (new game → claim → observe → act
  loop). The agent is responsible for tracking which player it controls and
  when it is that player's turn.
- **`dcs-core`:** provides the simulation. The protocol layer translates between
  JSON shapes and core types. No game rules live in the protocol layer.

## 4. Transport & Framing

| Aspect | Rule |
|---|---|
| **Transport** | stdin (requests) / stdout (responses), one process per game |
| **Framing** | One JSON object per line. Newline-terminated (`\n`). |
| **Encoding** | UTF-8. No binary frames. |
| **Logging** | stderr only. Agent must not read stderr for game data. |
| **Startup** | Process starts with no game state. Agent must call `new_game` or `load_game` before any other game operation. |
| **Shutdown** | Agent signals end-of-session by closing stdin (EOF). Server cleans up and exits. To restart, agent kills the process and launches a new one. |
| **Restart** | No state persists across process restarts. Agent must call `new_game`/`load_game` on each fresh process. |

**Framing example:**

```
Agent writes to stdin:  {"type":"ping"}\n
Server writes to stdout: {"type":"pong"}\n
```

Each request-response pair is independent. There is no request ID correlation;
responses are returned in the order requests arrive (the protocol is synchronous
and single-threaded).

## 5. Request Envelope

Every request is a JSON object with a required `"type"` field identifying the
operation. Additional fields depend on the request type. All field names are
`snake_case`.

```json
{
  "type": "<request_type>",
  "<field_name>": "<value>",
  ...
}
```

**Valid request types:** `ping`, `new_game`, `claim_player`, `observe`, `act`,
`load_game`, `save_game`, `help`.

An unknown `"type"` value returns an error with code `unknown_type`.

## 6. Response Envelope

Every response is a JSON object with a required `"type"` field. Two categories:

- **Success:** `"type"` matches a known success type (e.g. `"pong"`,
  `"game_created"`, `"observation"`, `"events"`, etc.).
- **Error:** `"type": "error"` with structured error fields (see §7).

The server always produces exactly one response per request.

## 7. Error Response Format

All errors share a uniform shape:

```json
{
  "type": "error",
  "code": "<error_code>",
  "message": "<human-readable description of what went wrong>",
  "hint": "<optional agent-friendly guidance on how to fix the issue>",
  "request_type": "<echo of the original request type>"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | Always `"error"` |
| `code` | string | yes | Machine-readable error code (see §26) |
| `message` | string | yes | What went wrong |
| `hint` | string | no | Actionable guidance for the agent |
| `request_type` | string | yes | Echoes the `"type"` from the failed request |

**Error code enumeration:**

| Code | Meaning | Example hint |
|---|---|---|
| `game_not_initialized` | `new_game` / `load_game` hasn't been called yet | `"Call {\"type\":\"new_game\",\"scenario\":\"mvp_preset\"} to start a new game."` |
| `no_player_claimed` | Must call `claim_player` before `observe` / `act` | `"Call {\"type\":\"claim_player\",\"player_id\":0} to claim a player."` |
| `duplicate_player` | Player already claimed by this agent | `"This player is already claimed. Use a different player_id, or observe with the claimed player."` |
| `not_your_turn` | Not currently your claimed player's turn | `"It is player 1's turn. Wait for TurnAdvanced or call observe to check current_actor."` |
| `invalid_player` | `player_id` does not exist in this game | `"Valid player IDs are 0 and 1. Check the game_created response for available players."` |
| `invalid_command` | Command validation failed | `"MoveUnit requires a valid unit_id and destination tile. The unit 5 does not exist. Use observe to list your units."` |
| `unknown_type` | Request type not recognized | `"Valid types: ping, new_game, claim_player, observe, act, load_game, save_game, help."` |
| `scenario_load_failed` | Scenario name unknown or config validation failed | `"Unknown scenario 'foo'. Available scenarios: mvp_preset, default. Or provide a ScenarioConfig object."` |
| `io_file` | File operation failed (read/write) | `"Could not write to '/tmp/save.bin': permission denied. Check the path is writable."` |
| `serialize` | (De)serialization error | `"Failed to parse save file: invalid version header. The file may be corrupted or from an incompatible version."` |
| `game_over` | Game has ended (Victory was produced) | `"The game is over. Winner: player 1 (WealthScore). Start a new game to play again."` |
| `other_unexpected` | Undocumented internal error | `"An unexpected error occurred. This is a bug — please report it."` |

## 8. Request: `ping`

The simplest request. Verifies the server is alive and responsive.

**Request fields:** none.

**Response type:** `pong`

```json
// Request
{"type": "ping"}

// Response
{"type": "pong"}
```

## 9. Request: `new_game`

Creates a new game state from a scenario configuration.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"new_game"` |
| `scenario` | string or object | yes | Scenario name (`"mvp_preset"`, `"default"`, `""` for default) **or** a partial/full `ScenarioConfig` object that will be merged with defaults |
| `seed` | u64 | no | RNG seed. If omitted, an arbitrary seed is generated. |

**Response type:** `game_created`

```json
{
  "type": "game_created",
  "players": [
    {"player_id": 0, "label": "Player 1"},
    {"player_id": 1, "label": "Player 2"}
  ],
  "turn": 1,
  "map_radius": 4
}
```

| Response field | Type | Description |
|---|---|---|
| `players` | array | List of all players in the game (human and AI). `label` is a display name. |
| `turn` | u32 | Starting turn number (always 1). |
| `map_radius` | u32 | Hex map radius (for coordinate validation). |

**Error codes:** `scenario_load_failed`, `serialize`.

**Scenario as object example:**

```json
{
  "type": "new_game",
  "scenario": {
    "map_radius": 5,
    "num_ai_players": 3,
    "start_water": 15
  },
  "seed": 42
}
```

Fields not specified in the object use default values. Unknown fields are rejected.

## 10. Request: `claim_player`

Claims a player slot for this agent session. Only one player can be claimed per
process. A player that is already claimed by another connection (or by an AI slot)
cannot be claimed.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"claim_player"` |
| `player_id` | u32 | yes | Player ID to claim (from `game_created` response) |
| `player_name` | string | no | Display name. Defaults to empty string. |

**Response type:** `player_claimed`

```json
{
  "type": "player_claimed",
  "player_id": 0,
  "name": "Agent Alice",
  "color": "Sand"
}
```

| Response field | Type | Description |
|---|---|---|
| `player_id` | u32 | The claimed player ID |
| `name` | string | Display name (player_name or default) |
| `color` | string | Faction color name for rendering |

**Error codes:** `invalid_player`, `duplicate_player`, `game_not_initialized`.

## 11. Request: `observe`

Returns the current game state from the perspective of a player, filtered by
fog-of-war. This is the primary way agents read the state of the game.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"observe"` |
| `player_id` | u32 | no | Whose perspective to observe. Defaults to the claimed player. |
| `detail` | string | no | What to return. One of: `"full"`, `"resources"`, `"cities"`, `"units"`, `"routes"`, `"map"`, `"legal_actions"`, `"victory"`. Defaults to `"full"`. |

**Response type:** `observation`

The observation object always includes the header fields; fields not requested
by `detail` are set to `null`.

```json
{
  "type": "observation",
  "turn": 1,
  "player_id": 0,
  "current_phase": "Order",
  "current_actor": 0,
  "is_my_turn": true,
  "resources": { "water": 10, "wealth": 10, "influence": 10 },
  "cities": [ ... ] | null,
  "units": [ ... ] | null,
  "routes": [ ... ] | null,
  "visible_enemies": { ... } | null,
  "tiles": [ ... ] | null,
  "legal_actions": [ ... ] | null,
  "victory": { ... } | null
}
```

**Header fields (always present):**

| Field | Type | Description |
|---|---|---|
| `turn` | u32 | Current turn number |
| `player_id` | u32 | The player whose perspective this is |
| `current_phase` | string | One of `"Order"`, `"Resolution"`, `"Income"`, `"EndOfTurn"` |
| `current_actor` | u32 | Player ID whose turn it currently is |
| `is_my_turn` | bool | `true` if `current_actor == player_id` |

**Detail → field mapping:**

| `detail` value | Fields populated | Fields null |
|---|---|---|
| `"full"` | All fields | None |
| `"resources"` | `resources` | `cities`, `units`, `routes`, `visible_enemies`, `tiles`, `legal_actions`, `victory` |
| `"cities"` | `cities` | `resources`, `units`, `routes`, `visible_enemies`, `tiles`, `legal_actions`, `victory` |
| `"units"` | `units` | `resources`, `cities`, `routes`, `visible_enemies`, `tiles`, `legal_actions`, `victory` |
| `"routes"` | `routes` | `resources`, `cities`, `units`, `visible_enemies`, `tiles`, `legal_actions`, `victory` |
| `"map"` | `tiles` | `resources`, `cities`, `units`, `routes`, `visible_enemies`, `legal_actions`, `victory` |
| `"legal_actions"` | `legal_actions` | `resources`, `cities`, `units`, `routes`, `visible_enemies`, `tiles`, `victory` |
| `"victory"` | `victory` | `resources`, `cities`, `units`, `routes`, `visible_enemies`, `tiles`, `legal_actions` |

**Error codes:** `game_not_initialized`, `no_player_claimed`, `invalid_player`.

## 12. Observation: `resources` Field

```json
{
  "water": 10,
  "wealth": 10,
  "influence": 10
}
```

The observing player's empire-wide stockpile. See §19 for `ResourceStack`.

## 13. Observation: `cities` Field

Array of city objects visible to the player (own cities always; enemy cities if
discovered — see §17).

```json
{
  "city_id": 0,
  "name": null,
  "tile": {
    "coord": {"q": 3, "letter_r": -2},
    "terrain": "Oasis",
    "improvement": []
  },
  "population": 1,
  "specialization": null,
  "buildings": [],
  "production_queue": [],
  "water_yield": 2,
  "wealth_yield": 1,
  "route_slots": 2,
  "route_count": 0,
  "is_isolated": true,
  "stockpiles": {"water": 0, "wealth": 0, "influence": 0}
}
```

| Field | Type | Description |
|---|---|---|
| `city_id` | u32 | Unique city identifier |
| `name` | string or null | City name (if named) |
| `tile` | object | The tile the city is on: `coord`, `terrain`, `improvement` |
| `population` | u32 | Current population |
| `specialization` | string or null | City specialization or null |
| `buildings` | array of strings | Built buildings |
| `production_queue` | array | Current production items |
| `water_yield` | u32 | Water produced per turn |
| `wealth_yield` | u32 | Wealth produced per turn |
| `route_slots` | u32 | Maximum trade route capacity |
| `route_count` | u32 | Currently active routes from this city |
| `is_isolated` | bool | Whether the city has no active routes |
| `stockpiles` | object | City-level resource stockpiles |

## 14. Observation: `units` Field

Array of unit objects owned by the observing player.

```json
{
  "unit_id": 0,
  "name": null,
  "kind": "Scout",
  "tile_coord": {"q": 4, "letter_r": -2},
  "hp": 3,
  "moves_left": 2,
  "max_moves": 2,
  "ability": "None",
  "attack": 0,
  "defense": 1,
  "upkeep": 0,
  "current_action": null
}
```

| Field | Type | Description |
|---|---|---|
| `unit_id` | u32 | Unique unit identifier |
| `name` | string or null | Unit name (if named) |
| `kind` | string | Unit kind: `"Scout"`, `"CaravanGuard"`, `"Raider"` |
| `tile_coord` | HexCoord | Current position |
| `hp` | u32 | Current hit points |
| `moves_left` | u32 | Moves remaining this turn |
| `max_moves` | u32 | Maximum moves per turn |
| `ability` | string | Current ability state |
| `attack` | u32 | Attack strength |
| `defense` | u32 | Defense strength |
| `upkeep` | u32 | Water upkeep cost |
| `current_action` | string or null | Current action (e.g. `"Patrolling"`, `"Garrisoned"`) |

## 15. Observation: `routes` Field

Array of the observing player's caravan routes.

```json
{
  "route_id": 0,
  "endpoint_cities": [0, 1],
  "endpoint_tiles": [
    {"q": 3, "letter_r": -2},
    {"q": -1, "letter_r": 0}
  ],
  "status": "Active",
  "length": 4,
  "upkeep": 1,
  "water_transfer": null,
  "threat_status": null
}
```

| Field | Type | Description |
|---|---|---|
| `route_id` | u32 | Unique route identifier |
| `endpoint_cities` | [u32, u32] | City IDs at each end |
| `endpoint_tiles` | [HexCoord, HexCoord] | Tile coordinates of endpoints |
| `status` | string | `"Active"`, `"Threatened"`, or `"Severed"` |
| `length` | u32 | Route length in tiles |
| `upkeep` | u32 | Water upkeep per turn |
| `water_transfer` | object or null | Water being transferred (if any) |
| `threat_status` | string or null | Threat details (if threatened) |

## 16. Observation: `visible_enemies` Field

Enemy entities visible to the observing player through fog-of-war.

```json
{
  "cities": [
    {
      "city_id": 2,
      "owner": 1,
      "tile": {"coord": {"q": -2, "letter_r": 3}, "terrain": "Oasis"},
      "population": 2,
      "specialization": null,
      "buildings": ["Well"],
      "route_count": 1,
      "is_isolated": false,
      "stale": false,
      "last_seen_turn": null
    }
  ],
  "units": [
    {
      "unit_id": 5,
      "owner": 1,
      "kind": "Raider",
      "tile_coord": {"q": -1, "letter_r": 2},
      "hp": 3,
      "attack": 3,
      "defense": 1,
      "stale": false,
      "last_seen_turn": null
    }
  ],
  "routes": [
    {
      "route_id": 3,
      "owner": 1,
      "endpoint_cities": [2, 4],
      "status": "Active",
      "length": 3,
      "stale": false,
      "last_seen_turn": null
    }
  ]
}
```

**Fog-of-war rules for enemy data (see §17 for full details):**

- **Enemy units:** only returned if currently visible (their tile is in the
  player's discovered set). Always fresh (`stale: false`).
- **Enemy cities:** returned if ever discovered. Once seen, always shown even if
  no longer in sight range. May be `stale: true` with `last_seen_turn` if the
  city is no longer currently visible.
- **Enemy routes:** returned only if any path tile is discovered. The path itself
  is **never** returned — only endpoint city IDs and status. May be stale.

## 17. Observation: `tiles` Field & Fog-of-War

Array of tiles in the player's discovered set. Undiscovered tiles are never
returned.

```json
{
  "coord": {"q": 4, "letter_r": -2},
  "terrain": "Dunes",
  "improvement": [],
  "units": ["Scout"],
  "city": null,
  "route": [],
  "in_fog": false,
  "relic": false
}
```

| Field | Type | Description |
|---|---|---|
| `coord` | HexCoord | Tile coordinates |
| `terrain` | string | Terrain type: `"Oasis"`, `"Dunes"`, `"SaltFlats"`, `"Ridges"`, `"Ruins"` |
| `improvement` | array | Buildings/improvements on this tile |
| `units` | array or null | Unit IDs/names on this tile (own units always; enemy only if visible) |
| `city` | object or null | City on this tile (if any) |
| `route` | array or null | Route IDs passing through this tile |
| `in_fog` | bool | `false` for all returned tiles (they are all discovered). Included for consistency with the rendering model. |
| `relic` | bool | Whether this tile contains a relic site |

**Fog-of-war visibility model (§12.3 of gameplay-fog-of-war.md):**

| Entity | Visibility rule |
|---|---|
| **Own tiles** | Always visible if in `Player::discovered` set |
| **Own units** | Always visible |
| **Own cities** | Always visible |
| **Enemy units** | Visible only if their current tile is in the observing player's discovered set |
| **Enemy cities** | Visible once **any** tile adjacent to the city is discovered; remains visible permanently afterward (static visibility) |
| **Enemy routes** | Visible if **any** path tile is discovered; path tiles themselves are never revealed, only endpoint city IDs and status |
| **Relics** | Visible on tiles in the discovered set |

**Discovery set:** The set of tiles within sight range of the player's units,
cities, and special buildings. See `gameplay-fog-of-war.md` §4 for radii
(Scout: 3, City: 2, Watchtower: 2, etc.).

## 18. Observation: `legal_actions` Field

What the player can do right now. Only populated when `is_my_turn` is true.

```json
[
  {
    "action_type": "found_city",
    "source_id": 0,
    "description": "Found a city on this Oasis tile",
    "cost": "10 influence",
    "target_type": "Oasis tile",
    "constraints": "Unit must be on an Oasis; no city already present"
  },
  {
    "action_type": "move_unit",
    "source_id": 0,
    "description": "Move Scout to adjacent tile",
    "cost": "1 move",
    "target_type": "Adjacent tile",
    "constraints": "Tile must be in discovered set; not blocked by enemy unit"
  }
]
```

| Field | Type | Description |
|---|---|---|
| `action_type` | string | Command type this maps to |
| `source_id` | u32 | ID of the entity performing the action |
| `description` | string | Human-readable description |
| `cost` | string | Resource cost description |
| `target_type` | string | What the action targets |
| `constraints` | string | Conditions that must be met |

## 19. Observation: `victory` Field

The victory tracker state for the observing player.

```json
{
  "oases_controlled_v1": 2,
  "total_oases": 6,
  "prestige_score": 0,
  "holds_relics": false,
  "turn_limit_remaining": 20
}
```

| Field | Type | Description |
|---|---|---|
| `oases_controlled_v1` | u32 | Oases currently controlled by this player |
| `total_oases` | u32 | Total oases on the map |
| `prestige_score` | u32 | Accumulated prestige |
| `holds_relics` | bool | Whether the player currently holds any relics |
| `turn_limit_remaining` | u32 | Turns remaining before the turn-limit victory check |

## 20. Request: `act`

Submits commands for the current player. The server validates and resolves the
commands, then automatically processes all AI turns for the remainder of the
round. The response contains the full event stream from the entire round.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"act"` |
| `player_id` | u32 | no | Defaults to the claimed player. Must be the `current_actor`. |
| `commands` | array | yes | Array of Command objects (see §21) |

**Processing semantics:**

1. The server validates **all** commands in the array.
2. Invalid commands are **rejected** with a `Rejected` event (included in the
   response's `errors` array). The entire batch is rolled back — no valid
   commands from the batch are applied if any command in the batch is invalid.
3. If all commands are valid, they are applied in order.
4. An implicit `EndTurn` is appended if the last command is not `EndTurn`.
5. After the player's turn resolves, the server runs each AI player's turn
   automatically (calling `ai_plan` + `EndTurn` for each).
6. If a `Victory` event occurs at any point during resolution, processing stops
   immediately — no further AI turns are processed.
7. The response contains **all** events from the entire round (player's turn
   + all AI turns).

**Response type:** `events`

```json
{
  "type": "events",
  "turn": 1,
  "events": [
    {"type": "UnitMoved", "unit": 0, "from": {"q": 4, "letter_r": -2}, "to": {"q": 4, "letter_r": -3}},
    {"type": "Revealed", "player": 0, "tiles": [{"q": 4, "letter_r": -3}]},
    {"type": "Income", "player": 0, "water": 2, "wealth": 1, "influence": 0},
    {"type": "TurnAdvanced", "turn": 2}
  ],
  "victory": {"kind": "WealthScore", "winner": 1},
  "errors": []
}
```

| Response field | Type | Description |
|---|---|---|
| `turn` | u32 | The turn number after processing |
| `events` | array | All `GameEvent` objects from the round |
| `victory` | object or null | Victory event if the game ended |
| `errors` | array | Rejected commands with reasons |

**Command batch validation rule:**
If **any** command in the `commands` array fails validation, the **entire batch
is rolled back**. No partial application occurs. The response includes the
`events` from any pre-validation events (if any) and the `errors` array lists
each rejected command with its `RejectReason`.

**EndTurn handling:**
The server automatically appends `EndTurn` if it is not the last command in the
array. The agent does **not** need to explicitly include `EndTurn`, but may do
so for clarity. If `EndTurn` appears mid-array, only commands before it are
processed; commands after it are ignored with a warning.

**AI turn processing:**
After the player's `EndTurn`, the server iterates through remaining players in
actor order. For each AI player, `ai_plan` generates commands and `EndTurn` is
appended. All events from AI turns are included in the response. If a `Victory`
event occurs during an AI turn, processing stops and the `victory` field is set.

**Error codes:** `game_not_initialized`, `no_player_claimed`, `not_your_turn`,
`invalid_command`.

## 21. Request: `load_game`

Loads a previously saved game from a file path.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"load_game"` |
| `path` | string | yes | Path to the serialized save file |

**Response type:** `game_loaded` (same shape as `game_created`)

```json
{
  "type": "game_loaded",
  "players": [
    {"player_id": 0, "label": "Player 1"},
    {"player_id": 1, "label": "Player 2"}
  ],
  "turn": 5,
  "map_radius": 4
}
```

**Error codes:** `io_file`, `serialize`, `other_unexpected`.

## 22. Request: `save_game`

Saves the current game state to a file path.

**Request fields:**

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | yes | `"save_game"` |
| `path` | string | yes | Path to write the serialized save file |

**Response type:** `saved`

```json
{
  "type": "saved",
  "path": "/tmp/game_save.bin"
}
```

**Error codes:** `game_not_initialized`, `io_file`, `serialize`.

## 23. Request: `help`

Returns metadata about available request types.

**Request fields:** none.

**Response type:** `help_info`

```json
{
  "type": "help_info",
  "available_types": [
    {"type": "ping", "description": "Check if server is alive"},
    {"type": "new_game", "description": "Start a new game"},
    {"type": "claim_player", "description": "Claim a player slot"},
    {"type": "observe", "description": "Observe game state"},
    {"type": "act", "description": "Submit commands for current turn"},
    {"type": "load_game", "description": "Load a saved game"},
    {"type": "save_game", "description": "Save current game state"},
    {"type": "help", "description": "Show this help"}
  ]
}
```

## 24. Command Input Shape

Commands are submitted in the `commands` array of an `act` request. Each
command is a single-variant JSON object: the key is the variant name, the value
is the variant's fields (or `{}` for unit variants).

**General pattern:**

```json
{"<VariantName>": {<fields>}}
```

**Examples:**

```json
// EndTurn (unit variant)
{"EndTurn": {}}

// MoveUnit
{"MoveUnit": {"unit": 0, "to": {"q": 4, "letter_r": -3}}}

// FoundCity
{"FoundCity": {"unit": 0, "tile": {"q": 4, "letter_r": -3}}}

// TrainUnit
{"TrainUnit": {"city": 0, "kind": "Scout"}}

// Build
{"Build": {"city": 0, "building": "Well"}}

// Specialize
{"Specialize": {"city": 0, "spec": "TradeHub"}}

// ConnectRoute
{"ConnectRoute": {"from": null, "to": 0}}

// Patrol
{"Patrol": {"unit": 0, "tile": {"q": 4, "letter_r": -3}}}

// Garrison
{"Garrison": {"unit": 0, "city": 0}}

// RaidRoute
{"RaidRoute": {"unit": 0, "route": 0}}

// RaidCity
{"RaidCity": {"unit": 0, "city": 2}}
```

**Serialization note:** These map directly to Rust `serde` enum serialization.
Rust `serde` serializes enums in externally-tagged representation by default:
`{"VariantName": {"field": value, ...}}`. The JSON schema follows this
convention exactly.

## 25. Shared ID Types

All IDs are `u32` integers. They are stable within a game session and survive
serialization.

| Type | JSON type | Description |
|---|---|---|
| `TileId` | u32 | Tile identifier |
| `UnitId` | u32 | Unit identifier |
| `CityId` | u32 | City identifier |
| `RouteId` | u32 | Trade route identifier |
| `PlayerId` | u32 | Player identifier |
| `RelicId` | u32 | Relic identifier |

## 26. Shared Data Types

### 26.1 `HexCoord`

Hexagonal grid coordinates in axial format (pointy-top).

```json
{"q": 3, "letter_r": -2}
```

| Field | Type | Description |
|---|---|---|
| `q` | i32 | Axial q coordinate |
| `letter_r` | i32 | Axial r coordinate (named `letter_r` to avoid JSON key conflicts with Rust reserved words) |

**Constraint:** `s = -q - r`. For a map of radius `R`, valid coordinates
satisfy `|q| + |r| + |s| <= 2 * R`.

### 26.2 `ResourceStack`

```json
{"water": 10, "wealth": 5, "influence": 3}
```

| Field | Type | Description |
|---|---|---|
| `water` | u32 | Water resources |
| `wealth` | u32 | Wealth resources |
| `influence` | u32 | Influence resources |

All three fields are always present in the JSON representation.

## 27. Command Enum (Rust → JSON)

The authoritative Rust enum definitions and their JSON serialization:

| Variant | Fields | JSON |
|---|---|---|
| `MoveUnit` | `unit: UnitId, to: TileId` | `{"MoveUnit": {"unit": <u32>, "to": <u32>}}` |
| `FoundCity` | `unit: UnitId, tile: TileId` | `{"FoundCity": {"unit": <u32>, "tile": <u32>}}` |
| `TrainUnit` | `city: CityId, kind: UnitKind` | `{"TrainUnit": {"city": <u32>, "kind": "<string>"}}` |
| `Build` | `city: CityId, building: BuildingKind` | `{"Build": {"city": <u32>, "building": "<string>"}}` |
| `Specialize` | `city: CityId, spec: CitySpecialization` | `{"Specialize": {"city": <u32>, "spec": "<string>"}}` |
| `ConnectRoute` | `from: Option<CityId>, to: CityId` | `{"ConnectRoute": {"from": <u32|null>, "to": <u32>}}` |
| `Patrol` | `unit: Option<UnitId>, tile: TileId` | `{"Patrol": {"unit": <u32|null>, "tile": <u32>}}` |
| `Garrison` | `unit: Option<UnitId>, city: CityId` | `{"Garrison": {"unit": <u32|null>, "city": <u32>}}` |
| `RaidRoute` | `unit: Option<UnitId>, route: RouteId` | `{"RaidRoute": {"unit": <u32|null>, "route": <u32>}}` |
| `RaidCity` | `unit: Option<UnitId>, city: CityId` | `{"RaidCity": {"unit": <u32|null>, "city": <u32>}}` |
| `EndTurn` | (none) | `{"EndTurn": {}}` |

**Note on `to` in `MoveUnit`:** The field is serialized as a `TileId` (u32) in
the canonical Rust serialization. However, the protocol also accepts `HexCoord`
objects as tile references for agent convenience — the server resolves the
coordinate to a `TileId` internally. The `observe` response returns `HexCoord`
for tile positions; the `act` request accepts either form.

## 28. GameEvent Enum (Rust → JSON)

Events returned in the `events` array of an `act` response:

| Variant | Fields | JSON |
|---|---|---|
| `UnitMoved` | `unit: UnitId, from: TileId, to: TileId` | `{"type":"UnitMoved","unit":<u32>,"from":<u32>,"to":<u32>}` |
| `CityFounded` | `city: CityId, owner: PlayerId, tile: TileId` | `{"type":"CityFounded","city":<u32>,"owner":<u32>,"tile":<u32>}` |
| `UnitTrained` | `unit: UnitId, city: CityId` | `{"type":"UnitTrained","unit":<u32>,"city":<u32>}` |
| `Built` | `city: CityId, building: BuildingKind` | `{"type":"Built","city":<u32>,"building":"<string>"}` |
| `Specialized` | `city: CityId, spec: CitySpecialization` | `{"type":"Specialized","city":<u32>,"spec":"<string>"}` |
| `RouteCreated` | `route: RouteId, from: CityId, to: CityId, path: Vec<TileId>` | `{"type":"RouteCreated","route":<u32>,"from":<u32>,"to":<u32>,"path":[<u32>,...]}` |
| `RouteStatusChanged` | `route: RouteId, old_status: RouteStatus, status: RouteStatus` | `{"type":"RouteStatusChanged","route":<u32>,"old_status":"<string>","status":"<string>"}` |
| `UnitPatrolled` | `unit: UnitId, tile: TileId` | `{"type":"UnitPatrolled","unit":<u32>,"tile":<u32>}` |
| `UnitGarrisoned` | `unit: UnitId, city: CityId` | `{"type":"UnitGarrisoned","unit":<u32>,"city":<u32>}` |
| `RouteRaided` | `route: RouteId, by: PlayerId, severed: bool` | `{"type":"RouteRaided","route":<u32>,"by":<u32>,"severed":true}` |
| `CityRaided` | `city: CityId, by: PlayerId, pop_lost: u32` | `{"type":"CityRaided","city":<u32>,"by":<u32>,"pop_lost":1}` |
| `Combat` | `attacker: UnitId, defender: UnitId, attacker_loss: u32, defender_loss: u32, retreated: bool` | `{"type":"Combat","attacker":<u32>,"defender":<u32>,"attacker_loss":1,"defender_loss":2,"retreated":false}` |
| `Income` | `player: PlayerId, water: u32, wealth: u32, influence: i32` | `{"type":"Income","player":<u32>,"water":2,"wealth":1,"influence":0}` |
| `Grown` | `city: CityId, population: u32` | `{"type":"Grown","city":<u32>,"population":2}` |
| `Starved` | `city: CityId, population: u32` | `{"type":"Starved","city":<u32>,"population":0}` |
| `Revealed` | `player: PlayerId, tiles: Vec<TileId>` | `{"type":"Revealed","player":<u32>,"tiles":[<u32>,...]}` |
| `Victory` | `kind: VictoryKind, winner: PlayerId` | `{"type":"Victory","kind":"<string>","winner":<u32>}` |
| `TurnAdvanced` | `turn: u32` | `{"type":"TurnAdvanced","turn":2}` |
| `Rejected` | `command: Command, reason: RejectReason` | `{"type":"Rejected","command":{...},"reason":"<string>"}` |
| `Warn` | `message: String` | `{"type":"Warn","message":"<string>"}` |

**Note:** Event objects always include a `"type"` field with the variant name.
The field names in the JSON match the Rust struct field names (snake_case).

## 29. Supporting Enums

These enums are serialized as their string variant names in JSON:

### `UnitKind`

| Variant | JSON |
|---|---|
| `Scout` | `"Scout"` |
| `CaravanGuard` | `"CaravanGuard"` |
| `Raider` | `"Raider"` |

### `BuildingKind`

| Variant | JSON |
|---|---|
| `Well` | `"Well"` |
| `Market` | `"Market"` |
| `Granary` | `"Granary"` |
| `Watchtower` | `"Watchtower"` |
| `Caravanserai` | `"Caravanserai"` |
| `Temple` | `"Temple"` |

### `CitySpecialization`

| Variant | JSON |
|---|---|
| `TradeHub` | `"TradeHub"` |
| `WellFort` | `"WellFort"` |
| `Fortress` | `"Fortress"` |
| `ScholarOutpost` | `"ScholarOutpost"` |

### `RouteStatus`

| Variant | JSON |
|---|---|
| `Active` | `"Active"` |
| `Threatened` | `"Threatened"` |
| `Severed` | `"Severed"` |

### `TerrainType`

| Variant | JSON |
|---|---|
| `Oasis` | `"Oasis"` |
| `Dunes` | `"Dunes"` |
| `SaltFlats` | `"SaltFlats"` |
| `Ridges` | `"Ridges"` |
| `Ruins` | `"Ruins"` |

### `VictoryKind`

| Variant | JSON |
|---|---|
| `OasisDominance` | `"OasisDominance"` |
| `WealthScore` | `"WealthScore"` |
| `RelicHold` | `"RelicHold"` |
| `TurnLimit` | `"TurnLimit"` |

### `RejectReason`

| Variant | JSON |
|---|---|
| `NotYourUnit` | `"NotYourUnit"` |
| `OffMap` | `"OffMap"` |
| `NotOasis` | `"NotOasis"` |
| `IllegalTarget` | `"IllegalTarget"` |
| `NoResource` | `"NoResource"` |
| `Blocked` | `"Blocked"` |
| `OutOfMoves` | `"OutOfMoves"` |
| `NotYourTurn` | `"NotYourTurn"` |
| `InvalidState` | `"InvalidState"` |

## 30. Agent Workflow Guidance

A typical agent session follows this pattern:

### Step 1: Initialize the game

```json
→ {"type": "new_game", "scenario": "mvp_preset", "seed": 42}
← {"type": "game_created", "players": [{"player_id": 0, "label": "Player 1"}, ...], "turn": 1, "map_radius": 4}
```

### Step 2: Claim a player

```json
→ {"type": "claim_player", "player_id": 0, "player_name": "MyBot"}
← {"type": "player_claimed", "player_id": 0, "name": "MyBot", "color": "Sand"}
```

### Step 3: Observe the initial state

```json
→ {"type": "observe", "detail": "full"}
← {"type": "observation", "turn": 1, "player_id": 0, "is_my_turn": true, ...}
```

### Step 4: Play loop — observe → act → observe

```json
// Observe to check it's your turn and see legal actions
→ {"type": "observe", "detail": "legal_actions"}
← {"type": "observation", "is_my_turn": true, "legal_actions": [...], ...}

// Act: submit commands
→ {"type": "act", "commands": [{"MoveUnit": {"unit": 0, "to": {"q": 4, "letter_r": -3}}}, {"EndTurn": {}}]}
← {"type": "events", "turn": 1, "events": [...], "errors": []}

// Observe again — check new state, it should be turn 2
→ {"type": "observe", "detail": "full"}
← {"type": "observation", "turn": 2, "is_my_turn": true, ...}
```

### Step 5: Save/Load for long sessions

```json
// Save
→ {"type": "save_game", "path": "/tmp/game_turn5.bin"}
← {"type": "saved", "path": "/tmp/game_turn5.bin"}

// Load (in a new process)
→ {"type": "load_game", "path": "/tmp/game_turn5.bin"}
← {"type": "game_loaded", "players": [...], "turn": 5, "map_radius": 4}
```

### Step 6: Detect game end

After an `act` response, check the `victory` field:

```json
← {"type": "events", "turn": 15, "events": [...], "victory": {"kind": "OasisDominance", "winner": 0}, "errors": []}
```

Once `victory` is non-null, the game is over. Further `observe` or `act` calls
will return an error with code `game_over`.

### Retry on error

When receiving an error response, read the `hint` field for corrective action.
For example, if `invalid_command` is returned with a hint about tile coordinates,
adjust the coordinates and retry.

## 31. Edge Cases & Boundaries

### 31.1 Multiple `claim_player` calls

Only **one** player can be claimed per process. A second `claim_player` call
with a different `player_id` returns `duplicate_player`. To switch players, the
agent must restart the process and load a save.

### 31.2 `observe` before `new_game`

Returns error code `game_not_initialized` with hint:
`"Call {\"type\":\"new_game\",\"scenario\":\"mvp_preset\"} to start a new game."`

### 31.3 `act` before `claim_player`

Returns error code `no_player_claimed` with hint:
`"Call {\"type\":\"claim_player\",\"player_id\":0} to claim a player."`

### 31.4 `act` for a player that is not `current_actor`

Returns error code `not_your_turn` with hint:
`"It is player 1's turn. Wait for TurnAdvanced or call observe to check current_actor."`

### 31.5 Invalid commands in a batch

If the `commands` array contains **any** invalid command, the **entire batch
is rolled back**. No valid commands from the batch are applied. The response
includes:
- `events`: any events generated before the validation failure (usually empty).
- `errors`: an array of `Rejected` events, one per invalid command, each with
  a `reason` (`RejectReason`) and the original command.

Example error response:

```json
{
  "type": "error",
  "code": "invalid_command",
  "message": "Command batch rejected: 1 invalid command(s).",
  "hint": "Check the errors array for details. All commands were rolled back.",
  "request_type": "act"
}
```

### 31.6 `save_game` and `load_game` — resumed sessions

- **Save:** Serializes the full `GameState` (including RNG state) to the
  specified path using the `VersionedSave<T>` envelope (see
  `foundation-save-load.md`). The file format is opaque to the agent.
- **Load:** Reads and deserializes the save file. The `game_loaded` response
  confirms the loaded state (turn, players, map radius). After loading, the
  agent must call `claim_player` before `observe`/`act`.
- **Cross-version compatibility:** The `serialize` error code is returned if
  the save file is from an incompatible version or is corrupted.

### 31.7 `EndTurn` placement

- If `EndTurn` is the **last** command: all commands before it are processed,
  then the turn ends.
- If `EndTurn` is **not present**: the server appends it implicitly after
  processing all commands.
- If `EndTurn` appears **mid-array**: only commands before `EndTurn` are
  processed; commands after it are ignored with a `Warn` event.

### 31.8 After a `Victory` event

Once a `Victory` event is produced:
- The game state is frozen.
- Further `act` calls return error code `game_over` with hint:
  `"The game is over. Winner: player 1 (WealthScore). Start a new game to play again."`
- `observe` calls still succeed and return the final state (useful for
  post-game analysis).
- `save_game` still works (to save the final state).

### 31.9 AI turn processing after `EndTurn`

After the player submits `EndTurn`:
1. The server determines the actor sequence for remaining players (excluding
   the player who just ended their turn).
2. For each AI player in actor order:
   a. `current_actor` is updated to the AI player.
   b. `ai_plan` generates a command list.
   c. `EndTurn` is appended.
   d. Commands are resolved.
   e. All events from the AI turn are collected.
3. After all AI players have acted:
   a. `TurnAdvanced` is emitted with the new turn number.
   b. `current_actor` is set to the first player of the next turn.
4. If a `Victory` event occurs during any AI turn, steps 2.d+ for subsequent
   AI players are skipped. The `victory` field is set and processing stops.

### 31.10 `observe` with `player_id` for other players

The `observe` request accepts an optional `player_id`. If specified, the
observation is returned from **that** player's fog-of-war perspective. This is
useful for debugging or multi-agent setups. The server returns the same
structure but filtered through the specified player's discovered set.

### 31.11 Unknown request type

Returns error code `unknown_type` with hint:
`"Valid types: ping, new_game, claim_player, observe, act, load_game, save_game, help."`

### 31.12 Malformed JSON

If the server cannot parse a line as JSON, it returns error code
`other_unexpected` with a message describing the parse error. The server
continues processing subsequent lines.

### 31.13 Empty stdin / EOF

When stdin reaches EOF (the agent closes the input stream), the server
performs cleanup (flushing any pending writes, closing file handles) and
exits with status 0. No response is produced for the EOF itself.

## 32. Sample Request-Response Sequence

Complete example of a game session start:

```
Agent → {"type":"help"}
Server → {"type":"help_info","available_types":[{"type":"ping","description":"..."},{"type":"new_game","description":"..."},...]}

Agent → {"type":"new_game","scenario":"mvp_preset","seed":42}
Server → {"type":"game_created","players":[{"player_id":0,"label":"Player 1"},{"player_id":1,"label":"Player 2"}],"turn":1,"map_radius":4}

Agent → {"type":"claim_player","player_id":0}
Server → {"type":"player_claimed","player_id":0,"name":"Player 1","color":"Sand"}

Agent → {"type":"observe","detail":"resources"}
Server → {"type":"observation","turn":1,"player_id":0,"current_actor":0,"is_my_turn":true,"resources":{"water":10,"wealth":10,"influence":10},"cities":null,"units":null,"routes":null,"visible_enemies":null,"tiles":null,"legal_actions":null,"victory":null}

Agent → {"type":"observe","detail":"full"}
Server → {"type":"observation","turn":1,"player_id":0,"current_phase":"Order","current_actor":0,"is_my_turn":true,"resources":{"water":10,"wealth":10,"influence":10},"cities":[{"city_id":0,"name":null,"tile":{"coord":{"q":3,"letter_r":-2},"terrain":"Oasis","improvement":[]},"population":1,"specialization":null,"buildings":[],"production_queue":[],"water_yield":2,"wealth_yield":1,"route_slots":2,"route_count":0,"is_isolated":true,"stockpiles":{"water":0,"wealth":0,"influence":0}}],"units":[{"unit_id":0,"name":null,"kind":"Scout","tile_coord":{"q":4,"letter_r":-2},"hp":3,"moves_left":2,"max_moves":2,"ability":"None","attack":0,"defense":1,"upkeep":0,"current_action":null},{"unit_id":1,"name":null,"kind":"CaravanGuard","tile_coord":{"q":1,"letter_r":-1},"hp":3,"moves_left":2,"max_moves":2,"ability":"None","attack":2,"defense":2,"upkeep":1,"current_action":null}],"routes":[],"visible_enemies":null,"tiles":[{"coord":{"q":4,"letter_r":-2},"terrain":"Dunes","improvement":[],"units":["Scout"],"city":null,"route":[],"in_fog":false,"relic":false},...],"legal_actions":[{"action_type":"found_city","source_id":0,"description":"Found a city on this Oasis tile","cost":"10 influence","target_type":"Oasis tile","constraints":"Unit must be on an Oasis"}],"victory":{"oases_controlled_v1":1,"total_oases":6,"prestige_score":0,"holds_relics":false,"turn_limit_remaining":20}}

Agent → {"type":"act","player_id":0,"commands":[{"MoveUnit":{"unit":0,"to":{"q":5,"letter_r":-2}}}]}
Server → {"type":"error","code":"invalid_command","message":"Destination tile (5,-2) is not on map for radius 4.","hint":"Choose a tile where |q|+|r|+|s| <= 8. For radius 4, valid tiles satisfy |q|<=4, |r|<=4, |s|<=4 where s=-q-r. The nearest valid tiles from (4,-2) are (4,-3) and (5,-3). Did you mean (4,-3)?","request_type":"act"}

Agent → {"type":"act","player_id":0,"commands":[{"MoveUnit":{"unit":0,"to":{"q":4,"letter_r":-3}}},{"EndTurn":{}}]}
Server → {"type":"events","turn":1,"events":[{"type":"UnitMoved","unit":0,"from":{"q":4,"letter_r":-2},"to":{"q":4,"letter_r":-3}},{"type":"Revealed","player":0,"tiles":[{"q":4,"letter_r":-3}]},{"type":"Income","player":0,"water":2,"wealth":1,"influence":0},{"type":"TurnAdvanced","turn":2}],"victory":null,"errors":[]}
```

## 33. Error Message Quality Standards

Error responses are designed to be **helpful and actionable** for agents. Each
error includes:

1. **`message`:** What went wrong, specific to this request.
2. **`hint`:** How to fix it, including concrete examples when possible.

**Good hint examples:**

| Error | Hint |
|---|---|
| `invalid_command` — off-map tile | `"Choose a tile where |q|+|r|+|s| <= 8. The nearest valid oases are (4,-3) and (5,-3). Did you mean (4,-3)?"` |
| `invalid_command` — unit doesn't exist | `"Unit 5 does not exist. Use observe with detail='units' to list your units."` |
| `not_your_turn` | `"It is player 1's turn. Wait for TurnAdvanced or call observe to check current_actor."` |
| `game_not_initialized` | `"Call {\"type\":\"new_game\",\"scenario\":\"mvp_preset\"} to start a new game."` |
| `duplicate_player` | `"Player 0 is already claimed. Use a different player_id, or observe with the claimed player."` |
| `unknown_type` | `"Valid types: ping, new_game, claim_player, observe, act, load_game, save_game, help."` |

## 34. Testing Strategy

### Unit tests

- **Request parsing:** Each request type has a deserialization test confirming
  valid JSON maps to the expected Rust struct.
- **Response serialization:** Each response type has a serialization test
  confirming the Rust struct produces the expected JSON shape.
- **Command validation:** Each `Command` variant has validation tests for both
  valid and invalid inputs (e.g., off-map tile, non-existent unit, wrong owner).
- **Error formatting:** Each error code produces the correct JSON shape with
  all required fields.

### Integration tests

- **Full round-trip:** Start game → claim → observe → act → verify events.
- **Fog-of-war filtering:** Verify enemy units are hidden, enemy cities are
  shown, and route paths are never revealed.
- **AI turn processing:** After `EndTurn`, verify AI events appear and
  `current_actor` advances correctly.
- **Victory handling:** Verify processing stops after `Victory` and subsequent
  `act` calls return `game_over`.
- **Save/load round-trip:** Save a game, load it in a new process, verify
  state matches.

### Test harness

Tests use `dcs-app --serve` as a subprocess, writing to its stdin and reading
from its stdout. Each test case is a sequence of request-response pairs.
The test harness must handle async I/O (the server writes responses
asynchronously after receiving requests).

### Coverage targets

- 100% coverage of all request types and error codes.
- 100% coverage of all `Command` variants.
- 100% coverage of all `GameEvent` variants in the `events` response.
- Fog-of-war tests cover: own units, enemy units, enemy cities (discovered vs
  undiscovered), routes (path hidden, endpoints shown).

## 35. Open Questions

- **`MoveUnit` `to` field:** Currently the `to` field in `MoveUnit` accepts
  either a `TileId` (u32) or a `HexCoord` object. Should we standardize on
  one form, or keep the dual acceptance for agent convenience?
- **Multi-process concurrent agents:** Not in scope for Phase 4, but the
  protocol design should not preclude future support for multiple agents
  connecting to the same game (e.g., via HTTP).
- **Replay/undo:** Not in scope. The agent cannot undo commands. Should we
  add a `revert_to_turn` request in a future phase?
- **Event filtering:** Should agents be able to request only specific event
  types in `act` responses (e.g., `"filter": ["UnitMoved", "Victory"]`)?
  Current design returns all events.
- **`observe` for multiple detail fields:** Should `detail` accept an array
  (e.g., `["resources", "units"]`) to avoid multiple round-trips?
