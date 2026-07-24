//! MCP adapter for Desert City States.
//!
//! Wraps the `dcs-app serve` binary as a subprocess and exposes game
//! operations as MCP tools for AI agent interaction.

pub mod process;
pub mod tools;

pub use process::GameProcess;
pub use tools::DcsTools;

use rmcp::model::{Implementation, ServerInfo};
use rmcp::ServerHandler;

/// The main MCP server for Desert City States.
#[derive(Clone)]
pub struct DcsServer {
    tools: DcsTools,
}

impl DcsServer {
    /// Create a new `DcsServer` with an already-spawned game process.
    pub fn new(process: GameProcess) -> Self {
        Self {
            tools: DcsTools::new(process),
        }
    }

    /// Get a reference to the tools (for mounting in the service).
    pub fn tools(&self) -> &DcsTools {
        &self.tools
    }
}

impl ServerHandler for DcsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: Some("dcs-mcp".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            },
            ..Default::default()
        }
    }
}
