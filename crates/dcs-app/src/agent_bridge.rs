//! In-browser JSON agent bridge (feature = "dev-tools").
//!
//! Exposes the same `Request`/`Response` protocol `dcs-app::serve` already
//! speaks over stdin/stdout (and that `dcs-mcp` proxies as an MCP server),
//! from inside the running wasm module — reusing [`crate::serve::ServeState`]
//! as-is rather than inventing a second protocol. The actual `#[no_mangle]`
//! wasm exports live in `dcs-web.rs` (see that file for why); this module is
//! the ordinary-Rust plumbing they call into.
//!
//! wasm32 is single-threaded, so a plain `thread_local!` `RefCell` is
//! sufficient: a JS call into an export only ever happens *between*
//! animation frames, never concurrently with one.

use crate::protocol::{ERR_OTHER_UNEXPECTED, Request, Response};
use crate::serve::ServeState;
use dcs_core::GameState;
use std::cell::RefCell;

thread_local! {
    static STATE: RefCell<ServeState> = const {
        RefCell::new(ServeState {
            game: None,
            player_id: None,
        })
    };
}

/// Takes the current `GameState` out of the shared bridge state, leaving
/// `None` behind. Used by `orchestrate::run_gui` to swap it into the render
/// loop's local state for the duration of a frame.
pub fn take_game_state() -> Option<GameState> {
    STATE.with(|s| s.borrow_mut().game.take())
}

/// Puts a `GameState` into the shared bridge state (overwriting whatever was
/// there). Used both to seed the initial state and to swap the render loop's
/// state back in after a frame.
pub fn put_game_state(state: GameState) {
    STATE.with(|s| s.borrow_mut().game = Some(state));
}

/// Parses a JSON `Request`, dispatches it against the shared [`ServeState`],
/// and serializes the resulting `Response` back to JSON.
///
/// Malformed input produces a JSON-encoded error `Response`, not a panic —
/// an agent driving this over a raw byte-buffer FFI boundary should never be
/// able to crash the render loop by sending bad input.
pub fn dispatch_json(input: &str) -> String {
    let response = match serde_json::from_str::<Request>(input) {
        Ok(request) => STATE.with(|s| s.borrow_mut().dispatch(request)),
        Err(e) => Response::error(
            ERR_OTHER_UNEXPECTED,
            format!("malformed JSON: {e}"),
            Some("send a valid Request JSON object".into()),
            "unknown",
        ),
    };
    serde_json::to_string(&response).unwrap_or_else(|e| {
        format!(r#"{{"type":"error","code":"serialize_failed","message":"{e}"}}"#)
    })
}
