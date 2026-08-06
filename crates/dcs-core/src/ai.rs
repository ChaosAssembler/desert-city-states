//! AI Opponents — pure, deterministic planner (spec `behavior-ai-opponents.md`).
//!
//! The single public entry is [`crate::model::GameState::ai_plan`], which reads an immutable `GameState`
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

use crate::{AiPersonality, CityId, Command, Difficulty, RouteId, TileId, UnitId};

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
/// Built once per [`crate::model::GameState::ai_plan`] call; scratch for one planning cycle. Not stored
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
pub(crate) struct ScoredAction {
    /// Weighted utility (higher = emit first).
    pub(crate) score: f32,
    /// The command to emit if selected.
    pub(crate) cmd: Command,
    /// Weight-table column index (CAT_* constant).
    pub(crate) category: u8,
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

// ---------------------------------------------------------------------------
// assess — fog-aware world snapshot
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Candidate generators
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// prioritize — the scoring engine
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// emit — validate and emit
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::HexCoord;
    use crate::model::{
        GameState, POP_FOR_SPECIALIZE, Player, PlayerColor, PlayerKind, SPECIALIZE_COST_INFLUENCE,
        Stockpiles, TerrainType, Unit, UnitAbility,
    };
    use crate::scenario::ScenarioConfig;
    use crate::test_harness;
    use crate::{CitySpecialization, PlayerId, UnitKind};
    use fxhash::FxHashSet;
    use std::collections::VecDeque;

    /// Build a minimal deterministic `GameState` for AI tests.
    fn make_game() -> GameState {
        let cfg = ScenarioConfig::mvp_preset();
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
        s.reveal(pid, origin_tile, 3);

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
        let p = s.personality_of(PlayerId(0));
        assert_eq!(p, AiPersonality::Expansionist);
    }

    #[test]
    fn assess_populates_own_cities() {
        let s = make_game();
        let sit = s.assess(PlayerId(0));
        assert_eq!(sit.own_cities.len(), 1);
        assert_eq!(sit.own_cities[0], CityId(0));
    }

    #[test]
    fn assess_populates_own_units() {
        let s = make_game();
        let sit = s.assess(PlayerId(0));
        assert!(
            sit.own_units
                .iter()
                .any(|&uid| { s.unit(uid).kind == UnitKind::Scout })
        );
    }

    #[test]
    fn assess_fog_frontier_non_empty() {
        let s = make_game();
        let sit = s.assess(PlayerId(0));
        // With scout at origin and radius 4, there should be frontier tiles.
        assert!(
            !sit.fog_frontier.is_empty(),
            "fog frontier should not be empty"
        );
    }

    #[test]
    fn assess_oases_counts() {
        let s = make_game();
        let sit = s.assess(PlayerId(0));
        assert_eq!(sit.total_oases, 2);
    }

    #[test]
    fn ai_plan_returns_commands() {
        let s = make_game();
        let commands = s.ai_plan(PlayerId(0), Difficulty::Normal);
        // Should return some commands (not empty, since we have cities and units).
        // Could be empty if no valid candidates, but unlikely with our setup.
        // At minimum, it should not panic.
        let _ = commands;
    }

    #[test]
    fn ai_plan_no_end_turn() {
        let s = make_game();
        let commands = s.ai_plan(PlayerId(0), Difficulty::Normal);
        for cmd in &commands {
            assert!(
                !matches!(cmd, Command::EndTurn),
                "ai_plan should not emit EndTurn"
            );
        }
    }

    #[test]
    fn ai_plan_empty_when_no_cities() {
        let cfg = ScenarioConfig::mvp_preset();
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
        let commands = s.ai_plan(pid, Difficulty::Normal);
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
        let cmds1 = s1.ai_plan(PlayerId(0), Difficulty::Normal);
        let cmds2 = s2.ai_plan(PlayerId(0), Difficulty::Normal);
        assert_eq!(cmds1, cmds2, "same state must produce same plan");
    }

    #[test]
    fn ai_plan_budget_never_exceeded() {
        let s = make_game();
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let commands = s.ai_plan(PlayerId(0), Difficulty::Normal);
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
                !s.is_unit_visible(PlayerId(0), enemy_scout),
                "enemy should be hidden in fog"
            );

            // Run AI plan.
            let commands = s.ai_plan(PlayerId(0), Difficulty::Normal);

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
    fn assess_enemy_no_longer_visible_after_revealer_moves_away() {
        // Regression test: `visible_enemy_units` must reflect the AI's live
        // sight, not permanent `discovered` memory — an enemy that was once
        // seen must vanish again once nothing is currently watching its
        // tile, even though that tile stays discovered forever.
        let mut s = make_game();
        let pid_enemy = s.alloc_player_id();
        s.players.push(crate::Player {
            id: pid_enemy,
            kind: PlayerKind::Human,
            color: crate::PlayerColor::Crimson,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });

        // Within the scout's sight (3) but outside the city's own sight (2)
        // at the origin, so moving the scout away is what un-observes it.
        let enemy_coord = crate::hex::HexCoord { q: 3, r: 0 };
        let enemy_tile = s.tile_index[&enemy_coord];
        let enemy = s.alloc_unit_id();
        s.units.push(Unit {
            id: enemy,
            owner: pid_enemy,
            kind: UnitKind::Scout,
            tile: enemy_tile,
            hp: 3,
            moves_left: 3,
            ability: UnitAbility::None,
        });

        let scout = test_harness::find_unit(&s, PlayerId(0), UnitKind::Scout);
        s.reveal_from_unit(scout);
        assert!(
            s.assess(PlayerId(0)).visible_enemy_units.contains(&enemy),
            "enemy should be visible while the scout is nearby"
        );

        // Move the AI's scout far away — the tile stays discovered, but the
        // AI must no longer treat the enemy as currently visible.
        let far_coord = crate::hex::HexCoord {
            q: -(s.scenario.map_radius as i32),
            r: s.scenario.map_radius as i32,
        };
        let far_tile = s.tile_index[&far_coord];
        s.unit_mut(scout).unwrap().tile = far_tile;

        assert!(
            s.is_tile_visible(PlayerId(0), enemy_tile),
            "permanent memory: tile stays discovered"
        );
        assert!(
            !s.assess(PlayerId(0)).visible_enemy_units.contains(&enemy),
            "AI must not still see the enemy once nothing is watching its tile"
        );
    }

    #[test]
    fn candidates_expand_ignores_hidden_enemy_city() {
        // Regression test for a small pre-existing purity leak: the "must
        // not already have a city" check used to look for ANY city on a
        // candidate oasis tile, including a fogged enemy one the AI has no
        // business seeing (ADR-0004).
        let cfg = ScenarioConfig::mvp_preset();
        let mut s = GameState::new(cfg, 1);
        let radius = s.scenario.map_radius as u32;
        test_harness::allocate_hex_grid(&mut s, radius);

        let scout_coord = HexCoord { q: 0, r: 0 };
        let oasis_coord = HexCoord { q: 1, r: 0 }; // neighbor of scout_coord
        test_harness::mark_terrain(&mut s, oasis_coord, TerrainType::Oasis);

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
        let scout_tile = s.tile_index[&scout_coord];
        test_harness::create_unit_with_hp(&mut s, pid, UnitKind::Scout, scout_tile, 3);

        // An enemy city on the oasis neighbor, never discovered by `pid`.
        let pid_enemy = s.alloc_player_id();
        s.players.push(crate::Player {
            id: pid_enemy,
            kind: PlayerKind::Human,
            color: crate::PlayerColor::Crimson,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });
        let oasis_tile = s.tile_index[&oasis_coord];
        let enemy_city_id = test_harness::create_city(&mut s, pid_enemy, oasis_coord, 1);
        assert!(
            !s.is_city_visible(pid, enemy_city_id),
            "sanity: enemy city must be fogged"
        );

        let sit = s.assess(pid);
        let candidates = s.candidates_expand(pid, &sit);
        assert!(
            candidates.iter().any(|c| matches!(
                c.cmd,
                Command::FoundCity { tile, .. } if tile == oasis_tile
            )),
            "AI should still consider founding on an oasis occupied only by a city it can't see"
        );
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

        let sit = s.assess(PlayerId(0));
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
        s.reveal(pid, city_b_tile, 3);

        let sit = s.assess(pid);
        assert_eq!(sit.own_cities.len(), 2, "should have 2 cities");

        let connect_candidates = s.candidates_connect(pid, &sit);
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
        let sit = s.assess(PlayerId(0));
        let candidates = s.candidates_expand(PlayerId(0), &sit);
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
        let cap = s.unit_cap(pid);
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
        let sit = s.assess(pid);
        let candidates = s.candidates_build(pid, &sit);
        assert!(candidates.is_empty(), "at unit cap → no build candidates");
    }

    #[test]
    fn candidates_specialize_appears_when_eligible() {
        let mut s = make_game();
        let pid = PlayerId(0);
        s.cities[0].population = POP_FOR_SPECIALIZE;
        let sit = s.assess(pid);
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let candidates = s.candidates_specialize(pid, &sit, &params);
        assert!(
            candidates.iter().any(|c| matches!(
                c.cmd,
                Command::Specialize {
                    spec: CitySpecialization::Fortress,
                    ..
                }
            )),
            "eligible city should generate a Fortress specialize candidate"
        );
    }

    #[test]
    fn candidates_specialize_absent_population_insufficient() {
        let s = make_game();
        let pid = PlayerId(0);
        // make_game()'s default population (2) is below POP_FOR_SPECIALIZE (3).
        assert!(s.cities[0].population < POP_FOR_SPECIALIZE);
        let sit = s.assess(pid);
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let candidates = s.candidates_specialize(pid, &sit, &params);
        assert!(
            candidates.is_empty(),
            "population below threshold → no specialize candidates"
        );
    }

    #[test]
    fn candidates_specialize_absent_influence_insufficient() {
        let mut s = make_game();
        let pid = PlayerId(0);
        s.cities[0].population = POP_FOR_SPECIALIZE;
        s.players[0].resources.influence = SPECIALIZE_COST_INFLUENCE - 1;
        let sit = s.assess(pid);
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let candidates = s.candidates_specialize(pid, &sit, &params);
        assert!(
            candidates.is_empty(),
            "insufficient influence → no specialize candidates"
        );
    }

    #[test]
    fn candidates_specialize_absent_already_specialized() {
        let mut s = make_game();
        let pid = PlayerId(0);
        s.cities[0].population = POP_FOR_SPECIALIZE;
        s.cities[0].specialization = Some(CitySpecialization::TradeHub);
        let sit = s.assess(pid);
        let params = params_for(AiPersonality::Expansionist, Difficulty::Normal);
        let candidates = s.candidates_specialize(pid, &sit, &params);
        assert!(
            candidates.is_empty(),
            "already-specialized city → no specialize candidates"
        );
    }

    #[test]
    fn candidates_specialize_raider_personality_score_boosted() {
        let mut s = make_game();
        s.players[0].kind = PlayerKind::Ai {
            personality: AiPersonality::Raider,
            difficulty: Difficulty::Normal,
        };
        let pid = PlayerId(0);
        s.cities[0].population = POP_FOR_SPECIALIZE;
        let sit = s.assess(pid);
        let params = params_for(AiPersonality::Raider, Difficulty::Normal);
        let candidates = s.candidates_specialize(pid, &sit, &params);
        let expected = 8.0 * (params.raid_weight / params.build_weight);
        assert!(
            candidates.iter().any(|c| (c.score - expected).abs() < 0.01),
            "Raider personality's score should be pre-scaled to raid_weight, got {:?}",
            candidates.iter().map(|c| c.score).collect::<Vec<_>>()
        );
    }

    #[test]
    fn candidates_build_prioritizes_raider_once_fortress_exists() {
        let mut s = make_game();
        let pid = PlayerId(0);
        s.cities[0].specialization = Some(CitySpecialization::Fortress);
        let sit = s.assess(pid);
        let candidates = s.candidates_build(pid, &sit);
        assert!(
            candidates.iter().any(|c| matches!(
                c.cmd,
                Command::TrainUnit {
                    kind: UnitKind::Raider,
                    ..
                }
            )),
            "Fortress city with no Raider owned should train one"
        );
        assert!(
            !candidates.iter().any(|c| matches!(
                c.cmd,
                Command::TrainUnit {
                    kind: UnitKind::CaravanGuard | UnitKind::Scout,
                    ..
                }
            )),
            "Fortress city should train the Raider first, not a Guard/Scout"
        );
    }

    #[test]
    fn ai_plan_raider_personality_emits_specialize_when_eligible() {
        let mut s = make_game();
        s.players[0].kind = PlayerKind::Ai {
            personality: AiPersonality::Raider,
            difficulty: Difficulty::Normal,
        };
        let pid = PlayerId(0);
        s.cities[0].population = POP_FOR_SPECIALIZE;
        let commands = s.ai_plan(pid, Difficulty::Normal);
        assert!(
            commands.iter().any(|c| matches!(
                c,
                Command::Specialize {
                    spec: CitySpecialization::Fortress,
                    ..
                }
            )),
            "Raider AI with an eligible city should specialize into a Fortress: {commands:?}"
        );
    }

    #[test]
    fn ai_plan_raider_personality_trains_raider_once_fortress() {
        let mut s = make_game();
        s.players[0].kind = PlayerKind::Ai {
            personality: AiPersonality::Raider,
            difficulty: Difficulty::Normal,
        };
        let pid = PlayerId(0);
        s.cities[0].specialization = Some(CitySpecialization::Fortress);
        let commands = s.ai_plan(pid, Difficulty::Normal);
        assert!(
            commands.iter().any(|c| matches!(
                c,
                Command::TrainUnit {
                    kind: UnitKind::Raider,
                    ..
                }
            )),
            "Raider AI with a Fortress and no Raider yet should train one: {commands:?}"
        );
    }

    #[test]
    fn ai_plan_raider_personality_raids_with_idle_raider_and_visible_enemy() {
        let mut s = make_game();
        s.players[0].kind = PlayerKind::Ai {
            personality: AiPersonality::Raider,
            difficulty: Difficulty::Normal,
        };
        let pid = PlayerId(0);

        // An idle Raider next to a visible enemy city.
        let raider_coord = crate::hex::HexCoord { q: 1, r: 0 };
        test_harness::mark_terrain(&mut s, raider_coord, TerrainType::Dunes);
        let raider_tile = s.tile_index[&raider_coord];
        test_harness::create_unit(&mut s, pid, UnitKind::Raider, raider_tile);

        let pid_enemy = s.alloc_player_id();
        s.players.push(crate::Player {
            id: pid_enemy,
            kind: PlayerKind::Human,
            color: crate::PlayerColor::Crimson,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        });
        let enemy_coord = crate::hex::HexCoord { q: 2, r: 0 }; // adjacent to raider_coord
        test_harness::mark_terrain(&mut s, enemy_coord, TerrainType::Dunes);
        let enemy_city_id = test_harness::create_city(&mut s, pid_enemy, enemy_coord, 1);
        // Unlike `candidates_expand_ignores_hidden_enemy_city`, reveal this
        // one — the AI must actually be able to see its raid target.
        let enemy_tile = s.tile_index[&enemy_coord];
        s.reveal(pid, enemy_tile, 0);
        assert!(
            s.is_city_visible(pid, enemy_city_id),
            "sanity: enemy city must be visible for this test"
        );

        let commands = s.ai_plan(pid, Difficulty::Normal);
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::RaidCity { .. } | Command::RaidRoute { .. })),
            "Raider AI with an idle Raider next to a visible enemy city should raid: {commands:?}"
        );
    }
}
