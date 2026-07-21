//! Agent-to-server wire protocol for Desert City-States.
//!
//! The agent interface communicates over **line-delimited JSON on stdin/stdout**.
//! Each line is a single JSON object representing either a [`Request`] (agent →
//! server) or a [`Response`] (server → agent). This module defines every type
//! that crosses that wire.
//!
//! # Design decisions
//!
//! - **Tagged enums** (`#[serde(tag = "type")]`) are used for the top-level
//!   `Request` and `Response` so the JSON object always carries a `"type"` field
//!   that selects the variant.
//! - All types here are **self-contained** — they do not import from `dcs-core`
//!   or `dcs-protocol`. This keeps the wire format decoupled from internal
//!   representations and lets us evolve either side independently.
//! - [`EventOutput`] uses [`serde_json::Value`] for now; it will be tightened
//!   once the event-to-JSON mapping stabilises (wave 3–4).

use serde::{Deserialize, Serialize};

// ===========================================================================
// Top-level request (agent → server)
// ===========================================================================

/// A request from the agent (client) to the game server.
///
/// The `"type"` field in the JSON object selects the variant; any variant-
/// specific data lives alongside it (internally tagged, flattened).
///
/// # Examples
///
/// ```json
/// { "type": "ping" }
/// { "type": "new_game", "scenario": "mvp", "seed": 12345 }
/// { "type": "act", "player_id": 0, "commands": ["EndTurn"] }
/// ```
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Connectivity / keep-alive probe. Server replies with [`Response::Pong`].
    Ping,

    /// Start a new game.
    ///
    /// `scenario` accepts either a preset name (e.g. `"mvp"`) or a filesystem
    /// path to a scenario TOML/JSON. `seed` is optional; when omitted the
    /// server picks one (and reports it in the response).
    NewGame {
        /// Preset name or path to a scenario definition file.
        scenario: ScenarioInput,
        /// Optional PRNG seed. `None` → server generates one.
        #[serde(skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },

    /// Claim a player slot so subsequent commands target that player.
    ClaimPlayer {
        /// The player slot to claim (0-based).
        player_id: u32,
        /// Optional human-readable name for the player.
        #[serde(skip_serializing_if = "Option::is_none")]
        player_name: Option<String>,
    },

    /// Request an observation of the current game state.
    ///
    /// `player_id` scopes the observation (fog of war). When omitted the
    /// server uses the last claimed player. `detail` controls the level of
    /// detail (future extension).
    Observe {
        /// Which player's view to return. `None` → use last claimed.
        #[serde(skip_serializing_if = "Option::is_none")]
        player_id: Option<u32>,
        /// Detail level hint (reserved for future use).
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    /// Submit one or more commands for the current actor's turn.
    ///
    /// Commands are applied in order. The server returns
    /// [`Response::Events`] with the resulting game events.
    Act {
        /// Which player is acting. `None` → use last claimed.
        #[serde(skip_serializing_if = "Option::is_none")]
        player_id: Option<u32>,
        /// Ordered list of commands to execute.
        commands: Vec<CommandInput>,
    },

    /// Load a previously saved game from disk.
    LoadGame {
        /// Filesystem path to the save file.
        path: String,
    },

    /// Save the current game state to disk.
    SaveGame {
        /// Filesystem path to write the save file.
        path: String,
    },

    /// Show help / version information.
    Help,
}

// ===========================================================================
// Top-level response (server → agent)
// ===========================================================================

/// A response from the game server to the agent.
///
/// Every response carries a `"type"` field. Success and error responses are
/// distinct variants; the agent should match on `"type"` to decide how to
/// handle each one.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Reply to [`Request::Ping`].
    Pong,

    /// Acknowledgement that a new game was created.
    GameCreated {
        /// List of all players in the game.
        players: Vec<PlayerInfo>,
        /// Starting turn number (always 1).
        turn: u32,
        /// Hex map radius (for coordinate validation).
        map_radius: u32,
    },

    /// Acknowledgement that a player slot was claimed.
    PlayerClaimed {
        /// The claimed player slot.
        player_id: u32,
        /// The effective player name.
        name: String,
        /// Faction color name for rendering (e.g. "Sand", "Crimson").
        color: String,
    },

    /// A game-state observation (details TBD — wave 3–4).
    Observation {
        /// Placeholder: will carry a structured observation payload.
        #[serde(flatten)]
        _placeholder: serde_json::Value,
    },

    /// Events produced by executing commands (or turn advancement).
    Events {
        /// The turn number after processing.
        turn: u32,
        /// Ordered list of game events.
        events: Vec<EventOutput>,
        /// Victory event if the game ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        victory: Option<VictoryInfo>,
        /// Rejected commands with reasons.
        errors: Vec<String>,
    },

    /// Acknowledgement that a game was loaded from disk.
    GameLoaded {
        /// List of all players in the game.
        players: Vec<PlayerInfo>,
        /// Current turn number.
        turn: u32,
        /// Hex map radius (for coordinate validation).
        map_radius: u32,
    },

    /// Acknowledgement that a game was saved to disk.
    Saved {
        /// The path the file was written to.
        path: String,
    },

    /// Help / version information.
    HelpInfo {
        /// One-line descriptions of available request types.
        available_types: Vec<TypeInfo>,
    },

    /// An error that prevented the request from completing.
    Error {
        /// Machine-readable error code (see `ERR_*` constants).
        code: String,
        /// Human-readable description of what went wrong.
        message: String,
        /// Optional hint suggesting how to fix the problem.
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        /// Echoes the `"type"` from the failed request.
        request_type: String,
    },
}

// ===========================================================================
// Supporting types
// ===========================================================================

/// Player information returned in `GameCreated` and `GameLoaded` responses.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayerInfo {
    /// The player's numeric ID.
    pub player_id: u32,
    /// Display label for the player.
    pub label: String,
}

/// Type information for the help response.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TypeInfo {
    /// The request type name (e.g. "ping", "new_game").
    #[serde(rename = "type")]
    pub type_name: String,
    /// One-line description of what this request type does.
    pub description: String,
}

/// Victory information returned in `Events` responses.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VictoryInfo {
    /// The victory kind (e.g. "WealthScore", "OasisDominance").
    pub kind: String,
    /// The winning player's ID.
    pub winner: u32,
}

/// A scenario identifier — either a built-in preset name or a partial config object.
///
/// Serialized as either a bare JSON string or an object:
///
/// ```json
/// "mvp"
/// { "map_radius": 5, "num_ai_players": 3 }
/// ```
///
/// The handler distinguishes names from objects by checking the variant.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ScenarioInput {
    /// A scenario preset name or filesystem path.
    Name(String),
    /// A partial/full ScenarioConfig object (merged with defaults).
    Config(serde_json::Value),
}

/// A command in the agent wire format.
///
/// Mirrors the core `Command` enum from `dcs-protocol` but uses `u32` IDs
/// and string identifiers so the JSON representation is self-describing
/// and does not depend on internal newtypes.
///
/// Uses serde's default externally-tagged representation:
/// each command is a single-key JSON object where the key is the variant
/// name and the value is the variant's fields (or `{}` for unit variants).
///
/// # Examples
///
/// ```json
/// { "MoveUnit": { "unit": 3, "to": 7 } }
/// { "MoveUnit": { "unit": 3, "to": { "q": 2, "letter_r": -1 } } }
/// { "TrainUnit": { "city": 1, "kind": "Scout" } }
/// "EndTurn"
/// ```
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CommandInput {
    /// Move a unit to a target tile (by ID or hex coordinate).
    MoveUnit {
        /// Unit ID to move.
        unit: u32,
        /// Destination tile.
        to: TileCoord,
    },

    /// Found a new city with a unit on a tile.
    FoundCity {
        /// Unit that founds the city (must be on an oasis).
        unit: u32,
        /// Tile to found on (must be an oasis).
        tile: TileCoord,
    },

    /// Train a new unit in a city.
    TrainUnit {
        /// City ID.
        city: u32,
        /// Unit kind name (e.g. `"Scout"`, `"CaravanGuard"`, `"Raider"`).
        kind: String,
    },

    /// Build a structure in a city.
    Build {
        /// City ID.
        city: u32,
        /// Building kind name (e.g. `"Well"`, `"Market"`, `"Temple"`).
        building: String,
    },

    /// Specialize a city.
    Specialize {
        /// City ID.
        city: u32,
        /// Specialization name (e.g. `"TradeHub"`, `"Fortress"`).
        spec: String,
    },

    /// Connect a caravan route between two cities (endpoints only; path is
    /// auto-computed by the engine).
    ConnectRoute {
        /// Origin city. `None` → let the engine pick based on the destination.
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<u32>,
        /// Destination city.
        to: u32,
    },

    /// Station a unit to patrol a tile (route guard duty).
    Patrol {
        /// Unit to patrol. `None` → use a default available unit.
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<u32>,
        /// Tile to patrol.
        tile: u32,
    },

    /// Garrison a unit inside a city.
    Garrison {
        /// Unit to garrison. `None` → use a default available unit.
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<u32>,
        /// City to garrison in.
        city: u32,
    },

    /// Raid (cut) an enemy caravan route.
    RaidRoute {
        /// Raider unit. `None` → use a default available raider.
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<u32>,
        /// Route ID to raid.
        route: u32,
    },

    /// Raid an enemy city.
    RaidCity {
        /// Raider unit. `None` → use a default available raider.
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<u32>,
        /// City ID to raid.
        city: u32,
    },

    /// End the current actor's turn. No payload.
    EndTurn,
}

/// A tile coordinate in the wire format.
///
/// Accepts either a raw tile ID (`u32`) or an axial hex coordinate
/// `{"q": ..., "letter_r": ...}`. Uses `#[serde(untagged)]` so both forms are
/// transparent in JSON:
///
/// ```json
/// 42
/// { "q": 2, "letter_r": -1 }
/// ```
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum TileCoord {
    /// A tile ID (index into the tile array).
    Id(u32),
    /// An axial hex coordinate.
    Hex {
        /// Axial q coordinate.
        q: i32,
        /// Axial r coordinate (named `letter_r` in JSON to avoid
        /// conflicts with Rust reserved words).
        #[serde(rename = "letter_r")]
        r: i32,
    },
}

/// A game event in the agent wire format.
///
/// This is a thin JSON-friendly wrapper. The `event_type` string identifies
/// the event kind; `data` carries the event-specific payload as raw JSON.
/// This will be tightened into a proper tagged enum once the event-to-JSON
/// mapping stabilises (wave 3–4).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EventOutput {
    /// The event kind name (e.g. `"unit_moved"`, `"city_founded"`).
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event-specific payload. Structure depends on `event_type`.
    #[serde(flatten)]
    pub data: serde_json::Value,
}

// ===========================================================================
// Error code constants
// ===========================================================================

/// The game has not been initialised yet (`NewGame` not called).
pub const ERR_GAME_NOT_INITIALIZED: &str = "game_not_initialized";
/// No player has been claimed yet (`ClaimPlayer` not called).
pub const ERR_NO_PLAYER_CLAIMED: &str = "no_player_claimed";
/// The requested player slot is already taken.
pub const ERR_DUPLICATE_PLAYER: &str = "duplicate_player";
/// The acting player is not the current turn holder.
pub const ERR_NOT_YOUR_TURN: &str = "not_your_turn";
/// The referenced player ID does not exist.
pub const ERR_INVALID_PLAYER: &str = "invalid_player";
/// A command failed validation.
pub const ERR_INVALID_COMMAND: &str = "invalid_command";
/// The `"type"` field in a request was not recognised.
pub const ERR_UNKNOWN_TYPE: &str = "unknown_type";
/// A scenario file could not be parsed.
pub const ERR_SCENARIO_LOAD_FAILED: &str = "scenario_load_failed";
/// An I/O error occurred (file not found, permission denied, etc.).
pub const ERR_IO_FILE: &str = "io_file";
/// A serialisation/deserialisation error occurred.
pub const ERR_SERIALIZE: &str = "serialize";
/// The game has already ended.
pub const ERR_GAME_OVER: &str = "game_over";
/// An unexpected internal error.
pub const ERR_OTHER_UNEXPECTED: &str = "other_unexpected";

// ===========================================================================
// Helper constructors on Response
// ===========================================================================

impl Response {
    /// Create a [`Response::Pong`] reply.
    pub fn pong() -> Self {
        Self::Pong
    }

    /// Create an error response.
    pub fn error(
        code: &str,
        message: impl Into<String>,
        hint: Option<String>,
        request_type: &str,
    ) -> Self {
        Self::Error {
            code: code.to_owned(),
            message: message.into(),
            hint,
            request_type: request_type.to_owned(),
        }
    }

    /// Create a [`Response::GameCreated`] acknowledgement.
    pub fn game_created(players: Vec<PlayerInfo>, turn: u32, map_radius: u32) -> Self {
        Self::GameCreated {
            players,
            turn,
            map_radius,
        }
    }

    /// Create a [`Response::PlayerClaimed`] acknowledgement.
    pub fn player_claimed(player_id: u32, name: String, color: String) -> Self {
        Self::PlayerClaimed {
            player_id,
            name,
            color,
        }
    }

    /// Create a [`Response::Saved`] acknowledgement.
    pub fn saved(path: String) -> Self {
        Self::Saved { path }
    }

    /// Create a [`Response::Events`] response.
    pub fn events(
        turn: u32,
        events: Vec<EventOutput>,
        victory: Option<VictoryInfo>,
        errors: Vec<String>,
    ) -> Self {
        Self::Events {
            turn,
            events,
            victory,
            errors,
        }
    }

    /// Create a [`Response::HelpInfo`] with available request types.
    pub fn help_info() -> Self {
        Self::HelpInfo {
            available_types: vec![
                TypeInfo {
                    type_name: "ping".into(),
                    description: "Check if server is alive".into(),
                },
                TypeInfo {
                    type_name: "new_game".into(),
                    description: "Start a new game".into(),
                },
                TypeInfo {
                    type_name: "claim_player".into(),
                    description: "Claim a player slot".into(),
                },
                TypeInfo {
                    type_name: "observe".into(),
                    description: "Observe game state".into(),
                },
                TypeInfo {
                    type_name: "act".into(),
                    description: "Submit commands for current turn".into(),
                },
                TypeInfo {
                    type_name: "load_game".into(),
                    description: "Load a saved game".into(),
                },
                TypeInfo {
                    type_name: "save_game".into(),
                    description: "Save current game state".into(),
                },
                TypeInfo {
                    type_name: "help".into(),
                    description: "Show this help".into(),
                },
            ],
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Serialization round-trip tests
    // -----------------------------------------------------------------------

    #[test]
    fn ping_round_trip() {
        let req = Request::Ping;
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"type":"ping"}"#);
        let back: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Request::Ping));
    }

    #[test]
    fn pong_round_trip() {
        let resp = Response::pong();
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
        let back: Response = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Response::Pong));
    }

    #[test]
    fn new_game_round_trip() {
        let req = Request::NewGame {
            scenario: ScenarioInput::Name("mvp".into()),
            seed: Some(42),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""type":"new_game""#));
        assert!(json.contains(r#""scenario":"mvp""#));
        assert!(json.contains(r#""seed":42"#));
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::NewGame { scenario, seed } = back {
            match scenario {
                ScenarioInput::Name(name) => assert_eq!(name, "mvp"),
                _ => panic!("expected ScenarioInput::Name"),
            }
            assert_eq!(seed, Some(42));
        } else {
            panic!("expected NewGame");
        }
    }

    #[test]
    fn new_game_without_seed() {
        let json = r#"{"type":"new_game","scenario":"mvp"}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        if let Request::NewGame { seed, .. } = req {
            assert_eq!(seed, None);
        } else {
            panic!("expected NewGame");
        }
    }

    #[test]
    fn claim_player_round_trip() {
        let req = Request::ClaimPlayer {
            player_id: 1,
            player_name: Some("Alice".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""player_id":1"#));
        assert!(json.contains(r#""player_name":"Alice""#));
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::ClaimPlayer { player_id, player_name } = back {
            assert_eq!(player_id, 1);
            assert_eq!(player_name.as_deref(), Some("Alice"));
        } else {
            panic!("expected ClaimPlayer");
        }
    }

    #[test]
    fn act_end_turn_round_trip() {
        let req = Request::Act {
            player_id: None,
            commands: vec![CommandInput::EndTurn],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""EndTurn""#));
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::Act { commands, .. } = back {
            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], CommandInput::EndTurn));
        } else {
            panic!("expected Act");
        }
    }

    #[test]
    fn act_move_unit_with_tile_id() {
        let req = Request::Act {
            player_id: Some(0),
            commands: vec![CommandInput::MoveUnit {
                unit: 3,
                to: TileCoord::Id(7),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""MoveUnit":{"#));
        assert!(json.contains(r#""unit":3"#));
        assert!(json.contains(r#""to":7"#));
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::Act { commands, .. } = back {
            if let CommandInput::MoveUnit { unit, to } = &commands[0] {
                assert_eq!(*unit, 3);
                assert!(matches!(to, TileCoord::Id(7)));
            } else {
                panic!("expected MoveUnit");
            }
        } else {
            panic!("expected Act");
        }
    }

    #[test]
    fn move_unit_with_hex_coord() {
        let json = r#"{"type":"act","commands":[{"MoveUnit":{"unit":1,"to":{"q":2,"letter_r":-1}}}]}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        if let Request::Act { commands, .. } = req {
            if let CommandInput::MoveUnit { to, .. } = &commands[0] {
                assert!(matches!(to, TileCoord::Hex { q: 2, r: -1 }));
            } else {
                panic!("expected MoveUnit");
            }
        } else {
            panic!("expected Act");
        }
    }

    #[test]
    fn train_unit_round_trip() {
        let cmd = CommandInput::TrainUnit {
            city: 0,
            kind: "Scout".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""TrainUnit":{"#));
        assert!(json.contains(r#""kind":"Scout""#));
        let back: CommandInput = serde_json::from_str(&json).unwrap();
        if let CommandInput::TrainUnit { city, kind } = back {
            assert_eq!(city, 0);
            assert_eq!(kind, "Scout");
        } else {
            panic!("expected TrainUnit");
        }
    }

    #[test]
    fn scenario_input_untagged() {
        // Preset name
        let json = r#""mvp""#;
        let input: ScenarioInput = serde_json::from_str(json).unwrap();
        match input {
            ScenarioInput::Name(name) => assert_eq!(name, "mvp"),
            _ => panic!("expected ScenarioInput::Name"),
        }

        // Config object
        let json = r#"{"map_radius": 5, "num_ai_players": 3}"#;
        let input: ScenarioInput = serde_json::from_str(json).unwrap();
        match input {
            ScenarioInput::Config(val) => {
                assert_eq!(val["map_radius"], 5);
                assert_eq!(val["num_ai_players"], 3);
            }
            _ => panic!("expected ScenarioInput::Config"),
        }
    }

    #[test]
    fn tile_coord_id_vs_hex() {
        // u32 ID
        let json = "42";
        let coord: TileCoord = serde_json::from_str(json).unwrap();
        assert!(matches!(coord, TileCoord::Id(42)));

        // Hex object — deserialize with letter_r
        let json = r#"{"q": 3, "letter_r": -2}"#;
        let coord: TileCoord = serde_json::from_str(json).unwrap();
        assert!(matches!(coord, TileCoord::Hex { q: 3, r: -2 }));

        // Hex object — serialize produces letter_r
        let coord = TileCoord::Hex { q: 1, r: -1 };
        let json = serde_json::to_string(&coord).unwrap();
        assert!(json.contains(r#""letter_r":-1"#));
        assert!(!json.contains(r#""r":-1"#));

        // Round-trip
        let back: TileCoord = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, TileCoord::Hex { q: 1, r: -1 }));
    }

    #[test]
    fn event_output_round_trip() {
        let event = EventOutput {
            event_type: "unit_moved".into(),
            data: serde_json::json!({ "unit": 1, "from": 5, "to": 6 }),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"unit_moved""#));
        assert!(json.contains(r#""unit":1"#));
        let back: EventOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_type, "unit_moved");
        assert_eq!(back.data["unit"], 1);
    }

    #[test]
    fn error_response_round_trip() {
        let resp = Response::error(
            ERR_INVALID_COMMAND,
            "Unit 99 does not exist",
            Some("Use `observe` to list valid unit IDs".into()),
            "act",
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""code":"invalid_command""#));
        assert!(json.contains(r#""hint":""#));
        assert!(json.contains(r#""request_type":"act""#));
        let back: Response = serde_json::from_str(&json).unwrap();
        if let Response::Error { code, message, hint, request_type } = back {
            assert_eq!(code, "invalid_command");
            assert_eq!(message, "Unit 99 does not exist");
            assert_eq!(hint.as_deref(), Some("Use `observe` to list valid unit IDs"));
            assert_eq!(request_type, "act");
        } else {
            panic!("expected Error");
        }
    }

    #[test]
    fn game_created_response() {
        let resp = Response::game_created(
            vec![
                PlayerInfo { player_id: 0, label: "Player 1".into() },
                PlayerInfo { player_id: 1, label: "Player 2".into() },
            ],
            1,
            4,
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""players""#));
        assert!(json.contains(r#""turn":1"#));
        assert!(json.contains(r#""map_radius":4"#));
    }

    #[test]
    fn help_info_response() {
        let resp = Response::help_info();
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""available_types""#));
        assert!(json.contains(r#""type":"ping""#));
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn optional_fields_omitted_when_none() {
        let req = Request::Observe {
            player_id: None,
            detail: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        // Optional fields should not appear in JSON when None
        assert!(!json.contains("player_id"));
        assert!(!json.contains("detail"));
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::Observe { player_id, detail } = back {
            assert_eq!(player_id, None);
            assert_eq!(detail, None);
        } else {
            panic!("expected Observe");
        }
    }

    #[test]
    fn commands_vec_in_act() {
        let req = Request::Act {
            player_id: Some(0),
            commands: vec![
                CommandInput::MoveUnit {
                    unit: 1,
                    to: TileCoord::Id(5),
                },
                CommandInput::MoveUnit {
                    unit: 2,
                    to: TileCoord::Hex { q: 0, r: 3 },
                },
                CommandInput::EndTurn,
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        if let Request::Act { commands, .. } = back {
            assert_eq!(commands.len(), 3);
        } else {
            panic!("expected Act");
        }
    }

    #[test]
    fn deserialization_rejects_unknown_type() {
        let json = r#"{"type":"bogus"}"#;
        let result = serde_json::from_str::<Request>(json);
        assert!(result.is_err());
    }

    #[test]
    fn connect_route_with_optional_from() {
        let cmd = CommandInput::ConnectRoute {
            from: None,
            to: 5,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(!json.contains("from"));
        let back: CommandInput = serde_json::from_str(&json).unwrap();
        if let CommandInput::ConnectRoute { from, to } = back {
            assert_eq!(from, None);
            assert_eq!(to, 5);
        } else {
            panic!("expected ConnectRoute");
        }
    }
}
