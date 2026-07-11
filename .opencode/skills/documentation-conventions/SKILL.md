---
name: documentation-conventions
description: Use when writing or updating documentation, changelogs, module docs, or public API rustdoc comments
---

# Documentation Conventions

## Rules

- Document the *why*, not the *what* — code shows what was built, docs explain why
- Don't document obvious code or comments that restate what code already says
- Public APIs must have parameter and return type documentation

## Workflow

1. Identify whether a change needs documentation updates
2. For features: update README, changelog, and relevant module docs
3. For public APIs: add rustdoc comments covering arguments, returns, errors, examples
4. Review documentation for accuracy against the implementation

## Conventions

- Changelog format: `## [version] - date` with Added/Changed/Fixed sections
- Comment the *why* (non-obvious intent, constraints), not the *what* (restating code)
- Cross-reference ADRs when documenting architectural decisions (see `adr` skill for ADR format)
