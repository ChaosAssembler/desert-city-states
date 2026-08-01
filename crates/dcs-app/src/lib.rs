//! `dcs-app`: top-level application crate that wires the engine
//! (`dcs-core`), the protocol contract (`dcs-protocol`), and the
//! presentation layer (`dcs-render`) together.

pub mod orchestrate;
pub mod protocol;
pub mod serve;
