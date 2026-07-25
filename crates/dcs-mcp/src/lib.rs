//! MCP adapter for Desert City States.
//!
//! Wraps the `dcs-app serve` binary as a subprocess and exposes game
//! operations as MCP tools for AI agent interaction.

pub mod error;
pub mod process;
pub mod tools;

pub use error::DcsError;

pub use process::GameProcess;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::{ServerHandler, tool_handler};

/// The main MCP server for Desert City States.
#[derive(Clone)]
pub struct DcsServer {
    pub(crate) tool_router: ToolRouter<Self>,
    pub(crate) process: GameProcess,
}

impl DcsServer {
    /// Create a new `DcsServer` with an already-spawned game process.
    pub fn new(process: GameProcess) -> Self {
        Self {
            tool_router: Self::tool_router(),
            process,
        }
    }
}

#[tool_handler]
impl ServerHandler for DcsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "dcs-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some("MCP server for Desert City States game".into()),
                icons: None,
                title: Some("Desert City States".into()),
                website_url: None,
            },
            ..Default::default()
        }
    }
}
