//! `dcs-core`: engine-free game core (rules, state, simulation).
//!
//! This crate intentionally has no rendering, no input, and no app-loop
//! concerns. It depends only on `dcs-protocol` (the shared `Command` /
//! `GameEvent` / `VersionedSave` contract surface).
//!
//! # Scope of this crate
//!
//! - [`model`]: the single mutable aggregate [`GameState`] plus entities, ID
//!   allocators, accessor helpers, and balance tables.
//! - [`hex`]: pure, deterministic hex-coordinate math and pathfinding.
//! - `scenario`, `serialize`, `map`, `turn`: populated by sibling agents
//!   (scenario configuration, save I/O, world generation, and the turn
//!   resolver, respectively).
//!
//! Game logic is added in later phases; the data model here is the foundation.

#![forbid(unsafe_code)]

pub mod ai;
pub mod caravan;
pub mod combat;
pub mod economy;
pub mod fog;
pub mod hex;
pub mod map;
pub mod model;
pub mod scenario;
pub mod serialize;
pub mod turn;
pub mod victory;
pub mod world;

// Re-export the shared contract types from dcs-protocol so the whole crate
// and downstream crates use ONE canonical definition. These types are owned by
// `dcs-protocol` to avoid a circular dependency (see ADR-0007/0008).
pub use dcs_protocol::{
    BuildingKind, CityId, CitySpecialization, Command, GameEvent, PlayerId, RejectReason, RelicId,
    RouteId, RouteStatus, SAVE_VERSION, TileId, UnitId, UnitKind, VersionedSave, VictoryKind,
};

// Re-export the scenario configuration types. `AiPersonality` and `Difficulty`
// are owned by `scenario` (also referenced by `model::PlayerKind::Ai`).
pub use scenario::{AiPersonality, Difficulty, ScenarioConfig, ScenarioError};

pub use model::{
    CaravanRoute, City, GameState, Player, PlayerColor, PlayerKind, QueuedOrder, Relic,
    ResourceKind, Stockpiles, TerrainType, Tile, TurnPhase, Unit, UnitAbility, VictoryTracker,
};
