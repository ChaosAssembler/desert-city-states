---
name: incremental-implementation
description: Use when implementing any feature or change that touches more than one file — build in thin vertical slices with test/verify/commit cycles
---

# Incremental Implementation

## Rules

- Each increment must leave the system in a working, testable state
- Implement the smallest complete piece of functionality before expanding
- Never write more than ~100 lines without running tests
- Touch only what the task requires — no opportunistic cleanup or refactoring
- Keep the project compilable after every increment

## Workflow

1. Identify the smallest vertical slice that delivers end-to-end functionality
2. Implement the slice
3. Run tests (`cargo test`) and verify the slice works
4. Commit with a descriptive message
5. Move to the next slice, carrying forward from the previous one
6. Repeat until the task is complete

## Slicing Strategies

- **Vertical (preferred):** One complete path through the stack — component, system, basic UI
- **Risk-first:** Tackle the most uncertain piece first — if it fails, you discover it before investing in later slices
- **ECS-first:** Define components and resources first, then systems, then integration

## Conventions

- One logical change per increment — don't mix concerns
- Each increment is independently revertable
- Note improvements outside task scope, don't fix them inline
- Use feature flags for incomplete features that need to merge
