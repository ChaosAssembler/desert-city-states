//! stdin/stdout JSON protocol server for Desert City-States.
//!
//! The server reads one JSON [`Request`] per line from stdin, dispatches it to
//! the appropriate handler, and writes one JSON [`Response`] per line to stdout.
//! The protocol is synchronous and single-threaded — no async runtime is needed.
//!
//! # Usage
//!
//! ```bash
//! echo '{"type":"ping"}' | dcs-app serve
//! ```

use crate::protocol::*;
use dcs_core::hex::HexCoord;
use dcs_core::{Command, GameState, PlayerId, ScenarioConfig, TerrainType, TileId, UnitId, UnitKind};
use std::io::{self, BufRead, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// State maintained across requests in a serve session.
pub struct ServeState {
    game: Option<GameState>,
    player_id: Option<PlayerId>,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run the stdin/stdout protocol server.
///
/// Reads line-delimited JSON from stdin, dispatches each request, and writes
/// line-delimited JSON responses to stdout. Returns when stdin reaches EOF or
/// an I/O error occurs.
pub fn run_serve() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut state = ServeState {
        game: None,
        player_id: None,
    };

    for line in stdin.lock().lines() {
        let line = line?;
        let response = handle_line(&mut state, &line);
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parse + dispatch
// ---------------------------------------------------------------------------

/// Parse a single line and dispatch it to the appropriate handler.
fn handle_line(state: &mut ServeState, line: &str) -> Response {
    let line = line.trim();
    if line.is_empty() {
        return Response::error(
            ERR_OTHER_UNEXPECTED,
            "empty line",
            Some("send a JSON request".into()),
            "unknown",
        );
    }

    let request: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return Response::error(
                ERR_OTHER_UNEXPECTED,
                format!("malformed JSON: {e}"),
                Some("each line must be a valid JSON object".into()),
                "unknown",
            );
        }
    };

    dispatch(state, request)
}

/// Dispatch a parsed [`Request`] to the matching handler.
fn dispatch(state: &mut ServeState, request: Request) -> Response {
    match request {
        Request::Ping => Response::pong(),
        Request::Help => Response::help_info(),
        Request::NewGame { scenario, seed } => handle_new_game(state, scenario, seed),
        Request::LoadGame { path } => handle_load_game(state, &path),
        Request::SaveGame { path } => handle_save_game(state, &path),
        Request::ClaimPlayer {
            player_id,
            player_name,
        } => handle_claim_player(state, player_id, player_name),
        Request::Observe { player_id, detail } => handle_observe(state, player_id, detail),
        Request::Act {
            player_id,
            commands,
        } => handle_act(state, player_id, commands),
    }
}

// ---------------------------------------------------------------------------
// Handler: new_game
// ---------------------------------------------------------------------------

/// Create a new game from a scenario preset or config object.
///
/// Uses [`dcs_core::map::new_game`] for full world generation (tiles, oases,
/// players, starting units, fog). The generated [`GameState`] is stored in the
/// session state.
fn handle_new_game(state: &mut ServeState, scenario: ScenarioInput, seed: Option<u64>) -> Response {
    // Resolve the scenario configuration.
    let config = match scenario {
        ScenarioInput::Name(name) => {
            if name.contains('/') || name.contains('\\') || name.ends_with(".toml") || name.ends_with(".json") {
                match ScenarioConfig::load(Path::new(&name)) {
                    Ok(c) => c,
                    Err(e) => {
                        return Response::error(
                            ERR_SCENARIO_LOAD_FAILED,
                            format!("failed to load scenario from {}: {e}", name),
                            None,
                            "new_game",
                        );
                    }
                }
            } else {
                match name.as_str() {
                    "mvp" => ScenarioConfig::mvp_preset(),
                    _ => {
                        return Response::error(
                            ERR_SCENARIO_LOAD_FAILED,
                            format!("unknown scenario preset: {}", name),
                            Some("try \"mvp\" or provide a config object".into()),
                            "new_game",
                        );
                    }
                }
            }
        }
        ScenarioInput::Config(_value) => {
            // TODO: merge partial config with defaults
            ScenarioConfig::mvp_preset()
        }
    };

    // Use the provided seed or generate one from the current time.
    let effective_seed = seed.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    });

    // Run full world generation.
    let game = dcs_core::map::new_game(&config, effective_seed);

    // Extract game info before storing.
    let players: Vec<PlayerInfo> = game
        .players
        .iter()
        .map(|p| PlayerInfo {
            player_id: p.id.0,
            label: format!("Player {}", p.id.0 + 1),
        })
        .collect();
    let turn = game.turn;
    let map_radius = game.scenario.map_radius as u32;

    state.game = Some(game);
    state.player_id = None; // Reset claimed player on new game.

    Response::game_created(players, turn, map_radius)
}

// ---------------------------------------------------------------------------
// Handler: load_game
// ---------------------------------------------------------------------------

/// Load a previously saved game from disk.
///
/// Uses [`GameState::load`] which auto-detects the format from the file
/// extension (json, postcard, bin, bincode).
fn handle_load_game(state: &mut ServeState, path: &str) -> Response {
    match GameState::load(path) {
        Ok(game) => {
            let players: Vec<PlayerInfo> = game
                .players
                .iter()
                .map(|p| PlayerInfo {
                    player_id: p.id.0,
                    label: format!("Player {}", p.id.0 + 1),
                })
                .collect();
            let turn = game.turn;
            let map_radius = game.scenario.map_radius as u32;

            state.game = Some(game);
            state.player_id = None; // Reset claimed player on load.

            Response::GameLoaded {
                players,
                turn,
                map_radius,
            }
        }
        Err(e) => Response::error(
            ERR_IO_FILE,
            format!("failed to load game: {e}"),
            None,
            "load_game",
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: save_game
// ---------------------------------------------------------------------------

/// Save the current game state to disk as JSON.
///
/// Uses [`GameState::save_debug`] which writes human-readable JSON.
fn handle_save_game(state: &mut ServeState, path: &str) -> Response {
    let game = match ensure_game(state) {
        Ok(g) => g,
        Err(resp) => return resp,
    };

    match game.save_debug(path) {
        Ok(()) => Response::saved(path.to_owned()),
        Err(e) => Response::error(
            ERR_IO_FILE,
            format!("failed to save game: {e}"),
            None,
            "save_game",
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: claim_player
// ---------------------------------------------------------------------------

/// Claim a player slot so subsequent commands target that player.
///
/// Validates that a game is loaded, the player ID exists, and the slot hasn't
/// already been claimed (for simplicity, only one claim per session — the last
/// claim wins).
fn handle_claim_player(state: &mut ServeState, player_id: u32, player_name: Option<String>) -> Response {
    let game = match ensure_game(state) {
        Ok(g) => g,
        Err(resp) => return resp,
    };

    // Validate player ID exists in the game.
    if player_id as usize >= game.players.len() {
        return Response::error(
            ERR_INVALID_PLAYER,
            format!(
                "player_id {player_id} does not exist (game has {} players)",
                game.players.len()
            ),
            Some(format!("use a player_id between 0 and {}", game.players.len() - 1)),
            "claim_player",
        );
    }

    let name = player_name.unwrap_or_else(|| format!("Player {}", player_id + 1));
    let color = format!("{}", game.players[player_id as usize].color);
    state.player_id = Some(PlayerId(player_id));

    Response::player_claimed(player_id, name, color)
}

// ---------------------------------------------------------------------------
// Handler: observe
// ---------------------------------------------------------------------------

/// Return the current game state for the observing player.
///
/// Provides turn number, current phase, current actor, the player's
/// resources, cities, and units. Fog-of-war filtering is deferred to a
/// later wave — this returns raw game data.
fn handle_observe(
    state: &mut ServeState,
    player_id: Option<u32>,
    _detail: Option<String>,
) -> Response {
    // Validate game is loaded.
    let game = match ensure_game(state) {
        Ok(g) => g,
        Err(resp) => return resp,
    };

    // Resolve which player is observing.
    let observing_player = player_id
        .or_else(|| state.player_id.map(|p| p.0))
        .unwrap_or(0);

    // Validate the observing player exists.
    if observing_player as usize >= game.players.len() {
        return Response::error(
            ERR_INVALID_PLAYER,
            format!(
                "player_id {observing_player} does not exist (game has {} players)",
                game.players.len()
            ),
            Some(format!(
                "use a player_id between 0 and {}",
                game.players.len() - 1
            )),
            "observe",
        );
    }

    let player = &game.players[observing_player as usize];

    // Determine if it is this player's turn.
    let is_my_turn = game.current_actor.0 == observing_player;

    // Build cities array for the observing player.
    let cities: Vec<serde_json::Value> = game
        .cities
        .iter()
        .filter(|c| c.owner.0 == observing_player)
        .map(|city| {
            let tile = &game.tiles[city.tile.0 as usize];
            serde_json::json!({
                "city_id": city.id.0,
                "name": format!("City {}", city.id.0 + 1),
                "population": city.population,
                "production_capacity": city.building_slots(),
                "tile": {
                    "q": tile.coord.q,
                    "r": tile.coord.r,
                },
            })
        })
        .collect();

    // Build units array for the observing player.
    let units: Vec<serde_json::Value> = game
        .units
        .iter()
        .filter(|u| u.owner.0 == observing_player)
        .map(|unit| {
            let tile = &game.tiles[unit.tile.0 as usize];
            serde_json::json!({
                "unit_id": unit.id.0,
                "unit_type": unit.kind.to_string(),
                "tile": {
                    "q": tile.coord.q,
                    "r": tile.coord.r,
                },
                "hp": unit.hp,
            })
        })
        .collect();

    // Compute legal actions for the observing player.
    let legal_actions = compute_legal_actions(game, PlayerId(observing_player));

    // Build the observation payload.
    let observation = serde_json::json!({
        "turn": game.turn,
        "player_id": observing_player,
        "current_phase": game.phase.to_string(),
        "current_actor": game.current_actor.0,
        "is_my_turn": is_my_turn,
        "resources": {
            "water": player.resources.water,
            "wealth": player.resources.wealth,
            "influence": player.resources.influence,
        },
        "cities": cities,
        "units": units,
        "legal_actions": legal_actions,
    });

    Response::Observation {
        _placeholder: observation,
    }
}

// ---------------------------------------------------------------------------
// Handler: act
// ---------------------------------------------------------------------------

/// Execute one or more commands for the acting player's turn.
///
/// Validates game state and player claim, converts wire [`CommandInput`]s to
/// core [`Command`]s, runs them through [`GameState::step`], and returns the
/// resulting turn info and any errors.
///
/// Currently supports [`CommandInput::EndTurn`] and [`CommandInput::MoveUnit`].
/// Other command variants will be added in later waves.
fn handle_act(
    state: &mut ServeState,
    player_id: Option<u32>,
    commands: Vec<CommandInput>,
) -> Response {
    // Validate game is loaded. Mutable access is required for `step`.
    let game = match state.game.as_mut() {
        Some(g) => g,
        None => {
            return Response::error(
                ERR_GAME_NOT_INITIALIZED,
                "no game loaded",
                Some("call new_game or load_game first".into()),
                "act",
            );
        }
    };

    // Validate player is claimed.
    let claimed_player = match state.player_id {
        Some(pid) => pid,
        None => {
            return Response::error(
                ERR_NO_PLAYER_CLAIMED,
                "no player claimed",
                Some("call claim_player first".into()),
                "act",
            );
        }
    };

    // Resolve effective player: explicit player_id overrides the claimed one.
    let effective_player = player_id.unwrap_or(claimed_player.0);

    // Validate player ID exists in the game.
    if effective_player as usize >= game.players.len() {
        return Response::error(
            ERR_INVALID_PLAYER,
            format!(
                "player_id {effective_player} does not exist (game has {} players)",
                game.players.len()
            ),
            Some(format!("use a player_id between 0 and {}", game.players.len() - 1)),
            "act",
        );
    }

    let acting_player = PlayerId(effective_player);

    // Validate it is this player's turn.
    if game.current_actor != acting_player {
        return Response::error(
            ERR_NOT_YOUR_TURN,
            format!(
                "it is player {}'s turn, not player {}'s",
                game.current_actor.0, effective_player
            ),
            Some("wait for your turn or use observe to check the current actor".into()),
            "act",
        );
    }

    // Convert wire commands to core commands, collecting unsupported ones as
    // errors so the client knows which commands were skipped.
    let mut core_commands = Vec::new();
    let mut errors = Vec::new();

    for cmd_input in &commands {
        match cmd_input {
            CommandInput::EndTurn => {
                core_commands.push(Command::EndTurn);
            }
            CommandInput::MoveUnit { unit, to } => {
                let to_tile = match to {
                    TileCoord::Id(id) => TileId(*id),
                    TileCoord::Hex { q, r } => {
                        let coord = HexCoord { q: *q, r: *r };
                        match game.tile_index.get(&coord) {
                            Some(&tile_id) => tile_id,
                            None => {
                                errors.push(format!(
                                    "unknown tile coordinate ({q}, {r})"
                                ));
                                continue;
                            }
                        }
                    }
                };
                core_commands.push(Command::MoveUnit {
                    unit: UnitId(*unit),
                    to: to_tile,
                });
            }
            other => {
                errors.push(format!("unsupported command: {other:?}"));
            }
        }
    }

    // Execute the supported commands through the turn engine.
    if !core_commands.is_empty() {
        let _events = game.step(&core_commands);
        // Game events are intentionally not mapped to EventOutput yet.
        // They will be converted in a later wave.
    }

    // Return the current turn state and any errors from unsupported commands.
    Response::events(game.turn, Vec::new(), None, errors)
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Ensure a game is loaded, returning an error [`Response`] if not.
fn ensure_game(state: &ServeState) -> Result<&GameState, Response> {
    state.game.as_ref().ok_or_else(|| {
        Response::error(
            ERR_GAME_NOT_INITIALIZED,
            "no game loaded",
            Some("call new_game or load_game first".into()),
            "unknown",
        )
    })
}

/// Compute the legal actions for a player given the current game state.
///
/// Returns a list of command name strings that the player can execute right now.
/// This is a simplified MVP check — it does not validate full preconditions
/// (e.g. resource costs), only whether the player has the prerequisite entities.
fn compute_legal_actions(game: &GameState, player_id: PlayerId) -> Vec<String> {
    let mut actions = Vec::new();
    let is_my_turn = game.current_actor == player_id;

    // Collect the player's units and cities once for reuse.
    let player_units: Vec<_> = game.units.iter().filter(|u| u.owner == player_id).collect();
    let player_cities: Vec<_> = game.cities.iter().filter(|c| c.owner == player_id).collect();

    // end_turn: always available when it's the player's turn.
    if is_my_turn {
        actions.push("end_turn".into());
    }

    // move_unit: available if the player has any units.
    if !player_units.is_empty() {
        actions.push("move_unit".into());
    }

    // found_city: available if the player has a scout on an oasis tile
    // that doesn't already have a city.
    let has_founding_scout = player_units.iter().any(|u| {
        u.kind == UnitKind::Scout
            && u.moves_left > 0
            && game.tiles[u.tile.0 as usize].terrain == TerrainType::Oasis
            && !game.cities.iter().any(|c| c.tile == u.tile)
    });
    if has_founding_scout {
        actions.push("found_city".into());
    }

    // build: available if the player has cities with free building slots.
    let has_buildable_city = player_cities.iter().any(|c| {
        let slots = c.building_slots();
        (c.buildings.len() as u8) < slots
    });
    if has_buildable_city {
        actions.push("build".into());
    }

    // train_unit: available if the player has cities (population >= 1).
    if player_cities.iter().any(|c| c.population >= 1) {
        actions.push("train_unit".into());
    }

    // patrol: available if the player has CaravanGuard units.
    if player_units.iter().any(|u| u.kind == UnitKind::CaravanGuard) {
        actions.push("patrol".into());
    }

    // garrison: available if the player has CaravanGuard units and cities.
    if player_units.iter().any(|u| u.kind == UnitKind::CaravanGuard)
        && !player_cities.is_empty()
    {
        actions.push("garrison".into());
    }

    // raid: available if the player has Raider units.
    if player_units.iter().any(|u| u.kind == UnitKind::Raider) {
        actions.push("raid".into());
    }

    // connect_route: available if the player has 2+ cities.
    if player_cities.len() >= 2 {
        actions.push("connect_route".into());
    }

    actions
}
