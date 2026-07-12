---
name: doc-consistency
description: Use when writing or updating documentation — verifies cross-reference integrity, index synchronization, and terminology alignment across docs/ after changes
---

# Documentation Consistency

## Rules

- Check consistency of changed files before reporting completion
- Auto-fix structural issues within edit scope
- Report issues outside edit scope with severity, file, reference, and problem description
- Never delete or modify cross-references without updating all targets
- Prefer fixing over reporting — only report what is genuinely outside scope

## Workflow

1. Identify changed files from the current task
2. Extract all cross-references from changed files (DD §, ARCH §, ADR-nnnn, relative links, bare §)
3. Validate structural references — resolve each reference and verify the target exists (see Reference Resolution below)
4. Validate spec metadata — check Implements header, References section, and README index sync for spec files
5. Validate ADR metadata — check status field and sequential numbering for ADR files
6. Run semantic checks — terminology alignment and bidirectionality (see Semantic Checks below)
7. Auto-fix structural issues within edit scope (wrong section numbers, broken links to files you can edit, missing metadata in your domain)
8. Report remaining issues in the structured format below

## Reference Resolution

| Reference Type | Target Location | Verification |
|---|---|---|
| `DD §N.M` | `docs/design/Desert-City-States.md` heading `## N.` or `### M.` | Section heading exists |
| `ARCH §N.M` | `docs/architecture/ARCHITECTURE.md` heading `## N.` or `### M.` | Section heading exists |
| `ADR-nnnn` | `docs/architecture/decisions/nnnn-*.md` | File exists |
| Relative `[text](path)` | Resolve from source file location | File exists |
| Bare `§N` | Current document's own headings | Section exists in current doc |
| Cross-spec `name.md §N` | `docs/specs/name.md` heading | File and section exist |

## Spec Metadata Checks

For files in `docs/specs/`:

- **Implements header** — blockquote near the top must contain `> **Implements:**` with at least one DD §, ARCH §, or ADR-nnnn reference
- **References section** — must contain `## 9. References` (or similar) with four bullets: Design (DD §), Architecture (ARCH §), ADRs (ADR-nnnn), Related specs (backtick-quoted filenames)
- **README sync** — the spec's Implements header must match the Implements column in `docs/specs/README.md` for the corresponding row; the spec file must exist as listed in the README table
- **Orphan check** — any `.md` file in `docs/specs/` that is not listed in the README table is an orphan

## ADR Metadata Checks

For files in `docs/architecture/decisions/`:

- **Status field** — must contain exactly one of: `Accepted`, `Superseded by ADR-XXX`, `Deprecated`
- **Sequential numbering** — ADR filenames must use zero-padded 4-digit numbers with no gaps or duplicates (0001, 0002, ...)

## Semantic Checks

These checks produce warnings, not errors. Report but do not auto-fix unless the fix is unambiguous.

- **Terminology alignment** — when a cross-reference uses a term (e.g., "combat system"), the target section heading should use the same or equivalent term; flag mismatches like "hex grid" vs "hexgrid"
- **Bidirectionality** — if a spec file heavily references another spec (3+ references), the referenced spec should have at least one reference back; flag one-directional heavy references

## Rename and Move Protocol

Before renaming or moving any documentation file:

1. Grep for all references to the old filename across `docs/`
2. Update every found reference to the new filename
3. Verify no broken links remain after the rename

## Issue Reporting

Report unresolved issues as structured text using this format:

```markdown
## Consistency Verification

### Checked
- [list of files and patterns checked]

### Issues Found
| Severity | File | Reference | Problem | Fix Action |
|----------|------|-----------|---------|------------|
| error | combat.md | `DD §8.4` | Target section not found in DD | REPORT |
| warning | movement.md | `ARCH §4` | Terminology mismatch: "hex grid" vs "hexgrid" | FIX |

### Summary
- Errors: 1 (report to orchestrator)
- Warnings: 0 pending, 1 auto-fixed
```

Severity levels:

- **error** — broken reference, missing target file, invalid or missing required metadata
- **warning** — terminology drift, missing bidirectional reference, stale but resolvable reference

Fix actions:

- **FIX** — issue was auto-fixed within edit scope
- **REPORT** — issue is outside edit scope, requires action from the subagent that owns the affected file
