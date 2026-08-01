//! `dcs-render`: presentation layer for `dcs-core`.
//!
//! Depends on `dcs-core`, `dcs-protocol`, and `macroquad`.

use dcs_core::map;
use dcs_core::{GameState, ScenarioConfig, TerrainType};
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

    /// One immediate-mode frame: tiles only, for now.
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
