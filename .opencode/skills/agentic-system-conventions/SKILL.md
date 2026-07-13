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

- **Orchestrators** (plan, build) delegate broadly to specialists via the task tool. They do not modify files directly. They load `delegation-guide` and have unrestricted `task` permission.
- **Specialists** execute within their scoped domain. Each loads `subagent-autonomy` and its domain skill. Most specialists do not delegate further.
- **Delegating specialists** are specialists that also delegate one specific subtask to a dedicated sub-subagent. They have narrowly-scoped `task` permission (allowing only the specific agent they need) and load both `subagent-autonomy` and `delegation-guide`. This is not orchestration — it is limited delegation for a single capability the specialist does not own.
- **Consultant** loads skills on demand to provide expert guidance on the correct approach. Orchestrators consult it frequently.
- **Permissions** are deny-by-default with per-skill allow rules. Each agent only accesses the skills it needs.

## Delegation Depth

Delegation is bounded: an orchestrator may delegate freely, a delegating specialist may delegate one narrow subtask, and a regular specialist does not delegate at all. This keeps delegation chains short (maximum depth of 2) and traceable.

Rules for delegating specialists:
- The `task` permission must name the specific agent(s) allowed — never `task: allow` with an unrestricted allowlist.
- The specialist's own domain skill must describe the delegation as part of its process (e.g., "delegate URL fetching to web-fetcher").
- A delegating specialist still loads `subagent-autonomy` — it remains a specialist subject to autonomy principles, not an orchestrator.

## Voice Convention

Constraint sections use imperative voice (no subject):
- ✓ "Never write Rust source code in `src/`"
- ✓ "Only create and modify files under `.opencode/agents/`"
- ✗ "You do NOT write Rust source code" (second person)
- ✗ "Never writes Rust source code" (third person)

## No Redundancy

If a rule exists in a skill file, don't repeat it in the agent constraints. Agent constraints define scope. Skill rules define methodology. Duplication creates maintenance burden and confusion.
- Agent and skill files must stay generic: reference project conventions and docs rather than duplicating their specific rules or examples. Link to the source doc instead of restating its contents.
