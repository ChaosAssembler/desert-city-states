//! Save / load serialization for [`GameState`].
//!
//! This module is the single home for (de)serializing the simulation state to
//! and from the three supported wire formats (JSON, postcard, bincode) and for
//! the versioned save envelope defined in `dcs-protocol`.
//!
//! # Envelope strategy (ADR-0007)
//!
//! We serialize the [`VersionedSave<GameState>`] envelope rather than the raw
//! [`GameState`]. The envelope carries the schema `version` that gates
//! migrations on load; `GameState` independently mirrors `SAVE_VERSION` for
//! in-payload migration bookkeeping. The current `SAVE_VERSION` is `1`, so the
//! migration loop is forward-proof but currently a no-op.

use std::path::Path;

use thiserror::Error;

use crate::model::GameState;
use crate::{SAVE_VERSION, VersionedSave};

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// The on-disk wire format used for a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
    /// Human-readable UTF-8 JSON (good for debugging / diffs).
    Json,
    /// Compact, deterministic, no-std-friendly binary (the default).
    Postcard,
    /// Compact binary via `bincode`.
    Bincode,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while serializing, deserializing, or migrating a save.
#[derive(Debug, Error)]
pub enum SaveError {
    /// A save was produced by a newer engine than we can understand.
    #[error("unsupported save version {0} (current {1})")]
    VersionTooNew(u32, u32),
    /// A migration step from an older payload version failed.
    #[error("migration from {0} failed: {1}")]
    MigrationFailed(u32, String),
    /// A filesystem error (read / write / open).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A `serde` (de)serialization error.
    #[error("serde error: {0}")]
    Serde(String),
}

// ---------------------------------------------------------------------------
// Core (de)serialization
// ---------------------------------------------------------------------------

/// Serialize a game state to bytes using the specified format.
///
/// The state is wrapped in a [`VersionedSave`] envelope carrying the current
/// [`SAVE_VERSION`] before encoding, so every saved file is self-describing.
///
/// # Arguments
/// * `state` - Game state to serialize
/// * `fmt` - Output format (JSON, Postcard, or Bincode)
///
/// # Returns
/// `Ok(Vec<u8>)` with the serialized bytes, or `Err(SaveError)` on failure.
pub fn serialize(state: &GameState, fmt: SaveFormat) -> Result<Vec<u8>, SaveError> {
    let env = VersionedSave {
        version: SAVE_VERSION,
        payload: state.clone(),
    };
    match fmt {
        SaveFormat::Json => serde_json::to_vec(&env).map_err(|e| SaveError::Serde(e.to_string())),
        SaveFormat::Postcard => {
            postcard::to_stdvec(&env).map_err(|e| SaveError::Serde(e.to_string()))
        }
        SaveFormat::Bincode => {
            bincode::serialize(&env).map_err(|e| SaveError::Serde(e.to_string()))
        }
    }
}

/// Deserialize a game state from bytes.
///
/// Decodes the [`VersionedSave`] envelope, rejects saves newer than our schema
/// version, and runs the (currently empty) forward migration loop before
/// returning the payload.
///
/// # Arguments
/// * `bytes` - Serialized game state bytes
/// * `fmt` - Format to use for deserialization
///
/// # Returns
/// `Ok(GameState)` on success, or `Err(SaveError)` if the data is invalid
/// or uses an unsupported format.
pub fn deserialize(bytes: &[u8], fmt: SaveFormat) -> Result<GameState, SaveError> {
    let env: VersionedSave<GameState> = match fmt {
        SaveFormat::Json => {
            serde_json::from_slice(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
        }
        SaveFormat::Postcard => {
            postcard::from_bytes(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
        }
        SaveFormat::Bincode => {
            bincode::deserialize(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
        }
    };

    if env.version > SAVE_VERSION {
        return Err(SaveError::VersionTooNew(env.version, SAVE_VERSION));
    }

    let mut payload = env.payload;
    // Migration loop: while the payload is older than the current schema,
    // migrate it one version forward. No older versions exist yet
    // (SAVE_VERSION == 1), so this is currently a no-op.
    while payload.version < SAVE_VERSION {
        payload = migrate(payload.version, payload)?;
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// Write a [`GameState`] to `path` in the given [`SaveFormat`].
pub fn save(state: &GameState, path: &Path, fmt: SaveFormat) -> Result<(), SaveError> {
    let bytes = serialize(state, fmt)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a [`GameState`] from `path`, auto-detecting the format from the
/// file extension.
///
/// - `json` -> [`SaveFormat::Json`]
/// - `postcard` / `bin` -> [`SaveFormat::Postcard`]
/// - `bincode` -> [`SaveFormat::Bincode`]
/// - anything else -> [`SaveFormat::Postcard`] (the compact default)
pub fn load(path: &Path) -> Result<GameState, SaveError> {
    let bytes = std::fs::read(path)?;
    let fmt = match path.extension().and_then(|s| s.to_str()) {
        Some("json") => SaveFormat::Json,
        Some("postcard") | Some("bin") => SaveFormat::Postcard,
        Some("bincode") => SaveFormat::Bincode,
        _ => SaveFormat::Postcard, // unknown extension -> postcard default
    };
    deserialize(&bytes, fmt)
}

/// Write a human-readable JSON debug save to `path`.
pub fn save_debug(state: &GameState, path: &Path) -> Result<(), SaveError> {
    save(state, path, SaveFormat::Json)
}

// ---------------------------------------------------------------------------
// Migration registry
// ---------------------------------------------------------------------------

/// Migrate a payload from `from` to `from + 1`.
///
/// No older versions exist yet (`SAVE_VERSION == 1`), so there is nothing to
/// migrate. When a breaking schema change lands, register a deterministic
/// `(version -> version + 1)` transform here. The transform must be pure and
/// deterministic so that a save always migrates to the same result.
fn migrate(_from: u32, _state: GameState) -> Result<GameState, SaveError> {
    Err(SaveError::MigrationFailed(
        _from,
        "no migrations registered".into(),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::mvp_preset;
    use crate::{Command, GameEvent, PlayerId, RejectReason};
    use insta::assert_debug_snapshot;
    use insta::assert_json_snapshot;

    /// Build a small but non-trivial `GameState` to exercise the (de)serialize
    /// paths, including the fxhash serde helpers and the `SeededRng`.
    fn sample_state() -> GameState {
        let mut s = GameState::new(mvp_preset(), 12345);
        // Exercise fxhash collections.
        s.tile_index
            .insert(crate::hex::HexCoord { q: 0, r: 0 }, crate::TileId(0));
        s.victory.oases_controlled.insert(crate::PlayerId(0), 3);
        s.victory.prestige_score.insert(crate::PlayerId(1), 12);
        // Exercise the seeded RNG serialization path.
        let _ = s.rng.next_u32();
        s
    }

    /// Round-trip a state through the given format and assert that the bytes
    /// produced by re-serializing the loaded state are byte-identical to the
    /// original serialized bytes. `GameState` does not derive `PartialEq`
    /// (it holds an RNG and fxhash collections), so we compare serialized
    /// bytes instead.
    fn round_trip(fmt: SaveFormat) {
        let s = sample_state();
        let original = serialize(&s, fmt).expect("serialize");
        let loaded = deserialize(&original, fmt).expect("deserialize");
        let reencoded = serialize(&loaded, fmt).expect("re-serialize");
        assert_eq!(
            original, reencoded,
            "round-trip through {fmt:?} was not byte-stable"
        );
    }

    #[test]
    fn round_trip_json() {
        round_trip(SaveFormat::Json);
    }

    #[test]
    fn round_trip_postcard() {
        round_trip(SaveFormat::Postcard);
    }

    #[test]
    fn round_trip_bincode() {
        round_trip(SaveFormat::Bincode);
    }

    #[test]
    fn version_too_new() {
        let s = sample_state();
        // Hand-craft an envelope tagged with a version newer than ours.
        let env = VersionedSave {
            version: SAVE_VERSION + 1,
            payload: s.clone(),
        };
        let bytes = serde_json::to_vec(&env).expect("serialize envelope");
        let result = deserialize(&bytes, SaveFormat::Json);
        match result {
            Err(SaveError::VersionTooNew(v, c)) => {
                assert_eq!(v, SAVE_VERSION + 1);
                assert_eq!(c, SAVE_VERSION);
            }
            other => panic!("expected VersionTooNew, got {other:?}"),
        }
    }

    #[test]
    fn format_autodetect_json() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("game.save.json");
        save(&s, &path, SaveFormat::Json).expect("save");
        let loaded = load(&path).expect("load autodetect");
        let reencoded = serialize(&loaded, SaveFormat::Json).expect("re-serialize");
        let original = serialize(&s, SaveFormat::Json).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn format_autodetect_postcard() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("game.save.postcard");
        save(&s, &path, SaveFormat::Postcard).expect("save");
        let loaded = load(&path).expect("load autodetect");
        let reencoded = serialize(&loaded, SaveFormat::Postcard).expect("re-serialize");
        let original = serialize(&s, SaveFormat::Postcard).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn format_autodetect_bin() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("game.save.bin");
        save(&s, &path, SaveFormat::Postcard).expect("save");
        let loaded = load(&path).expect("load autodetect");
        let reencoded = serialize(&loaded, SaveFormat::Postcard).expect("re-serialize");
        let original = serialize(&s, SaveFormat::Postcard).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rng_preserved() {
        // A stream that is saved at draw N and resumed must continue
        // identically to a fresh never-serialized stream. We verify this by
        // comparing the full serialized envelope (which embeds the seed and
        // draw counter) before and after round-trip.
        let mut s = sample_state();
        // Advance the RNG a few draws.
        for _ in 0..5 {
            let _ = s.rng.next_u32();
        }
        let original = serialize(&s, SaveFormat::Json).expect("serialize");
        let loaded = deserialize(&original, SaveFormat::Json).expect("deserialize");
        let reencoded = serialize(&loaded, SaveFormat::Json).expect("re-serialize");
        assert_eq!(
            original, reencoded,
            "rng state not preserved across save/load"
        );
    }

    #[test]
    fn serde_json_byte_stable() {
        // Calling `serialize` twice must yield identical bytes (determinism).
        let s = sample_state();
        let a = serialize(&s, SaveFormat::Json).expect("serialize 1");
        let b = serialize(&s, SaveFormat::Json).expect("serialize 2");
        assert_eq!(a, b, "JSON serialization is not deterministic");
    }

    #[test]
    fn save_debug_writes_json() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("debug.save.json");
        save_debug(&s, &path).expect("save_debug");
        let loaded = load(&path).expect("load debug save");
        let reencoded = serialize(&loaded, SaveFormat::Json).expect("re-serialize");
        let original = serialize(&s, SaveFormat::Json).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn snapshot_game_state_json() {
        let state = sample_state();
        assert_json_snapshot!(state);
    }

    #[test]
    fn snapshot_game_event_rejected() {
        let event = GameEvent::Rejected {
            command: Command::EndTurn,
            reason: RejectReason::NotYourTurn,
        };
        assert_debug_snapshot!(event);
    }

    #[test]
    fn snapshot_game_event_income() {
        let event = GameEvent::Income {
            player: PlayerId(0),
            water: 5,
            wealth: 10,
            influence: 2,
        };
        assert_debug_snapshot!(event);
    }
}
