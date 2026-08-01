//! `dcs-render`: presentation layer for `dcs-core`.
//!
//! Depends on `dcs-core`, `dcs-protocol`, and `macroquad`.

use dcs_core::hex;
use dcs_core::{
    Command, GameState, PlayerColor, PlayerId, RouteStatus, TerrainType, TileId, Unit, UnitId,
    UnitKind,
};
use macroquad::prelude::*;

/// Configuration for the render window.
pub struct RenderConfig {
    pub window_title: String,
    pub window_width: i32,
    pub window_height: i32,
    pub background_color: Color,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            window_title: "Desert City-States".to_string(),
            window_width: 1280,
            window_height: 720,
            background_color: Color::from_rgba(194, 178, 128, 255), // desert sand
        }
    }
}

/// World pixels per hex "radius" (center to corner).
const HEX_SIZE: f32 = 32.0;

/// Per-scroll-tick zoom change, as a fraction of current zoom.
const ZOOM_SPEED: f32 = 0.1;

/// How far past `min_zoom_scale` (the whole-map-visible level) scrolling in
/// is allowed to go, so you can't zoom into a sliver of a single tile.
const MAX_ZOOM_MULTIPLIER: f32 = 6.0;

/// Max screen-pixel movement between a left mouse press and release still
/// counted as a click (selection) rather than a drag (camera pan).
const CLICK_DRAG_THRESHOLD: f32 = 6.0;

/// Flat fill for tiles outside the viewer's discovered set — no terrain
/// detail is shown, per the fog spec (`presentation-rendering-ui.md` §6.3).
const FOG_TILE_COLOR: Color = Color::from_rgba(24, 22, 20, 255);

/// RGB multiplier used to render a fog "memory marker" — a city/route whose
/// location is remembered but whose current dynamic state (population,
/// route status) isn't directly observed this frame (spec §6.3).
const DIM_FACTOR: f32 = 0.45;

/// Turns a `&GameState` into pixels. Read-only (ADR-0003, Rule B) — no
/// `&mut GameState` exists here or anywhere outside `dcs-app`.
///
/// `map_bounds`/`zoom_scale`/`min_zoom_scale`/`drag_anchor`/`selected_unit`/
/// `left_press_pos`/`just_clicked` are ephemeral camera- and UI-control state
/// (never part of `GameState`, never serialized — CLAUDE.md's camera/UI
/// state rule).
pub struct Renderer {
    pub camera: Camera2D,
    pub hex_size: f32,
    map_bounds: Rect,
    /// Current scale, in screen pixels per world unit. The source of truth
    /// for zoom level — `camera.zoom` (macroquad's per-axis NDC scale) is
    /// derived from this every frame via `sync_zoom`, rather than being
    /// mutated directly, so zoom stays aspect-ratio-independent regardless
    /// of the current window shape.
    zoom_scale: f32,
    /// `zoom_scale` at which the whole map exactly fit the window when
    /// `fit_map` last ran — the zoom-out floor.
    min_zoom_scale: f32,
    drag_anchor: Option<Vec2>,
    /// The currently selected unit, if any (`docs/specs/presentation-rendering-ui.md`
    /// §6.5's `UiView.selected_unit`). Only unit selection is tracked this
    /// slice — no `selected_city`/`selected_tile` yet, since nothing uses them.
    selected_unit: Option<UnitId>,
    /// Screen position of an in-progress left-button press, for click-vs-drag
    /// disambiguation against the existing pan-drag behavior in
    /// `handle_input`. Internal bookkeeping only.
    left_press_pos: Option<Vec2>,
    /// Set by `handle_input` for exactly one frame when a left click (not a
    /// drag) just completed, consumed by `poll_input` the same frame.
    just_clicked: Option<Vec2>,
}

impl Renderer {
    /// Centers the camera and records the scale at which every tile in
    /// `state` is visible, as the pan/zoom-out limits.
    pub fn fit_map(&mut self, state: &GameState) {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for tile in &state.tiles {
            let (x, y) = tile.coord.to_pixel(self.hex_size);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let padding = self.hex_size * 2.0;
        let bounds = Rect::new(
            min_x - padding,
            min_y - padding,
            (max_x - min_x) + padding * 2.0,
            (max_y - min_y) + padding * 2.0,
        );
        // `.min(...)`, not the larger ratio: "contain" the map inside the
        // window rather than "cover" it, so the shorter axis shows extra
        // background instead of the map being cropped.
        let min_zoom_scale = (screen_width() / bounds.w).min(screen_height() / bounds.h);
        self.zoom_scale = min_zoom_scale;
        self.min_zoom_scale = min_zoom_scale;
        self.camera.target = bounds.center();
        self.camera.rotation = 0.0;
        self.camera.offset = Vec2::ZERO;
        self.map_bounds = bounds;
        self.sync_zoom();
    }

    /// Drag-to-pan (left button) and cursor-anchored scroll-to-zoom.
    /// Call once per frame, before `draw_frame`.
    pub fn handle_input(&mut self) {
        // Re-derive `camera.zoom` against the *current* screen size first,
        // so a live window resize is corrected before any pan/zoom math
        // (which reads `camera.zoom` via screen_to_world/world_to_screen)
        // runs this frame.
        self.sync_zoom();

        let mouse_screen = Vec2::from(mouse_position());

        if is_mouse_button_pressed(MouseButton::Left) {
            self.left_press_pos = Some(mouse_screen);
        }

        if is_mouse_button_down(MouseButton::Left) {
            let current_world = self.camera.screen_to_world(mouse_screen);
            if let Some(anchor) = self.drag_anchor {
                self.camera.target += anchor - current_world;
            }
            self.drag_anchor = Some(self.camera.screen_to_world(mouse_screen));
        } else {
            self.drag_anchor = None;
        }

        if is_mouse_button_released(MouseButton::Left) {
            if let Some(press_pos) = self.left_press_pos.take() {
                if press_pos.distance(mouse_screen) < CLICK_DRAG_THRESHOLD {
                    self.just_clicked = Some(mouse_screen);
                }
            }
        }

        let wheel_y = mouse_wheel().1;
        if wheel_y != 0.0 {
            let world_before = self.camera.screen_to_world(mouse_screen);
            // A fixed step per wheel tick, not scaled by the raw delta
            // magnitude: real devices report wildly different deltaY scales
            // (trackpad inertial flings can report values in the hundreds),
            // so scaling directly by `wheel_y` would make zoom speed
            // unpredictable and, at the extreme, numerically degenerate.
            self.zoom_scale *= 1.0 + wheel_y.signum() * ZOOM_SPEED;
            self.zoom_scale = self.zoom_scale.clamp(
                self.min_zoom_scale,
                self.min_zoom_scale * MAX_ZOOM_MULTIPLIER,
            );
            self.sync_zoom();
            let world_after = self.camera.screen_to_world(mouse_screen);
            self.camera.target += world_before - world_after;
        }

        self.clamp_target();
    }

    /// Converts a screen-space point (e.g. `mouse_position()`) to the hex it
    /// falls in, via the same cube-round `dcs-core` uses, so clicks map to
    /// exactly the tile core pathfinding/placement would agree on.
    pub fn screen_to_hex(&self, screen: Vec2) -> hex::HexCoord {
        let world = self.camera.screen_to_world(screen);
        hex::HexCoord::from_pixel((world.x, world.y), self.hex_size)
    }

    /// Translates keyboard/pointer input into `Command`s against the current
    /// `GameState` read-only view (ADR-0003, Rule B — mutates only `self`'s
    /// own ephemeral UI state, never `state`).
    ///
    /// Left-click selects a friendly unit at the clicked hex (or deselects,
    /// if there isn't one); right-click issues a validated `MoveUnit` for
    /// the current selection. No city/tile selection or other action types
    /// yet (`FoundCity`, route-planning, `Patrol`, `RaidRoute`/`RaidCity`) —
    /// future slices extend this same method, not a new one.
    pub fn poll_input(&mut self, state: &GameState) -> Vec<Command> {
        if let Some(pos) = self.just_clicked.take() {
            let hex_coord = self.screen_to_hex(pos);
            self.selected_unit = state
                .tile_at(hex_coord)
                .filter(|tile| state.is_tile_visible(state.current_actor, tile.id))
                .and_then(|tile| {
                    state
                        .units_on(tile.id)
                        .find(|u| u.owner == state.current_actor)
                        .map(|u| u.id)
                });
        }

        let mut commands = Vec::new();

        if is_key_pressed(KeyCode::Space) {
            commands.push(Command::EndTurn);
        }

        if is_mouse_button_pressed(MouseButton::Right) {
            if let Some(unit_id) = self.selected_unit {
                let unit_tile = state.units.iter().find(|u| u.id == unit_id).map(|u| u.tile);
                if let Some(unit_tile) = unit_tile {
                    let hex_coord = self.screen_to_hex(Vec2::from(mouse_position()));
                    if let Some(tile) = state.tile_at(hex_coord) {
                        // Fog guard: a tile the acting player hasn't
                        // discovered can't be targeted for planning (spec
                        // §6.3) — a precondition to attempting a command at
                        // all, distinct from (and checked before) the game-
                        // rule legality check below.
                        if state.is_tile_visible(state.current_actor, tile.id) {
                            // Right-clicking the selected unit's own tile can
                            // only sensibly mean "found a city here" (moving
                            // a unit to the tile it's already on is
                            // meaningless); any other tile means "move
                            // there" — a clean, non-overlapping split needing
                            // no separate arming state.
                            let cmd = if tile.id == unit_tile {
                                Command::FoundCity {
                                    unit: unit_id,
                                    tile: tile.id,
                                }
                            } else {
                                Command::MoveUnit {
                                    unit: unit_id,
                                    to: tile.id,
                                }
                            };
                            if state.validate(&cmd).is_ok() {
                                commands.push(cmd);
                            }
                        }
                    }
                }
            }
        }

        commands
    }

    /// Derives macroquad's per-axis `camera.zoom` from `zoom_scale` against
    /// the current screen size, keeping both axes uniformly scaled (fixing
    /// the map-stretches-on-resize bug `Camera2D::from_display_rect` alone
    /// would otherwise cause, since it scales each axis to independently
    /// fill whatever the window's current aspect ratio happens to be).
    fn sync_zoom(&mut self) {
        self.camera.zoom = vec2(
            2.0 * self.zoom_scale / screen_width(),
            -2.0 * self.zoom_scale / screen_height(),
        );
    }

    /// Keeps the camera target inside the map's bounding rect, so panning
    /// can't drift the view off into empty background.
    fn clamp_target(&mut self) {
        self.camera.target.x = self
            .camera
            .target
            .x
            .clamp(self.map_bounds.x, self.map_bounds.x + self.map_bounds.w);
        self.camera.target.y = self
            .camera
            .target
            .y
            .clamp(self.map_bounds.y, self.map_bounds.y + self.map_bounds.h);
    }

    /// One immediate-mode frame: tiles, routes, cities, units, fog (spec draw
    /// order, `docs/specs/presentation-rendering-ui.md` §6.2/§6.3 — HUD layer
    /// doesn't exist yet).
    ///
    /// Fog is rendered from `state.current_actor`'s point of view — the same
    /// "whose turn is it" value `poll_input` already uses for selection
    /// ownership, and the same value the "Turn N - Player X" HUD text reads.
    /// Since this runs *after* `on_frame` (see [`run`]), a just-processed
    /// `EndTurn` is already reflected here — the screen always shows the
    /// about-to-act player's fog, matching that existing HUD text exactly.
    ///
    /// Tiles render in three tiers, same live/memory split as cities/routes:
    /// never discovered → flat fog color; discovered but outside current
    /// live sight → dimmed terrain (remembered, not necessarily still
    /// accurate); currently observed → full terrain color.
    pub fn draw_frame(&self, state: &GameState) {
        let view_player = state.current_actor;
        set_camera(&self.camera);
        for tile in &state.tiles {
            let (x, y) = tile.coord.to_pixel(self.hex_size);
            let fill = if !state.is_tile_visible(view_player, tile.id) {
                FOG_TILE_COLOR
            } else if state.is_tile_currently_observed(view_player, tile.id) {
                terrain_color(tile.terrain)
            } else {
                dim(terrain_color(tile.terrain))
            };
            draw_hexagon(
                x,
                y,
                self.hex_size * 0.95,
                1.0,
                true, // pointy-top, matching hex::to_pixel's orientation
                BLACK,
                fill,
            );
        }
        draw_routes(state, self.hex_size, view_player);
        draw_cities(state, self.hex_size, view_player);
        draw_units(state, self.hex_size, view_player);
        if let Some(unit) = self.selected_unit {
            draw_selection(state, unit, self.hex_size);
        }
        set_default_camera();
    }
}

fn terrain_color(terrain: TerrainType) -> Color {
    match terrain {
        TerrainType::Oasis => Color::from_rgba(64, 156, 148, 255),
        TerrainType::Dunes => Color::from_rgba(214, 186, 130, 255),
        TerrainType::SaltFlats => Color::from_rgba(226, 222, 210, 255),
        TerrainType::Ridges => Color::from_rgba(120, 92, 68, 255),
        TerrainType::Ruins => Color::from_rgba(150, 120, 150, 255),
    }
}

/// Dims a color's RGB channels toward black by [`DIM_FACTOR`], preserving
/// alpha — used for fog "memory marker" rendering.
fn dim(color: Color) -> Color {
    Color::new(
        color.r * DIM_FACTOR,
        color.g * DIM_FACTOR,
        color.b * DIM_FACTOR,
        color.a,
    )
}

fn tile_pixel(state: &GameState, tile: TileId, hex_size: f32) -> (f32, f32) {
    state.tiles[tile.0 as usize].coord.to_pixel(hex_size)
}

fn owner_color(state: &GameState, owner: PlayerId) -> Color {
    match state.players[owner.0 as usize].color {
        PlayerColor::Sand => Color::from_rgba(240, 230, 200, 255),
        PlayerColor::Crimson => Color::from_rgba(200, 30, 40, 255),
        PlayerColor::Teal => Color::from_rgba(20, 150, 170, 255),
        PlayerColor::Violet => Color::from_rgba(150, 90, 210, 255),
    }
}

fn draw_dashed_line(x1: f32, y1: f32, x2: f32, y2: f32, dash: f32, gap: f32, color: Color) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < f32::EPSILON {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let step = dash + gap;
    let mut travelled = 0.0;
    while travelled < len {
        let seg_end = (travelled + dash).min(len);
        draw_line(
            x1 + ux * travelled,
            y1 + uy * travelled,
            x1 + ux * seg_end,
            y1 + uy * seg_end,
            2.0,
            color,
        );
        travelled += step;
    }
}

/// Fog-filtered per spec §6.3: routes the viewer has never discovered any
/// path tile of are skipped entirely (`is_route_visible`); once ever seen, a
/// route stays visible as a memory marker (`dim(owner_color)`, status hidden
/// — `status` is dynamic and must not leak through color even when dimmed)
/// unless the viewer currently has live sight on one of its path tiles
/// (`is_route_currently_observed`), in which case it's drawn live with its
/// real status color — this genuinely toggles both ways as sight moves on
/// and off the route, not a one-way ratchet.
fn draw_routes(state: &GameState, hex_size: f32, view_player: PlayerId) {
    for route in &state.routes {
        let is_own = route.owner == view_player;
        if !is_own && !state.is_route_visible(view_player, route.id) {
            continue;
        }
        let currently_observed = is_own || state.is_route_currently_observed(view_player, route.id);
        let color = if currently_observed {
            match route.status {
                RouteStatus::Active => owner_color(state, route.owner),
                RouteStatus::Threatened | RouteStatus::Severed => RED,
            }
        } else {
            dim(owner_color(state, route.owner))
        };
        for pair in route.path.windows(2) {
            let (x1, y1) = tile_pixel(state, pair[0], hex_size);
            let (x2, y2) = tile_pixel(state, pair[1], hex_size);
            draw_dashed_line(x1, y1, x2, y2, 6.0, 4.0, color);
        }
    }
}

/// Fog-filtered per spec §6.3: cities the viewer has never discovered the
/// tile or worked ring of are skipped entirely (`is_city_visible`); once
/// ever seen, a city stays visible as a memory marker (dimmed, fixed-size —
/// `population` is dynamic and must not leak through marker size even when
/// dimmed) unless the viewer currently has live sight on the city's own
/// tile (`is_city_currently_observed`), in which case it's drawn live,
/// scaled by its real population — this genuinely toggles both ways as
/// sight moves on and off the city, not a one-way ratchet.
fn draw_cities(state: &GameState, hex_size: f32, view_player: PlayerId) {
    for city in &state.cities {
        let is_own = city.owner == view_player;
        if !is_own && !state.is_city_visible(view_player, city.id) {
            continue;
        }
        let currently_observed = is_own || state.is_city_currently_observed(view_player, city.id);
        let (x, y) = tile_pixel(state, city.tile, hex_size);
        let base_color = owner_color(state, city.owner);
        if currently_observed {
            let radius = (hex_size * 0.25 + city.population as f32 * 1.5).min(hex_size * 0.8);
            draw_circle(x, y, radius, base_color);
            draw_circle_lines(x, y, radius, 1.5, BLACK);
        } else {
            let radius = hex_size * 0.3;
            draw_circle(x, y, radius, dim(base_color));
            draw_circle_lines(x, y, radius, 1.5, dim(BLACK));
        }
    }
}

/// Fog-filtered per spec §6.3: enemy units are drawn only while
/// `is_unit_visible` holds *this frame* — no memory marker, unlike
/// cities/routes, since a unit's position isn't static.
fn draw_units(state: &GameState, hex_size: f32, view_player: PlayerId) {
    for unit in &state.units {
        if unit.owner != view_player && !state.is_unit_visible(view_player, unit.id) {
            continue;
        }
        let (x, y) = unit_marker_pos(state, unit, hex_size);
        draw_unit_marker(unit, x, y, owner_color(state, unit.owner), hex_size);
    }
}

/// A unit's marker position, offset off-center when it shares a tile with a
/// city so the two markers don't fully overlap. Shared by `draw_units` and
/// `draw_selection` so the highlight ring lines up with the marker exactly.
fn unit_marker_pos(state: &GameState, unit: &Unit, hex_size: f32) -> (f32, f32) {
    let (mut x, mut y) = tile_pixel(state, unit.tile, hex_size);
    if state.cities.iter().any(|c| c.tile == unit.tile) {
        x += hex_size * 0.35;
        y -= hex_size * 0.35;
    }
    (x, y)
}

/// Highlight ring around the selected unit's marker (placeholder styling —
/// the spec doesn't specify selection visuals, `docs/specs/presentation-rendering-ui.md`).
fn draw_selection(state: &GameState, unit: UnitId, hex_size: f32) {
    if let Some(unit) = state.units.iter().find(|u| u.id == unit) {
        let (x, y) = unit_marker_pos(state, unit, hex_size);
        draw_circle_lines(x, y, hex_size * 0.3, 3.0, YELLOW);
    }
}

/// Shape per `UnitKind` stands in for the spec's iconographic tokens
/// (DD §3.4) until real art exists.
fn draw_unit_marker(unit: &Unit, x: f32, y: f32, color: Color, hex_size: f32) {
    let radius = hex_size * 0.2;
    match unit.kind {
        UnitKind::Scout => draw_poly(x, y, 3, radius, 0.0, color),
        UnitKind::CaravanGuard => draw_poly(x, y, 4, radius, 45.0, color),
        UnitKind::Raider => draw_poly(x, y, 4, radius, 0.0, color),
    }
}

/// Launch the macroquad window and run the render loop over a real,
/// caller-owned `GameState`.
///
/// `on_frame` is called once per frame with `&mut GameState` and
/// `&mut Renderer` (the latter `&mut` for `Renderer::poll_input`, which
/// updates ephemeral selection state) — it is the *only* place that should
/// call `GameState::step`, keeping the actual state mutation in the
/// caller's code (ADR-0003, Rule C: `dcs-app` owns the main loop) even
/// though this function drives the frame loop itself.
///
/// This function blocks until the window is closed. It takes over the main
/// thread — call it only from the GUI entry point, never from async code.
pub fn run(
    config: RenderConfig,
    mut state: GameState,
    mut on_frame: impl FnMut(&mut GameState, &mut Renderer) + 'static,
) {
    macroquad::Window::from_config(
        Conf {
            window_title: config.window_title,
            window_width: config.window_width,
            window_height: config.window_height,
            ..Conf::default()
        },
        async move {
            let mut renderer = Renderer {
                camera: Camera2D::default(),
                hex_size: HEX_SIZE,
                map_bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
                zoom_scale: 1.0,
                min_zoom_scale: 1.0,
                drag_anchor: None,
                selected_unit: None,
                left_press_pos: None,
                just_clicked: None,
            };
            renderer.fit_map(&state);

            loop {
                clear_background(config.background_color);
                renderer.handle_input();
                on_frame(&mut state, &mut renderer);
                renderer.draw_frame(&state);
                draw_text(
                    &format!("Turn {} - Player {}", state.turn, state.current_actor.0),
                    10.0,
                    24.0,
                    24.0,
                    BLACK,
                );
                next_frame().await;
            }
        },
    );
}
