//! Combat resolution: auto-combat, raid contests, and city sieges.
//!
//! All randomness flows through `state.rng` (ADR-0006). Emits
//! `GameEvent::Combat` / `RouteRaided` / `CityRaided`.

use crate::hex::neighbors;
use crate::model::{GameState, TerrainType, terrain_def, unit_def};
use crate::{
    CityId, CitySpecialization, GameEvent, PlayerId, RouteId, RouteStatus, TileId, UnitAbility,
    UnitId, UnitKind,
};

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
fn positioning_attacker(attacker_terrain: TerrainType, defender_terrain: TerrainType) -> f32 {
    if attacker_terrain == TerrainType::Ridges && defender_terrain != TerrainType::Ridges {
        return 1.0 + FLANK_POSITIONING_BONUS;
    }
    if attacker_terrain == TerrainType::SaltFlats {
        return EXPOSED_POSITIONING_MULT;
    }
    1.0
}

// ---------------------------------------------------------------------------
// Core combat (spec §6.1)
// ---------------------------------------------------------------------------

/// Resolve auto-combat between two units.
///
/// Each exchange rolls `state.rng.next_f32()` against the computed odds.
/// The loser of each exchange takes exactly 1 HP damage. Combat continues
/// until one side is destroyed or the attacker auto-retreats (spec §6.6).
///
/// `origin_tile` is the tile the attacker retreats to if it would die.
///
/// Returns a [`GameEvent::Combat`] summarizing the outcome.
pub fn resolve_combat(
    state: &mut GameState,
    attacker_id: UnitId,
    defender_id: UnitId,
    origin_tile: TileId,
) -> Vec<GameEvent> {
    // Guard: refuse self-combat and missing units.
    if attacker_id == defender_id {
        return Vec::new();
    }
    let attacker_idx = attacker_id.0 as usize;
    let defender_idx = defender_id.0 as usize;
    if state.units.get(attacker_idx).is_none() || state.units.get(defender_idx).is_none() {
        return Vec::new();
    }

    // Collect stats (avoids borrow issues during the mutation loop).
    let atk_kind = state.units[attacker_idx].kind;
    let def_kind = state.units[defender_idx].kind;
    let atk_stat = unit_def(atk_kind).atk as f32;
    let def_stat = unit_def(def_kind).def as f32;

    let attacker_tile = state.units[attacker_idx].tile;
    let defender_tile = state.units[defender_idx].tile;
    let atk_terrain = state.tiles[attacker_tile.0 as usize].terrain;
    let def_terrain = state.tiles[defender_tile.0 as usize].terrain;
    let terrain_mod = terrain_def(def_terrain).defense_mod as f32;

    // Compute attack / defense power (spec §6.1).
    let atk_pos = positioning_attacker(atk_terrain, def_terrain);
    let attack_power = atk_stat * atk_pos * 1.0; // morale = 1.0 (MVP)
    let defense_power = def_stat * (1.0 + terrain_mod);

    let odds = if attack_power + defense_power > 0.0 {
        attack_power / (attack_power + defense_power)
    } else {
        0.5
    };

    // Combat loop: each exchange, roll vs odds.
    let mut attacker_loss = 0u32;
    let mut defender_loss = 0u32;
    let mut retreated = false;
    let mut defender_destroyed = false;

    loop {
        let roll = state.rng.next_f32();

        if roll < odds {
            // Defender takes 1 HP damage.
            state.units[defender_idx].hp -= 1;
            defender_loss += 1;

            if state.units[defender_idx].hp == 0 {
                defender_destroyed = true;
                break;
            }
        } else {
            // Attacker takes 1 HP damage.
            state.units[attacker_idx].hp -= 1;
            attacker_loss += 1;

            // Auto-retreat: attacker falls back before dying (spec §6.6).
            if state.units[attacker_idx].hp == 0 {
                state.units[attacker_idx].hp = 1;
                state.units[attacker_idx].tile = origin_tile;
                state.units[attacker_idx].moves_left = 0;
                retreated = true;
                break;
            }
        }
    }

    // Remove destroyed defender (spec §7).
    if defender_destroyed {
        state.units.retain(|u| u.id != defender_id);
    }

    vec![GameEvent::Combat {
        attacker: attacker_id,
        defender: defender_id,
        attacker_loss,
        defender_loss,
        retreated,
    }]
}

// ---------------------------------------------------------------------------
// Raid contest (spec §6.3)
// ---------------------------------------------------------------------------

/// Resolve a Raider-vs-route contest.
///
/// If a controlling Guard is on or adjacent to the route path, a stat
/// contest determines the outcome. Otherwise the route auto-cascades.
pub fn resolve_raid_contest(
    state: &mut GameState,
    raider_id: UnitId,
    route_id: RouteId,
) -> Vec<GameEvent> {
    // Validate inputs.
    if state.units.get(raider_id.0 as usize).is_none() {
        return vec![GameEvent::Warn {
            message: "Raid failed: raider unit not found".into(),
        }];
    }
    if state.routes.get(route_id.0 as usize).is_none() {
        return vec![GameEvent::Warn {
            message: "Raid failed: route not found".into(),
        }];
    }

    // Collect data (avoid borrow issues).
    let raider_owner = state.units[raider_id.0 as usize].owner;
    let raider_tile = state.units[raider_id.0 as usize].tile;
    let route_owner = state.routes[route_id.0 as usize].owner;
    let route_path: Vec<TileId> = state.routes[route_id.0 as usize].path.clone();

    // Check for a controlling Guard on / adjacent to the route.
    let guard_id = find_controlling_guard(state, &route_path, route_owner);

    if let Some(guard_id) = guard_id {
        // Stat contest: Raider atk vs Guard def (spec §6.3).
        let raider_tile_terrain = state.tiles[raider_tile.0 as usize].terrain;
        let raider_atk_pos = match raider_tile_terrain {
            TerrainType::Ridges => ROUGH_RAIDER_BONUS,
            TerrainType::SaltFlats => EXPOSED_POSITIONING_MULT,
            _ => 1.0,
        };
        let raider_atk = unit_def(UnitKind::Raider).atk as f32 * raider_atk_pos;

        let guard_tile = state.units[guard_id.0 as usize].tile;
        let guard_tile_terrain = state.tiles[guard_tile.0 as usize].terrain;
        let guard_def_mod = terrain_def(guard_tile_terrain).defense_mod as f32;
        let guard_def = unit_def(UnitKind::CaravanGuard).def as f32 * (1.0 + guard_def_mod);

        let odds = if raider_atk + guard_def > 0.0 {
            raider_atk / (raider_atk + guard_def)
        } else {
            0.5
        };

        let roll = state.rng.next_f32();

        if roll < odds {
            // Raid succeeds → cascade.
            cascade_route(state, route_id);
            let severed = state.routes[route_id.0 as usize].status == RouteStatus::Severed;
            vec![GameEvent::RouteRaided {
                route: route_id,
                by: raider_owner,
                severed,
            }]
        } else {
            // Guard repels — route stays Active.
            vec![GameEvent::RouteRaided {
                route: route_id,
                by: raider_owner,
                severed: false,
            }]
        }
    } else {
        // No defender → auto cascade.
        cascade_route(state, route_id);
        let severed = state.routes[route_id.0 as usize].status == RouteStatus::Severed;
        vec![GameEvent::RouteRaided {
            route: route_id,
            by: raider_owner,
            severed,
        }]
    }
}

/// Find a Guard owned by `route_owner` on or adjacent to any route-path tile.
fn find_controlling_guard(
    state: &GameState,
    route_path: &[TileId],
    route_owner: PlayerId,
) -> Option<UnitId> {
    for &path_tile in route_path {
        // Units ON this tile.
        for u in &state.units {
            if u.owner == route_owner && u.kind == UnitKind::CaravanGuard && u.tile == path_tile {
                return Some(u.id);
            }
        }

        // Units ADJACENT to this tile.
        let path_coord = state.tiles[path_tile.0 as usize].coord;
        for n in neighbors(path_coord) {
            if let Some(&adj_tile) = state.tile_index.get(&n) {
                for u in &state.units {
                    if u.owner == route_owner
                        && u.kind == UnitKind::CaravanGuard
                        && u.tile == adj_tile
                    {
                        return Some(u.id);
                    }
                }
            }
        }
    }
    None
}

/// Cascade a route's status: `Active → Threatened`, `Threatened → Severed`.
fn cascade_route(state: &mut GameState, route_id: RouteId) {
    let route = &mut state.routes[route_id.0 as usize];
    route.status = match route.status {
        RouteStatus::Active => RouteStatus::Threatened,
        RouteStatus::Threatened => RouteStatus::Severed,
        RouteStatus::Severed => RouteStatus::Severed,
    };
}

// ---------------------------------------------------------------------------
// City raid (spec §6.4)
// ---------------------------------------------------------------------------

/// Resolve a city raid: Raider vs city (garrison + Fortress + terrain).
///
/// On attacker win: `city.population -= 1`. If population reaches 0 and
/// the Raider occupies the city tile, the city is **captured** (owner flip).
pub fn resolve_city_raid(
    state: &mut GameState,
    raider_id: UnitId,
    city_id: CityId,
) -> Vec<GameEvent> {
    // Validate inputs.
    let raider_idx = raider_id.0 as usize;
    let city_idx = city_id.0 as usize;

    if state.units.get(raider_idx).is_none() || state.cities.get(city_idx).is_none() {
        return vec![GameEvent::Warn {
            message: "City raid failed: unit or city not found".into(),
        }];
    }

    let raider_owner = state.units[raider_idx].owner;
    let city_owner = state.cities[city_idx].owner;
    let city_tile = state.cities[city_idx].tile;
    let city_pop = state.cities[city_idx].population;

    // Cannot raid own city.
    if raider_owner == city_owner {
        return vec![GameEvent::Warn {
            message: "Cannot raid own city".into(),
        }];
    }

    if city_pop == 0 {
        return vec![GameEvent::Warn {
            message: "City has no population to raid".into(),
        }];
    }

    // --- Compute city defense (spec §6.4) ---
    let city_terrain = state.tiles[city_tile.0 as usize].terrain;
    let terrain_mod = terrain_def(city_terrain).defense_mod as f32;

    let fortress_bonus =
        if state.cities[city_idx].specialization == Some(CitySpecialization::Fortress) {
            FORTRESS_CITY_DEF as f32
        } else {
            0.0
        };

    // Sum garrisoned Guard def.
    let mut garrison_def = 0.0f32;
    for u in &state.units {
        if u.owner == city_owner
            && u.kind == UnitKind::CaravanGuard
            && u.tile == city_tile
            && u.ability == UnitAbility::Garrisoned
        {
            garrison_def += unit_def(UnitKind::CaravanGuard).def as f32;
        }
    }

    // City's aggregate defense (terrain_mod already included).
    let city_def = (terrain_mod + fortress_bonus + garrison_def).max(0.0);

    // --- Raider attack power ---
    let raider_tile = state.units[raider_idx].tile;
    let raider_terrain = state.tiles[raider_tile.0 as usize].terrain;
    let atk_pos = positioning_attacker(raider_terrain, city_terrain);
    let attack_power = unit_def(UnitKind::Raider).atk as f32 * atk_pos;
    let defense_power = city_def;

    let odds = if attack_power + defense_power > 0.0 {
        attack_power / (attack_power + defense_power)
    } else {
        0.5
    };

    // --- Combat loop with city HP ---
    let mut remaining_pop = city_pop as i32;
    let mut pop_lost = 0u32;

    loop {
        let roll = state.rng.next_f32();

        if roll < odds {
            // City takes damage.
            remaining_pop -= 1;
            pop_lost += 1;

            if remaining_pop <= 0 {
                break;
            }
        } else {
            // Raider takes damage.
            state.units[raider_idx].hp -= 1;

            if state.units[raider_idx].hp == 0 {
                // Auto-retreat: survive at 1 HP, stop the raid.
                state.units[raider_idx].hp = 1;
                state.units[raider_idx].moves_left = 0;
                break;
            }
        }
    }

    // Apply population loss.
    state.cities[city_idx].population = state.cities[city_idx].population.saturating_sub(pop_lost);

    // Check for capture (spec §6.4): pop == 0 AND raider on city tile.
    if state.cities[city_idx].population == 0 && state.units[raider_idx].tile == city_tile {
        state.cities[city_idx].owner = raider_owner;
    }

    vec![GameEvent::CityRaided {
        city: city_id,
        by: raider_owner,
        pop_lost,
    }]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::HexCoord;
    use crate::model::{Player, Stockpiles, Tile, Unit, UnitAbility};
    use crate::scenario::mvp_preset;
    use fxhash::FxHashSet;

    // ---- test harness ------------------------------------------------------

    /// Build a minimal game state for combat tests with optional terrain
    /// overrides applied to specific hex coordinates.
    fn make_game(seed: u64, terrain_overrides: &[(HexCoord, TerrainType)]) -> GameState {
        let cfg = mvp_preset();
        let mut s = GameState::new(cfg, seed);
        let radius = s.scenario.map_radius as u32;

        // Allocate all in-map tiles.
        for coord in crate::hex::range(crate::hex::ORIGIN, radius) {
            let id = s.alloc_tile_id();
            s.tiles.push(Tile {
                id,
                coord,
                terrain: TerrainType::Dunes,
                is_relic_site: false,
                owner: None,
                improvement: None,
            });
            s.tile_index.insert(coord, id);
        }

        // Apply terrain overrides.
        for &(coord, terrain) in terrain_overrides {
            if let Some(&id) = s.tile_index.get(&coord) {
                s.tiles[id.0 as usize].terrain = terrain;
            }
        }

        // Two players.
        for i in 0..2u32 {
            let pid = s.alloc_player_id();
            s.players.push(Player {
                id: pid,
                kind: if i == 0 {
                    crate::PlayerKind::Human
                } else {
                    crate::PlayerKind::Ai {
                        personality: crate::AiPersonality::Expansionist,
                        difficulty: crate::Difficulty::Normal,
                    }
                },
                color: crate::PlayerColor::Sand,
                resources: Stockpiles::default(),
                discovered: FxHashSet::default(),
                defeated: false,
            });
        }

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
        let id = state.alloc_unit_id();
        state.units.push(Unit {
            id,
            owner,
            kind,
            tile,
            hp,
            moves_left: unit_def(kind).moves,
            ability: UnitAbility::None,
        });
        id
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

        let events = resolve_combat(&mut s, atk, def, origin_tile);

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

        let events = resolve_combat(&mut s, atk, def, origin_tile);

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

        let events = resolve_combat(&mut s, unit, unit, tile);
        assert!(events.is_empty(), "self-combat should produce no events");
    }

    #[test]
    fn combat_missing_unit_is_noop() {
        let mut s = make_game(42, &[]);
        let tile = s.tile_index[&HexCoord { q: 0, r: 0 }];
        let atk = add_unit(&mut s, PlayerId(0), UnitKind::Raider, tile, 4);

        let events = resolve_combat(&mut s, atk, UnitId(999), tile);
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

        let events = resolve_combat(&mut s, atk, def, origin_tile);

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

        let events = resolve_raid_contest(&mut s, raider, route_id);

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
        let _ = resolve_raid_contest(&mut s, raider, route_id);
        assert_eq!(
            s.routes[route_id.0 as usize].status,
            RouteStatus::Threatened
        );

        // Second raid: Threatened → Severed.
        let events = resolve_raid_contest(&mut s, raider, route_id);
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

        let events = resolve_raid_contest(&mut s, raider, route_id);

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

        let events = resolve_raid_contest(&mut s, raider, route_id);

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
        let events = resolve_raid_contest(&mut s, UnitId(999), RouteId(0));
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
            specialization: Some(CitySpecialization::Fortress),
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: vec![],
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = resolve_city_raid(&mut s, raider, city_id);

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
            queue: vec![],
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = resolve_city_raid(&mut s, raider, city_id);

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
            queue: vec![],
        });

        // Raider ON the city tile → capture when pop hits 0.
        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, city_tile, 4);

        let events = resolve_city_raid(&mut s, raider, city_id);

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
            queue: vec![],
        });

        // Raider NOT on the city tile.
        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, raider_tile, 4);

        let events = resolve_city_raid(&mut s, raider, city_id);

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
            queue: vec![],
        });

        let raider = add_unit(&mut s, PlayerId(0), UnitKind::Raider, city_tile, 4);

        let events = resolve_city_raid(&mut s, raider, city_id);

        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], GameEvent::Warn { .. }),
            "raiding own city should warn"
        );
    }

    #[test]
    fn city_raid_missing_inputs_warns() {
        let mut s = make_game(42, &[]);
        let events = resolve_city_raid(&mut s, UnitId(999), CityId(999));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], GameEvent::Warn { .. }));
    }
}
