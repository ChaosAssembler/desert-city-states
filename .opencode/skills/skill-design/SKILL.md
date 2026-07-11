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

## Workflow

1. Read `.opencode/skills/` directory to see existing skill directories
2. Confirm no existing skill covers the same area
3. Create directory: `.opencode/skills/<skill-name>/`
4. Write `SKILL.md` with correct frontmatter and focused instructions

## Conventions

- Skill files: `.opencode/skills/<skill-name>/SKILL.md`
- Directory name becomes the skill name
- Description: short, front-loaded with trigger keywords, "Use when..." phrasing
- Instructions: focused and actionable, no background or fluff
- Self-contained — no dependencies on other skill files
