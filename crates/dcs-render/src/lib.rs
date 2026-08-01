//! `dcs-render`: presentation layer for `dcs-core`.
//!
//! Depends on `dcs-core`, `dcs-protocol`, and `macroquad`.

use dcs_core::map;
use dcs_core::{
    GameState, PlayerColor, PlayerId, RouteStatus, ScenarioConfig, TerrainType, TileId, Unit,
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

/// How far past `min_zoom` (the whole-map-visible level) scrolling in is
/// allowed to go, so you can't zoom into a sliver of a single tile.
const MAX_ZOOM_MULTIPLIER: f32 = 6.0;

/// Turns a `&GameState` into pixels. Read-only (ADR-0003, Rule B) — no
/// `&mut GameState` exists here or anywhere outside `dcs-app`.
///
/// `map_bounds`/`min_zoom`/`drag_anchor` are ephemeral camera-control state
/// (never part of `GameState`, never serialized — CLAUDE.md's camera/UI
/// state rule).
pub struct Renderer {
    pub camera: Camera2D,
    pub hex_size: f32,
    map_bounds: Rect,
    min_zoom: Vec2,
    drag_anchor: Option<Vec2>,
}

impl Renderer {
    /// Centers and zooms the camera so every tile in `state` is visible.
    /// Also records the resulting bounds/zoom as the pan/zoom-out limits.
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
        self.camera = Camera2D::from_display_rect(bounds);
        self.map_bounds = bounds;
        self.min_zoom = self.camera.zoom;
    }

    /// Drag-to-pan (left button) and cursor-anchored scroll-to-zoom.
    /// Call once per frame, before `draw_frame`.
    pub fn handle_input(&mut self) {
        let mouse_screen = Vec2::from(mouse_position());

        if is_mouse_button_down(MouseButton::Left) {
            let current_world = self.camera.screen_to_world(mouse_screen);
            if let Some(anchor) = self.drag_anchor {
                self.camera.target += anchor - current_world;
            }
            self.drag_anchor = Some(self.camera.screen_to_world(mouse_screen));
        } else {
            self.drag_anchor = None;
        }

        let wheel_y = mouse_wheel().1;
        if wheel_y != 0.0 {
            let world_before = self.camera.screen_to_world(mouse_screen);
            // A fixed step per wheel tick, not scaled by the raw delta
            // magnitude: real devices report wildly different deltaY scales
            // (trackpad inertial flings can report values in the hundreds),
            // so scaling directly by `wheel_y` would make zoom speed
            // unpredictable and, at the extreme, numerically degenerate.
            self.camera.zoom *= 1.0 + wheel_y.signum() * ZOOM_SPEED;
            self.clamp_zoom();
            let world_after = self.camera.screen_to_world(mouse_screen);
            self.camera.target += world_before - world_after;
        }

        self.clamp_target();
    }

    /// Keeps zoom magnitude in `[min_zoom, min_zoom * MAX_ZOOM_MULTIPLIER]`
    /// per axis, preserving `from_display_rect`'s sign convention (`zoom.y`
    /// is negative).
    fn clamp_zoom(&mut self) {
        let clamp_axis = |z: f32, min_mag: f32| {
            if min_mag >= 0.0 {
                z.clamp(min_mag, min_mag * MAX_ZOOM_MULTIPLIER)
            } else {
                z.clamp(min_mag * MAX_ZOOM_MULTIPLIER, min_mag)
            }
        };
        self.camera.zoom.x = clamp_axis(self.camera.zoom.x, self.min_zoom.x);
        self.camera.zoom.y = clamp_axis(self.camera.zoom.y, self.min_zoom.y);
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

    /// One immediate-mode frame: tiles, routes, cities, units (spec draw
    /// order, `docs/specs/presentation-rendering-ui.md` §6.2 — fog and HUD
    /// layers don't exist yet).
    pub fn draw_frame(&self, state: &GameState) {
        set_camera(&self.camera);
        for tile in &state.tiles {
            let (x, y) = tile.coord.to_pixel(self.hex_size);
            draw_hexagon(
                x,
                y,
                self.hex_size * 0.95,
                1.0,
                true, // pointy-top, matching hex::to_pixel's orientation
                BLACK,
                terrain_color(tile.terrain),
            );
        }
        draw_routes(state, self.hex_size);
        draw_cities(state, self.hex_size);
        draw_units(state, self.hex_size);
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

fn draw_routes(state: &GameState, hex_size: f32) {
    for route in &state.routes {
        let color = match route.status {
            RouteStatus::Active => owner_color(state, route.owner),
            RouteStatus::Threatened | RouteStatus::Severed => RED,
        };
        for pair in route.path.windows(2) {
            let (x1, y1) = tile_pixel(state, pair[0], hex_size);
            let (x2, y2) = tile_pixel(state, pair[1], hex_size);
            draw_dashed_line(x1, y1, x2, y2, 6.0, 4.0, color);
        }
    }
}

fn draw_cities(state: &GameState, hex_size: f32) {
    for city in &state.cities {
        let (x, y) = tile_pixel(state, city.tile, hex_size);
        let radius = (hex_size * 0.25 + city.population as f32 * 1.5).min(hex_size * 0.8);
        draw_circle(x, y, radius, owner_color(state, city.owner));
        draw_circle_lines(x, y, radius, 1.5, BLACK);
    }
}

fn draw_units(state: &GameState, hex_size: f32) {
    let city_tiles: std::collections::HashSet<TileId> =
        state.cities.iter().map(|c| c.tile).collect();
    for unit in &state.units {
        let (mut x, mut y) = tile_pixel(state, unit.tile, hex_size);
        if city_tiles.contains(&unit.tile) {
            x += hex_size * 0.35;
            y -= hex_size * 0.35;
        }
        draw_unit_marker(unit, x, y, owner_color(state, unit.owner), hex_size);
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

/// Launch the macroquad window and run the render loop.
///
/// This function blocks until the window is closed. It takes over the main
/// thread — call it only from the GUI entry point, never from async code.
pub fn run(config: RenderConfig) {
    macroquad::Window::from_config(
        Conf {
            window_title: config.window_title,
            window_width: config.window_width,
            window_height: config.window_height,
            ..Conf::default()
        },
        async move {
            // Temporary: `dcs-render` doesn't own real game state (ADR-0003
            // assigns that to `dcs-app`). This builds a throwaway demo map
            // just to exercise draw_frame until the orchestration-loop slice
            // wires in the real GameState + Command dispatch.
            let state = map::new_game(&ScenarioConfig::mvp_preset(), 42);
            let mut renderer = Renderer {
                camera: Camera2D::default(),
                hex_size: HEX_SIZE,
                map_bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
                min_zoom: Vec2::ONE,
                drag_anchor: None,
            };
            renderer.fit_map(&state);

            loop {
                clear_background(config.background_color);
                renderer.handle_input();
                renderer.draw_frame(&state);
                next_frame().await;
            }
        },
    );
}
