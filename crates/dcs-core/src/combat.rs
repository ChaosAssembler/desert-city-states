//! Combat resolution: auto-combat, raid contests, and city sieges.
//!
//! All randomness flows through `state.rng` (ADR-0006). Emits
//! `GameEvent::Combat` / `RouteRaided` / `CityRaided`.
//!
//! The core resolution functions (`resolve_combat`, `resolve_raid_contest`,
//! `resolve_city_raid`) are methods on [`GameState`](crate::model::GameState).
//! This module retains the constants and the pure `positioning_attacker`
//! utility.

use crate::model::TerrainType;
// ---------------------------------------------------------------------------
// Constants (spec §4)
// ---------------------------------------------------------------------------

/// Attacker from advantageous terrain (e.g. Ridge vs Dunes) gains +25 %.
pub const FLANK_POSITIONING_BONUS: f32 = 0.25;
/// Attacker on Salt Flats (exposed) takes a 10 % penalty.
pub const EXPOSED_POSITIONING_MULT: f32 = 0.90;
/// Raider on Ridges gains +25 % for route raids.
pub const ROUGH_RAIDER_BONUS: f32 = 1.25;
/// Fortress city specialization adds this to city defense.
pub const FORTRESS_CITY_DEF: i8 = 3;

// ---------------------------------------------------------------------------
// Positioning (spec §6.2)
// ---------------------------------------------------------------------------

/// Compute the attacker's positioning multiplier based on terrain.
///
/// - Ridge attacker vs non-Ridge defender → `1.0 + FLANK_POSITIONING_BONUS`
/// - Salt Flats attacker → `EXPOSED_POSITIONING_MULT`
/// - Otherwise → `1.0`
pub fn positioning_attacker(attacker_terrain: TerrainType, defender_terrain: TerrainType) -> f32 {
    if attacker_terrain == TerrainType::Ridges && defender_terrain != TerrainType::Ridges {
        return 1.0 + FLANK_POSITIONING_BONUS;
    }
    if attacker_terrain == TerrainType::SaltFlats {
        return EXPOSED_POSITIONING_MULT;
    }
    1.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use crate::GameState;
    use crate::hex::HexCoord;
    use crate::model::Stockpiles;
    use crate::scenario::ScenarioConfig;
    use crate::test_harness;
    use crate::{
        AiPersonality, CityId, Difficulty, GameEvent, PlayerId, PlayerKind, RouteId, RouteStatus,
        TileId, UnitId, UnitKind,
    };

    // ---- test harness ------------------------------------------------------

    /// Build a minimal game state for combat tests with optional terrain
    /// overrides applied to specific hex coordinates.
    fn make_game(seed: u64, terrain_overrides: &[(HexCoord, TerrainType)]) -> GameState {
        let cfg = ScenarioConfig::mvp_preset();
        let mut s = GameState::new(cfg, seed);
        let radius = s.scenario.map_radius as u32;
        test_harness::allocate_hex_grid(&mut s, radius);

        // Apply terrain overrides
        for &(coord, terrain) in terrain_overrides {
            test_harness::mark_terrain(&mut s, coord, terrain);
        }

        // Two players
        test_harness::create_player(&mut s, PlayerKind::Human, Stockpiles::default());
        test_harness::create_player(
            &mut s,
            PlayerKind::Ai {
                personality: AiPersonality::Expansionist,
                difficulty: Difficulty::Normal,
            },
            Stockpiles::default(),
        );

        s
    }

    /// Helper: add a unit to the game state and return its id.
    fn add_unit(
        state: &mut GameState,
        owner: PlayerId,
        kind: UnitKind,
        tile: TileId,
        hp: u32,
    ) -> UnitId {
        test_harness::create_unit_with_hp(state, owner, kind, tile, hp)
    }

    // ---- positioning tests --------------------------------------------------

    #[test]
    fn positioning_ridge_vs_dunes() {
        let pos = positioning_attacker(TerrainType::Ridges, TerrainType::Dunes);
        assert!((pos - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn positioning_ridge_vs_ridge_no_bonus() {
        // Ridge vs Ridge → no flank bonus (both on equal ground).
        let pos = positioning_attacker(TerrainType::Ridges, TerrainType::Ridges);
        assert!((pos - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn positioning_ridge_vs_oasis() {
        let pos = positioning_attacker(TerrainType::Ridges, TerrainType::Oasis);
        assert!((pos - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn positioning_salt_flats_exposed() {
        let pos = positioning_attacker(TerrainType::SaltFlats, TerrainType::Dunes);
        assert!((pos - 0.90).abs() < f32::EPSILON);
    }

    #[test]
    fn positioning_default_no_mod() {
        let pos = positioning_attacker(TerrainType::Dunes, TerrainType::Oasis);
        assert!((pos - 1.0).abs() < f32::EPSILON);
    }

    // ---- basic combat tests -------------------------------------------------

    #[test]
    fn combat_one_side_dies() {
        let origin = HexCoord { q: 0, r: 0 };
        let neighbor = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let origin_tile = s.tile_index[&origin];
        let neighbor_tile = s.tile_index[&neighbor];

        let atk = add_unit(&mut s, PlayerId(0), UnitKind::Raider, origin_tile, 4);
        let def = add_unit(&mut s, PlayerId(1), UnitKind::Scout, neighbor_tile, 1);

        let events = s.resolve_combat(atk, def, origin_tile);

        assert_eq!(events.len(), 1);
        if let GameEvent::Combat {
            attacker,
            defender,
            retreated,
            ..
        } = &events[0]
        {
            assert_eq!(*attacker, atk);
            assert_eq!(*defender, def);
            // Defender had 1 HP → must be destroyed (or attacker retreated).
            let defender_alive = s.units.iter().any(|u| u.id == def);
            assert!(
                !defender_alive || *retreated,
                "defender with 1 HP should be destroyed or attacker retreated"
            );
        } else {
            panic!("expected GameEvent::Combat");
        }
    }

    #[test]
    fn combat_attacker_retreats_at_low_hp() {
        let origin = HexCoord { q: 0, r: 0 };
        let neighbor = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let origin_tile = s.tile_index[&origin];
        let neighbor_tile = s.tile_index[&neighbor];

        // Attacker at 1 HP; on first damage it must retreat.
        let atk = add_unit(&mut s, PlayerId(0), UnitKind::Raider, neighbor_tile, 1);
        let def = add_unit(&mut s, PlayerId(1), UnitKind::Scout, neighbor_tile, 3);

        let events = s.resolve_combat(atk, def, origin_tile);

        assert_eq!(events.len(), 1);
        if let GameEvent::Combat { retreated, .. } = &events[0] {
            if *retreated {
                let attacker = s.units.iter().find(|u| u.id == atk).unwrap();
                assert_eq!(attacker.tile, origin_tile, "must retreat to origin");
                assert_eq!(attacker.moves_left, 0, "moves_left must be 0 after retreat");
                assert_eq!(attacker.hp, 1, "hp must be 1 after retreat");
            } else {
                // Attacker won every exchange — defender must be gone.
                assert!(
                    !s.units.iter().any(|u| u.id == def),
                    "defender should be destroyed"
                );
            }
        } else {
            panic!("expected GameEvent::Combat");
        }
    }

    #[test]
    fn combat_self_combat_is_noop() {
        let mut s = make_game(42, &[]);
        let tile = s.tile_index[&HexCoord { q: 0, r: 0 }];
        let unit = add_unit(&mut s, PlayerId(0), UnitKind::Raider, tile, 4);

        let events = s.resolve_combat(unit, unit, tile);
        assert!(events.is_empty(), "self-combat should produce no events");
    }

    #[test]
    fn combat_missing_unit_is_noop() {
        let mut s = make_game(42, &[]);
        let tile = s.tile_index[&HexCoord { q: 0, r: 0 }];
        let atk = add_unit(&mut s, PlayerId(0), UnitKind::Raider, tile, 4);

        let events = s.resolve_combat(atk, UnitId(999), tile);
        assert!(events.is_empty(), "missing unit should produce no events");
    }

    // ---- terrain defense tests ----------------------------------------------

    #[test]
    fn ridge_defender_shifts_odds() {
        // With a Ridge defender (+2 mod), the attacker has worse odds.
        // We just verify the combat completes and emits the right event.
        let origin = HexCoord { q: 0, r: 0 };
        let ridge = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[(ridge, TerrainType::Ridges)]);
        let origin_tile = s.tile_index[&origin];
        let ridge_tile = s.tile_index[&ridge];

        let atk = add_unit(&mut s, PlayerId(0), UnitKind::Raider, origin_tile, 4);
        let def = add_unit(&mut s, PlayerId(1), UnitKind::Scout, ridge_tile, 3);

        let events = s.resolve_combat(atk, def, origin_tile);

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], GameEvent::Combat { .. }));
    }

    // ---- raid tests ---------------------------------------------------------

    #[test]
    fn raid_no_defender_cascades_to_threatened() {
        let route_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let route_tile = s.tile_index[&route_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        // Create an Active route.
        let route_id = RouteId(0);
        s.routes.push(crate::model::CaravanRoute {
            id: route_id,
            owner: PlayerId(1),
            endpoints: (CityId(0), CityId(1)),
            path: vec![route_tile],
            status: RouteStatus::Active,
            length: 1,
            upkeep: 1,
            consecutive_threatened: 0,
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = s.resolve_raid_contest(raider, route_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::RouteRaided { route, severed, .. } = &events[0] {
            assert_eq!(*route, route_id);
            assert!(
                !*severed,
                "first raid should produce Threatened, not Severed"
            );
            assert_eq!(
                s.routes[route_id.0 as usize].status,
                RouteStatus::Threatened
            );
        } else {
            panic!("expected GameEvent::RouteRaided");
        }
    }

    #[test]
    fn raid_twice_severs() {
        let route_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let route_tile = s.tile_index[&route_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        let route_id = RouteId(0);
        s.routes.push(crate::model::CaravanRoute {
            id: route_id,
            owner: PlayerId(1),
            endpoints: (CityId(0), CityId(1)),
            path: vec![route_tile],
            status: RouteStatus::Active,
            length: 1,
            upkeep: 1,
            consecutive_threatened: 0,
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        // First raid: Active → Threatened.
        let _ = s.resolve_raid_contest(raider, route_id);
        assert_eq!(
            s.routes[route_id.0 as usize].status,
            RouteStatus::Threatened
        );

        // Second raid: Threatened → Severed.
        let events = s.resolve_raid_contest(raider, route_id);
        assert_eq!(events.len(), 1);
        if let GameEvent::RouteRaided { severed, .. } = &events[0] {
            assert!(*severed, "second raid should sever the route");
        }
        assert_eq!(s.routes[route_id.0 as usize].status, RouteStatus::Severed);
    }

    #[test]
    fn raid_with_guard_stat_contest() {
        let route_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let route_tile = s.tile_index[&route_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        let route_id = RouteId(0);
        s.routes.push(crate::model::CaravanRoute {
            id: route_id,
            owner: PlayerId(1),
            endpoints: (CityId(0), CityId(1)),
            path: vec![route_tile],
            status: RouteStatus::Active,
            length: 1,
            upkeep: 1,
            consecutive_threatened: 0,
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);
        // Guard on the route tile — should trigger stat contest.
        let _guard = add_unit(&mut s, PlayerId(1), UnitKind::CaravanGuard, route_tile, 5);

        let events = s.resolve_raid_contest(raider, route_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::RouteRaided { route, severed, .. } = &events[0] {
            assert_eq!(*route, route_id);
            // Guard present → stat contest. Either outcome is valid.
            let status = s.routes[route_id.0 as usize].status;
            if *severed {
                assert_eq!(status, RouteStatus::Severed);
            } else {
                assert_eq!(status, RouteStatus::Active);
            }
        } else {
            panic!("expected GameEvent::RouteRaided");
        }
    }

    #[test]
    fn raid_guard_adjacent_counts() {
        let route_tile_coord = HexCoord { q: 0, r: 0 };
        let guard_tile_coord = HexCoord { q: 1, r: 0 }; // adjacent to route tile
        let raider_tile_coord = HexCoord { q: -1, r: 0 };

        let mut s = make_game(42, &[]);
        let route_tile = s.tile_index[&route_tile_coord];
        let guard_tile = s.tile_index[&guard_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        let route_id = RouteId(0);
        s.routes.push(crate::model::CaravanRoute {
            id: route_id,
            owner: PlayerId(1),
            endpoints: (CityId(0), CityId(1)),
            path: vec![route_tile],
            status: RouteStatus::Active,
            length: 1,
            upkeep: 1,
            consecutive_threatened: 0,
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);
        // Guard ADJACENT to route tile — should still trigger stat contest.
        let _guard = add_unit(&mut s, PlayerId(1), UnitKind::CaravanGuard, guard_tile, 5);

        let events = s.resolve_raid_contest(raider, route_id);

        assert_eq!(events.len(), 1);
        // With a guard (even adjacent), the route should not auto-cascade
        // unconditionally — it's a stat contest.
        if let GameEvent::RouteRaided { .. } = &events[0] {
            // Event emitted correctly.
        } else {
            panic!("expected GameEvent::RouteRaided");
        }
    }

    #[test]
    fn raid_missing_unit_warns() {
        let mut s = make_game(42, &[]);
        let events = s.resolve_raid_contest(UnitId(999), RouteId(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], GameEvent::Warn { .. }));
    }

    // ---- city raid tests ----------------------------------------------------

    #[test]
    fn city_raid_fortress_defense() {
        let city_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let city_tile = s.tile_index[&city_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(1),
            tile: city_tile,
            population: 3,
            specialization: Some(crate::CitySpecialization::Fortress),
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = s.resolve_city_raid(raider, city_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::CityRaided { city, pop_lost, .. } = &events[0] {
            assert_eq!(*city, city_id);
            // Fortress (+3 def) makes the city harder to raid.
            // Verify the event is emitted correctly.
            let city_data = &s.cities[city_id.0 as usize];
            assert!(city_data.population <= 3, "population should not increase");
            let _ = pop_lost; // value depends on RNG
        } else {
            panic!("expected GameEvent::CityRaided");
        }
    }

    #[test]
    fn city_raid_population_reduces() {
        let city_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let city_tile = s.tile_index[&city_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        // Non-Fortress city on Dunes → city_def = 0, odds ≈ 1.0.
        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(1),
            tile: city_tile,
            population: 5,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = s.resolve_city_raid(raider, city_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::CityRaided { pop_lost, .. } = &events[0] {
            let city = &s.cities[city_id.0 as usize];
            if *pop_lost > 0 {
                assert!(
                    city.population < 5,
                    "population should decrease after a successful raid"
                );
            }
        } else {
            panic!("expected GameEvent::CityRaided");
        }
    }

    #[test]
    fn city_raid_capture_at_zero_pop() {
        let city_tile_coord = HexCoord { q: 0, r: 0 };

        let mut s = make_game(42, &[]);
        let city_tile = s.tile_index[&city_tile_coord];

        // Non-Fortress on Dunes, population 1, no garrison → odds = 1.0.
        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(1),
            tile: city_tile,
            population: 1,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        // Raider ON the city tile → capture when pop hits 0.
        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, city_tile, 4);

        let events = s.resolve_city_raid(raider, city_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::CityRaided { pop_lost, .. } = &events[0] {
            assert!(*pop_lost >= 1, "should lose at least 1 pop");
            assert_eq!(
                s.cities[city_id.0 as usize].population, 0,
                "population should be 0"
            );
            assert_eq!(
                s.cities[city_id.0 as usize].owner,
                PlayerId(0),
                "city should be captured by raider's owner"
            );
        } else {
            panic!("expected GameEvent::CityRaided");
        }
    }

    #[test]
    fn city_raid_no_capture_when_not_on_tile() {
        let city_tile_coord = HexCoord { q: 0, r: 0 };
        let raider_tile_coord = HexCoord { q: 1, r: 0 };

        let mut s = make_game(42, &[]);
        let city_tile = s.tile_index[&city_tile_coord];
        let raider_tile = s.tile_index[&raider_tile_coord];

        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(1),
            tile: city_tile,
            population: 1,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        // Raider NOT on the city tile.
        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = s.resolve_city_raid(raider, city_id);

        assert_eq!(events.len(), 1);
        if let GameEvent::CityRaided { pop_lost, .. } = &events[0] {
            if *pop_lost >= 1 {
                assert_eq!(
                    s.cities[city_id.0 as usize].population, 0,
                    "population should be 0"
                );
                // NOT captured because raider is not on the city tile.
                assert_eq!(
                    s.cities[city_id.0 as usize].owner,
                    PlayerId(1),
                    "city should NOT be captured"
                );
            }
        } else {
            panic!("expected GameEvent::CityRaided");
        }
    }

    #[test]
    fn city_raid_own_city_warns() {
        let city_tile_coord = HexCoord { q: 0, r: 0 };

        let mut s = make_game(42, &[]);
        let city_tile = s.tile_index[&city_tile_coord];

        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(0),
            tile: city_tile,
            population: 3,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, city_tile, 4);

        let events = s.resolve_city_raid(raider, city_id);

        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], GameEvent::Warn { .. }),
            "raiding own city should warn"
        );
    }

    #[test]
    fn city_raid_missing_inputs_warns() {
        let mut s = make_game(42, &[]);
        let events = s.resolve_city_raid(UnitId(999), CityId(999));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], GameEvent::Warn { .. }));
    }
}
