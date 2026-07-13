---
name: incremental-implementation
description: Use when writing any feature or change that touches more than one file — implement in thin vertical slices
---

# Incremental Implementation

## Rules
- Implement the smallest complete piece of functionality before expanding
- Touch only what the task requires — no opportunistic cleanup or refactoring
- One logical change per increment — don't mix concerns
- Each increment is independently revertable
- Use feature flags for incomplete features that need to merge

## Workflow
1. Identify the smallest vertical slice that delivers end-to-end functionality
2. Implement the slice by writing its source
3. Move to the next slice, carrying forward from the previous one
4. Repeat until the task is complete

## Slicing Strategies
- Vertical (preferred): One complete path through the stack — component, system, basic UI
- Risk-first: Tackle the most uncertain piece first — if it fails, you discover it before investing in later slices
- ECS-first: Define components and resources first, then systems, then integration
