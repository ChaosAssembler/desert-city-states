//! `dcs-protocol`: the shared, serializable **contract surface** for Desert City-States.
//!
//! This crate is *strictly* the wire/save contract between the pure simulation
//! engine (`dcs-core`) and its clients (the app / render layers). It contains
//! **no game logic and no entity structs** — those live in `dcs-core` (see
//! ADR-0007/0008).
//!
//! What this crate owns (the contract):
//!
//! - **ID newtypes** ([`TileId`], [`CityId`], [`UnitId`], [`RouteId`],
//!   [`PlayerId`], [`RelicId`]): plain `u32` newtypes. These are *defined here*
//!   so that both `dcs-core` and the protocol agree on a single ID type without
//!   creating a circular dependency (`dcs-core → dcs-protocol → dcs-core`).
//!   `dcs-core` re-exports them via `pub use dcs_protocol::{ ... };`.
//!
//! - **Catalog enums** ([`UnitKind`], [`BuildingKind`], [`CitySpecialization`],
//!   [`RouteStatus`], [`VictoryKind`]): the small, shared, additive-enum
//!   vocabulary referenced by commands and events.
//!
//! - **Messages**: [`Command`] (player/AI intent) and [`GameEvent`] (engine
//!   outcome), the only payloads crossing the core/app boundary.
//!
//! - **Envelope**: [`VersionedSave<T>`] plus the [`SAVE_VERSION`] constant,
//!   which guards save-file forward/backward compatibility.
//!
//! Everything here is `serde`-derivable so it can be carried over `serde_json`
//! (dev) or `postcard`/`bincode` (ship) behind `dcs-core::serialize`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Identity newtypes
// ---------------------------------------------------------------------------
//
// Defined in dcs-protocol (not dcs-core) to avoid a circular dependency while
// keeping a single canonical ID type. dcs-core does `pub use dcs_protocol::*`
// for these so there is exactly one definition across the workspace.

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash,
            Serialize, Deserialize, PartialOrd, Ord, Default,
        )]
        pub struct $name(pub u32);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<$name> for u32 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl From<$name> for usize {
            fn from(id: $name) -> Self {
                id.0 as usize
            }
        }
    };
}

id_newtype!(
    /// Identifier of a tile in the map (index into `GameState::tiles`).
    TileId
);
id_newtype!(
    /// Identifier of a city.
    CityId
);
id_newtype!(
    /// Identifier of a unit.
    UnitId
);
id_newtype!(
    /// Identifier of a caravan route.
    RouteId
);
id_newtype!(
    /// Identifier of a player (human or AI).
    PlayerId
);
id_newtype!(
    /// Identifier of a relic site.
    RelicId
);

// ---------------------------------------------------------------------------
// Catalog enums (shared, additive contract vocabulary)
// ---------------------------------------------------------------------------

/// The three unit archetypes in the game.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UnitKind {
    /// Cheap, fast scout. Default variant.
    #[default]
    Scout,
    /// Defensive escort for caravans.
    CaravanGuard,
    /// Offensive raider.
    Raider,
}

/// Buildings that can be constructed in a city.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BuildingKind {
    /// Basic water source. Default variant.
    #[default]
    Well,
    Market,
    Granary,
    Watchtower,
    Caravanserai,
    Temple,
}

/// How a city is specialized.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CitySpecialization {
    /// Trade-oriented hub. Default variant.
    #[default]
    TradeHub,
    WellFort,
    Fortress,
    ScholarOutpost,
}

/// Live operational status of a caravan route.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RouteStatus {
    /// Routing and transferring normally. Default variant.
    #[default]
    Active,
    /// Under threat but still functioning.
    Threatened,
    /// Cut; no transfers this turn.
    Severed,
}

/// Victory condition labels.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VictoryKind {
    /// Control the majority of oases. Default variant.
    #[default]
    OasisDominance,
    WealthScore,
    RelicHold,
    /// Timed-out fallback label for turn-limit games.
    TurnLimit,
}

// ---------------------------------------------------------------------------
// Display implementations for catalog enums
// ---------------------------------------------------------------------------

impl std::fmt::Display for UnitKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scout => write!(f, "Scout"),
            Self::CaravanGuard => write!(f, "CaravanGuard"),
            Self::Raider => write!(f, "Raider"),
        }
    }
}

impl std::fmt::Display for BuildingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Well => write!(f, "Well"),
            Self::Market => write!(f, "Market"),
            Self::Granary => write!(f, "Granary"),
            Self::Watchtower => write!(f, "Watchtower"),
            Self::Caravanserai => write!(f, "Caravanserai"),
            Self::Temple => write!(f, "Temple"),
        }
    }
}

impl std::fmt::Display for CitySpecialization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TradeHub => write!(f, "TradeHub"),
            Self::WellFort => write!(f, "WellFort"),
            Self::Fortress => write!(f, "Fortress"),
            Self::ScholarOutpost => write!(f, "ScholarOutpost"),
        }
    }
}

impl std::fmt::Display for RouteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Threatened => write!(f, "Threatened"),
            Self::Severed => write!(f, "Severed"),
        }
    }
}

impl std::fmt::Display for VictoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OasisDominance => write!(f, "OasisDominance"),
            Self::WealthScore => write!(f, "WealthScore"),
            Self::RelicHold => write!(f, "RelicHold"),
            Self::TurnLimit => write!(f, "TurnLimit"),
        }
    }
}

// ---------------------------------------------------------------------------
// Commands (player / AI intent)
// ---------------------------------------------------------------------------

/// A command issued by a player or AI. Commands are the **only** mutation
/// entry point into `dcs-core`; the engine validates and resolves them,
/// emitting [`GameEvent`]s.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Move a unit to an adjacent/reachable tile.
    MoveUnit { unit: UnitId, to: TileId },
    /// Found a city with a unit on an oasis tile.
    FoundCity { unit: UnitId, tile: TileId },
    /// Train a new unit in a city.
    TrainUnit { city: CityId, kind: UnitKind },
    /// Build an improvement in a city.
    Build {
        city: CityId,
        building: BuildingKind,
    },
    /// Specialize a city.
    Specialize {
        city: CityId,
        spec: CitySpecialization,
    },
    /// Connect an auto-routed caravan route between two cities (endpoints only).
    ConnectRoute { from: CityId, to: CityId },
    /// Station a unit to patrol a tile (route guard).
    Patrol { unit: UnitId, tile: TileId },
    /// Garrison a unit inside a city.
    Garrison { unit: UnitId, city: CityId },
    /// Raid (cut) an enemy caravan route.
    RaidRoute { unit: UnitId, route: RouteId },
    /// Raid an enemy city.
    RaidCity { unit: UnitId, city: CityId },
    /// End the actor's turn.
    EndTurn,
}

// ---------------------------------------------------------------------------
// Rejection reasons
// ---------------------------------------------------------------------------

/// Why a [`Command`] was rejected by the engine resolver.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// The referenced unit does not belong to the acting player.
    NotYourUnit,
    /// Target tile is off the map.
    OffMap,
    /// Tile is not an oasis (e.g. for founding).
    NotOasis,
    /// Command target is not a legal destination/interaction.
    IllegalTarget,
    /// Required resource is unavailable.
    NoResource,
    /// Path/action is blocked.
    Blocked,
    /// Unit has no moves remaining.
    OutOfMoves,
    /// It is not the acting player's turn.
    NotYourTurn,
    /// Game/command state is invalid for this command.
    InvalidState,
}

// ---------------------------------------------------------------------------
// Events (engine outcomes)
// ---------------------------------------------------------------------------

/// An event emitted by `dcs-core` in response to one or more [`Command`]s, or
/// as part of end-of-round bookkeeping. The full event log is also the
/// replay/UI feed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum GameEvent {
    /// A unit moved from one tile to another.
    UnitMoved {
        unit: UnitId,
        from: TileId,
        to: TileId,
    },
    /// A new city was founded.
    CityFounded {
        city: CityId,
        owner: PlayerId,
        tile: TileId,
    },
    /// A unit was trained in a city.
    UnitTrained { unit: UnitId, city: CityId },
    /// A building was completed in a city.
    Built {
        city: CityId,
        building: BuildingKind,
    },
    /// A city changed specialization.
    Specialized {
        city: CityId,
        spec: CitySpecialization,
    },
    /// A caravan route was created (with its computed tile path).
    RouteCreated {
        route: RouteId,
        from: CityId,
        to: CityId,
        path: Vec<TileId>,
    },
    /// A route's status changed.
    RouteStatusChanged {
        route: RouteId,
        old_status: RouteStatus,
        status: RouteStatus,
    },
    /// A unit began patrolling a tile.
    UnitPatrolled { unit: UnitId, tile: TileId },
    /// A unit was garrisoned in a city.
    UnitGarrisoned { unit: UnitId, city: CityId },
    /// A route was raided (possibly severed).
    RouteRaided {
        route: RouteId,
        by: PlayerId,
        severed: bool,
    },
    /// A city was raided (population lost).
    CityRaided {
        city: CityId,
        by: PlayerId,
        pop_lost: u32,
    },
    /// Combat resolved between two units.
    Combat {
        attacker: UnitId,
        defender: UnitId,
        attacker_loss: u32,
        defender_loss: u32,
        retreated: bool,
    },
    /// Income applied to a player.
    Income {
        player: PlayerId,
        water: i32,
        wealth: i32,
        influence: i32,
    },
    /// A city grew in population.
    Grown { city: CityId, population: u32 },
    /// A city starved (lost population).
    Starved { city: CityId, population: u32 },
    /// Tiles were revealed to a player (fog of war).
    Revealed {
        player: PlayerId,
        tiles: Vec<TileId>,
    },
    /// A victory condition was satisfied.
    Victory { kind: VictoryKind, winner: PlayerId },
    /// The turn counter advanced.
    TurnAdvanced { turn: u32 },
    /// A command was rejected.
    Rejected {
        command: Command,
        reason: RejectReason,
    },
    /// A non-fatal warning message.
    Warn { message: String },
}

// ---------------------------------------------------------------------------
// Display implementations for RejectReason and GameEvent
// ---------------------------------------------------------------------------

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotYourUnit => write!(f, "Unit does not belong to you"),
            Self::OffMap => write!(f, "Target tile is off the map"),
            Self::NotOasis => write!(f, "Tile is not an oasis"),
            Self::IllegalTarget => write!(f, "Illegal target for this command"),
            Self::NoResource => write!(f, "Required resource is unavailable"),
            Self::Blocked => write!(f, "Path or action is blocked"),
            Self::OutOfMoves => write!(f, "Unit has no moves remaining"),
            Self::NotYourTurn => write!(f, "It is not your turn"),
            Self::InvalidState => write!(f, "Invalid game state for this command"),
        }
    }
}

impl std::fmt::Display for GameEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnitMoved { unit, from, to } => {
                write!(f, "Unit {} moved from {} to {}", unit, from, to)
            }
            Self::CityFounded { city, owner, tile } => {
                write!(f, "City {} founded by {} on tile {}", city, owner, tile)
            }
            Self::UnitTrained { unit, city } => {
                write!(f, "Unit {} trained in city {}", unit, city)
            }
            Self::Built { city, building } => {
                write!(f, "{} built in city {}", building, city)
            }
            Self::Specialized { city, spec } => {
                write!(f, "City {} specialized to {}", city, spec)
            }
            Self::RouteCreated {
                route,
                from,
                to,
                path: _,
            } => {
                write!(f, "Route {} created from {} to {}", route, from, to)
            }
            Self::RouteStatusChanged {
                route,
                old_status,
                status,
            } => {
                write!(
                    f,
                    "Route {} status changed from {} to {}",
                    route, old_status, status
                )
            }
            Self::UnitPatrolled { unit, tile } => {
                write!(f, "Unit {} patrolling tile {}", unit, tile)
            }
            Self::UnitGarrisoned { unit, city } => {
                write!(f, "Unit {} garrisoned in city {}", unit, city)
            }
            Self::RouteRaided { route, by, severed } => {
                write!(
                    f,
                    "Route {} raided by {}{}",
                    route,
                    by,
                    if *severed { " (severed)" } else { "" }
                )
            }
            Self::CityRaided { city, by, pop_lost } => {
                write!(f, "City {} raided by {} (lost {} pop)", city, by, pop_lost)
            }
            Self::Combat {
                attacker,
                defender,
                attacker_loss,
                defender_loss,
                retreated,
            } => {
                write!(
                    f,
                    "Combat: {} vs {} (losses: {} vs {}{})",
                    attacker,
                    defender,
                    attacker_loss,
                    defender_loss,
                    if *retreated { ", retreated" } else { "" }
                )
            }
            Self::Income {
                player,
                water,
                wealth,
                influence,
            } => {
                write!(
                    f,
                    "Income for {}: water={}, wealth={}, influence={}",
                    player, water, wealth, influence
                )
            }
            Self::Grown { city, population } => {
                write!(f, "City {} grew to {} population", city, population)
            }
            Self::Starved { city, population } => {
                write!(f, "City {} starved ({} remaining)", city, population)
            }
            Self::Revealed { player, tiles } => {
                write!(f, "Revealed {} tiles to {}", tiles.len(), player)
            }
            Self::Victory { kind, winner } => {
                write!(f, "{} victory for player {}", kind, winner)
            }
            Self::TurnAdvanced { turn } => {
                write!(f, "Turn advanced to {}", turn)
            }
            Self::Rejected { command, reason } => {
                write!(f, "Command {:?} rejected: {}", command, reason)
            }
            Self::Warn { message } => {
                write!(f, "Warning: {}", message)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Save envelope
// ---------------------------------------------------------------------------

/// Forward/backward-compatible save wrapper. The `payload` is the engine's
/// serializable state (e.g. `GameState`); `version` is checked against
/// [`SAVE_VERSION`] on load to gate migrations.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionedSave<T> {
    /// Schema version of the payload.
    pub version: u32,
    /// The serialized payload.
    pub payload: T,
}

/// Current save schema version. Bump when the on-disk layout changes in a
/// breaking way; older versions run migration functions.
pub const SAVE_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_turn_round_trips() {
        let cmd = Command::EndTurn;
        let json = serde_json::to_string(&cmd).expect("serialize Command");
        let back: Command = serde_json::from_str(&json).expect("deserialize Command");
        assert_eq!(cmd, back);
    }

    #[test]
    fn versioned_save_round_trips() {
        let save = VersionedSave {
            version: SAVE_VERSION,
            payload: 42u32,
        };
        let json = serde_json::to_string(&save).expect("serialize VersionedSave");
        let back: VersionedSave<u32> =
            serde_json::from_str(&json).expect("deserialize VersionedSave");
        assert_eq!(back.version, SAVE_VERSION);
        assert_eq!(back.payload, 42);
    }

    #[test]
    fn ids_default_to_zero() {
        assert_eq!(TileId::default(), TileId(0));
        assert_eq!(CityId::default(), CityId(0));
        assert_eq!(UnitId::default(), UnitId(0));
        assert_eq!(RouteId::default(), RouteId(0));
        assert_eq!(PlayerId::default(), PlayerId(0));
        assert_eq!(RelicId::default(), RelicId(0));
    }

    #[test]
    fn catalog_defaults() {
        assert_eq!(UnitKind::default(), UnitKind::Scout);
        assert_eq!(BuildingKind::default(), BuildingKind::Well);
        assert_eq!(CitySpecialization::default(), CitySpecialization::TradeHub);
        assert_eq!(RouteStatus::default(), RouteStatus::Active);
        assert_eq!(VictoryKind::default(), VictoryKind::OasisDominance);
    }
}
