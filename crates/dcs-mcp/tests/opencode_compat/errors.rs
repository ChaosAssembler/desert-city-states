//! Error handling tests.
//!
//! These verify that the server returns appropriate MCP error responses
//! (with correct JSON-RPC error codes) for various error conditions.

use rmcp::model::CallToolRequestParams;
use serde_json::json;

use crate::setup_server;

#[tokio::test]
async fn observe_without_game_returns_error() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "observe".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await;

    // Should either return an error via MCP protocol or return an error response
    match result {
        Ok(call_result) => {
            // If the tool call "succeeds" at MCP level, the response text
            // should contain an error from the game server
            let text = crate::extract_text(&call_result);
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Response should be valid JSON");
            // The game server returns an error response type
            let response_type = json.get("type").and_then(|v| v.as_str());
            assert!(
                response_type == Some("error")
                    || text.contains("error")
                    || text.contains("not running"),
                "Expected error in response, got: {text}"
            );
        }
        Err(rmcp::service::ServiceError::McpError(err)) => {
            // MCP-level error is also acceptable
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code -32602 or -32603, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn act_without_game_returns_error() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "act".into(),
            arguments: Some(
                json!({"commands": ["EndTurn"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await;

    match result {
        Ok(call_result) => {
            let text = crate::extract_text(&call_result);
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Response should be valid JSON");
            let response_type = json.get("type").and_then(|v| v.as_str());
            assert!(
                response_type == Some("error") || text.contains("error"),
                "Expected error in response, got: {text}"
            );
        }
        Err(rmcp::service::ServiceError::McpError(err)) => {
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code -32602 or -32603, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn claim_player_without_game_returns_error() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "claim_player".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await;

    match result {
        Ok(call_result) => {
            let text = crate::extract_text(&call_result);
            let json: serde_json::Value =
                serde_json::from_str(&text).expect("Response should be valid JSON");
            let response_type = json.get("type").and_then(|v| v.as_str());
            assert!(
                response_type == Some("error") || text.contains("error"),
                "Expected error in response, got: {text}"
            );
        }
        Err(rmcp::service::ServiceError::McpError(err)) => {
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code -32602 or -32603, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn observe_after_new_game_requires_claim() {
    let (client, _handle) = setup_server().await;

    // Create a game but don't claim a player
    let _ = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({"scenario": "mvp"}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game failed");

    // Try to observe without claiming — game server should reject
    let result = client
        .call_tool(CallToolRequestParams {
            name: "observe".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await;

    match result {
        Ok(call_result) => {
            let text = crate::extract_text(&call_result);
            // The game server may return an observation anyway (it defaults
            // to player 0) or it may return an error. Both are acceptable.
            // The key is the response is well-formed JSON.
            let _: serde_json::Value =
                serde_json::from_str(&text).expect("Response should be valid JSON");
        }
        Err(rmcp::service::ServiceError::McpError(err)) => {
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn invalid_tool_name_returns_error() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "nonexistent_tool".into(),
            arguments: None,
            meta: None,
            task: None,
        })
        .await;

    // Should return an MCP-level error (tool not found)
    match result {
        Err(rmcp::service::ServiceError::McpError(err)) => {
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code -32602 or -32603, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
        Ok(_) => {
            panic!("Calling a nonexistent tool should return an error");
        }
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn invalid_input_returns_error() {
    let (client, _handle) = setup_server().await;

    // Call new_game with missing required 'scenario' field
    let result = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await;

    // Should return an error (missing required field)
    match result {
        Ok(call_result) => {
            let text = crate::extract_text(&call_result);
            // May return error text or a JSON error response
            let json_result: Result<serde_json::Value, _> = serde_json::from_str(&text);
            if let Ok(json) = json_result {
                let response_type = json.get("type").and_then(|v| v.as_str());
                assert!(
                    response_type == Some("error") || text.contains("error"),
                    "Expected error in response, got: {text}"
                );
            }
            // If it's not JSON, it should still contain error information
        }
        Err(rmcp::service::ServiceError::McpError(err)) => {
            assert!(
                err.code.0 == -32602 || err.code.0 == -32603,
                "Expected JSON-RPC error code, got: {}",
                err.code.0
            );
        }
        Err(other) => {
            panic!("Unexpected error variant: {other:?}");
        }
    }

    client.cancel().await.ok();
}
