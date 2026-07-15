# Gameplay Spec: Combat

> **Phase:** 2 — Per-system gameplay specs (group: gameplay)
> **Crate:** `dcs-core` (module `dcs-core::combat`)
> **Status:** Draft for review
> **Implements:** DD §10; ARCH §5, §16; ADR-0004 (Command-only), ADR-0006 (RNG in state)

---

## 1. Purpose

Specify the **automatic combat resolution** model: the core odds formula combining
attacker/defender stats, **terrain defense modifier**, and **positioning** (flanking
/ adjacency > raw count), city **sieges**, and the **Raider-vs-route contest**.
All randomness draws from `GameState.rng`. Emits `GameEvent::Combat` /
`RouteRaided` / `CityRaided`. Tied to DD §10.

## 2. Scope

**In scope**

- Auto-resolution formula (attack/defense power, odds, HP loss, retreat/destroy).
- Terrain defense modifiers (table) + attacker positioning factor.
- Local combat (only on/adjacent units fight — no global stack, DD §10.3).
- City sieges (defender city-tile mod + Garrison + Fortress).
- Raider-vs-route contest (guard present ⇒ stat contest; absent ⇒ auto-Threatened/Severed).
- Resolution-order rule for contested raids (DD #6, recommended default).

**Out of scope**

- Unit movement / A* (units-movement spec) — it triggers combat on enter.
- Route state machine details (caravan spec) — this spec feeds `Threatened/Severed`.
- Morale system (DD §10.3 optional/off for MVP — noted, not implemented).

## 3. Responsibilities

- Own `resolve_combat(attacker, defender) -> (events)` used by move/siege/raid.
- Apply **terrain modifiers** and **positioning** consistently.
- Draw **only** from `state.rng`; never `thread_rng`/`std::time` (ADR-0006).
- Emit `Combat`/`RouteRaided`/`CityRaided` events for render/UI.

## 4. Core Data Structures / Additions

No new entity types. Reuses `UnitDef` (units spec §4), `TerrainDef.defense_mod`
(core-data-model §4.12), `RouteStatus` + `CaravanRoute` (caravan spec). Combat-
only constants:

```rust
pub const FLANK_POSITIONING_BONUS: f32 = 0.25;   // attacker from advantageous tile, DD §10.1
pub const EXPOSED_POSITIONING_MULT: f32 = 0.90;  // attacker on Salt Flats (exposed) penalty
pub const ROUGH_RAIDER_BONUS: f32   = 1.25;     // Raider on Ridge raids better, caravan §6.7
pub const CITY_TILE_DEF_MOD: i8      = 0;         // base; +Fortress(3) added if specialized
pub const FORTRESS_CITY_DEF: i8      = 3;         // DD §10.2
pub const SIEGE_THREATEN: bool       = true;       // besieged city's outgoing routes Threatened
```

Terrain defense mods (DD §10.2, mirrored in `TERRAIN` table):

```text
Oasis 0, Dunes 0, SaltFlats -1 (exposed), Ridges +2 (defensive), City(Fortress +3)
```

## 5. Key Functions / API

```rust
/// Core auto-resolution between two units (used by move-into-enemy, siege, raid).
/// Mutates HP; may destroy/retreat; draws from state.rng. Returns events.
pub fn resolve_combat(state: &mut GameState, attacker: UnitId, defender: UnitId)
    -> Vec<GameEvent>;

/// Raider-vs-route contest: guard present => stat contest (this fn); absent =>
/// auto Threatened / Severed cascade (caravan spec §6.5). Calls resolve_combat
/// when a controlling Guard is adjacent.
pub fn resolve_raid_contest(state: &mut GameState, raider: UnitId, route: RouteId)
    -> Vec<GameEvent>;

/// City siege: Raider vs city (garrison def + Fortress). On success pop-=1 / capture.
pub fn resolve_city_raid(state: &mut GameState, raider: UnitId, city: CityId)
    -> Vec<GameEvent>;
```

## 6. Algorithms

### 6.1 Core odds formula (DD §10.1)

```rust
let atk = UNITS[attacker.kind].atk as f32;
let def = UNITS[defender.kind].def as f32;

let terrain_def = TERRAIN[defender_tile.terrain].defense_mod as f32;

let atk_pos = positioning_attacker(attacker_tile, defender_tile); // §6.2 (flank 1.25, Salt-Flats-exposed 0.90)
let def_pos = 1.0;                                              // defender positional (terrain already in def_mod)
let morale  = 1.0;                                              // reserved for post-MVP (DD §10.3); always 1.0 in MVP

let attack_power  = atk * atk_pos * morale;                    // atk_pos encodes attacker positioning (flank / Salt-Flats-exposed)
let defense_power = def * (1.0 + terrain_def) * def_pos;

let odds = attack_power / (attack_power + defense_power);
let roll = state.rng.next_f32();                                 // ADR-0006: only state.rng
if roll < odds {
    defender loses 1 HP; if defender.hp <= 0 -> destroyed/retreat
} else {
    attacker loses 1 HP; if attacker.hp  <= 0 -> destroyed/retreat
}
// Combat continues (re-roll) until one side is destroyed or the attacker
// auto-retreats at HP 1 (see Retreat, §6.6).
```

Terrain defense modifiers apply to the defender's tile; the attacker's tile effect is captured in atk_pos (flank / Salt-Flats-exposed), not a defense multiplier.

**HP loss model (deterministic-ish):** each exchange the loser-of-the-roll loses
exactly **1 HP**. Combat iterates (new `roll` each exchange) until one unit's HP
hits 0 (destroyed) or the attacker **auto-retreats** (the round that would drop
its HP to 1 triggers the retreat; see Retreat, §6.6). This keeps HP small ints
meaningful and fully RNG-driven.

### 6.2 Positioning > Numbers (DD §10.1, §10.3)

- **Attacker flank bonus:** if the attacker stands on a tile with a **defensive
  advantage over the defender's tile** (e.g. attacker on Ridge, defender on Dune),
  `atk_pos = 1.0 + FLANK_POSITIONING_BONUS (0.25)`.
- **Attacker exposed penalty:** if the attacker stands on **Salt Flats** (exposed),
  `atk_pos = EXPOSED_POSITIONING_MULT (0.90)`.
- **Local only:** only units **on or adjacent to the contested tile** fight (DD §10.3).
  A blob of 10 scattered Raiders loses to 2 Guards on a Ridge — no global stack,
  so positioning beats count. Defender `def_pos` is folded into `terrain_def` (Ridge
  +2, Salt Flats −1).
- **Terrain defense mod** applies to the **defender's tile** (where it defends):
  Ridges +2 ⇒ strong; Salt Flats −1 ⇒ exposed. The attacker's tile mod feeds
  `atk_pos` per the flank/exposed rules above.

### 6.3 Raider-vs-route contest (DD §8.4, DD #6 open)

Triggered by `resolve_raid_route` (units spec §6.5) when a Raider is on/adjacent
to an **exposed** route tile. **Recommended default (carried DD #6):**

```rust
if route tile HAS a controlling Guard adjacent/on it (caravan spec §6.6):
    // stat contest: Raider (atk) vs Guard (def), terrain-adjusted (caravan §6.7)
    // Raider attacker tile effect via atk_pos (§6.1): Ridge flank x1.25, Salt-Flats-exposed x0.90
    raider_atk_pos = match raider_tile.terrain {
        Ridges    => ROUGH_RAIDER_BONUS,        // 1.25 (flank from Ridge)
        SaltFlats => EXPOSED_POSITIONING_MULT,  // 0.90 (exposed on Salt Flats)
        _         => 1.0,
    };
    raider_atk_eff = UNITS[Raider].atk * raider_atk_pos;
    // Guard defender uses terrain defense mod (Salt Flats -1 -> easier raid)
    guard_def_eff  = UNITS[CaravanGuard].def * (1.0 + TERRAIN[guard_tile].defense_mod);
    odds_raider = raider_atk_eff / (raider_atk_eff + guard_def_eff)
    roll = state.rng.next_f32()
    if roll < odds_raider:
        // raid succeeds -> Threatened (1st) or Severed (2nd consecutive)
        cascade(route) ; emit RouteRaided{severed}
    else:
        // guard repels; route stays Active; (optional: guard takes 1 dmg)
        emit RouteRaided{severed:false}
else:
    // no defender -> auto cascade (no RNG needed for the state change itself)
    cascade(route) ; emit RouteRaided{severed}
```

`cascade(route)` = set `enemy_adjacent` marker so `recompute_routes` (caravan
spec §6.5) flips Active→Threatened (yields halved) on the 1st raid, and
Threatened→Severed on the 2nd **consecutive** turn with no controlling Guard.

> **DD #6 / OQ-3 (OPEN, recommended default):** when a contested route tile is
> targeted by raids from **different actors within one turn cycle**, the **first raid
> in actor order wins resolution**; subsequent raids in the same cycle re-evaluate
> against the post-raid state (turn-engine §6.6). Flagged for design sign-off.

### 6.4 City sieges (DD §10.4)

`resolve_city_raid(raider, city)`:
1. Defender effective def = `TERRAIN[city_tile].defense_mod (0)` + `FORTRESS_CITY_DEF(3)`
   **if** city is `Fortress` + any `Garrisoned` Guard's `def`.
2. Resolve `resolve_combat(raider, city_defense_aggregate)` — the city fights as a
   unit with `def` = the aggregate above, `hp` = `city.population` (each pop point
   is a "hit"). Attacker = Raider.
3. On attacker win: `city.population -= 1` (DD §10.4). If `population == 0` and
   the Raider occupies the city tile ⇒ **capture**: flip `owner`, re-evaluate its
   routes at next `advance_turn`.
4. **Routes during siege:** the besieged city's **outgoing routes become
   `Threatened`** while the siege is active (DD §10.4) — applied via the same
   `recompute_routes` control logic (an enemy unit adjacent to an exposed route tile
   of that city).
5. Emit `CityRaided{pop_lost}`.

### 6.5 RNG usage (ADR-0006)

- Every `roll` is exactly one `state.rng.next_f32()` draw, in **fixed order**
  (command resolution order → deterministic replays).
- No `thread_rng`, no `std::time`, no other RNG in `dcs-core`.
- Combat is the **only** place combat-spec draws RNG; route establishment/conomy do not.

### 6.6 Retreat (auto)

Combat is **fully automatic** — the player never chooses to retreat mid-fight.
The **ATTACKER auto-retreats** the moment a combat exchange would take its HP to
**1 remaining** (i.e., the round that would drop it to 0 HP instead stops the
attack; the attacker breaks off and falls back to the tile it attacked from — its
last safe tile / origin of the attack — at no extra move cost). This prevents
guaranteed mutual destruction while keeping resolution hands-off.

> **FIRST-PASS RULE — EXPECTED TO BE REVISITED AFTER THE MVP.** This auto-retreat
> threshold (retreat at HP 1) is a first-pass simplification, per the user's
> directive to keep combat changeable. Possible post-MVP revisions (explicit):
> - **(a)** Let the player *choose* to retreat via a prompt instead of (or in
>   addition to) the automatic rule.
> - **(b)** Raise the auto-retreat threshold so it triggers sooner — e.g., retreat
>   at a higher remaining HP (2+) rather than only at 1.
>
> The rule is intentionally lightweight now and is slated for tuning after MVP
> playtesting.

## 7. Edge Cases / Invariants

- **Local only:** units not on/adjacent to the contested tile never participate.
- **Attacker retreat (auto):** the attacker **auto-retreats to its origin / last
  safe tile at no extra move cost** when an exchange would drop its HP to 1 (see
  §6.6) — combat is fully automatic; the player does not choose mid-fight.
  Prevents guaranteed mutual destruction.
- **Destroyed unit:** removed from `units` (ID reserved, core-data-model §6); its
  `Patrolling`/`Garrisoned` control on a route/city is lifted (route re-evaluates).
- **Siege population floor:** Well Fort cannot starve below Pop 1 (economy spec);
  but a Raider *capture* at Pop 0 still flips it (capture is a combat outcome, not
  starvation) — note this interaction; both rules coexist.
- **Determinism:** identical `(seed, commands)` ⇒ identical combat outcomes; the
  `log`/`Revealed`/`Combat` events are reproducible.

## 8. Acceptance Criteria / Unit-Test Checklist

- [ ] Attack power = Atk × atk_pos; defense power = Def × (1+TERRAIN[defender].defense_mod); odds = a/(a+d); morale = 1.0, no terrain_atk term.
- [ ] Ridge defender (+2) and Salt-Flats defender (−1) shift odds per table.
- [ ] Attacker on Ridge vs Dune defender gets ×1.25; on Salt Flats ×0.90.
- [ ] Only on/adjacent units fight (local combat; no global stack).
- [ ] Each roll loses exactly 1 HP; combat ends on HP 0 or attacker retreat.
- [ ] Raid with controlling Guard ⇒ stat contest using ROUGH_RAIDER_BONUS / Salt-Flats −1; guard wins ⇒ route stays Active.
- [ ] Raid with NO defender ⇒ auto Threatened (1st) / Severed (2nd consecutive).
- [ ] City siege: Fortress +3 def + Garrisoned Guard def; win ⇒ pop−1; Pop 0 + occupy ⇒ capture (owner flip).
- [ ] Besieged city's outgoing routes Threatened during siege.
- [ ] All rolls from `state.rng`; same seed+commands ⇒ identical outcome.
- [ ] Contested raid resolves first-come in actor order (DD #6 recommended default).

## 9. References

- Design: DD §10 (combat) — §10.1 auto-resolution formula, §10.2 terrain mods, §10.3 morale/local, §10.4 sieges, §10.5 route raids.
- Architecture: ARCH §5.2 (combat in resolution), §16 (errors/events), §5.3 (contested raid note).
- ADRs: ADR-0004 (Command-only), ADR-0006 (RNG in state), ADR-0003 (pure core).
- Related specs: `gameplay-units-movement.md` (triggers combat on move/siege/raid), `gameplay-caravan-routes.md` (`RaidRoute`, control, state machine, terrain raid difficulty), `gameplay-cities.md` (Fortress def, siege capture, Well Fort floor), `foundation-core-data-model.md` (`TerrainDef`, `UnitDef`), `foundation-turn-engine.md` (`Combat`/`RouteRaided`/`CityRaided` events, contested-raid order).

## 10. Open Questions (carried, not resolved)

- **DD #6 / OQ-3:** Contested-raid *resolution order* across actors — **recommended
  default = first-come in actor order**; flagged for design sign-off. The raid
  *contest odds* (§6.3) are otherwise fully specified.
- **DD #3 / OQ-1:** flank bonus (0.25), synergy, terrain mods are first-pass tables.
- **DD §10.3 morale:** optional/off for MVP; formula keeps `morale = 1.0` slot.
- **Capture vs Well-Fort floor:** both rules coexist (capture is combat, floor is
  starvation); noted as an interaction to watch in playtest, not a contradiction.
