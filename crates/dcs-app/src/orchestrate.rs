//! GUI orchestration loop: owns the real `GameState` and drives the
//! human/AI turn cycle, per ADR-0003 ("A thin `dcs-app` crate owns the main
//! loop and dispatches input→core→render").
//!
//! `dcs-render` drives the actual per-frame macroquad loop (window setup,
//! camera, drawing) via `dcs_render::run`, calling back into the closure
//! below once per frame — but the closure's body, not `dcs-render`'s own
//! code, is what calls `GameState::step`, keeping the only `&mut GameState`
//! mutation in `dcs-app` as the architecture requires.

use dcs_core::{Command, GameEvent, PlayerKind, ScenarioConfig, map};
use dcs_render::RenderConfig;

const SEED: u64 = 42;

/// Launch the graphical game window with a real, freshly-generated
/// `GameState` and drive its turn cycle until the window is closed.
pub fn run_gui() {
    let state = map::new_game(&ScenarioConfig::mvp_preset(), SEED);
    let mut game_over = false;

    dcs_render::run(RenderConfig::default(), state, move |state, renderer| {
        if game_over {
            return;
        }

        let actor = state.current_actor;
        let commands = match state.players[actor.0 as usize].kind {
            PlayerKind::Human => renderer.poll_input(state),
            PlayerKind::Ai { difficulty, .. } => {
                let mut cmds = state.ai_plan(actor, difficulty);
                cmds.push(Command::EndTurn);
                cmds
            }
        };
        // `step`'s income phase runs unconditionally on every call, so it
        // must only be called when there's actually something to submit —
        // not every frame while waiting for human input.
        if commands.is_empty() {
            return;
        }

        let events = state.step(&commands);
        if events
            .iter()
            .any(|e| matches!(e, GameEvent::Victory { .. }))
        {
            // Freeze the loop on the final frame; no game-over screen yet
            // (no HUD).
            game_over = true;
        }
    });
}
