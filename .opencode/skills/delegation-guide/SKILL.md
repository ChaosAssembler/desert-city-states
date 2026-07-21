---
name: delegation-guide
description: Use when delegating tasks to subagents — teaches how to collaborate effectively with subagents
---

# Delegation Guide

## Rules

- Do not delegate without stating the goal and why it matters first
- Never give exact commands or scripts — describe what needs to be achieved and why
- Do not ask subagents to relay raw file contents, logs, or unfiltered tool output when processed results suffice
- Never attempt to create new agents or modify agent configurations unless the user explicitly requested it
- Do not override a subagent's expertise with overly specific instructions when you lack domain knowledge

## Workflow

1. State the intent — the goal and why it matters — before considering implementation
2. Assess whether you know the best approach; if uncertain, delegate with a question to get a recommendation
3. If you know exactly what is needed and why, provide specific instructions with your reasoning
4. Describe what needs to be achieved, not how to achieve it — the subagent knows its tools better than you do
5. Instruct the subagent to return synthesized, filtered, and directly usable results
6. If a request spans multiple domains, break it down and delegate sequentially
7. When you need information from a subagent's domain to proceed, ask that subagent first, then use the result to drive the next step
8. Iterate when needed — delegate to explore first, gather context, then delegate with better-informed instructions

## Conventions

- Aim for clear intent with context, letting the subagent fill in the method
- Use subagents for consultation, not just execution — delegate partial work, ask for proposals, or request assessments
- Prefer processed output (analysis, decisions, recommendations, structured findings) over raw data dumps
