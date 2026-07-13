---
name: review-reporting
description: Use when reporting review findings — structured issue table format, severity levels, and fix actions for any review agent
---

# Review Reporting

## Rules

- Every review report uses the structured format below
- One issue per row — never combine multiple problems in a single row
- Severity reflects impact, not effort to fix
- Fix actions reflect scope ownership, not severity

## Report structure

```markdown
## Review Report

### Checked
- [list of files, patterns, and conventions checked]

### Issues Found
| Severity | File | Reference | Problem | Fix Action |
|----------|------|-----------|---------|------------|
| error | <file> | <reference or section> | <what is wrong> | FIX or REPORT |
| warning | <file> | <reference or section> | <what could improve> | REPORT |

### Summary
- Errors: N
- Warnings: N
```

## Severity levels

- **error** — convention violation, missing required field, broken reference, or structural defect that must be resolved
- **warning** — advisory finding, style improvement, or drift that should be addressed but is not blocking

## Fix actions

- **FIX** — issue was auto-fixed within the reviewer's edit scope
- **REPORT** — issue is outside the reviewer's edit scope and requires action from the agent that owns the affected file

## Conventions

- Always include the "Checked" section so the reader knows the review scope
- Reference columns use the project's cross-reference format: `DD §N.M`, `ARCH §N.M`, `ADR-nnnn`, or relative file paths
- Keep problem descriptions factual and specific — no vague "needs improvement"
- Summary counts should match the Issues Found table
