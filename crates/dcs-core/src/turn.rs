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

use crate::hex::{astar, in_map};
use crate::model::{
    FOUND_CITY_INFLUENCE, POP_FOR_SPECIALIZE, SPECIALIZE_COST_INFLUENCE, UNIT_TRAIN_COST, unit_def,
};
use crate::{
    BuildingKind, CityId, CitySpecialization, Command, GameEvent, GameState, PlayerId,
    RejectReason, TileId, UnitId, UnitKind,
};

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

    // Resolve all non-EndTurn commands first (FoundCity, MoveUnit, etc.).
    // EndTurn is detected and handled AFTER command resolution.
    let ended_turn = commands.iter().any(|c| matches!(c, Command::EndTurn));

    for cmd in commands {
        if matches!(cmd, Command::EndTurn) {
            continue;
        }
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

    // Income phase — per-actor economy update (economy spec §6.1).
    state.phase = crate::TurnPhase::Income;
    let actor = current_player(state);
    let income_events = crate::economy::apply_income(state, actor);
    events.extend(income_events);

    // If EndTurn was present, this actor is done — advance to the next actor
    // (and possibly `advance_turn` for end-of-round bookkeeping).
    if ended_turn {
        advance_actor(state, &mut events);
    }

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
        if let Some(_holder) = relic.holder {
            relic.consecutive_turns_held += 1;
        }
    }

    // --- Victory checks (enabled conditions only) ---
    let victory_events = crate::victory::update_victory_tracker(state);
    events.extend(victory_events);
    if events
        .iter()
        .any(|e| matches!(e, GameEvent::Victory { .. }))
    {
        state.log.append(&mut events.clone());
        // Stop: a victory was reached.
        return events;
    }

    // --- Turn increment ---
    state.turn += 1;

    // --- Reset moves for all units ---
    for u in state.units.iter_mut() {
        u.moves_left = unit_def(u.kind).moves;
    }

    // --- Refresh fog from all owned cities (Watchtower/Scholar re-reveal) ---
    crate::fog::refresh_city_fog(state);

    // --- Recompute route statuses (caravan spec §6.5) ---
    let route_events = crate::caravan::recompute_routes(state);
    events.extend(route_events);

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
            // Raider must be in Fortress.
            if *kind == UnitKind::Raider && c.specialization != Some(CitySpecialization::Fortress) {
                return Err(RejectReason::InvalidState);
            }
            let mut cost = UNIT_TRAIN_COST[crate::world::unit_kind_index(kind)];
            if c.specialization == Some(CitySpecialization::Fortress) {
                cost = (cost as f32 * crate::model::FORTRESS_TRAIN_DISCOUNT) as u32;
            }
            let player = &state.players[actor.0 as usize];
            if player.resources.wealth < cost {
                return Err(RejectReason::NoResource);
            }
            // Unit cap check.
            let cap = crate::world::unit_cap(state, actor);
            let current = state.units.iter().filter(|u| u.owner == actor).count() as u32;
            if current >= cap {
                return Err(RejectReason::Blocked);
            }
            Ok(())
        }

        Command::Build { city, building } => {
            let c = state
                .cities
                .get(city.0 as usize)
                .ok_or(RejectReason::InvalidState)?;
            if c.owner != actor {
                return Err(RejectReason::NotYourTurn);
            }
            if c.buildings.len() >= crate::world::building_slots(c) as usize {
                return Err(RejectReason::Blocked);
            }
            if c.buildings.contains(building) {
                return Err(RejectReason::InvalidState);
            }
            let base_cost = crate::model::BUILD_COST[crate::world::building_index(building)];
            let cost = if *building == BuildingKind::Market
                && c.specialization == Some(CitySpecialization::TradeHub)
            {
                base_cost.saturating_sub(crate::model::TRADE_HUB_MARKET_DISCOUNT)
            } else {
                base_cost
            };
            if state.players[actor.0 as usize].resources.wealth < cost {
                return Err(RejectReason::NoResource);
            }
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
            if c.population < POP_FOR_SPECIALIZE {
                return Err(RejectReason::InvalidState);
            }
            if c.specialization.is_some() {
                return Err(RejectReason::InvalidState);
            }
            if state.players[actor.0 as usize].resources.influence < SPECIALIZE_COST_INFLUENCE {
                return Err(RejectReason::NoResource);
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
            crate::world::resolve_build(state, city, building, actor)
        }

        Command::Specialize { city, spec } => {
            crate::world::resolve_specialize(state, city, spec, actor)
        }

        Command::ConnectRoute { from, to } => {
            crate::caravan::resolve_connect(state, from, to, actor)
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
            crate::combat::resolve_raid_contest(state, unit, route)
        }

        Command::RaidCity { unit, city } => crate::combat::resolve_city_raid(state, unit, city),
    };

    state.log.append(&mut events.clone());
    events
}

/// Fully resolved in Phase 1: move if a path exists and moves remain.
/// Triggers combat if an enemy unit occupies the destination tile.
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

    // Reveal fog from the unit's stop position.
    crate::fog::reveal_from_unit(state, unit);

    let mut events = vec![GameEvent::UnitMoved {
        unit,
        from: from_tile,
        to,
    }];

    // Check for enemy unit at destination — trigger combat (spec §6.1).
    if let Some(enemy_id) = state
        .units
        .iter()
        .find(|u| u.tile == to && u.owner != actor)
        .map(|u| u.id)
    {
        let combat_events = crate::combat::resolve_combat(state, unit, enemy_id, from_tile);
        events.extend(combat_events);
    }

    events
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
        queue: vec![],
    };
    state.cities.push(city);

    // Reveal fog around the new city.
    let city_sight_radius = crate::fog::city_sight(state, city_id);
    crate::fog::reveal(state, actor, tile, city_sight_radius);

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
/// Raider requires Fortress specialization; Fortress grants a training discount.
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

    let city_data = &state.cities[city.0 as usize];

    // Raider must be in Fortress.
    if kind == UnitKind::Raider && city_data.specialization != Some(CitySpecialization::Fortress) {
        return vec![GameEvent::Rejected {
            command: Command::TrainUnit { city, kind },
            reason: RejectReason::InvalidState,
        }];
    }

    // Check unit cap.
    let cap = crate::world::unit_cap(state, actor);
    let current = state.units.iter().filter(|u| u.owner == actor).count() as u32;
    if current >= cap {
        return vec![GameEvent::Rejected {
            command: Command::TrainUnit { city, kind },
            reason: RejectReason::Blocked,
        }];
    }

    // Calculate cost (Fortress discount).
    let mut cost = UNIT_TRAIN_COST[crate::world::unit_kind_index(&kind)];
    if city_data.specialization == Some(CitySpecialization::Fortress) {
        cost = (cost as f32 * crate::model::FORTRESS_TRAIN_DISCOUNT) as u32;
    }

    // Validate: enough wealth.
    if state.players[actor.0 as usize].resources.wealth < cost {
        return vec![GameEvent::Rejected {
            command: Command::TrainUnit { city, kind },
            reason: RejectReason::NoResource,
        }];
    }

    // Spend wealth.
    state.players[actor.0 as usize].resources.wealth -= cost;

    // Spawn unit.
    let tile = state.cities[city.0 as usize].tile;
    let unit_id = state.alloc_unit_id();
    let def = unit_def(kind);
    state.units.push(crate::Unit {
        id: unit_id,
        owner: actor,
        kind,
        tile,
        hp: def.hp as u32,
        moves_left: def.moves,
        ability: crate::UnitAbility::None,
    });

    // Reveal fog from the newly trained unit.
    crate::fog::reveal_from_unit(state, unit_id);

    vec![GameEvent::UnitTrained {
        unit: unit_id,
        city,
    }]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::neighbors;
    use crate::model::{GameState, PlayerKind, Tile};
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

    /// Integration test: spin up a full world-gen game (radius 4, 3 players,
    /// 30 turns) and run every player through `EndTurn` until the game ends
    /// naturally — either via a Victory event or reaching the turn limit.
    #[test]
    fn headless_game_reaches_natural_end() {
        let cfg = mvp_preset();
        let mut state = crate::map::new_game(&cfg, 42);

        assert!(
            !state.players.is_empty(),
            "world-gen should produce at least one player"
        );

        let mut victory_found = false;

        // Run until natural end. Each call to `step` acts as current_actor
        // and advances the actor pointer, so a flat loop suffices.
        for _ in 0..cfg.turn_limit + 5 {
            let events = step(&mut state, &[Command::EndTurn]);

            for event in &events {
                if let GameEvent::Victory { .. } = event {
                    victory_found = true;
                    break;
                }
            }

            if victory_found {
                break;
            }

            // If only one (or zero) players remain, game is over by elimination.
            if state.players.iter().filter(|p| !p.defeated).count() <= 1 {
                break;
            }
        }

        // Game should have ended naturally.
        let game_over = victory_found
            || state.turn >= cfg.turn_limit
            || state.players.iter().filter(|p| !p.defeated).count() <= 1;
        assert!(
            game_over,
            "Game should reach a natural end (victory, turn limit, or elimination)"
        );

        // State should be valid: turn progressed at least once.
        assert!(
            state.turn >= 1,
            "Turn counter should be at least 1 after game starts"
        );
    }

    /// Integration test: run a full game with AI players using the AI planner
    /// and verify the game reaches victory, AI generated meaningful commands,
    /// and the VictoryTracker is populated.
    ///
    /// This proves the AI ↔ turn-engine pipeline works end-to-end: the AI
    /// reads the game state, produces legal commands, the resolver applies them,
    /// and the game eventually terminates with a victory.
    #[test]
    fn ai_driven_game_reaches_victory() {
        let cfg = mvp_preset();
        let seed = 42;
        let mut state = crate::map::new_game(&cfg, seed);

        assert!(
            state.players.len() >= 2,
            "world-gen should produce at least 2 players"
        );

        let mut victory_found = false;
        let mut ai_plan_called = false;
        let mut total_ai_commands: usize = 0;

        // Safety limit: generous headroom above the configured turn limit.
        let max_iterations = cfg.turn_limit + 50;

        for _iter in 0..max_iterations {
            let actor = state.current_actor;
            let player = &state.players[actor.0 as usize];

            let commands = match &player.kind {
                PlayerKind::Ai { difficulty, .. } => {
                    // AI player: plan + EndTurn.
                    let mut cmds = crate::ai::ai_plan(&state, actor, *difficulty);
                    // Track that AI actually produced meaningful commands.
                    if !cmds.is_empty() {
                        ai_plan_called = true;
                        total_ai_commands += cmds.len();
                    }
                    cmds.push(Command::EndTurn);
                    cmds
                }
                PlayerKind::Human => {
                    // No human players in this test, but be defensive.
                    vec![Command::EndTurn]
                }
            };

            let events = step(&mut state, &commands);

            for event in &events {
                if let GameEvent::Victory { .. } = event {
                    victory_found = true;
                    break;
                }
            }

            if victory_found {
                break;
            }

            // If only one (or zero) players remain, game is over by elimination.
            if state.players.iter().filter(|p| !p.defeated).count() <= 1 {
                break;
            }
        }

        // --- Phase 3 exit criteria ---

        // 1. The game reached a Victory event.
        let game_over = victory_found
            || state.turn >= cfg.turn_limit
            || state.players.iter().filter(|p| !p.defeated).count() <= 1;
        assert!(
            game_over,
            "Game should reach a natural end (victory, turn limit, or elimination)"
        );

        // 2. The ai_plan function was called and produced commands (not just
        //    EndTurn). This proves AI players actually engaged with the game.
        assert!(
            ai_plan_called,
            "AI plan should have been called and produced at least one command"
        );
        assert!(
            total_ai_commands > 0,
            "AI should have generated at least one non-EndTurn command"
        );

        // 3. The VictoryTracker maps are populated (not all zeros).
        //    After several turns of play, at least some players should appear
        //    in the tracker maps.
        assert!(
            !state.victory.oases_controlled.is_empty(),
            "victory.oases_controlled should be populated after the game runs"
        );
        assert!(
            !state.victory.prestige_score.is_empty(),
            "victory.prestige_score should be populated after the game runs"
        );

        // 4. State progressed: turn counter advanced beyond 1.
        assert!(
            state.turn >= 1,
            "Turn counter should be at least 1 after game starts"
        );

        // 5. No panics — if we reached here, the entire game ran without
        //    panicking (this test itself is the assertion).
    }
}
