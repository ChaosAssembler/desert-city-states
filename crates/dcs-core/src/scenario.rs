//! Scenario configuration for Desert-City-States.
//!
//! This module owns the [`ScenarioConfig`] data model (the tunable knobs that
//! describe a single game: map size, player count, victory conditions, AI
//! behavior, and the various balance thresholds) along with the validation and
//! (de)serialization helpers used to load it from disk.
//!
//! Two enums live here because they are shared between the scenario config and
//! the player model: [`AiPersonality`] and [`Difficulty`]. The player model's
//! `PlayerKind::Ai` references both of them.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::VictoryKind;

// ---------------------------------------------------------------------------
// Enum types
// ---------------------------------------------------------------------------

/// The behavioral archetype assigned to an AI-controlled player.
///
/// These are scenario-config values (they describe *how* an AI should be
/// driven) and are also referenced by the player model's `PlayerKind::Ai`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AiPersonality {
    /// Expands territory aggressively, prioritizes claiming new tiles.
    #[default]
    Expansionist,
    /// Focuses on harassing and attacking rival players.
    Raider,
    /// Optimizes trade routes and economic throughput.
    Trader,
    /// Builds up defenses and holds a compact, fortified core.
    Fortifier,
}

/// Overall difficulty level of the simulation (used by AI and balance scaling).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Difficulty {
    #[default]
    Normal,
    Easy,
    Hard,
}

// ---------------------------------------------------------------------------
// ScenarioConfig
// ---------------------------------------------------------------------------

/// Tunable configuration describing a single game session.
///
/// `ScenarioConfig` is the single source of truth for map size, player count,
/// victory conditions, and balance thresholds. It is constructed either via
/// [`Default`] (full game), [`mvp_preset`] (minimal vertical-slice game), or
/// [`load`] (deserialized from a JSON file with defaults preserved for absent
/// fields).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScenarioConfig {
    /// Map radius in hexes. MVP uses 4 (= 61 tiles); full game 7-9 (= 169-271).
    pub map_radius: u8,
    /// Number of players, 2-4.
    pub player_count: u8,
    /// Maximum number of turns. MVP uses 30; full game 60.
    pub turn_limit: u32,
    /// AI personalities, one per non-human player. Length must equal
    /// `player_count - 1`.
    pub ai_personalities: Vec<AiPersonality>,
    /// Percentage of oases a player must control to win OasisDominance (V1).
    /// Default >= 50.
    pub oasis_majority_pct: u8,
    /// Wealth-score target needed to win WealthScore (V2). Default 200.
    pub wealth_score_target: u32,
    /// Number of relics to place (V3). If 0, derived from map size at world-gen.
    pub relic_count: u8,
    /// Number of consecutive turns a relic must be held to win RelicHold (V3).
    pub relic_hold_turns: u32,
    /// Victory conditions enabled for this game. Default: all three. MVP: only
    /// OasisDominance.
    pub victories_enabled: Vec<VictoryKind>,
    /// Mirror-placement toggle for symmetric starting positions.
    #[serde(default)]
    pub symmetry: bool,
    /// Deterministic seed, surfaced in the menu.
    #[serde(default)]
    pub seed: u64,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        ScenarioConfig {
            map_radius: 7,
            player_count: 4,
            turn_limit: 60,
            ai_personalities: vec![
                AiPersonality::Expansionist,
                AiPersonality::Raider,
                AiPersonality::Trader,
            ],
            oasis_majority_pct: 50,
            wealth_score_target: 200,
            relic_count: 2,
            relic_hold_turns: 10,
            symmetry: false,
            seed: 0,
            victories_enabled: vec![
                VictoryKind::OasisDominance,
                VictoryKind::WealthScore,
                VictoryKind::RelicHold,
            ],
        }
    }
}

/// Minimal "vertical slice" preset for Phase 1 development: a small, fast,
/// single-victory-condition game.
pub fn mvp_preset() -> ScenarioConfig {
    ScenarioConfig {
        map_radius: 4,
        player_count: 3,
        turn_limit: 30,
        ai_personalities: vec![AiPersonality::Expansionist, AiPersonality::Raider],
        oasis_majority_pct: 50,
        wealth_score_target: 200,
        relic_count: 1,
        relic_hold_turns: 6,
        symmetry: false,
        seed: 0,
        victories_enabled: vec![VictoryKind::OasisDominance],
    }
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Load a [`ScenarioConfig`] from a JSON file on disk.
///
/// `toml` is not a crate dependency, so configuration files are JSON (parsed
/// via `serde_json`, which *is* available). The on-disk format is a partial
/// override map: any field present in the file overwrites the corresponding
/// field on a base config, while absent fields keep their base value (including
/// `victories_enabled`, which therefore defaults to "all three" rather than an
/// empty list).
///
/// The base config is [`Default`] unless the file contains `"preset": "mvp"`,
/// in which case [`mvp_preset`] is used as the base.
///
/// After merging, [`scale_thresholds`] and [`validate`] are applied; any
/// validation error is returned.
pub fn load(path: &std::path::Path) -> Result<ScenarioConfig, ScenarioError> {
    let raw = std::fs::read_to_string(path).map_err(ScenarioError::Io)?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| ScenarioError::Parse(e.to_string()))?;

    let obj = value
        .as_object()
        .ok_or_else(|| ScenarioError::Parse("top-level JSON must be an object".into()))?;

    // Choose the base config. A `preset` key of "mvp" selects the MVP preset;
    // anything else (or absence) falls back to Default.
    let use_mvp = obj
        .get("preset")
        .and_then(|v| v.as_str())
        .map(|s| s == "mvp")
        .unwrap_or(false);
    let mut cfg = if use_mvp {
        mvp_preset()
    } else {
        Default::default()
    };

    // Helper: if the key is present, deserialize just that field and overwrite.
    let apply = |key: &str, setter: &mut dyn FnMut(serde_json::Value)| {
        if let Some(v) = obj.get(key) {
            setter(v.clone());
        }
    };

    apply("map_radius", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u8>(v) {
            cfg.map_radius = x;
        }
    });
    apply("player_count", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u8>(v) {
            cfg.player_count = x;
        }
    });
    apply("turn_limit", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u32>(v) {
            cfg.turn_limit = x;
        }
    });
    apply("ai_personalities", &mut |v| {
        if let Ok(x) = serde_json::from_value::<Vec<AiPersonality>>(v) {
            cfg.ai_personalities = x;
        }
    });
    apply("oasis_majority_pct", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u8>(v) {
            cfg.oasis_majority_pct = x;
        }
    });
    apply("wealth_score_target", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u32>(v) {
            cfg.wealth_score_target = x;
        }
    });
    apply("relic_count", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u8>(v) {
            cfg.relic_count = x;
        }
    });
    apply("relic_hold_turns", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u32>(v) {
            cfg.relic_hold_turns = x;
        }
    });
    apply("victories_enabled", &mut |v| {
        if let Ok(x) = serde_json::from_value::<Vec<VictoryKind>>(v) {
            cfg.victories_enabled = x;
        }
    });
    apply("symmetry", &mut |v| {
        if let Ok(x) = serde_json::from_value::<bool>(v) {
            cfg.symmetry = x;
        }
    });
    apply("seed", &mut |v| {
        if let Ok(x) = serde_json::from_value::<u64>(v) {
            cfg.seed = x;
        }
    });

    scale_thresholds(&mut cfg);
    validate(&cfg)?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Threshold scaling
// ---------------------------------------------------------------------------

/// Linearly scale balance thresholds based on map size and player count so that
/// smaller / lower-player games remain winnable in a reasonable number of turns.
pub fn scale_thresholds(cfg: &mut ScenarioConfig) {
    let size_factor = (cfg.map_radius as f32 / 7.0).clamp(0.5, 1.0);
    let pct_factor = (cfg.player_count as f32 / 4.0).clamp(0.5, 1.0);

    cfg.wealth_score_target = (200.0 * size_factor * pct_factor).round().max(50.0) as u32;
    cfg.relic_count = if cfg.map_radius < 6 || cfg.player_count <= 2 {
        1
    } else {
        2
    };
    cfg.relic_hold_turns = if cfg.map_radius < 6 { 6 } else { 10 };
    cfg.turn_limit = (cfg.turn_limit as f32 * lerp(size_factor, 1.0, 0.5)).max(20.0) as u32;
    // oasis_majority_pct is intentionally left untouched (stays >= 50).
}

/// Linear interpolation: `a` when `t == 0`, `b` when `t == 1`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Errors produced while loading, scaling, or validating a [`ScenarioConfig`].
#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("map_radius must be 4..=9")]
    BadRadius,
    #[error("player_count must be 2..=4")]
    BadPlayerCount,
    #[error("ai_personalities len ({0}) != player_count-1 ({1})")]
    PersonalityMismatch(usize, u8),
    #[error("oasis_majority_pct must be 50..=100")]
    BadMajority,
    #[error("relic_count exceeds available ruins budget for this map")]
    TooManyRelics,
    #[error("turn_limit must be > 0")]
    BadTurnLimit,
    #[error("victories_enabled must be non-empty")]
    NoVictories,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

/// Validate a [`ScenarioConfig`] against the hard constraints of the engine.
///
/// Returns [`Ok`] if every constraint holds, otherwise the first failing
/// [`ScenarioError`].
pub fn validate(cfg: &ScenarioConfig) -> Result<(), ScenarioError> {
    if !(4..=9).contains(&cfg.map_radius) {
        return Err(ScenarioError::BadRadius);
    }
    if !(2..=4).contains(&cfg.player_count) {
        return Err(ScenarioError::BadPlayerCount);
    }
    if cfg.ai_personalities.len() != (cfg.player_count - 1) as usize {
        return Err(ScenarioError::PersonalityMismatch(
            cfg.ai_personalities.len(),
            cfg.player_count,
        ));
    }
    if !(50..=100).contains(&cfg.oasis_majority_pct) {
        return Err(ScenarioError::BadMajority);
    }
    if cfg.turn_limit == 0 {
        return Err(ScenarioError::BadTurnLimit);
    }
    // world-gen creates a small fixed budget of ruins (~2-3); cap relic_count.
    if cfg.relic_count as usize > 3 {
        return Err(ScenarioError::TooManyRelics);
    }
    if cfg.victories_enabled.is_empty() {
        return Err(ScenarioError::NoVictories);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_validates_ok() {
        let cfg = ScenarioConfig::default();
        assert!(validate(&cfg).is_ok(), "Default should validate");
    }

    #[test]
    fn mvp_preset_validates_ok() {
        let cfg = mvp_preset();
        assert!(validate(&cfg).is_ok(), "MVP preset should validate");
    }

    #[test]
    fn scale_thresholds_small_game() {
        let mut cfg = ScenarioConfig {
            map_radius: 4,
            player_count: 3,
            turn_limit: 30,
            ai_personalities: vec![AiPersonality::Expansionist, AiPersonality::Raider],
            oasis_majority_pct: 50,
            wealth_score_target: 200,
            relic_count: 2,
            relic_hold_turns: 10,
            symmetry: false,
            seed: 0,
            victories_enabled: vec![VictoryKind::OasisDominance],
        };
        scale_thresholds(&mut cfg);
        assert_eq!(cfg.relic_count, 1);
        assert_eq!(cfg.relic_hold_turns, 6);
        assert!(cfg.wealth_score_target < 200);
        assert!(cfg.turn_limit < 60);
    }

    #[test]
    fn scale_thresholds_full_game_unchanged() {
        let mut cfg = ScenarioConfig::default();
        scale_thresholds(&mut cfg);
        assert_eq!(cfg.relic_count, 2);
        assert_eq!(cfg.relic_hold_turns, 10);
        assert_eq!(cfg.wealth_score_target, 200);
        assert_eq!(cfg.turn_limit, 60);
    }

    #[test]
    fn validate_rejects_player_count_one() {
        let cfg = ScenarioConfig {
            player_count: 1,
            ..Default::default()
        };
        let result = validate(&cfg);
        assert!(
            matches!(result, Err(ScenarioError::BadPlayerCount)),
            "expected BadPlayerCount, got {:?}",
            result
        );
    }

    #[test]
    fn validate_rejects_small_radius() {
        let cfg = ScenarioConfig {
            map_radius: 3,
            ..Default::default()
        };
        let result = validate(&cfg);
        assert!(
            matches!(result, Err(ScenarioError::BadRadius)),
            "expected BadRadius, got {:?}",
            result
        );
    }

    #[test]
    fn validate_rejects_personality_mismatch() {
        let cfg = ScenarioConfig {
            player_count: 3,
            ai_personalities: vec![AiPersonality::Expansionist],
            ..Default::default()
        };
        let result = validate(&cfg);
        assert!(
            matches!(result, Err(ScenarioError::PersonalityMismatch(1, 3))),
            "expected PersonalityMismatch(1, 3), got {:?}",
            result
        );
    }

    #[test]
    fn validate_rejects_bad_majority() {
        let cfg = ScenarioConfig {
            oasis_majority_pct: 40,
            ..Default::default()
        };
        let result = validate(&cfg);
        assert!(
            matches!(result, Err(ScenarioError::BadMajority)),
            "expected BadMajority, got {:?}",
            result
        );
    }

    #[test]
    fn validate_rejects_zero_turn_limit() {
        let cfg = ScenarioConfig {
            turn_limit: 0,
            ..Default::default()
        };
        let result = validate(&cfg);
        assert!(
            matches!(result, Err(ScenarioError::BadTurnLimit)),
            "expected BadTurnLimit, got {:?}",
            result
        );
    }

    #[test]
    fn load_round_trips_json() {
        // Use the full-game preset (radius 7, size_factor 1.0): scaling is then
        // the identity, so `load`'s documented scale_thresholds+validate pass
        // leaves every threshold unchanged. (mvp_preset would be rescaled down,
        // e.g. turn_limit 30 -> 23, which is correct behaviour, not a round-trip
        // bug, so it cannot be asserted here.)
        let cfg = ScenarioConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let file = std::env::temp_dir().join("dcs_test_scenario_load.json");
        std::fs::write(&file, &json).unwrap();

        let loaded = load(&file).expect("load should succeed");
        assert_eq!(loaded.map_radius, 7);
        assert_eq!(loaded.player_count, 4);
        // After scale_thresholds (identity at size_factor 1.0) the value is kept.
        assert_eq!(loaded.turn_limit, 60);
        assert_eq!(loaded.ai_personalities.len(), 3);
        assert_eq!(
            loaded.victories_enabled,
            vec![
                VictoryKind::OasisDominance,
                VictoryKind::WealthScore,
                VictoryKind::RelicHold,
            ]
        );

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn load_preserves_default_victories_when_absent() {
        // JSON with many fields but deliberately omitting `victories_enabled`.
        let json = r#"{
            "map_radius": 7,
            "player_count": 4,
            "turn_limit": 60,
            "ai_personalities": ["Expansionist", "Raider", "Trader"],
            "oasis_majority_pct": 50,
            "wealth_score_target": 200,
            "relic_count": 2,
            "relic_hold_turns": 10,
            "symmetry": false,
            "seed": 42
        }"#;
        let file = std::env::temp_dir().join("dcs_test_scenario_nodefault.json");
        std::fs::write(&file, json).unwrap();

        let loaded = load(&file).expect("load should succeed");
        // Absent victories_enabled must keep the Default (all three), not empty.
        assert_eq!(
            loaded.victories_enabled,
            vec![
                VictoryKind::OasisDominance,
                VictoryKind::WealthScore,
                VictoryKind::RelicHold,
            ]
        );
        assert_eq!(loaded.seed, 42);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn serde_round_trip_mvp_preset() {
        let cfg = mvp_preset();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ScenarioConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }
}
