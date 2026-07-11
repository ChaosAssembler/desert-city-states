---
name: skill-design
description: Use when creating or maintaining OpenCode skill files under .opencode/skills/ — file format, frontmatter, and design principles for standalone skill knowledge
---

# Skill Design

## Rules

- One clearly defined area per skill (Single Responsibility)
- Required frontmatter: `name` (must match directory name), `description` ("Use when..." phrasing)
- Skills are discovered automatically — no registration step needed
- Check existing skills before creating new ones to avoid duplicates
- Skills contain agent-agnostic instructions — no mention of "specialist", "agent", or "delegated to"
- No "ask the user" instructions — these are agent behavior, not methodology
- No scope constraints — "only work within X" belongs in the agent file
- Rules section uses imperative voice (no subject)

## Workflow

1. Read `.opencode/skills/` directory to see existing skill directories
2. Confirm no existing skill covers the same area
3. Create directory: `.opencode/skills/<skill-name>/`
4. Write `SKILL.md` with correct frontmatter and focused instructions

## Conventions

- Skill files: `.opencode/skills/<skill-name>/SKILL.md`
- Directory name becomes the skill name
- Description: short, front-loaded with trigger keywords, "Use when..." phrasing
- Body structure: Rules (methodology) → Workflow (procedures) → Conventions (formatting)
- Instructions: focused and actionable, no background or fluff
- Self-contained — no dependencies on other skill files
