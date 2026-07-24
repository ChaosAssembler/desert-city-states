//! MCP tool definitions for interacting with Desert City States.
//!
//! Each tool wraps a JSON request to the `dcs-app serve` subprocess and
//! returns the formatted response.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::process::GameProcess;
use rmcp::tool;

/// Input for the new_game tool.
#[derive(Deserialize, JsonSchema)]
pub struct NewGameInput {
    /// Scenario name (e.g., "mvp") or scenario config as JSON.
    pub scenario: String,
    /// Optional seed for deterministic generation.
    #[schemars(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// Input for the claim_player tool.
#[derive(Deserialize, JsonSchema)]
pub struct ClaimPlayerInput {
    /// Player ID to claim (0-based).
    pub player_id: u32,
    /// Optional player name.
    #[schemars(skip_serializing_if = "Option::is_none")]
    pub player_name: Option<String>,
}

/// Input for the observe tool.
#[derive(Deserialize, JsonSchema)]
pub struct ObserveInput {
    /// Player ID to observe as (uses claimed player if omitted).
    #[schemars(skip_serializing_if = "Option::is_none")]
    pub player_id: Option<u32>,
    /// Detail level: "full" or "summary" (default: "full").
    #[schemars(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Input for the act tool.
#[derive(Deserialize, JsonSchema)]
pub struct ActInput {
    /// Player ID submitting commands (uses claimed player if omitted).
    #[schemars(skip_serializing_if = "Option::is_none")]
    pub player_id: Option<u32>,
    /// List of commands to execute.
    pub commands: Vec<serde_json::Value>,
}

/// Input for the load_game tool.
#[derive(Deserialize, JsonSchema)]
pub struct LoadGameInput {
    /// File path to load from.
    pub path: String,
}

/// Input for the save_game tool.
#[derive(Deserialize, JsonSchema)]
pub struct SaveGameInput {
    /// File path to save to.
    pub path: String,
}

// ---------------------------------------------------------------------------
// Tool implementation
// ---------------------------------------------------------------------------

/// MCP tools for Desert City States.
#[derive(Clone)]
pub struct DcsTools {
    pub process: GameProcess,
}

impl DcsTools {
    /// Create a new `DcsTools` wrapping an already-spawned game process.
    pub fn new(process: GameProcess) -> Self {
        Self { process }
    }
}

#[tool(tool_box)]
impl DcsTools {
    /// Health check — verify the game server is running.
    #[tool(description = "Check if the game server is responding")]
    pub async fn ping(&self) -> Result<String, String> {
        let request = serde_json::json!({"type": "ping"});
        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to ping server: {e}")),
        }
    }

    /// Create a new game session.
    #[tool(description = "Create a new game. Returns player list, turn number, and map radius.")]
    pub async fn new_game(&self, #[tool(aggr)] input: NewGameInput) -> Result<String, String> {
        let scenario = if serde_json::from_str::<serde_json::Value>(&input.scenario).is_ok() {
            serde_json::from_str(&input.scenario).unwrap()
        } else {
            serde_json::json!(input.scenario)
        };

        let request = serde_json::json!({
            "type": "new_game",
            "scenario": scenario,
            "seed": input.seed
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to create game: {e}")),
        }
    }

    /// Claim a player slot.
    #[tool(description = "Claim a player slot to control. Must be called after new_game.")]
    pub async fn claim_player(
        &self,
        #[tool(aggr)] input: ClaimPlayerInput,
    ) -> Result<String, String> {
        let request = serde_json::json!({
            "type": "claim_player",
            "player_id": input.player_id,
            "player_name": input.player_name
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to claim player: {e}")),
        }
    }

    /// Observe the current game state (fog-of-war filtered).
    #[tool(description = "Get the current game state observation. Includes resources, cities, units, routes, and legal actions. Filtered by fog of war for the specified player.")]
    pub async fn observe(&self, #[tool(aggr)] input: ObserveInput) -> Result<String, String> {
        let request = serde_json::json!({
            "type": "observe",
            "player_id": input.player_id,
            "detail": input.detail
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to observe: {e}")),
        }
    }

    /// Submit commands for the current turn.
    #[tool(description = "Submit commands for the current turn. Commands include: EndTurn, MoveUnit, FoundCity, TrainUnit, Build, ConnectRoute, Patrol, Garrison, RaidRoute, RaidCity. Include EndTurn at the end to advance to the next turn.")]
    pub async fn act(&self, #[tool(aggr)] input: ActInput) -> Result<String, String> {
        let request = serde_json::json!({
            "type": "act",
            "player_id": input.player_id,
            "commands": input.commands
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to execute commands: {e}")),
        }
    }

    /// Load a saved game.
    #[tool(description = "Load a previously saved game from a file path.")]
    pub async fn load_game(&self, #[tool(aggr)] input: LoadGameInput) -> Result<String, String> {
        let request = serde_json::json!({
            "type": "load_game",
            "path": input.path
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to load game: {e}")),
        }
    }

    /// Save the current game.
    #[tool(description = "Save the current game state to a file path.")]
    pub async fn save_game(&self, #[tool(aggr)] input: SaveGameInput) -> Result<String, String> {
        let request = serde_json::json!({
            "type": "save_game",
            "path": input.path
        });

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to save game: {e}")),
        }
    }

    /// List available operations and their parameters.
    #[tool(description = "Show help information about available request types and their parameters.")]
    pub async fn help(&self) -> Result<String, String> {
        let request = serde_json::json!({"type": "help"});

        match self.process.send_request(&request).await {
            Ok(response) => Ok(serde_json::to_string_pretty(&response).unwrap_or_default()),
            Err(e) => Err(format!("Failed to get help: {e}")),
        }
    }
}
