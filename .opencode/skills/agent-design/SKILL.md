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

## Workflow

1. Read `.opencode/agents/` directory to see what already exists
2. Identify the specific responsibility this agent will handle
3. Ensure no existing agent covers the same scope
4. Write the agent file with: role, capabilities, constraints, key procedures
5. Scope permissions precisely — deny by default, allow only what's needed

## Conventions

- Agent files: `.opencode/agents/<agent-name>.md`
- Frontmatter: `name`, `description`, `mode`, `permission`
- Body: concise role description, constraints, key procedures
- Load `subagent-autonomy` skill at session start for subagents
