---
name: agent-design
description: Use when creating or maintaining OpenCode agent files under .opencode/agents/ — design principles, file format, and single-responsibility guidelines
---

# Agent Design

## Rules

- One clearly defined purpose per agent (Single Responsibility Principle)
- Agents do not have permissions they do not need (least privilege)
- No overlap with already-defined agents — check existing agents first
- Clear, concise language; no redundancies; no unclear statements
- No obvious explanations, no unneeded elaborations or examples
- Constraints in agent files define scope only — no behavioral rules (those live in skills)
- No redundancy with skill files — if a rule is in the skill, don't repeat it in the agent
- Use imperative voice in constraint sections (no subject)

## Workflow

1. Read `.opencode/agents/` directory to see what already exists
2. Identify the specific responsibility this agent will handle
3. Ensure no existing agent covers the same scope
4. Write the agent file with: role, constraints (scope only), skill references
5. Scope permissions precisely — deny by default, allow only what's needed
6. If the agent needs domain knowledge, reference the appropriate skill file

## Conventions

- Agent files: `.opencode/agents/<agent-name>.md`
- Frontmatter: `description`, `mode`, `permission`, `skill`
- Body structure: role description → skill loading → Constraints (scope) → Guidelines (quality, optional)
- Load `subagent-autonomy` skill at session start for subagents
- Constraints use imperative voice: "Never do X", "Only work within Y"
- Guidelines (if present) use imperative voice: "Focus on...", "If X, say so"
