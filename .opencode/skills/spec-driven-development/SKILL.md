---
name: spec-driven-development
description: Use when starting a new feature or significant change and no specification exists yet — forces structured specs before code
---

# Spec-Driven Development

## Rules

- Never start implementation without a written spec
- Surface assumptions explicitly before writing spec content
- Reframe vague requirements as concrete, testable success criteria
- Do not advance to the next phase until the current one is validated

## Workflow

1. Identify what is being built and why
2. List assumptions being made, present them for confirmation
3. Write a spec covering six areas:
   - **Objective** — what and why, who is the user, what does success look like
   - **Commands** — full executable commands with flags
   - **Project Structure** — where source, tests, and docs live
   - **Code Style** — one real code snippet showing conventions
   - **Testing Strategy** — framework, locations, coverage expectations
   - **Boundaries** — Always do / Ask first / Never do
4. Validate the spec before proceeding
5. Break the spec into a technical plan with ordered tasks
6. Each task gets explicit acceptance criteria and a verification step
7. Implement tasks one at a time, following incremental-implementation
8. Review the spec for accuracy against the implemented code

## Conventions

- Specs live in `docs/specs/` or alongside the feature they describe
- Task lists use checkbox format: `- [ ] Task: [description]`
- Each task includes: acceptance criteria, verification step, files touched
- No task should require changing more than ~5 files
