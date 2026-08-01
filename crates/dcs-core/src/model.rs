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

use crate::ai::{
    AiParams, CAT_BUILD, CAT_CONNECT, CAT_DEFEND, CAT_EXPAND, CAT_RAID, CAT_SCOUT, ScoredAction,
    Situation,
};
use crate::hex::HexCoord;
use crate::scenario::{AiPersonality, Difficulty};
use crate::serialize::{SaveError, SaveFormat};
use crate::traits::{BuildingKindExt, TerrainTypeDef, UnitKindExt};
use crate::{
    BuildingKind, CityId, CitySpecialization, Command, GameEvent, PlayerId, RejectReason, RelicId,
    RouteId, RouteStatus, SAVE_VERSION, TileId, UnitId, UnitKind, VersionedSave, VictoryKind,
};
use fxhash::{FxHashMap, FxHashSet};
use nanorand::{Rng, WyRand};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::path::Path;

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
        // `new_seed` builds directly from our u64, unlike `new()` (which
        // pulls system/browser entropy before being immediately discarded
        // by a reseed — dead weight natively, and a genuine wasm32 problem:
        // it requires JS glue this project deliberately doesn't generate).
        let mut rng = WyRand::new_seed(self.seed);
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

impl fmt::Display for TerrainType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oasis => write!(f, "Oasis"),
            Self::Dunes => write!(f, "Dunes"),
            Self::SaltFlats => write!(f, "SaltFlats"),
            Self::Ridges => write!(f, "Ridges"),
            Self::Ruins => write!(f, "Ruins"),
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Water => write!(f, "Water"),
            Self::Wealth => write!(f, "Wealth"),
            Self::Influence => write!(f, "Influence"),
        }
    }
}

impl fmt::Display for PlayerColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sand => write!(f, "Sand"),
            Self::Crimson => write!(f, "Crimson"),
            Self::Teal => write!(f, "Teal"),
            Self::Violet => write!(f, "Violet"),
        }
    }
}

impl fmt::Display for TurnPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Order => write!(f, "Order"),
            Self::Resolution => write!(f, "Resolution"),
            Self::Income => write!(f, "Income"),
            Self::EndOfTurn => write!(f, "EndOfTurn"),
        }
    }
}

impl fmt::Display for Stockpiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Wealth: {}, Water: {}, Influence: {}",
            self.wealth, self.water, self.influence
        )
    }
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

/// An order queued in a city's production queue.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum QueuedOrder {
    /// Build a building in the city.
    Build(BuildingKind),
    /// Train a unit in the city.
    Train(UnitKind),
    /// Specialize the city.
    Specialize(CitySpecialization),
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
    /// Production queue of pending orders.
    pub queue: VecDeque<QueuedOrder>,
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

// --- Income deltas (intermediate breakdown before applying to state) ---

/// Intermediate breakdown of income deltas before applying to state.
struct IncomeDeltas {
    water: i32,
    wealth: i32,
    influence: i32,
}

impl IncomeDeltas {
    fn new() -> Self {
        Self {
            water: 0,
            wealth: 0,
            influence: 0,
        }
    }
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

    // --- Accessor helpers ---

    /// Look up a tile by coordinate, if one exists at that position.
    pub fn tile_at(&self, coord: HexCoord) -> Option<&Tile> {
        self.tile_index.get(&coord).map(|id| {
            self.tiles
                .get(id.0 as usize)
                .expect("invariant: tile_index points to a live tile")
        })
    }

    /// Look up a city by id (invariant: must exist).
    pub fn city(&self, id: CityId) -> &City {
        self.cities
            .get(id.0 as usize)
            .expect("invariant: city id must reference a live city")
    }

    /// Look up a unit by id (invariant: must exist).
    pub fn unit(&self, id: UnitId) -> &Unit {
        self.units
            .get(id.0 as usize)
            .expect("invariant: unit id must reference a live unit")
    }

    /// Iterate over all units currently on the given tile.
    pub fn units_on(&self, tile: TileId) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(move |u| u.tile == tile)
    }

    /// Iterate over all routes whose path includes the given tile.
    pub fn routes_through(&self, tile: TileId) -> impl Iterator<Item = &CaravanRoute> {
        self.routes.iter().filter(move |r| r.path.contains(&tile))
    }

    /// Iterate over all cities owned by the given player.
    pub fn cities_of(&self, player: PlayerId) -> impl Iterator<Item = &City> {
        self.cities.iter().filter(move |c| c.owner == player)
    }

    /// Return the owning player of a city (invariant: city must exist).
    pub fn city_owner(&self, city_id: CityId) -> PlayerId {
        self.city(city_id).owner
    }

    // --- Serialization / persistence ---

    /// Serialize this game state to bytes using the specified format.
    ///
    /// The state is wrapped in a [`VersionedSave`] envelope carrying the current
    /// [`SAVE_VERSION`] before encoding, so every saved file is self-describing.
    ///
    /// # Arguments
    /// * `fmt` - Output format (JSON, Postcard, or Bincode)
    ///
    /// # Returns
    /// `Ok(Vec<u8>)` with the serialized bytes, or `Err(SaveError)` on failure.
    pub fn serialize_to(&self, fmt: SaveFormat) -> Result<Vec<u8>, SaveError> {
        let env = VersionedSave {
            version: SAVE_VERSION,
            payload: self.clone(),
        };
        match fmt {
            SaveFormat::Json => {
                serde_json::to_vec(&env).map_err(|e| SaveError::Serde(e.to_string()))
            }
            SaveFormat::Postcard => {
                postcard::to_stdvec(&env).map_err(|e| SaveError::Serde(e.to_string()))
            }
            SaveFormat::Bincode => {
                bincode::serialize(&env).map_err(|e| SaveError::Serde(e.to_string()))
            }
        }
    }

    /// Deserialize a game state from bytes.
    ///
    /// Decodes the [`VersionedSave`] envelope, rejects saves newer than our schema
    /// version, and runs the (currently empty) forward migration loop before
    /// returning the payload.
    ///
    /// # Arguments
    /// * `bytes` - Serialized game state bytes
    /// * `fmt` - Format to use for deserialization
    ///
    /// # Returns
    /// `Ok(GameState)` on success, or `Err(SaveError)` if the data is invalid
    /// or uses an unsupported format.
    pub fn from_bytes(bytes: &[u8], fmt: SaveFormat) -> Result<GameState, SaveError> {
        let env: VersionedSave<GameState> = match fmt {
            SaveFormat::Json => {
                serde_json::from_slice(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
            }
            SaveFormat::Postcard => {
                postcard::from_bytes(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
            }
            SaveFormat::Bincode => {
                bincode::deserialize(bytes).map_err(|e| SaveError::Serde(e.to_string()))?
            }
        };

        if env.version > SAVE_VERSION {
            return Err(SaveError::VersionTooNew(env.version, SAVE_VERSION));
        }

        let mut payload = env.payload;
        // Migration loop: while the payload is older than the current schema,
        // migrate it one version forward. No older versions exist yet
        // (SAVE_VERSION == 1), so this is currently a no-op.
        while payload.version < SAVE_VERSION {
            payload = migrate(payload.version, payload)?;
        }
        Ok(payload)
    }

    /// Write this game state to `path` in the given [`SaveFormat`].
    ///
    /// # Arguments
    /// * `path` - Filesystem path to write to
    /// * `fmt` - Output format (JSON, Postcard, or Bincode)
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(SaveError)` on I/O or serialization failure.
    pub fn save(&self, path: impl AsRef<Path>, fmt: SaveFormat) -> Result<(), SaveError> {
        let bytes = self.serialize_to(fmt)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Read a [`GameState`] from `path`, auto-detecting the format from the
    /// file extension.
    ///
    /// - `json` -> [`SaveFormat::Json`]
    /// - `postcard` / `bin` -> [`SaveFormat::Postcard`]
    /// - `bincode` -> [`SaveFormat::Bincode`]
    /// - anything else -> [`SaveFormat::Postcard`] (the compact default)
    ///
    /// # Arguments
    /// * `path` - Filesystem path to read from
    ///
    /// # Returns
    /// `Ok(GameState)` on success, or `Err(SaveError)` on I/O or deserialization failure.
    pub fn load(path: impl AsRef<Path>) -> Result<GameState, SaveError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let fmt = match path.extension().and_then(|s| s.to_str()) {
            Some("json") => SaveFormat::Json,
            Some("postcard") | Some("bin") => SaveFormat::Postcard,
            Some("bincode") => SaveFormat::Bincode,
            _ => SaveFormat::Postcard, // unknown extension -> postcard default
        };
        GameState::from_bytes(&bytes, fmt)
    }

    /// Write a human-readable JSON debug save to `path`.
    ///
    /// # Arguments
    /// * `path` - Filesystem path to write to
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(SaveError)` on I/O or serialization failure.
    pub fn save_debug(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        self.save(path, SaveFormat::Json)
    }

    // ------------------------------------------------------------------
    // Economy (moved from economy.rs free functions)
    // ------------------------------------------------------------------

    /// Per-turn upkeep cost (Wealth) for each unit kind, indexed by
    /// [`UnitKind::index`]: Scout, CaravanGuard, Raider.
    ///
    /// These values differ from the `UnitDef.upkeep` balance table and represent
    /// the upkeep drawn during the Income phase (economy spec §6.1 step 5).
    pub const UNIT_UPKEEP: [i32; 3] = [0, 1, 1];

    /// Is `city` isolated (zero active routes)?
    ///
    /// A city is isolated when it has **no** routes with [`RouteStatus::Active`]
    /// whose endpoints include this city. Severed routes do **not** count — this
    /// is the core "isolation" rule (DD §8.5).
    pub fn is_city_isolated(&self, city: CityId) -> bool {
        !self.routes.iter().any(|r| {
            r.owner == self.cities[city.0 as usize].owner
                && r.status == RouteStatus::Active
                && (r.endpoints.0 == city || r.endpoints.1 == city)
        })
    }

    /// Total Wealth produced by the **active** route network of `player` this
    /// turn. Used by income + victory V2. Encapsulates the route-yield formula
    /// so combat/AI modules can read it without duplicating math.
    pub fn network_wealth_yield(&self, player: PlayerId) -> u32 {
        self.routes
            .iter()
            .filter(|r| r.owner == player && r.status != RouteStatus::Severed)
            .map(|r| self.route_wealth(r) as u32)
            .sum()
    }

    /// Step 1: City base yields + building/specialization bonuses.
    ///
    /// Computes worked-tile yields (Water, Wealth) and adds building/specialization
    /// bonuses (Well, WellFort, ScholarOutpost, Temple).
    fn compute_city_yields(&self, _player: PlayerId, city_ids: &[CityId]) -> IncomeDeltas {
        let mut deltas = IncomeDeltas::new();
        for &city_id in city_ids {
            let worked = self.worked_tiles(city_id);
            for &tid in &worked {
                let tile = &self.tiles[tid.0 as usize];
                let td = tile.terrain.def();
                deltas.water += td.water as i32;
                deltas.wealth += td.wealth as i32;
            }

            let city = &self.cities[city_id.0 as usize];
            if city.buildings.contains(&BuildingKind::Well) {
                deltas.water += 2;
            }
            if city.specialization == Some(CitySpecialization::WellFort) {
                deltas.water += 3;
            }
            if city.specialization == Some(CitySpecialization::ScholarOutpost) {
                deltas.influence += 2;
            }
            if city.buildings.contains(&BuildingKind::Temple) {
                deltas.influence += 1;
            }
        }
        deltas
    }

    /// Step 2: Route wealth accumulation + water transfer.
    fn compute_route_income(&self, player: PlayerId) -> IncomeDeltas {
        let mut deltas = IncomeDeltas::new();
        let active_route_ids: Vec<RouteId> = self
            .routes
            .iter()
            .filter(|r| r.owner == player && r.status == RouteStatus::Active)
            .map(|r| r.id)
            .collect();
        for &route_id in &active_route_ids {
            let route = &self.routes[route_id.0 as usize];
            deltas.wealth += self.route_wealth(route) as i32;
            if self.water_transfer(route).is_some() {
                deltas.water += crate::caravan::WATER_TRANSFER_PER_ROUTE;
            }
        }
        deltas
    }

    /// Step 3: Route upkeep (−1 Water per owned route).
    fn compute_route_upkeep(&self, player: PlayerId) -> i32 {
        let route_count = self.routes.iter().filter(|r| r.owner == player).count() as i32;
        -(route_count * crate::caravan::ROUTE_UPKEEP_WATER)
    }

    /// Step 4: Isolation penalty (−2 Water per isolated city).
    fn compute_isolation_penalty(&self, city_ids: &[CityId]) -> i32 {
        let mut penalty = 0i32;
        for &city_id in city_ids {
            if self.is_city_isolated(city_id) {
                penalty += crate::caravan::ISOLATION_PENALTY_WATER;
            }
        }
        penalty
    }

    /// Step 5: Unit upkeep (Wealth) — negative flow.
    fn compute_unit_upkeep(&self, player: PlayerId) -> i32 {
        let mut upkeep = 0i32;
        for unit in &self.units {
            if unit.owner == player {
                let ki = unit.kind.index();
                upkeep -= Self::UNIT_UPKEEP[ki];
            }
        }
        upkeep
    }

    /// Step 7: Apply deltas to player resources and clamp to caps.
    fn apply_deltas(&mut self, player: PlayerId, deltas: &IncomeDeltas) {
        let pi = player.0 as usize;
        let cap = self.water_cap(player);
        let p = &mut self.players[pi];
        p.resources.water = p.resources.water.saturating_add(deltas.water as u32);
        p.resources.wealth = p.resources.wealth.saturating_add(deltas.wealth as u32);
        p.resources.influence = p
            .resources
            .influence
            .saturating_add(deltas.influence as u32);
        p.resources.water = p.resources.water.min(cap);
        p.resources.wealth = p.resources.wealth.min(WEALTH_CAP_BASE);
        p.resources.influence = p.resources.influence.min(INFLUENCE_CAP_BASE);
    }

    /// Step 8: Population growth for all cities.
    fn apply_growth_all(&mut self, city_ids: &[CityId]) -> Vec<GameEvent> {
        let mut events = Vec::new();
        for &city_id in city_ids {
            let ge = self.apply_growth(city_id);
            events.extend(ge);
        }
        events
    }

    /// Step 9: Starvation check — pop −1 when water is 0 and net flow is negative.
    fn apply_starvation(&mut self, player: PlayerId, city_ids: &[CityId]) -> Vec<GameEvent> {
        let water_after_clamp = self.players[player.0 as usize].resources.water;
        let mut events = Vec::new();

        for &city_id in city_ids {
            let ci = city_id.0 as usize;
            let city_yield = self.city_water_yield(city_id) as i32;
            let isolation = if self.is_city_isolated(city_id) {
                crate::caravan::ISOLATION_PENALTY_WATER
            } else {
                0
            };
            let net_flow = city_yield + isolation;

            if water_after_clamp == 0 && net_flow < 0 {
                let city = &mut self.cities[ci];
                if city.specialization == Some(CitySpecialization::WellFort) {
                    // Well Fort: never drops below Pop 1.
                } else {
                    city.population = city.population.saturating_sub(1);
                    let pop = city.population;
                    events.push(GameEvent::Starved {
                        city: city_id,
                        population: pop,
                    });
                }
            }
        }
        events
    }

    /// Step 10: Elimination — mark player defeated if no living cities.
    fn apply_elimination(&mut self, player: PlayerId) {
        let pi = player.0 as usize;
        let living_cities = self
            .cities
            .iter()
            .filter(|c| c.owner == player && c.population > 0)
            .count();
        if living_cities == 0 {
            self.players[pi].defeated = true;
        }
    }

    /// Run the full per-turn economy update for ONE actor.
    ///
    /// Called from the Income phase of `step()` (turn-engine §6.3). Returns the
    /// events produced: an [`GameEvent::Income`] summary plus any growth/starved
    /// events.
    ///
    /// Follows the fixed 10-step order from spec §6.1:
    ///
    /// 1. City base + worked-ring yields
    /// 2. Route Wealth/Water transfer
    /// 3. Route upkeep (Water)
    /// 4. Isolation penalty (Water)
    /// 5. Unit upkeep (Wealth)
    /// 6. Building/specialization Influence (folded into step 1)
    /// 7. Cap & overflow (clamp to empire_cap)
    /// 8. Growth
    /// 9. Starvation
    /// 10. Elimination
    pub fn apply_income(&mut self, player: PlayerId) -> Vec<GameEvent> {
        // Snapshot city IDs to avoid borrow issues.
        let city_ids: Vec<CityId> = self
            .cities
            .iter()
            .filter(|c| c.owner == player)
            .map(|c| c.id)
            .collect();

        // Compute all deltas
        let mut deltas = IncomeDeltas::new();

        // Step 1: City yields + building bonuses
        let city_yields = self.compute_city_yields(player, &city_ids);
        deltas.water += city_yields.water;
        deltas.wealth += city_yields.wealth;
        deltas.influence += city_yields.influence;

        // Step 2: Route income
        let route_income = self.compute_route_income(player);
        deltas.water += route_income.water;
        deltas.wealth += route_income.wealth;

        // Step 3: Route upkeep
        deltas.water += self.compute_route_upkeep(player);

        // Step 4: Isolation penalty
        deltas.water += self.compute_isolation_penalty(&city_ids);

        // Step 5: Unit upkeep
        deltas.wealth += self.compute_unit_upkeep(player);

        // Step 7: Apply deltas and clamp
        self.apply_deltas(player, &deltas);

        // Step 8: Growth
        let mut events = Vec::new();
        events.extend(self.apply_growth_all(&city_ids));

        // Step 9: Starvation
        events.extend(self.apply_starvation(player, &city_ids));

        // Step 10: Elimination
        self.apply_elimination(player);

        // Emit income summary
        events.push(GameEvent::Income {
            player,
            water: deltas.water,
            wealth: deltas.wealth,
            influence: deltas.influence,
        });

        events
    }
}

// ---------------------------------------------------------------------------
// Combat resolution (spec §6) — moved from combat.rs free functions
// ---------------------------------------------------------------------------

impl GameState {
    /// Resolve auto-combat between two units.
    ///
    /// Each exchange rolls `self.rng.next_f32()` against the computed odds.
    /// The loser of each exchange takes exactly 1 HP damage. Combat continues
    /// until one side is destroyed or the attacker auto-retreats (spec §6.6).
    ///
    /// `origin_tile` is the tile the attacker retreats to if it would die.
    ///
    /// Returns a [`GameEvent::Combat`] summarizing the outcome.
    pub fn resolve_combat(
        &mut self,
        attacker_id: UnitId,
        defender_id: UnitId,
        origin_tile: TileId,
    ) -> Vec<GameEvent> {
        // Guard: refuse self-combat and missing units.
        if attacker_id == defender_id {
            return Vec::new();
        }
        let attacker_idx = attacker_id.0 as usize;
        let defender_idx = defender_id.0 as usize;
        if self.units.get(attacker_idx).is_none() || self.units.get(defender_idx).is_none() {
            return Vec::new();
        }

        // Collect stats (avoids borrow issues during the mutation loop).
        let atk_kind = self.units[attacker_idx].kind;
        let def_kind = self.units[defender_idx].kind;
        let atk_stat = atk_kind.def().atk as f32;
        let def_stat = def_kind.def().def as f32;

        let attacker_tile = self.units[attacker_idx].tile;
        let defender_tile = self.units[defender_idx].tile;
        let atk_terrain = self.tiles[attacker_tile.0 as usize].terrain;
        let def_terrain = self.tiles[defender_tile.0 as usize].terrain;
        let terrain_mod = def_terrain.def().defense_mod as f32;

        // Compute attack / defense power (spec §6.1).
        let atk_pos = crate::combat::positioning_attacker(atk_terrain, def_terrain);
        let attack_power = atk_stat * atk_pos * 1.0; // morale = 1.0 (MVP)
        let defense_power = def_stat * (1.0 + terrain_mod);

        let odds = if attack_power + defense_power > 0.0 {
            attack_power / (attack_power + defense_power)
        } else {
            0.5
        };

        // Combat loop: each exchange, roll vs odds.
        let mut attacker_loss = 0u32;
        let mut defender_loss = 0u32;
        let mut retreated = false;
        let mut defender_destroyed = false;

        loop {
            let roll = self.rng.next_f32();

            if roll < odds {
                // Defender takes 1 HP damage.
                self.units[defender_idx].hp -= 1;
                defender_loss += 1;

                if self.units[defender_idx].hp == 0 {
                    defender_destroyed = true;
                    break;
                }
            } else {
                // Attacker takes 1 HP damage.
                self.units[attacker_idx].hp -= 1;
                attacker_loss += 1;

                // Auto-retreat: attacker falls back before dying (spec §6.6).
                if self.units[attacker_idx].hp == 0 {
                    self.units[attacker_idx].hp = 1;
                    self.units[attacker_idx].tile = origin_tile;
                    self.units[attacker_idx].moves_left = 0;
                    retreated = true;
                    break;
                }
            }
        }

        // Remove destroyed defender (spec §7).
        if defender_destroyed {
            self.units.retain(|u| u.id != defender_id);
        }

        vec![GameEvent::Combat {
            attacker: attacker_id,
            defender: defender_id,
            attacker_loss,
            defender_loss,
            retreated,
        }]
    }

    /// Resolve a Raider-vs-route contest.
    ///
    /// If a controlling Guard is on or adjacent to the route path, a stat
    /// contest determines the outcome. Otherwise the route auto-cascades.
    pub fn resolve_raid_contest(&mut self, raider_id: UnitId, route_id: RouteId) -> Vec<GameEvent> {
        // Validate inputs.
        if self.units.get(raider_id.0 as usize).is_none() {
            return vec![GameEvent::Warn {
                message: "Raid failed: raider unit not found".into(),
            }];
        }
        if self.routes.get(route_id.0 as usize).is_none() {
            return vec![GameEvent::Warn {
                message: "Raid failed: route not found".into(),
            }];
        }

        // Collect data (avoid borrow issues).
        let raider_owner = self.units[raider_id.0 as usize].owner;
        let raider_tile = self.units[raider_id.0 as usize].tile;
        let route_owner = self.routes[route_id.0 as usize].owner;
        let route_path: Vec<TileId> = self.routes[route_id.0 as usize].path.clone();

        // Check for a controlling Guard on / adjacent to the route.
        let guard_id = self.find_controlling_guard(&route_path, route_owner);

        if let Some(guard_id) = guard_id {
            // Stat contest: Raider atk vs Guard def (spec §6.3).
            let raider_tile_terrain = self.tiles[raider_tile.0 as usize].terrain;
            let raider_atk_pos = match raider_tile_terrain {
                TerrainType::Ridges => crate::combat::ROUGH_RAIDER_BONUS,
                TerrainType::SaltFlats => crate::combat::EXPOSED_POSITIONING_MULT,
                _ => 1.0,
            };
            let raider_atk = UnitKind::Raider.def().atk as f32 * raider_atk_pos;

            let guard_tile = self.units[guard_id.0 as usize].tile;
            let guard_tile_terrain = self.tiles[guard_tile.0 as usize].terrain;
            let guard_def_mod = guard_tile_terrain.def().defense_mod as f32;
            let guard_def = UnitKind::CaravanGuard.def().def as f32 * (1.0 + guard_def_mod);

            let odds = if raider_atk + guard_def > 0.0 {
                raider_atk / (raider_atk + guard_def)
            } else {
                0.5
            };

            let roll = self.rng.next_f32();

            if roll < odds {
                // Raid succeeds → cascade.
                self.cascade_route(route_id);
                let severed = self.routes[route_id.0 as usize].status == RouteStatus::Severed;
                vec![GameEvent::RouteRaided {
                    route: route_id,
                    by: raider_owner,
                    severed,
                }]
            } else {
                // Guard repels — route stays Active.
                vec![GameEvent::RouteRaided {
                    route: route_id,
                    by: raider_owner,
                    severed: false,
                }]
            }
        } else {
            // No defender → auto cascade.
            self.cascade_route(route_id);
            let severed = self.routes[route_id.0 as usize].status == RouteStatus::Severed;
            vec![GameEvent::RouteRaided {
                route: route_id,
                by: raider_owner,
                severed,
            }]
        }
    }

    /// Find a Guard owned by `route_owner` on or adjacent to any route-path tile.
    fn find_controlling_guard(
        &self,
        route_path: &[TileId],
        route_owner: PlayerId,
    ) -> Option<UnitId> {
        for &path_tile in route_path {
            // Units ON this tile.
            for u in &self.units {
                if u.owner == route_owner && u.kind == UnitKind::CaravanGuard && u.tile == path_tile
                {
                    return Some(u.id);
                }
            }

            // Units ADJACENT to this tile.
            let path_coord = self.tiles[path_tile.0 as usize].coord;
            for n in path_coord.neighbors() {
                if let Some(&adj_tile) = self.tile_index.get(&n) {
                    for u in &self.units {
                        if u.owner == route_owner
                            && u.kind == UnitKind::CaravanGuard
                            && u.tile == adj_tile
                        {
                            return Some(u.id);
                        }
                    }
                }
            }
        }
        None
    }

    /// Cascade a route's status: `Active → Threatened`, `Threatened → Severed`.
    fn cascade_route(&mut self, route_id: RouteId) {
        let route = &mut self.routes[route_id.0 as usize];
        route.status = match route.status {
            RouteStatus::Active => RouteStatus::Threatened,
            RouteStatus::Threatened => RouteStatus::Severed,
            RouteStatus::Severed => RouteStatus::Severed,
        };
    }

    /// Resolve a city raid: Raider vs city (garrison + Fortress + terrain).
    ///
    /// On attacker win: `city.population -= 1`. If population reaches 0 and
    /// the Raider occupies the city tile, the city is **captured** (owner flip).
    pub fn resolve_city_raid(&mut self, raider_id: UnitId, city_id: CityId) -> Vec<GameEvent> {
        // Validate inputs.
        let raider_idx = raider_id.0 as usize;
        let city_idx = city_id.0 as usize;

        if self.units.get(raider_idx).is_none() || self.cities.get(city_idx).is_none() {
            return vec![GameEvent::Warn {
                message: "City raid failed: unit or city not found".into(),
            }];
        }

        let raider_owner = self.units[raider_idx].owner;
        let city_owner = self.cities[city_idx].owner;
        let city_tile = self.cities[city_idx].tile;
        let city_pop = self.cities[city_idx].population;

        // Cannot raid own city.
        if raider_owner == city_owner {
            return vec![GameEvent::Warn {
                message: "Cannot raid own city".into(),
            }];
        }

        if city_pop == 0 {
            return vec![GameEvent::Warn {
                message: "City has no population to raid".into(),
            }];
        }

        // --- Compute city defense (spec §6.4) ---
        let city_terrain = self.tiles[city_tile.0 as usize].terrain;
        let terrain_mod = city_terrain.def().defense_mod as f32;

        let fortress_bonus =
            if self.cities[city_idx].specialization == Some(CitySpecialization::Fortress) {
                crate::combat::FORTRESS_CITY_DEF as f32
            } else {
                0.0
            };

        // Sum garrisoned Guard def.
        let mut garrison_def = 0.0f32;
        for u in &self.units {
            if u.owner == city_owner
                && u.kind == UnitKind::CaravanGuard
                && u.tile == city_tile
                && u.ability == UnitAbility::Garrisoned
            {
                garrison_def += UnitKind::CaravanGuard.def().def as f32;
            }
        }

        // City's aggregate defense (terrain_mod already included).
        let city_def = (terrain_mod + fortress_bonus + garrison_def).max(0.0);

        // --- Raider attack power ---
        let raider_tile = self.units[raider_idx].tile;
        let raider_terrain = self.tiles[raider_tile.0 as usize].terrain;
        let atk_pos = crate::combat::positioning_attacker(raider_terrain, city_terrain);
        let attack_power = UnitKind::Raider.def().atk as f32 * atk_pos;
        let defense_power = city_def;

        let odds = if attack_power + defense_power > 0.0 {
            attack_power / (attack_power + defense_power)
        } else {
            0.5
        };

        // --- Combat loop with city HP ---
        let mut remaining_pop = city_pop as i32;
        let mut pop_lost = 0u32;

        loop {
            let roll = self.rng.next_f32();

            if roll < odds {
                // City takes damage.
                remaining_pop -= 1;
                pop_lost += 1;

                if remaining_pop <= 0 {
                    break;
                }
            } else {
                // Raider takes damage.
                self.units[raider_idx].hp -= 1;

                if self.units[raider_idx].hp == 0 {
                    // Auto-retreat: survive at 1 HP, stop the raid.
                    self.units[raider_idx].hp = 1;
                    self.units[raider_idx].moves_left = 0;
                    break;
                }
            }
        }

        // Apply population loss.
        self.cities[city_idx].population =
            self.cities[city_idx].population.saturating_sub(pop_lost);

        // Check for capture (spec §6.4): pop == 0 AND raider on city tile.
        if self.cities[city_idx].population == 0 && self.units[raider_idx].tile == city_tile {
            self.cities[city_idx].owner = raider_owner;
        }

        vec![GameEvent::CityRaided {
            city: city_id,
            by: raider_owner,
            pop_lost,
        }]
    }
}

// ---------------------------------------------------------------------------
// Victory condition logic (moved from victory.rs free functions)
// ---------------------------------------------------------------------------

impl GameState {
    /// Count of oases owned by `player` (V1 numerator).
    ///
    /// An oasis tile is one with `terrain == Oasis`; "control" means
    /// `tile.owner == Some(player)`.
    pub fn oases_controlled_by(&self, player: PlayerId) -> u32 {
        self.tiles
            .iter()
            .filter(|t| t.terrain == TerrainType::Oasis && t.owner == Some(player))
            .count() as u32
    }

    /// Total oases on the map (V1 denominator).
    ///
    /// Derived from tile terrain — always known regardless of fog.
    pub fn total_oases(&self) -> u32 {
        self.tiles
            .iter()
            .filter(|t| t.terrain == TerrainType::Oasis)
            .count() as u32
    }

    /// Count of active caravan routes owned by `player`.
    ///
    /// Only `RouteStatus::Active` routes contribute — Threatened/Severed routes
    /// count as 0 (spec §6.2).
    pub fn active_routes(&self, player: PlayerId) -> u32 {
        self.routes
            .iter()
            .filter(|r| r.owner == player && r.status == RouteStatus::Active)
            .count() as u32
    }

    /// Does `player` hold ALL relic sites? (V3 holder test)
    ///
    /// Returns `true` if every relic site tile with `is_relic_site == true` has a
    /// corresponding `Relic` whose `holder == Some(player)`. If there are no relic
    /// sites, returns `false` (no relics to hold).
    pub fn holds_required_relics(&self, player: PlayerId) -> bool {
        let relic_sites: Vec<_> = self.tiles.iter().filter(|t| t.is_relic_site).collect();
        if relic_sites.is_empty() {
            return false;
        }
        relic_sites.iter().all(|site| {
            self.relics
                .iter()
                .any(|r| r.tile == site.id && r.holder == Some(player))
        })
    }

    /// Prestige score for one player (V2 + turn-limit fallback).
    ///
    /// Implements DD §13's formula:
    /// ```text
    /// floor(Wealth×1 + Influence×2 + oases×8 + active_routes×4)
    /// ```
    ///
    /// Stockpile values come from `Player.resources`; oases and routes are
    /// recomputed live for accuracy.
    pub fn prestige_score(&self, player: PlayerId) -> u32 {
        let p = &self.players[player.0 as usize];
        let wealth = p.resources.wealth as f32;
        let influence = p.resources.influence as f32;
        let oases = self.oases_controlled_by(player) as f32;
        let routes = self.active_routes(player) as f32;

        let score = wealth * crate::victory::PRESTIGE_WEALTH_W
            + influence * crate::victory::PRESTIGE_INFLUENCE_W
            + oases * crate::victory::PRESTIGE_OASIS_W
            + routes * crate::victory::PRESTIGE_ROUTE_W;

        score.floor() as u32
    }

    /// V1 — Oasis Dominance (spec §6.3).
    ///
    /// A player wins if they control ≥ `ceil(oasis_majority_pct / 100 * total_oases)`
    /// oases. Defeated players are excluded from this check entirely — a defeated
    /// player can never win, and surviving players are not granted an automatic
    /// victory just because all other players are defeated.
    fn check_oasis_dominance(&self) -> Option<PlayerId> {
        let total = self.total_oases().max(1);
        let threshold =
            (self.scenario.oasis_majority_pct as f32 / 100.0 * total as f32).ceil() as u32;

        // Check threshold-based win — defeated players are skipped.
        for p in &self.players {
            if p.defeated {
                continue;
            }
            let controlled = self.oases_controlled_by(p.id);
            if controlled >= threshold {
                return Some(p.id);
            }
        }

        None
    }

    /// V2 — Wealth/Prestige Score (spec §6.2).
    ///
    /// A player wins when `prestige_score(p) >= wealth_score_target`.
    fn check_wealth_score(&self) -> Option<PlayerId> {
        for p in &self.players {
            if p.defeated {
                continue;
            }
            if self.prestige_score(p.id) >= self.scenario.wealth_score_target {
                return Some(p.id);
            }
        }
        None
    }

    /// V3 — Relic Hold (spec §6.4).
    ///
    /// A player wins if they hold ALL relic sites (`holds_required_relics`) AND
    /// every held relic's `consecutive_turns_held >= relic_hold_turns`.
    fn check_relic_hold(&self) -> Option<PlayerId> {
        let hold_turns = self.scenario.relic_hold_turns;

        // Collect relic ids that are on relic sites.
        let relic_site_ids: Vec<RelicId> = self
            .relics
            .iter()
            .filter(|r| {
                self.tiles
                    .get(r.tile.0 as usize)
                    .is_some_and(|t| t.is_relic_site)
            })
            .map(|r| r.id)
            .collect();

        if relic_site_ids.is_empty() {
            return None;
        }

        for p in &self.players {
            if p.defeated {
                continue;
            }
            // Check that every relic site is held by this player.
            let all_held = relic_site_ids.iter().all(|rid| {
                self.relics
                    .iter()
                    .any(|r| r.id == *rid && r.holder == Some(p.id))
            });
            if !all_held {
                continue;
            }
            // Check that every held relic meets the hold timer.
            let all_long_enough = relic_site_ids.iter().all(|rid| {
                self.relics.iter().any(|r| {
                    r.id == *rid && r.holder == Some(p.id) && r.consecutive_turns_held >= hold_turns
                })
            });
            if all_long_enough {
                return Some(p.id);
            }
        }
        None
    }

    /// Turn-limit fallback (spec §6.5).
    ///
    /// When `self.turn >= scenario.turn_limit`, the living player with the
    /// highest `prestige_score` wins. Tiebreak: (1) more oases controlled,
    /// (2) lower `PlayerId` (deterministic).
    fn check_turn_limit(&self) -> Option<PlayerId> {
        if self.turn < self.scenario.turn_limit {
            return None;
        }

        let mut living: Vec<_> = self.players.iter().filter(|p| !p.defeated).collect();
        if living.is_empty() {
            return None;
        }

        living.sort_by(|a, b| {
            let score_a = self.prestige_score(a.id);
            let score_b = self.prestige_score(b.id);
            score_b
                .cmp(&score_a)
                .then_with(|| {
                    let oases_a = self.oases_controlled_by(a.id);
                    let oases_b = self.oases_controlled_by(b.id);
                    oases_b.cmp(&oases_a)
                })
                .then_with(|| a.id.cmp(&b.id))
        });

        Some(living[0].id)
    }

    /// Decide if a victory has occurred (V1/V2/V3 met or turn limit).
    ///
    /// Returns `Some(GameEvent::Victory { kind, winner })` to be emitted by the
    /// engine, else `None`. Pure read — no mutation.
    pub fn check_victory(&self) -> Option<GameEvent> {
        // Check threshold-based conditions (V1–V3) only for enabled kinds.
        for kind in &self.scenario.victories_enabled {
            let winner = match kind {
                VictoryKind::OasisDominance => self.check_oasis_dominance(),
                VictoryKind::WealthScore => self.check_wealth_score(),
                VictoryKind::RelicHold => self.check_relic_hold(),
                VictoryKind::TurnLimit => None, // handled below
            };
            if let Some(winner) = winner {
                return Some(GameEvent::Victory {
                    kind: *kind,
                    winner,
                });
            }
        }

        // Turn-limit fallback.
        if let Some(winner) = self.check_turn_limit() {
            return Some(GameEvent::Victory {
                kind: VictoryKind::TurnLimit,
                winner,
            });
        }

        None
    }

    /// Recompute all tracker maps and check for victory.
    ///
    /// Called once from `advance_turn` (turn-engine §6.3 step 2) BEFORE the turn
    /// counter is incremented. Updates `self.victory` (oases_controlled,
    /// prestige_score, relic_timers) for every living player, then calls
    /// [`check_victory`](Self::check_victory) and returns any resulting event.
    pub fn update_victory_tracker(&mut self) -> Vec<GameEvent> {
        // --- Populate tracker maps for every living player ---
        for p in &self.players {
            if p.defeated {
                continue;
            }
            self.victory
                .oases_controlled
                .insert(p.id, self.oases_controlled_by(p.id));
            self.victory
                .prestige_score
                .insert(p.id, self.prestige_score(p.id));
        }

        // --- Update relic_timers (mirror current holders for fast win-checking) ---
        self.victory.relic_timers.clear();
        for relic in &self.relics {
            if let Some(holder) = relic.holder {
                self.victory.relic_timers.insert(relic.id, holder);
            }
        }

        // --- Check victory and return any event ---
        if let Some(event) = self.check_victory() {
            vec![event]
        } else {
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// City / unit gameplay logic — moved from world.rs free functions
// ---------------------------------------------------------------------------

impl GameState {
    // -------------------------------------------------------------------
    // Worked tiles & building slots
    // -------------------------------------------------------------------

    /// Returns worked tiles for a city: the city tile plus all tiles in ring(1).
    pub fn worked_tiles(&self, city_id: CityId) -> Vec<TileId> {
        let c = &self.cities[city_id.0 as usize];
        let coord = self.tiles[c.tile.0 as usize].coord;
        let mut tiles = vec![c.tile];
        for hex in coord.range(1) {
            if let Some(&tid) = self.tile_index.get(&hex) {
                tiles.push(tid);
            }
        }
        tiles
    }

    // -------------------------------------------------------------------
    // Shared validation helpers
    // -------------------------------------------------------------------

    /// Validate and look up a city for a build/specialize command.
    /// Returns `(city_id, city_index, &City)` or `Err(RejectReason)`.
    fn validate_city_for_command(
        &self,
        actor: PlayerId,
        city_id: CityId,
    ) -> Result<(CityId, usize, &City), RejectReason> {
        let ci = city_id.0 as usize;
        let city = self.cities.get(ci).ok_or(RejectReason::InvalidState)?;
        if city.owner != actor {
            return Err(RejectReason::InvalidState);
        }
        Ok((city_id, ci, city))
    }

    /// Check if a building can be built in a city.
    /// Returns the (possibly discounted) building cost or `Err(RejectReason)`.
    fn validate_build_conditions(
        &self,
        city: &City,
        building: &BuildingKind,
    ) -> Result<u32, RejectReason> {
        let bs = city.building_slots() as usize;
        if city.buildings.len() >= bs {
            return Err(RejectReason::Blocked);
        }
        if city.buildings.contains(building) {
            return Err(RejectReason::InvalidState);
        }

        // Calculate cost (with TradeHub Market discount).
        let base_cost = BUILD_COST[building.index()];
        let cost = if *building == BuildingKind::Market
            && city.specialization == Some(CitySpecialization::TradeHub)
        {
            base_cost.saturating_sub(TRADE_HUB_MARKET_DISCOUNT)
        } else {
            base_cost
        };

        let player = &self.players[city.owner.0 as usize];
        if player.resources.wealth < cost {
            return Err(RejectReason::NoResource);
        }

        Ok(cost)
    }

    /// Check if a city can specialize.
    /// Returns the influence cost or `Err(RejectReason)`.
    fn validate_specialize_conditions(&self, city: &City) -> Result<u32, RejectReason> {
        if city.population < POP_FOR_SPECIALIZE {
            return Err(RejectReason::InvalidState);
        }
        if city.specialization.is_some() {
            return Err(RejectReason::InvalidState);
        }

        let player = &self.players[city.owner.0 as usize];
        if player.resources.influence < SPECIALIZE_COST_INFLUENCE {
            return Err(RejectReason::NoResource);
        }

        Ok(SPECIALIZE_COST_INFLUENCE)
    }

    // -------------------------------------------------------------------
    // Build resolution
    // -------------------------------------------------------------------

    /// Resolve a Build command: validate, spend wealth, add building to city.
    pub fn resolve_build(
        &mut self,
        city_id: CityId,
        building: BuildingKind,
        actor: PlayerId,
    ) -> Vec<GameEvent> {
        let cmd = Command::Build {
            city: city_id,
            building,
        };

        // Defense-in-depth: validate via shared helpers (validate() already ran).
        let (_cid, ci, _city) = match self.validate_city_for_command(actor, city_id) {
            Ok(v) => v,
            Err(r) => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: r,
                }];
            }
        };

        let cost = match self.validate_build_conditions(&self.cities[ci], &building) {
            Ok(c) => c,
            Err(r) => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: r,
                }];
            }
        };

        // Spend wealth.
        self.players[actor.0 as usize].resources.wealth -= cost;

        // Add building.
        self.cities[ci].buildings.push(building);

        vec![GameEvent::Built {
            city: city_id,
            building,
        }]
    }

    // -------------------------------------------------------------------
    // Specialize resolution
    // -------------------------------------------------------------------

    /// Resolve a Specialize command: validate, spend influence, set specialization.
    pub fn resolve_specialize(
        &mut self,
        city_id: CityId,
        spec: CitySpecialization,
        actor: PlayerId,
    ) -> Vec<GameEvent> {
        let cmd = Command::Specialize {
            city: city_id,
            spec,
        };

        // Defense-in-depth: validate via shared helpers (validate() already ran).
        let (_cid, ci, _city) = match self.validate_city_for_command(actor, city_id) {
            Ok(v) => v,
            Err(r) => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: r,
                }];
            }
        };

        match self.validate_specialize_conditions(&self.cities[ci]) {
            Ok(_cost) => {}
            Err(r) => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: r,
                }];
            }
        };

        // Spend influence.
        self.players[actor.0 as usize].resources.influence -= SPECIALIZE_COST_INFLUENCE;

        // Set specialization.
        self.cities[ci].specialization = Some(spec);

        vec![GameEvent::Specialized {
            city: city_id,
            spec,
        }]
    }

    // -------------------------------------------------------------------
    // Production queue
    // -------------------------------------------------------------------

    /// Process a city's production queue at Income phase, consuming orders
    /// head-to-tail until one cannot be afforded or completed.
    pub fn process_queue(&mut self, city_id: CityId) -> Vec<GameEvent> {
        let mut events = Vec::new();
        let city_idx = city_id.0 as usize;

        while let Some(order) = self.cities[city_idx].queue.front().cloned() {
            let result = match order {
                QueuedOrder::Build(building) => {
                    if self.cities[city_idx].buildings.len()
                        >= self.cities[city_idx].building_slots() as usize
                    {
                        break; // no more slots
                    }
                    if self.cities[city_idx].buildings.contains(&building) {
                        break; // already built
                    }
                    let cost = BUILD_COST[building.index()];
                    let owner = self.cities[city_idx].owner;
                    if self.players[owner.0 as usize].resources.wealth < cost {
                        break; // can't afford
                    }
                    self.players[owner.0 as usize].resources.wealth -= cost;
                    self.cities[city_idx].buildings.push(building);
                    Some(GameEvent::Built {
                        city: city_id,
                        building,
                    })
                }
                QueuedOrder::Train(kind) => {
                    let base_cost = UNIT_TRAIN_COST[kind.index()];
                    let owner = self.cities[city_idx].owner;
                    let city = &self.cities[city_idx];

                    // Raider must be in Fortress.
                    if kind == UnitKind::Raider
                        && city.specialization != Some(CitySpecialization::Fortress)
                    {
                        break;
                    }

                    // Calculate cost with Fortress discount.
                    let mut cost = base_cost;
                    if city.specialization == Some(CitySpecialization::Fortress) {
                        cost = (cost as f32 * FORTRESS_TRAIN_DISCOUNT) as u32;
                    }

                    if self.players[owner.0 as usize].resources.wealth < cost {
                        break;
                    }

                    // Check unit cap.
                    let cap = self.unit_cap(owner);
                    let current = self.units.iter().filter(|u| u.owner == owner).count() as u32;
                    if current >= cap {
                        break;
                    }

                    self.players[owner.0 as usize].resources.wealth -= cost;
                    let tile = self.cities[city_idx].tile;
                    let unit_id = self.alloc_unit_id();
                    let def = kind.def();
                    self.units.push(Unit {
                        id: unit_id,
                        owner,
                        kind,
                        tile,
                        hp: def.hp as u32,
                        moves_left: def.moves,
                        ability: UnitAbility::None,
                    });
                    self.reveal_from_unit(unit_id);
                    Some(GameEvent::UnitTrained {
                        unit: unit_id,
                        city: city_id,
                    })
                }
                QueuedOrder::Specialize(spec) => {
                    if self.cities[city_idx].specialization.is_some() {
                        break;
                    }
                    if self.cities[city_idx].population < POP_FOR_SPECIALIZE {
                        break;
                    }
                    let owner = self.cities[city_idx].owner;
                    if self.players[owner.0 as usize].resources.influence
                        < SPECIALIZE_COST_INFLUENCE
                    {
                        break;
                    }
                    self.players[owner.0 as usize].resources.influence -= SPECIALIZE_COST_INFLUENCE;
                    self.cities[city_idx].specialization = Some(spec);
                    Some(GameEvent::Specialized {
                        city: city_id,
                        spec,
                    })
                }
            };

            if let Some(event) = result {
                self.cities[city_idx].queue.pop_front();
                events.push(event);
            } else {
                break;
            }
        }

        events
    }

    // -------------------------------------------------------------------
    // City growth
    // -------------------------------------------------------------------

    /// Apply city growth: check water threshold and increment timer/pop.
    pub fn apply_growth(&mut self, city_id: CityId) -> Vec<GameEvent> {
        let mut events = Vec::new();
        let city_idx = city_id.0 as usize;

        let water_yield = self.city_water_yield(city_id);

        if water_yield > GROWTH_WATER_THRESHOLD {
            self.cities[city_idx].growth_timer += 1;
            if self.cities[city_idx].growth_timer >= GROWTH_PERIOD_TURNS {
                self.cities[city_idx].population += 1;
                self.cities[city_idx].growth_timer = 0;
                events.push(GameEvent::Grown {
                    city: city_id,
                    population: self.cities[city_idx].population,
                });
            }
        } else {
            self.cities[city_idx].growth_timer = 0;
        }

        events
    }

    /// Compute water yield for a city from worked tiles, Well bonus, and
    /// Well Fort specialization bonus.
    pub(crate) fn city_water_yield(&self, city_id: CityId) -> u32 {
        let c = &self.cities[city_id.0 as usize];
        let mut water = 0u32;

        // Worked tile yields.
        for &tid in &self.worked_tiles(city_id) {
            let tile = &self.tiles[tid.0 as usize];
            water += TERRAIN[tile.terrain as usize].water as u32;
        }

        // Well building bonus.
        if c.buildings.contains(&BuildingKind::Well) {
            water += 2;
        }

        // Well Fort specialization bonus.
        if c.specialization == Some(CitySpecialization::WellFort) {
            water += 3;
        }

        water
    }

    // -------------------------------------------------------------------
    // Unit cap
    // -------------------------------------------------------------------

    /// Check how many units a player can field (base + total population across
    /// their cities).
    pub fn unit_cap(&self, player: PlayerId) -> u32 {
        let total_pop: u32 = self
            .cities
            .iter()
            .filter(|c| c.owner == player)
            .map(|c| c.population)
            .sum();
        UNIT_CAP_BASE + total_pop
    }

    // -------------------------------------------------------------------
    // Zone of Control
    // -------------------------------------------------------------------

    /// Fortress cities project Zone of Control onto the city tile plus its 6
    /// immediate neighbors.
    pub fn zone_of_control(&self, player: PlayerId) -> FxHashSet<TileId> {
        let mut zoc = FxHashSet::default();

        for city in &self.cities {
            if city.owner == player && city.specialization == Some(CitySpecialization::Fortress) {
                let coord = self.tiles[city.tile.0 as usize].coord;
                zoc.insert(city.tile);
                for hex in coord.range(1) {
                    if let Some(&tid) = self.tile_index.get(&hex) {
                        zoc.insert(tid);
                    }
                }
            }
        }

        zoc
    }

    // -------------------------------------------------------------------
    // Resource caps
    // -------------------------------------------------------------------

    /// Get the water cap for a player (base + Granary bonuses).
    pub fn water_cap(&self, player: PlayerId) -> u32 {
        let granaries: u32 = self
            .cities
            .iter()
            .filter(|c| c.owner == player)
            .map(|c| {
                c.buildings
                    .iter()
                    .filter(|b| **b == BuildingKind::Granary)
                    .count() as u32
            })
            .sum();
        WATER_CAP_BASE + granaries * GRANARY_WATER_BONUS
    }
}

/// Migrate a payload from `from` to `from + 1`.
///
/// No older versions exist yet (`SAVE_VERSION == 1`), so there is nothing to
/// migrate. When a breaking schema change lands, register a deterministic
/// `(version -> version + 1)` transform here. The transform must be pure and
/// deterministic so that a save always migrates to the same result.
fn migrate(_from: u32, _state: GameState) -> Result<GameState, SaveError> {
    Err(SaveError::MigrationFailed(
        _from,
        "no migrations registered".into(),
    ))
}

impl City {
    /// Building slots for a city: 2 + population / 2.
    pub fn building_slots(&self) -> u8 {
        (2 + self.population / 2) as u8
    }

    /// Whether a city sits on an oasis tile (invariant: city/tile must exist).
    pub fn is_on_oasis(&self, state: &GameState) -> bool {
        let tile = state
            .tiles
            .get(self.tile.0 as usize)
            .expect("invariant: city tile must exist");
        tile.terrain == TerrainType::Oasis
    }
}

// ---------------------------------------------------------------------------
// Caravan & Trade Routes (spec §8) — moved from caravan.rs free functions
// ---------------------------------------------------------------------------

impl GameState {
    /// Threat-weighted shortest safe path between two city tiles (DD §8.2).
    ///
    /// Auto-route ONLY — the player supplies endpoints, not a tile list. Returns
    /// the inclusive `Vec<TileId>` path from `from` to `to`, or an empty `Vec` if
    /// no path exists. Uses [`crate::hex::HexCoord::safe_route`] (Dijkstra) under the hood.
    pub fn compute_route(&self, from: TileId, to: TileId, actor: PlayerId) -> Vec<TileId> {
        let from_coord = self.tiles[from.0 as usize].coord;
        let to_coord = self.tiles[to.0 as usize].coord;

        if from_coord == to_coord {
            return vec![from];
        }

        let threat_fn = |coord: HexCoord| -> f32 {
            if let Some(&tile_id) = self.tile_index.get(&coord) {
                let tile = &self.tiles[tile_id.0 as usize];
                let mc = TERRAIN[tile.terrain as usize].move_cost as f32;

                let mut penalty = 0.0f32;

                if self.is_enemy_present(tile_id, actor) {
                    penalty += crate::caravan::THREAT_ENEMY_TILE;
                } else if tile.terrain == TerrainType::SaltFlats
                    && !self.is_tile_controlled_by(actor, tile_id)
                {
                    penalty += crate::caravan::THREAT_EXPOSED_FLAT;
                }

                mc * (1.0 + penalty) - 1.0
            } else {
                1000.0
            }
        };

        let result = from_coord.safe_route(to_coord, threat_fn, 1.0);

        match result {
            Some((coords, _total_cost)) => {
                let mut path = Vec::with_capacity(coords.len());
                for coord in coords {
                    if let Some(&tid) = self.tile_index.get(&coord) {
                        path.push(tid);
                    } else {
                        return Vec::new();
                    }
                }
                path
            }
            None => Vec::new(),
        }
    }

    /// Is `tile` owned by an enemy of `player`? (owned by someone other than
    /// `player`, or has an enemy unit present.)
    fn is_enemy_present(&self, tile: TileId, player: PlayerId) -> bool {
        let t = &self.tiles[tile.0 as usize];
        if let Some(owner) = t.owner {
            if owner != player {
                return true;
            }
        }
        for unit in &self.units {
            if unit.owner != player && unit.tile == tile {
                return true;
            }
        }
        false
    }

    /// Is `tile` controlled by `player` via territory or a patrolling Guard?
    pub fn is_tile_controlled_by(&self, player: PlayerId, tile: TileId) -> bool {
        for city in &self.cities {
            if city.owner == player {
                let worked = self.worked_tiles(city.id);
                if worked.contains(&tile) {
                    return true;
                }
            }
        }

        let tile_coord = self.tiles[tile.0 as usize].coord;
        for unit in &self.units {
            if unit.owner == player
                && unit.kind == UnitKind::CaravanGuard
                && unit.ability == UnitAbility::Patrolling
            {
                let unit_coord = self.tiles[unit.tile.0 as usize].coord;
                if tile_coord.distance(unit_coord) <= 1 {
                    return true;
                }
            }
        }

        false
    }

    /// Is route-tile `t` controlled by `route.owner`? (territory ring OR
    /// patrolling Guard).
    pub fn is_route_tile_controlled(&self, route: &CaravanRoute, t: TileId) -> bool {
        self.is_tile_controlled_by(route.owner, t)
    }

    /// Cost preview for the UI (DD §3.3 route-planning mode).
    ///
    /// Returns `(wealth_cost, path)` where `cost = ROUTE_ESTABLISH_BASE_COST +
    /// path.len() * ROUTE_ESTABLISH_PER_TILE`.
    pub fn preview_cost(&self, from: CityId, to: CityId) -> (i32, Vec<TileId>) {
        let from_tile = self.cities[from.0 as usize].tile;
        let to_tile = self.cities[to.0 as usize].tile;
        let path = self.compute_route(from_tile, to_tile, self.cities[from.0 as usize].owner);
        let cost = crate::caravan::establishment_cost(path.len());
        (cost, path)
    }

    /// Resolve a `ConnectRoute` command: validate, compute path, spend Wealth,
    /// store the route, and emit events.
    pub fn resolve_connect(&mut self, from: CityId, to: CityId, actor: PlayerId) -> Vec<GameEvent> {
        let cmd = Command::ConnectRoute { from, to };

        if from == to {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::InvalidState,
            }];
        }

        let city_a = match self.cities.get(from.0 as usize) {
            Some(c) if c.owner == actor => c.clone(),
            _ => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: RejectReason::InvalidState,
                }];
            }
        };
        let city_b = match self.cities.get(to.0 as usize) {
            Some(c) if c.owner == actor => c.clone(),
            _ => {
                return vec![GameEvent::Rejected {
                    command: cmd,
                    reason: RejectReason::InvalidState,
                }];
            }
        };

        if city_a.route_slots == 0 || city_b.route_slots == 0 {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::Blocked,
            }];
        }

        let path = self.compute_route(city_a.tile, city_b.tile, actor);
        if path.is_empty() {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::Blocked,
            }];
        }

        let cost = crate::caravan::establishment_cost(path.len());

        if (self.players[actor.0 as usize].resources.wealth as i32) < cost {
            return vec![GameEvent::Rejected {
                command: cmd,
                reason: RejectReason::NoResource,
            }];
        }

        self.players[actor.0 as usize].resources.wealth -= cost as u32;

        self.cities[from.0 as usize].route_slots -= 1;
        self.cities[to.0 as usize].route_slots -= 1;

        let route_id = self.alloc_route_id();
        let route = CaravanRoute {
            id: route_id,
            owner: actor,
            endpoints: (from, to),
            path: path.clone(),
            status: RouteStatus::Active,
            length: path.len() as u32,
            upkeep: crate::caravan::ROUTE_UPKEEP_WATER as u8,
            consecutive_threatened: 0,
        };
        self.routes.push(route);

        vec![GameEvent::RouteCreated {
            route: route_id,
            from,
            to,
            path,
        }]
    }

    /// Wealth produced by ONE active route (the core formula), as `f32` for
    /// synergy multiplication. Returns 0.0 for Threatened or Severed routes.
    pub fn route_wealth(&self, route: &CaravanRoute) -> f32 {
        if route.status == RouteStatus::Severed {
            return 0.0;
        }

        let city_a = &self.cities[route.endpoints.0.0 as usize];
        let city_b = &self.cities[route.endpoints.1.0 as usize];

        let trade_endpoints = [city_a, city_b]
            .iter()
            .filter(|c| c.specialization == Some(CitySpecialization::TradeHub))
            .count() as i32;

        let markets = [city_a, city_b]
            .iter()
            .filter(|c| c.buildings.contains(&BuildingKind::Market))
            .count() as i32;

        let dist_factor = (route
            .length
            .saturating_sub(1)
            .min(crate::caravan::ROUTE_DIST_CAP) as f32)
            * crate::caravan::ROUTE_DIST_BONUS;

        let mut base = crate::caravan::ROUTE_WEALTH_BASE as f32
            + (trade_endpoints * crate::caravan::TRADE_HUB_BONUS_PER_EP) as f32
            + dist_factor
            + (markets * crate::caravan::MARKET_BONUS) as f32;

        if city_a.specialization == Some(CitySpecialization::TradeHub)
            || city_b.specialization == Some(CitySpecialization::TradeHub)
        {
            base *= crate::caravan::TRADE_HUB_WEALTH_MULT;
        }

        base *= self.network_synergy(route.owner);

        if route.status == RouteStatus::Threatened {
            base *= 0.5;
        }

        base.floor()
    }

    /// Network synergy multiplier for `player`.
    pub fn network_synergy(&self, player: PlayerId) -> f32 {
        let c = self.connected_city_count(player);
        1.0 + crate::caravan::NETWORK_SYNERGY_PER_CITY * (c as f32 - 2.0).max(0.0)
    }

    /// Count of distinct cities in `player`'s active-route connected component.
    pub fn connected_city_count(&self, player: PlayerId) -> u32 {
        let mut connected = FxHashSet::default();
        for route in &self.routes {
            if route.owner == player && route.status == RouteStatus::Active {
                connected.insert(route.endpoints.0);
                connected.insert(route.endpoints.1);
            }
        }
        connected.len() as u32
    }

    /// Water source/sink pair for a route's transfer.
    ///
    /// Returns `Some((source, sink))` if a transfer should happen, or `None` if
    /// both endpoints are self-sufficient or the sink is a Well Fort.
    pub fn water_transfer(&self, route: &CaravanRoute) -> Option<(CityId, CityId)> {
        let prod_a = self.city_water_yield(route.endpoints.0);
        let prod_b = self.city_water_yield(route.endpoints.1);

        let (source, sink) = if prod_a >= prod_b {
            (route.endpoints.0, route.endpoints.1)
        } else {
            (route.endpoints.1, route.endpoints.0)
        };

        let sink_city = &self.cities[sink.0 as usize];

        if sink_city.specialization == Some(CitySpecialization::WellFort) {
            return None;
        }

        let sink_prod = if sink == route.endpoints.0 {
            prod_a
        } else {
            prod_b
        };
        if sink_prod >= 5 {
            return None;
        }

        Some((source, sink))
    }

    /// Recompute every route's status at `advance_turn`.
    ///
    /// Implements the Active → Threatened → Severed state machine per spec §6.5.
    pub fn recompute_routes(&mut self) -> Vec<GameEvent> {
        let mut events = Vec::new();

        let route_meta: Vec<(RouteId, PlayerId, RouteStatus, Vec<TileId>, u8)> = self
            .routes
            .iter()
            .map(|r| {
                (
                    r.id,
                    r.owner,
                    r.status,
                    r.path.clone(),
                    r.consecutive_threatened,
                )
            })
            .collect();

        for (route_idx, meta) in route_meta.iter().enumerate() {
            let route_id = meta.0;
            let owner = meta.1;
            let old_status = meta.2;
            let path = &meta.3;
            let consecutive = meta.4;

            let enemy_adjacent_to_exposed = self.has_enemy_adjacent_to_exposed(owner, path);

            let mut new_status = old_status;
            let mut new_consecutive = consecutive;

            match old_status {
                RouteStatus::Active => {
                    if enemy_adjacent_to_exposed {
                        new_status = RouteStatus::Threatened;
                        new_consecutive = 1;
                    } else {
                        new_consecutive = 0;
                    }
                }
                RouteStatus::Threatened => {
                    if enemy_adjacent_to_exposed {
                        new_status = RouteStatus::Severed;
                        new_consecutive = 2;
                    } else {
                        new_status = RouteStatus::Active;
                        new_consecutive = 0;
                    }
                }
                RouteStatus::Severed => {
                    if !enemy_adjacent_to_exposed {
                        let guard_controls = self.has_guard_controlling_exposed(owner, path);
                        if guard_controls {
                            new_status = RouteStatus::Active;
                            new_consecutive = 0;
                        }
                    }
                }
            }

            if new_status != old_status || new_consecutive != consecutive {
                self.routes[route_idx].status = new_status;
                self.routes[route_idx].consecutive_threatened = new_consecutive;
                events.push(GameEvent::RouteStatusChanged {
                    route: route_id,
                    old_status,
                    status: new_status,
                });
            }
        }

        events
    }

    /// Check if any uncontrolled (exposed) tile on the path has an enemy unit
    /// on or adjacent to it.
    fn has_enemy_adjacent_to_exposed(&self, owner: PlayerId, path: &[TileId]) -> bool {
        for &tid in path {
            if !self.is_tile_controlled_by(owner, tid) {
                let tile_coord = self.tiles[tid.0 as usize].coord;
                for unit in &self.units {
                    if unit.owner != owner {
                        let unit_coord = self.tiles[unit.tile.0 as usize].coord;
                        if tile_coord.distance(unit_coord) <= 1 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a friendly patrolling Guard controls an exposed tile on the path.
    fn has_guard_controlling_exposed(&self, owner: PlayerId, path: &[TileId]) -> bool {
        for &tid in path {
            if !self.is_tile_controlled_by(owner, tid) {
                let tile_coord = self.tiles[tid.0 as usize].coord;
                for unit in &self.units {
                    if unit.owner == owner
                        && unit.kind == UnitKind::CaravanGuard
                        && unit.ability == UnitAbility::Patrolling
                    {
                        let unit_coord = self.tiles[unit.tile.0 as usize].coord;
                        if tile_coord.distance(unit_coord) <= 1 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Balance tables
// ---------------------------------------------------------------------------

/// Static movement / economy parameters for a terrain type.
#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
        sight: 3, // updated to match fog-of-war spec (was 2)
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

/// Starting influence granted to each player at world-gen.
pub fn starting_influence() -> u32 {
    FOUND_CITY_INFLUENCE
}

/// Starting wealth granted to each player at world-gen.
pub fn starting_wealth() -> u32 {
    10
}

// ---------------------------------------------------------------------------
// Phase 2 balance tables
// ---------------------------------------------------------------------------

/// Building build costs indexed by [`BuildingKind`] discriminant order:
/// Well, Market, Granary, Watchtower, Caravanserai, Temple.
pub const BUILD_COST: [u32; 6] = [8, 10, 6, 12, 10, 12];

/// Unit training costs indexed by [`UnitKind`] discriminant order:
/// Scout, CaravanGuard, Raider.
pub const UNIT_TRAIN_COST: [u32; 3] = [4, 6, 5];

/// Influence cost to specialize a city.
pub const SPECIALIZE_COST_INFLUENCE: u32 = 10;

/// Minimum population required to specialize a city.
pub const POP_FOR_SPECIALIZE: u32 = 3;

/// Fortress specialization multiplies training cost by this factor.
pub const FORTRESS_TRAIN_DISCOUNT: f32 = 0.75;

/// Trade Hub specialization reduces Market build cost by this amount.
pub const TRADE_HUB_MARKET_DISCOUNT: u32 = 4;

/// Minimum water yield above this threshold for city growth to progress.
pub const GROWTH_WATER_THRESHOLD: u32 = 5;

/// Number of consecutive growth-eligible turns before a population increment.
pub const GROWTH_PERIOD_TURNS: u32 = 3;

/// Base unit cap (added to total population across all cities).
pub const UNIT_CAP_BASE: u32 = 2;

/// Base water stockpile cap.
pub const WATER_CAP_BASE: u32 = 30;

/// Base wealth stockpile cap.
pub const WEALTH_CAP_BASE: u32 = 50;

/// Base influence stockpile cap.
pub const INFLUENCE_CAP_BASE: u32 = 30;

/// Water cap bonus per Granary building.
pub const GRANARY_WATER_BONUS: u32 = 5;

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
// Fog of War — moved from fog.rs free functions
// ---------------------------------------------------------------------------

impl GameState {
    /// Compute the city's sight radius based on buildings and specialization.
    ///
    /// Base sight is [`SIGHT_CITY_BASE`](crate::fog::SIGHT_CITY_BASE); Scholar Outpost adds
    /// [`SIGHT_SCHOLAR_BONUS`](crate::fog::SIGHT_SCHOLAR_BONUS). Watchtower is handled separately in
    /// [`refresh_city_fog`](Self::refresh_city_fog) (it reveals around its own tile).
    pub fn city_sight(&self, city: CityId) -> u32 {
        let c = &self.cities[city.0 as usize];
        let mut radius = crate::fog::SIGHT_CITY_BASE;
        if c.specialization == Some(CitySpecialization::ScholarOutpost) {
            radius += crate::fog::SIGHT_SCHOLAR_BONUS;
        }
        radius
    }

    /// Reveal `range(center, r)` tiles into `player.discovered`.
    ///
    /// Returns the set of *newly* revealed tiles (not already in the discovered
    /// set). Emits a [`GameEvent::Revealed`] if any new tiles were uncovered.
    /// Idempotent: calling with the same center/radius twice is a no-op the
    /// second time.
    pub fn reveal(&mut self, player: PlayerId, center: TileId, r: u32) -> Vec<TileId> {
        let center_coord = self.tiles[center.0 as usize].coord;
        let mut newly: Vec<TileId> = Vec::new();
        for hex in center_coord.range(r) {
            if let Some(&tid) = self.tile_index.get(&hex) {
                if self.players[player.0 as usize].discovered.insert(tid) {
                    newly.push(tid);
                }
            }
        }
        if !newly.is_empty() {
            self.log.push(GameEvent::Revealed {
                player,
                tiles: newly.clone(),
            });
        }
        newly
    }

    /// Reveal fog from a unit's current position using its sight radius.
    ///
    /// Called by the resolver after a unit moves (reveal-on-move, spec §6.2) and
    /// after training a new unit.
    pub fn reveal_from_unit(&mut self, unit_id: UnitId) {
        let u = &self.units[unit_id.0 as usize];
        let actor = u.owner;
        let tile = u.tile;
        let kind = u.kind;
        self.reveal(actor, tile, kind.sight());
    }

    /// Is `tile` currently in `player`'s discovered set?
    pub fn is_tile_visible(&self, player: PlayerId, tile: TileId) -> bool {
        self.players[player.0 as usize].discovered.contains(&tile)
    }

    /// A unit is visible only if its **current** tile is discovered by the viewer.
    ///
    /// Enemy units in fog are **hidden** — including from the AI (ADR-0004 purity).
    pub fn is_unit_visible(&self, viewer: PlayerId, unit_id: UnitId) -> bool {
        let u = &self.units[unit_id.0 as usize];
        self.is_tile_visible(viewer, u.tile)
    }

    /// A city is visible if ANY of its tiles (city tile + worked ring) are
    /// discovered.
    ///
    /// Once seen, stays visible as a memory marker (spec §6.3): the city remains
    /// displayed at its remembered location, but its dynamic state is only
    /// live-updated while currently observed.
    pub fn is_city_visible(&self, viewer: PlayerId, city_id: CityId) -> bool {
        let c = &self.cities[city_id.0 as usize];
        // Check city tile.
        if self.is_tile_visible(viewer, c.tile) {
            return true;
        }
        // Check worked ring (city tile + ring(1)).
        let city_coord = self.tiles[c.tile.0 as usize].coord;
        for hex in city_coord.range(1) {
            if let Some(&tid) = self.tile_index.get(&hex) {
                if self.is_tile_visible(viewer, tid) {
                    return true;
                }
            }
        }
        false
    }

    /// A route is visible if ANY path tile is discovered.
    ///
    /// Static memory marker: once seen, stays visible (spec §6.3).
    pub fn is_route_visible(&self, viewer: PlayerId, route_id: RouteId) -> bool {
        let r = &self.routes[route_id.0 as usize];
        for &tid in &r.path {
            if self.is_tile_visible(viewer, tid) {
                return true;
            }
        }
        false
    }

    /// Re-scan all owned cities and their buildings/specializations to refresh fog.
    ///
    /// Called from [`GameState::advance_turn`](Self::advance_turn) to handle late
    /// building/specialization (Watchtower/Scholar re-reveal, spec §6.2).
    pub fn refresh_city_fog(&mut self) {
        // First pass: collect data without holding a borrow on `self`.
        let cities_data: Vec<(PlayerId, TileId, u32, bool)> = self
            .cities
            .iter()
            .map(|c| {
                // Inline city_sight logic to avoid reborrowing self.
                let mut sight = crate::fog::SIGHT_CITY_BASE;
                if c.specialization == Some(CitySpecialization::ScholarOutpost) {
                    sight += crate::fog::SIGHT_SCHOLAR_BONUS;
                }
                let has_watchtower = c.buildings.contains(&BuildingKind::Watchtower);
                (c.owner, c.tile, sight, has_watchtower)
            })
            .collect();

        // Second pass: reveal fog using the collected data.
        for (owner, tile, sight, has_watchtower) in cities_data {
            self.reveal(owner, tile, sight);
            if has_watchtower {
                self.reveal(owner, tile, crate::fog::SIGHT_WATCHTOWER);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AI opponent planning (spec `behavior-ai-opponents.md`) — moved from `ai.rs`
// free functions. Each method takes `&self` (the immutable `GameState` view)
// and mirrors the original pure-function signatures.
// ---------------------------------------------------------------------------

impl GameState {
    /// Look up the AI personality for this player (reads `player.kind`).
    pub(crate) fn personality_of(&self, player: PlayerId) -> AiPersonality {
        match &self.players[player.0 as usize].kind {
            PlayerKind::Ai { personality, .. } => *personality,
            PlayerKind::Human => AiPersonality::default(),
        }
    }

    /// Build the fog-aware [`Situation`] snapshot for `player`.
    ///
    /// Uses fog queries from `fog.rs` — the AI **never** sees through its own fog.
    pub(crate) fn assess(&self, player: PlayerId) -> Situation {
        let mut sit = Situation {
            own_cities: Vec::new(),
            own_units: Vec::new(),
            own_routes: Vec::new(),
            own_oases: self.oases_controlled_by(player),
            total_oases: self.total_oases(),
            fog_frontier: Vec::new(),
            visible_enemy_units: Vec::new(),
            visible_enemy_cities: Vec::new(),
            visible_enemy_routes: Vec::new(),
            exposed_own_routes: Vec::new(),
            threatened_own_routes: Vec::new(),
            unconnected_own_cities: Vec::new(),
            isolated_cities: Vec::new(),
            turn: self.turn,
            total_turns: self.scenario.turn_limit,
        };

        // --- Own cities ---
        for city in &self.cities {
            if city.owner == player {
                sit.own_cities.push(city.id);
            }
        }

        // --- Own units ---
        for unit in &self.units {
            if unit.owner == player {
                sit.own_units.push(unit.id);
            }
        }

        // --- Own routes ---
        for route in &self.routes {
            if route.owner == player {
                sit.own_routes.push(route.id);
                // Check for exposed tiles.
                let has_exposed = route
                    .path
                    .iter()
                    .any(|&tid| !self.is_route_tile_controlled(route, tid));
                if has_exposed {
                    sit.exposed_own_routes.push(route.id);
                }
                // Check for threatened/severed status.
                if route.status == RouteStatus::Threatened || route.status == RouteStatus::Severed {
                    sit.threatened_own_routes.push(route.id);
                }
            }
        }

        // --- Visible enemy units ---
        for unit in &self.units {
            if unit.owner != player && self.is_unit_visible(player, unit.id) {
                sit.visible_enemy_units.push(unit.id);
            }
        }

        // --- Visible enemy cities ---
        for city in &self.cities {
            if city.owner != player && self.is_city_visible(player, city.id) {
                sit.visible_enemy_cities.push(city.id);
            }
        }

        // --- Visible enemy routes ---
        for route in &self.routes {
            if route.owner != player && self.is_route_visible(player, route.id) {
                sit.visible_enemy_routes.push(route.id);
            }
        }

        // --- Fog frontier: discovered tiles adjacent to undiscovered tiles ---
        let player_discovered = &self.players[player.0 as usize].discovered;
        for &tid in player_discovered.iter() {
            let coord = self.tiles[tid.0 as usize].coord;
            for neighbor in coord.neighbors() {
                if let Some(&nid) = self.tile_index.get(&neighbor) {
                    if !player_discovered.contains(&nid) {
                        sit.fog_frontier.push(tid);
                        break; // one frontier tile is enough per discovered tile
                    }
                }
            }
        }

        // --- Unconnected own cities ---
        // Cities not in the same connected component as any route.
        let connected = self.connected_city_count(player);
        if connected < sit.own_cities.len() as u32 {
            // Find cities that are endpoints of no active route.
            let mut route_endpoints = FxHashSet::default();
            for route in &self.routes {
                if route.owner == player && route.status == RouteStatus::Active {
                    route_endpoints.insert(route.endpoints.0);
                    route_endpoints.insert(route.endpoints.1);
                }
            }
            for &cid in &sit.own_cities {
                if !route_endpoints.contains(&cid) {
                    sit.unconnected_own_cities.push(cid);
                }
            }
        }

        // --- Isolated cities ---
        for &cid in &sit.own_cities {
            if self.is_city_isolated(cid) {
                sit.isolated_cities.push(cid);
            }
        }

        sit
    }

    /// Generate FoundCity commands for expansion.
    ///
    /// For each own city, look at hex neighbors for valid founding spots on oases.
    /// Score based on oasis proximity and distance from own cities.
    pub(crate) fn candidates_expand(&self, player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();
        let player_data = &self.players[player.0 as usize];

        // Need influence to found.
        if player_data.resources.influence < FOUND_CITY_INFLUENCE {
            return candidates;
        }

        // Need a scout to found.
        let scout = self
            .units
            .iter()
            .find(|u| u.owner == player && u.kind == UnitKind::Scout);
        let (scout_id, scout_tile, scout_moves) = match scout {
            Some(u) => (u.id, u.tile, u.moves_left),
            None => return candidates,
        };
        if scout_moves == 0 {
            return candidates;
        }

        let scout_coord = self.tiles[scout_tile.0 as usize].coord;
        let radius = self.scenario.map_radius as u32;

        // Also consider the scout's own tile
        let own_tile_id = self.tile_index[&scout_coord];
        let own_tile = &self.tiles[own_tile_id.0 as usize];
        if own_tile.terrain == TerrainType::Oasis
            && !self.cities.iter().any(|c| c.tile == own_tile_id)
        {
            let is_available = own_tile.owner.is_none() || own_tile.owner == Some(player);
            if is_available {
                candidates.push(ScoredAction {
                    score: 15.0, // highest priority — found on current position
                    cmd: Command::FoundCity {
                        unit: scout_id,
                        tile: own_tile_id,
                    },
                    category: CAT_EXPAND,
                });
            }
        }

        // Look for oases reachable by the scout (adjacent to scout position).
        for neighbor_coord in scout_coord.neighbors() {
            if !neighbor_coord.in_map(radius) {
                continue;
            }
            if let Some(&tile_id) = self.tile_index.get(&neighbor_coord) {
                let tile = &self.tiles[tile_id.0 as usize];
                if tile.terrain != TerrainType::Oasis {
                    continue;
                }
                // Must not already have a city.
                if self.cities.iter().any(|c| c.tile == tile_id) {
                    continue;
                }
                // Must not be owned by an enemy.
                if let Some(owner) = tile.owner {
                    if owner != player {
                        continue;
                    }
                }

                // Score: prefer oases closer to existing cities (network cohesion)
                // and prefer unoccupied oases.
                let min_dist = sit
                    .own_cities
                    .iter()
                    .map(|&cid| {
                        let city_tile = self.cities[cid.0 as usize].tile;
                        let city_coord = self.tiles[city_tile.0 as usize].coord;
                        neighbor_coord.distance(city_coord) as f32
                    })
                    .fold(f32::INFINITY, f32::min);

                // Closer is better; invert distance for score.
                let score = 10.0 / (min_dist + 1.0);

                candidates.push(ScoredAction {
                    score,
                    cmd: Command::FoundCity {
                        unit: scout_id,
                        tile: tile_id,
                    },
                    category: CAT_EXPAND,
                });
            }
        }

        candidates
    }

    /// Generate TrainUnit commands for building up military.
    ///
    /// Prioritize if `own_units.len() < own_cities.len()` (under-militarized).
    pub(crate) fn candidates_build(&self, player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();
        let player_data = &self.players[player.0 as usize];

        // Check unit cap.
        let cap = self.unit_cap(player);
        let current_units = sit.own_units.len() as u32;
        if current_units >= cap {
            return candidates;
        }

        // Determine deficit: more cities than units → need more units.
        let unit_deficit = if sit.own_cities.len() as u32 > current_units {
            (sit.own_cities.len() as u32 - current_units) as f32
        } else {
            0.0
        };

        // Determine if we need more guards for exposed routes.
        let guards_needed = sit.exposed_own_routes.len() as f32;
        let current_guards = self
            .units
            .iter()
            .filter(|u| u.owner == player && u.kind == UnitKind::CaravanGuard)
            .count() as f32;

        // Try each city for training.
        for &cid in &sit.own_cities {
            let city = &self.cities[cid.0 as usize];

            // Determine what to train.
            let kind_to_train = if current_guards < guards_needed {
                UnitKind::CaravanGuard
            } else if unit_deficit > 0.0 {
                UnitKind::Scout
            } else {
                // No pressing need — skip.
                continue;
            };

            let cost = UNIT_TRAIN_COST[kind_to_train.index()];
            // Apply Fortress discount if applicable.
            let actual_cost = if city.specialization == Some(CitySpecialization::Fortress) {
                (cost as f32 * FORTRESS_TRAIN_DISCOUNT) as u32
            } else {
                cost
            };

            if player_data.resources.wealth < actual_cost {
                continue;
            }

            let score = if kind_to_train == UnitKind::CaravanGuard {
                // Higher score if we have exposed routes needing guards.
                guards_needed * 2.0
            } else {
                unit_deficit
            };

            candidates.push(ScoredAction {
                score,
                cmd: Command::TrainUnit {
                    city: cid,
                    kind: kind_to_train,
                },
                category: CAT_BUILD,
            });
        }

        candidates
    }

    /// Generate ConnectRoute commands for unconnected city pairs.
    pub(crate) fn candidates_connect(
        &self,
        player: PlayerId,
        sit: &Situation,
    ) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();
        let player_data = &self.players[player.0 as usize];

        // We need unconnected cities and available resources.
        if sit.own_cities.len() < 2 {
            return candidates;
        }

        // Try all pairs of own cities.
        for i in 0..sit.own_cities.len() {
            for j in (i + 1)..sit.own_cities.len() {
                let from = sit.own_cities[i];
                let to = sit.own_cities[j];

                // Check if already connected by an active route.
                let already_connected = self.routes.iter().any(|r| {
                    r.owner == player
                        && r.status == RouteStatus::Active
                        && ((r.endpoints.0 == from && r.endpoints.1 == to)
                            || (r.endpoints.0 == to && r.endpoints.1 == from))
                });
                if already_connected {
                    continue;
                }

                // Check route slots on both endpoints.
                let city_a = &self.cities[from.0 as usize];
                let city_b = &self.cities[to.0 as usize];
                if city_a.route_slots == 0 || city_b.route_slots == 0 {
                    continue;
                }

                // Compute route preview.
                let (cost, path) = self.preview_cost(from, to);
                if path.is_empty() {
                    continue;
                }

                // Check if we can afford it.
                if (player_data.resources.wealth as i32) < cost {
                    continue;
                }

                // Check fog-legal: all path tiles must be discovered.
                let player_discovered = &self.players[player.0 as usize].discovered;
                let fog_legal = path.iter().all(|&tid| player_discovered.contains(&tid));
                if !fog_legal {
                    continue;
                }

                // Score: network synergy improvement × route_weight.
                let synergy_bonus = if sit.unconnected_own_cities.contains(&from)
                    || sit.unconnected_own_cities.contains(&to)
                {
                    5.0
                } else {
                    2.0
                };

                candidates.push(ScoredAction {
                    score: synergy_bonus,
                    cmd: Command::ConnectRoute { from, to },
                    category: CAT_CONNECT,
                });
            }
        }

        candidates
    }

    /// Generate Patrol commands for defending exposed routes.
    pub(crate) fn candidates_defend(
        &self,
        _player: PlayerId,
        sit: &Situation,
    ) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();

        // Find idle guards (not patrolling, not garrisoned).
        let idle_guards: Vec<UnitId> = self
            .units
            .iter()
            .filter(|u| {
                u.owner == _player
                    && u.kind == UnitKind::CaravanGuard
                    && u.ability == UnitAbility::None
                    && u.moves_left > 0
            })
            .map(|u| u.id)
            .collect();

        if idle_guards.is_empty() {
            return candidates;
        }

        // For each exposed route, find the most exposed tile.
        let mut seen_routes = FxHashSet::default();
        for &route_id in sit
            .exposed_own_routes
            .iter()
            .chain(sit.threatened_own_routes.iter())
        {
            if !seen_routes.insert(route_id) {
                continue;
            }
            let route = &self.routes[route_id.0 as usize];

            // Find the first uncontrolled tile.
            for &tid in &route.path {
                if !self.is_route_tile_controlled(route, tid) {
                    // Try to move an idle guard to patrol near this tile.
                    if let Some(&guard_id) = idle_guards.first() {
                        let guard = &self.units[guard_id.0 as usize];
                        let guard_tile = guard.tile;
                        let guard_coord = self.tiles[guard_tile.0 as usize].coord;
                        let tile_coord = self.tiles[tid.0 as usize].coord;

                        // If already adjacent or on the tile, just patrol.
                        if guard_coord.distance(tile_coord) <= 1 {
                            candidates.push(ScoredAction {
                                score: 3.0,
                                cmd: Command::Patrol {
                                    unit: guard_id,
                                    tile: tid,
                                },
                                category: CAT_DEFEND,
                            });
                        } else if guard.moves_left > 0 {
                            candidates.push(ScoredAction {
                                score: 2.0,
                                cmd: Command::MoveUnit {
                                    unit: guard_id,
                                    to: tid,
                                },
                                category: CAT_DEFEND,
                            });
                        }
                    }
                    break; // One tile per route is enough.
                }
            }
        }

        candidates
    }

    /// Generate RaidRoute / RaidCity commands (fog-aware — no cheating).
    pub(crate) fn candidates_raid(
        &self,
        _player: PlayerId,
        sit: &Situation,
        params: &AiParams,
    ) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();

        if params.raid_weight <= 0.0 {
            return candidates;
        }

        // Find idle raiders.
        let idle_raiders: Vec<UnitId> = self
            .units
            .iter()
            .filter(|u| u.owner == _player && u.kind == UnitKind::Raider && u.moves_left > 0)
            .map(|u| u.id)
            .collect();

        if idle_raiders.is_empty() {
            return candidates;
        }

        // Try raiding visible enemy routes.
        for &route_id in &sit.visible_enemy_routes {
            let route = &self.routes[route_id.0 as usize];
            if route.status != RouteStatus::Active {
                continue;
            }

            // Find an exposed tile on the enemy route.
            for &tid in &route.path {
                if !self.is_route_tile_controlled(route, tid) {
                    if let Some(&raider_id) = idle_raiders.first() {
                        let raider = &self.units[raider_id.0 as usize];
                        let raider_coord = self.tiles[raider.tile.0 as usize].coord;
                        let tile_coord = self.tiles[tid.0 as usize].coord;

                        if raider_coord.distance(tile_coord) <= 1 {
                            candidates.push(ScoredAction {
                                score: params.raid_weight * 3.0,
                                cmd: Command::RaidRoute {
                                    unit: raider_id,
                                    route: route_id,
                                },
                                category: CAT_RAID,
                            });
                        }
                    }
                    break;
                }
            }
        }

        // Try raiding visible enemy cities.
        for &city_id in &sit.visible_enemy_cities {
            if let Some(&raider_id) = idle_raiders.first() {
                let raider = &self.units[raider_id.0 as usize];
                let city = &self.cities[city_id.0 as usize];
                let raider_coord = self.tiles[raider.tile.0 as usize].coord;
                let city_coord = self.tiles[city.tile.0 as usize].coord;

                if raider_coord.distance(city_coord) <= 1 {
                    candidates.push(ScoredAction {
                        score: params.raid_weight * 2.0,
                        cmd: Command::RaidCity {
                            unit: raider_id,
                            city: city_id,
                        },
                        category: CAT_RAID,
                    });
                }
            }
        }

        candidates
    }

    /// Generate MoveUnit commands for scouting.
    pub(crate) fn candidates_scout(&self, _player: PlayerId, sit: &Situation) -> Vec<ScoredAction> {
        let mut candidates = Vec::new();

        if sit.fog_frontier.is_empty() {
            return candidates;
        }

        // Find idle scouts.
        for unit in self.units.iter() {
            if unit.owner != _player || unit.kind != UnitKind::Scout || unit.moves_left == 0 {
                continue;
            }
            if unit.ability != UnitAbility::None {
                continue;
            }

            // Find the nearest fog frontier tile.
            let unit_coord = self.tiles[unit.tile.0 as usize].coord;
            let mut best_tile = None;
            let mut best_dist = u32::MAX;

            for &fid in sit.fog_frontier.iter().take(20) {
                // Limit search for performance.
                let f_coord = self.tiles[fid.0 as usize].coord;
                let dist = unit_coord.distance(f_coord);
                if dist < best_dist {
                    best_dist = dist;
                    best_tile = Some(fid);
                }
            }

            if let Some(target) = best_tile {
                let score = 1.0; // Base exploration score.
                candidates.push(ScoredAction {
                    score,
                    cmd: Command::MoveUnit {
                        unit: unit.id,
                        to: target,
                    },
                    category: CAT_SCOUT,
                });
            }
        }

        candidates
    }

    /// Score candidate actions by weighted utility; returns them sorted descending.
    ///
    /// Calls all `candidates_*` functions internally, applies per-category weights,
    /// and sorts the combined results.
    pub(crate) fn prioritize(
        &self,
        player: PlayerId,
        sit: &Situation,
        params: &AiParams,
    ) -> Vec<ScoredAction> {
        let mut all: Vec<ScoredAction> = Vec::new();

        // Generate candidates from all categories.
        all.extend(self.candidates_expand(player, sit));
        all.extend(self.candidates_build(player, sit));
        all.extend(self.candidates_connect(player, sit));
        all.extend(self.candidates_defend(player, sit));
        all.extend(self.candidates_raid(player, sit, params));
        all.extend(self.candidates_scout(player, sit));

        // Apply category weights.
        for action in &mut all {
            let weight = match action.category {
                CAT_CONNECT => params.route_weight,
                CAT_DEFEND => params.route_security,
                CAT_EXPAND => params.expand_weight,
                CAT_RAID => params.raid_weight,
                CAT_SCOUT => params.scout_weight,
                CAT_BUILD => params.build_weight,
                _ => 1.0,
            };
            action.score *= weight;
        }

        // Sort by score descending; break ties by category, then by command
        // ordering for determinism.
        all.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.category.cmp(&b.category))
        });

        all
    }

    /// Validate and emit a legal, budget-limited command list.
    ///
    /// For each `ScoredAction`, run `self.validate(cmd)`. Keep it if legal and
    /// the per-turn `command_budget` is not exceeded.
    pub(crate) fn emit(
        &self,
        _player: PlayerId,
        ranked: &[ScoredAction],
        params: &AiParams,
    ) -> Vec<Command> {
        let mut commands = Vec::new();
        let mut budget_remaining = params.command_budget;

        for action in ranked {
            if budget_remaining == 0 {
                break;
            }

            // Skip commands that are no longer valid due to earlier mutations
            // (budget-limited, so we validate against the original state).
            match self.validate(&action.cmd) {
                Ok(()) => {
                    commands.push(action.cmd.clone());
                    budget_remaining -= 1;
                }
                Err(_) => {
                    // Command is illegal — skip it.
                }
            }
        }

        commands
    }

    /// Generate a list of commands for an AI player.
    ///
    /// The AI reads the game state, builds a fog-aware situation snapshot,
    /// generates candidate actions, scores them by personality weights,
    /// validates each via `GameState::validate`, and returns a budget-capped list.
    ///
    /// # Arguments
    /// * `player` - The AI player to plan for
    /// * `difficulty` - AI difficulty level affecting decision quality
    ///
    /// # Returns
    /// A vector of `Command`s the AI wants to execute, ordered by priority.
    ///
    /// The orchestrator appends `EndTurn` before calling `step` — `ai_plan` returns
    /// actions only (no `EndTurn`).
    ///
    /// Returns an empty `Vec` if the player has no cities (defeated).
    pub fn ai_plan(&self, player: PlayerId, difficulty: Difficulty) -> Vec<Command> {
        let personality = self.personality_of(player);
        let params = crate::ai::params_for(personality, difficulty);
        let sit = self.assess(player);

        // NOTE: We intentionally do NOT bail out when `own_cities` is empty.
        // At game start, players begin without cities and must FoundCity via a
        // scout. The candidate generators (especially `candidates_expand`) handle
        // the empty-city case by requiring influence + a scout with moves. If
        // the player is truly defeated (no cities, no units), all candidates
        // will be empty and `emit` returns `Vec::new()` naturally.

        let ranked = self.prioritize(player, &sit, &params);
        self.emit(player, &ranked, &params)
    }
}

// ---------------------------------------------------------------------------
// Legal actions (used by the serve protocol layer)
// ---------------------------------------------------------------------------

impl GameState {
    /// Compute the legal actions for a player given the current game state.
    ///
    /// Returns a list of command name strings that the player can execute right now.
    /// This is a simplified MVP check — it does not validate full preconditions
    /// (e.g. resource costs), only whether the player has the prerequisite entities.
    pub fn legal_actions_for(&self, player_id: PlayerId) -> Vec<String> {
        let mut actions = Vec::new();
        let is_my_turn = self.current_actor == player_id;

        // Collect the player's units and cities once for reuse.
        let player_units: Vec<_> = self.units.iter().filter(|u| u.owner == player_id).collect();
        let player_cities: Vec<_> = self
            .cities
            .iter()
            .filter(|c| c.owner == player_id)
            .collect();

        // end_turn: always available when it's the player's turn.
        if is_my_turn {
            actions.push("end_turn".into());
        }

        // All other actions are only available when it's the player's turn.
        if is_my_turn {
            // move_unit: available if the player has any units.
            if !player_units.is_empty() {
                actions.push("move_unit".into());
            }

            // found_city: available if the player has a scout on an oasis tile
            // that doesn't already have a city.
            let has_founding_scout = player_units.iter().any(|u| {
                u.kind == UnitKind::Scout
                    && u.moves_left > 0
                    && self.tiles[u.tile.0 as usize].terrain == TerrainType::Oasis
                    && !self.cities.iter().any(|c| c.tile == u.tile)
            });
            if has_founding_scout {
                actions.push("found_city".into());
            }

            // build: available if the player has cities with free building slots.
            let has_buildable_city = player_cities.iter().any(|c| {
                let slots = c.building_slots();
                (c.buildings.len() as u8) < slots
            });
            if has_buildable_city {
                actions.push("build".into());
            }

            // train_unit: available if the player has cities (population >= 1).
            if player_cities.iter().any(|c| c.population >= 1) {
                actions.push("train_unit".into());
            }

            // patrol: available if the player has CaravanGuard units.
            if player_units
                .iter()
                .any(|u| u.kind == UnitKind::CaravanGuard)
            {
                actions.push("patrol".into());
            }

            // garrison: available if the player has CaravanGuard units and cities.
            if player_units
                .iter()
                .any(|u| u.kind == UnitKind::CaravanGuard)
                && !player_cities.is_empty()
            {
                actions.push("garrison".into());
            }

            // raid: available if the player has Raider units.
            if player_units.iter().any(|u| u.kind == UnitKind::Raider) {
                actions.push("raid".into());
            }

            // connect_route: available if the player has 2+ cities.
            if player_cities.len() >= 2 {
                actions.push("connect_route".into());
            }
        }

        actions
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{TerrainTypeDef, UnitKindExt};
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
            let _ = t.def();
        }
        for k in [UnitKind::Scout, UnitKind::CaravanGuard, UnitKind::Raider] {
            let _ = k.def();
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
