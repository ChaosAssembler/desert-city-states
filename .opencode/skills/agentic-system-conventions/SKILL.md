---
name: agentic-system-conventions
description: Use when designing or maintaining agents and skills — defines the system taxonomy, architecture, and design conventions
---

# Agentic System Conventions

## Taxonomy

Three types of instructions, each with a designated location:

- **Constraints** (agent files) — hard scope limits. WHERE you operate, what you touch and don't touch. Non-negotiable boundaries.
- **Rules** (skill files) — behavioral methodology. HOW to approach work correctly. The right way to do things within your scope.
- **Guidelines** (agent files) — soft quality recommendations. HOW WELL you produce output. Not about scope or methodology, but about quality.

## What Goes Where

**Agent files** contain:
- Identity and role description
- Scope constraints (what you can and can't do)
- Skill references (which skills to load)
- Guidelines (quality recommendations, if needed)

**Skill files** contain:
- Methodology rules (how to approach work correctly)
- Workflows (step-by-step procedures)
- Conventions (formatting, naming, patterns)

**Never in skill files:**
- "Ask the user" instructions — these are agent behavior, not methodology
- Scope constraints — "only work within X" or "never touch Y" belongs in the agent
- References to specific agents ("the specialist", "the workspace architect") — skills are agent-agnostic

## Agent File Structure

```
---
description: <what this agent does>
mode: <primary|subagent>
permission: <scoped tools>
skill: <allowed skills>
---

<Role description>

<Skill loading instructions>

## Constraints
<Scope limits — imperative voice, no subject>

## Guidelines
<Quality recommendations — optional>
```

## Skill File Structure

```
---
name: <skill-name>
description: <"Use when..." phrasing>
---

# <Topic>

## Rules
<Behavioral methodology — imperative voice>

## Workflow
<Step-by-step procedures>

## Conventions
<Formatting, naming, patterns>
```

## Architecture

- **Orchestrators** (plan, build) delegate to specialists via the task tool. They do not modify files directly.
- **Specialists** execute within their scoped domain. Each loads `subagent-autonomy` and its domain skill.
- **Consultant** loads skills on demand to provide expert guidance on the correct approach. Orchestrators consult it frequently.
- **Permissions** are deny-by-default with per-skill allow rules. Each agent only accesses the skills it needs.

## Voice Convention

Constraint sections use imperative voice (no subject):
- ✓ "Never write Rust source code in `src/`"
- ✓ "Only create and modify files under `.opencode/agents/`"
- ✗ "You do NOT write Rust source code" (second person)
- ✗ "Never writes Rust source code" (third person)

## No Redundancy

If a rule exists in a skill file, don't repeat it in the agent constraints. Agent constraints define scope. Skill rules define methodology. Duplication creates maintenance burden and confusion.
