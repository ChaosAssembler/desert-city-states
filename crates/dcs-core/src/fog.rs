//! Fog of War: per-player visibility model.
//!
//! Each [`Player`](crate::model::Player) maintains a
//! [`discovered: FxHashSet<TileId>`](crate::model::Player::discovered) that
//! tracks which tiles have been revealed. This module owns all fog *data* and
//! reveal logic, called from the resolver on move/found/route and from
//! [`GameState::advance_turn`](crate::model::GameState::advance_turn) for building/specialization
//! re-reveal.
//!
//! # Reveal sources & radii (gameplay-fog-of-war spec §4)
//!
//! | Source | Radius |
//! |---|---|
//! | Scout | 3 |
//! | Caravan Guard | 1 |
//! | Raider | 2 |
//! | City (base) | 2 |
//! | City = Scholar Outpost | +1 (→3) |
//! | Watchtower building | 2 around the tower tile |
//!
//! Fog *methods* live on [`GameState`](crate::model::GameState); see
//! [`crate::model::GameState::reveal`], [`crate::model::GameState::city_sight`], etc.

// ---------------------------------------------------------------------------
// Reveal radius constants (fog-of-war spec §4)
// ---------------------------------------------------------------------------

/// Scout sight radius — most reveal of any unit.
pub const SIGHT_SCOUT: u32 = 3;
/// Caravan Guard sight radius — minimal.
pub const SIGHT_GUARD: u32 = 1;
/// Raider sight radius.
pub const SIGHT_RAIDER: u32 = 2;
/// Base city sight radius around the city tile.
pub const SIGHT_CITY_BASE: u32 = 2;
/// Scholar Outpost bonus added to city base sight.
pub const SIGHT_SCHOLAR_BONUS: u32 = 1;
/// Watchtower sight radius around the tower tile.
pub const SIGHT_WATCHTOWER: u32 = 2;
/// Initial fog reveal at world generation (same as city base).
pub const SIGHT_START: u32 = SIGHT_CITY_BASE;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use crate::model::{GameState, Stockpiles};
    use crate::scenario::ScenarioConfig;
    use crate::test_harness;
    use crate::traits::UnitKindExt;
    use crate::{CityId, CitySpecialization, PlayerId, RouteId, UnitId, UnitKind};

    /// Build a minimal game state with a small hex map for fog tests.
    fn make_game() -> GameState {
        let cfg = ScenarioConfig::mvp_preset();
        let mut s = GameState::new(cfg, 1);
        let radius = s.scenario.map_radius as u32;
        test_harness::allocate_hex_grid(&mut s, radius);

        // Two players
        test_harness::create_player(&mut s, crate::PlayerKind::Human, Stockpiles::default());
        test_harness::create_player(
            &mut s,
            crate::PlayerKind::Ai {
                personality: crate::AiPersonality::Expansionist,
                difficulty: crate::Difficulty::Normal,
            },
            Stockpiles::default(),
        );

        // A scout for player 0 on the origin tile
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        test_harness::create_unit_with_hp(&mut s, PlayerId(0), UnitKind::Scout, origin_tile, 3);

        s
    }

    #[test]
    fn sight_of_matches_spec() {
        assert_eq!(UnitKind::Scout.sight(), SIGHT_SCOUT);
        assert_eq!(UnitKind::Scout.sight(), 3);
        assert_eq!(UnitKind::CaravanGuard.sight(), SIGHT_GUARD);
        assert_eq!(UnitKind::CaravanGuard.sight(), 1);
        assert_eq!(UnitKind::Raider.sight(), SIGHT_RAIDER);
        assert_eq!(UnitKind::Raider.sight(), 2);
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(SIGHT_SCOUT, 3);
        assert_eq!(SIGHT_GUARD, 1);
        assert_eq!(SIGHT_RAIDER, 2);
        assert_eq!(SIGHT_CITY_BASE, 2);
        assert_eq!(SIGHT_SCHOLAR_BONUS, 1);
        assert_eq!(SIGHT_WATCHTOWER, 2);
        assert_eq!(SIGHT_START, SIGHT_CITY_BASE);
    }

    #[test]
    fn reveal_adds_tiles() {
        let mut s = make_game();
        let player = PlayerId(0);
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        let before = s.players[player.0 as usize].discovered.len();
        let newly = s.reveal(player, origin_tile, 1);
        let after = s.players[player.0 as usize].discovered.len();
        assert!(after > before, "reveal should add tiles");
        assert!(!newly.is_empty(), "newly should be non-empty");
        // range(center, 1) = 1 + 3*1*2 = 7 tiles, but some may be off-map.
        assert!(newly.len() <= 7, "at most 7 tiles in range(1) of origin");
    }

    #[test]
    fn reveal_is_idempotent() {
        let mut s = make_game();
        let player = PlayerId(0);
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        let before = s.players[player.0 as usize].discovered.len();
        let newly1 = s.reveal(player, origin_tile, 1);
        let newly2 = s.reveal(player, origin_tile, 1);
        assert!(newly2.is_empty(), "second reveal should be a no-op");
        assert_eq!(
            s.players[player.0 as usize].discovered.len(),
            before + newly1.len()
        );
    }

    #[test]
    fn is_tile_visible_after_reveal() {
        let mut s = make_game();
        let player = PlayerId(0);
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        assert!(!s.is_tile_visible(player, origin_tile));
        s.reveal(player, origin_tile, 1);
        assert!(s.is_tile_visible(player, origin_tile));
    }

    #[test]
    fn is_tile_visible_false_for_other_player() {
        let mut s = make_game();
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        s.reveal(PlayerId(0), origin_tile, 1);
        assert!(
            !s.is_tile_visible(PlayerId(1), origin_tile),
            "other player should not see it"
        );
    }

    #[test]
    fn is_unit_visible_depends_on_tile() {
        let mut s = make_game();
        let scout = UnitId(0);
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        // Player 0 sees own scout (tile not yet revealed, but own-unit check
        // still uses the same discovered set).
        // Reveal the scout's tile for player 0.
        s.reveal(PlayerId(0), origin_tile, 0);
        assert!(s.is_unit_visible(PlayerId(0), scout));
        // Player 1 hasn't revealed that tile.
        assert!(!s.is_unit_visible(PlayerId(1), scout));
    }

    #[test]
    fn city_sight_base() {
        let mut s = make_game();
        // No cities yet, so just test the function doesn't panic with a valid city.
        // Instead, add a test city.
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(0),
            tile: origin_tile,
            population: 1,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });
        assert_eq!(s.city_sight(city_id), SIGHT_CITY_BASE);
    }

    #[test]
    fn city_sight_scholar_bonus() {
        let mut s = make_game();
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        let city_id = CityId(0);
        s.cities.push(crate::model::City {
            id: city_id,
            owner: PlayerId(0),
            tile: origin_tile,
            population: 1,
            specialization: Some(CitySpecialization::ScholarOutpost),
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });
        assert_eq!(s.city_sight(city_id), SIGHT_CITY_BASE + SIGHT_SCHOLAR_BONUS);
    }

    #[test]
    fn reveal_from_unit_uses_unit_sight() {
        let mut s = make_game();
        let scout = UnitId(0);
        let before = s.players[0].discovered.len();
        s.reveal_from_unit(scout);
        let after = s.players[0].discovered.len();
        // Scout has sight 3, so range(center, 3) is quite large.
        assert!(after > before, "reveal_from_unit should reveal tiles");
    }

    #[test]
    fn is_route_visible_false_when_empty() {
        let mut s = make_game();
        // No routes exist, but let's add one for testing.
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        let neighbor = crate::hex::ORIGIN.neighbors()[0];
        let neighbor_tile = s.tile_index[&neighbor];
        let route_id = RouteId(0);
        s.routes.push(crate::model::CaravanRoute {
            id: route_id,
            owner: PlayerId(0),
            endpoints: (CityId(0), CityId(1)),
            path: vec![origin_tile, neighbor_tile],
            status: crate::RouteStatus::Active,
            length: 2,
            upkeep: 1,
            consecutive_threatened: 0,
        });
        // Neither tile is discovered for player 1.
        assert!(!s.is_route_visible(PlayerId(1), route_id));
        // Reveal the origin tile for player 0 → route becomes visible.
        s.reveal(PlayerId(0), origin_tile, 0);
        assert!(s.is_route_visible(PlayerId(0), route_id));
    }
}
