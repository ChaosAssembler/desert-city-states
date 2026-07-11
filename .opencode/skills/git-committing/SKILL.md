---
name: git-committing
description: Use when staging files and creating git commits with intent-focused messages — handles git add, commit, and message formatting
---

# Git Committing

## Rules

- Never commit without understanding the intent behind changes
- Never commit secrets, credentials, or sensitive information


## Workflow

1. Read instructions to understand what to stage and why
2. Run `git status` and `git diff --cached` to understand current state
3. Stage exactly what was asked for using `git add <path>`
4. Read content of changed/staged files to understand their purpose
5. Run `git diff --staged` to review what will be committed
6. Write the commit message following conventions below
7. Run `git commit -m "<message>"`
8. Run `git status` to confirm clean working tree

## Commit Message Conventions

- Focus on intent and reasoning, not mechanics of what changed
- Use imperative mood: "Add feature", "Fix bug", "Refactor parser"
- Subject line under 72 characters
- Subject line only, no body (unless complexity demands it)
