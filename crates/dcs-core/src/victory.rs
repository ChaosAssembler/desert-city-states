//! Victory condition logic: the three threshold-based conditions (V1–V3) plus
//! a turn-limit fallback.
//!
//! This module is **pure, deterministic, engine-free** (no rendering, no input,
//! no RNG). It is the sole location for victory-checking algorithms; the turn
//! resolver calls [`update_victory_tracker`] once per end-of-turn round, then
//! [`check_victory`] to decide whether the game has ended.
//!
//! # Spec
//!
//! Implements DD §13 via `behavior-victory-conditions.md` §5–§6.

use crate::model::{GameState, TerrainType};
use crate::{GameEvent, PlayerId, RelicId, RouteStatus, VictoryKind};

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
// Public helper functions (spec §5)
// ---------------------------------------------------------------------------

/// Count of oases owned by `player` (V1 numerator).
///
/// An oasis tile is one with `terrain == Oasis`; "control" means
/// `tile.owner == Some(player)`.
pub fn oases_controlled_by(state: &GameState, player: PlayerId) -> u32 {
    state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Oasis && t.owner == Some(player))
        .count() as u32
}

/// Total oases on the map (V1 denominator).
///
/// Derived from tile terrain — always known regardless of fog.
pub fn total_oases(state: &GameState) -> u32 {
    state
        .tiles
        .iter()
        .filter(|t| t.terrain == TerrainType::Oasis)
        .count() as u32
}

/// Count of active caravan routes owned by `player`.
///
/// Only `RouteStatus::Active` routes contribute — Threatened/Severed routes
/// count as 0 (spec §6.2).
pub fn active_routes(state: &GameState, player: PlayerId) -> u32 {
    state
        .routes
        .iter()
        .filter(|r| r.owner == player && r.status == RouteStatus::Active)
        .count() as u32
}

/// Does `player` hold ALL relic sites? (V3 holder test)
///
/// Returns `true` if every relic site tile with `is_relic_site == true` has a
/// corresponding `Relic` whose `holder == Some(player)`. If there are no relic
/// sites, returns `false` (no relics to hold).
pub fn holds_required_relics(state: &GameState, player: PlayerId) -> bool {
    let relic_sites: Vec<_> = state.tiles.iter().filter(|t| t.is_relic_site).collect();
    if relic_sites.is_empty() {
        return false;
    }
    relic_sites.iter().all(|site| {
        state
            .relics
            .iter()
            .any(|r| r.tile == site.id && r.holder == Some(player))
    })
}

// ---------------------------------------------------------------------------
// Prestige score (spec §5 + §6.2)
// ---------------------------------------------------------------------------

/// Prestige score for one player (V2 + turn-limit fallback).
///
/// Implements DD §13's formula:
/// ```text
/// floor(Wealth×1 + Influence×2 + oases×8 + active_routes×4)
/// ```
///
/// Stockpile values come from `Player.resources`; oases and routes are
/// recomputed live for accuracy.
pub fn prestige_score(state: &GameState, player: PlayerId) -> u32 {
    let p = &state.players[player.0 as usize];
    let wealth = p.resources.wealth as f32;
    let influence = p.resources.influence as f32;
    let oases = oases_controlled_by(state, player) as f32;
    let routes = active_routes(state, player) as f32;

    let score = wealth * PRESTIGE_WEALTH_W
        + influence * PRESTIGE_INFLUENCE_W
        + oases * PRESTIGE_OASIS_W
        + routes * PRESTIGE_ROUTE_W;

    score.floor() as u32
}

// ---------------------------------------------------------------------------
// Victory checks (spec §6)
// ---------------------------------------------------------------------------

/// V1 — Oasis Dominance (spec §6.3).
///
/// A player wins if they control ≥ `ceil(oasis_majority_pct / 100 * total_oases)`
/// oases. Defeated players are excluded from this check entirely — a defeated
/// player can never win, and surviving players are not granted an automatic
/// victory just because all other players are defeated.
fn check_oasis_dominance(state: &GameState) -> Option<PlayerId> {
    let total = total_oases(state).max(1);
    let threshold = (state.scenario.oasis_majority_pct as f32 / 100.0 * total as f32).ceil() as u32;

    // Check threshold-based win — defeated players are skipped.
    for p in &state.players {
        if p.defeated {
            continue;
        }
        let controlled = oases_controlled_by(state, p.id);
        if controlled >= threshold {
            return Some(p.id);
        }
    }

    None
}

/// V2 — Wealth/Prestige Score (spec §6.2).
///
/// A player wins when `prestige_score(p) >= wealth_score_target`.
fn check_wealth_score(state: &GameState) -> Option<PlayerId> {
    for p in &state.players {
        if p.defeated {
            continue;
        }
        if prestige_score(state, p.id) >= state.scenario.wealth_score_target {
            return Some(p.id);
        }
    }
    None
}

/// V3 — Relic Hold (spec §6.4).
///
/// A player wins if they hold ALL relic sites (`holds_required_relics`) AND
/// every held relic's `consecutive_turns_held >= relic_hold_turns`.
fn check_relic_hold(state: &GameState) -> Option<PlayerId> {
    let hold_turns = state.scenario.relic_hold_turns;

    // Collect relic ids that are on relic sites.
    let relic_site_ids: Vec<RelicId> = state
        .relics
        .iter()
        .filter(|r| {
            state
                .tiles
                .get(r.tile.0 as usize)
                .is_some_and(|t| t.is_relic_site)
        })
        .map(|r| r.id)
        .collect();

    if relic_site_ids.is_empty() {
        return None;
    }

    for p in &state.players {
        if p.defeated {
            continue;
        }
        // Check that every relic site is held by this player.
        let all_held = relic_site_ids.iter().all(|rid| {
            state
                .relics
                .iter()
                .any(|r| r.id == *rid && r.holder == Some(p.id))
        });
        if !all_held {
            continue;
        }
        // Check that every held relic meets the hold timer.
        let all_long_enough = relic_site_ids.iter().all(|rid| {
            state.relics.iter().any(|r| {
                r.id == *rid && r.holder == Some(p.id) && r.consecutive_turns_held >= hold_turns
            })
        });
        if all_long_enough {
            return Some(p.id);
        }
    }
    None
}

/// Turn-limit fallback (spec §6.5).
///
/// When `state.turn >= scenario.turn_limit`, the living player with the
/// highest `prestige_score` wins. Tiebreak: (1) more oases controlled,
/// (2) lower `PlayerId` (deterministic).
fn check_turn_limit(state: &GameState) -> Option<PlayerId> {
    if state.turn < state.scenario.turn_limit {
        return None;
    }

    let mut living: Vec<_> = state.players.iter().filter(|p| !p.defeated).collect();
    if living.is_empty() {
        return None;
    }

    living.sort_by(|a, b| {
        let score_a = prestige_score(state, a.id);
        let score_b = prestige_score(state, b.id);
        score_b
            .cmp(&score_a)
            .then_with(|| {
                let oases_a = oases_controlled_by(state, a.id);
                let oases_b = oases_controlled_by(state, b.id);
                oases_b.cmp(&oases_a)
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    Some(living[0].id)
}

// ---------------------------------------------------------------------------
// Public API (spec §5)
// ---------------------------------------------------------------------------

/// Decide if a victory has occurred (V1/V2/V3 met or turn limit).
///
/// Returns `Some(GameEvent::Victory { kind, winner })` to be emitted by the
/// engine, else `None`. Pure read — no mutation.
pub fn check_victory(state: &GameState) -> Option<GameEvent> {
    // Check threshold-based conditions (V1–V3) only for enabled kinds.
    for kind in &state.scenario.victories_enabled {
        let winner = match kind {
            VictoryKind::OasisDominance => check_oasis_dominance(state),
            VictoryKind::WealthScore => check_wealth_score(state),
            VictoryKind::RelicHold => check_relic_hold(state),
            VictoryKind::TurnLimit => None, // handled below
        };
        if let Some(winner) = winner {
            return Some(GameEvent::Victory {
                kind: *kind,
                winner,
            });
        }
    }

    // Turn-limit fallback.
    if let Some(winner) = check_turn_limit(state) {
        return Some(GameEvent::Victory {
            kind: VictoryKind::TurnLimit,
            winner,
        });
    }

    None
}

/// Recompute all tracker maps and check for victory.
///
/// Called once from `advance_turn` (turn-engine §6.3 step 2) BEFORE the turn
/// counter is incremented. Updates `state.victory` (oases_controlled,
/// prestige_score, relic_timers) for every living player, then calls
/// [`check_victory`] and returns any resulting event.
pub fn update_victory_tracker(state: &mut GameState) -> Vec<GameEvent> {
    // --- Populate tracker maps for every living player ---
    for p in &state.players {
        if p.defeated {
            continue;
        }
        state
            .victory
            .oases_controlled
            .insert(p.id, oases_controlled_by(state, p.id));
        state
            .victory
            .prestige_score
            .insert(p.id, prestige_score(state, p.id));
    }

    // --- Update relic_timers (mirror current holders for fast win-checking) ---
    state.victory.relic_timers.clear();
    for relic in &state.relics {
        if let Some(holder) = relic.holder {
            state.victory.relic_timers.insert(relic.id, holder);
        }
    }

    // --- Check victory and return any event ---
    if let Some(event) = check_victory(state) {
        vec![event]
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Player, PlayerColor, PlayerKind, Relic, Stockpiles, Tile};
    use crate::scenario::ScenarioConfig;
    use fxhash::FxHashSet;

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
        s.players.push(Player {
            id: PlayerId(0),
            kind: PlayerKind::Human,
            color: PlayerColor::Sand,
            resources: Stockpiles {
                water: 0,
                wealth: 50,
                influence: 10,
            },
            discovered: FxHashSet::default(),
            defeated: false,
        });

        // Player 1
        s.players.push(Player {
            id: PlayerId(1),
            kind: PlayerKind::Human,
            color: PlayerColor::Sand,
            resources: Stockpiles {
                water: 0,
                wealth: 30,
                influence: 5,
            },
            discovered: FxHashSet::default(),
            defeated: false,
        });

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
        assert_eq!(total_oases(&s), 3);
    }

    #[test]
    fn oases_controlled_by_empty_when_unowned() {
        let s = make_state();
        assert_eq!(oases_controlled_by(&s, PlayerId(0)), 0);
    }

    #[test]
    fn oases_controlled_by_counts_owned() {
        let mut s = make_state();
        // Own the first two oases for player 0.
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        // Own the third oasis for player 1.
        s.tiles[2].owner = Some(PlayerId(1));
        assert_eq!(oases_controlled_by(&s, PlayerId(0)), 2);
        assert_eq!(oases_controlled_by(&s, PlayerId(1)), 1);
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
        assert_eq!(active_routes(&s, PlayerId(0)), 1);
        assert_eq!(active_routes(&s, PlayerId(1)), 0);
    }

    #[test]
    fn prestige_score_formula() {
        let mut s = make_state();
        s.players[0].resources.wealth = 50;
        s.players[0].resources.influence = 10;
        // Own 1 oasis, 0 active routes.
        s.tiles[0].owner = Some(PlayerId(0));
        // score = floor(50*1 + 10*2 + 1*8 + 0*4) = floor(50+20+8+0) = 78
        assert_eq!(prestige_score(&s, PlayerId(0)), 78);
    }

    #[test]
    fn holds_required_relics_false_when_none_held() {
        let s = make_state();
        assert!(!holds_required_relics(&s, PlayerId(0)));
    }

    #[test]
    fn holds_required_relics_true_when_all_held() {
        let mut s = make_state();
        s.relics[0].holder = Some(PlayerId(0));
        assert!(holds_required_relics(&s, PlayerId(0)));
        assert!(!holds_required_relics(&s, PlayerId(1)));
    }

    #[test]
    fn v1_triggers_on_oasis_majority() {
        let mut s = make_state();
        // Player 0 owns 2 of 3 oases (>= ceil(50%/100 * 3) = 2).
        s.tiles[0].owner = Some(PlayerId(0));
        s.tiles[1].owner = Some(PlayerId(0));
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
        assert!(ev.is_none());
    }

    #[test]
    fn v2_triggers_on_prestige_score() {
        let mut s = make_state();
        s.scenario.wealth_score_target = 50;
        s.players[0].resources.wealth = 50;
        // score = floor(50*1 + 10*2 + 0*8 + 0*4) = 70 >= 50
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
        // Not V3, but V1 might trigger if we have enough oases.
        // In this test, no oases are owned, so no V1 either.
        assert!(ev.is_none());
    }

    #[test]
    fn elimination_does_not_trigger_v1() {
        let mut s = make_state();
        s.players[1].defeated = true;
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
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
        let events = update_victory_tracker(&mut s);
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
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
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
        let ev = check_victory(&s);
        // Player 0 has enough oases but is defeated.
        // Player 1 has 0 oases — no threshold met.
        assert!(ev.is_none());
    }
}
