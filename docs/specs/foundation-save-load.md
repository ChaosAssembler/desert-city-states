# Foundation Spec: Save / Load

> **Phase:** 1 — Per-system foundation specs
> **Crate:** `dcs-core` (module `dcs-core::serialize`) + `dcs-protocol` (`VersionedSave<T>`)
> **Status:** Draft for review
> **Implements:** DD §5.4 (reproducibility), ARCH §7; ADR-0007 (serde + version envelope)

---

## 1. Purpose

Define the save/load contract: the `VersionedSave<T>` envelope, the serde
abstraction supporting `serde_json` (dev/debug) and `postcard`/`bincode` (ship),
the file API, and the forward-compat migration strategy. Because the seeded RNG
state lives inside `GameState` (ADR-0006), a load resumes **deterministically**
and save == replay (ARCH §6, ADR-0007).

## 2. Scope

**In scope:** `VersionedSave<T>` envelope (in `dcs-protocol`); `SaveFormat`
enum + `serialize`/`deserialize` abstraction; `save`/`load` file API;
migration registry for version bumps; RNG-state-in-state reproducibility note.

**Out of scope:** the `GameState` shape itself (core-data-model spec); combat
tuning; render/app state (never saved — ARCH §7).

## 3. Responsibilities

- Serialize the **entire `GameState`** (core only) through `serde`.
- Wrap payload in a version envelope so old saves can migrate.
- Support pluggable formats behind one API (debug json, ship postcard/bincode).
- Reject saves newer than `SAVE_VERSION`.
- Keep render/app state (camera, UI) entirely out of the file.

## 4. Core Data Structures

```rust
// ---- dcs-protocol ----
/// Versioned save envelope. The save contract lives here, not in core (ADR-0003, ADR-0007).
#[derive(Serialize, Deserialize)]
pub struct VersionedSave<T> {
    pub version: u32,
    pub payload: T,
}

/// Current schema version. Bump on breaking changes; add a migration (§6).
pub const SAVE_VERSION: u32 = 1;

// ---- dcs-core::serialize ----
#[derive(Clone, Copy, Debug)]
pub enum SaveFormat { Json, Postcard, Bincode }

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("unsupported save version {0} (current {1})")]
    VersionTooNew(u32, u32),
    #[error("migration from {0} failed: {1}")]
    MigrationFailed(u32, String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(String),
}
```

`GameState` is the `T` in `VersionedSave<GameState>`; it carries `version` too
(mirrors `SAVE_VERSION`) for redundancy checks.

## 5. Key Functions / API

```rust
/// Serialize a state to bytes in the chosen format, wrapped in VersionedSave.
pub fn serialize(state: &GameState, fmt: SaveFormat) -> Result<Vec<u8>, SaveError>;

/// Deserialize bytes -> GameState, running migrations if version < SAVE_VERSION,
/// and rejecting version > SAVE_VERSION.
pub fn deserialize(bytes: &[u8], fmt: SaveFormat) -> Result<GameState, SaveError>;

/// Write a save file (format inferred from extension or explicit arg).
pub fn save(state: &GameState, path: &Path, fmt: SaveFormat) -> Result<(), SaveError>;

/// Load a save file; auto-detect format from extension; migrate as needed.
pub fn load(path: &Path) -> Result<GameState, SaveError>;

/// Convenience: human-readable dev save.
pub fn save_debug(state: &GameState, path: &Path) -> Result<(), SaveError> { save(state, path, SaveFormat::Json) }
```

**Format mapping:**
- `Json` → `serde_json::{to_vec, from_slice}` (dev/debug, human-readable).
- `Postcard` → `postcard::{to_stdvec, from_bytes}` (compact ship default).
- `Bincode` → `bincode::{serialize, deserialize}` (alternative ship format).

## 6. Algorithm — migration strategy

`GameState::version` (and `VersionedSave::version`) tracks schema. On
`deserialize`:

```rust
let env: VersionedSave<...> = match fmt {
    Json    => serde_json::from_slice(bytes)?,
    Postcard=> postcard::from_bytes(bytes)?,
    Bincode => bincode::deserialize(bytes)?,
};
if env.version > SAVE_VERSION {
    return Err(SaveError::VersionTooNew(env.version, SAVE_VERSION));
}
let mut payload = env.payload;
while payload.version < SAVE_VERSION {
    payload = migrate(payload.version, payload)?;   // stepwise, one version at a time
}
Ok(payload)
```

- **`migrate(from, payload)`** is a registry of `(version -> version+1)`
  transforms. Each adds defaulted fields, renames, or recomputes derived data.
  Breaking vs non-breaking changes are documented per migration in a changelog.
- **Non-breaking** (e.g., adding an `Option` field with `#[serde(default)]`):
  often needs no code migration if the field deserializes with a default.
- **Breaking** (e.g., removing/renaming a field, changing an enum): requires an
  explicit `migrate` step.
- Migrations must be **deterministic** (no RNG) so a migrated save equals a
  re-serialized current-state save.

### 6.1 Reproducibility Guarantee

`GameState.rng` (ADR-0006) is serialized with everything else. Therefore:

- **Save = serialize `GameState`.** Load resumes the *exact* same RNG sequence.
- **Replay = re-issue the saved `Command` log** against `new_game(scenario,
  seed)`; identical end state (turn-engine spec §8). Save and replay share the
  same purity guarantee (ARCH §6, ADR-0007).

Render/app state (camera, selection, UI panels) is **never** part of the file —
it is ephemeral and rebuilt on load (ARCH §7).

## 7. Edge Cases / Invariants

- **Invariant:** a save written by version `V` loaded by version `V` round-trips
  bit-for-bit in `payload` (modulo format canonicalization for json).
- **Invariant:** `env.version > SAVE_VERSION` ⇒ hard reject (never attempt
  forward-migration — we can't know the future schema).
- `serde` must use `#[serde(default)]` on newly added `Option`/scalar fields so
  old saves without them still deserialize (reduces migration burden).
- `tile_index`/`VictoryTracker` use `FxHashMap`/`FxHashSet` (fixed iteration
  order, ADR-0006) so json output is stable across platforms.
- File extension → format: `.json`→Json, `.postcard`/`.bin`→Postcard (or
  `.bincode`→Bincode). Unknown → default to Postcard for ship.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] `save`→`load` round-trip yields a `GameState` deep-equal to the original (all fields incl. `rng` state).
- [ ] Json and Postcard formats produce loadable saves for the same state.
- [ ] Loading a save with `version > SAVE_VERSION` returns `VersionTooNew`.
- [ ] A simulated version-1 → version-2 migration yields a payload whose re-save at v2 equals a native v2 save (deterministic migration).
- [ ] RNG state preserved: after load, applying the same `Command` sequence produces identical events as before save.
- [ ] Render/app fields absent from serialized output (only core `GameState`).
- [ ] `serde_json` output for a fixed seeded game is byte-stable across runs (FxHashMap order).
- [ ] `load` auto-detects format from extension.

## 9. References

- Design: DD §5.4 (determinism / shareable maps).
- Architecture: ARCH §7 (save/load), §6 (replay == save), §16 (errors/logging).
- ADRs: ADR-0007 (serde + version envelope), ADR-0003 (envelope in
  `dcs-protocol`, core stays clean), ADR-0006 (RNG state in `GameState`).
- Related specs: `foundation-core-data-model.md` (`GameState`, `SeededRng`),
  `foundation-turn-engine.md` (`Command` log replay), `foundation-scenario-config.md`
  (`scenario` is a `GameState` field, not separately versioned).

## 10. Open Questions (carried)

- **ARCH OQ-5 / OQ-6:** Final ship format (postcard vs bincode) and PRNG crate
  (`nanorand` vs `rand::StdRng`) are recommended-but-open; both are abstracted
  here so the choice is a one-line swap. Resolve at implementation.
- **Compression:** not specified; postcard/bincode are already compact. Optional
  zstd layer is a later decision, behind the `SaveFormat` API.
