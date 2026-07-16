//! World generation for Desert-City-States (Phase 1: Foundation).
//!
//! [`new_game`] builds a fully-populated, deterministic [`GameState`] from a
//! [`ScenarioConfig`] and a `u64` seed. It runs an eight-step pipeline:
//!
//! 1. `generate_tiles`      – lay out every in-map hex as a Dunes tile.
//! 2. `place_oases`        – scatter oases, spaced ≥ 2 apart.
//! 3. `place_ridges`       – carve 1–2 linear ridge chains.
//! 4. `carve_salt_flats`   – carve 1–2 salt-flat corridors.
//! 5. `place_ruins_and_relics` – mark relic sites on distinct Ruins tiles.
//! 6. `place_players`      – choose pairwise-spaced start oases.
//! 7. `init_players_and_starting_units` – create players + 2 units each.
//! 8. `reveal_start_fog` + `finalize` – lift fog around each start, set turn.
//!
//! # Determinism contract
//!
//! *Every* procedural draw flows through `state.rng` ([`crate::model::SeededRng`]).
//! No `thread_rng`, no wall-clock, no `HashMap` iteration drives any generation
//! decision — only deterministic walks over `state.tiles` / collected `Vec`s.
//! Identical `(scenario, seed)` therefore always yields a byte-identical state.

use fxhash::FxHashSet;

use crate::hex::{AXIAL_DIRS, HexCoord, ORIGIN, distance, in_map, neighbors, range};
use crate::model::{
    FOUND_CITY_INFLUENCE, GameState, Player, PlayerColor, PlayerKind, Relic, Stockpiles,
    TerrainType, Tile, TurnPhase, Unit, UnitAbility, starting_wealth, unit_def,
};
use crate::scenario::{AiPersonality, Difficulty, ScenarioConfig};
use crate::{GameEvent, PlayerId, RelicId, TileId, UnitId, UnitKind};

/// Build a fresh, fully-generated game state from a scenario and seed.
///
/// The pipeline is deterministic: the same `(scenario, seed)` always produces a
/// byte-identical [`GameState`].
pub fn new_game(scenario: &ScenarioConfig, seed: u64) -> GameState {
    let mut state = GameState::new(scenario.clone(), seed);

    generate_tiles(&mut state);
    place_oases(&mut state);
    place_ridges(&mut state);
    carve_salt_flats(&mut state);
    place_ruins_and_relics(&mut state);

    let start_oases = place_players(&mut state);
    init_players_and_starting_units(&mut state, &start_oases);
    reveal_start_fog(&mut state, &start_oases);

    finalize(&mut state);
    state
}

// ---------------------------------------------------------------------------
// Step 1: tile grid
// ---------------------------------------------------------------------------

/// Create one [`Tile`] (default Dunes) for every hex inside the map radius.
fn generate_tiles(state: &mut GameState) {
    let radius = state.scenario.map_radius as u32;
    // Iterate deterministically: every in-map hex in a fixed scan order.
    for coord in range(ORIGIN, radius) {
        if !in_map(coord, radius) {
            continue;
        }
        let id = TileId(state.tiles.len() as u32);
        state.tiles.push(Tile {
            id,
            coord,
            terrain: TerrainType::Dunes,
            is_relic_site: false,
            owner: None,
            improvement: None,
        });
        state.tile_index.insert(coord, id);
    }
}

// ---------------------------------------------------------------------------
// Step 2: oases
// ---------------------------------------------------------------------------

/// Scatter oases. Target count is `max(player_count, total_tiles / 15)`. Oases
/// are kept ≥ 2 apart; if candidates run out we relax to ≥ 1 spacing.
fn place_oases(state: &mut GameState) {
    let radius = state.scenario.map_radius as u32;
    let player_count = state.scenario.player_count as usize;
    let total_tiles = state.tiles.len();
    let oasis_target = player_count.max(total_tiles / 15);

    // Build a deterministic candidate list: Dunes tiles within (radius - 1) of
    // the origin that currently have no oasis within distance < 2.
    let mut oases: Vec<TileId> = Vec::new();

    let mut candidates: Vec<TileId> = state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Dunes && distance(t.coord, ORIGIN) < radius)
        .map(|t| t.id)
        .collect();

    // Fisher-Yates shuffle for unbiased deterministic selection.
    shuffle_ids(state, &mut candidates);

    let mut i = 0;
    while oases.len() < oasis_target && i < candidates.len() {
        let cand = candidates[i];
        i += 1;
        let coord = tile_coord(state, cand);
        if oases
            .iter()
            .any(|&o| distance(coord, tile_coord(state, o)) < 2)
        {
            continue;
        }
        set_terrain(state, cand, TerrainType::Oasis);
        oases.push(cand);
    }

    // Relax the spacing constraint if we still need more oases.
    while oases.len() < oasis_target && i < candidates.len() {
        let cand = candidates[i];
        i += 1;
        let coord = tile_coord(state, cand);
        if oases
            .iter()
            .any(|&o| distance(coord, tile_coord(state, o)) < 1)
        {
            continue;
        }
        set_terrain(state, cand, TerrainType::Oasis);
        oases.push(cand);
    }
}

// ---------------------------------------------------------------------------
// Steps 3 & 4: ridge chains and salt-flat corridors
// ---------------------------------------------------------------------------

/// Carve 1–2 linear chains of [`TerrainType::Ridges`] over Dunes only.
fn place_ridges(state: &mut GameState) {
    let radius = state.scenario.map_radius as u32;
    let chains = 1 + (rng_range(state, 2) as usize); // 1 or 2
    for _ in 0..chains {
        carve_line(state, radius, TerrainType::Ridges);
    }
}

/// Carve 1–2 linear corridors of [`TerrainType::SaltFlats`] over Dunes only.
fn carve_salt_flats(state: &mut GameState) {
    let radius = state.scenario.map_radius as u32;
    let corridors = 1 + (rng_range(state, 2) as usize); // 1 or 2
    for _ in 0..corridors {
        carve_line(state, radius, TerrainType::SaltFlats);
    }
}

/// Walk a linear chain of `length` steps from a random Dunes start, overwriting
/// only Dunes tiles with `terrain`. Used by [`place_ridges`] / [`carve_salt_flats`].
fn carve_line(state: &mut GameState, radius: u32, terrain: TerrainType) {
    let Some(start_tile) = random_dunes_tile(state) else {
        return;
    };
    let start_coord = tile_coord(state, start_tile);

    // Pick a random direction index 0..6.
    let dir = AXIAL_DIRS[rng_range(state, 6) as usize];
    // Length ~ radius, with ±2 variation but at least 2.
    let variation = rng_range(state, 5) as i32; // 0..5
    let length = (radius as i32 + variation - 2).max(2) as u32;

    for step in 1..=length {
        let next = HexCoord {
            q: start_coord.q + dir.0 * step as i32,
            r: start_coord.r + dir.1 * step as i32,
        };
        if !in_map(next, radius) {
            break;
        }
        if let Some(&tile) = state.tile_index.get(&next) {
            if state.tiles[tile.0 as usize].terrain == TerrainType::Dunes {
                set_terrain(state, tile, terrain);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 5: ruins + relic sites
// ---------------------------------------------------------------------------

/// Choose `scenario.relic_count` distinct Dunes tiles, mark them Ruins with
/// `is_relic_site = true`, and create a [`Relic`] per site.
///
/// If not enough distinct Dunes remain, the count is capped at what's available
/// (acceptable for the MVP). The number of relic *sites* equals the number of
/// relics created.
fn place_ruins_and_relics(state: &mut GameState) {
    let relic_count = state.scenario.relic_count as usize;

    let mut candidates: Vec<TileId> = state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Dunes)
        .map(|t| t.id)
        .collect();

    shuffle_ids(state, &mut candidates);

    for &tile in candidates.iter().take(relic_count) {
        // Convert to Ruins and flag as a relic site.
        set_terrain(state, tile, TerrainType::Ruins);
        state.tiles[tile.0 as usize].is_relic_site = true;
        let relic_id = RelicId(state.relics.len() as u32);
        state.relics.push(Relic {
            id: relic_id,
            tile,
            holder: None,
            consecutive_turns_held: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Step 6: player start oases
// ---------------------------------------------------------------------------

/// Pick `player_count` distinct oasis tiles with pairwise distance ≥ 4.
///
/// Returns the chosen start [`TileId`]s. Emits a non-fatal [`GameEvent::Warn`]
/// if the spacing constraint could not be fully satisfied.
fn place_players(state: &mut GameState) -> Vec<TileId> {
    let player_count = state.scenario.player_count as usize;
    let oasis_tiles = oasis_tiles(state);

    // Shuffle the candidates deterministically.
    let mut candidates: Vec<TileId> = oasis_tiles.clone();
    shuffle_ids(state, &mut candidates);

    let mut chosen: Vec<TileId> = Vec::new();
    for &cand in candidates.iter() {
        if chosen.len() >= player_count {
            break;
        }
        let coord = tile_coord(state, cand);
        if chosen
            .iter()
            .any(|&c| distance(coord, tile_coord(state, c)) < 4)
        {
            continue;
        }
        chosen.push(cand);
    }

    // If we couldn't satisfy spacing (or didn't have enough oases), greedily
    // fill remaining slots from any unused oasis to guarantee `player_count`.
    if chosen.len() < player_count {
        for &cand in candidates.iter() {
            if chosen.len() >= player_count {
                break;
            }
            if !chosen.contains(&cand) {
                chosen.push(cand);
            }
        }
    }

    if chosen.len() < player_count {
        // Still short (not enough oases at all) — log a non-fatal warning and
        // proceed with whatever we have.
        state.log.push(GameEvent::Warn {
            message: format!(
                "Could only place {} of {} player starts (insufficient oases).",
                chosen.len(),
                player_count
            ),
        });
    } else if chosen.len() == player_count {
        // Spacing may not have been fully satisfied; the fill loop above
        // completed the count but possibly with closer-than-4 spacing.
        let spacing_ok = chosen.iter().all(|&a| {
            chosen
                .iter()
                .all(|&b| a == b || distance(tile_coord(state, a), tile_coord(state, b)) >= 4)
        });
        if !spacing_ok {
            state.log.push(GameEvent::Warn {
                message: "Player start spacing (>=4) could not be fully satisfied.".into(),
            });
        }
    }

    chosen
}

// ---------------------------------------------------------------------------
// Step 7: players + starting units
// ---------------------------------------------------------------------------

/// Create one [`Player`] per start oasis (player 0 human, the rest AI) with
/// starting resources, then spawn a Scout and a CaravanGuard on/near the start.
fn init_players_and_starting_units(state: &mut GameState, start_oases: &[TileId]) {
    let colors = [
        PlayerColor::Sand,
        PlayerColor::Crimson,
        PlayerColor::Teal,
        PlayerColor::Violet,
    ];

    for (i, &start_tile) in start_oases.iter().enumerate() {
        let player_id = PlayerId(state.players.len() as u32);

        let kind = if i == 0 {
            PlayerKind::Human
        } else {
            let personality = state
                .scenario
                .ai_personalities
                .get(i - 1)
                .copied()
                .unwrap_or(AiPersonality::Expansionist);
            PlayerKind::Ai {
                personality,
                difficulty: Difficulty::Normal,
            }
        };

        let color = colors[i % colors.len()];

        state.players.push(Player {
            id: player_id,
            kind,
            color,
            resources: Stockpiles {
                water: 0,
                wealth: starting_wealth(),
                influence: FOUND_CITY_INFLUENCE,
            },
            discovered: FxHashSet::default(),
            defeated: false,
        });

        // Spawn a Scout and a CaravanGuard near the start oasis.
        spawn_unit(state, player_id, UnitKind::Scout, start_tile);
        spawn_unit(state, player_id, UnitKind::CaravanGuard, start_tile);
    }
}

/// Place a unit of `kind` owned by `player` on `preferred`; if occupied, pick a
/// random adjacent Dunes tile, else fall back to `preferred`.
fn spawn_unit(state: &mut GameState, player: PlayerId, kind: UnitKind, preferred: TileId) {
    let tile = choose_unit_tile(state, preferred);
    let def = unit_def(kind);
    let id = UnitId(state.units.len() as u32);
    state.units.push(Unit {
        id,
        owner: player,
        kind,
        tile,
        hp: def.hp as u32,
        moves_left: def.moves,
        ability: UnitAbility::None,
    });
}

/// Choose the tile to place a starting unit on: prefer `preferred` if free,
/// otherwise a random adjacent Dunes/neutral tile; otherwise `preferred`.
fn choose_unit_tile(state: &mut GameState, preferred: TileId) -> TileId {
    let occupied = state.units.iter().any(|u| u.tile == preferred);
    if !occupied {
        return preferred;
    }
    let coord = tile_coord(state, preferred);
    // Gather adjacent in-map tiles that are Dunes (neutral) and free.
    let options: Vec<TileId> = neighbors(coord)
        .iter()
        .filter(|&&n| in_map(n, state.scenario.map_radius as u32))
        .filter_map(|&n| state.tile_index.get(&n).copied())
        .filter(|&t| state.tiles[t.0 as usize].terrain == TerrainType::Dunes)
        .filter(|&t| !state.units.iter().any(|u| u.tile == t))
        .collect();
    if options.is_empty() {
        return preferred;
    }
    let idx = rng_range(state, options.len() as u32) as usize;
    options[idx]
}

// ---------------------------------------------------------------------------
// Step 8: fog reveal + finalize
// ---------------------------------------------------------------------------

/// Reveal the start oasis plus `Scout.sight` radius to each player.
fn reveal_start_fog(state: &mut GameState, start_oases: &[TileId]) {
    let sight = unit_def(UnitKind::Scout).sight as u32;
    for (i, &start_tile) in start_oases.iter().enumerate() {
        if i >= state.players.len() {
            break;
        }
        let coord = tile_coord(state, start_tile);
        let mut revealed = FxHashSet::default();
        for c in range(coord, sight) {
            if let Some(&tid) = state.tile_index.get(&c) {
                revealed.insert(tid);
            }
        }
        // Union into the player's discovered set.
        for tid in revealed {
            state.players[i].discovered.insert(tid);
        }
    }
}

/// Set the turn bookkeeping to the opening state. No city is created here.
fn finalize(state: &mut GameState) {
    state.turn = 1;
    state.current_actor = PlayerId(0);
    state.phase = TurnPhase::Order;
    // `victory` is already the default; ensure it's zeroed for cleanliness.
    state.victory = Default::default();
    // Any non-fatal warnings emitted earlier (e.g. spacing) are preserved.
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// All tiles whose terrain is [`TerrainType::Oasis`].
fn oasis_tiles(state: &GameState) -> Vec<TileId> {
    state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Oasis)
        .map(|t| t.id)
        .collect()
}

/// All tiles whose terrain is [`TerrainType::Dunes`].
fn dunes_tiles(state: &GameState) -> Vec<TileId> {
    state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Dunes)
        .map(|t| t.id)
        .collect()
}

/// Lookup a tile's coordinate by id (invariant: must exist).
fn tile_coord(state: &GameState, id: TileId) -> HexCoord {
    state
        .tiles
        .get(id.0 as usize)
        .expect("invariant: tile id references a live tile")
        .coord
}

/// Draw `next_range(max)` from the deterministic `state.rng`.
fn rng_range(state: &mut GameState, max: u32) -> u32 {
    state.rng.next_range(max)
}

/// Fisher-Yates shuffle of a `TileId` slice using `state.rng` (deterministic).
fn shuffle_ids(state: &mut GameState, v: &mut [TileId]) {
    for i in (1..v.len()).rev() {
        let j = rng_range(state, (i + 1) as u32) as usize;
        v.swap(i, j);
    }
}

/// Set a tile's terrain in place (indexed lookup).
fn set_terrain(state: &mut GameState, tile: TileId, terrain: TerrainType) {
    state.tiles[tile.0 as usize].terrain = terrain;
}

/// Pick a random Dunes tile id, or `None` if there are no Dunes tiles.
fn random_dunes_tile(state: &mut GameState) -> Option<TileId> {
    let dunes = dunes_tiles(state);
    if dunes.is_empty() {
        None
    } else {
        let idx = rng_range(state, dunes.len() as u32) as usize;
        Some(dunes[idx])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::mvp_preset;

    fn relic_site_count(state: &GameState) -> usize {
        state.tiles.iter().filter(|t| t.is_relic_site).count()
    }

    #[test]
    fn new_game_deterministic() {
        let a = new_game(&mvp_preset(), 42);
        let b = new_game(&mvp_preset(), 42);
        // Field-by-field comparison (GameState lacks PartialEq).
        assert_eq!(a.tiles.len(), b.tiles.len(), "tiles len differs");
        assert_eq!(a.players.len(), b.players.len(), "players len differs");
        assert_eq!(a.units.len(), b.units.len(), "units len differs");
        assert_eq!(a.relics.len(), b.relics.len(), "relics len differs");
        // Terrain distribution must match tile-for-tile.
        for (ta, tb) in a.tiles.iter().zip(b.tiles.iter()) {
            assert_eq!(ta.coord, tb.coord);
            assert_eq!(ta.terrain, tb.terrain);
            assert_eq!(ta.is_relic_site, tb.is_relic_site);
        }
        // Unit placement must match.
        for (ua, ub) in a.units.iter().zip(b.units.iter()) {
            assert_eq!(ua.owner, ub.owner);
            assert_eq!(ua.kind, ub.kind);
            assert_eq!(ua.tile, ub.tile);
        }
        // Relic tiles match.
        for (ra, rb) in a.relics.iter().zip(b.relics.iter()) {
            assert_eq!(ra.tile, rb.tile);
        }
    }

    #[test]
    fn new_game_different_seed_differs() {
        let a = new_game(&mvp_preset(), 1);
        let b = new_game(&mvp_preset(), 2);
        // At least one tile's terrain should differ (extremely likely).
        let differs = a
            .tiles
            .iter()
            .zip(b.tiles.iter())
            .any(|(ta, tb)| ta.terrain != tb.terrain);
        assert!(differs, "different seeds produced identical maps");
    }

    #[test]
    fn no_city_at_gen() {
        let state = new_game(&mvp_preset(), 7);
        assert!(state.cities.is_empty(), "world-gen must not create cities");
    }

    #[test]
    fn player_count_starts() {
        let state = new_game(&mvp_preset(), 11);
        assert_eq!(state.players.len(), 3, "mvp_preset has 3 players");
        assert_eq!(state.units.len(), 6, "3 players * 2 units = 6");
        assert_eq!(state.units.len(), state.players.len() * 2);
    }

    #[test]
    fn relic_sites_present() {
        let state = new_game(&mvp_preset(), 13);
        let expected = state.scenario.relic_count as usize;
        assert_eq!(
            relic_site_count(&state),
            expected,
            "relic site count must equal scenario.relic_count"
        );
        assert_eq!(
            state.relics.len(),
            expected,
            "relic count must equal scenario.relic_count"
        );
    }

    #[test]
    fn all_tiles_in_map() {
        let state = new_game(&mvp_preset(), 17);
        let radius = state.scenario.map_radius as u32;
        for t in &state.tiles {
            assert!(
                in_map(t.coord, radius),
                "tile {:?} outside map radius {}",
                t.coord,
                radius
            );
        }
    }

    #[test]
    fn finalize_state() {
        let state = new_game(&mvp_preset(), 19);
        assert_eq!(state.turn, 1);
        assert_eq!(state.current_actor, PlayerId(0));
        assert_eq!(state.phase, TurnPhase::Order);
    }

    #[test]
    fn oases_no_adjacent() {
        let state = new_game(&mvp_preset(), 23);
        let oases = oasis_tiles(&state);
        for i in 0..oases.len() {
            for j in (i + 1)..oases.len() {
                let d = distance(tile_coord(&state, oases[i]), tile_coord(&state, oases[j]));
                assert!(d >= 2, "two oases are closer than 2 apart (distance {d})");
            }
        }
    }

    #[test]
    fn player_starts_have_fog() {
        // Player 0 must have discovered at least its start oasis tile.
        let state = new_game(&mvp_preset(), 29);
        // Re-derive start oases via oasis set; player 0 discovered should be
        // non-empty and include its scout's sight radius.
        assert!(!state.players[0].discovered.is_empty());
        // Scout sight radius is 2, so at least the start + neighbors revealed.
        assert!(state.players[0].discovered.len() >= 1);
    }

    #[test]
    fn starting_resources_set() {
        let state = new_game(&mvp_preset(), 31);
        for p in &state.players {
            assert_eq!(p.resources.wealth, starting_wealth());
            assert_eq!(p.resources.influence, FOUND_CITY_INFLUENCE);
            assert!(!p.defeated);
        }
    }
}
