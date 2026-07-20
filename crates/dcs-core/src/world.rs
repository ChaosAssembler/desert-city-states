//! City and unit gameplay logic: building, training, specialization, growth,
//! production queues, and zone-of-control calculations.
//!
//! The free functions that used to live here have been moved onto
//! [`crate::model::GameState`] (and [`crate::model::City`]) as methods. This
//! module now only hosts their tests.

#[cfg(test)]
mod tests {
    use crate::hex::ORIGIN;
    use crate::model::GameState;
    use crate::model::{
        BUILD_COST, GRANARY_WATER_BONUS, GROWTH_PERIOD_TURNS, SPECIALIZE_COST_INFLUENCE,
        TRADE_HUB_MARKET_DISCOUNT, WATER_CAP_BASE,
    };
    use crate::test_harness;
    use crate::traits::BuildingKindExt;
    use crate::{
        BuildingKind, City, CityId, CitySpecialization, GameEvent, PlayerId, QueuedOrder,
        RejectReason, Stockpiles, TerrainType, TileId,
    };
    use std::collections::VecDeque;

    /// Build a minimal deterministic `GameState` for tests.
    fn make_game() -> GameState {
        let mut s = test_harness::minimal_state();
        // minimal_state already creates 1 Human player with wealth=100, influence=50
        // and marks origin as Oasis
        let pid = PlayerId(0);
        let origin = ORIGIN;
        test_harness::create_city(&mut s, pid, origin, 2);
        s
    }

    #[test]
    fn worked_tiles_returns_city_plus_ring() {
        let s = make_game();
        let city_id = CityId(0);
        let tiles = s.worked_tiles(city_id);
        // Should include city tile + ring(1) tiles that are in the map.
        assert!(tiles.contains(&s.cities[0].tile));
        assert!(tiles.len() > 1, "should include ring-1 neighbors");
    }

    #[test]
    fn building_slots_scales_with_population() {
        let city = City {
            id: CityId(0),
            owner: PlayerId(0),
            tile: TileId(0),
            population: 6,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        };
        assert_eq!(city.building_slots(), 5); // 2 + 6/2
    }

    #[test]
    fn building_slots_zero_pop() {
        let city = City {
            id: CityId(0),
            owner: PlayerId(0),
            tile: TileId(0),
            population: 0,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        };
        assert_eq!(city.building_slots(), 2); // 2 + 0/2
    }

    #[test]
    fn resolve_build_spends_wealth_and_adds_building() {
        let mut s = make_game();
        let wealth_before = s.players[0].resources.wealth;
        let events = s.resolve_build(CityId(0), BuildingKind::Well, PlayerId(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], GameEvent::Built { .. }));
        assert_eq!(
            s.players[0].resources.wealth,
            wealth_before - BUILD_COST[BuildingKind::Well.index()]
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
        let events = s.resolve_build(CityId(0), BuildingKind::Watchtower, PlayerId(0));
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
        let events = s.resolve_build(CityId(0), BuildingKind::Well, PlayerId(0));
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
        let events = s.resolve_build(CityId(0), BuildingKind::Well, PlayerId(0));
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
        let events = s.resolve_build(CityId(0), BuildingKind::Market, PlayerId(0));
        assert!(matches!(events[0], GameEvent::Built { .. }));
        let expected_cost = BUILD_COST[BuildingKind::Market.index()] - TRADE_HUB_MARKET_DISCOUNT;
        assert_eq!(s.players[0].resources.wealth, wealth_before - expected_cost);
    }

    #[test]
    fn resolve_specialize_requires_pop_3() {
        let mut s = make_game();
        // City has pop=2, needs >= 3.
        let events = s.resolve_specialize(CityId(0), CitySpecialization::Fortress, PlayerId(0));
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
        let events = s.resolve_specialize(CityId(0), CitySpecialization::Fortress, PlayerId(0));
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
        let events = s.resolve_specialize(CityId(0), CitySpecialization::Fortress, PlayerId(0));
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
        s.cities[0].queue = VecDeque::from([
            QueuedOrder::Build(BuildingKind::Well),
            QueuedOrder::Build(BuildingKind::Market),
        ]);
        let events = s.process_queue(CityId(0));
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
        s.cities[0].queue = VecDeque::from([
            QueuedOrder::Build(BuildingKind::Well),
            QueuedOrder::Build(BuildingKind::Market),
        ]);
        let events = s.process_queue(CityId(0));
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
        for hex in city_coord.range(1) {
            if let Some(&tid) = s.tile_index.get(&hex) {
                s.tiles[tid.0 as usize].terrain = TerrainType::Oasis;
                break; // just one extra oasis
            }
        }
        // Oasis city: water_yield > GROWTH_WATER_THRESHOLD.
        s.cities[0].growth_timer = GROWTH_PERIOD_TURNS - 1;
        let events = s.apply_growth(CityId(0));
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
        let events = s.apply_growth(CityId(0));
        assert!(events.is_empty());
        assert_eq!(s.cities[0].growth_timer, 0, "timer should reset");
    }

    #[test]
    fn unit_cap_calculation() {
        let s = make_game();
        let cap = s.unit_cap(PlayerId(0));
        // pop = 2, UNIT_CAP_BASE = 2 → cap = 4.
        assert_eq!(cap, 4);
    }

    #[test]
    fn zone_of_control_from_fortress() {
        let mut s = make_game();
        s.cities[0].specialization = Some(CitySpecialization::Fortress);
        let zoc = s.zone_of_control(PlayerId(0));
        assert!(zoc.contains(&s.cities[0].tile));
        assert!(zoc.len() > 1, "should include ring-1 tiles");
    }

    #[test]
    fn zone_of_control_empty_without_fortress() {
        let s = make_game();
        let zoc = s.zone_of_control(PlayerId(0));
        assert!(zoc.is_empty());
    }

    #[test]
    fn water_cap_with_granary() {
        let mut s = make_game();
        s.cities[0].buildings.push(BuildingKind::Granary);
        let cap = s.water_cap(PlayerId(0));
        assert_eq!(cap, WATER_CAP_BASE + GRANARY_WATER_BONUS);
    }

    #[test]
    fn water_cap_without_granary() {
        let s = make_game();
        let cap = s.water_cap(PlayerId(0));
        assert_eq!(cap, WATER_CAP_BASE);
    }
}
