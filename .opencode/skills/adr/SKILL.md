---
name: adr
description: Use when creating, maintaining, or referencing Architecture Decision Records (ADRs) in docs/decisions/
---

# ADR Guide

## Rules

- Never delete old ADRs — write a new one that supersedes it
- Document the *why* behind the decision, not just the *what*

## Workflow

1. Identify whether a decision needs an ADR
2. Write the ADR in `docs/decisions/` using the template below
3. Assign sequential numbering: `ADR-001-title.md`, `ADR-002-title.md`, etc.
4. Review for accuracy against the implementation

## ADR Template

```markdown
# ADR-NNN: [Title]

## Status
Accepted | Superseded by ADR-XXX | Deprecated

## Date
YYYY-MM-DD

## Context
[Why is this decision needed?]

## Decision
[What and why.]

## Alternatives
[Options and why rejected.]

## Consequences
[Trade-offs.]
```

## Conventions

- ADRs live in `docs/decisions/` with sequential numbering: `ADR-001-title.md`
- ADR lifecycle: Proposed → Accepted → (Superseded or Deprecated)
