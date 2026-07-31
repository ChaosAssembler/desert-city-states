# Desert City-States

Small-scale 4X strategy game: rival oasis city-states on a hex map in a harsh desert (water scarcity + trade-route control as the core tension). Rust, 5-crate Cargo workspace, macroquad 2D renderer, turn-based.

## Workspace layout

| Crate | Role | Depends on |
|---|---|---|
| `dcs-core` | Pure, deterministic, serializable simulation — all game logic, zero rendering | `dcs-protocol` only |
| `dcs-protocol` | Save-file schema versioning + command/event envelopes | — |
| `dcs-render` | macroquad presentation layer (reads state, emits `Command`s, never mutates) | `dcs-core`, `dcs-protocol` |
| `dcs-app` | Main entry point, main-loop glue, input → command dispatch | all of the above |
| `dcs-mcp` | MCP server for headless play-testing (JSON-line protocol over stdio) | `dcs-app`/`dcs-core` |

## Hard architectural rules (non-negotiable — see `docs/architecture/ARCHITECTURE.md` §1)

- **`dcs-core` must stay engine-free.** It may never depend on `macroquad`, `dcs-render`, or `dcs-app`. Enforced by `mise run purity` and CI. Determinism (same `GameState` + same ordered `Command`s + same seed → bit-identical result) depends on this.
- **`dcs-render` never mutates `GameState`.** It only reads state and emits `Command`s; the only `&mut GameState` in the program lives in `dcs-app` and is touched exclusively by `core::step`/`core::advance_turn`.
- **No ECS.** Plain data + functions over a single `GameState` aggregate (ADR-0002) — deliberate, don't reintroduce one.
- Camera/selection/UI state is **ephemeral**, never serialized into saves.

Full design intent for the renderer specifically: `docs/specs/presentation-rendering-ui.md`. `dcs-render` is currently a stub (clears the screen only) — real drawing/input logic is the active work.

## Common commands (via `mise run <task>`, see `mise.toml`)

- `mise run build` — `cargo build --workspace`
- `mise run test` — `cargo test --workspace`
- `mise run fmt` — `cargo fmt --all`
- `mise run check` — clippy, warnings as errors
- `mise run purity` — fails if `dcs-core` pulls in `macroquad`/`dcs-render`/`dcs-app`
- `mise run lint-md` — markdownlint over `docs/**/*.md`

CI (`.github/workflows/ci.yml`) runs build → purity gate → fmt check → clippy → test → doc lint, in that order. Match this locally before considering work done.

## Docs

- `docs/design/Desert-City-States.md` — game design (source of truth for behavior/balance)
- `docs/architecture/ARCHITECTURE.md` — as-built software architecture
- `docs/architecture/decisions/` — ADRs
- `docs/specs/` — per-system implementation specs (one per gameplay/foundation/presentation system)
- `docs/specs/presentation-agent-protocol.md` — the `dcs-mcp` JSON protocol (used by both the `game-tester` skill and any Playwright-driven browser testing)

## MCP servers (`.mcp.json`)

- `dcs` — the project's own headless play-testing server (`cargo run -p dcs-mcp`). Use for fast, deterministic protocol-level testing without a browser.
- `playwright` — browser automation (Firefox) for testing the WASM build end-to-end once it exists.

## Prior agentic tooling (`.opencode/`)

This project was previously developed with OpenCode, which used a 24-agent delegation hierarchy with a fine-grained permission DSL — largely a guardrail structure for weaker/free models via narrow single-purpose roles. That hierarchy is **not** ported to Claude Code and shouldn't be rebuilt as-is; broader-scoped work generally doesn't need it here. `.opencode/` itself is left untouched as OpenCode may still be used alongside Claude Code.

The project-*knowledge* skills (as opposed to the agent-system-design ones) were verified against the actual repo and ported to `.claude/skills/`: `web-deployment`, `rust-quality-conventions`, `rust-workspace-management`, `game-tester`, `adr`, `architecture-doc`, `design-doc`, `planning`, `doc-consistency`. Deliberately **not** ported:

- `agent-design`, `agentic-system-conventions`, `skill-design`, `subagent-autonomy`, `delegation-guide` — about designing OpenCode's own agent/skill system, not the game.
- `review-reporting` — would conflict with Claude Code's native `ReportFindings` tool, which has its own schema for the same purpose.
- `mise-management` — generic `mise` CLI reference, no project-specific content.
- `git-committing` — duplicates Claude Code's built-in git-commit conventions (no secrets, no `-A`, imperative intent-focused messages).
- `spec-driven-development` — its template doesn't match the spec format actually used in `docs/specs/*.md` (Purpose/Scope/Responsibilities/Data Structures/Key Functions/Algorithms/Edge Cases/Acceptance Criteria/References/Open Questions); looks stale/generic rather than real practice.
- `incremental-implementation` — generic vertical-slicing advice, not project-specific.
