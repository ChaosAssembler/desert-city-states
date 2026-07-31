---
name: rust-workspace-management
description: Use when managing Rust workspace structure — crate creation, dependencies, Cargo.toml files, toolchain config, and directory layout
---

# Rust Workspace Management

## Rules

- Prefer CLI commands (`cargo add`, `cargo remove`) over hand-editing `Cargo.toml`
- Only edit `Cargo.toml` for: `[features]`, workspace metadata sections, removing members after `rm`
- Always verify dependency changes with `cargo tree`

## Workflow

1. Creating a crate: `cargo new --lib crates/<name>`, add to `[workspace.members]` if needed
2. Adding a dependency: `cargo add -p <crate> <dep>` (or `--workspace`), verify with `cargo tree`
3. Removing a dependency: `cargo remove -p <crate> <dep>`, check for orphaned features
4. Modifying toolchain/config: edit `rust-toolchain.toml` or `.cargo/config.toml` directly
5. Dependency hygiene: report duplicates, version mismatches, cycles using `cargo tree`

## Conventions

- Crate directory structure: `crates/<name>/`
- Use `--lib` flag when creating crates
- Workspace deps declared in root `Cargo.toml` with `--workspace`
