# Desert City States — Planning Index

> **Status:** Planning doc 4 of 4 — implementation roadmap. Implementation status: Phase 0 complete, Phase 1 next.
> Companion to `docs/design/`, `docs/architecture/`, `docs/architecture/decisions/`, and `docs/specs/`.

This directory holds the phased implementation plan. All plans are documentation
only — no `Cargo.toml`, `.rs`, or code is created here.

## Documents

| Document | Purpose |
|---|---|
| [ROADMAP.md](./ROADMAP.md) | Master phased plan (Phases 0–6): goals, scope, deliverables mapped to specs, dependencies, exit/DoD criteria, milestone table, and open-question mapping. **Start here.** |

## How to read the roadmap

- Phases are **ordered by dependency**: a phase's work only begins once its
  dependency phases satisfy their exit criteria.
- Each phase lists **Key deliverables** mapped to specific spec files under
  `docs/specs/` so implementation tracks the binding specs.
- The **Open questions to resolve per phase** section maps every carried design
  (DD #3/#4/#6/#10) and architecture (PRNG, save format, route-through-fog)
  open question to the phase where it is decided or validated.

## Phase status (single active phase)

Per the planning convention, only one phase is active at a time.

- [x] **Phase 0 — Workspace scaffold**
- [~] **Phase 1 — Foundation (dcs-core)** — active / next
- [ ] Phase 2 — Gameplay systems (dcs-core)
- [ ] Phase 3 — Behavior (dcs-core)
- [ ] Phase 4 — Presentation (dcs-render + dcs-app)
- [ ] Phase 5 — MVP integration & playtest
- [ ] Phase 6 — Full scope / stretch

## Source documents

- Design: `docs/design/Desert-City-States.md`
- Architecture: `docs/architecture/ARCHITECTURE.md` + `docs/architecture/decisions/` (ADR-0001…0008)
- Specs: `docs/specs/README.md` (index of 15 specs + this roadmap)
