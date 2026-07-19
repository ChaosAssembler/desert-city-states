//! Fog of War: per-player visibility model.
//!
//! Each [`Player`](crate::model::Player) maintains a
//! [`discovered: FxHashSet<TileId>`](crate::model::Player::discovered) that
//! tracks which tiles have been revealed. This module owns all fog *data* and
//! reveal logic, called from the resolver on move/found/route and from
//! [`advance_turn`](crate::turn::advance_turn) for building/specialization
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


use crate::model::GameState;
use crate::{
    BuildingKind, CityId, CitySpecialization, GameEvent, PlayerId, RouteId, TileId, UnitId,
    UnitKind,
};
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
// Sight helpers
// ---------------------------------------------------------------------------

/// Return the sight radius for a unit kind.
pub fn sight_of(kind: UnitKind) -> u32 {
    match kind {
        UnitKind::Scout => SIGHT_SCOUT,
        UnitKind::CaravanGuard => SIGHT_GUARD,
        UnitKind::Raider => SIGHT_RAIDER,
    }
}

/// Compute the city's sight radius based on buildings and specialization.
///
/// Base sight is [`SIGHT_CITY_BASE`]; Scholar Outpost adds
/// [`SIGHT_SCHOLAR_BONUS`]. Watchtower is handled separately in
/// [`refresh_city_fog`] (it reveals around its own tile).
pub fn city_sight(state: &GameState, city: CityId) -> u32 {
    let c = &state.cities[city.0 as usize];
    let mut radius = SIGHT_CITY_BASE;
    if c.specialization == Some(CitySpecialization::ScholarOutpost) {
        radius += SIGHT_SCHOLAR_BONUS;
    }
    radius
}

// ---------------------------------------------------------------------------
// Reveal core
// ---------------------------------------------------------------------------

/// Reveal `range(center, r)` tiles into `player.discovered`.
///
/// Returns the set of *newly* revealed tiles (not already in the discovered
/// set). Emits a [`GameEvent::Revealed`] if any new tiles were uncovered.
/// Idempotent: calling with the same center/radius twice is a no-op the
/// second time.
pub fn reveal(state: &mut GameState, player: PlayerId, center: TileId, r: u32) -> Vec<TileId> {
    let center_coord = state.tiles[center.0 as usize].coord;
    let mut newly: Vec<TileId> = Vec::new();
    for hex in center_coord.range(r) {
        if let Some(&tid) = state.tile_index.get(&hex) {
            if state.players[player.0 as usize].discovered.insert(tid) {
                newly.push(tid);
            }
        }
    }
    if !newly.is_empty() {
        state.log.push(GameEvent::Revealed {
            player,
            tiles: newly.clone(),
        });
    }
    newly
}

/// Reveal fog from a unit's current position using its sight radius.
///
/// Called by the resolver after a unit moves (reveal-on-move, spec §6.2) and
/// after training a new unit.
pub fn reveal_from_unit(state: &mut GameState, unit_id: UnitId) {
    let u = &state.units[unit_id.0 as usize];
    let actor = u.owner;
    let tile = u.tile;
    let kind = u.kind;
    reveal(state, actor, tile, sight_of(kind));
}

// ---------------------------------------------------------------------------
// Visibility queries
// ---------------------------------------------------------------------------

/// Is `tile` currently in `player`'s discovered set?
pub fn is_tile_visible(state: &GameState, player: PlayerId, tile: TileId) -> bool {
    state.players[player.0 as usize].discovered.contains(&tile)
}

/// A unit is visible only if its **current** tile is discovered by the viewer.
///
/// Enemy units in fog are **hidden** — including from the AI (ADR-0004 purity).
pub fn is_unit_visible(state: &GameState, viewer: PlayerId, unit_id: UnitId) -> bool {
    let u = &state.units[unit_id.0 as usize];
    is_tile_visible(state, viewer, u.tile)
}

/// A city is visible if ANY of its tiles (city tile + worked ring) are
/// discovered.
///
/// Once seen, stays visible as a memory marker (spec §6.3): the city remains
/// displayed at its remembered location, but its dynamic state is only
/// live-updated while currently observed.
pub fn is_city_visible(state: &GameState, viewer: PlayerId, city_id: CityId) -> bool {
    let c = &state.cities[city_id.0 as usize];
    // Check city tile.
    if is_tile_visible(state, viewer, c.tile) {
        return true;
    }
    // Check worked ring (city tile + ring(1)).
    let city_coord = state.tiles[c.tile.0 as usize].coord;
    for hex in city_coord.range(1) {
        if let Some(&tid) = state.tile_index.get(&hex) {
            if is_tile_visible(state, viewer, tid) {
                return true;
            }
        }
    }
    false
}

/// A route is visible if ANY path tile is discovered.
///
/// Static memory marker: once seen, stays visible (spec §6.3).
pub fn is_route_visible(state: &GameState, viewer: PlayerId, route_id: RouteId) -> bool {
    let r = &state.routes[route_id.0 as usize];
    for &tid in &r.path {
        if is_tile_visible(state, viewer, tid) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Re-scan helpers
// ---------------------------------------------------------------------------

/// Re-scan all owned cities and their buildings/specializations to refresh fog.
///
/// Called from [`advance_turn`](crate::turn::advance_turn) to handle late
/// building/specialization (Watchtower/Scholar re-reveal, spec §6.2).
pub fn refresh_city_fog(state: &mut GameState) {
    // First pass: collect data without holding a borrow on `state`.
    let cities_data: Vec<(PlayerId, TileId, u32, bool)> = state
        .cities
        .iter()
        .map(|c| {
            // Inline city_sight logic to avoid reborrowing state.
            let mut sight = SIGHT_CITY_BASE;
            if c.specialization == Some(CitySpecialization::ScholarOutpost) {
                sight += SIGHT_SCHOLAR_BONUS;
            }
            let has_watchtower = c.buildings.contains(&BuildingKind::Watchtower);
            (c.owner, c.tile, sight, has_watchtower)
        })
        .collect();

    // Second pass: reveal fog using the collected data.
    for (owner, tile, sight, has_watchtower) in cities_data {
        reveal(state, owner, tile, sight);
        if has_watchtower {
            reveal(state, owner, tile, SIGHT_WATCHTOWER);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use crate::model::{GameState, Stockpiles};
    use crate::scenario::mvp_preset;
    use crate::test_harness;

    /// Build a minimal game state with a small hex map for fog tests.
    fn make_game() -> GameState {
        let cfg = mvp_preset();
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
        assert_eq!(sight_of(UnitKind::Scout), SIGHT_SCOUT);
        assert_eq!(sight_of(UnitKind::Scout), 3);
        assert_eq!(sight_of(UnitKind::CaravanGuard), SIGHT_GUARD);
        assert_eq!(sight_of(UnitKind::CaravanGuard), 1);
        assert_eq!(sight_of(UnitKind::Raider), SIGHT_RAIDER);
        assert_eq!(sight_of(UnitKind::Raider), 2);
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
        let newly = reveal(&mut s, player, origin_tile, 1);
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
        let newly1 = reveal(&mut s, player, origin_tile, 1);
        let newly2 = reveal(&mut s, player, origin_tile, 1);
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
        assert!(!is_tile_visible(&s, player, origin_tile));
        reveal(&mut s, player, origin_tile, 1);
        assert!(is_tile_visible(&s, player, origin_tile));
    }

    #[test]
    fn is_tile_visible_false_for_other_player() {
        let mut s = make_game();
        let origin_tile = s.tile_index[&crate::hex::ORIGIN];
        reveal(&mut s, PlayerId(0), origin_tile, 1);
        assert!(
            !is_tile_visible(&s, PlayerId(1), origin_tile),
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
        reveal(&mut s, PlayerId(0), origin_tile, 0);
        assert!(is_unit_visible(&s, PlayerId(0), scout));
        // Player 1 hasn't revealed that tile.
        assert!(!is_unit_visible(&s, PlayerId(1), scout));
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
        assert_eq!(city_sight(&s, city_id), SIGHT_CITY_BASE);
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
        assert_eq!(
            city_sight(&s, city_id),
            SIGHT_CITY_BASE + SIGHT_SCHOLAR_BONUS
        );
    }

    #[test]
    fn reveal_from_unit_uses_unit_sight() {
        let mut s = make_game();
        let scout = UnitId(0);
        let before = s.players[0].discovered.len();
        reveal_from_unit(&mut s, scout);
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
        assert!(!is_route_visible(&s, PlayerId(1), route_id));
        // Reveal the origin tile for player 0 → route becomes visible.
        reveal(&mut s, PlayerId(0), origin_tile, 0);
        assert!(is_route_visible(&s, PlayerId(0), route_id));
    }
}
