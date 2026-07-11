---
name: documentation-and-adrs
description: Use when making architectural decisions, shipping features, or when you need to record context that future engineers and agents will need to understand
---

# Documentation and ADRs

## Rules

- Document the *why*, not the *what* — code shows what was built, docs explain why
- Never delete old ADRs — write a new one that supersedes it
- Don't document obvious code or comments that restate what code already says
- Public APIs must have parameter and return type documentation

## Workflow

1. Identify whether a decision, feature, or change needs documentation
2. For architectural decisions: write an ADR in `docs/decisions/` using the template
3. For features: update README, changelog, and relevant module docs
4. For public APIs: add rustdoc comments covering arguments, returns, errors, examples
5. Review documentation for accuracy against the implementation

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
- Changelog format: `## [version] - date` with Added/Changed/Fixed sections
- Comment the *why* (non-obvious intent, constraints), not the *what* (restating code)
