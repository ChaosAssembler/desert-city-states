//! `dcs-core`: engine-free game core (rules, state, simulation).
//!
//! This crate intentionally has no rendering, no input, and no app-loop
//! concerns. It depends only on `dcs-protocol` (the shared `Command` /
//! `GameEvent` / `VersionedSave` contract surface). Game logic is added
//! in later phases.
