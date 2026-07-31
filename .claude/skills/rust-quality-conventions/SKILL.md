---
name: rust-quality-conventions
description: Use when writing or reviewing Rust code in this project — enforces quality standards for tests, style, documentation, validation, error handling, and performance
---

# Rust Quality Conventions

Quality conventions for the Desert City-States Rust codebase. These rules apply to all code written or reviewed in this project.

## Test Code

- **No duplicate test harness functions** — use `crate::test_harness` functions (`minimal_state()`, `state_with_cities()`, `create_player()`, `create_city()`, `create_unit()`, etc.)
- **Use `GameStateBuilder`** for tests needing custom state configurations
- **Integration tests** go in `crates/*/tests/`, not inline in source modules
- **No `make_game()` in individual test modules** — use the shared harness

## Code Style

- **Use `VecDeque`** for front-removal operations (not `Vec::remove(0)`)
- **Use `FxHashSet`** consistently (import from `fxhash`, not `std::collections::HashSet`)
- **No unnecessary `.clone()`** — prefer borrows or moves
- **Functions ≤50 lines** — extract helpers when larger

## Architecture & Trait Design

- **Prefer methods over free functions** — when a function's primary operand is a type you own, implement it as a method or associated function (`fn f(s: &GameState)` → `impl GameState { fn f(&self) }`). This improves discoverability via autocomplete and reduces import noise.
- **Orphan-rule workaround: extension traits** — for foreign types you cannot `impl` directly (e.g. types owned by another crate), define a trait in this crate and implement it for the foreign type instead of leaving scattered free functions. Example: `trait UnitKindExt { fn def(self) -> &'static UnitDef; }` implemented for `dcs_protocol::UnitKind`, then called as `kind.def()`.
- **Only implement traits with a real usage site** — avoid speculative `impl`s (e.g. `Sub`/`Neg` on a coordinate type, `Ord` on an enum used only for equality). Grep for actual call sites before adding a trait implementation or derive.

## Documentation

- **Public functions** must have `# Arguments`, `# Returns`, `# Panics` sections
- **Complex design decisions** need documentation comments

## Validation

- **Shared validation helpers** — no duplicate validation between `validate()` and `resolve_*`
- **Use `Result<T, RejectReason>`** for validation functions

## Error Handling

- **Don't implement `PartialEq`** on error types unless specifically required
- **Use `thiserror`** for library error types
- **Events for runtime warnings**, `Result` for recoverable failures

## Testing Requirements

- **Mathematical functions** need property-based tests (proptest)
- **Serialization changes** need snapshot tests (insta)
- **Performance-critical code** needs benchmarks (criterion)

## Performance

- **No `Vec::remove(0)` in hot paths**
- **`FxHashSet` for deterministic hashing**
