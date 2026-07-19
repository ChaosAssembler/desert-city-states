// This file should be moved to crates/dcs-core/benches/game_benchmarks.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use dcs_core::hex::{self, HexCoord};
use dcs_core::model::Tile;
use dcs_core::scenario::mvp_preset;
use dcs_core::{GameState, TerrainType, TileId};

fn setup_game_state() -> GameState {
    let cfg = mvp_preset();
    let mut state = GameState::new(cfg, 1);
    let radius = state.scenario.map_radius as u32;
    for (i, coord) in hex::ORIGIN.range(radius).into_iter().enumerate() {
        let id = TileId(i as u32);
        state.tiles.push(Tile {
            id,
            coord,
            terrain: TerrainType::Dunes,
            is_relic_site: false,
            owner: None,
            improvement: None,
        });
        state.tile_index.insert(coord, id);
    }
    state
}

fn bench_hex_distance(c: &mut Criterion) {
    let start = HexCoord { q: 0, r: 0 };
    let goal = HexCoord { q: 5, r: -3 };
    c.bench_function("hex_distance_5_3", |b| {
        b.iter(|| black_box(start).distance(black_box(goal)))
    });
}

fn bench_hex_neighbors(c: &mut Criterion) {
    let coord = HexCoord { q: 0, r: 0 };
    c.bench_function("hex_neighbors", |b| {
        b.iter(|| black_box(coord).neighbors())
    });
}

fn bench_hex_range(c: &mut Criterion) {
    c.bench_function("hex_range_radius_5", |b| {
        b.iter(|| black_box(HexCoord { q: 0, r: 0 }).range(black_box(5)))
    });
}

fn bench_game_state_creation(c: &mut Criterion) {
    c.bench_function("game_state_creation", |b| {
        b.iter(setup_game_state)
    });
}

criterion_group!(
    benches,
    bench_hex_distance,
    bench_hex_neighbors,
    bench_hex_range,
    bench_game_state_creation,
);
criterion_main!(benches);
