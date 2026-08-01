//! GUI orchestration loop: owns the real `GameState` and drives the
//! human/AI turn cycle, per ADR-0003 ("A thin `dcs-app` crate owns the main
//! loop and dispatches input→core→render").
//!
//! `dcs-render` drives the actual per-frame macroquad loop (window setup,
//! camera, drawing) via `dcs_render::run`, calling back into the closure
//! below once per frame — but the closure's body, not `dcs-render`'s own
//! code, is what calls `GameState::step`, keeping the only `&mut GameState`
//! mutation in `dcs-app` as the architecture requires.

use dcs_core::{Command, GameEvent, GameState, PlayerKind, ScenarioConfig, map};
use dcs_render::{RenderConfig, Renderer};

const SEED: u64 = 42;

/// Launch the graphical game window with a real, freshly-generated
/// `GameState` and drive its turn cycle until the window is closed.
pub fn run_gui() {
    let state = map::new_game(&ScenarioConfig::mvp_preset(), SEED);
    #[cfg(feature = "dev-tools")]
    crate::agent_bridge::put_game_state(state.clone());
    let mut game_over = false;

    dcs_render::run(RenderConfig::default(), state, move |state, renderer| {
        // Pick up anything an agent_bridge dispatch call changed since the
        // last frame (a no-op copy of last frame's own checkin, below, if
        // nothing did) — dev-tools builds only; the bridge doesn't exist
        // otherwise.
        #[cfg(feature = "dev-tools")]
        if let Some(bridged) = crate::agent_bridge::take_game_state() {
            *state = bridged;
        }

        step_turn(state, renderer, &mut game_over);

        // Hand the latest state to agent_bridge so a dispatch call between
        // this frame and the next sees it. Cloned (not moved) since
        // `dcs_render::run` still needs `*state` valid for `draw_frame`,
        // called right after this closure returns.
        #[cfg(feature = "dev-tools")]
        crate::agent_bridge::put_game_state(state.clone());
    });
}

/// One turn-loop step: resolve the current actor's commands (human input or
/// AI plan) and apply them via `step`, if there are any.
fn step_turn(state: &mut GameState, renderer: &mut Renderer, game_over: &mut bool) {
    if *game_over {
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
    // `step`'s income phase runs unconditionally on every call, so it must
    // only be called when there's actually something to submit — not every
    // frame while waiting for human input.
    if commands.is_empty() {
        return;
    }

    let events = state.step(&commands);
    if events
        .iter()
        .any(|e| matches!(e, GameEvent::Victory { .. }))
    {
        // Freeze the loop on the final frame; no game-over screen yet (no
        // HUD).
        *game_over = true;
    }
}
