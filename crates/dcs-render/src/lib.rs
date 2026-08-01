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

/// Turns a `&GameState` into pixels. Read-only (ADR-0003, Rule B) — no
/// `&mut GameState` exists here or anywhere outside `dcs-app`.
pub struct Renderer {
    pub camera: Camera2D,
    pub hex_size: f32,
}

impl Renderer {
    /// Centers and zooms the camera so every tile in `state` is visible.
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
        self.camera = Camera2D::from_display_rect(Rect::new(
            min_x - padding,
            min_y - padding,
            (max_x - min_x) + padding * 2.0,
            (max_y - min_y) + padding * 2.0,
        ));
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
            };
            renderer.fit_map(&state);

            loop {
                clear_background(config.background_color);
                renderer.draw_frame(&state);
                next_frame().await;
            }
        },
    );
}
