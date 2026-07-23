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
use dcs_core::model::{BUILD_COST, UNIT_TRAIN_COST};
use dcs_core::{
    BuildingKind, BuildingKindExt, CityId, Command, GameEvent, GameState, PlayerId,
    RouteId, ScenarioConfig, TileId, UnitId, UnitKind, UnitKindExt,
};
use std::io::{self, BufRead, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Act-error helper
// ---------------------------------------------------------------------------

/// A structured error from command conversion, carrying a code, message, and
/// optional hint so agents know *why* an action failed and *what* to do next.
struct ActError {
    code: &'static str,
    message: String,
    hint: Option<String>,
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// State maintained across requests in a serve session.
pub struct ServeState {
    game: Option<GameState>,
    player_id: Option<PlayerId>,
}

impl ServeState {
    // ------------------------------------------------------------------
    // Parse + dispatch
    // ------------------------------------------------------------------

    /// Parse a single line and dispatch it to the appropriate handler.
    fn handle_line(&mut self, line: &str) -> Response {
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

        self.dispatch(request)
    }

    /// Dispatch a parsed [`Request`] to the matching handler.
    fn dispatch(&mut self, request: Request) -> Response {
        match request {
            Request::Ping => Response::pong(),
            Request::Help => Response::help_info(),
            Request::NewGame { scenario, seed } => self.handle_new_game(scenario, seed),
            Request::LoadGame { path } => self.handle_load_game(&path),
            Request::SaveGame { path } => self.handle_save_game(&path),
            Request::ClaimPlayer {
                player_id,
                player_name,
            } => self.handle_claim_player(player_id, player_name),
            Request::Observe { player_id, detail } => self.handle_observe(player_id, detail),
            Request::Act {
                player_id,
                commands,
            } => self.handle_act(player_id, commands),
        }
    }

    // ------------------------------------------------------------------
    // Handler: new_game
    // ------------------------------------------------------------------

    /// Create a new game from a scenario preset or config object.
    ///
    /// Uses [`dcs_core::map::new_game`] for full world generation (tiles, oases,
    /// players, starting units, fog). The generated [`GameState`] is stored in the
    /// session state.
    fn handle_new_game(&mut self, scenario: ScenarioInput, seed: Option<u64>) -> Response {
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

        self.game = Some(game);
        self.player_id = None; // Reset claimed player on new game.

        Response::game_created(players, turn, map_radius)
    }

    // ------------------------------------------------------------------
    // Handler: load_game
    // ------------------------------------------------------------------

    /// Load a previously saved game from disk.
    ///
    /// Uses [`GameState::load`] which auto-detects the format from the file
    /// extension (json, postcard, bin, bincode).
    fn handle_load_game(&mut self, path: &str) -> Response {
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

                self.game = Some(game);
                self.player_id = None; // Reset claimed player on load.

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

    // ------------------------------------------------------------------
    // Handler: save_game
    // ------------------------------------------------------------------

    /// Save the current game state to disk as JSON.
    ///
    /// Uses [`GameState::save_debug`] which writes human-readable JSON.
    fn handle_save_game(&mut self, path: &str) -> Response {
        let game = match self.ensure_game() {
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

    // ------------------------------------------------------------------
    // Handler: claim_player
    // ------------------------------------------------------------------

    /// Claim a player slot so subsequent commands target that player.
    ///
    /// Validates that a game is loaded, the player ID exists, and the slot hasn't
    /// already been claimed (for simplicity, only one claim per session — the last
    /// claim wins).
    fn handle_claim_player(&mut self, player_id: u32, player_name: Option<String>) -> Response {
        let game = match self.ensure_game() {
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
        self.player_id = Some(PlayerId(player_id));

        Response::player_claimed(player_id, name, color)
    }

    // ------------------------------------------------------------------
    // Handler: observe
    // ------------------------------------------------------------------

    /// Return the current game state for the observing player.
    ///
    /// Provides turn number, current phase, current actor, the player's
    /// resources, cities, and units. Fog-of-war filtering is applied:
    /// only tiles in the player's `discovered` set are visible, and
    /// only units/cities on those tiles (or owned by the player) are shown.
    fn handle_observe(
        &mut self,
        player_id: Option<u32>,
        _detail: Option<String>,
    ) -> Response {
        // Validate game is loaded.
        let game = match self.ensure_game() {
            Ok(g) => g,
            Err(resp) => return resp,
        };

        // Resolve which player is observing.
        let observing_player = player_id
            .or_else(|| self.player_id.map(|p| p.0))
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

        let observer = PlayerId(observing_player);

        // Build cities array — own cities plus visible enemy cities (fog-of-war).
        let cities: Vec<serde_json::Value> = game
            .cities
            .iter()
            .filter(|c| {
                c.owner == observer || game.is_city_visible(observer, c.id)
            })
            .map(|city| {
                let tile = &game.tiles[city.tile.0 as usize];
                serde_json::json!({
                    "city_id": city.id.0,
                    "name": format!("City {}", city.id.0 + 1),
                    "owner": city.owner.0,
                    "population": city.population,
                    "production_capacity": city.building_slots(),
                    "tile": {
                        "q": tile.coord.q,
                        "r": tile.coord.r,
                    },
                })
            })
            .collect();

        // Build units array — own units plus visible enemy units (fog-of-war).
        let units: Vec<serde_json::Value> = game
            .units
            .iter()
            .filter(|u| {
                u.owner == observer || game.is_unit_visible(observer, u.id)
            })
            .map(|unit| {
                let tile = &game.tiles[unit.tile.0 as usize];
                serde_json::json!({
                    "unit_id": unit.id.0,
                    "unit_type": unit.kind.to_string(),
                    "owner": unit.owner.0,
                    "tile": {
                        "q": tile.coord.q,
                        "r": tile.coord.r,
                    },
                    "hp": unit.hp,
                })
            })
            .collect();

        // Compute legal actions for the observing player.
        let legal_actions = game.legal_actions_for(observer);

        // Collect the player's discovered tile IDs for the client.
        let discovered_tiles: Vec<u32> = player.discovered.iter().map(|t| t.0).collect();

        // Build routes array — visible trade routes (fog-of-war filtered).
        let routes: Vec<serde_json::Value> = game
            .routes
            .iter()
            .filter(|r| game.is_route_visible(observer, r.id))
            .map(|route| {
                serde_json::json!({
                    "route_id": route.id.0,
                    "from": route.endpoints.0 .0,
                    "to": route.endpoints.1 .0,
                    "status": route.status.to_string(),
                })
            })
            .collect();

        // Determine game-over status and winner from the event log.
        let is_game_over = game.log.iter().any(|e| matches!(e, GameEvent::Victory { .. }));
        let winner = game
            .log
            .iter()
            .find_map(|e| {
                if let GameEvent::Victory { winner, .. } = e {
                    Some(winner.0)
                } else {
                    None
                }
            });

        // Extract victory points (prestige score) and oases controlled for this player.
        let victory_points = game.victory.prestige_score.get(&observer).copied().unwrap_or(0);
        let oases_controlled = game.victory.oases_controlled.get(&observer).copied().unwrap_or(0);
        let turn_limit = game.scenario.turn_limit;

        // Build the observation payload.
        let observation = serde_json::json!({
            "turn": game.turn,
            "player_id": observing_player,
            "current_phase": game.phase.to_string(),
            "current_actor": game.current_actor.0,
            "is_my_turn": is_my_turn,
            "turn_limit": turn_limit,
            "is_game_over": is_game_over,
            "winner": winner,
            "victory_points": victory_points,
            "oases_controlled": oases_controlled,
            "resources": {
                "water": player.resources.water,
                "wealth": player.resources.wealth,
                "influence": player.resources.influence,
            },
            "discovered_tiles": discovered_tiles,
            "cities": cities,
            "units": units,
            "routes": routes,
            "legal_actions": legal_actions,
        });

        Response::Observation {
            _placeholder: observation,
        }
    }

    // ------------------------------------------------------------------
    // Handler: act
    // ------------------------------------------------------------------

    /// Execute one or more commands for the acting player's turn.
    ///
    /// Validates game state and player claim, converts wire [`CommandInput`]s to
    /// core [`Command`]s, runs them through [`GameState::step`], and returns the
    /// resulting turn info and any errors.
    ///
    /// Currently supports [`CommandInput::EndTurn`], [`CommandInput::MoveUnit`],
    /// [`CommandInput::FoundCity`], [`CommandInput::Build`],
    /// [`CommandInput::TrainUnit`], [`CommandInput::Patrol`],
    /// [`CommandInput::RaidCity`], [`CommandInput::RaidRoute`],
    /// [`CommandInput::ConnectRoute`], and [`CommandInput::Garrison`].
    /// Other command variants will be added in later waves.
    fn handle_act(
        &mut self,
        player_id: Option<u32>,
        commands: Vec<CommandInput>,
    ) -> Response {
        // Validate game is loaded. Mutable access is required for `step`.
        let game = match self.game.as_mut() {
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
        let claimed_player = match self.player_id {
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

        // Convert wire commands to core commands.
        let (core_commands, act_errors) = convert_commands(&commands, game, acting_player);

        // Execute the supported commands through the turn engine.
        let mut output_events = Vec::new();
        let mut victory_info = None;

        if !core_commands.is_empty() {
            let events = game.step(&core_commands);
            for event in &events {
                // Extract Victory info for the top-level field.
                if let GameEvent::Victory { kind, winner } = event {
                    victory_info = Some(VictoryInfo {
                        kind: kind.to_string(),
                        winner: winner.0,
                    });
                }
                output_events.push(game_event_to_output(event));
            }
        }

        // Format ActErrors into human-readable error strings with hints.
        let errors: Vec<String> = act_errors
            .iter()
            .map(|e| {
                match &e.hint {
                    Some(hint) => format!("[{}] {} — hint: {}", e.code, e.message, hint),
                    None => format!("[{}] {}", e.code, e.message),
                }
            })
            .collect();

        // Return the current turn state, events, and any errors from unsupported commands.
        Response::events(game.turn, output_events, victory_info, errors)
    }

    // ------------------------------------------------------------------
    // Validation helpers
    // ------------------------------------------------------------------

    /// Ensure a game is loaded, returning an error [`Response`] if not.
    fn ensure_game(&self) -> Result<&GameState, Response> {
        self.game.as_ref().ok_or_else(|| {
            Response::error(
                ERR_GAME_NOT_INITIALIZED,
                "no game loaded",
                Some("call new_game or load_game first".into()),
                "unknown",
            )
        })
    }
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
        let response = state.handle_line(&line);
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Command conversion
// ---------------------------------------------------------------------------

/// Convert wire [`CommandInput`]s to core [`Command`]s.
///
/// Returns the list of valid core commands and any [`ActError`]s for
/// unsupported or malformed commands. Each error carries a code, message,
/// and optional hint so agents know why an action failed and what to do next.
fn convert_commands(
    commands: &[CommandInput],
    game: &GameState,
    acting_player: PlayerId,
) -> (Vec<Command>, Vec<ActError>) {
    let mut core_commands = Vec::new();
    let mut errors = Vec::new();

    for cmd_input in commands {
        match cmd_input {
            CommandInput::EndTurn => {
                core_commands.push(Command::EndTurn);
            }
            CommandInput::MoveUnit { unit, to } => {
                // Validate the unit exists and is owned by the acting player.
                let unit_exists = game.units.iter().any(|u| u.id == UnitId(*unit) && u.owner == acting_player);
                if !unit_exists {
                    errors.push(ActError {
                        code: ERR_INVALID_UNIT,
                        message: format!("unit {unit} does not exist or is not yours"),
                        hint: Some("call observe to list your units and their IDs".into()),
                    });
                    continue;
                }
                // Validate the unit has moves remaining.
                let unit_data = game.units.iter().find(|u| u.id == UnitId(*unit)).unwrap();
                if unit_data.moves_left == 0 {
                    errors.push(ActError {
                        code: ERR_NO_MOVES_LEFT,
                        message: format!("unit {unit} has no moves left this turn"),
                        hint: Some("units get 1 move per turn; end your turn and wait for the next one".into()),
                    });
                    continue;
                }
                let to_tile = match to {
                    TileCoord::Id(id) => TileId(*id),
                    TileCoord::Hex { q, r } => {
                        let coord = HexCoord { q: *q, r: *r };
                        match game.tile_index.get(&coord) {
                            Some(&tile_id) => tile_id,
                            None => {
                                errors.push(ActError {
                                    code: ERR_INVALID_TILE,
                                    message: format!("unknown tile coordinate ({q}, {r})"),
                                    hint: Some("use observe to see valid tile coordinates (q, r) or tile IDs".into()),
                                });
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
            CommandInput::FoundCity { unit, tile } => {
                // Validate the unit exists and is owned by the acting player.
                let unit_exists = game.units.iter().any(|u| u.id == UnitId(*unit) && u.owner == acting_player);
                if !unit_exists {
                    errors.push(ActError {
                        code: ERR_INVALID_UNIT,
                        message: format!("unit {unit} does not exist or is not yours"),
                        hint: Some("call observe to list your units and their IDs".into()),
                    });
                    continue;
                }
                let tile_id = match tile {
                    TileCoord::Id(id) => TileId(*id),
                    TileCoord::Hex { q, r } => {
                        let coord = HexCoord { q: *q, r: *r };
                        match game.tile_index.get(&coord) {
                            Some(&tile_id) => tile_id,
                            None => {
                                errors.push(ActError {
                                    code: ERR_INVALID_TILE,
                                    message: format!("unknown tile coordinate ({q}, {r})"),
                                    hint: Some("use observe to see valid tile coordinates (q, r) or tile IDs".into()),
                                });
                                continue;
                            }
                        }
                    }
                };
                core_commands.push(Command::FoundCity {
                    unit: UnitId(*unit),
                    tile: tile_id,
                });
            }
            CommandInput::TrainUnit { city, kind } => {
                // Validate the city exists and is owned by the acting player.
                let city_exists = game.cities.iter().any(|c| c.id == CityId(*city) && c.owner == acting_player);
                if !city_exists {
                    errors.push(ActError {
                        code: ERR_INVALID_CITY,
                        message: format!("city {city} does not exist or is not yours"),
                        hint: Some("call observe to list your cities and their IDs".into()),
                    });
                    continue;
                }
                let unit_kind = match kind.as_str() {
                    "Scout" => UnitKind::Scout,
                    "CaravanGuard" => UnitKind::CaravanGuard,
                    "Raider" => UnitKind::Raider,
                    other => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: format!("unknown unit kind: {other}"),
                            hint: Some("valid kinds are: Scout, CaravanGuard, Raider".into()),
                        });
                        continue;
                    }
                };
                // Check that the player has enough wealth.
                let player = &game.players[acting_player.0 as usize];
                let cost = UNIT_TRAIN_COST[unit_kind.index()];
                if player.resources.wealth < cost {
                    errors.push(ActError {
                        code: ERR_INSUFFICIENT_RESOURCES,
                        message: format!(
                            "not enough wealth to train {kind}: need {cost}, have {}",
                            player.resources.wealth
                        ),
                        hint: Some(
                            "earn wealth from Market buildings or trade routes; \
                             end your turn to receive income"
                                .into(),
                        ),
                    });
                    continue;
                }
                core_commands.push(Command::TrainUnit {
                    city: CityId(*city),
                    kind: unit_kind,
                });
            }
            CommandInput::Build { city, building } => {
                // Validate the city exists and is owned by the acting player.
                let city_exists = game.cities.iter().any(|c| c.id == CityId(*city) && c.owner == acting_player);
                if !city_exists {
                    errors.push(ActError {
                        code: ERR_INVALID_CITY,
                        message: format!("city {city} does not exist or is not yours"),
                        hint: Some("call observe to list your cities and their IDs".into()),
                    });
                    continue;
                }
                let building_kind = match building.as_str() {
                    "Well" => BuildingKind::Well,
                    "Market" => BuildingKind::Market,
                    "Granary" => BuildingKind::Granary,
                    "Watchtower" => BuildingKind::Watchtower,
                    "Caravanserai" => BuildingKind::Caravanserai,
                    "Temple" => BuildingKind::Temple,
                    other => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: format!("unknown building kind: {other}"),
                            hint: Some(
                                "valid kinds are: Well, Market, Granary, Watchtower, Caravanserai, Temple"
                                    .into(),
                            ),
                        });
                        continue;
                    }
                };
                // Check that the player has enough resources.
                let player = &game.players[acting_player.0 as usize];
                let cost = BUILD_COST[building_kind.index()];
                if player.resources.wealth < cost {
                    errors.push(ActError {
                        code: ERR_INSUFFICIENT_RESOURCES,
                        message: format!(
                            "not enough wealth to build {building}: need {cost}, have {}",
                            player.resources.wealth
                        ),
                        hint: Some(
                            "earn wealth from Market buildings or trade routes; \
                             end your turn to receive income"
                                .into(),
                        ),
                    });
                    continue;
                }
                core_commands.push(Command::Build {
                    city: CityId(*city),
                    building: building_kind,
                });
            }
            CommandInput::Patrol { unit, tile } => {
                let unit_id = match unit {
                    Some(id) => {
                        // Validate the unit exists and is owned by the acting player.
                        let unit_exists = game.units.iter().any(|u| u.id == UnitId(*id) && u.owner == acting_player);
                        if !unit_exists {
                            errors.push(ActError {
                                code: ERR_INVALID_UNIT,
                                message: format!("unit {id} does not exist or is not yours"),
                                hint: Some("call observe to list your units and their IDs".into()),
                            });
                            continue;
                        }
                        UnitId(*id)
                    }
                    None => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: "Patrol requires a unit_id".into(),
                            hint: Some("include \"unit\": <id> in the command; use observe to find unit IDs".into()),
                        });
                        continue;
                    }
                };
                core_commands.push(Command::Patrol {
                    unit: unit_id,
                    tile: TileId(*tile),
                });
            }
            CommandInput::RaidCity { unit, city } => {
                let unit_id = match unit {
                    Some(id) => {
                        // Validate the unit exists and is owned by the acting player.
                        let unit_exists = game.units.iter().any(|u| u.id == UnitId(*id) && u.owner == acting_player);
                        if !unit_exists {
                            errors.push(ActError {
                                code: ERR_INVALID_UNIT,
                                message: format!("unit {id} does not exist or is not yours"),
                                hint: Some("call observe to list your units and their IDs".into()),
                            });
                            continue;
                        }
                        UnitId(*id)
                    }
                    None => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: "RaidCity requires a unit_id".into(),
                            hint: Some("include \"unit\": <id> in the command; use observe to find unit IDs".into()),
                        });
                        continue;
                    }
                };
                core_commands.push(Command::RaidCity {
                    unit: unit_id,
                    city: CityId(*city),
                });
            }
            CommandInput::RaidRoute { unit, route } => {
                let unit_id = match unit {
                    Some(id) => {
                        // Validate the unit exists and is owned by the acting player.
                        let unit_exists = game.units.iter().any(|u| u.id == UnitId(*id) && u.owner == acting_player);
                        if !unit_exists {
                            errors.push(ActError {
                                code: ERR_INVALID_UNIT,
                                message: format!("unit {id} does not exist or is not yours"),
                                hint: Some("call observe to list your units and their IDs".into()),
                            });
                            continue;
                        }
                        UnitId(*id)
                    }
                    None => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: "RaidRoute requires a unit_id".into(),
                            hint: Some("include \"unit\": <id> in the command; use observe to find unit IDs".into()),
                        });
                        continue;
                    }
                };
                // Validate the route exists.
                if !game.routes.iter().any(|r| r.id == RouteId(*route)) {
                    errors.push(ActError {
                        code: ERR_INVALID_ROUTE,
                        message: format!("route {route} does not exist"),
                        hint: Some("call observe to list visible routes and their IDs".into()),
                    });
                    continue;
                }
                core_commands.push(Command::RaidRoute {
                    unit: unit_id,
                    route: RouteId(*route),
                });
            }
            CommandInput::ConnectRoute { from, to } => {
                let from_city = match from {
                    Some(id) => {
                        // Validate the city exists and is owned by the acting player.
                        let city_exists = game.cities.iter().any(|c| c.id == CityId(*id) && c.owner == acting_player);
                        if !city_exists {
                            errors.push(ActError {
                                code: ERR_INVALID_CITY,
                                message: format!("city {id} does not exist or is not yours"),
                                hint: Some("call observe to list your cities and their IDs".into()),
                            });
                            continue;
                        }
                        CityId(*id)
                    }
                    None => {
                        // Pick the first city owned by the acting player.
                        match game
                            .cities
                            .iter()
                            .find(|c| c.owner == acting_player)
                        {
                            Some(city) => city.id,
                            None => {
                                errors.push(ActError {
                                    code: ERR_INVALID_CITY,
                                    message: "no owned city to use as route origin".into(),
                                    hint: Some(
                                        "found a city first, or supply a \"from\" city ID; \
                                         use observe to list your cities"
                                            .into(),
                                    ),
                                });
                                continue;
                            }
                        }
                    }
                };
                let to_city = CityId(*to);
                core_commands.push(Command::ConnectRoute {
                    from: from_city,
                    to: to_city,
                });
            }
            CommandInput::Garrison { unit, city } => {
                let unit_id = match unit {
                    Some(id) => {
                        // Validate the unit exists and is owned by the acting player.
                        let unit_exists = game.units.iter().any(|u| u.id == UnitId(*id) && u.owner == acting_player);
                        if !unit_exists {
                            errors.push(ActError {
                                code: ERR_INVALID_UNIT,
                                message: format!("unit {id} does not exist or is not yours"),
                                hint: Some("call observe to list your units and their IDs".into()),
                            });
                            continue;
                        }
                        UnitId(*id)
                    }
                    None => {
                        errors.push(ActError {
                            code: ERR_INVALID_COMMAND,
                            message: "Garrison requires a unit_id".into(),
                            hint: Some("include \"unit\": <id> in the command; use observe to find unit IDs".into()),
                        });
                        continue;
                    }
                };
                core_commands.push(Command::Garrison {
                    unit: unit_id,
                    city: CityId(*city),
                });
            }
            other => {
                errors.push(ActError {
                    code: ERR_INVALID_COMMAND,
                    message: format!("unsupported command: {other:?}"),
                    hint: Some(
                        "supported commands: EndTurn, MoveUnit, FoundCity, TrainUnit, Build, Patrol, RaidCity, RaidRoute, ConnectRoute, Garrison"
                            .into(),
                    ),
                });
            }
        }
    }

    (core_commands, errors)
}

/// Convert a core [`GameEvent`] into a wire-format [`EventOutput`].
///
/// Uses snake_case event type names and flattens the event fields into
/// a `serde_json::Value` payload. The mapping is straightforward since
/// `GameEvent` already derives `Serialize`.
fn game_event_to_output(event: &GameEvent) -> EventOutput {
    let (event_type, data) = match event {
        GameEvent::UnitMoved { unit, from, to } => (
            "unit_moved",
            serde_json::json!({ "unit": unit.0, "from": from.0, "to": to.0 }),
        ),
        GameEvent::CityFounded { city, owner, tile } => (
            "city_founded",
            serde_json::json!({ "city": city.0, "owner": owner.0, "tile": tile.0 }),
        ),
        GameEvent::UnitTrained { unit, city } => (
            "unit_trained",
            serde_json::json!({ "unit": unit.0, "city": city.0 }),
        ),
        GameEvent::Built { city, building } => (
            "building_built",
            serde_json::json!({ "city": city.0, "building": building.to_string() }),
        ),
        GameEvent::Specialized { city, spec } => (
            "city_specialized",
            serde_json::json!({ "city": city.0, "specialization": spec.to_string() }),
        ),
        GameEvent::RouteCreated {
            route,
            from,
            to,
            path,
        } => (
            "route_created",
            serde_json::json!({
                "route": route.0,
                "from": from.0,
                "to": to.0,
                "path": path.iter().map(|t| t.0).collect::<Vec<_>>(),
            }),
        ),
        GameEvent::RouteStatusChanged {
            route,
            old_status,
            status,
        } => (
            "route_status_changed",
            serde_json::json!({
                "route": route.0,
                "old_status": old_status.to_string(),
                "status": status.to_string(),
            }),
        ),
        GameEvent::UnitPatrolled { unit, tile } => (
            "unit_patrolled",
            serde_json::json!({ "unit": unit.0, "tile": tile.0 }),
        ),
        GameEvent::UnitGarrisoned { unit, city } => (
            "unit_garrisoned",
            serde_json::json!({ "unit": unit.0, "city": city.0 }),
        ),
        GameEvent::RouteRaided { route, by, severed } => (
            "route_raided",
            serde_json::json!({ "route": route.0, "by": by.0, "severed": severed }),
        ),
        GameEvent::CityRaided { city, by, pop_lost } => (
            "city_raided",
            serde_json::json!({ "city": city.0, "by": by.0, "pop_lost": pop_lost }),
        ),
        GameEvent::Combat {
            attacker,
            defender,
            attacker_loss,
            defender_loss,
            retreated,
        } => (
            "combat",
            serde_json::json!({
                "attacker": attacker.0,
                "defender": defender.0,
                "attacker_loss": attacker_loss,
                "defender_loss": defender_loss,
                "retreated": retreated,
            }),
        ),
        GameEvent::Income {
            player,
            water,
            wealth,
            influence,
        } => (
            "income",
            serde_json::json!({
                "player": player.0,
                "water": water,
                "wealth": wealth,
                "influence": influence,
            }),
        ),
        GameEvent::Grown { city, population } => (
            "city_grew",
            serde_json::json!({ "city": city.0, "population": population }),
        ),
        GameEvent::Starved { city, population } => (
            "city_starved",
            serde_json::json!({ "city": city.0, "population": population }),
        ),
        GameEvent::Revealed { player, tiles } => (
            "tiles_revealed",
            serde_json::json!({
                "player": player.0,
                "tiles": tiles.iter().map(|t| t.0).collect::<Vec<_>>(),
            }),
        ),
        GameEvent::Victory { kind, winner } => (
            "victory",
            serde_json::json!({ "kind": kind.to_string(), "winner": winner.0 }),
        ),
        GameEvent::TurnAdvanced { turn } => (
            "turn_advanced",
            serde_json::json!({ "turn": turn }),
        ),
        GameEvent::Rejected { command, reason } => (
            "rejected",
            serde_json::json!({ "command": format!("{command:?}"), "reason": reason.to_string() }),
        ),
        GameEvent::Warn { message } => (
            "warn",
            serde_json::json!({ "message": message }),
        ),
    };

    EventOutput {
        event_type: event_type.to_owned(),
        data,
    }
}
