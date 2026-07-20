//! Save / load serialization for [`crate::GameState`].
//!
//! This module defines the [`SaveFormat`] and [`SaveError`] types used by the
//! serialization methods on [`crate::GameState`]. The actual implementations live on
//! `GameState` itself (see [`crate::model::GameState`]).
//!
//! # Envelope strategy (ADR-0007)
//!
//! We serialize the [`crate::VersionedSave<crate::GameState>`] envelope rather than the raw
//! [`crate::GameState`]. The envelope carries the schema `version` that gates
//! migrations on load; `GameState` independently mirrors `SAVE_VERSION` for
//! in-payload migration bookkeeping. The current `SAVE_VERSION` is `1`, so the
//! migration loop is forward-proof but currently a no-op.

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
#[derive(Debug, thiserror::Error)]
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameState;
    use crate::scenario::ScenarioConfig;
    use crate::{Command, GameEvent, PlayerId, RejectReason, SAVE_VERSION, VersionedSave};
    use insta::assert_debug_snapshot;
    use insta::assert_json_snapshot;

    /// Build a small but non-trivial `GameState` to exercise the (de)serialize
    /// paths, including the fxhash serde helpers and the `SeededRng`.
    fn sample_state() -> GameState {
        let mut s = GameState::new(ScenarioConfig::mvp_preset(), 12345);
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
        let original = s.serialize_to(fmt).expect("serialize");
        let loaded = GameState::from_bytes(&original, fmt).expect("deserialize");
        let reencoded = loaded.serialize_to(fmt).expect("re-serialize");
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
        let result = GameState::from_bytes(&bytes, SaveFormat::Json);
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
        s.save(&path, SaveFormat::Json).expect("save");
        let loaded = GameState::load(&path).expect("load autodetect");
        let reencoded = loaded.serialize_to(SaveFormat::Json).expect("re-serialize");
        let original = s.serialize_to(SaveFormat::Json).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn format_autodetect_postcard() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("game.save.postcard");
        s.save(&path, SaveFormat::Postcard).expect("save");
        let loaded = GameState::load(&path).expect("load autodetect");
        let reencoded = loaded
            .serialize_to(SaveFormat::Postcard)
            .expect("re-serialize");
        let original = s.serialize_to(SaveFormat::Postcard).expect("serialize");
        assert_eq!(original, reencoded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn format_autodetect_bin() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("game.save.bin");
        s.save(&path, SaveFormat::Postcard).expect("save");
        let loaded = GameState::load(&path).expect("load autodetect");
        let reencoded = loaded
            .serialize_to(SaveFormat::Postcard)
            .expect("re-serialize");
        let original = s.serialize_to(SaveFormat::Postcard).expect("serialize");
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
        let original = s.serialize_to(SaveFormat::Json).expect("serialize");
        let loaded = GameState::from_bytes(&original, SaveFormat::Json).expect("deserialize");
        let reencoded = loaded.serialize_to(SaveFormat::Json).expect("re-serialize");
        assert_eq!(
            original, reencoded,
            "rng state not preserved across save/load"
        );
    }

    #[test]
    fn serde_json_byte_stable() {
        // Calling `serialize_to` twice must yield identical bytes (determinism).
        let s = sample_state();
        let a = s.serialize_to(SaveFormat::Json).expect("serialize 1");
        let b = s.serialize_to(SaveFormat::Json).expect("serialize 2");
        assert_eq!(a, b, "JSON serialization is not deterministic");
    }

    #[test]
    fn save_debug_writes_json() {
        let s = sample_state();
        let dir = std::env::temp_dir().join("dcs_serialize_tests");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("debug.save.json");
        s.save_debug(&path).expect("save_debug");
        let loaded = GameState::load(&path).expect("load debug save");
        let reencoded = loaded.serialize_to(SaveFormat::Json).expect("re-serialize");
        let original = s.serialize_to(SaveFormat::Json).expect("serialize");
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
