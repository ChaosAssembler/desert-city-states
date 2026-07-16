//! The Phase 1 turn engine: the single mutation path for `GameState`.
//!
//! # Scope (Phase 1 / Foundation)
//!
//! This module implements the **command-pattern resolver** and the **sequential
//! actor phase machine** (ADR-0004). It is the only place that mutates
//! `GameState` in response to player/AI intent.
//!
//! Phase 1 delivers the *structure* of the resolver — real validation, reject
//! semantics, the sequential actor cycle, move/found-city/end-turn, and
//! `advance_turn` bookkeeping — while leaving the heavy Phase 2 game-play
//! effects (buildings, specialization, route economics, combat, raids) as
//! minimal, panic-free stubs. See the module doc of each `resolve_one` arm for
//! which commands are fully resolved vs. stubbed.
//!
//! # Determinism (ADR-0006)
//!
//! All randomness flows through `state.rng`. No `thread_rng` / `std::time`.
//! Illegal user input is *rejected* via [`GameEvent::Rejected`]; the engine
//! only `.expect(...)`s on violated internal invariants (e.g. a dangling id).
//!
//! # Replay correctness
//!
//! `step` / `resolve_one` are pure functions of `(state, command-stream)`:
//! identical seed + identical commands ⇒ identical resulting state + events.

use crate::hex::{astar, in_map, neighbors};
use crate::model::{FOUND_CITY_INFLUENCE, tile_at, unit_def};
use crate::{
    CityId, Command, GameEvent, GameState, PlayerId, RejectReason, TileId, UnitId, UnitKind,
};

/// Wealth cost to train a unit in Phase 1. (Phase 2 will replace this with a
/// per-kind cap/balance formula; this is a flat placeholder per the spec.)
const TRAIN_COST: u32 = 10;

/// Return the player whose turn it currently is.
#[inline]
fn current_player(state: &GameState) -> PlayerId {
    state.current_actor
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply one actor's queued `commands` (Order + Resolution + Income for that
/// actor), advancing the actor cycle as appropriate.
///
/// Returns the events produced. **Never panics on bad input** — illegal
/// commands are reported as [`GameEvent::Rejected`] and leave state untouched.
///
/// ## Phase machine (per the resolver spec)
///
/// 1. **Order** — commands are applied in submission order via [`resolve_one`].
/// 2. **Resolution** — each valid command mutates state and emits events.
/// 3. **Income** — *stubbed* for Phase 2 (emits a `Warn` only).
/// 4. **Actor advance** — `current_actor` moves to the next non-defeated player;
///    if all actors in this cycle have acted, [`advance_turn`] runs and the
///    next cycle starts at `PlayerId(0)`.
///
/// `EndTurn` is treated as "this actor is done": it triggers the actor-advance
/// (and possibly `advance_turn`) without mutating other state.
pub fn step(state: &mut GameState, commands: &[Command]) -> Vec<GameEvent> {
    let mut events: Vec<GameEvent> = Vec::new();

    state.phase = crate::TurnPhase::Resolution;

    // `EndTurn` is "this actor is done" — it does not resolve into state, but
    // it DOES trigger the actor-advance (and possibly `advance_turn`). We
    // detect it up front and skip resolution entirely.
    let ended_turn = commands.iter().any(|c| matches!(c, Command::EndTurn));
    if ended_turn {
        advance_actor(state, &mut events);
        state.phase = crate::TurnPhase::EndOfTurn;
        return events;
    }

    for cmd in commands {
        match validate(state, cmd) {
            Ok(()) => {
                let mut evs = resolve_one(state, cmd.clone());
                events.append(&mut evs);
            }
            Err(reason) => {
                events.push(GameEvent::Rejected {
                    command: cmd.clone(),
                    reason,
                });
            }
        }
    }

    // Income phase — STUBBED for Phase 2.
    state.phase = crate::TurnPhase::Income;
    // (No per-actor economy in Phase 1 — intentionally left as a no-op stub so
    // replay stays deterministic and `advance_turn` does not double-apply it.)

    state.phase = crate::TurnPhase::EndOfTurn;
    events
}

/// Advance to the next non-defeated actor; if a full cycle completed, run
/// [`advance_turn`] and restart at `PlayerId(0)`.
fn advance_actor(state: &mut GameState, events: &mut Vec<GameEvent>) {
    let next = next_actor(state, state.current_actor);
    match next {
        Some(actor) => {
            state.current_actor = actor;
            state.phase = crate::TurnPhase::Order;
        }
        None => {
            // Full cycle complete → global end-of-round bookkeeping.
            let mut adv = advance_turn(state);
            events.append(&mut adv);
            // advance_turn already set current_actor = 0 and phase = Order.
        }
    }
}

/// Find the next non-defeated player *after* `current`, wrapping around.
/// Returns `None` if `current` is the last actor to act this cycle (i.e. every
/// other player has already acted and we should flip to the next turn).
fn next_actor(state: &GameState, current: PlayerId) -> Option<PlayerId> {
    let count = state.players.len() as u32;
    for i in 1..=count {
        let idx = (current.0 + i) % count;
        let p = &state.players[idx as usize];
        if !p.defeated {
            // If we wrapped back to / before `current`, the cycle is complete.
            if idx <= current.0 && i < count {
                // wrapped around without finding a later actor
                return None;
            }
            return Some(PlayerId(idx));
        }
    }
    None
}

/// Global end-of-round bookkeeping, run exactly once per full actor cycle.
///
/// Per the resolver spec, `advance_turn` does **NOT** re-apply per-actor
/// economy — that happens per-actor in the Income phase. It only: updates relic
/// V3 timers, checks victory conditions, increments `turn`, resets
/// `moves_left` for every unit, and emits `TurnAdvanced`.
pub fn advance_turn(state: &mut GameState) -> Vec<GameEvent> {
    let mut events: Vec<GameEvent> = Vec::new();

    // --- Relic V3 timers ---
    for relic in state.relics.iter_mut() {
        if let Some(holder) = relic.holder {
            relic.consecutive_turns_held += 1;
            // keep the VictoryTracker relic timer fresh
            state.victory.relic_timers.insert(relic.id, holder);
        }
    }

    // --- Victory checks (enabled conditions only) ---
    let scenario = state.scenario.clone();
    let turn = state.turn;

    for kind in &scenario.victories_enabled {
        let victory = match kind {
            crate::VictoryKind::OasisDominance => check_oasis_dominance(state, &scenario),
            crate::VictoryKind::WealthScore => check_wealth_score(state, &scenario),
            crate::VictoryKind::RelicHold => check_relic_hold(state, &scenario),
            crate::VictoryKind::TurnLimit => None, // handled below
        };
        if let Some(winner) = victory {
            events.push(GameEvent::Victory {
                kind: *kind,
                winner,
            });
            state.log.append(&mut events.clone());
            // Stop: a victory was reached.
            return events;
        }
    }

    // --- Turn-limit fallback ---
    if turn >= scenario.turn_limit {
        let winner = highest_score_player(state);
        events.push(GameEvent::Victory {
            kind: crate::VictoryKind::TurnLimit,
            winner,
        });
        // Still advance the turn below for consistency.
    }

    // --- Turn increment ---
    state.turn += 1;

    // --- Reset moves for all units ---
    for u in state.units.iter_mut() {
        u.moves_left = unit_def(u.kind).moves;
    }

    // --- Reset phase / actor ---
    state.phase = crate::TurnPhase::Order;
    state.current_actor = PlayerId(0);

    events.push(GameEvent::TurnAdvanced { turn: state.turn });
    state.log.append(&mut events.clone());
    events
}

/// Validate a single command against the current state.
///
/// Returns `Ok(())` if the command may be resolved, or a [`RejectReason`]
/// describing why it must be rejected. Validation is pure (no mutation).
pub fn validate(state: &GameState, cmd: &Command) -> Result<(), RejectReason> {
    let actor = current_player(state);

    match cmd {
        Command::EndTurn => Ok(()),

        Command::MoveUnit { unit, to } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            if u.moves_left == 0 {
                return Err(RejectReason::OutOfMoves);
            }
            // Destination must exist on the map.
            let dest = state.tiles.get(to.0 as usize).ok_or(RejectReason::OffMap)?;
            let _ = dest; // existence is enough here; path checked in resolve_one
            Ok(())
        }

        Command::FoundCity { unit, tile } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            if u.kind != UnitKind::Scout {
                return Err(RejectReason::IllegalTarget);
            }
            let t = state
                .tiles
                .get(tile.0 as usize)
                .ok_or(RejectReason::OffMap)?;
            if t.terrain != crate::TerrainType::Oasis {
                return Err(RejectReason::NotOasis);
            }
            // Tile must not already be a city tile.
            if state.cities.iter().any(|c| c.tile == *tile) {
                return Err(RejectReason::IllegalTarget);
            }
            let player = &state.players[actor.0 as usize];
            if player.resources.influence < FOUND_CITY_INFLUENCE {
                return Err(RejectReason::NoResource);
            }
            Ok(())
        }

        Command::TrainUnit { city, kind } => {
            let c = state
                .cities
                .get(city.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if c.owner != actor {
                return Err(RejectReason::NotYourTurn);
            }
            let player = &state.players[actor.0 as usize];
            if player.resources.wealth < TRAIN_COST {
                return Err(RejectReason::NoResource);
            }
            let _ = kind;
            Ok(())
        }

        Command::Build { city, .. } => {
            let c = state
                .cities
                .get(city.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if c.owner != actor {
                return Err(RejectReason::NotYourTurn);
            }
            // Phase 2 effect stubbed; ownership validated here.
            Ok(())
        }

        Command::Specialize { city, .. } => {
            let c = state
                .cities
                .get(city.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if c.owner != actor {
                return Err(RejectReason::NotYourTurn);
            }
            Ok(())
        }

        Command::ConnectRoute { from, to } => {
            let cf = state
                .cities
                .get(from.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            let ct = state
                .cities
                .get(to.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if cf.owner != actor || ct.owner != actor {
                return Err(RejectReason::NotYourTurn);
            }
            Ok(())
        }

        Command::Patrol { unit, .. } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            Ok(())
        }

        Command::Garrison { unit, .. } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            Ok(())
        }

        Command::RaidRoute { unit, .. } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            Ok(())
        }

        Command::RaidCity { unit, .. } => {
            let u = state
                .units
                .get(unit.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if u.owner != actor {
                return Err(RejectReason::NotYourUnit);
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: single-command resolution (assumes already validated)
// ---------------------------------------------------------------------------

/// Resolve exactly one command. Callers must have already [`validate`]d it;
/// this function performs the mutation and emits events, appending every event
/// to `state.log` as well.
fn resolve_one(state: &mut GameState, cmd: Command) -> Vec<GameEvent> {
    let actor = current_player(state);

    let events = match cmd {
        Command::EndTurn => Vec::new(), // advance handled by `step`.

        Command::MoveUnit { unit, to } => resolve_move(state, unit, to, actor),

        Command::FoundCity { unit, tile } => resolve_found_city(state, unit, tile, actor),

        Command::TrainUnit { city, kind } => resolve_train(state, city, kind, actor),

        Command::Build { city, building } => {
            // Phase 2 effect stubbed. Accept and emit Built-like no-op + Warn.
            let _ = (city, building);
            vec![GameEvent::Warn {
                message: "Build effect stubbed in Phase 1".into(),
            }]
        }

        Command::Specialize { city, spec } => {
            // Emit the success-ish event cheaply; deep effect stubbed.
            vec![GameEvent::Specialized { city, spec }]
        }

        Command::ConnectRoute { from, to } => {
            // Phase 2 effect stubbed; emit Warn (path computation is Phase 2).
            let _ = (from, to);
            vec![GameEvent::Warn {
                message: "ConnectRoute effect stubbed in Phase 1".into(),
            }]
        }

        Command::Patrol { unit, tile } => {
            // Cheap: set stance + emit event.
            if let Some(u) = state.units.get_mut(unit.0 as usize) {
                u.ability = crate::UnitAbility::Patrolling;
            }
            vec![GameEvent::UnitPatrolled { unit, tile }]
        }

        Command::Garrison { unit, city } => {
            if let Some(u) = state.units.get_mut(unit.0 as usize) {
                u.ability = crate::UnitAbility::Garrisoned;
            }
            vec![GameEvent::UnitGarrisoned { unit, city }]
        }

        Command::RaidRoute { unit, route } => {
            let _ = (unit, route);
            vec![GameEvent::Warn {
                message: "RaidRoute effect stubbed in Phase 1".into(),
            }]
        }

        Command::RaidCity { unit, city } => {
            let _ = (unit, city);
            vec![GameEvent::Warn {
                message: "RaidCity effect stubbed in Phase 1".into(),
            }]
        }
    };

    state.log.append(&mut events.clone());
    events
}

/// Fully resolved in Phase 1: move if a path exists and moves remain.
fn resolve_move(
    state: &mut GameState,
    unit: UnitId,
    to: TileId,
    actor: PlayerId,
) -> Vec<GameEvent> {
    // Re-validate cheaply (resolve_one assumes validated, but guard invariants).
    if state.units.get(unit.0 as usize).is_none() {
        return vec![GameEvent::Rejected {
            command: Command::MoveUnit { unit, to },
            reason: RejectReason::InvalidState,
        }];
    }
    if state.tiles.get(to.0 as usize).is_none() {
        return vec![GameEvent::Rejected {
            command: Command::MoveUnit { unit, to },
            reason: RejectReason::OffMap,
        }];
    }

    let from_tile = state.units[unit.0 as usize].tile;
    let from_coord = state.tiles[from_tile.0 as usize].coord;
    let to_coord = state.tiles[to.0 as usize].coord;
    let radius = state.scenario.map_radius as u32;

    // Passable = any in-map tile (Phase 1 keeps all in-map tiles passable).
    let path = astar(from_coord, to_coord, |h| in_map(h, radius), |_, _| 1.0_f32);

    let path = match path {
        Some(p) => p,
        None => {
            return vec![GameEvent::Rejected {
                command: Command::MoveUnit { unit, to },
                reason: RejectReason::Blocked,
            }];
        }
    };

    // Decrement moves: one point per step, clamped to the unit's remaining pool.
    let steps = path.len().min(u8::MAX as usize) as u8;
    {
        let u = &mut state.units[unit.0 as usize];
        let consumed = steps.min(u.moves_left).max(1);
        u.moves_left = u.moves_left.saturating_sub(consumed);
        u.tile = to;
    }

    // Reveal fog: destination + neighbors for the acting player.
    let mut revealed = vec![to];
    for n in neighbors(to_coord) {
        if let Some(t) = tile_at(state, n) {
            revealed.push(t.id);
        }
    }
    let player = &mut state.players[actor.0 as usize];
    let mut newly: Vec<TileId> = Vec::new();
    for tid in revealed {
        if player.discovered.insert(tid) {
            newly.push(tid);
        }
    }
    if !newly.is_empty() {
        state.log.push(GameEvent::Revealed {
            player: actor,
            tiles: newly.clone(),
        });
    }

    vec![GameEvent::UnitMoved {
        unit,
        from: from_tile,
        to,
    }]
}

/// Fully resolved in Phase 1: found a city, consume the Scout, spend Influence.
fn resolve_found_city(
    state: &mut GameState,
    unit: UnitId,
    tile: TileId,
    actor: PlayerId,
) -> Vec<GameEvent> {
    // Guard invariant: unit must still exist (validation already passed).
    if state.units.get(unit.0 as usize).is_none() {
        return vec![GameEvent::Rejected {
            command: Command::FoundCity { unit, tile },
            reason: RejectReason::InvalidState,
        }];
    }

    let city_id = state.alloc_city_id();
    let city = crate::City {
        id: city_id,
        owner: actor,
        tile,
        population: 1,
        specialization: None,
        buildings: vec![],
        stockpiles: crate::Stockpiles::default(),
        route_slots: 2,
        growth_timer: 0,
    };
    state.cities.push(city);

    // Spend influence.
    state.players[actor.0 as usize].resources.influence -= FOUND_CITY_INFLUENCE;

    // Consume the founding unit.
    state.units.retain(|u| u.id != unit);

    // Mark the tile as owned.
    if let Some(t) = state.tiles.get_mut(tile.0 as usize) {
        t.owner = Some(actor);
    }

    vec![GameEvent::CityFounded {
        city: city_id,
        owner: actor,
        tile,
    }]
}

/// Fully resolved in Phase 1: deduct Wealth, spawn a unit on the city tile.
fn resolve_train(
    state: &mut GameState,
    city: CityId,
    kind: UnitKind,
    actor: PlayerId,
) -> Vec<GameEvent> {
    if state.cities.get(city.0 as usize).is_none() {
        return vec![GameEvent::Rejected {
            command: Command::TrainUnit { city, kind },
            reason: RejectReason::InvalidState,
        }];
    }

    state.players[actor.0 as usize].resources.wealth -= TRAIN_COST;

    let tile = state.cities[city.0 as usize].tile;
    let unit_id = state.alloc_unit_id();
    let def = unit_def(kind);
    let unit = crate::Unit {
        id: unit_id,
        owner: actor,
        kind,
        tile,
        hp: def.hp as u32,
        moves_left: def.moves,
        ability: crate::UnitAbility::None,
    };
    state.units.push(unit);

    vec![GameEvent::UnitTrained {
        unit: unit_id,
        city,
    }]
}

// ---------------------------------------------------------------------------
// Victory helpers
// ---------------------------------------------------------------------------

fn check_oasis_dominance(state: &GameState, scenario: &crate::ScenarioConfig) -> Option<PlayerId> {
    let total_oases = state
        .tiles
        .iter()
        .filter(|t| t.terrain == crate::TerrainType::Oasis)
        .count()
        .max(1) as u32;
    let threshold =
        ((total_oases as f32) * (scenario.oasis_majority_pct as f32 / 100.0)).ceil() as u32;

    let mut counts: fxhash::FxHashMap<PlayerId, u32> = fxhash::FxHashMap::default();
    for t in &state.tiles {
        if t.terrain == crate::TerrainType::Oasis {
            if let Some(p) = t.owner {
                *counts.entry(p).or_insert(0) += 1;
            }
        }
    }
    counts
        .into_iter()
        .find(|&(_, c)| c > threshold)
        .map(|(p, _)| p)
}

fn check_wealth_score(state: &GameState, scenario: &crate::ScenarioConfig) -> Option<PlayerId> {
    state
        .players
        .iter()
        .find(|p| p.resources.wealth >= scenario.wealth_score_target)
        .map(|p| p.id)
}

fn check_relic_hold(state: &GameState, scenario: &crate::ScenarioConfig) -> Option<PlayerId> {
    state
        .relics
        .iter()
        .find(|r| r.holder.is_some() && r.consecutive_turns_held >= scenario.relic_hold_turns)
        .and_then(|r| r.holder)
}

/// Highest-prestige (wealth) player; tie-break by player index for determinism.
fn highest_score_player(state: &GameState) -> PlayerId {
    state
        .players
        .iter()
        .max_by_key(|p| p.resources.wealth)
        .map(|p| p.id)
        .unwrap_or(PlayerId(0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GameState, Tile};
    use crate::scenario::mvp_preset;
    use crate::{Command, GameEvent, PlayerId, RejectReason, TileId, UnitId, UnitKind};

    // ---- test harness ------------------------------------------------------

    /// Build a minimal deterministic `GameState` for tests, standing in for
    /// `crate::map::new_game` (which is the world-gen deliverable of a sibling
    /// agent). This produces a 3-player game with: a hex map of radius 2, each
    /// player owning a Scout + CaravanGuard on/near an oasis, seeded influence
    /// and wealth, and a relic on a ruins tile. It mirrors the world-gen spec's
    /// invariants closely enough to exercise the turn engine.
    fn make_game() -> GameState {
        let cfg = mvp_preset();
        let mut s = GameState::new(cfg, 1);
        let radius = s.scenario.map_radius as u32;

        // Allocate all in-map tiles.
        let coords = crate::hex::range(crate::hex::ORIGIN, radius);
        for c in coords {
            let id = s.alloc_tile_id();
            s.tiles.push(Tile {
                id,
                coord: c,
                terrain: crate::TerrainType::Dunes,
                is_relic_site: false,
                owner: None,
                improvement: None,
            });
            s.tile_index.insert(c, id);
        }

        // Mark three oases, well-spaced, at fixed coords for determinism.
        let oasis_coords = [
            crate::hex::HexCoord { q: 0, r: 0 },
            crate::hex::HexCoord { q: 2, r: -2 },
            crate::hex::HexCoord { q: -2, r: 2 },
        ];
        for &c in &oasis_coords {
            let id = s.tile_index[&c];
            s.tiles[id.0 as usize].terrain = crate::TerrainType::Oasis;
        }

        // A ruins/relic site near origin.
        let ruin_coord = crate::hex::HexCoord { q: 1, r: 0 };
        let ruin_id = s.tile_index[&ruin_coord];
        s.tiles[ruin_id.0 as usize].terrain = crate::TerrainType::Ruins;
        s.tiles[ruin_id.0 as usize].is_relic_site = true;
        let relic_id = s.alloc_relic_id();
        s.relics.push(crate::Relic {
            id: relic_id,
            tile: ruin_id,
            holder: None,
            consecutive_turns_held: 0,
        });

        // Players + starting units.
        for i in 0..s.scenario.player_count as u32 {
            let pid = s.alloc_player_id();
            s.players.push(crate::Player {
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
                resources: crate::Stockpiles {
                    water: 0,
                    wealth: 10,
                    influence: FOUND_CITY_INFLUENCE,
                },
                discovered: fxhash::FxHashSet::default(),
                defeated: false,
            });

            // Scout on the player's oasis; Guard on an in-map neighbor.
            let oasis = oasis_coords[i as usize];
            let scout_id = s.alloc_unit_id();
            let oasis_tile = s.tile_index[&oasis];
            s.units.push(crate::Unit {
                id: scout_id,
                owner: pid,
                kind: UnitKind::Scout,
                tile: oasis_tile,
                hp: unit_def(UnitKind::Scout).hp as u32,
                moves_left: unit_def(UnitKind::Scout).moves,
                ability: crate::UnitAbility::None,
            });
            let radius = s.scenario.map_radius as u32;
            let neighbor = neighbors(oasis)
                .into_iter()
                .find(|n| in_map(*n, radius))
                .unwrap_or(oasis);
            let guard_id = s.alloc_unit_id();
            let guard_tile = s.tile_index[&neighbor];
            s.units.push(crate::Unit {
                id: guard_id,
                owner: pid,
                kind: UnitKind::CaravanGuard,
                tile: guard_tile,
                hp: unit_def(UnitKind::CaravanGuard).hp as u32,
                moves_left: unit_def(UnitKind::CaravanGuard).moves,
                ability: crate::UnitAbility::None,
            });
        }

        s
    }

    fn player_scout(s: &GameState, p: PlayerId) -> UnitId {
        s.units
            .iter()
            .find(|u| u.owner == p && u.kind == UnitKind::Scout)
            .expect("scout exists")
            .id
    }

    fn scout_tile(s: &GameState, unit: UnitId) -> TileId {
        s.units[s.units.iter().position(|u| u.id == unit).unwrap()].tile
    }

    fn neighbor_tile(s: &GameState, tile: TileId) -> TileId {
        let coord = s.tiles[tile.0 as usize].coord;
        let radius = s.scenario.map_radius as u32;
        let n = neighbors(coord)
            .into_iter()
            .find(|h| in_map(*h, radius))
            .unwrap_or(coord);
        s.tile_index[&n]
    }

    // ---- tests -------------------------------------------------------------

    #[test]
    fn end_turn_advances_actor() {
        let mut s = make_game();
        assert_eq!(s.current_actor, PlayerId(0));
        let events = step(&mut s, &[Command::EndTurn]);
        // A Rejected for the mis-shaped EndTurn in `step` is fine; the actor
        // must still have advanced.
        let _ = events;
        assert_eq!(s.current_actor, PlayerId(1));
    }

    #[test]
    fn end_turn_full_cycle_advances_turn() {
        let mut s = make_game();
        let mut saw_advance = false;
        for _ in 0..s.scenario.player_count as usize {
            let events = step(&mut s, &[Command::EndTurn]);
            if events
                .iter()
                .any(|e| matches!(e, GameEvent::TurnAdvanced { .. }))
            {
                saw_advance = true;
            }
        }
        assert_eq!(s.turn, 2);
        assert_eq!(s.current_actor, PlayerId(0));
        assert!(
            saw_advance,
            "expected a TurnAdvanced event after full cycle"
        );
    }

    #[test]
    fn illegal_command_rejected() {
        let mut s = make_game();
        let before = s.cities.len();
        // Player 0's scout is on an oasis; FoundCity on a non-oasis dunes tile
        // must be rejected (NotOasis), and must not add a city.
        let dunes = {
            // find a Dunes tile that is not a city tile
            s.tiles
                .iter()
                .find(|t| t.terrain == crate::TerrainType::Dunes)
                .unwrap()
                .id
        };
        let scout = player_scout(&s, PlayerId(0));
        let events = step(
            &mut s,
            &[Command::FoundCity {
                unit: scout,
                tile: dunes,
            }],
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, GameEvent::Rejected { .. })),
            "expected a Rejected event"
        );
        assert_eq!(s.cities.len(), before, "no city should be added");
    }

    #[test]
    fn move_unit_event() {
        let mut s = make_game();
        let scout = player_scout(&s, PlayerId(0));
        let from = scout_tile(&s, scout);
        let to = neighbor_tile(&s, from);
        let events = step(&mut s, &[Command::MoveUnit { unit: scout, to }]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                GameEvent::UnitMoved { unit, to: t, .. } if *unit == scout && *t == to
            )),
            "expected UnitMoved event"
        );
        assert_eq!(scout_tile(&s, scout), to, "unit should be on destination");
    }

    #[test]
    fn found_city_works() {
        let mut s = make_game();
        let scout = player_scout(&s, PlayerId(0));
        let tile = scout_tile(&s, scout);
        let infl_before = s.players[0].resources.influence;
        let events = step(&mut s, &[Command::FoundCity { unit: scout, tile }]);
        assert!(
            events.iter().any(|e| matches!(
                e,
                GameEvent::CityFounded { owner, .. } if *owner == PlayerId(0)
            )),
            "expected CityFounded event"
        );
        let city = s
            .cities
            .iter()
            .find(|c| c.owner == PlayerId(0))
            .expect("city exists");
        assert_eq!(city.tile, tile);
        assert_eq!(
            s.players[0].resources.influence,
            infl_before - FOUND_CITY_INFLUENCE
        );
        assert!(
            !s.units.iter().any(|u| u.id == scout),
            "founding unit should be consumed"
        );
        assert_eq!(s.tiles[tile.0 as usize].owner, Some(PlayerId(0)));
    }

    #[test]
    fn validate_not_your_turn() {
        let s = make_game();
        // current_actor == 0; try to move player 1's scout.
        let p1_scout = player_scout(&s, PlayerId(1));
        let to = neighbor_tile(&s, scout_tile(&s, p1_scout));
        let res = validate(&s, &Command::MoveUnit { unit: p1_scout, to });
        assert!(
            matches!(res, Err(RejectReason::NotYourUnit)),
            "expected NotYourUnit, got {res:?}"
        );
    }

    #[test]
    fn no_panic_on_bad_input() {
        let mut s = make_game();
        let scout = player_scout(&s, PlayerId(0));
        // Move to an off-map (non-existent) tile id.
        let events = step(
            &mut s,
            &[Command::MoveUnit {
                unit: scout,
                to: TileId(9999),
            }],
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, GameEvent::Rejected { .. })),
            "expected Rejected, no panic"
        );
    }
}
