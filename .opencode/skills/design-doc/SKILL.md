---
name: design-doc
description: Use when creating or maintaining game design documentation — subsystem designs, mechanics descriptions, balance parameters, and feature specs in docs/design/
---

# Design Documentation

## Rules

- Document design intent and player experience, not implementation details
- Each system gets its own document — don't merge unrelated mechanics
- Balance parameters should be explicit and easy to find (tables, not buried in prose)
- Link to ADRs for technical decisions that constrain or drive design choices

## Workflow

1. Identify the game system or feature to document
2. Read existing design docs to avoid duplication and ensure consistency
3. Choose the appropriate document type:
   - **Subsystem design** — mechanics, rules, interactions, edge cases
   - **Feature spec** — player-facing behavior, user flows, acceptance criteria
   - **Balance doc** — numerical parameters, scaling curves, tuning rationale
   - **System overview** — how multiple subsystems connect, data flow between systems
4. Write the document following conventions below
5. Cross-reference related design docs, architecture docs, and ADRs

## Document Types

- **Subsystem design:** Purpose → Rules → Interactions → Edge cases → Open questions
- **Feature spec:** User story → Behavior → Acceptance criteria → Out of scope
- **Balance doc:** Parameters table → Scaling rules → Tuning rationale → Targets
- **System overview:** Components → Responsibilities → Data flow → Dependencies

## Conventions

- Design docs live in `docs/design/`
- Use tables for balance parameters and numerical values
- Include the current status: Draft | Active | Superseded
- Cross-link related docs: `See [Combat System](combat.md)`
- One topic per document — split if a document covers multiple systems
