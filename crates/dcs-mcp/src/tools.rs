//! MCP tool definitions for interacting with Desert City States.
//!
//! Each tool wraps a JSON request to the `dcs-app serve` subprocess and
//! returns the formatted response.

use schemars::JsonSchema;
use serde::Deserialize;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::tool;
use rmcp::tool_router;

use crate::error::DcsError;

/// Input for the new_game tool.
#[derive(Deserialize, JsonSchema)]
pub struct NewGameInput {
    /// Scenario name (e.g., "mvp") or scenario config as JSON.
    pub scenario: String,
    /// Optional seed for deterministic generation.
    pub seed: Option<u64>,
}

/// Input for the claim_player tool.
#[derive(Deserialize, JsonSchema)]
pub struct ClaimPlayerInput {
    /// Player ID to claim (0-based).
    pub player_id: u32,
    /// Optional player name.
    pub player_name: Option<String>,
}

/// Input for the observe tool.
#[derive(Deserialize, JsonSchema)]
pub struct ObserveInput {
    /// Player ID to observe as (uses claimed player if omitted).
    pub player_id: Option<u32>,
    /// Detail level: "full" or "summary" (default: "full").
    pub detail: Option<String>,
}

/// Input for the act tool.
#[derive(Deserialize, JsonSchema)]
pub struct ActInput {
    /// Player ID submitting commands (uses claimed player if omitted).
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

/// Valid detail levels for the observe tool.
const VALID_DETAIL_LEVELS: &[&str] = &[
    "full",
    "resources",
    "cities",
    "units",
    "routes",
    "map",
    "legal_actions",
    "victory",
];

/// Validate that a detail level string is one of the accepted values.
///
/// Returns `Ok(())` if valid, or `Err(DcsError::InvalidInput)` with a
/// helpful message listing the allowed values.
fn validate_detail_level(detail: &str) -> Result<(), DcsError> {
    if VALID_DETAIL_LEVELS.contains(&detail) {
        Ok(())
    } else {
        Err(DcsError::InvalidInput(format!(
            "Invalid detail level '{}'. Must be one of: {}",
            detail,
            VALID_DETAIL_LEVELS.join(", ")
        )))
    }
}

// ---------------------------------------------------------------------------
// Tool implementation — methods are on DcsServer (defined in lib.rs)
// ---------------------------------------------------------------------------

#[tool_router(vis = "pub")]
impl crate::DcsServer {
    /// Health check — verify the game server is running.
    #[tool(description = "Check if the game server is responding")]
    pub async fn ping(&self) -> Result<String, McpError> {
        let request = serde_json::json!({"type": "ping"});
        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Create a new game session.
    #[tool(description = "Create a new game. Returns player list, turn number, and map radius.")]
    pub async fn new_game(
        &self,
        Parameters(NewGameInput { scenario, seed }): Parameters<NewGameInput>,
    ) -> Result<String, McpError> {
        let scenario = if serde_json::from_str::<serde_json::Value>(&scenario).is_ok() {
            serde_json::from_str(&scenario).unwrap()
        } else {
            serde_json::json!(scenario)
        };

        let request = serde_json::json!({
            "type": "new_game",
            "scenario": scenario,
            "seed": seed
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Claim a player slot.
    #[tool(description = "Claim a player slot to control. Must be called after new_game.")]
    pub async fn claim_player(
        &self,
        Parameters(ClaimPlayerInput {
            player_id,
            player_name,
        }): Parameters<ClaimPlayerInput>,
    ) -> Result<String, McpError> {
        let request = serde_json::json!({
            "type": "claim_player",
            "player_id": player_id,
            "player_name": player_name
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Observe the current game state (fog-of-war filtered).
    #[tool(
        description = "Get the current game state observation. Includes resources, cities, units, routes, and legal actions. Filtered by fog of war for the specified player."
    )]
    pub async fn observe(
        &self,
        Parameters(ObserveInput { player_id, detail }): Parameters<ObserveInput>,
    ) -> Result<String, McpError> {
        // Validate detail level if provided.
        if let Some(ref d) = detail {
            validate_detail_level(d)?;
        }

        let request = serde_json::json!({
            "type": "observe",
            "player_id": player_id,
            "detail": detail
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Submit commands for the current turn.
    #[tool(
        description = "Submit commands for the current turn. Commands include: EndTurn, MoveUnit, FoundCity, TrainUnit, Build, ConnectRoute, Patrol, Garrison, RaidRoute, RaidCity. Include EndTurn at the end to advance to the next turn."
    )]
    pub async fn act(
        &self,
        Parameters(ActInput {
            player_id,
            commands,
        }): Parameters<ActInput>,
    ) -> Result<String, McpError> {
        let request = serde_json::json!({
            "type": "act",
            "player_id": player_id,
            "commands": commands
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Load a saved game.
    #[tool(description = "Load a previously saved game from a file path.")]
    pub async fn load_game(
        &self,
        Parameters(LoadGameInput { path }): Parameters<LoadGameInput>,
    ) -> Result<String, McpError> {
        let request = serde_json::json!({
            "type": "load_game",
            "path": path
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// Save the current game.
    #[tool(description = "Save the current game state to a file path.")]
    pub async fn save_game(
        &self,
        Parameters(SaveGameInput { path }): Parameters<SaveGameInput>,
    ) -> Result<String, McpError> {
        let request = serde_json::json!({
            "type": "save_game",
            "path": path
        });

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }

    /// List available operations and their parameters.
    #[tool(
        description = "Show help information about available request types and their parameters."
    )]
    pub async fn help(&self) -> Result<String, McpError> {
        let request = serde_json::json!({"type": "help"});

        let response = self
            .process
            .send_request(&request)
            .await
            .map_err(DcsError::from)?;
        Ok(serde_json::to_string_pretty(&response).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_detail_levels() {
        let levels = [
            "full",
            "resources",
            "cities",
            "units",
            "routes",
            "map",
            "legal_actions",
            "victory",
        ];
        for level in levels {
            assert!(
                validate_detail_level(level).is_ok(),
                "Expected '{level}' to be a valid detail level"
            );
        }
    }

    #[test]
    fn invalid_detail_level_string() {
        let result = validate_detail_level("invalid");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, DcsError::InvalidInput(_)));
    }

    #[test]
    fn empty_detail_level_string() {
        let result = validate_detail_level("");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            DcsError::InvalidInput(msg) => {
                assert!(msg.contains("Must be one of"));
            }
            other => panic!("Expected InvalidInput, got {:?}", other),
        }
    }
}
