//! The `dcs-core` data model: the single mutable aggregate [`GameState`] plus
//! entities, ID allocators, accessor helpers, and balance tables.
//!
//! # Determinism
//!
//! `GameState` uses [`fxhash::FxHashMap`] / [`fxhash::FxHashSet`] for its index
//! and discovery collections. `fxhash` does not implement `serde`'s
//! `Serialize`/`Deserialize` on its own, so the affected fields use the
//! [`fxhash_map`] / [`fxhash_set`] helper modules (via `#[serde(with = "..")]`)
//! which (de)serialize as a `Vec` of pairs/elements. This keeps a deterministic
//! on-disk layout (we control the ordering) while remaining `serde` compatible
//! across `serde_json`, `postcard`, and `bincode`.

use crate::hex::HexCoord;
use crate::scenario::{AiPersonality, Difficulty};
use crate::{
    BuildingKind, CityId, CitySpecialization, GameEvent, PlayerId, RelicId, RouteId, RouteStatus,
    SAVE_VERSION, TileId, UnitId, UnitKind,
};
use fxhash::{FxHashMap, FxHashSet};
use nanorand::{Rng, SeedableRng, WyRand};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Seeded PRNG (ADR-0006)
// ---------------------------------------------------------------------------
//
// `nanorand` RNG state does not serialize directly. We therefore persist only a
// `seed` and a `counter` (number of draws consumed) and reconstruct the RNG from
// the seed, fast-forwarding by re-drawing `counter` times. This guarantees that
// a save → load → continue stream produces a byte-identical random sequence to
// a never-serialized stream.

/// A deterministic, serializable pseudo-random number generator wrapper.
///
/// Only the seed and the draw counter are stored; the underlying `nanorand`
/// generator is rebuilt on demand. See the module docs for the determinism
/// rationale.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeededRng {
    seed: u64,
    counter: u64,
}

impl SeededRng {
    /// Create a new seeded generator starting at draw 0.
    pub fn new(seed: u64) -> Self {
        Self { seed, counter: 0 }
    }

    /// Rebuild the underlying generator from the seed and fast-forward past
    /// every draw already consumed.
    fn rng(&self) -> WyRand {
        let mut rng = WyRand::new();
        rng.reseed(self.seed.to_le_bytes());
        for _ in 0..self.counter {
            let _: u32 = rng.generate();
        }
        rng
    }

    /// Draw the next `u32` from the stream and advance the counter.
    pub fn next_u32(&mut self) -> u32 {
        let mut rng = self.rng();
        let v = rng.generate();
        self.counter += 1;
        v
    }

    /// Draw the next `f32` in the range `[0.0, 1.0)` from the stream.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }

    /// Draw a `u32` in the range `[0, max)` (for indexing).
    pub fn next_range(&mut self, max: u32) -> u32 {
        self.next_u32() % max
    }
}

// ---------------------------------------------------------------------------
// Value types / catalog enums (defined here; protocol enums are re-exported)
// ---------------------------------------------------------------------------

/// A player's stock of the three tradeable resources.
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Stockpiles {
    /// Life-sustaining water.
    pub water: u32,
    /// Trade currency.
    pub wealth: u32,
    /// Diplomatic / founding power.
    pub influence: u32,
}

/// The three resource kinds, used for generic resource operations.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// Life-sustaining water.
    Water,
    /// Trade currency.
    Wealth,
    /// Diplomatic / founding power.
    Influence,
}

/// Visual identity of a player.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayerColor {
    /// Default sand color.
    #[default]
    Sand,
    /// Crimson.
    Crimson,
    /// Teal.
    Teal,
    /// Violet.
    Violet,
}

/// Terrain classification of a tile.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TerrainType {
    /// Fertile, defensible, foundable.
    Oasis,
    /// Open desert. Default.
    #[default]
    Dunes,
    /// Mineral-rich flats.
    SaltFlats,
    /// Defensive high ground.
    Ridges,
    /// Ancient ruins (wealth).
    Ruins,
}

/// High-level controller kind for a player.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerKind {
    /// A human-controlled player.
    Human,
    /// An AI-controlled player with a personality and difficulty.
    Ai {
        /// Behavioural archetype.
        personality: AiPersonality,
        /// Skill level.
        difficulty: Difficulty,
    },
}

/// Per-unit tactical stance.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UnitAbility {
    /// No special stance. Default.
    #[default]
    None,
    /// Patrolling a tile (route guard).
    Patrolling,
    /// Garrisoned in a city.
    Garrisoned,
}

/// The phase of the current turn's resolution cycle.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TurnPhase {
    /// Order intake. Default.
    #[default]
    Order,
    /// Command resolution.
    Resolution,
    /// Income / upkeep application.
    Income,
    /// Cleanup before the next turn.
    EndOfTurn,
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

/// A single map cell.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tile {
    /// Stable identity of this tile.
    pub id: TileId,
    /// Axial coordinate of this tile.
    pub coord: HexCoord,
    /// Terrain classification.
    pub terrain: TerrainType,
    /// Whether this tile is a relic site.
    pub is_relic_site: bool,
    /// Controlling player, if any.
    pub owner: Option<PlayerId>,
    /// Constructed improvement, if any.
    pub improvement: Option<BuildingKind>,
}

/// A founded city.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct City {
    /// Stable identity of this city.
    pub id: CityId,
    /// Owning player.
    pub owner: PlayerId,
    /// Tile this city sits on.
    pub tile: TileId,
    /// Current population.
    pub population: u32,
    /// Active specialization, if any.
    pub specialization: Option<CitySpecialization>,
    /// Constructed buildings.
    pub buildings: Vec<BuildingKind>,
    /// Local stockpiles.
    pub stockpiles: Stockpiles,
    /// Number of caravan route slots.
    pub route_slots: u8,
    /// Turns until the next growth tick.
    pub growth_timer: u32,
}

/// A mobile unit.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Unit {
    /// Stable identity of this unit.
    pub id: UnitId,
    /// Owning player.
    pub owner: PlayerId,
    /// Unit archetype.
    pub kind: UnitKind,
    /// Tile this unit occupies.
    pub tile: TileId,
    /// Remaining hit points.
    pub hp: u32,
    /// Movement points left this turn.
    pub moves_left: u8,
    /// Current tactical stance.
    pub ability: UnitAbility,
}

/// A caravan trade route between two cities.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CaravanRoute {
    /// Stable identity of this route.
    pub id: RouteId,
    /// Owning player.
    pub owner: PlayerId,
    /// Connected endpoint cities.
    pub endpoints: (CityId, CityId),
    /// Ordered tile path between endpoints.
    pub path: Vec<TileId>,
    /// Operational status.
    pub status: RouteStatus,
    /// Cached path length in tiles.
    pub length: u32,
    /// Per-turn upkeep cost.
    pub upkeep: u8,
    /// Consecutive turns spent threatened.
    pub consecutive_threatened: u8,
}

/// A player (human or AI).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Player {
    /// Stable identity of this player.
    pub id: PlayerId,
    /// Controller kind.
    pub kind: PlayerKind,
    /// Visual identity.
    pub color: PlayerColor,
    /// Global resource pool.
    pub resources: Stockpiles,
    /// Tiles revealed to this player (fog of war).
    #[serde(with = "fxhash_set")]
    pub discovered: FxHashSet<TileId>,
    /// Whether this player has been eliminated.
    pub defeated: bool,
}

/// A relic site and its current holder.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Relic {
    /// Stable identity of this relic.
    pub id: RelicId,
    /// Tile this relic occupies.
    pub tile: TileId,
    /// Current holder, if any.
    pub holder: Option<PlayerId>,
    /// Consecutive turns held by the current holder.
    pub consecutive_turns_held: u32,
}

// ---------------------------------------------------------------------------
// Victory tracking
// ---------------------------------------------------------------------------

/// Aggregate progress toward the victory conditions.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct VictoryTracker {
    /// Oases controlled per player.
    #[serde(with = "fxhash_map")]
    pub oases_controlled: FxHashMap<PlayerId, u32>,
    /// Prestige (wealth-based) score per player.
    #[serde(with = "fxhash_map")]
    pub prestige_score: FxHashMap<PlayerId, u32>,
    /// Relic → current holder, for hold-timers.
    #[serde(with = "fxhash_map")]
    pub relic_timers: FxHashMap<RelicId, PlayerId>,
}

// ---------------------------------------------------------------------------
// GameState aggregate
// ---------------------------------------------------------------------------

/// The single mutable aggregate of the entire simulation.
///
/// All mutations flow through commands validated against this state. World
/// generation (`map`) and the turn resolver (`turn`) populate the entities and
/// advance the phase; this struct owns the canonical, serializable snapshot.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameState {
    /// Mirrors [`SAVE_VERSION`] for migration gating.
    pub version: u32,
    /// Scenario configuration this game was created from.
    pub scenario: crate::scenario::ScenarioConfig,
    /// Deterministic RNG for all procedural draws.
    pub rng: SeededRng,
    /// Current turn number (starts at 1).
    pub turn: u32,
    /// The player whose turn it currently is.
    pub current_actor: PlayerId,
    /// Current resolution phase.
    pub phase: TurnPhase,
    /// All tiles, indexed by [`Tile::id`].
    pub tiles: Vec<Tile>,
    /// Reverse lookup from coordinate to tile id.
    #[serde(with = "fxhash_map")]
    pub tile_index: FxHashMap<HexCoord, TileId>,
    /// All cities, indexed by [`City::id`].
    pub cities: Vec<City>,
    /// All units, indexed by [`Unit::id`].
    pub units: Vec<Unit>,
    /// All caravan routes, indexed by [`CaravanRoute::id`].
    pub routes: Vec<CaravanRoute>,
    /// All players, indexed by [`Player::id`].
    pub players: Vec<Player>,
    /// All relics, indexed by [`Relic::id`].
    pub relics: Vec<Relic>,
    /// Victory progress bookkeeping.
    pub victory: VictoryTracker,
    /// Chronological event log (also the replay feed).
    pub log: Vec<GameEvent>,
    // Monotonic id counters (private; never collide within a game).
    next_tile_id: u32,
    next_city_id: u32,
    next_unit_id: u32,
    next_route_id: u32,
    next_player_id: u32,
    next_relic_id: u32,
}

impl GameState {
    /// Construct an empty game-state shell for the given scenario and seed.
    ///
    /// World generation (in `map`) fills the entities; this builds the
    /// immutable frame (version, rng, turn bookkeeping, zeroed counters).
    pub fn new(scenario: crate::scenario::ScenarioConfig, seed: u64) -> Self {
        Self {
            version: SAVE_VERSION,
            scenario,
            rng: SeededRng::new(seed),
            turn: 1,
            current_actor: PlayerId(0),
            phase: TurnPhase::Order,
            tiles: Vec::new(),
            tile_index: FxHashMap::default(),
            cities: Vec::new(),
            units: Vec::new(),
            routes: Vec::new(),
            players: Vec::new(),
            relics: Vec::new(),
            victory: VictoryTracker::default(),
            log: Vec::new(),
            next_tile_id: 0,
            next_city_id: 0,
            next_unit_id: 0,
            next_route_id: 0,
            next_player_id: 0,
            next_relic_id: 0,
        }
    }

    // --- ID allocators (used by world-gen and the resolver); crate-visible so
    //     `map` and `turn` can mint entities without reaching into the private
    //     counter fields. ---

    /// Allocate the next unique [`TileId`].
    // Part of the allocator API; used in later phases.
    #[allow(dead_code)]
    pub(crate) fn alloc_tile_id(&mut self) -> TileId {
        let id = TileId(self.next_tile_id);
        self.next_tile_id += 1;
        id
    }

    /// Allocate the next unique [`CityId`].
    pub(crate) fn alloc_city_id(&mut self) -> CityId {
        let id = CityId(self.next_city_id);
        self.next_city_id += 1;
        id
    }

    /// Allocate the next unique [`UnitId`].
    pub(crate) fn alloc_unit_id(&mut self) -> UnitId {
        let id = UnitId(self.next_unit_id);
        self.next_unit_id += 1;
        id
    }

    /// Allocate the next unique [`RouteId`].
    // Part of the public ID-allocator API for future phases; not yet consumed
    // anywhere in Phase 1, so it would otherwise trip `dead_code`.
    #[allow(dead_code)]
    pub(crate) fn alloc_route_id(&mut self) -> RouteId {
        let id = RouteId(self.next_route_id);
        self.next_route_id += 1;
        id
    }

    /// Allocate the next unique [`PlayerId`].
    // Part of the allocator API; used in later phases.
    #[allow(dead_code)]
    pub(crate) fn alloc_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.next_player_id);
        self.next_player_id += 1;
        id
    }

    /// Allocate the next unique [`RelicId`].
    // Part of the allocator API; used in later phases.
    #[allow(dead_code)]
    pub(crate) fn alloc_relic_id(&mut self) -> RelicId {
        let id = RelicId(self.next_relic_id);
        self.next_relic_id += 1;
        id
    }
}

// ---------------------------------------------------------------------------
// Accessor helpers (free functions, per spec)
// ---------------------------------------------------------------------------

/// Look up a tile by coordinate, if one exists at that position.
pub fn tile_at(state: &GameState, coord: HexCoord) -> Option<&Tile> {
    state.tile_index.get(&coord).map(|id| {
        state
            .tiles
            .get(id.0 as usize)
            .expect("invariant: tile_index points to a live tile")
    })
}

/// Look up a city by id (invariant: must exist).
pub fn city(state: &GameState, id: CityId) -> &City {
    state
        .cities
        .get(id.0 as usize)
        .expect("invariant: city id must reference a live city")
}

/// Look up a unit by id (invariant: must exist).
pub fn unit(state: &GameState, id: UnitId) -> &Unit {
    state
        .units
        .get(id.0 as usize)
        .expect("invariant: unit id must reference a live unit")
}

/// Iterate over all units currently on the given tile.
pub fn units_on(state: &GameState, tile: TileId) -> impl Iterator<Item = &Unit> {
    state.units.iter().filter(move |u| u.tile == tile)
}

/// Iterate over all routes whose path includes the given tile.
pub fn routes_through(state: &GameState, tile: TileId) -> impl Iterator<Item = &CaravanRoute> {
    state.routes.iter().filter(move |r| r.path.contains(&tile))
}

/// Iterate over all cities owned by the given player.
pub fn cities_of(state: &GameState, player: PlayerId) -> impl Iterator<Item = &City> {
    state.cities.iter().filter(move |c| c.owner == player)
}

/// Return the owning player of a city (invariant: city must exist).
pub fn city_owner(state: &GameState, city_id: CityId) -> PlayerId {
    city(state, city_id).owner
}

/// Whether a city sits on an oasis tile (invariant: city/tile must exist).
pub fn assert_city_on_oasis(c: &City, state: &GameState) -> bool {
    let tile = state
        .tiles
        .get(c.tile.0 as usize)
        .expect("invariant: city tile must exist");
    tile.terrain == TerrainType::Oasis
}

// ---------------------------------------------------------------------------
// Balance tables
// ---------------------------------------------------------------------------

/// Static movement / economy parameters for a terrain type.
pub struct TerrainDef {
    /// Movement points to enter.
    pub move_cost: u8,
    /// Defensive modifier (may be negative).
    pub defense_mod: i8,
    /// Base water yield.
    pub water: u8,
    /// Base wealth yield.
    pub wealth: u8,
}

/// Per-terrain balance, indexed by [`TerrainType`] discriminant order
/// (Oasis, Dunes, SaltFlats, Ridges, Ruins).
pub const TERRAIN: &[TerrainDef; 5] = &[
    TerrainDef {
        move_cost: 1,
        defense_mod: 0,
        water: 2,
        wealth: 1,
    }, // Oasis
    TerrainDef {
        move_cost: 1,
        defense_mod: 0,
        water: 0,
        wealth: 1,
    }, // Dunes
    TerrainDef {
        move_cost: 2,
        defense_mod: 0,
        water: 0,
        wealth: 3,
    }, // SaltFlats
    TerrainDef {
        move_cost: 3,
        defense_mod: 2,
        water: 0,
        wealth: 1,
    }, // Ridges
    TerrainDef {
        move_cost: 1,
        defense_mod: 0,
        water: 0,
        wealth: 4,
    }, // Ruins
];

/// Static combat / movement parameters for a unit archetype.
pub struct UnitDef {
    /// Movement points per turn.
    pub moves: u8,
    /// Attack strength.
    pub atk: u8,
    /// Defense strength.
    pub def: u8,
    /// Max hit points.
    pub hp: u8,
    /// Per-turn upkeep cost.
    pub upkeep: u8,
    /// Vision radius (tiles).
    pub sight: u8,
}

/// Per-unit balance, indexed by [`UnitKind`] discriminant order
/// (Scout, CaravanGuard, Raider).
pub const UNITS: &[UnitDef; 3] = &[
    UnitDef {
        moves: 3,
        atk: 1,
        def: 1,
        hp: 3,
        upkeep: 1,
        sight: 2,
    }, // Scout
    UnitDef {
        moves: 2,
        atk: 2,
        def: 3,
        hp: 5,
        upkeep: 2,
        sight: 1,
    }, // CaravanGuard
    UnitDef {
        moves: 3,
        atk: 4,
        def: 2,
        hp: 4,
        upkeep: 2,
        sight: 2,
    }, // Raider
];

/// Influence required to found a city.
pub const FOUND_CITY_INFLUENCE: u32 = 10;

/// Look up the balance record for a terrain type.
pub fn terrain_def(t: TerrainType) -> &'static TerrainDef {
    &TERRAIN[t as usize]
}

/// Look up the balance record for a unit kind.
pub fn unit_def(k: UnitKind) -> &'static UnitDef {
    &UNITS[k as usize]
}

/// Starting influence granted to each player at world-gen.
pub fn starting_influence() -> u32 {
    FOUND_CITY_INFLUENCE
}

/// Starting wealth granted to each player at world-gen.
pub fn starting_wealth() -> u32 {
    10
}

// ---------------------------------------------------------------------------
// serde helpers for fxhash collections
// ---------------------------------------------------------------------------
//

// `fxhash` does not implement `Serialize`/`Deserialize`. These modules
// (de)serialize an `FxHashMap`/`FxHashSet` as a `Vec`, giving us a stable,
// deterministic on-disk layout we fully control.

/// (De)serialize an [`FxHashMap<K, V>`] as a `Vec<(K, V)>`.
pub mod fxhash_map {
    use super::FxHashMap;
    use serde::Serialize;
    use serde::de::{Deserialize, Deserializer};
    use serde::ser::Serializer;

    /// Serialize the map as a vector of key/value pairs.
    pub fn serialize<K, V, S>(map: &FxHashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        let entries: Vec<(&K, &V)> = map.iter().collect();
        entries.serialize(serializer)
    }

    /// Deserialize a vector of key/value pairs back into an [`FxHashMap`].
    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<FxHashMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Eq + std::hash::Hash,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let entries: Vec<(K, V)> = Vec::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

/// (De)serialize an [`FxHashSet<T>`] as a `Vec<T>`.
pub mod fxhash_set {
    use super::FxHashSet;
    use serde::Serialize;
    use serde::de::{Deserialize, Deserializer};
    use serde::ser::Serializer;

    /// Serialize the set as a vector of elements.
    pub fn serialize<T, S>(set: &FxHashSet<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        let entries: Vec<&T> = set.iter().collect();
        entries.serialize(serializer)
    }

    /// Deserialize a vector of elements back into an [`FxHashSet`].
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<FxHashSet<T>, D::Error>
    where
        T: Deserialize<'de> + Eq + std::hash::Hash,
        D: Deserializer<'de>,
    {
        let entries: Vec<T> = Vec::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_str, to_string};

    #[test]
    fn seeded_rng_is_deterministic() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        let seq_a: Vec<u32> = (0..10).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..10).map(|_| b.next_u32()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn seeded_rng_next_f32_in_range() {
        let mut r = SeededRng::new(7);
        for _ in 0..100 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f), "f32 out of range: {f}");
        }
    }

    #[test]
    fn seeded_rng_next_range_bounded() {
        let mut r = SeededRng::new(99);
        for _ in 0..1000 {
            let v = r.next_range(16);
            assert!(v < 16, "next_range exceeded bound: {v}");
        }
    }

    #[test]
    fn seeded_rng_resume_matches_fresh() {
        // A stream that is serialized at draw 5 and resumed must continue
        // identically to a fresh never-serialized stream.
        let mut fresh = SeededRng::new(1234);
        let first_5: Vec<u32> = (0..5).map(|_| fresh.next_u32()).collect();

        let mut serialized = SeededRng::new(1234);
        let pre: Vec<u32> = (0..5).map(|_| serialized.next_u32()).collect();
        let json = to_string(&serialized).expect("serialize rng");
        let mut resumed: SeededRng = from_str(&json).expect("deserialize rng");

        // Consume the rest from the fresh stream and the resumed stream.
        let rest_fresh: Vec<u32> = (0..20).map(|_| fresh.next_u32()).collect();
        let rest_resumed: Vec<u32> = (0..20).map(|_| resumed.next_u32()).collect();

        assert_eq!(first_5, pre);
        assert_eq!(rest_fresh, rest_resumed);
    }

    #[test]
    fn balance_tables_have_expected_lengths() {
        assert_eq!(TERRAIN.len(), 5);
        assert_eq!(UNITS.len(), 3);
    }

    #[test]
    fn balance_lookup_indexes_all_variants() {
        for t in [
            TerrainType::Oasis,
            TerrainType::Dunes,
            TerrainType::SaltFlats,
            TerrainType::Ridges,
            TerrainType::Ruins,
        ] {
            let _ = terrain_def(t);
        }
        for k in [UnitKind::Scout, UnitKind::CaravanGuard, UnitKind::Raider] {
            let _ = unit_def(k);
        }
    }

    #[test]
    fn new_game_state_initial_frame() {
        let cfg = crate::scenario::ScenarioConfig::default();
        let s = GameState::new(cfg, 7);
        assert_eq!(s.turn, 1);
        assert_eq!(s.current_actor, PlayerId(0));
        assert_eq!(s.phase, TurnPhase::Order);
        assert_eq!(s.version, SAVE_VERSION);
        assert!(s.tiles.is_empty());
        assert!(s.log.is_empty());
    }

    #[test]
    fn allocators_never_collide() {
        let cfg = crate::scenario::ScenarioConfig::default();
        let mut s = GameState::new(cfg, 1);
        let mut seen = FxHashSet::default();
        for _ in 0..1000 {
            let id = s.alloc_tile_id();
            assert!(seen.insert(id), "duplicate tile id allocated: {id:?}");
        }
        assert_eq!(s.next_tile_id, 1000);
    }

    #[test]
    fn game_state_serde_round_trip() {
        let cfg = crate::scenario::ScenarioConfig::default();
        let mut s = GameState::new(cfg, 42);
        // Exercise the fxhash serde paths.
        s.tile_index.insert(HexCoord { q: 0, r: 0 }, TileId(0));
        s.victory.oases_controlled.insert(PlayerId(0), 3);
        s.victory.prestige_score.insert(PlayerId(1), 12);
        s.victory.relic_timers.insert(RelicId(0), PlayerId(0));
        let mut player = Player {
            id: PlayerId(0),
            kind: PlayerKind::Human,
            color: PlayerColor::Sand,
            resources: Stockpiles::default(),
            discovered: FxHashSet::default(),
            defeated: false,
        };
        player.discovered.insert(TileId(0));
        s.players.push(player);
        // Also exercise SeededRng serialization via a draw.
        let _ = s.rng.next_u32();

        let json = to_string(&s).expect("serialize GameState");
        let back: GameState = from_str(&json).expect("deserialize GameState");
        // `GameState` does not derive `PartialEq` (it holds an RNG and fxhash
        // collections); compare the observable, serializable fields instead.
        assert_eq!(back.version, SAVE_VERSION);
        assert_eq!(back.turn, 1);
        assert_eq!(back.phase, TurnPhase::Order);
        assert_eq!(
            back.tile_index.get(&HexCoord { q: 0, r: 0 }),
            Some(&TileId(0))
        );
        assert_eq!(back.victory.oases_controlled.get(&PlayerId(0)), Some(&3));
        assert_eq!(back.victory.prestige_score.get(&PlayerId(1)), Some(&12));
        assert_eq!(
            back.victory.relic_timers.get(&RelicId(0)),
            Some(&PlayerId(0))
        );
        assert_eq!(back.players.len(), 1);
        assert!(back.players[0].discovered.contains(&TileId(0)));
    }
}
