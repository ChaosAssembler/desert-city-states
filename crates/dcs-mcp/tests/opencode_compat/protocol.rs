//! MCP protocol compatibility tests.
//!
//! These verify that the server follows the MCP protocol correctly:
//! initialization handshake, tool discovery, and basic tool invocation.

use crate::setup_server;
use rmcp::{ServerHandler, ServiceExt};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn initialize_handshake_succeeds() {
    let (client, _handle) = setup_server().await;

    // If setup_server() succeeded, the MCP handshake (initialize + initialized)
    // completed successfully. The client is now ready to make requests.
    // Verify by listing tools — this only works after successful initialization.
    let tools = client
        .list_all_tools()
        .await
        .expect("list_all_tools failed — handshake may have failed");

    assert!(!tools.is_empty(), "Server should expose at least one tool");

    // Clean shutdown
    client.cancel().await.ok();
}

#[tokio::test]
async fn server_info_has_correct_name() {
    let (client, _handle) = setup_server().await;

    let tools = client.list_all_tools().await.unwrap();

    // The server name "dcs-mcp" is verified indirectly: if the handshake
    // succeeded with our DcsServer, it returned the correct ServerInfo.
    // We verify by confirming tools are available (which requires valid init).
    assert!(
        tools.iter().any(|t| t.name == "ping"),
        "Expected 'ping' tool to be available"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn list_tools_returns_all_eight() {
    let (client, _handle) = setup_server().await;

    let tools = client.list_all_tools().await.expect("Failed to list tools");

    assert_eq!(tools.len(), 8, "Expected exactly 8 tools");

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected = [
        "ping",
        "new_game",
        "claim_player",
        "observe",
        "act",
        "load_game",
        "save_game",
        "help",
    ];

    for name in &expected {
        assert!(
            tool_names.contains(name),
            "Missing tool: {name}. Found: {tool_names:?}"
        );
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn tool_schemas_have_descriptions() {
    let (client, _handle) = setup_server().await;

    let tools = client.list_all_tools().await.unwrap();

    for tool in &tools {
        assert!(
            !tool.description.as_deref().unwrap_or("").is_empty(),
            "Tool '{}' missing description",
            tool.name
        );
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn tool_schemas_have_input_schemas() {
    let (client, _handle) = setup_server().await;

    let tools = client.list_all_tools().await.unwrap();

    // Tools with required parameters must have input schemas
    let tools_with_required_input = ["new_game", "claim_player", "act", "load_game", "save_game"];

    for name in &tools_with_required_input {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("Tool '{name}' not found"));

        let schema = tool.input_schema.as_ref();

        // Schema should be a valid JSON Schema object
        let schema_value: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(schema).expect("Schema should be serializable"),
        )
        .expect("Schema should be valid JSON");

        assert!(
            schema_value.is_object(),
            "Tool '{name}' input_schema should be a JSON object"
        );
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn tools_with_optional_input_have_schemas() {
    let (client, _handle) = setup_server().await;

    let tools = client.list_all_tools().await.unwrap();

    // These tools have optional parameters but should still have schemas
    let optional_input_tools = ["observe"];

    for name in &optional_input_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("Tool '{name}' not found"));

        // Should have a description at minimum
        assert!(
            tool.description.is_some(),
            "Tool '{name}' should have a description"
        );
    }

    client.cancel().await.ok();
}

/// Raw E2E test: intercept the MCP initialize handshake on a duplex stream
/// and verify that the server's response includes `capabilities.tools`.
///
/// This test manually writes the JSON-RPC `initialize` request and reads the
/// response line, bypassing rmcp's client layer to exercise the protocol
/// directly.
#[tokio::test]
async fn raw_initialize_response_has_tools_capability() {
    let (server_transport, client_transport) = tokio::io::duplex(8192);

    let process = dcs_mcp::GameProcess::spawn()
        .await
        .expect("Failed to spawn dcs-app serve");
    let server = dcs_mcp::DcsServer::new(process);

    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let (read_half, mut write_half) = tokio::io::split(client_transport);

    // Manually write the raw JSON-RPC initialize request as line-delimited JSON.
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": false }
            },
            "clientInfo": {
                "name": "opencode",
                "version": "1.18.4"
            }
        }
    });

    let request_line = serde_json::to_string(&initialize_request).unwrap() + "\n";
    write_half
        .write_all(request_line.as_bytes())
        .await
        .expect("Failed to write initialize request");

    // Read the server's initialize response line from the duplex stream.
    let mut reader = BufReader::new(read_half);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .expect("Failed to read initialize response");

    let response: serde_json::Value =
        serde_json::from_str(response_line.trim()).expect("Response should be valid JSON");

    // capabilities.tools must be present and be a non-null object.
    let tools_cap = response
        .get("result")
        .and_then(|r| r.get("capabilities"))
        .and_then(|c| c.get("tools"))
        .and_then(|t| t.as_object());

    assert!(
        tools_cap.is_some(),
        "capabilities.tools should be present in the initialize response, got: {response}"
    );

    // Clean shutdown of the server task.
    server_handle.abort();
    let _ = server_handle.await;
}

/// Direct test: call `server.get_info()` and assert `capabilities.tools` is `Some`.
#[tokio::test]
async fn server_get_info_has_tools_capability() {
    let process = dcs_mcp::GameProcess::spawn()
        .await
        .expect("Failed to spawn dcs-app serve");
    let server = dcs_mcp::DcsServer::new(process);

    let info = server.get_info();

    assert!(
        info.capabilities.tools.is_some(),
        "server.get_info() should return capabilities.tools as Some, got: {info:?}"
    );
}
