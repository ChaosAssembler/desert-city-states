---
name: architecture-doc
description: Use when generating architecture documentation — module docs, system descriptions, data flow, and technical overviews for the docs/ directory
---

# Architecture Documentation

## Rules

- Document structure as it exists, not as it was intended to be
- Use concrete file paths and type names — no vague "the module" references
- One document per architectural concern — don't merge unrelated systems
- Keep diagrams text-based (Mermaid, ASCII) for version control friendliness
- Link to source files and ADRs rather than duplicating their content

## Workflow

1. Explore the codebase to understand actual structure and relationships
2. Identify the architectural concern to document (module, system, data flow)
3. Choose the appropriate document type:
   - **Module overview** — purpose, public API, key types, dependencies
   - **System description** — how components interact, event flow, state management
   - **Data flow** — how data moves through the system, transformations, storage
   - **Technical overview** — high-level architecture for onboarding
4. Write the document following conventions below
5. Cross-reference related documents and ADRs

## Document Types

- **Module overview:** Purpose → Key types → Public API → Dependencies → Usage
- **System description:** Components → Interactions → Events → State transitions
- **Data flow:** Inputs → Transformations → Outputs → Storage
- **Technical overview:** Architecture → Crate structure → Key decisions → Entry points

## Conventions

- Architecture docs live in `docs/architecture/`
- Use Mermaid diagrams for visual representations when helpful
- Include file paths in code references: `crates/game/src/map/hex.rs`
- Cross-link related ADRs: `See [ADR-001](../decisions/ADR-001-title.md)`
- One topic per document — split if a document covers multiple concerns
