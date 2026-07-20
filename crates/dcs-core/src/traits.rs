//! Extension traits for foreign types (dcs-protocol enums).
//!
//! These traits work around the orphan rule by defining methods in dcs-core
//! for types owned by dcs-protocol.

use crate::fog::{SIGHT_GUARD, SIGHT_RAIDER, SIGHT_SCOUT};
use crate::model::{TERRAIN, TerrainDef, UNITS, UnitDef};
use dcs_protocol::{BuildingKind, CitySpecialization, UnitKind};

/// Extension methods for [`UnitKind`].
pub trait UnitKindExt {
    /// Returns the sight radius for this unit kind.
    fn sight(self) -> u32;

    /// Returns whether this unit kind can found a city.
    fn can_found_city(self) -> bool;

    /// Returns the index into the balance table for this unit kind.
    fn index(self) -> usize;

    /// Returns the static unit definition for this unit kind.
    fn def(self) -> &'static UnitDef;
}

impl UnitKindExt for UnitKind {
    fn sight(self) -> u32 {
        match self {
            UnitKind::Scout => SIGHT_SCOUT,
            UnitKind::CaravanGuard => SIGHT_GUARD,
            UnitKind::Raider => SIGHT_RAIDER,
        }
    }

    fn can_found_city(self) -> bool {
        matches!(self, UnitKind::Scout)
    }

    fn index(self) -> usize {
        match self {
            UnitKind::Scout => 0,
            UnitKind::CaravanGuard => 1,
            UnitKind::Raider => 2,
        }
    }

    fn def(self) -> &'static UnitDef {
        &UNITS[self as usize]
    }
}

/// Extension methods for [`BuildingKind`].
pub trait BuildingKindExt {
    /// Returns the index into the balance table for this building kind.
    fn index(&self) -> usize;
}

impl BuildingKindExt for BuildingKind {
    fn index(&self) -> usize {
        match self {
            BuildingKind::Well => 0,
            BuildingKind::Market => 1,
            BuildingKind::Granary => 2,
            BuildingKind::Watchtower => 3,
            BuildingKind::Caravanserai => 4,
            BuildingKind::Temple => 5,
        }
    }
}

/// Extension methods for [`CitySpecialization`].
pub trait CitySpecializationExt {
    /// Returns the index into the balance table for this specialization.
    fn index(&self) -> usize;
}

impl CitySpecializationExt for CitySpecialization {
    fn index(&self) -> usize {
        match self {
            CitySpecialization::TradeHub => 0,
            CitySpecialization::WellFort => 1,
            CitySpecialization::Fortress => 2,
            CitySpecialization::ScholarOutpost => 3,
        }
    }
}

/// Extension methods for [`crate::model::TerrainType`].
pub trait TerrainTypeDef {
    /// Returns the static terrain definition for this terrain type.
    fn def(self) -> &'static TerrainDef;
}

impl TerrainTypeDef for crate::model::TerrainType {
    fn def(self) -> &'static TerrainDef {
        &TERRAIN[self as usize]
    }
}
