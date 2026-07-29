# Desert City-States

**Genre:** Small-scale 4X strategy — rival oasis city-states competing for survival and dominance across a sparse hex map in a harsh desert.

**Tech:** Rust, macroquad 2D renderer, turn-based, hex grid, water scarcity + trade-route control as the core tension.

**Workspace:** 5-crate Cargo workspace.

| Crate | Role |
|---|---|
| `dcs-core` | Pure, deterministic, serializable simulation — all game logic, zero rendering |
| `dcs-protocol` | Save-file schema versioning + command/event envelopes |
| `dcs-render` | macroquad presentation layer (2D graphical, immediate-mode) |
| `dcs-app` | Main entry point, main-loop glue, input → command dispatch |
| `dcs-mcp` | MCP server for headless play-testing and tooling |

📖 **Game design** → [`docs/design/Desert-City-States.md`](docs/design/Desert-City-States.md)  
🏛️ **Software architecture** → [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md)

---

## Multi-Agent Development Architecture

This project uses an **OpenCode agentic framework** with **24 specialized agents** and **18 skills** to automate and orchestrate development. The system follows a structured delegation hierarchy with strict role separation.

### Tier 1 — Overview: Delegation Flow

Two orchestrators sit at the top: `plan` (the default read-only planner) and `build` (the execution coordinator). They delegate tasks to category groups and individual specialists. Only `researcher` delegates further (to `web-fetcher`).

```mermaid
graph TD
    User -->|default agent| plan
    User -->|explicit| build

    subgraph Orchestrators
        plan["<b>plan</b><br/><i>read-only planner</i>"]
        build["<b>build</b><br/><i>execution coordinator</i>"]
    end

    plan -->|task| explore
    plan -->|task| consultant
    plan -->|task| researcher
    plan -->|task| doc-reviewer
    plan -->|task| agentic-reviewer

    build -->|task| dev["<b>Rust Development</b><br/>5 agents"]
    build -->|task| docs["<b>Documentation</b><br/>5 agents"]
    build -->|task| review["<b>Document Review</b><br/>2 agents"]
    build -->|task| agentsys["<b>Agent System</b><br/>2 agents"]
    build -->|task| infra["<b>Infrastructure</b><br/>3 agents"]
    build -->|task| investigate["<b>Read-Only Investigation</b><br/>3 agents"]
    build -->|task| testmaint["<b>Testing & Maintenance</b><br/>2 agents"]

    researcher -->|task| web-fetcher

    style plan fill:#e1f5fe,stroke:#0288d1
    style build fill:#fff3e0,stroke:#f57c00
    style researcher fill:#f3e5f5,stroke:#7b1fa2
```

### Tier 2 — Detailed Agent Taxonomy

All agents grouped by domain with their role descriptions:

```mermaid
graph TD
    subgraph "Rust Development"
        rust-coder["<b>rust-coder</b><br/>writes Rust source code"]
        rust-builder["<b>rust-builder</b><br/>compiles, lints, formats"]
        rust-tester["<b>rust-tester</b><br/>runs tests & benchmarks"]
        code-reviewer["<b>code-reviewer</b><br/>reviews Rust source"]
        workspace-architect["<b>workspace-architect</b><br/>manages Cargo manifests"]
    end

    subgraph "Documentation"
        arch-doc["<b>arch-doc</b><br/>architecture docs"]
        adr-writer["<b>adr-writer</b><br/>Architecture Decision Records"]
        spec-writer["<b>spec-writer</b><br/>implementation specs"]
        design-doc["<b>design-doc</b><br/>game design docs"]
        planner["<b>planner</b><br/>roadmaps & milestones"]
    end

    subgraph "Document Review"
        doc-reviewer["<b>doc-reviewer</b><br/>reviews documentation"]
        agentic-reviewer["<b>agentic-reviewer</b><br/>reviews agent/skill files"]
    end

    subgraph "Agent System"
        agentic-engineer["<b>agentic-engineer</b><br/>creates/configures agents"]
        skill-manager["<b>skill-manager</b><br/>creates/configures skills"]
    end

    subgraph "Infrastructure & Operations"
        mise-manager["<b>mise-manager</b><br/>manages dev tools (mise)"]
        committer["<b>committer</b><br/>creates git commits"]
        github-actions-writer["<b>github-actions-writer</b><br/>CI/CD workflows"]
    end

    subgraph "Read-Only Investigation"
        explore["<b>explore</b><br/>codebase exploration"]
        consultant["<b>consultant</b><br/>domain guidance"]
        web-fetcher["<b>web-fetcher</b><br/>fetches URLs"]
    end

    subgraph "Testing & Maintenance"
        game-tester["<b>game-tester</b><br/>play-tests via MCP"]
        readme-maintainer["<b>readme-maintainer</b><br/>keeps README accurate"]
    end
```

### Architecture Summary

- **2 orchestrators**: `plan` (default read-only planner) and `build` (execution coordinator)
- **22 subagent specialists** across 7 domains (plus 1 delegating specialist: `researcher`)
- **Maximum delegation depth of 2** (orchestrator → subagent → web-fetcher)
- **Deny-by-default permission model** with least-privilege tool access
- **Auto-discovery**: agents are loaded from `.opencode/agents/*.md`
- **Skill loading**: all subagents load the `subagent-autonomy` skill; orchestrators load the `delegation-guide` skill

---

## Links

| Path | Description |
|---|---|
| [`.opencode/agents/`](.opencode/agents/) | Agent definitions (24 agents) |
| [`.opencode/skills/`](.opencode/skills/) | Skill definitions (18 skills) |
| [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md) | Game software architecture |
| [`docs/design/Desert-City-States.md`](docs/design/Desert-City-States.md) | Game design document |
| [`docs/planning/README.md`](docs/planning/README.md) | Planning documents & roadmap |
