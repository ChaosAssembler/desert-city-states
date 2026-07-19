//! Shared test utilities for dcs-core.
//!
//! This module provides common factory functions and helpers used across
//! all test modules to reduce duplication.

#[cfg(test)]
use crate::hex::{HexCoord, ORIGIN};
#[cfg(test)]
use crate::model::*;
#[cfg(test)]
use crate::scenario::mvp_preset;
#[cfg(test)]
use crate::*;
#[cfg(test)]
use fxhash::FxHashSet;
#[cfg(test)]
use std::collections::VecDeque;

/// Allocate all hex tiles in a circular map of the given radius.
/// Marks origin as Oasis, and a second oasis at (2, -2) if in range.
#[cfg(test)]
pub fn allocate_hex_grid(state: &mut GameState, radius: u32) {
    let coords = ORIGIN.range(radius);
    for c in coords {
        let id = state.alloc_tile_id();
        state.tiles.push(Tile {
            id,
            coord: c,
            terrain: TerrainType::Dunes,
            is_relic_site: false,
            owner: None,
            improvement: None,
        });
        state.tile_index.insert(c, id);
    }
}

/// Mark a tile at the given coordinate with a specific terrain type.
/// No-op if the coordinate is not in the tile index.
#[cfg(test)]
pub fn mark_terrain(state: &mut GameState, coord: HexCoord, terrain: TerrainType) {
    if let Some(&id) = state.tile_index.get(&coord) {
        state.tiles[id.0 as usize].terrain = terrain;
    }
}

/// Mark a tile as a relic site.
#[cfg(test)]
pub fn mark_relic_site(state: &mut GameState, coord: HexCoord) {
    if let Some(&id) = state.tile_index.get(&coord) {
        state.tiles[id.0 as usize].is_relic_site = true;
    }
}

/// Create a player with the given kind and resource levels.
/// Returns the PlayerId.
#[cfg(test)]
pub fn create_player(state: &mut GameState, kind: PlayerKind, resources: Stockpiles) -> PlayerId {
    let pid = state.alloc_player_id();
    state.players.push(Player {
        id: pid,
        kind,
        color: PlayerColor::Sand,
        resources,
        discovered: FxHashSet::default(),
        defeated: false,
    });
    pid
}

/// Create a city at the given coordinate for the specified owner.
/// Returns the CityId.
#[cfg(test)]
pub fn create_city(
    state: &mut GameState,
    owner: PlayerId,
    coord: HexCoord,
    population: u32,
) -> CityId {
    let tile = state.tile_index[&coord];
    let city_id = state.alloc_city_id();
    state.cities.push(City {
        id: city_id,
        owner,
        tile,
        population,
        specialization: None,
        buildings: vec![],
        stockpiles: Stockpiles::default(),
        route_slots: 2,
        growth_timer: 0,
        queue: VecDeque::new(),
    });
    city_id
}

/// Create a unit at the given tile for the specified owner.
/// Uses unit_def() for default HP and moves unless overridden.
/// Returns the UnitId.
#[cfg(test)]
pub fn create_unit(state: &mut GameState, owner: PlayerId, kind: UnitKind, tile: TileId) -> UnitId {
    let def = unit_def(kind);
    let id = state.alloc_unit_id();
    state.units.push(Unit {
        id,
        owner,
        kind,
        tile,
        hp: def.hp as u32,
        moves_left: def.moves,
        ability: UnitAbility::None,
    });
    id
}

/// Create a unit with custom HP and moves (for testing damage scenarios).
#[cfg(test)]
pub fn create_unit_with_hp(
    state: &mut GameState,
    owner: PlayerId,
    kind: UnitKind,
    tile: TileId,
    hp: u32,
) -> UnitId {
    let def = unit_def(kind);
    let id = state.alloc_unit_id();
    state.units.push(Unit {
        id,
        owner,
        kind,
        tile,
        hp,
        moves_left: def.moves,
        ability: UnitAbility::None,
    });
    id
}

/// Create a basic game state with:
/// - MVP preset config, seed=1
/// - Full hex grid allocated
/// - Origin marked as Oasis
/// - One Human player with default resources
#[cfg(test)]
pub fn minimal_state() -> GameState {
    let cfg = mvp_preset();
    let mut s = GameState::new(cfg, 1);
    let radius = s.scenario.map_radius as u32;
    allocate_hex_grid(&mut s, radius);
    mark_terrain(&mut s, ORIGIN, TerrainType::Oasis);
    create_player(
        &mut s,
        PlayerKind::Human,
        Stockpiles {
            water: 10,
            wealth: 100,
            influence: 50,
        },
    );
    s
}

/// Create a game state with N cities on oases.
#[cfg(test)]
pub fn state_with_cities(n: usize) -> GameState {
    let mut s = minimal_state();
    let pid = PlayerId(0);
    // Standard oasis positions for test harnesses
    let oasis_coords = [
        HexCoord { q: 0, r: 0 },
        HexCoord { q: 2, r: -2 },
        HexCoord { q: -2, r: 2 },
        HexCoord { q: 1, r: -1 },
        HexCoord { q: -1, r: 1 },
    ];
    for &coord in oasis_coords.iter().take(n.min(oasis_coords.len())) {
        mark_terrain(&mut s, coord, TerrainType::Oasis);
        create_city(&mut s, pid, coord, 2);
    }
    s
}

/// Find a unit belonging to the given player with the specified kind.
/// Panics if no such unit exists.
#[cfg(test)]
pub fn find_unit(state: &GameState, owner: PlayerId, kind: UnitKind) -> UnitId {
    state
        .units
        .iter()
        .find(|u| u.owner == owner && u.kind == kind)
        .expect("unit not found")
        .id
}

/// Get the tile a unit is currently on.
#[cfg(test)]
pub fn unit_tile(state: &GameState, unit: UnitId) -> TileId {
    state
        .units
        .iter()
        .find(|u| u.id == unit)
        .expect("unit exists")
        .tile
}

/// Get a neighbor tile that is within the map bounds.
#[cfg(test)]
pub fn neighbor_tile_in_map(state: &GameState, tile: TileId) -> TileId {
    let coord = state.tiles[tile.0 as usize].coord;
    let radius = state.scenario.map_radius as u32;
    let n = coord.neighbors()
        .into_iter()
        .find(|h| h.in_map(radius))
        .unwrap_or(coord);
    state.tile_index[&n]
}

/// Create an active trade route between two cities.
#[cfg(test)]
pub fn create_active_route(state: &mut GameState, from: CityId, to: CityId) -> RouteId {
    let route_id = RouteId(state.routes.len() as u32);
    let owner = state.cities[from.0 as usize].owner;
    state.routes.push(CaravanRoute {
        id: route_id,
        owner,
        endpoints: (from, to),
        path: vec![
            state.cities[from.0 as usize].tile,
            state.cities[to.0 as usize].tile,
        ],
        status: RouteStatus::Active,
        length: 2,
        upkeep: crate::caravan::ROUTE_UPKEEP_WATER as u8,
        consecutive_threatened: 0,
    });
    route_id
}

/// Builder for creating test game states with a fluent API.
///
/// # Example
/// ```
/// let state = GameStateBuilder::new()
///     .with_seed(42)
///     .with_player_resources(Stockpiles { water: 10, wealth: 50, influence: 20 })
///     .with_city(HexCoord { q: 0, r: 0 }, PlayerId(0), 3)
///     .with_city(HexCoord { q: 2, r: -2 }, PlayerId(0), 2)
///     .with_unit(UnitKind::Scout, PlayerId(0), HexCoord { q: 0, r: 0 })
///     .build();
/// ```
#[cfg(test)]
pub struct GameStateBuilder {
    config: ScenarioConfig,
    seed: u64,
    player_kind: PlayerKind,
    player_resources: Stockpiles,
    cities: Vec<(HexCoord, PlayerId, u32)>, // (coord, owner, population)
    units: Vec<(UnitKind, PlayerId, HexCoord)>, // (kind, owner, coord)
    oasis_coords: Vec<HexCoord>,
}

#[cfg(test)]
impl GameStateBuilder {
    /// Create a new builder with default settings (MVP preset, seed=1, Human player).
    pub fn new() -> Self {
        Self {
            config: mvp_preset(),
            seed: 1,
            player_kind: PlayerKind::Human,
            player_resources: Stockpiles {
                water: 10,
                wealth: 100,
                influence: 50,
            },
            cities: Vec::new(),
            units: Vec::new(),
            oasis_coords: vec![HexCoord { q: 0, r: 0 }],
        }
    }

    /// Set the random seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Set the scenario config.
    pub fn with_config(mut self, config: ScenarioConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the player kind (Human or AI).
    pub fn with_player_kind(mut self, kind: PlayerKind) -> Self {
        self.player_kind = kind;
        self
    }

    /// Set the starting resources for the player.
    pub fn with_player_resources(mut self, resources: Stockpiles) -> Self {
        self.player_resources = resources;
        self
    }

    /// Add an oasis at the given coordinate.
    pub fn with_oasis(mut self, coord: HexCoord) -> Self {
        self.oasis_coords.push(coord);
        self
    }

    /// Add a city at the given coordinate for the specified owner.
    pub fn with_city(mut self, coord: HexCoord, owner: PlayerId, population: u32) -> Self {
        self.cities.push((coord, owner, population));
        self
    }

    /// Add a unit of the given kind at the coordinate for the specified owner.
    pub fn with_unit(mut self, kind: UnitKind, owner: PlayerId, coord: HexCoord) -> Self {
        self.units.push((kind, owner, coord));
        self
    }

    /// Build the GameState.
    pub fn build(self) -> GameState {
        let mut s = GameState::new(self.config, self.seed);
        let radius = s.scenario.map_radius as u32;

        // Allocate hex grid
        allocate_hex_grid(&mut s, radius);

        // Mark oases
        for coord in &self.oasis_coords {
            mark_terrain(&mut s, *coord, TerrainType::Oasis);
        }

        // Create player
        create_player(&mut s, self.player_kind, self.player_resources);

        // Create cities
        for (coord, owner, pop) in self.cities {
            create_city(&mut s, owner, coord, pop);
        }

        // Create units
        for (kind, owner, coord) in self.units {
            let tile = s.tile_index[&coord];
            create_unit(&mut s, owner, kind, tile);
        }

        s
    }
}

impl Default for GameStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}
