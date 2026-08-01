//! `dcs-app`: top-level application crate that wires the engine
//! (`dcs-core`), the protocol contract (`dcs-protocol`), and the
//! presentation layer (`dcs-render`) together.

#[cfg(feature = "dev-tools")]
pub mod agent_bridge;
pub mod orchestrate;
pub mod protocol;
pub mod serve;
