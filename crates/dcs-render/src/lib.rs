//! `dcs-render`: presentation layer for `dcs-core`.
//!
//! Depends on `dcs-core`, `dcs-protocol`, and `macroquad`.

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
            loop {
                clear_background(config.background_color);
                // Future: draw_frame(state, view) goes here
                next_frame().await;
            }
        },
    );
}
