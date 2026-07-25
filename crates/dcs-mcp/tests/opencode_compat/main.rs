//! End-to-end tests for dcs-mcp MCP server compatibility with OpenCode.
//!
//! These tests use an in-memory duplex transport to exercise the full MCP
//! protocol stack (rmcp server -> tool dispatch -> subprocess -> dcs-app serve)
//! without requiring real stdio pipes.

mod errors;
mod protocol;
mod tools;

use rmcp::{
    ClientHandler, ServiceExt,
    model::ClientInfo,
    service::{RoleClient, RunningService},
};
use serde_json::Value;

/// Minimal client handler for testing.
#[derive(Debug, Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

/// Set up a DcsServer connected to a test client via in-memory duplex transport.
///
/// Returns the connected client service and a JoinHandle for the server task.
/// The server spawns `dcs-app serve` as a subprocess (may be slow on first run).
pub(crate) async fn setup_server() -> (
    RunningService<RoleClient, TestClient>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_transport, client_transport) = tokio::io::duplex(8192);

    let process = dcs_mcp::GameProcess::spawn()
        .await
        .expect("Failed to spawn dcs-app serve");
    let server = dcs_mcp::DcsServer::new(process);

    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient
        .serve(client_transport)
        .await
        .expect("Failed to create MCP client");

    (client, server_handle)
}

/// Extract the first text content from a CallToolResult.
pub(crate) fn extract_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("Expected text content in tool result")
}

/// Parse the text content of a CallToolResult as JSON.
pub(crate) fn extract_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = extract_text(result);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("Failed to parse tool result as JSON: {e}\nRaw text: {text}"))
}
