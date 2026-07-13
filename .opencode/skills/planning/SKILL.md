---
name: planning
description: Use when creating or maintaining planning documents in docs/planning/
---

# Planning Documentation

## Rules

- Never delete completed items — mark them done instead
- Document the *why* behind priority changes or deferrals
- Keep planning documents actionable — if an item has been sitting in "planned" across multiple reviews, either scope it down or defer it
- Cross-reference ADRs, design docs, and specs when items are driven by specific decisions or requirements
- Maintain only one active phase at a time in roadmap-level documents

## Workflow

1. Identify the planning document to create or update
2. Assess current state — what's changed since the last update
3. Add, update, or reprioritize items based on current project status
4. Cross-reference related documents (ADRs, design docs, specs)
5. Write all changes to the document
6. Review for accuracy against current project status and recent decisions

## Document Types

### Roadmap
Master project plan with phases, milestones, and items.

- **Structure:** Phases → Milestones → Items
- **File:** `docs/planning/roadmap.md`
- **When to use:** High-level project planning and milestone tracking

### Milestone Plan
Detailed breakdown of a specific milestone from the roadmap.

- **Structure:** Objective → Tasks → Acceptance criteria → Dependencies
- **File:** `docs/planning/milestone-<name>.md`
- **When to use:** When a milestone is complex enough to warrant its own document

### Retrospective
Review of what worked, what didn't, and what to change.

- **Structure:** What went well → What didn't → Action items
- **File:** `docs/planning/retro-<date>.md`
- **When to use:** At the end of a phase or milestone

## Conventions

### Roadmap Format

Roadmap document structure:

```markdown
# Project Roadmap

## Phase: MVP
Target: 2026-Q3

### Milestone: Core Mechanics
- [~] Basic hex movement [in progress]
- [ ] Turn-based resolution
- [x] Map generation [done]

### Milestone: Combat System
- [ ] Unit types and stats
- [ ] Combat resolution

---

## Phase: v1.0
Target: 2027-Q1

### Milestone: Trade Routes
- [ ] Caravan system
- [ ] Route management

---
```

### Status Indicators

- `- [ ]` planned
- `- [~]` in progress
- `- [x]` done
- `- [-]` deferred

### Naming Conventions

- Phase headers: `## Phase: [Name]`
- Milestone headers: `### Milestone: [Name]`
- Milestone documents: `milestone-<kebab-name>.md`
- Retrospectives: `retro-YYYY-MM-DD.md`

### Cross-References

Reference related documents using relative paths:
- ADRs: `See [ADR-0001](../architecture/decisions/ADR-0001-title.md)`
- Design docs: `See [Feature: Combat](../design/Combat.md)`
- Specs: `See [Spec: Movement](../specs/movement.md)`
