//! Error types for the DCS MCP server.

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use std::fmt;

/// Errors that can occur when interacting with the DCS game process.
#[derive(Debug)]
pub enum DcsError {
    /// Game process not running or crashed.
    ProcessNotRunning,
    /// Failed to communicate with game process.
    CommunicationError(String),
    /// Game process returned an error response.
    GameError { code: String, message: String },
    /// Failed to parse response from game process.
    ParseError(String),
    /// Invalid input parameters.
    InvalidInput(String),
}

impl fmt::Display for DcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessNotRunning => write!(f, "Game process is not running"),
            Self::CommunicationError(e) => write!(f, "Communication error: {}", e),
            Self::GameError { code, message } => write!(f, "[{}] {}", code, message),
            Self::ParseError(e) => write!(f, "Failed to parse response: {}", e),
            Self::InvalidInput(e) => write!(f, "Invalid input: {}", e),
        }
    }
}

impl std::error::Error for DcsError {}

impl From<anyhow::Error> for DcsError {
    fn from(err: anyhow::Error) -> Self {
        let msg = format!("{err:#}");
        // Classify common failure patterns.
        if msg.contains("not running") {
            DcsError::ProcessNotRunning
        } else {
            DcsError::CommunicationError(msg)
        }
    }
}

impl From<DcsError> for McpError {
    fn from(err: DcsError) -> Self {
        match err {
            DcsError::ProcessNotRunning => McpError {
                code: ErrorCode(-32603),
                message: "Game process is not running".into(),
                data: None,
            },
            DcsError::GameError { code, message } => McpError {
                code: ErrorCode(-32603),
                message: format!("[{}] {}", code, message).into(),
                data: None,
            },
            DcsError::CommunicationError(e) => McpError {
                code: ErrorCode(-32603),
                message: format!("Communication error: {}", e).into(),
                data: None,
            },
            DcsError::ParseError(e) => McpError {
                code: ErrorCode(-32602),
                message: format!("Failed to parse response: {}", e).into(),
                data: None,
            },
            DcsError::InvalidInput(e) => McpError {
                code: ErrorCode(-32602),
                message: format!("Invalid input: {}", e).into(),
                data: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_process_not_running() {
        let err = DcsError::ProcessNotRunning;
        assert_eq!(format!("{err}"), "Game process is not running");
    }

    #[test]
    fn display_communication_error() {
        let err = DcsError::CommunicationError("connection refused".into());
        assert_eq!(format!("{err}"), "Communication error: connection refused");
    }

    #[test]
    fn display_game_error() {
        let err = DcsError::GameError {
            code: "E001".into(),
            message: "turn already ended".into(),
        };
        assert_eq!(format!("{err}"), "[E001] turn already ended");
    }

    #[test]
    fn display_parse_error() {
        let err = DcsError::ParseError("unexpected token".into());
        assert_eq!(format!("{err}"), "Failed to parse response: unexpected token");
    }

    #[test]
    fn display_invalid_input() {
        let err = DcsError::InvalidInput("missing field".into());
        assert_eq!(format!("{err}"), "Invalid input: missing field");
    }

    #[test]
    fn mcp_error_from_process_not_running() {
        let mcp: McpError = DcsError::ProcessNotRunning.into();
        assert_eq!(mcp.code, ErrorCode(-32603));
        assert_eq!(mcp.message.as_ref(), "Game process is not running");
        assert!(mcp.data.is_none());
    }

    #[test]
    fn mcp_error_from_game_error() {
        let dcs = DcsError::GameError {
            code: "E042".into(),
            message: "no units left".into(),
        };
        let mcp: McpError = dcs.into();
        assert_eq!(mcp.code, ErrorCode(-32603));
        assert_eq!(mcp.message.as_ref(), "[E042] no units left");
    }

    #[test]
    fn mcp_error_from_communication_error() {
        let dcs = DcsError::CommunicationError("broken pipe".into());
        let mcp: McpError = dcs.into();
        assert_eq!(mcp.code, ErrorCode(-32603));
        assert_eq!(mcp.message.as_ref(), "Communication error: broken pipe");
    }

    #[test]
    fn mcp_error_from_parse_error() {
        let dcs = DcsError::ParseError("trailing comma".into());
        let mcp: McpError = dcs.into();
        assert_eq!(mcp.code, ErrorCode(-32602));
        assert_eq!(mcp.message.as_ref(), "Failed to parse response: trailing comma");
    }

    #[test]
    fn mcp_error_from_invalid_input() {
        let dcs = DcsError::InvalidInput("out of range".into());
        let mcp: McpError = dcs.into();
        assert_eq!(mcp.code, ErrorCode(-32602));
        assert_eq!(mcp.message.as_ref(), "Invalid input: out of range");
    }

    #[test]
    fn anyhow_error_classified_as_process_not_running() {
        let anyhow_err = anyhow::anyhow!("game process is not running");
        let dcs: DcsError = anyhow_err.into();
        assert!(matches!(dcs, DcsError::ProcessNotRunning));
    }

    #[test]
    fn anyhow_error_classified_as_communication_error() {
        let anyhow_err = anyhow::anyhow!("connection timed out");
        let dcs: DcsError = anyhow_err.into();
        assert!(matches!(dcs, DcsError::CommunicationError(ref msg) if msg == "connection timed out"));
    }
}
