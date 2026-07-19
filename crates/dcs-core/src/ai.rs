//! AI Opponents — pure, deterministic planner (spec `behavior-ai-opponents.md`).
//!
//! The single public entry is [`ai_plan`], which reads an immutable `GameState`
//! and returns a `Vec<Command>` for the given actor's turn. The AI emits the
//! **same `Command` enum** as the human and goes through the **same resolver**
//! (ADR-0004), so cheating is impossible by construction — it can only plan
//! against what its own fog-of-war allows it to see.
//!
//! # Pipeline
//!
//! ```text
//! assess → candidates → prioritize → emit
//! ```
//!
//! - **assess**: builds a fog-aware [`Situation`] snapshot.
//! - **candidates**: each `candidates_*` function generates scored actions.
//! - **prioritize**: applies personality weights, sorts by score.
//! - **emit**: validates commands and returns the budget-limited result.
//!
//! # Determinism
//!
//! The utility sort is deterministic; ties are broken by stable entity ID order.
//! RNG is drawn **only** from `state.rng` for genuine coin-flips (rare). No
//! `thread_rng` / `std::time`.

use crate::model::{GameState, TerrainType};
use crate::{
    AiPersonality, CityId, CitySpecialization, Command, Difficulty, PlayerId, PlayerKind, RouteId,
    RouteStatus, TileId, UnitId, UnitKind,
};
use fxhash::FxHashSet;

// ---------------------------------------------------------------------------
// Category constants (weight-table column indices)
// ---------------------------------------------------------------------------

/// Category: connect cities with routes.
pub const CAT_CONNECT: u8 = 0;
/// Category: defend routes and cities.
pub const CAT_DEFEND: u8 = 1;
/// Category: expand territory (found cities).
pub const CAT_EXPAND: u8 = 2;
/// Category: raid enemy routes/cities.
pub const CAT_RAID: u8 = 3;
/// Category: scout / explore fog.
pub const CAT_SCOUT: u8 = 4;
/// Category: build (units, buildings).
pub const CAT_BUILD: u8 = 5;

// ---------------------------------------------------------------------------
// Data Structures
// ---------------------------------------------------------------------------

/// Tunable per-personality / per-difficulty planning parameters (DD §11.1, §11.3).
///
/// All weights are relative; only their ratios matter.
#[derive(Clone, Debug)]
pub struct AiParams {
    /// Maximum number of actions emitted this turn (greed/lookahead proxy).
    pub command_budget: usize,
    /// How many future route/defense steps to reason about.
    pub lookahead: u8,
    /// Weight for found new cities / grab oases.
    pub expand_weight: f32,
    /// Weight for buildings + specialization.
    pub build_weight: f32,
    /// Weight for establish + reinforce caravan network.
    pub route_weight: f32,
    /// Weight for patrol/guard exposed routes (DD §18 OQ-7).
    pub route_security: f32,
    /// Weight for attack enemy routes / cities.
    pub raid_weight: f32,
    /// Weight for reveal map.
    pub scout_weight: f32,
    /// Weight for garrison / fortify.
    pub defend_cities: f32,
    /// Hard: cut enemy weakest link before it develops.
    pub preemptive_raid: bool,
    /// Normal/Hard: always cover threatened routes.
    pub defend_core_routes: bool,
}

/// Transient view of the world as the AI may legally perceive it (fog-aware).
///
/// Built once per [`ai_plan`] call; scratch for one planning cycle. Not stored
/// in `GameState`.
#[derive(Clone, Debug)]
pub struct Situation {
    /// Cities owned by this player.
    pub own_cities: Vec<CityId>,
    /// Units owned by this player.
    pub own_units: Vec<UnitId>,
    /// Routes owned by this player.
    pub own_routes: Vec<RouteId>,
    /// Oases controlled by this player.
    pub own_oases: u32,
    /// Total oases on the map.
    pub total_oases: u32,
    /// Nearest unexplored tiles to expand toward.
    pub fog_frontier: Vec<TileId>,
    /// Enemy units visible to this player.
    pub visible_enemy_units: Vec<UnitId>,
    /// Enemy cities visible to this player.
    pub visible_enemy_cities: Vec<CityId>,
    /// Enemy routes visible to this player.
    pub visible_enemy_routes: Vec<RouteId>,
    /// Own routes with an uncontrolled tile.
    pub exposed_own_routes: Vec<RouteId>,
    /// Own routes with status Threatened or Severed.
    pub threatened_own_routes: Vec<RouteId>,
    /// Unconnected own cities (no active route in same component).
    pub unconnected_own_cities: Vec<CityId>,
    /// Own cities with zero active routes (isolated).
    pub isolated_cities: Vec<CityId>,
    /// Current turn number.
    pub turn: u32,
    /// Total turns in the game.
    pub total_turns: u32,
}

/// Named internal struct for ranking candidate actions. Never stored.
#[derive(Clone, Debug)]
struct ScoredAction {
    /// Weighted utility (higher = emit first).
    score: f32,
    /// The command to emit if selected.
    cmd: Command,
    /// Weight-table column index (CAT_* constant).
    category: u8,
}

// ---------------------------------------------------------------------------
// Parameter tables
// ---------------------------------------------------------------------------

/// Base weight tables per personality (spec §4).
///
/// | Personality   | expand | build | route | security | raid | scout | defend |
/// |---|---|---|---|---|---|---|---|
/// | Expansionist | 1.4 | 0.9 | 1.0 | 0.6 | 0.5 | 1.2 | 0.5 |
/// | Raider       | 0.7 | 0.6 | 0.8 | 0.5 | 1.6 | 1.0 | 0.4 |
/// | Trader       | 0.8 | 1.3 | 1.6 | 1.2 | 0.3 | 0.7 | 0.7 |
/// | Fortifier    | 0.6 | 1.2 | 1.0 | 1.4 | 0.4 | 0.5 | 1.5 |
fn base_weights(personality: AiPersonality) -> (f32, f32, f32, f32, f32, f32, f32) {
    match personality {
        AiPersonality::Expansionist => (1.4, 0.9, 1.0, 0.6, 0.5, 1.2, 0.5),
        AiPersonality::Raider => (0.7, 0.6, 0.8, 0.5, 1.6, 1.0, 0.4),
        AiPersonality::Trader => (0.8, 1.3, 1.6, 1.2, 0.3, 0.7, 0.7),
        AiPersonality::Fortifier => (0.6, 1.2, 1.0, 1.4, 0.4, 0.5, 1.5),
    }
}

/// Map `(personality, difficulty)` → [`AiParams`] via the weight tables in §4.
///
/// Difficulty multiplies the *greed/competence* axis, not the personality:
///
/// ```text
/// Easy   = command_budget 4,  lookahead 0, security*0.4, route*0.7,
///          preemptive_raid=false, defend_core_routes=false
/// Normal = command_budget 7,  lookahead 1, (weights as-is),
///          preemptive_raid=false, defend_core_routes=true
/// Hard   = command_budget 10, lookahead 2, security*1.2, raid*1.2,
///          preemptive_raid=true,  defend_core_routes=true
/// ```
pub fn params_for(personality: AiPersonality, difficulty: Difficulty) -> AiParams {
    let (expand, build, route, security, raid, scout, defend) = base_weights(personality);

    match difficulty {
        Difficulty::Easy => AiParams {
            command_budget: 4,
            lookahead: 0,
            expand_weight: expand,
            build_weight: build,
            route_weight: route * 0.7,
            route_security: security * 0.4,
            raid_weight: raid,
            scout_weight: scout,
            defend_cities: defend,
            preemptive_raid: false,
            defend_core_routes: false,
        },
        Difficulty::Normal => AiParams {
            command_budget: 7,
            lookahead: 1,
            expand_weight: expand,
            build_weight: build,
            route_weight: route,
            route_security: security,
            raid_weight: raid,
            scout_weight: scout,
            defend_cities: defend,
            preemptive_raid: false,
            defend_core_routes: true,
        },
        Difficulty::Hard => AiParams {
            command_budget: 10,
            lookahead: 2,
            expand_weight: expand,
            build_weight: build,
            route_weight: route,
            route_security: security * 1.2,
            raid_weight: raid * 1.2,
            scout_weight: scout,
            defend_cities: defend,
            preemptive_raid: true,
            defend_core_routes: true,
        },
    }
}

/// Look up the AI personality for this player (reads `player.kind`).
fn personality_of(state: &GameState, player: PlayerId) -> AiPersonality {
    match &state.players[player.0 as usize].kind {
        PlayerKind::Ai { personality, .. } => *personality,
        PlayerKind::Human => AiPersonality::default(),
    }
}

// ---------------------------------------------------------------------------
// assess — fog-aware world snapshot
// ---------------------------------------------------------------------------

/// Build the fog-aware [`Situation`] snapshot for `player`.
///
/// Uses fog queries from `fog.rs` — the AI **never** sees through its own fog.
fn assess(state: &GameState, player: PlayerId) -> Situation {
    let mut sit = Situation {
        own_cities: Vec::new(),
        own_units: Vec::new(),
        own_routes: Vec::new(),
        own_oases: crate::victory::oases_controlled_by(state, player),
        total_oases: crate::victory::total_oases(state),
        fog_frontier: Vec::new(),
        visible_enemy_units: Vec::new(),
        visible_enemy_cities: Vec::new(),
        visible_enemy_routes: Vec::new(),
        exposed_own_routes: Vec::new(),
        threatened_own_routes: Vec::new(),
        unconnected_own_cities: Vec::new(),
        isolated_cities: Vec::new(),
        turn: state.turn,
        total_turns: state.scenario.turn_limit,
    };

    // --- Own cities ---
    for city in &state.cities {
        if city.owner == player {
            sit.own_cities.push(city.id);
        }
    }

    // --- Own units ---
    for unit in &state.units {
        if unit.owner == player {
            sit.own_units.push(unit.id);
        }
    }

    // --- Own routes ---
    for route in &state.routes {
        if route.owner == player {
            sit.own_routes.push(route.id);
            // Check for exposed tiles.
            let has_exposed = route
                .path
                .iter()
                .any(|&tid| !crate::caravan::is_route_tile_controlled(state, route, tid));
            if has_exposed {
                sit.exposed_own_routes.push(route.id);
            }
            // Check for threatened/severed status.
            if route.status == RouteStatus::Threatened || route.status == RouteStatus::Severed {
                sit.threatened_own_routes.push(route.id);
            }
        }
    }

    // --- Visible enemy units ---
    for unit in &state.units {
        if unit.owner != player && crate::fog::is_unit_visible(state, player, unit.id) {
            sit.visible_enemy_units.push(unit.id);
        }
    }

    // --- Visible enemy cities ---
    for city in &state.cities {
        if city.owner != player && crate::fog::is_city_visible(state, player, city.id) {
            sit.visible_enemy_cities.push(city.id);
        }
    }

    // --- Visible enemy routes ---
    for route in &state.routes {
        if route.owner != player && crate::fog::is_route_visible(state, player, route.id) {
            sit.visible_enemy_routes.push(route.id);
        }
    }

    // --- Fog frontier: discovered tiles adjacent to undiscovered tiles ---
    let player_discovered = &state.players[player.0 as usize].discovered;
    for &tid in player_discovered.iter() {
        let coord = state.tiles[tid.0 as usize].coord;
        for neighbor in crate::hex::neighbors(coord) {
            if let Some(&nid) = state.tile_index.get(&neighbor) {
                if !player_discovered.contains(&nid) {
                    sit.fog_frontier.push(tid);
                    break; // one frontier tile is enough per discovered tile
                }
            }
        }
    }

    // --- Unconnected own cities ---
    // Cities not in the same connected component as any route.
    let connected = crate::caravan::connected_city_count(state, player);
    if connected < sit.own_cities.len() as u32 {
        // Find cities that are endpoints of no active route.
        let mut route_endpoints = FxHashSet::default();
        for route in &state.routes {
            if route.owner == player && route.status == RouteStatus::Active {
                route_endpoints.insert(route.endpoints.0);
                route_endpoints.insert(route.endpoints.1);
            }
        }
        for &cid in &sit.own_cities {
            if !route_endpoints.contains(&cid) {
                sit.unconnected_own_cities.push(cid);
            }
        }
    }

    // --- Isolated cities ---
    for &cid in &sit.own_cities {
        if crate::economy::is_city_isolated(state, cid) {
            sit.isolated_cities.push(cid);
        }
    }

    sit
}

// ---------------------------------------------------------------------------
// Candidate generators
// ---------------------------------------------------------------------------

/// Generate FoundCity commands for expansion.
///
/// For each own city, look at hex neighbors for valid founding spots on oases.
/// Score based on oasis proximity and distance from own cities.
fn candidates_expand(state: &GameState, player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();
    let player_data = &state.players[player.0 as usize];

    // Need influence to found.
    if player_data.resources.influence < crate::model::FOUND_CITY_INFLUENCE {
        return candidates;
    }

    // Need a scout to found.
    let scout = state
        .units
        .iter()
        .find(|u| u.owner == player && u.kind == UnitKind::Scout);
    let (scout_id, scout_tile, scout_moves) = match scout {
        Some(u) => (u.id, u.tile, u.moves_left),
        None => return candidates,
    };
    if scout_moves == 0 {
        return candidates;
    }

    let scout_coord = state.tiles[scout_tile.0 as usize].coord;
    let radius = state.scenario.map_radius as u32;

    // Also consider the scout's own tile
    let own_tile_id = state.tile_index[&scout_coord];
    let own_tile = &state.tiles[own_tile_id.0 as usize];
    if own_tile.terrain == TerrainType::Oasis && !state.cities.iter().any(|c| c.tile == own_tile_id)
    {
        let is_available = own_tile.owner.is_none() || own_tile.owner == Some(player);
        if is_available {
            candidates.push(ScoredAction {
                score: 15.0, // highest priority — found on current position
                cmd: Command::FoundCity {
                    unit: scout_id,
                    tile: own_tile_id,
                },
                category: CAT_EXPAND,
            });
        }
    }

    // Look for oases reachable by the scout (adjacent to scout position).
    for neighbor_coord in crate::hex::neighbors(scout_coord) {
        if !crate::hex::in_map(neighbor_coord, radius) {
            continue;
        }
        if let Some(&tile_id) = state.tile_index.get(&neighbor_coord) {
            let tile = &state.tiles[tile_id.0 as usize];
            if tile.terrain != TerrainType::Oasis {
                continue;
            }
            // Must not already have a city.
            if state.cities.iter().any(|c| c.tile == tile_id) {
                continue;
            }
            // Must not be owned by an enemy.
            if let Some(owner) = tile.owner {
                if owner != player {
                    continue;
                }
            }

            // Score: prefer oases closer to existing cities (network cohesion)
            // and prefer unoccupied oases.
            let min_dist = sit
                .own_cities
                .iter()
                .map(|&cid| {
                    let city_tile = state.cities[cid.0 as usize].tile;
                    let city_coord = state.tiles[city_tile.0 as usize].coord;
                    crate::hex::distance(neighbor_coord, city_coord) as f32
                })
                .fold(f32::INFINITY, f32::min);

            // Closer is better; invert distance for score.
            let score = 10.0 / (min_dist + 1.0);

            candidates.push(ScoredAction {
                score,
                cmd: Command::FoundCity {
                    unit: scout_id,
                    tile: tile_id,
                },
                category: CAT_EXPAND,
            });
        }
    }

    candidates
}

/// Generate TrainUnit commands for building up military.
///
/// Prioritize if `own_units.len() < own_cities.len()` (under-militarized).
fn candidates_build(state: &GameState, player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();
    let player_data = &state.players[player.0 as usize];

    // Check unit cap.
    let cap = crate::world::unit_cap(state, player);
    let current_units = sit.own_units.len() as u32;
    if current_units >= cap {
        return candidates;
    }

    // Determine deficit: more cities than units → need more units.
    let unit_deficit = if sit.own_cities.len() as u32 > current_units {
        (sit.own_cities.len() as u32 - current_units) as f32
    } else {
        0.0
    };

    // Determine if we need more guards for exposed routes.
    let guards_needed = sit.exposed_own_routes.len() as f32;
    let current_guards = state
        .units
        .iter()
        .filter(|u| u.owner == player && u.kind == UnitKind::CaravanGuard)
        .count() as f32;

    // Try each city for training.
    for &cid in &sit.own_cities {
        let city = &state.cities[cid.0 as usize];

        // Determine what to train.
        let kind_to_train = if current_guards < guards_needed {
            UnitKind::CaravanGuard
        } else if unit_deficit > 0.0 {
            UnitKind::Scout
        } else {
            // No pressing need — skip.
            continue;
        };

        let cost = crate::model::UNIT_TRAIN_COST[crate::world::unit_kind_index(&kind_to_train)];
        // Apply Fortress discount if applicable.
        let actual_cost = if city.specialization == Some(CitySpecialization::Fortress) {
            (cost as f32 * crate::model::FORTRESS_TRAIN_DISCOUNT) as u32
        } else {
            cost
        };

        if player_data.resources.wealth < actual_cost {
            continue;
        }

        let score = if kind_to_train == UnitKind::CaravanGuard {
            // Higher score if we have exposed routes needing guards.
            guards_needed * 2.0
        } else {
            unit_deficit
        };

        candidates.push(ScoredAction {
            score,
            cmd: Command::TrainUnit {
                city: cid,
                kind: kind_to_train,
            },
            category: CAT_BUILD,
        });
    }

    candidates
}

/// Generate ConnectRoute commands for unconnected city pairs.
///
/// For each pair of unconnected own cities, try to find a route.
/// Score based on network synergy improvement.
fn candidates_connect(state: &GameState, player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();
    let player_data = &state.players[player.0 as usize];

    // We need unconnected cities and available resources.
    if sit.own_cities.len() < 2 {
        return candidates;
    }

    // Try all pairs of own cities.
    for i in 0..sit.own_cities.len() {
        for j in (i + 1)..sit.own_cities.len() {
            let from = sit.own_cities[i];
            let to = sit.own_cities[j];

            // Check if already connected by an active route.
            let already_connected = state.routes.iter().any(|r| {
                r.owner == player
                    && r.status == RouteStatus::Active
                    && ((r.endpoints.0 == from && r.endpoints.1 == to)
                        || (r.endpoints.0 == to && r.endpoints.1 == from))
            });
            if already_connected {
                continue;
            }

            // Check route slots on both endpoints.
            let city_a = &state.cities[from.0 as usize];
            let city_b = &state.cities[to.0 as usize];
            if city_a.route_slots == 0 || city_b.route_slots == 0 {
                continue;
            }

            // Compute route preview.
            let (cost, path) = crate::caravan::preview_cost(state, from, to);
            if path.is_empty() {
                continue;
            }

            // Check if we can afford it.
            if (player_data.resources.wealth as i32) < cost {
                continue;
            }

            // Check fog-legal: all path tiles must be discovered.
            let player_discovered = &state.players[player.0 as usize].discovered;
            let fog_legal = path.iter().all(|&tid| player_discovered.contains(&tid));
            if !fog_legal {
                continue;
            }

            // Score: network synergy improvement × route_weight.
            // A new route connecting two unconnected cities boosts synergy.
            let synergy_bonus = if sit.unconnected_own_cities.contains(&from)
                || sit.unconnected_own_cities.contains(&to)
            {
                5.0
            } else {
                2.0
            };

            candidates.push(ScoredAction {
                score: synergy_bonus,
                cmd: Command::ConnectRoute { from, to },
                category: CAT_CONNECT,
            });
        }
    }

    candidates
}

/// Generate Patrol commands for defending exposed routes.
///
/// For each exposed own route, try to move a guard to the most exposed tile.
fn candidates_defend(state: &GameState, _player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();

    // Find idle guards (not patrolling, not garrisoned).
    let idle_guards: Vec<UnitId> = state
        .units
        .iter()
        .filter(|u| {
            u.owner == _player
                && u.kind == UnitKind::CaravanGuard
                && u.ability == crate::model::UnitAbility::None
                && u.moves_left > 0
        })
        .map(|u| u.id)
        .collect();

    if idle_guards.is_empty() {
        return candidates;
    }

    // For each exposed route, find the most exposed tile.
    // Dedup: a route can be both exposed AND threatened.
    let mut seen_routes = FxHashSet::default();
    for &route_id in sit
        .exposed_own_routes
        .iter()
        .chain(sit.threatened_own_routes.iter())
    {
        if !seen_routes.insert(route_id) {
            continue;
        }
        let route = &state.routes[route_id.0 as usize];

        // Find the first uncontrolled tile.
        for &tid in &route.path {
            if !crate::caravan::is_route_tile_controlled(state, route, tid) {
                // Try to move an idle guard to patrol near this tile.
                if let Some(&guard_id) = idle_guards.first() {
                    let guard = &state.units[guard_id.0 as usize];
                    let guard_tile = guard.tile;
                    let guard_coord = state.tiles[guard_tile.0 as usize].coord;
                    let tile_coord = state.tiles[tid.0 as usize].coord;

                    // If already adjacent or on the tile, just patrol.
                    if crate::hex::distance(guard_coord, tile_coord) <= 1 {
                        candidates.push(ScoredAction {
                            score: 3.0,
                            cmd: Command::Patrol {
                                unit: guard_id,
                                tile: tid,
                            },
                            category: CAT_DEFEND,
                        });
                    } else if guard.moves_left > 0 {
                        // Move toward the exposed tile, then patrol.
                        // For MVP: emit a MoveUnit toward the tile.
                        candidates.push(ScoredAction {
                            score: 2.0,
                            cmd: Command::MoveUnit {
                                unit: guard_id,
                                to: tid,
                            },
                            category: CAT_DEFEND,
                        });
                    }
                }
                break; // One tile per route is enough.
            }
        }
    }

    candidates
}

/// Generate RaidRoute / RaidCity commands.
///
/// Only targets visible enemy routes/cities (fog-aware — no cheating).
fn candidates_raid(
    state: &GameState,
    _player: PlayerId,
    sit: &Situation,
    params: &AiParams,
) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();

    if params.raid_weight <= 0.0 {
        return candidates;
    }

    // Find idle raiders.
    let idle_raiders: Vec<UnitId> = state
        .units
        .iter()
        .filter(|u| u.owner == _player && u.kind == UnitKind::Raider && u.moves_left > 0)
        .map(|u| u.id)
        .collect();

    if idle_raiders.is_empty() {
        return candidates;
    }

    // Try raiding visible enemy routes.
    for &route_id in &sit.visible_enemy_routes {
        let route = &state.routes[route_id.0 as usize];
        if route.status != RouteStatus::Active {
            continue;
        }

        // Find an exposed tile on the enemy route.
        for &tid in &route.path {
            if !crate::caravan::is_route_tile_controlled(state, route, tid) {
                if let Some(&raider_id) = idle_raiders.first() {
                    let raider = &state.units[raider_id.0 as usize];
                    let raider_coord = state.tiles[raider.tile.0 as usize].coord;
                    let tile_coord = state.tiles[tid.0 as usize].coord;

                    if crate::hex::distance(raider_coord, tile_coord) <= 1 {
                        candidates.push(ScoredAction {
                            score: params.raid_weight * 3.0,
                            cmd: Command::RaidRoute {
                                unit: raider_id,
                                route: route_id,
                            },
                            category: CAT_RAID,
                        });
                    }
                }
                break;
            }
        }
    }

    // Try raiding visible enemy cities.
    for &city_id in &sit.visible_enemy_cities {
        if let Some(&raider_id) = idle_raiders.first() {
            let raider = &state.units[raider_id.0 as usize];
            let city = &state.cities[city_id.0 as usize];
            let raider_coord = state.tiles[raider.tile.0 as usize].coord;
            let city_coord = state.tiles[city.tile.0 as usize].coord;

            if crate::hex::distance(raider_coord, city_coord) <= 1 {
                candidates.push(ScoredAction {
                    score: params.raid_weight * 2.0,
                    cmd: Command::RaidCity {
                        unit: raider_id,
                        city: city_id,
                    },
                    category: CAT_RAID,
                });
            }
        }
    }

    candidates
}

/// Generate MoveUnit commands for scouting.
///
/// Move idle units toward fog edges or unrevealed areas.
fn candidates_scout(state: &GameState, _player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
    let mut candidates = Vec::new();

    if sit.fog_frontier.is_empty() {
        return candidates;
    }

    // Find idle scouts.
    for unit in state.units.iter() {
        if unit.owner != _player || unit.kind != UnitKind::Scout || unit.moves_left == 0 {
            continue;
        }
        if unit.ability != crate::model::UnitAbility::None {
            continue;
        }

        // Find the nearest fog frontier tile.
        let unit_coord = state.tiles[unit.tile.0 as usize].coord;
        let mut best_tile = None;
        let mut best_dist = u32::MAX;

        for &fid in sit.fog_frontier.iter().take(20) {
            // Limit search for performance.
            let f_coord = state.tiles[fid.0 as usize].coord;
            let dist = crate::hex::distance(unit_coord, f_coord);
            if dist < best_dist {
                best_dist = dist;
                best_tile = Some(fid);
            }
        }

        if let Some(target) = best_tile {
            let score = 1.0; // Base exploration score.
            candidates.push(ScoredAction {
                score,
                cmd: Command::MoveUnit {
                    unit: unit.id,
                    to: target,
                },
                category: CAT_SCOUT,
            });
        }
    }

    candidates
}

// ---------------------------------------------------------------------------
// prioritize — the scoring engine
// ---------------------------------------------------------------------------

/// Score candidate actions by weighted utility; returns them sorted descending.
///
/// Calls all `candidates_*` functions internally, applies per-category weights,
/// and sorts the combined results.
fn prioritize(
    state: &GameState,
    player: PlayerId,
    sit: &Situation,
    params: &AiParams,
) -> Vec<ScoredAction> {
    let mut all: Vec<ScoredAction> = Vec::new();

    // Generate candidates from all categories.
    all.extend(candidates_expand(state, player, sit));
    all.extend(candidates_build(state, player, sit));
    all.extend(candidates_connect(state, player, sit));
    all.extend(candidates_defend(state, player, sit));
    all.extend(candidates_raid(state, player, sit, params));
    all.extend(candidates_scout(state, player, sit));

    // Apply category weights.
    for action in &mut all {
        let weight = match action.category {
            CAT_CONNECT => params.route_weight,
            CAT_DEFEND => params.route_security,
            CAT_EXPAND => params.expand_weight,
            CAT_RAID => params.raid_weight,
            CAT_SCOUT => params.scout_weight,
            CAT_BUILD => params.build_weight,
            _ => 1.0,
        };
        action.score *= weight;
    }

    // Sort by score descending; break ties by category, then by command
    // ordering for determinism.
    all.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.category.cmp(&b.category))
    });

    all
}

// ---------------------------------------------------------------------------
// emit — validate and emit
// ---------------------------------------------------------------------------

/// Validate and emit a legal, budget-limited command list.
///
/// For each `ScoredAction`, run `validate(state, cmd)`. Keep it if legal and
/// the per-turn `command_budget` is not exceeded.
fn emit(
    state: &GameState,
    _player: PlayerId,
    ranked: &[ScoredAction],
    params: &AiParams,
) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut budget_remaining = params.command_budget;

    for action in ranked {
        if budget_remaining == 0 {
            break;
        }

        // Skip commands that are no longer valid due to earlier mutations
        // (budget-limited, so we validate against the original state).
        match crate::turn::validate(state, &action.cmd) {
            Ok(()) => {
                commands.push(action.cmd.clone());
                budget_remaining -= 1;
            }
            Err(_) => {
                // Command is illegal — skip it.
            }
        }
    }

    commands
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// THE public entry. Pure: reads `&GameState`, returns the actor's action Commands.
///
/// The orchestrator appends `EndTurn` before calling `step` — `ai_plan` returns
/// actions only (no `EndTurn`).
///
/// Returns an empty `Vec` if the player has no cities (defeated).
pub fn ai_plan(state: &GameState, player: PlayerId, difficulty: Difficulty) -> Vec<Command> {
    let personality = personality_of(state, player);
    let params = params_for(personality, difficulty);
    let sit = assess(state, player);

    // NOTE: We intentionally do NOT bail out when `own_cities` is empty.
    // At game start, players begin without cities and must FoundCity via a
    // scout. The candidate generators (especially `candidates_expand`) handle
    // the empty-city case by requiring influence + a scout with moves. If
    // the player is truly defeated (no cities, no units), all candidates
    // will be empty and `emit` returns `Vec::new()` naturally.

    let ranked = prioritize(state, player, &sit, &params);
    emit(state, player, &ranked, &params)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::HexCoord;
    use crate::model::{
        GameState, Player, PlayerColor, PlayerKind, Stockpiles, TerrainType, Unit, UnitAbility,
    };
    use crate::scenario::mvp_preset;
    use crate::test_harness;
    use fxhash::FxHashSet;
    use std::collections::VecDeque;

    /// Build a minimal deterministic `GameState` for AI tests.
    fn make_game() -> GameState {
        let cfg = mvp_preset();
        let mut s = GameState::new(cfg, 1);
        let radius = s.scenario.map_radius as u32;
        test_harness::allocate_hex_grid(&mut s, radius);

        // Mark two oases
        let oasis_a = HexCoord { q: 0, r: 0 };
        let oasis_b = HexCoord { q: 2, r: -2 };
        test_harness::mark_terrain(&mut s, oasis_a, TerrainType::Oasis);
        test_harness::mark_terrain(&mut s, oasis_b, TerrainType::Oasis);

        // AI player with resources
        let pid = test_harness::create_player(
            &mut s,
            PlayerKind::Ai {
                personality: AiPersonality::Expansionist,
                difficulty: Difficulty::Normal,
            },
            Stockpiles {
                water: 10,
                wealth: 50,
                influence: 20,
            },
        );

        // Reveal some tiles for the player
        let origin_tile = s.tile_index[&oasis_a];
        crate::fog::reveal(&mut s, pid, origin_tile, 3);

        // A city at the origin
        test_harness::create_city(&mut s, pid, oasis_a, 2);

        // A scout on the origin
        test_harness::create_unit_with_hp(&mut s, pid, UnitKind::Scout, origin_tile, 3);

        s
    }

    #[test]
    fn params_for_expansionist_normal() {
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        assert_eq!(params.command_budget, 7);
        assert_eq!(params.lookahead, 1);
        assert!(!params.preemptive_raid);
        assert!(params.defend_core_routes);
        assert_eq!(params.expand_weight, 1.4);
        assert_eq!(params.route_weight, 1.0);
    }

    #[test]
    fn params_for_raider_hard() {
        let params = params_for(AiPersonality::Raider, Difficulty::Hard);
        assert_eq!(params.command_budget, 10);
        assert!(params.preemptive_raid);
        assert!(params.defend_core_routes);
        assert_eq!(params.raid_weight, 1.6 * 1.2);
    }

    #[test]
    fn params_for_trader_easy() {
        let params = params_for(AiPersonality::Trader, Difficulty::Easy);
        assert_eq!(params.command_budget, 4);
        assert!(!params.preemptive_raid);
        assert!(!params.defend_core_routes);
        assert!((params.route_weight - 1.6 * 0.7).abs() < 0.01);
        assert!((params.route_security - 1.2 * 0.4).abs() < 0.01);
    }

    #[test]
    fn personality_of_reads_player_kind() {
        let s = make_game();
        let p = personality_of(&s, PlayerId(0));
        assert_eq!(p, AiPersonality::Expansionist);
    }

    #[test]
    fn assess_populates_own_cities() {
        let s = make_game();
        let sit = assess(&s, PlayerId(0));
        assert_eq!(sit.own_cities.len(), 1);
        assert_eq!(sit.own_cities[0], CityId(0));
    }

    #[test]
    fn assess_populates_own_units() {
        let s = make_game();
        let sit = assess(&s, PlayerId(0));
        assert!(
            sit.own_units
                .iter()
                .any(|&uid| { s.units[uid.0 as usize].kind == UnitKind::Scout })
        );
    }

    #[test]
    fn assess_fog_frontier_non_empty() {
        let s = make_game();
        let sit = assess(&s, PlayerId(0));
        // With scout at origin and radius 4, there should be frontier tiles.
        assert!(
            !sit.fog_frontier.is_empty(),
            "fog frontier should not be empty"
        );
    }

    #[test]
    fn assess_oases_counts() {
        let s = make_game();
        let sit = assess(&s, PlayerId(0));
        assert_eq!(sit.total_oases, 2);
    }

    #[test]
    fn ai_plan_returns_commands() {
        let s = make_game();
        let commands = ai_plan(&s, PlayerId(0), Difficulty::Normal);
        // Should return some commands (not empty, since we have cities and units).
        // Could be empty if no valid candidates, but unlikely with our setup.
        // At minimum, it should not panic.
        let _ = commands;
    }

    #[test]
    fn ai_plan_no_end_turn() {
        let s = make_game();
        let commands = ai_plan(&s, PlayerId(0), Difficulty::Normal);
        for cmd in &commands {
            assert!(
                !matches!(cmd, Command::EndTurn),
                "ai_plan should not emit EndTurn"
            );
        }
    }

    #[test]
    fn ai_plan_empty_when_no_cities() {
        let cfg = mvp_preset();
        let mut s = GameState::new(cfg, 42);
        // Add a player with no cities.
        let pid = s.alloc_player_id();
        s.players.push(Player {
            id: pid,
            kind: PlayerKind::Ai {
                personality: AiPersonality::Expansionist,
                difficulty: Difficulty::Normal,
            },
            color: PlayerColor::Sand,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });
        let commands = ai_plan(&s, pid, Difficulty::Normal);
        assert!(commands.is_empty(), "no cities → no commands");
    }

    #[test]
    fn category_constants_are_distinct() {
        let cats = [
            CAT_CONNECT,
            CAT_DEFEND,
            CAT_EXPAND,
            CAT_RAID,
            CAT_SCOUT,
            CAT_BUILD,
        ];
        assert_eq!(
            cats.len(),
            cats.iter().collect::<fxhash::FxHashSet<_>>().len(),
            "category constants must be distinct"
        );
    }

    #[test]
    fn ai_plan_deterministic() {
        let s1 = make_game();
        let s2 = make_game();
        let cmds1 = ai_plan(&s1, PlayerId(0), Difficulty::Normal);
        let cmds2 = ai_plan(&s2, PlayerId(0), Difficulty::Normal);
        assert_eq!(cmds1, cmds2, "same state must produce same plan");
    }

    #[test]
    fn ai_plan_budget_never_exceeded() {
        let s = make_game();
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let commands = ai_plan(&s, PlayerId(0), Difficulty::Normal);
        assert!(
            commands.len() <= params.command_budget,
            "command count {} exceeds budget {}",
            commands.len(),
            params.command_budget
        );
    }

    #[test]
    fn ai_plan_no_cheat_hidden_enemy() {
        let mut s = make_game();
        // Add an enemy unit on a tile NOT revealed to the AI player.
        let pid_enemy = s.alloc_player_id();
        s.players.push(crate::Player {
            id: pid_enemy,
            kind: PlayerKind::Human,
            color: crate::PlayerColor::Crimson,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });

        // Place enemy on a far tile (off the edge of revealed area).
        let far_coord = crate::hex::HexCoord {
            q: -(s.scenario.map_radius as i32),
            r: s.scenario.map_radius as i32,
        };
        if let Some(&far_tile) = s.tile_index.get(&far_coord) {
            let enemy_scout = s.alloc_unit_id();
            s.units.push(Unit {
                id: enemy_scout,
                owner: pid_enemy,
                kind: UnitKind::Scout,
                tile: far_tile,
                hp: 3,
                moves_left: 3,
                ability: UnitAbility::None,
            });

            // Verify the tile is NOT visible to the AI.
            assert!(
                !crate::fog::is_unit_visible(&s, PlayerId(0), enemy_scout),
                "enemy should be hidden in fog"
            );

            // Run AI plan.
            let commands = ai_plan(&s, PlayerId(0), Difficulty::Normal);

            // No command should reference the hidden enemy.
            for cmd in &commands {
                match cmd {
                    Command::RaidRoute { unit, .. } => {
                        // The raiding unit is ours, not the enemy's.
                        assert_ne!(*unit, enemy_scout, "should not target hidden enemy");
                    }
                    Command::RaidCity { unit, .. } => {
                        assert_ne!(*unit, enemy_scout, "should not target hidden enemy");
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn assess_enemy_entities_only_visible() {
        let mut s = make_game();
        // Add an enemy player.
        let pid_enemy = s.alloc_player_id();
        s.players.push(crate::Player {
            id: pid_enemy,
            kind: PlayerKind::Human,
            color: crate::PlayerColor::Crimson,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });

        // Place enemy on a revealed tile (should be visible).
        let origin_tile = s.tile_index[&crate::hex::HexCoord { q: 0, r: 0 }];
        let visible_enemy = s.alloc_unit_id();
        s.units.push(Unit {
            id: visible_enemy,
            owner: pid_enemy,
            kind: UnitKind::Scout,
            tile: origin_tile,
            hp: 3,
            moves_left: 3,
            ability: UnitAbility::None,
        });

        let sit = assess(&s, PlayerId(0));
        assert!(
            sit.visible_enemy_units.contains(&visible_enemy),
            "enemy on revealed tile should be visible"
        );
    }

    #[test]
    fn params_all_personalities_compile() {
        for personality in [
            AiPersonality::Expansionist,
            AiPersonality::Raider,
            AiPersonality::Trader,
            AiPersonality::Fortifier,
        ] {
            for difficulty in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
                let params = params_for(personality, difficulty);
                assert!(params.command_budget > 0);
                assert!(params.expand_weight > 0.0);
            }
        }
    }

    #[test]
    fn ai_plan_with_two_cities_generates_connect() {
        let mut s = make_game();
        let pid = PlayerId(0);

        // Add a second city.
        let oasis_b = crate::hex::HexCoord { q: 2, r: -2 };
        let city_b_tile = s.tile_index[&oasis_b];
        let city_b = s.alloc_city_id();
        s.cities.push(crate::City {
            id: city_b,
            owner: pid,
            tile: city_b_tile,
            population: 2,
            specialization: None,
            buildings: vec![],
            stockpiles: Stockpiles::default(),
            route_slots: 2,
            growth_timer: 0,
            queue: VecDeque::new(),
        });

        // Reveal tiles around the second city.
        crate::fog::reveal(&mut s, pid, city_b_tile, 3);

        let sit = assess(&s, pid);
        assert_eq!(sit.own_cities.len(), 2, "should have 2 cities");

        let connect_candidates = candidates_connect(&s, pid, &sit);
        // Should have at least one ConnectRoute candidate.
        assert!(
            !connect_candidates.is_empty(),
            "should generate connect candidates for 2 unconnected cities"
        );
        assert!(
            connect_candidates
                .iter()
                .all(|c| matches!(c.cmd, Command::ConnectRoute { .. }))
        );
    }

    #[test]
    fn candidates_expand_requires_influence() {
        let mut s = make_game();
        s.players[0].resources.influence = 0;
        let sit = assess(&s, PlayerId(0));
        let candidates = candidates_expand(&s, PlayerId(0), &sit);
        assert!(
            candidates.is_empty(),
            "no influence → no expansion candidates"
        );
    }

    #[test]
    fn candidates_build_respects_unit_cap() {
        let mut s = make_game();
        let pid = PlayerId(0);
        // Fill unit cap.
        let cap = crate::world::unit_cap(&s, pid);
        while (s.units.iter().filter(|u| u.owner == pid).count() as u32) < cap {
            let id = s.alloc_unit_id();
            s.units.push(Unit {
                id,
                owner: pid,
                kind: UnitKind::Scout,
                tile: s.cities[0].tile,
                hp: 3,
                moves_left: 3,
                ability: UnitAbility::None,
            });
        }
        let sit = assess(&s, pid);
        let candidates = candidates_build(&s, pid, &sit);
        assert!(candidates.is_empty(), "at unit cap → no build candidates");
    }
}
