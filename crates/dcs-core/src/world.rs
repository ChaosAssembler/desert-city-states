//! City and unit gameplay logic: building, training, specialization, growth,
//! production queues, and zone-of-control calculations.
//!
//! This module is **pure** — no render dependencies. All randomness flows
//! through `state.rng`; all mutations go through `GameState`.

use crate::hex::range;
use crate::model::{
    BUILD_COST, FORTRESS_TRAIN_DISCOUNT, GRANARY_WATER_BONUS, GROWTH_PERIOD_TURNS,
    GROWTH_WATER_THRESHOLD, INFLUENCE_CAP_BASE, POP_FOR_SPECIALIZE, SPECIALIZE_COST_INFLUENCE,
    TERRAIN, TRADE_HUB_MARKET_DISCOUNT, UNIT_CAP_BASE, UNIT_TRAIN_COST, WATER_CAP_BASE,
    WEALTH_CAP_BASE, unit_def,
};
use crate::{
    BuildingKind, CityId, CitySpecialization, Command, GameEvent, GameState, PlayerId, QueuedOrder,
    RejectReason, TileId, UnitKind,
};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Index helpers (free functions — cannot impl foreign types)
// ---------------------------------------------------------------------------

/// Discriminant-based index for [`BuildingKind`] into cost/balance tables.
pub fn building_index(kind: &BuildingKind) -> usize {
    match kind {
        BuildingKind::Well => 0,
        BuildingKind::Market => 1,
        BuildingKind::Granary => 2,
        BuildingKind::Watchtower => 3,
        BuildingKind::Caravanserai => 4,
        BuildingKind::Temple => 5,
    }
}

/// Discriminant-based index for [`CitySpecialization`] into balance tables.
pub fn specialization_index(spec: &CitySpecialization) -> usize {
    match spec {
        CitySpecialization::TradeHub => 0,
        CitySpecialization::WellFort => 1,
        CitySpecialization::Fortress => 2,
        CitySpecialization::ScholarOutpost => 3,
    }
}

/// Discriminant-based index for [`UnitKind`] into balance tables.
pub fn unit_kind_index(kind: &UnitKind) -> usize {
    match kind {
        UnitKind::Scout => 0,
        UnitKind::CaravanGuard => 1,
        UnitKind::Raider => 2,
    }
}

// ---------------------------------------------------------------------------
// Worked tiles & building slots
// ---------------------------------------------------------------------------

/// Returns worked tiles for a city: the city tile plus all tiles in ring(1).
pub fn worked_tiles(state: &GameState, city_id: CityId) -> Vec<TileId> {
    let c = &state.cities[city_id.0 as usize];
    let coord = state.tiles[c.tile.0 as usize].coord;
    let mut tiles = vec![c.tile];
    for hex in range(coord, 1) {
        if let Some(&tid) = state.tile_index.get(&hex) {
            tiles.push(tid);
        }
    }
    tiles
}

/// Building slots for a city: 2 + population / 2.
pub fn building_slots(city: &crate::City) -> u8 {
    (2 + city.population / 2) as u8
}

/// Check if a unit kind can found a city (DD #4 default: only Scout).
pub fn is_founding_unit(kind: UnitKind) -> bool {
    matches!(kind, UnitKind::Scout)
}

// ---------------------------------------------------------------------------
// Build resolution
// ---------------------------------------------------------------------------

/// Resolve a Build command: validate, spend wealth, add building to city.
pub fn resolve_build(
    state: &mut GameState,
    city_id: CityId,
    building: BuildingKind,
    actor: PlayerId,
) -> Vec<GameEvent> {
    let cmd = Command::Build {
        city: city_id,
        building,
    };

    // Validate: city exists and is owned by actor.
    let city = match state.cities.get(city_id.0 as usize) {
        Some(c) if c.owner == actor => c,
        _ => {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::InvalidState,
            }];
        }
    };

    // Validate: building slots available.
    if city.buildings.len() >= building_slots(city) as usize {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::Blocked,
        }];
    }

    // Validate: building not already present (single-tier MVP).
    if city.buildings.contains(&building) {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::InvalidState,
        }];
    }

    // Calculate cost (Trade Hub discount for Market).
    let base_cost = BUILD_COST[building_index(&building)];
    let cost = if building == BuildingKind::Market
        && city.specialization == Some(CitySpecialization::TradeHub)
    {
        base_cost.saturating_sub(TRADE_HUB_MARKET_DISCOUNT)
    } else {
        base_cost
    };

    // Validate: enough wealth.
    if state.players[actor.0 as usize].resources.wealth < cost {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::NoResource,
        }];
    }

    // Spend wealth.
    state.players[actor.0 as usize].resources.wealth -= cost;

    // Add building.
    state.cities[city_id.0 as usize].buildings.push(building);

    vec![GameEvent::Built {
        city: city_id,
        building,
    }]
}

// ---------------------------------------------------------------------------
// Specialize resolution
// ---------------------------------------------------------------------------

/// Resolve a Specialize command: validate, spend influence, set specialization.
pub fn resolve_specialize(
    state: &mut GameState,
    city_id: CityId,
    spec: CitySpecialization,
    actor: PlayerId,
) -> Vec<GameEvent> {
    let cmd = Command::Specialize {
        city: city_id,
        spec,
    };

    // Validate: city exists and is owned by actor.
    let city = match state.cities.get(city_id.0 as usize) {
        Some(c) if c.owner == actor => c,
        _ => {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::InvalidState,
            }];
        }
    };

    // Validate: population >= 3.
    if city.population < POP_FOR_SPECIALIZE {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::InvalidState,
        }];
    }

    // Validate: not already specialized.
    if city.specialization.is_some() {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::InvalidState,
        }];
    }

    // Validate: enough influence.
    if state.players[actor.0 as usize].resources.influence < SPECIALIZE_COST_INFLUENCE {
        return vec![GameEvent::Rejected {
            command: cmd,
            reason: RejectReason::NoResource,
        }];
    }

    // Spend influence.
    state.players[actor.0 as usize].resources.influence -= SPECIALIZE_COST_INFLUENCE;

    // Set specialization.
    state.cities[city_id.0 as usize].specialization = Some(spec);

    vec![GameEvent::Specialized {
        city: city_id,
        spec,
    }]
}

// ---------------------------------------------------------------------------
// Production queue
// ---------------------------------------------------------------------------

/// Process a city's production queue at Income phase, consuming orders
/// head-to-tail until one cannot be afforded or completed.
pub fn process_queue(state: &mut GameState, city_id: CityId) -> Vec<GameEvent> {
    let mut events = Vec::new();
    let city_idx = city_id.0 as usize;

    while let Some(order) = state.cities[city_idx].queue.first().cloned() {
        let result = match order {
            QueuedOrder::Build(building) => {
                if state.cities[city_idx].buildings.len()
                    >= building_slots(&state.cities[city_idx]) as usize
                {
                    break; // no more slots
                }
                if state.cities[city_idx].buildings.contains(&building) {
                    break; // already built
                }
                let cost = BUILD_COST[building_index(&building)];
                let owner = state.cities[city_idx].owner;
                if state.players[owner.0 as usize].resources.wealth < cost {
                    break; // can't afford
                }
                state.players[owner.0 as usize].resources.wealth -= cost;
                state.cities[city_idx].buildings.push(building);
                Some(GameEvent::Built {
                    city: city_id,
                    building,
                })
            }
            QueuedOrder::Train(kind) => {
                let base_cost = UNIT_TRAIN_COST[unit_kind_index(&kind)];
                let owner = state.cities[city_idx].owner;
                let city = &state.cities[city_idx];

                // Raider must be in Fortress.
                if kind == UnitKind::Raider
                    && city.specialization != Some(CitySpecialization::Fortress)
                {
                    break;
                }

                // Calculate cost with Fortress discount.
                let mut cost = base_cost;
                if city.specialization == Some(CitySpecialization::Fortress) {
                    cost = (cost as f32 * FORTRESS_TRAIN_DISCOUNT) as u32;
                }

                if state.players[owner.0 as usize].resources.wealth < cost {
                    break;
                }

                // Check unit cap.
                let cap = unit_cap(state, owner);
                let current = state.units.iter().filter(|u| u.owner == owner).count() as u32;
                if current >= cap {
                    break;
                }

                state.players[owner.0 as usize].resources.wealth -= cost;
                let tile = state.cities[city_idx].tile;
                let unit_id = state.alloc_unit_id();
                let def = unit_def(kind);
                state.units.push(crate::Unit {
                    id: unit_id,
                    owner,
                    kind,
                    tile,
                    hp: def.hp as u32,
                    moves_left: def.moves,
                    ability: crate::UnitAbility::None,
                });
                crate::fog::reveal_from_unit(state, unit_id);
                Some(GameEvent::UnitTrained {
                    unit: unit_id,
                    city: city_id,
                })
            }
            QueuedOrder::Specialize(spec) => {
                if state.cities[city_idx].specialization.is_some() {
                    break;
                }
                if state.cities[city_idx].population < POP_FOR_SPECIALIZE {
                    break;
                }
                let owner = state.cities[city_idx].owner;
                if state.players[owner.0 as usize].resources.influence < SPECIALIZE_COST_INFLUENCE {
                    break;
                }
                state.players[owner.0 as usize].resources.influence -= SPECIALIZE_COST_INFLUENCE;
                state.cities[city_idx].specialization = Some(spec);
                Some(GameEvent::Specialized {
                    city: city_id,
                    spec,
                })
            }
        };

        if let Some(event) = result {
            state.cities[city_idx].queue.remove(0);
            events.push(event);
        } else {
            break;
        }
    }

    events
}

// ---------------------------------------------------------------------------
// City growth
// ---------------------------------------------------------------------------

/// Apply city growth: check water threshold and increment timer/pop.
pub fn apply_growth(state: &mut GameState, city_id: CityId) -> Vec<GameEvent> {
    let mut events = Vec::new();
    let city_idx = city_id.0 as usize;

    let water_yield = city_water_yield(state, city_id);

    if water_yield > GROWTH_WATER_THRESHOLD {
        state.cities[city_idx].growth_timer += 1;
        if state.cities[city_idx].growth_timer >= GROWTH_PERIOD_TURNS {
            state.cities[city_idx].population += 1;
            state.cities[city_idx].growth_timer = 0;
            events.push(GameEvent::Grown {
                city: city_id,
                population: state.cities[city_idx].population,
            });
        }
    } else {
        state.cities[city_idx].growth_timer = 0;
    }

    events
}

/// Compute water yield for a city from worked tiles, Well bonus, and
/// Well Fort specialization bonus.
pub(crate) fn city_water_yield(state: &GameState, city_id: CityId) -> u32 {
    let c = &state.cities[city_id.0 as usize];
    let mut water = 0u32;

    // Worked tile yields.
    for &tid in &worked_tiles(state, city_id) {
        let tile = &state.tiles[tid.0 as usize];
        water += TERRAIN[tile.terrain as usize].water as u32;
    }

    // Well building bonus.
    if c.buildings.contains(&BuildingKind::Well) {
        water += 2;
    }

    // Well Fort specialization bonus.
    if c.specialization == Some(CitySpecialization::WellFort) {
        water += 3;
    }

    water
}

// ---------------------------------------------------------------------------
// Unit cap
// ---------------------------------------------------------------------------

/// Check how many units a player can field (base + total population across
/// their cities).
pub fn unit_cap(state: &GameState, player: PlayerId) -> u32 {
    let total_pop: u32 = state
        .cities
        .iter()
        .filter(|c| c.owner == player)
        .map(|c| c.population)
        .sum();
    UNIT_CAP_BASE + total_pop
}

// ---------------------------------------------------------------------------
// Zone of Control
// ---------------------------------------------------------------------------

/// Fortress cities project Zone of Control onto the city tile plus its 6
/// immediate neighbors.
pub fn zone_of_control(state: &GameState, player: PlayerId) -> HashSet<TileId> {
    let mut zoc = HashSet::new();

    for city in &state.cities {
        if city.owner == player && city.specialization == Some(CitySpecialization::Fortress) {
            let coord = state.tiles[city.tile.0 as usize].coord;
            zoc.insert(city.tile);
            for hex in range(coord, 1) {
                if let Some(&tid) = state.tile_index.get(&hex) {
                    zoc.insert(tid);
                }
            }
        }
    }

    zoc
}

// ---------------------------------------------------------------------------
// Resource caps
// ---------------------------------------------------------------------------

/// Get the water cap for a player (base + Granary bonuses).
pub fn water_cap(state: &GameState, player: PlayerId) -> u32 {
    let granaries: u32 = state
        .cities
        .iter()
        .filter(|c| c.owner == player)
        .map(|c| {
            c.buildings
                .iter()
                .filter(|b| **b == BuildingKind::Granary)
                .count() as u32
        })
        .sum();
    WATER_CAP_BASE + granaries * GRANARY_WATER_BONUS
}

/// Get the wealth cap for a player (base, no building bonuses in MVP).
pub fn _wealth_cap(_state: &GameState, _player: PlayerId) -> u32 {
    WEALTH_CAP_BASE
}

/// Get the influence cap for a player (base, no building bonuses in MVP).
pub fn _influence_cap(_state: &GameState, _player: PlayerId) -> u32 {
    INFLUENCE_CAP_BASE
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::HexCoord;
    use crate::model::{GameState, Player, PlayerKind, Stockpiles, TerrainType, Tile};
    use crate::scenario::mvp_preset;
    use crate::{PlayerColor, PlayerId, TileId};

    /// Build a minimal deterministic `GameState` for tests.
    fn make_game() -> GameState {
        let cfg = mvp_preset();
        let mut s = GameState::new(cfg, 1);
        let radius = s.scenario.map_radius as u32;

        // Allocate in-map tiles.
        let coords = crate::hex::range(crate::hex::HexCoord { q: 0, r: 0 }, radius);
        for c in coords {
            let id = s.alloc_tile_id();
            s.tiles.push(Tile {
                id,
                coord: c,
                terrain: TerrainType::Dunes,
                is_relic_site: false,
                owner: None,
                improvement: None,
            });
            s.tile_index.insert(c, id);
        }

        // Mark an oasis at the origin.
        let origin_id = s.tile_index[&HexCoord { q: 0, r: 0 }];
        s.tiles[origin_id.0 as usize].terrain = TerrainType::Oasis;

        // Single player with resources.
        let pid = s.alloc_player_id();
        s.players.push(Player {
            id: pid,
            kind: PlayerKind::Human,
            color: PlayerColor::Sand,
            resources: Stockpiles {
                water: 10,
                wealth: 100,
                influence: 50,
            },
            discovered: fxhash::FxHashSet::default(),
            defeated: false,
        });

        // A city at the origin.
        let city_id = s.alloc_city_id();
        s.cities.push(crate::City {
            id: city_id,
            owner: pid,
            tile: origin_id,
            population: 2,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: vec![],
        });

        s
    }

    #[test]
    fn worked_tiles_returns_city_plus_ring() {
        let s = make_game();
        let city_id = CityId(0);
        let tiles = worked_tiles(&s, city_id);
        // Should include city tile + ring(1) tiles that are in the map.
        assert!(tiles.contains(&s.cities[0].tile));
        assert!(tiles.len() > 1, "should include ring-1 neighbors");
    }

    #[test]
    fn building_slots_scales_with_population() {
        let city = crate::City {
            id: CityId(0),
            owner: PlayerId(0),
            tile: TileId(0),
            population: 6,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: vec![],
        };
        assert_eq!(building_slots(&city), 5); // 2 + 6/2
    }

    #[test]
    fn building_slots_zero_pop() {
        let city = crate::City {
            id: CityId(0),
            owner: PlayerId(0),
            tile: TileId(0),
            population: 0,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: vec![],
        };
        assert_eq!(building_slots(&city), 2); // 2 + 0/2
    }

    #[test]
    fn resolve_build_spends_wealth_and_adds_building() {
        let mut s = make_game();
        let wealth_before = s.players[0].resources.wealth;
        let events = resolve_build(&mut s, CityId(0), BuildingKind::Well, PlayerId(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], GameEvent::Built { .. }));
        assert_eq!(
            s.players[0].resources.wealth,
            wealth_before - BUILD_COST[building_index(&BuildingKind::Well)]
        );
        assert!(s.cities[0].buildings.contains(&BuildingKind::Well));
    }

    #[test]
    fn resolve_build_rejects_when_slots_full() {
        let mut s = make_game();
        // Fill all slots: 2 + 2/2 = 3 slots for pop=2.
        s.cities[0].buildings = vec![
            BuildingKind::Well,
            BuildingKind::Market,
            BuildingKind::Granary,
        ];
        let events = resolve_build(&mut s, CityId(0), BuildingKind::Watchtower, PlayerId(0));
        assert!(matches!(
            events[0],
            GameEvent::Rejected {
                reason: RejectReason::Blocked,
                ..
            }
        ));
    }

    #[test]
    fn resolve_build_rejects_when_not_enough_wealth() {
        let mut s = make_game();
        s.players[0].resources.wealth = 0;
        let events = resolve_build(&mut s, CityId(0), BuildingKind::Well, PlayerId(0));
        assert!(matches!(
            events[0],
            GameEvent::Rejected {
                reason: RejectReason::NoResource,
                ..
            }
        ));
    }

    #[test]
    fn resolve_build_rejects_duplicate_building() {
        let mut s = make_game();
        s.cities[0].buildings.push(BuildingKind::Well);
        let events = resolve_build(&mut s, CityId(0), BuildingKind::Well, PlayerId(0));
        assert!(matches!(
            events[0],
            GameEvent::Rejected {
                reason: RejectReason::InvalidState,
                ..
            }
        ));
    }

    #[test]
    fn resolve_build_trade_hub_discount() {
        let mut s = make_game();
        s.cities[0].specialization = Some(CitySpecialization::TradeHub);
        let wealth_before = s.players[0].resources.wealth;
        let events = resolve_build(&mut s, CityId(0), BuildingKind::Market, PlayerId(0));
        assert!(matches!(events[0], GameEvent::Built { .. }));
        let expected_cost =
            BUILD_COST[building_index(&BuildingKind::Market)] - TRADE_HUB_MARKET_DISCOUNT;
        assert_eq!(s.players[0].resources.wealth, wealth_before - expected_cost);
    }

    #[test]
    fn resolve_specialize_requires_pop_3() {
        let mut s = make_game();
        // City has pop=2, needs >= 3.
        let events =
            resolve_specialize(&mut s, CityId(0), CitySpecialization::Fortress, PlayerId(0));
        assert!(matches!(
            events[0],
            GameEvent::Rejected {
                reason: RejectReason::InvalidState,
                ..
            }
        ));
    }

    #[test]
    fn resolve_specialize_rejects_if_already_specialized() {
        let mut s = make_game();
        s.cities[0].population = 5;
        s.cities[0].specialization = Some(CitySpecialization::TradeHub);
        let events =
            resolve_specialize(&mut s, CityId(0), CitySpecialization::Fortress, PlayerId(0));
        assert!(matches!(
            events[0],
            GameEvent::Rejected {
                reason: RejectReason::InvalidState,
                ..
            }
        ));
    }

    #[test]
    fn resolve_specialize_spends_influence() {
        let mut s = make_game();
        s.cities[0].population = 5;
        let infl_before = s.players[0].resources.influence;
        let events =
            resolve_specialize(&mut s, CityId(0), CitySpecialization::Fortress, PlayerId(0));
        assert!(matches!(events[0], GameEvent::Specialized { .. }));
        assert_eq!(
            s.players[0].resources.influence,
            infl_before - SPECIALIZE_COST_INFLUENCE
        );
        assert_eq!(
            s.cities[0].specialization,
            Some(CitySpecialization::Fortress)
        );
    }

    #[test]
    fn process_queue_builds_in_order() {
        let mut s = make_game();
        s.cities[0].queue = vec![
            QueuedOrder::Build(BuildingKind::Well),
            QueuedOrder::Build(BuildingKind::Market),
        ];
        let events = process_queue(&mut s, CityId(0));
        assert_eq!(events.len(), 2);
        assert!(s.cities[0].buildings.contains(&BuildingKind::Well));
        assert!(s.cities[0].buildings.contains(&BuildingKind::Market));
        assert!(s.cities[0].queue.is_empty());
    }

    #[test]
    fn process_queue_stops_on_insufficient_resources() {
        let mut s = make_game();
        // Well costs 8, Market costs 10, total 18. Give only 12.
        s.players[0].resources.wealth = 12;
        s.cities[0].queue = vec![
            QueuedOrder::Build(BuildingKind::Well),
            QueuedOrder::Build(BuildingKind::Market),
        ];
        let events = process_queue(&mut s, CityId(0));
        assert_eq!(events.len(), 1); // Only Well built.
        assert!(s.cities[0].buildings.contains(&BuildingKind::Well));
        assert!(!s.cities[0].buildings.contains(&BuildingKind::Market));
        assert_eq!(s.cities[0].queue.len(), 1); // Market remains.
    }

    #[test]
    fn apply_growth_increments_timer_and_pop() {
        let mut s = make_game();
        // Add a Well building for +2 water.
        s.cities[0].buildings.push(BuildingKind::Well);
        // Set one ring tile to Oasis for extra water.
        let city_coord = s.tiles[s.cities[0].tile.0 as usize].coord;
        for hex in crate::hex::range(city_coord, 1) {
            if let Some(&tid) = s.tile_index.get(&hex) {
                s.tiles[tid.0 as usize].terrain = TerrainType::Oasis;
                break; // just one extra oasis
            }
        }
        // Oasis city: water_yield > GROWTH_WATER_THRESHOLD.
        s.cities[0].growth_timer = GROWTH_PERIOD_TURNS - 1;
        let events = apply_growth(&mut s, CityId(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], GameEvent::Grown { population: 3, .. }));
        assert_eq!(s.cities[0].population, 3);
        assert_eq!(s.cities[0].growth_timer, 0);
    }

    #[test]
    fn growth_timer_resets_on_sub_threshold_water() {
        let mut s = make_game();
        // Place city on a Dunes tile (water yield 0 < threshold).
        s.tiles[s.cities[0].tile.0 as usize].terrain = TerrainType::Dunes;
        s.cities[0].growth_timer = 2;
        let events = apply_growth(&mut s, CityId(0));
        assert!(events.is_empty());
        assert_eq!(s.cities[0].growth_timer, 0, "timer should reset");
    }

    #[test]
    fn unit_cap_calculation() {
        let s = make_game();
        let cap = unit_cap(&s, PlayerId(0));
        // pop = 2, UNIT_CAP_BASE = 2 → cap = 4.
        assert_eq!(cap, 4);
    }

    #[test]
    fn zone_of_control_from_fortress() {
        let mut s = make_game();
        s.cities[0].specialization = Some(CitySpecialization::Fortress);
        let zoc = zone_of_control(&s, PlayerId(0));
        assert!(zoc.contains(&s.cities[0].tile));
        assert!(zoc.len() > 1, "should include ring-1 tiles");
    }

    #[test]
    fn zone_of_control_empty_without_fortress() {
        let s = make_game();
        let zoc = zone_of_control(&s, PlayerId(0));
        assert!(zoc.is_empty());
    }

    #[test]
    fn water_cap_with_granary() {
        let mut s = make_game();
        s.cities[0].buildings.push(BuildingKind::Granary);
        let cap = water_cap(&s, PlayerId(0));
        assert_eq!(cap, WATER_CAP_BASE + GRANARY_WATER_BONUS);
    }

    #[test]
    fn water_cap_without_granary() {
        let s = make_game();
        let cap = water_cap(&s, PlayerId(0));
        assert_eq!(cap, WATER_CAP_BASE);
    }
}
