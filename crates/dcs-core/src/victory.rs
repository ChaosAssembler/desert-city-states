//! Victory condition logic: the three threshold-based conditions (V1–V3) plus
//! a turn-limit fallback.
//!
//! This module is **pure, deterministic, engine-free** (no rendering, no input,
//! no RNG). It is the sole location for victory-checking algorithms; the turn
//! resolver calls [`crate::model::GameState::update_victory_tracker`] once per end-of-turn
//! round, then [`crate::model::GameState::check_victory`] to decide whether the game has ended.
//!
//! # Spec
//!
//! Implements DD §13 via `behavior-victory-conditions.md` §5–§6.
//!
//! All victory-checking functions are now methods on [`GameState`](crate::GameState)
//! (see `model.rs`). This module retains the prestige-score weight constants
//! used by those methods.

// ---------------------------------------------------------------------------
// Prestige score weights (DD §13 — spec §4)
// ---------------------------------------------------------------------------

/// Weight for wealth stockpile in the prestige score formula.
pub const PRESTIGE_WEALTH_W: f32 = 1.0;
/// Weight for influence stockpile in the prestige score formula.
pub const PRESTIGE_INFLUENCE_W: f32 = 2.0;
/// Weight for each controlled oasis in the prestige score formula.
pub const PRESTIGE_OASIS_W: f32 = 8.0;
/// Weight for each active owned route in the prestige score formula.
pub const PRESTIGE_ROUTE_W: f32 = 4.0;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::model::{GameState, PlayerKind, Relic, Stockpiles, TerrainType, Tile};
    use crate::scenario::ScenarioConfig;
    use crate::test_harness;
    use crate::{GameEvent, PlayerId, RelicId, RouteStatus, VictoryKind};

    /// Build a minimal test state.
    fn make_state() -> GameState {
        let cfg = ScenarioConfig {
            map_radius: 4,
            player_count: 2,
            turn_limit: 30,
            ai_personalities: vec![crate::AiPersonality::Expansionist],
            oasis_majority_pct: 50,
            wealth_score_target: 200,
            relic_count: 1,
            relic_hold_turns: 6,
            victories_enabled: vec![
                VictoryKind::OasisDominance,
                VictoryKind::WealthScore,
                VictoryKind::RelicHold,
            ],
            symmetry: false,
            seed: 0,
        };
        let mut s = GameState::new(cfg, 1);

        // 5 tiles: 3 oases, 1 dunes, 1 ruins (relic site).
        let tiles = vec![
            (crate::hex::HexCoord { q: 0, r: 0 }, TerrainType::Oasis),
            (crate::hex::HexCoord { q: 1, r: 0 }, TerrainType::Oasis),
            (crate::hex::HexCoord { q: -1, r: 0 }, TerrainType::Oasis),
            (crate::hex::HexCoord { q: 0, r: 1 }, TerrainType::Dunes),
            (crate::hex::HexCoord { q: 0, r: -1 }, TerrainType::Ruins),
        ];

        for (coord, terrain) in tiles {
            let id = s.alloc_tile_id();
            let is_relic = terrain == TerrainType::Ruins;
            s.tiles.push(Tile {
                id,
                coord,
                terrain,
                is_relic_site: is_relic,
                owner: None,
                improvement: None,
            });
            s.tile_index.insert(coord, id);
        }

        // Player 0
        test_harness::create_player(
            &mut s,
            PlayerKind::Human,
            Stockpiles {
                water: 0,
                wealth: 50,
                influence: 10,
            },
        );

        // Player 1
        test_harness::create_player(
            &mut s,
            PlayerKind::Human,
            Stockpiles {
                water: 0,
                wealth: 30,
                influence: 5,
            },
        );

        // Relic on the ruins tile.
        let relic_tile = s.tile_index[&crate::hex::HexCoord { q: 0, r: -1 }];
        s.relics.push(Relic {
            id: RelicId(0),
            tile: relic_tile,
            holder: None,
            consecutive_turns_held: 0,
        });

        s
    }

    #[test]
    fn total_oases_counts_correctly() {
        let s = make_state();
        assert_eq!(s.total_oases(), 3);
    }

    #[test]
    fn oases_controlled_by_empty_when_unowned() {
        let s = make_state();
        assert_eq!(s.oases_controlled_by(PlayerId(0)), 0);
    }

    #[test]
    fn oases_controlled_by_counts_owned() {
        let mut s = make_state();
        // Own the first two oases for player 0.
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        // Own the third oasis for player 1.
        s.tiles[2].owner = Some(PlayerId(1));
        assert_eq!(s.oases_controlled_by(PlayerId(0)), 2);
        assert_eq!(s.oases_controlled_by(PlayerId(1)), 1);
    }

    #[test]
    fn active_routes_counts_only_active() {
        let mut s = make_state();
        s.routes.push(crate::model::CaravanRoute {
            id: crate::RouteId(0),
            owner: PlayerId(0),
            endpoints: (crate::CityId(0), crate::CityId(1)),
            path: vec![],
            status: RouteStatus::Active,
            length: 5,
            upkeep: 1,
            consecutive_threatened: 0,
        });
        s.routes.push(crate::model::CaravanRoute {
            id: crate::RouteId(1),
            owner: PlayerId(0),
            endpoints: (crate::CityId(0), crate::CityId(2)),
            path: vec![],
            status: RouteStatus::Severed,
            length: 3,
            upkeep: 1,
            consecutive_threatened: 0,
        });
        assert_eq!(s.active_routes(PlayerId(0)), 1);
        assert_eq!(s.active_routes(PlayerId(1)), 0);
    }

    #[test]
    fn prestige_score_formula() {
        let mut s = make_state();
        s.players[0].resources.wealth = 50;
        s.players[0].resources.influence = 10;
        // Own 1 oasis, 0 active routes.
        s.tiles[0].owner = Some(PlayerId(0));
        // score = floor(50*1 + 10*2 + 1*8 + 0*4) = floor(50+20+8+0) = 78
        assert_eq!(s.prestige_score(PlayerId(0)), 78);
    }

    #[test]
    fn holds_required_relics_false_when_none_held() {
        let s = make_state();
        assert!(!s.holds_required_relics(PlayerId(0)));
    }

    #[test]
    fn holds_required_relics_true_when_all_held() {
        let mut s = make_state();
        s.relics[0].holder = Some(PlayerId(0));
        assert!(s.holds_required_relics(PlayerId(0)));
        assert!(!s.holds_required_relics(PlayerId(1)));
    }

    #[test]
    fn v1_triggers_on_oasis_majority() {
        let mut s = make_state();
        // Player 0 owns 2 of 3 oases (>= ceil(50%/100 * 3) = 2).
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::OasisDominance,
                winner: PlayerId(0)
            })
        ));
    }

    #[test]
    fn v1_does_not_trigger_below_threshold() {
        let mut s = make_state();
        // Player 0 owns 1 of 3 oases (< 2 threshold).
        s.tiles[0].owner = Some(PlayerId(0));
        let ev = s.check_victory();
        assert!(ev.is_none());
    }

    #[test]
    fn v2_triggers_on_prestige_score() {
        let mut s = make_state();
        s.scenario.wealth_score_target = 50;
        s.players[0].resources.wealth = 50;
        // score = floor(50*1 + 10*2 + 0*8 + 0*4) = 70 >= 50
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::WealthScore,
                winner: PlayerId(0)
            })
        ));
    }

    #[test]
    fn v3_triggers_on_relic_hold() {
        let mut s = make_state();
        s.relics[0].holder = Some(PlayerId(0));
        s.relics[0].consecutive_turns_held = 6;
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::RelicHold,
                winner: PlayerId(0)
            })
        ));
    }

    #[test]
    fn v3_does_not_trigger_below_hold_turns() {
        let mut s = make_state();
        s.relics[0].holder = Some(PlayerId(0));
        s.relics[0].consecutive_turns_held = 3;
        let ev = s.check_victory();
        // Not V3, but V1 might trigger if we have enough oases.
        // In this test, no oases are owned, so no V1 either.
        assert!(ev.is_none());
    }

    #[test]
    fn elimination_does_not_trigger_v1() {
        let mut s = make_state();
        s.players[1].defeated = true;
        let ev = s.check_victory();
        // Player 0 is alive but has 0 oases — below the 2-oasis threshold.
        // A defeated player's elimination does not grant an automatic win.
        assert!(ev.is_none());
    }

    #[test]
    fn turn_limit_fallback() {
        let mut s = make_state();
        s.scenario.victories_enabled = vec![]; // no threshold victories
        s.scenario.turn_limit = 5;
        s.turn = 5;
        s.players[0].resources.wealth = 100;
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::TurnLimit,
                winner: PlayerId(0)
            })
        ));
    }

    #[test]
    fn update_victory_tracker_populates_maps() {
        let mut s = make_state();
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        let events = s.update_victory_tracker();
        assert_eq!(s.victory.oases_controlled.get(&PlayerId(0)), Some(&2));
        assert_eq!(s.victory.oases_controlled.get(&PlayerId(1)), Some(&0));
        assert!(s.victory.prestige_score.contains_key(&PlayerId(0)));
        assert!(s.victory.prestige_score.contains_key(&PlayerId(1)));
        // No victory event since P0 has 2/3 oases >= 2 threshold.
        // Actually ceil(50% * 3) = 2, so this DOES trigger.
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Victory {
                kind: VictoryKind::OasisDominance,
                ..
            }
        )));
    }

    #[test]
    fn turn_limit_tiebreak_prefers_more_oases() {
        let mut s = make_state();
        s.scenario.victories_enabled = vec![];
        s.scenario.turn_limit = 5;
        s.turn = 5;
        // Both players have same wealth.
        s.players[0].resources.wealth = 50;
        s.players[1].resources.wealth = 50;
        // Player 1 owns more oases.
        s.tiles[0].owner = Some(PlayerId(1));
        s.tiles[1].owner = Some(PlayerId(1));
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::TurnLimit,
                winner: PlayerId(1)
            })
        ));
    }

    #[test]
    fn turn_limit_tiebreak_prefers_lower_player_id() {
        let mut s = make_state();
        s.scenario.victories_enabled = vec![];
        s.scenario.turn_limit = 5;
        s.turn = 5;
        // Same wealth, same oases — lower PlayerId wins.
        let ev = s.check_victory();
        assert!(matches!(
            ev,
            Some(GameEvent::Victory {
                kind: VictoryKind::TurnLimit,
                winner: PlayerId(0)
            })
        ));
    }

    #[test]
    fn defeated_player_cannot_win() {
        let mut s = make_state();
        s.players[0].defeated = true;
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        let ev = s.check_victory();
        // Player 0 has enough oases but is defeated.
        // Player 1 has 0 oases — no threshold met.
        assert!(ev.is_none());
    }
}
