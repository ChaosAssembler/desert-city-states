# Desert City-States — Game Design Document

> **Status:** Draft (pre-implementation)
> **Version:** 1.0 — full design expansion (Phase 1 of planning)
> **Genre:** Small-scale 4X strategy
> **Theme:** Rival oasis city-states in a harsh desert
> **Platform:** Browser/PC, turn-based
> **Rendering decision (confirmed):** 2D **graphical** rendering (not terminal/TUI). Presentation guidance below is written for 2D, but the underlying design is kept engine-agnostic so the eventual architecture can swap renderers/backends.
> **Single source of truth:** This document defines *what* the game is. The future `docs/architecture/` (software architecture) and `docs/architecture/decisions/` (ADRs) will define *how* it is built. Later phases: per-system technical specs and a phased roadmap.

---

## 1. Overview / High Concept / Vision & Pillars

### 1.1 High Concept

Players lead a desert city-state competing for survival and dominance across a sparse hex map. The core tension is **water scarcity** and **trade-route control**: expand to oases, connect them with caravans, defend them from raiders, and outmaneuver rival city-states.

This is a *small-scale* 4X: short games (target 30–60 minutes), a compact map, and a deliberately tight decision space. The ambition is not "bigger map" but "every tile and every route matters."

### 1.2 Vision

A readable, tense 4X where the board is a fragile web of dependencies rather than a blob of territory. Holding an oasis is meaningless if you cannot keep a caravan road to it alive. Players should feel like desert governors balancing a ledger of survival against the temptation to overextend.

### 1.3 Design Pillars

| Pillar | Meaning | How it shows up |
|---|---|---|
| **Scarcity** | Water is the hard constraint; you cannot grow without it. | Oases limited; cities starve if disconnected. |
| **Routes > Territory** | The road between oases matters as much as the oases. | Route control, raids, network effects (Section 8). |
| **Positioning > Numbers** | Smart placement beats unit spam. | Terrain combat modifiers, chokepoints (Section 10). |
| **Legible Board** | Short turns, readable map, no hidden math walls. | Clear tile value, visible routes, simple yields. |
| **Tense Expansion** | Growth creates vulnerability. | Each new city/oasis is another link to defend. |

### 1.4 Design Goals (from original doc, preserved)

- Simple enough to build quickly.
- Distinct theme without being weird.
- Strategic depth from scarcity and route control.
- Clear, readable map and short turns.

### 1.5 What Makes It Fun (from original doc, preserved)

- Every oasis matters.
- Expansion creates vulnerability.
- Trade routes create real strategic choices.
- Easy to understand, hard to optimize.

---

## 2. Target Audience & Core Experience

### 2.1 Audience

- **Primary:** Strategy players who enjoy *Civilization*-style 4X and *Hexagon*-style tactical games but want a session-length game (not a 10-hour commitment).
- **Secondary:** Players new to 4X who are intimidated by large titles; the small map and single resource ledger lower the barrier.
- **Not for:** Players seeking real-time action, or massive-grand-strategy scale.

### 2.2 Core Experience / "Feel"

The intended emotional arc per game:

1. **Discovery** — uncovering the map, racing rivals to oases.
2. **Construction** — the satisfaction of laying a first caravan road and watching yields climb.
3. **Anxiety** — realizing a road is exposed and a rival Raider is one move away.
4. **Triage** — deciding which route to defend, which city to reinforce, which to abandon.
5. **Resolution** — a clean win (route domination, wealth score, or relic hold) that feels earned from logistics, not luck.

Tone: dry, sun-baked, economic. Not apocalyptic. The drama is logistical.

---

## 3. Presentation & UI/UX Direction (2D Graphical)

> The game is rendered in **2D** (top-down hex map, panel-based HUD) using **macroquad** (2D graphical, immediate-mode) as the rendering engine, confirmed in Phase 2 architecture. Design intent remains engine-agnostic where practical.

### 3.1 Camera & Map View

- **Single-screen-friendly** by default (the MVP map fits one viewport); support **zoom/pan** for the full-scope larger map.
- **Hex map** drawn top-down with an isometric-ish shading only for readability (pure 2D, no 3D projection needed).
- **Smooth camera**: drag-to-pan, wheel-zoom, edge-scroll optional. Click a city/unit to focus.
- **Heat/tint cues**: oasis tiles tinted green-blue, dunes warm tan, salt flats pale grey, ridges darker, ruins with a marker. Route tiles show a dashed caravan line that turns red when threatened.

### 3.2 HUD Layout (proposed)

| Region | Content |
|---|---|
| Top bar | Turn number, current resources (Water / Wealth / Influence) with per-turn deltas, victory进度 meters. |
| Left panel | Selected entity (city/unit/tile) details and actions. |
| Right panel | Minimap + alerts (route under attack, city starving, rival nearby). |
| Bottom bar | End Turn, build/train queue, current research/specialization choice. |

### 3.3 Interaction Model

- **Left-click** select; **right-click** context action (move, found, build route).
- **Hover** tile → tooltip with yield, owner, threat.
- **Route planning mode**: click two cities (A→B) → engine auto-computes the shortest safe path and shows a cost preview → confirm. *(No manual tile-by-tile path editing — auto-route only, see Section 8.1.)*
- **Undo-on-cursor**: within a turn, moves/orders can be re-issued before End Turn (no commit until End Turn).

### 3.4 Art Direction (notes)

- **Palette:** desaturated, sun-bleached desert; reserve saturated color for player accents (blue=Water, gold=Wealth, violet=Influence, faction colors for borders).
- **Units:** small iconographic tokens (scout = footprint, caravan guard = shield-on-camel, raider = blade). No need for detailed sprites in MVP.
- **Cities:** grows visually with population/specialization (tent → walled town → specialized icon).
- **Readability over realism.**

### 3.5 Audio Direction (notes)

- Ambient wind bed; sparse UI clicks; route-raid sting; victory/defeat sting. Optional for MVP; flagged as stretch.

### 3.6 Accessibility

- **Colorblind-safe** palette + shape/icon redundancy (don't rely on color alone for ownership or threat).
- **Text scaling** for HUD; **high-contrast** mode toggle.
- **Optional tile-value labels** (numbers on tiles) for players who want explicit yields.
- **Hotkeys** for core actions (end turn, build, found).

---

## 4. Core Gameplay Loop (Detailed)

The original loop is preserved and expanded with **decision points**. One game turn = one "tick" in which players take **sequential turns** — players and AI act one after another in classic 4X order (e.g., player → AI → AI → income), never simultaneously. This turn model is **confirmed** (see Section 16.1).

| Step | Action | Player decision points |
|---|---|---|
| 1 | **Scout** nearby hexes | Which direction to reveal? Risk a scout vs. save the unit? |
| 2 | **Found / upgrade** a city on an oasis | Where to expand next? Which specialization to pick? |
| 3 | **Build caravan routes** between cities | Which pair to connect first? Direct vs. safe path? |
| 4 | **Earn** Water, Wealth, Influence | (passive) — but where is income concentrated? |
| 5 | **Spend** on expansion / defense / upgrades | Build a Guard or a building? Reinforce a route or a city? |
| 6 | **Fight** over routes and oases | Raid enemy route? Defend your own? Counterattack? |
| 7 | **Win** by control or score | Track victory meters; pivot strategy if behind. |

**Macro decision spine:** every turn the player answers *"Am I expanding, consolidating, or contesting?"* — and the resource ledger forces trade-offs (you rarely can do all three).

---

## 5. Map & World Generation

### 5.1 Grid & Coordinates

- **Hex grid**, **pointy-top** (recommended for 2D readability) or flat-top — *proposal: pointy-top*.
- **Axial coordinates** `(q, r)` with a `s = -q - r` invariant (standard Red Blob hex math). This keeps distance and neighbor math simple for the future architecture.
- **Distance:** axial hex distance `max(|Δq|, |Δr|, |Δs|)`.

### 5.2 Map Size

| Scope | Radius | Approx. tile count |
|---|---|---|
| MVP | 4 | 61 tiles |
| Full | 7–9 | 169–271 tiles |

*(Configurable — map size and turn count are scenario options; defaults above, scaled/tuned for session length — see Section 16.2 and Open Questions.)*

### 5.3 Tile Types (full attributes)

| Tile | Move cost | Defense mod | Yields (base) | Notes |
|---|---|---|---|---|
| **Oasis** | 1 | +0 | Water +3, Wealth +1 | Only city-foundable tile; scarce. |
| **Dunes** | 2 | +0 | — | Slow; low value; common filler. |
| **Salt Flats** | 1 (fast) | −1 (exposed) | Wealth +1 | Fast traversal but vulnerable to ambush. |
| **Ridges** | 3 (hard) | +2 (defensive) | — | Chokepoints; strong defender bonus. |
| **Ruins** | 1 | +0 | — | May contain relics / bonuses (Section 5.6). |

*All yields are per-turn to the owning/occupying player when worked.*

### 5.4 Biome Distribution & Generation Approach

**Procedural generation (proposal):**
1. Seed a **voronoi/cluster** of oases (target density ~1 oasis per 12–18 tiles; for MVP radius-4, ~4–5 oases total → ensures contest).
2. Scatter **Ridges** in 1–2 linear chains to form natural chokepoints between regions.
3. Fill remaining space with **Dunes** (majority) and **Salt Flats** patches (linear corridors — naturally create fast caravan corridors *and* ambush zones).
4. Place **Ruins** on a subset of non-oasis tiles (1–2 per map).
5. **Player/city start positions** on distinct oases, spaced by min distance (e.g., ≥4 hexes apart) so early game isn't an instant collision.

**Determinism:** seeded RNG (seed shown in menu) so maps are shareable/reproducible — useful for balance testing.

### 5.5 Oasis Placement Rules

- Oases never adjacent to each other (min 2 hex gap) to avoid trivial double-found.
- Each oasis gets a small **workable ring** (its 6 neighbors) that the owning city can improve.

### 5.6 Ruins & Relic Sites

- **Ruins** are enterable tiles that, when a unit stops on them, grant a one-time reward (Wealth, Influence, or a free building) — *proposal*.
- Some ruins are **Relic Sites** (see Victory Condition 3, Section 13): holding these for N consecutive turns wins.

### 5.7 Map Symmetry

For fairness in 2–4 player maps, generation can mirror oasis/city placement across the map center — *proposal for competitive feel; optional toggle.*

---

## 6. Resources & Economy

Three primary resources (preserved). All are **per-turn flows** plus a **stockpile**.

| Resource | Primary source | Primary sink | Role |
|---|---|---|---|
| **Water** | Oasis tiles, Well Fort specialization | City growth & maintenance, route upkeep | The survival constraint. |
| **Wealth** | Trade routes, Salt Flats, Trade Hub | Buildings, unit training, bribes | The expansion/war machine. |
| **Influence** | Scholar Outpost, relic holds, surplus | Diplomacy, special actions, founding | The soft-power lever. |

### 6.1 Stockpiles vs. Flow

- Each resource has a **stockpile cap** (proposal: Water 30, Wealth 50, Influence 30 at base; raised by buildings). Excess per-turn flow beyond cap is lost (encourages spending, prevents hoarding).
- **Water deficit** is the failure state: if a city's stockpile hits 0 and net flow is negative, the city **starves** (loses population, then becomes neutral/abandoned).

### 6.2 Interdependencies (the economic spine)

- Cities need **Water** to grow; growth unlocks more building slots and higher unit caps.
- **Wealth** is mostly generated *by routes*, not by cities directly — so Wealth scales with your **network**, not your territory size. This is the mechanical expression of "routes > oases."
- **Influence** lets you *found* new cities and trigger special actions (e.g., sabotage a route, recruit a neutral scout). Low Influence = stagnant expansion.

### 6.3 Secondary Mechanics (proposal, MVP-excluded)

- **Drought events:** periodic Water yield reduction on random oases → forces route diversification.
- **Caravan tolls:** passing a route through a rival's claimed tile yields them Influence (creates negotiation tension).

---

## 7. Cities

### 7.1 Founding Rules

- A city may be **founded** only on an **Oasis** tile by a unit with founding ability (Scout or a dedicated Founder — *proposal: Scout can found*).
- Cost: **Influence** (proposal: 10) + the tile must be outside enemy territory and not already owned.
- A player may own **multiple** cities (one per oasis).

### 7.2 Growth & Population

- Each city has **Population** (starts 1). Grows +1 when **Water stockpile > threshold** (proposal: >5) and a growth timer elapses (proposal: every 3 turns of surplus).
- Population drives: building slots, unit training capacity, and defense strength.

### 7.3 Tile Adjacency / Worked Ring

- A city **works** its 6 neighboring tiles (and itself). Worked tiles' base yields flow to the owner.
- Improvements (Section 7.5) can be built on worked tiles to boost yield or grant effects.

### 7.4 Buildings / Improvements Catalog (proposal)

| Building | Cost (Wealth) | Effect |
|---|---|---|
| **Well** | 8 | +2 Water on this oasis. |
| **Market** | 10 | +2 Wealth from this city's routes. |
| **Granary** | 6 | +5 Water stockpile cap. |
| **Watchtower** | 12 | Reveals fog in radius 2; +1 defense to adjacent tiles. |
| **Caravanserai** | 10 | Reduces route upkeep from this city by 1; +1 route capacity. |
| **Temple** | 12 | +1 Influence/turn. |

### 7.5 The Four Specializations

A city **upgrades into a specialization** once Population ≥ 3 (proposal) and you spend Influence. Each city picks **one**; roles are distinct and non-overlapping.

| Specialization | Role | Signature bonuses |
|---|---|---|
| **Trade Hub** | Economic engine | +50% Wealth from its routes; extra route capacity; cheaper Markets. |
| **Well Fort** | Water security | +3 Water/turn; its oasis cannot starve below Pop 1; supplies Water to connected cities. |
| **Fortress** | Military anchor | +3 defense to city tile and neighbors; trains units cheaper/faster; projects zone control. |
| **Scholar Outpost** | Influence & relics | +2 Influence/turn; +1 relic-hold progress; reveals more fog. |

**Design intent:** specialization creates *complementary* cities — you want at least one Well Fort (survival), one Trade Hub (economy), and probably a Fortress (defense). This naturally shapes the "what to build where" puzzle.

### 7.6 Upgrade Paths

Within a specialization, buildings can be **tiered** (e.g., Market → Great Market) in full scope. MVP: single-tier buildings only.

---

## 8. Caravan & Trade-Route System (Signature Mechanic)

> This is the heart of the game. The model below is detailed so an architect can spec it directly.

### 8.1 What a Route Is

A **route** is a directed/undirected connection between **two owned cities** following a **path of contiguous tiles** (the path is the set of hexes the caravan travels). A route is defined by:
- **Endpoints:** two cities you own.
- **Path:** a tile sequence **auto-computed as the shortest safe path at creation** (auto-route only — see 8.2). The player establishes the A→B connection; they do *not* draw or edit the exact tiles.
- **Status:** Active / Threatened / Severed.

### 8.2 Establishing a Route

- Action at a city: **"Connect to City B."** The engine **auto-routes** the caravan along the shortest safe path between the two cities (minimizing Ridge cost and avoiding enemy tiles where possible). The player only picks the endpoints A→B; they have **no manual path editing** (auto-route only is the confirmed design — see Open Questions).
- **Cost:** Wealth (proposal: 5 + 1 per tile of path length) paid once; plus **upkeep** (see 8.5).
- **Capacity limit:** a city can support a limited number of routes (proposal: base 2, +1 per Caravanserai / Trade Hub).

### 8.3 What Routes Yield

Per active route, per turn:
- **Wealth** = base (proposal: 2) + 1 per endpoint's Trade-leaning bonus + distance factor (longer safer routes slightly more, capped).
- **Water transfer:** routes **pipe Water** from a Well Fort / surplus oasis to connected cities. A city with no Water-yielding oasis of its own *depends entirely on routes* — this is how route control beats oasis control.
- **Influence:** small amount if route passes through claimed/contested tiles (presence).

### 8.4 Route Vulnerability & Defense

A route tile is **controlled** if it is within your territory (worked ring / zone control) or patrolled by a friendly **Caravan Guard** stationed on/adjacent to it.

- A route tile **not controlled** is **exposed**. An enemy **Raider** on or adjacent to an exposed tile can **raid** the route.
- **Raid outcome (proposal):** the route goes **Threatened** (yields halved) for the turn; if raided 2 consecutive turns with no defender, it is **Severed** (yields 0 until re-patrolled/rebuilt).
- **Defense:** a Caravan Guard on a route tile grants **control** of that tile and negates raids there. Multiple guards can cover a long route.
- **Terrain matters:** a route over **Ridges** is harder to raid (defender bonus propagates); a route over **Salt Flats** is easier to raid (exposed). This makes corridor choice a real decision.

### 8.5 Upkeep & Network Effects

- Each route costs **Water upkeep** (proposal: 1/turn) drawn from the network pool — long disconnected empires bleed Water.
- **Network effects (proposal):**
  - **Redundancy:** if two cities have 2+ independent routes, losing one does not Sever the link.
  - **Synergy:** each additional connected city adds +10% Wealth to *all* routes in the network (economy of scale) — rewards building a web, not spokes.
  - **Isolation penalty:** a city with **zero active routes** to the rest of your network suffers −2 Water/turn (starvation risk) even if it sits on an oasis — *this is the core "routes > oases" rule made mechanical.*

### 8.6 Why Routes Beat Oases (design summary)

You can **own** an oasis but if a rival **severs your only road** to it (or to your Water source), that city starves and is effectively neutralized without a single siege. Conversely, controlling the **corridors** between rival oases lets you strangle them. Territory is necessary but **insufficient**; logistics is the victory.

### 8.7 Route Diplomacy (stretch)

Routes may pass through neutral/rival tiles; tolls and right-of-passage (Influence bribes) create soft negotiation. Excluded from MVP.

---

## 9. Units

### 9.1 Stats Framework

Each unit has: **Move** (hexes/turn), **Attack**, **Defense**, **HP**, **Upkeep** (Wealth/turn), **Sight** (fog radius), **Abilities**.

### 9.2 Unit Catalog

| Unit | Move | Atk | Def | HP | Upkeep | Sight | Trained at | Role |
|---|---|---|---|---|---|---|---|---|
| **Scout** | 3 | 1 | 1 | 2 | 0 | 3 | Any city | Reveal map; **can found cities** (proposal). Cheap, fragile. |
| **Caravan Guard** | 2 | 3 | 4 | 4 | 1 | 1 | Fortress / any | Defends routes & cities; control-granting. |
| **Raider** | 3 | 4 | 2 | 3 | 1 | 2 | Fortress | Attacks routes & weak targets; fast, offensively weak on defense. |

*Numbers are **proposals** for first balance pass.*

### 9.3 Actions / Abilities

- **Scout:** Move (reveals), Found City, Embark (none). No attack worth noting.
- **Caravan Guard:** Move, **Patrol** (station on route tile → grants control), Garrison (boost city defense if in city tile).
- **Raider:** Move, **Raid Route** (sever/threaten an exposed route tile), **Raid City** (attack undefended/weak city — reduces population; captures at Pop 0).

### 9.4 Training & Upkeep

- Trained at cities (Fortress trains faster/cheaper — proposal: −25% cost/speed).
- Upkeep drawn from Wealth stockpile; inability to pay → unit is **disbanded** (or goes "mutinous"/idle — proposal: disband).
- **Unit cap** per player scales with total Population (proposal: cap = 2 + total Pop).

### 9.5 Counters (rock-paper-scissors-lite)

- **Raider beats** exposed routes and undefended cities.
- **Caravan Guard beats** Raiders (higher Def, control).
- **Scout** beats ignorance (information), loses to everything in combat.
- Positioning (terrain) can flip any of these (Section 10).

---

## 10. Combat System

### 10.1 Auto-Resolution Model

Combat is **automatic** (player sets intent via unit positioning/orders; resolution is deterministic-ish with a dice roll). No manual tactical battles — keeps turns short and matches the "legible board" pillar.

**Resolution formula (proposal):**
```
attack_power  = Atk * atk_pos           // morale = 1.0 (MVP off); atk_pos encodes attacker positioning (flank ×1.25 / Salt-Flats-exposed ×0.90)
defense_power = Def * (1 + terrain_def) // terrain_def applies to the *defender's* tile only (Section 10.2); defender positional factor = 1.0
odds = attack_power / (attack_power + defense_power)
result: roll → apply HP loss; loser retreats or is destroyed.
```
- **Positioning factor (`atk_pos`):** the attacker's tile effects are folded into `atk_pos`, *not* a separate defense/terrain multiplier. Attacking from a **flank/tile with advantage** (e.g., from Ridge onto Dune) gives ×1.25; attacking **from Salt Flats** (exposed) gives ×0.90. *Attacker choice of approach tile matters → positioning > numbers.* The only terrain term is the defender's tile `terrain_def` (Section 10.2); there is no attacker terrain-atk modifier.

### 10.2 Terrain Modifiers

| Tile (defender on) | Def mod | Effect |
|---|---|---|
| Oasis | +0 | neutral |
| Dunes | +0 | neutral |
| Salt Flats | −1 | exposed (easier to hit) |
| Ridges | +2 | strong defensive |
| City tile | +city def (Fortress +3) | fortified |

### 10.3 Morale / Odds

- Morale is fixed at **1.0** (MVP off — not a dynamic system; the formula retains a `morale` slot set to 1.0 for forward compatibility, per gameplay-combat.md §6.1). *Resolved: the earlier dynamic-morale proposal (outnumbered/replenish) is dropped.*
- Combat is **local**: only units on/adjacent to the contested tile fight (no global stack). This keeps "positioning > numbers" true — a blob of 10 Raiders scattered loses to 2 Guards on a Ridge.

### 10.4 Sieges on Cities

- Attacking a city: defender gets city tile def mod + any Garrisoned Guard + Fortress bonus.
- Successful siege: city **population −1** (or captured if Pop hits 0 and attacker occupies). Captured city flips ownership (its routes re-evaluate).
- **Routes during siege:** a besieged city's outgoing routes are Threatened while siege active.

### 10.5 Route Raids (combat-adjacent)

Covered in 8.4 — Raider vs. route is a lightweight contest: Raider "wins" if no controlling Guard adjacent; otherwise resolved by unit stats (Guard Def vs Raider Atk).

---

## 11. AI Opponents

### 11.1 Personalities (proposal)

| Personality | Behavior |
|---|---|
| **Expansionist** | Found cities aggressively; prioritizes oases. |
| **Raider** | Few cities; spams Raiders; targets exposed routes. |
| **Trader** | Builds dense route networks; defensive; wealth-score victory. |
| **Fortifier** | Few, heavily defended cities; relic-hold or oasis-control victory. |

### 11.2 Decision-Making

- **Turn-level goals** scored by a simple weighted utility function (expand vs. defend vs. raid vs. build).
- **Route awareness:** AI values route security (patrols exposed tiles) — critical so the signature mechanic is exercised by opponents, not just the player.
- **Threat response:** if a route is Threatened/Severed, AI reroutes or sends a Guard.

### 11.3 Difficulty

- **Easy:** slower expansion, occasional/unreliable route defense (defends a bit, but less reliably — not zero defense), ignores morale.
- **Normal:** balanced utility, defends core routes reliably.
- **Hard:** pre-emptive route cutting, focus-fire on player's weakest link, efficient specialization.

MVP AI: a single "Normal-ish" personality (expansion + basic raiding per original MVP scope).

---

## 12. Fog of War & Exploration

> **Authoritative source:** the fog-of-war behavior is fully specified in `gameplay-fog-of-war.md`. This section summarizes it; that spec governs in case of any discrepancy.

- Map starts **hidden** except around your starting city (radius = Scout sight or city sight).
- **Scout** and **Scholar Outpost** reveal the most; cities reveal a small radius.
- **Unexplored tiles:** no yield, no route planning through them.
- **Memory-marker model (changed during review):** enemy **cities/routes**, once any of their tiles has been revealed, remain shown afterward as a **memory marker** — rendered dimmed/stale to signal the information is not live. Their true current state (ownership, status, garrison) is shown **only while actually observed**; when out of sight, the marker reflects the last-seen state, not the present one.
- **Enemy units get NO memory:** an enemy unit is shown while it is within current sight and is **hidden entirely once it leaves sight** — no last-seen marker is retained.
- **Watchtower** building extends reveal radius (Section 7.4).

---

## 13. Victory Conditions

All three from the original doc, now with **concrete thresholds** (proposals) and tracking.

| # | Condition | Threshold (proposal) | How tracked / displayed |
|---|---|---|---|
| **V1 — Oasis Dominance** | Control the most oases at game end, OR a majority. | **≥50% of all oases** (or most if <50% reachable) at turn limit; or eliminate all rivals. | Live "Oases: X/Y" meter in top bar. |
| **V2 — Wealth/Prestige Score** | Accumulate a target Wealth+Influence score. | **Score ≥ 200**, where `Score = Wealth×1 + Influence×2 + oases×8 + active_routes×4` (proposal). Active routes boost the score (reinforcing the caravan theme), so a well-connected network out-scores raw wealth/prestige; the literal `Wealth×1 + Influence×2` form is the special case that results when the oasis and active-route weights are set to 0. | Score meter; visible to all. |
| **V3 — Relic Hold** | Hold all/majority relic sites continuously. | **Hold the relic sites for a configurable consecutive-turn duration** (proposal: ≥2 relic sites for 10 turns on the default full-scope map). | Relic timer per site; visible. |

- **Turn limit, relic thresholds, and hold-duration are configurable and scale with the scenario.** Rather than hard-coding, these derive from the configured **map size & player count** (see Section 16.2 / Open Questions): for a **2-player or small map**, relic count and hold-duration scale *down* (e.g., 1 relic for fewer turns), and the turn limit shrinks to match the smaller board. Defaults above are the full-scope starting point and will be tuned by playtest.
- **Turn limit (proposal, configurable):** default 60 turns for full scope; 30 for MVP. If no one meets a threshold by the limit, **highest score wins** (tiebreak: oases, then score).
- **MVP ships with V1 only** (per original scope), but the tracking UI should be built to show all three for forward-compatibility.

---

## 14. Progression / Tech / Social Systems

- **MVP: none.** No tech tree, no meta-progression, no multiplayer networking. Explicitly out of scope to keep build fast.
- **Full scope (stretch):** a light **"Traditions"** track (spend Influence on permanent empire bonuses, e.g., +1 route capacity, +1 Water cap) — *proposal, not committed.* No per-unit XP/leveling (keeps balance simple). No online play in current plan.

---

## 15. Content Catalogs (Consolidated)

### 15.1 Tile Types
Oasis · Dunes · Salt Flats · Ridges · Ruins (subset = Relic Sites).

### 15.2 Buildings / Improvements
Well · Market · Granary · Watchtower · Caravanserai · Temple.

### 15.3 Units
Scout · Caravan Guard · Raider.

### 15.4 City Specializations
Trade Hub · Well Fort · Fortress · Scholar Outpost.

### 15.5 Relics / Ruins Rewards (proposal)
Wealth cache · Influence burst · Free building · Relic Site (victory).

### 15.6 Victory Conditions
V1 Oasis Dominance · V2 Wealth/Prestige Score · V3 Relic Hold.

---

## 16. Balance & Pacing Considerations

### 16.1 Turn Structure (confirmed: Sequential)

> **Decision (confirmed):** The game uses **sequential turns** — players and each AI act one after another (classic 4X order: player → AI → AI → … → income), never simultaneously. This is simpler to implement and reason about, and avoids simultaneous-resolution fairness/race-condition complexity (see Open Question #6). Simultaneous-resolution is **no longer a contender**.

- **Turn flow:** order phase (current player/AI issues orders) → resolution phase (moves, combat, raids, yields applied for that actor) → income phase (resources tallied) → next actor's turn.
- **Sequential for all scopes** (MVP and full), player then each AI, then on to the next turn.

### 16.2 Pacing (early / mid / late)

> **Session length is configurable (confirmed):** turn count and map size are **fully configurable** via settings/scenario options rather than hard-coded. The ranges below are the default full-scope pacing; smaller maps / fewer players use proportionally shorter phase boundaries and turn limits, and relic/win thresholds scale down accordingly (see Section 13). Exact numbers are first-pass and tuned by playtest.

| Phase | Turns (full, default) | Player focus |
|---|---|---|
| **Early** | 1–15 | Scout, found 1–2 cities, lay first routes, secure Water. |
| **Mid** | 16–40 | Specialize cities, build route web, first raids/skirmishes. |
| **Late** | 41–60 | Contest relic/oases, decisive raids, push victory threshold. |

### 16.3 MVP vs Full Scope (reminder)

MVP = small map, fog, 3 resources, 3 units, 1 generic city + simple upgrades, caravan routes, basic AI (expand+raid), **1 victory condition (V1)**. Everything else above is full-scope unless marked MVP.

---

## 17. MVP Scope (Refined) & Stretch / Full-Scope

### 17.1 Refined MVP

- Hex map radius 4 (61 tiles), seeded RNG, 2–3 AI.
- Fog of war (Scout + city sight).
- 3 resources with stockpile caps and Water-starve rule.
- 3 unit types with the stats in Section 9.
- Cities: found on oasis, Population growth, **generic** city (specializations deferred but data-modeled so they slot in).
- Caravan route system with establish / upkeep / Threatened / Severed + isolation penalty (Section 8) — *this is the must-have signature, keep it in MVP.*
- Auto-combat with terrain modifiers (Section 10, morale optional/off).
- Basic AI: expansion + raiding.
- **Victory: V1 (oasis dominance)** only, with turn limit fallback.
- 2D graphical map + HUD (Section 3) at functional fidelity (no final art).

### 17.2 Stretch / Full-Scope

- Larger maps, map symmetry, drought events.
- 4 specializations, tiered buildings, Scholar/Influence actions.
- V2 & V3 victory conditions, relocation of relic sites.
- Route diplomacy / tolls, personality-difficulty AI matrix.
- Audio, final art, accessibility full pass, hotkeys.
- Simultaneous-resolution turns, "Traditions" progression.

---

## 18. Open Questions / Design Risks

> Honest unresolved trade-offs. **Needs user/team input** where marked ⚠. Items marked ✅ RESOLVED are confirmed decisions recorded above.

1. ✅ **RESOLVED — Turn model:** **Sequential turns** confirmed (players and AI act one after another, classic 4X; no simultaneous resolution). Recorded in Sections 4 and 16.1.
2. ✅ **RESOLVED — Route path editing:** **Auto-route only.** The engine auto-routes caravans along the shortest safe path between connected cities; the player establishes the A→B connection, not the exact tiles. No manual path editing. Recorded in Sections 3.3, 8.1, 8.2.
3. **Balance numbers** (all tables) are **first-pass proposals** — expect tuning. Especially the isolation penalty (−2 Water) and network synergy (+10%) which define the core fantasy; these need playtesting. ⚠ Still open.
4. **Can a Scout found a city, or do we need a dedicated Founder unit?** Chose Scout-for-now to limit unit count; revisit if founding feels cheap. ⚠ Still open.
5. ✅ **RESOLVED — Relic Sites scaling:** Relic/win thresholds and turn limits are **configurable and scale with map size & player count** (e.g., relic count and hold-duration scale down for 2-player / small maps). Tuning deferred to playtest. Recorded in Sections 13 and 16.2.
6. **Raids resolution order:** with sequential turns, a defined rule is still needed for which actor's raid resolves first when a contested route tile is targeted within the same overall turn cycle — address in Phase 2. ⚠ Still open.
7. **AI route-defense competence** is the make-or-break of the signature mechanic feeling real — risk that AI ignores routes and the fantasy falls flat. Mitigation: bake route-security into AI utility weights (Section 11.2). ⚠ Still open.
8. ✅ **RESOLVED — Rendering engine:** Rendering engine = **macroquad** (2D graphical, immediate-mode). Confirmed in Phase 2 architecture.
9. ✅ **RESOLVED — Session length vs. map size:** Turn count and map size are **fully configurable** scenario options; design targets a range and tunes via playtest rather than hard-coding (default ~30–60 min). Recorded in Sections 5.2, 13, 16.2.
10. **Single-screen "friendly" vs. zoom/pan:** original says single-screen; full scope implies pan. Proposal: MVP single-screen radius 4, full scope zoom/pan. Confirm acceptable. ⚠ Still open.

---

*End of Document — Draft v1.0. Forward references: `docs/architecture/` (to be created, Phase 2) and `docs/architecture/decisions/` (ADRs, Phase 2). Per-system technical specs and the phased roadmap are planned subsequent phases.*
