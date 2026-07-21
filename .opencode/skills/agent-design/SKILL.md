---
name: agent-design
description: Use when creating or maintaining OpenCode agent files under .opencode/agents/ — design principles, file format, and single-responsibility guidelines
---

# Agent Design

## Rules

- One clearly defined purpose per agent (Single Responsibility Principle)
- Agents do not have permissions they do not need (least privilege)
- No overlap with already-defined agents — check existing agents first
- Constraints in agent files define scope only — no behavioral rules (those live in skills)
- No redundancy with skill files — if a rule is in the skill, don't repeat it in the agent
- A delegating specialist uses `mode: subagent` with narrowly-scoped `task` permission naming the specific agent. Never grant open `task: allow` to a subagent — that makes it an orchestrator.

## Workflow

1. Read `.opencode/agents/` directory to see what already exists
2. Identify the specific responsibility this agent will handle
3. Ensure no existing agent covers the same scope
4. Write the agent file with: role, constraints (scope only), skill references
5. Scope permissions precisely — deny by default, allow only what's needed
6. If the agent needs domain knowledge, reference the appropriate skill file
7. If the agent needs to delegate a subtask, scope `task` permission to the single required agent name — never grant open `task` access to a subagent
8. If creating a delegating specialist, also reference `delegation-guide` in addition to `subagent-autonomy`

## Conventions

- Agent files: `.opencode/agents/<agent-name>.md`
- Frontmatter: `description`, `mode`, `permission`
- Body structure: role description → skill loading → Constraints (scope) → Guidelines (quality, optional)
- Load `subagent-autonomy` skill at session start for subagents
- Constraints use imperative voice: "Never do X", "Only work within Y"
- Guidelines (if present) use imperative voice: "Focus on...", "If X, say so"

### Description field

The `description` is shown to the delegating agent. Write it to answer: "When should I delegate to this agent?" Include:
- What the agent does (its purpose)
- What it does NOT do (scope boundaries)
- Whether it is read-only or read-write

Examples:
- "Reviews Rust source under crates/ for correctness, convention adherence, and quality before merging. Read-only. Reports issues and never edits or builds."
- "Compiles, lints with clippy, and format-checks the Rust workspace, then reports results. Build and static-check only. Does not run the test suite, benchmarks, or write source."

### Communicating permissions in the body

Agents do not have the frontmatter in context. After the frontmatter, restate key permission restrictions in the body so the subagent understands its boundaries. Use these patterns:

- **Bash allow-list**: "Bash is allow-listed to: `mkdir *`, `ls *`. Any other command is blocked (deny-by-default)."
- **No bash access**: "No bash/shell access — do not attempt to run commands; rely only on read, glob, and grep."
- **Read-only**: "Read-only. Never attempt to edit or create files."
- **File scope**: "Only create and modify files under `docs/design/`"
