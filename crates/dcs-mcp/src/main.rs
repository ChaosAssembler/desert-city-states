//! MCP server for Desert City States.
//!
//! Usage: `dcs-mcp`
//!
//! Spawns a `dcs-app serve` subprocess and exposes it as an MCP server
//! over stdio transport. The game server is automatically rebuilt if
//! source code has changed (via `cargo run`).

use anyhow::Result;
use rmcp::transport::io::stdio;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to stderr (stdout is reserved for MCP protocol).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    tracing::info!("Starting dcs-mcp server");

    // Spawn the game server process (auto-rebuilds via cargo run).
    tracing::info!("Spawning dcs-app serve (may rebuild if source changed)...");
    let process = dcs_mcp::GameProcess::spawn().await?;
    tracing::info!("Game server started successfully");

    // Create MCP server and serve on stdio.
    let server = dcs_mcp::DcsServer::new(process);
    let service = server.serve(stdio()).await?;

    tracing::info!("MCP server running, waiting for connections...");
    service.waiting().await?;

    tracing::info!("MCP server shutting down");
    Ok(())
}
