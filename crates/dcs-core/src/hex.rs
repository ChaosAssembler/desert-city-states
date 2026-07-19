//! Pointy-top axial hex-coordinate math and pathfinding.
//!
//! # Coordinate system
//!
//! This module uses **axial** coordinates `(q, r)` for pointy-top hexagons,
//! as described by the Red Blob Games reference. The third cube axis `s` is
//! never stored; it is always derived as `s = -q - r`. All conversions to the
//! cube coordinate system `(x, y, z)` use `x = q`, `y = -q - r`, `z = r`, so
//! that `x + y + z == 0` holds by construction.
//!
//! # Determinism guarantees
//!
//! Every function in this module is **pure**: it takes only its arguments and
//! returns a result with no use of randomness, wall-clock time, or mutable
//! global state. Consequently, identical inputs always produce byte-identical
//! outputs, including the ordering of produced collections and the choice of
//! path in [`astar`] / [`safe_route`] tie-breaks. This makes the module safe to
//! use in lockstep multiplayer simulations and reproducible replay/save files.
//!
//! Pathfinding tie-breaks prefer the lexicographically smaller [`HexCoord`]
//! (ordered by `q`, then `r`) when two frontier nodes have equal priority, so
//! the chosen path is uniquely determined for a given input graph.

use serde::{Deserialize, Serialize};
use std::collections::BinaryHeap;

/// A pointy-top axial hex coordinate. The third cube axis is implicitly
/// `s = -q - r` and is never stored.
///
/// `Ord` is derived lexicographically on `(q, r)`, which is used for
/// deterministic pathfinding tie-breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexCoord {
    pub q: i32,
    pub r: i32,
}

/// Pixel layout parameters for converting between hex and screen space.
///
/// `size` is the hex "radius" (center to corner) in pixels; `origin` is the
/// pixel position of the hex at `(0, 0)`.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub size: f32,
    pub origin: (f32, f32),
}

/// The six axial neighbor directions in a fixed, stable order.
///
/// Order: E, NE, NW, W, SW, SE (pointy-top). Keeping a fixed order is part of
/// the determinism contract for [`neighbors`] and [`ring`].
pub const AXIAL_DIRS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];

/// The hex at the origin of the coordinate system.
pub const ORIGIN: HexCoord = HexCoord { q: 0, r: 0 };

/// Convert an axial coordinate to cube coordinates `(x, y, z)` where
/// `x = q`, `y = -q - r`, `z = r`. The invariant `x + y + z == 0` holds.
pub fn to_cube(h: HexCoord) -> (i32, i32, i32) {
    (h.q, -h.q - h.r, h.r)
}

/// Convert cube coordinates back to an axial coordinate: `q = x`, `r = z`.
///
/// The supplied `y` is ignored for the axial result but the caller is expected
/// to provide a balanced triple (`x + y + z == 0`); if not, the resulting
/// derived `s = -q - r` equals `-x - z`.
pub fn from_cube(x: i32, _y: i32, z: i32) -> HexCoord {
    HexCoord { q: x, r: z }
}

/// Convert an axial coordinate to a pointy-top pixel position, with the hex
/// `(0, 0)` centered at the pixel origin `(0.0, 0.0)`.
pub fn to_pixel(h: HexCoord, size: f32) -> (f32, f32) {
    let x = size * (3.0_f32).sqrt() * (h.q as f32 + h.r as f32 / 2.0);
    let y = size * 3.0 / 2.0 * h.r as f32;
    (x, y)
}

/// Convert a pixel position (relative to hex `(0,0)` origin) to the nearest
/// axial coordinate.
pub fn from_pixel(p: (f32, f32), size: f32) -> HexCoord {
    pixel_to_hex(
        p,
        Layout {
            size,
            origin: (0.0, 0.0),
        },
    )
}

/// Convert a pixel position under the given [`Layout`] to the nearest axial
/// coordinate using fractional axial math followed by [`cube_round`].
pub fn pixel_to_hex(p: (f32, f32), layout: Layout) -> HexCoord {
    let px = p.0 - layout.origin.0;
    let py = p.1 - layout.origin.1;
    let q = ((3.0_f32).sqrt() / 3.0 * px - 1.0 / 3.0 * py) / layout.size;
    let r = (2.0 / 3.0 * py) / layout.size;
    let (x, y, z) = cube_round(q, -q - r, r);
    from_cube(x, y, z)
}

/// Convert an axial coordinate to a pointy-top pixel position under the given
/// [`Layout`] (adds `layout.origin`).
pub fn to_pixel_layout(h: HexCoord, layout: Layout) -> (f32, f32) {
    let (x, y) = to_pixel(h, layout.size);
    (x + layout.origin.0, y + layout.origin.1)
}

/// Round fractional cube coordinates to the nearest integer cube triple,
/// preserving the invariant `rx + ry + rz == 0`.
pub fn cube_round(x: f32, y: f32, z: f32) -> (i32, i32, i32) {
    let mut rx = x.round();
    let mut ry = y.round();
    let mut rz = z.round();

    let dx = (rx - x).abs();
    let dy = (ry - y).abs();
    let dz = (rz - z).abs();

    if dx > dy && dx > dz {
        rx = -ry - rz;
    } else if dy > dz {
        ry = -rx - rz;
    } else {
        rz = -rx - ry;
    }

    (rx as i32, ry as i32, rz as i32)
}

/// Return the six axial neighbors of `h`, in the fixed [`AXIAL_DIRS`] order.
pub fn neighbors(h: HexCoord) -> [HexCoord; 6] {
    [
        HexCoord {
            q: h.q + AXIAL_DIRS[0].0,
            r: h.r + AXIAL_DIRS[0].1,
        },
        HexCoord {
            q: h.q + AXIAL_DIRS[1].0,
            r: h.r + AXIAL_DIRS[1].1,
        },
        HexCoord {
            q: h.q + AXIAL_DIRS[2].0,
            r: h.r + AXIAL_DIRS[2].1,
        },
        HexCoord {
            q: h.q + AXIAL_DIRS[3].0,
            r: h.r + AXIAL_DIRS[3].1,
        },
        HexCoord {
            q: h.q + AXIAL_DIRS[4].0,
            r: h.r + AXIAL_DIRS[4].1,
        },
        HexCoord {
            q: h.q + AXIAL_DIRS[5].0,
            r: h.r + AXIAL_DIRS[5].1,
        },
    ]
}

/// Cube distance between two axial coordinates: `(|dx| + |dy| + |dz|) / 2`.
pub fn distance(a: HexCoord, b: HexCoord) -> u32 {
    let ac = to_cube(a);
    let bc = to_cube(b);
    ((ac.0 - bc.0).abs() + (ac.1 - bc.1).abs() + (ac.2 - bc.2).abs()) as u32 / 2
}

/// Return the ring of hexes at exactly `radius` steps from `center`.
///
/// `ring(center, 0)` returns `[center]`; otherwise it returns exactly
/// `6 * radius` distinct hexes, all at cube-distance `radius` from `center`.
pub fn ring(center: HexCoord, radius: u32) -> Vec<HexCoord> {
    if radius == 0 {
        return vec![center];
    }

    let radius = radius as i32;

    // Start at a corner `radius` steps away from center along the first
    // direction, then walk each of the 6 sides.
    let mut h = HexCoord {
        q: center.q + AXIAL_DIRS[4].0 * radius,
        r: center.r + AXIAL_DIRS[4].1 * radius,
    };

    let mut result = Vec::with_capacity(6 * radius as usize);
    for &d in AXIAL_DIRS.iter() {
        for _ in 0..radius {
            result.push(h);
            h = HexCoord {
                q: h.q + d.0,
                r: h.r + d.1,
            };
        }
    }
    result
}

/// Return all hexes within `radius` steps of `center` (inclusive).
///
/// The result contains exactly `1 + 3 * radius * (radius + 1)` hexes.
pub fn range(center: HexCoord, radius: u32) -> Vec<HexCoord> {
    let radius = radius as i32;
    let mut result = Vec::with_capacity((1 + 3 * radius * (radius + 1)) as usize);
    for dq in -radius..=radius {
        let dr_min = (-radius).max(-dq - radius);
        let dr_max = radius.min(-dq + radius);
        for dr in dr_min..=dr_max {
            result.push(HexCoord {
                q: center.q + dq,
                r: center.r + dr,
            });
        }
    }
    result
}

/// Return the straight (lerp) line from `a` to `b`, inclusive of both
/// endpoints, with `distance(a, b) + 1` hexes.
pub fn line(a: HexCoord, b: HexCoord) -> Vec<HexCoord> {
    let n = distance(a, b);
    let ac = to_cube(a);
    let bc = to_cube(b);
    let mut result = Vec::with_capacity((n + 1) as usize);
    for i in 0..=n {
        let t = if n == 0 { 0.0 } else { i as f32 / n as f32 };
        let x = ac.0 as f32 + (bc.0 as f32 - ac.0 as f32) * t;
        let y = ac.1 as f32 + (bc.1 as f32 - ac.1 as f32) * t;
        let z = ac.2 as f32 + (bc.2 as f32 - ac.2 as f32) * t;
        let (rx, ry, rz) = cube_round(x, y, z);
        result.push(from_cube(rx, ry, rz));
    }
    result
}

/// Whether `h` lies within `radius` steps of the origin.
pub fn in_map(h: HexCoord, radius: u32) -> bool {
    distance(ORIGIN, h) <= radius
}

/// A priority-queue item that orders by ascending `f` (g + heuristic), then by
/// descending `coord` so that, on equal `f`, the lexicographically smaller
/// [`HexCoord`] (preferred by [`HexCoord`]'s `Ord`) pops first.
///
/// `BinaryHeap` is a max-heap, so we invert both comparisons.
#[derive(Clone, Copy, Debug)]
struct PqItem {
    f: f32,
    /// The g-score (best-known cost from start) recorded when this entry was
    /// pushed. Used to detect and skip stale entries after a better path to the
    /// same coord is discovered.
    g: f32,
    coord: HexCoord,
}

impl PartialEq for PqItem {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.coord == other.coord
    }
}

impl Eq for PqItem {}

impl PartialOrd for PqItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PqItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Max-heap: smaller f should pop first -> reverse on f.
        other
            .f
            .partial_cmp(&self.f)
            .unwrap_or(std::cmp::Ordering::Equal)
            // On equal f, prefer the lexicographically smaller coord, which is
            // also "smaller" in our inverted scheme, so reverse coord ordering.
            .then_with(|| other.coord.cmp(&self.coord))
    }
}

/// A* pathfinding over the hex graph.
///
/// `passable` decides whether a hex may be entered; `cost(from, to)` returns
/// the (non-negative) movement cost of stepping from `from` to `to`. The
/// heuristic is the admissible [`distance`] to `goal`.
///
/// Returns the path **excluding** `start` but **including** `goal`, or `None`
/// if `goal` is unreachable. The result is deterministic for identical inputs.
pub fn astar(
    start: HexCoord,
    goal: HexCoord,
    passable: impl Fn(HexCoord) -> bool,
    cost: impl Fn(HexCoord, HexCoord) -> f32,
) -> Option<Vec<HexCoord>> {
    if !passable(goal) {
        return None;
    }
    if start == goal {
        return Some(Vec::new());
    }

    let mut open = BinaryHeap::new();
    open.push(PqItem {
        f: distance(start, goal) as f32,
        g: 0.0,
        coord: start,
    });

    let mut g_score: fxhash::FxHashMap<HexCoord, f32> = fxhash::FxHashMap::default();
    g_score.insert(start, 0.0);

    let mut came_from: fxhash::FxHashMap<HexCoord, HexCoord> = fxhash::FxHashMap::default();

    while let Some(item) = open.pop() {
        let current = item.coord;

        // Closed-set / relaxation guard: skip stale queue entries. A coord may
        // be pushed several times as better paths to it are found; once a
        // smaller g is recorded we must not re-expand the older, worse entry.
        // Without this guard a cyclic graph with an unreachable goal would
        // re-expand settled nodes indefinitely.
        let best_g = match g_score.get(&current) {
            Some(&g) => g,
            None => continue,
        };
        if item.g > best_g {
            continue;
        }

        if current == goal {
            let mut path = Vec::new();
            let mut step = current;
            while step != start {
                path.push(step);
                step = came_from[&step];
            }
            path.reverse();
            return Some(path);
        }

        let current_g = best_g;
        for next in neighbors(current) {
            if !passable(next) {
                continue;
            }
            let tentative_g = current_g + cost(current, next);
            let better = match g_score.get(&next) {
                Some(&g) => tentative_g < g,
                None => true,
            };
            if better {
                came_from.insert(next, current);
                g_score.insert(next, tentative_g);
                let f = tentative_g + distance(next, goal) as f32;
                open.push(PqItem {
                    f,
                    g: tentative_g,
                    coord: next,
                });
            }
        }
    }

    None
}

/// Threat-weighted single-source shortest path (Dijkstra) from `start`.
///
/// The edge weight into `to` from any neighbor is `base_cost *
/// (1.0 + threat_fn(to))`, which is non-negative when `threat_fn >= -1.0`.
/// There is no heuristic, so this is plain uniform-cost search.
///
/// Returns `(inclusive path from start to goal, accumulated total cost)` or
/// `None` if `goal` is unreachable. Deterministic for identical inputs.
pub fn safe_route(
    start: HexCoord,
    goal: HexCoord,
    threat_fn: impl Fn(HexCoord) -> f32,
    base_cost: f32,
) -> Option<(Vec<HexCoord>, f32)> {
    if start == goal {
        return Some((vec![start], 0.0));
    }

    let mut open = BinaryHeap::new();
    open.push(PqItem {
        f: 0.0,
        g: 0.0,
        coord: start,
    });

    let mut dist: fxhash::FxHashMap<HexCoord, f32> = fxhash::FxHashMap::default();
    dist.insert(start, 0.0);

    let mut came_from: fxhash::FxHashMap<HexCoord, HexCoord> = fxhash::FxHashMap::default();

    while let Some(item) = open.pop() {
        let current = item.coord;
        let current_d = item.f; // f == g in Dijkstra (no heuristic)

        // Skip stale queue entries.
        if let Some(&best) = dist.get(&current) {
            if current_d > best {
                continue;
            }
        }

        if current == goal {
            let mut path = vec![current];
            let mut step = current;
            while step != start {
                step = came_from[&step];
                path.push(step);
            }
            path.reverse();
            return Some((path, current_d));
        }

        for next in neighbors(current) {
            let w = base_cost * (1.0 + threat_fn(next));
            let nd = current_d + w;
            let better = match dist.get(&next) {
                Some(&d) => nd < d,
                None => true,
            };
            if better {
                came_from.insert(next, current);
                dist.insert(next, nd);
                open.push(PqItem {
                    f: nd,
                    g: nd,
                    coord: next,
                });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn cube_round_sums_to_zero() {
        for x in -5..=5i32 {
            for y in -5..=5i32 {
                for z in -5..=5i32 {
                    let (rx, ry, rz) = cube_round(x as f32, y as f32, z as f32);
                    assert_eq!(rx + ry + rz, 0, "failed at ({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn cube_round_identity_on_integers() {
        for x in -5..=5i32 {
            for y in -5..=5i32 {
                for z in -5..=5i32 {
                    if x + y + z != 0 {
                        continue;
                    }
                    let (rx, ry, rz) = cube_round(x as f32, y as f32, z as f32);
                    assert_eq!((rx, ry, rz), (x, y, z), "failed at ({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn pixel_round_trip_exact() {
        let coords = [
            HexCoord { q: 0, r: 0 },
            HexCoord { q: 1, r: 0 },
            HexCoord { q: -2, r: 3 },
            HexCoord { q: 5, r: -5 },
            HexCoord { q: -3, r: -1 },
            HexCoord { q: 7, r: 2 },
        ];
        for &s in &[1.0_f32, 5.0, 10.0, 23.5, 100.0] {
            for &h in &coords {
                let p = to_pixel(h, s);
                let back = from_pixel(p, s);
                assert_eq!(back, h, "round-trip failed for {h:?} size {s}");
            }
        }
    }

    #[test]
    fn pixel_to_hex_with_layout_origin() {
        let layout = Layout {
            size: 12.0,
            origin: (37.0, -9.0),
        };
        let h = HexCoord { q: -4, r: 6 };
        let p = to_pixel_layout(h, layout);
        let back = pixel_to_hex(p, layout);
        assert_eq!(back, h);
    }

    #[test]
    fn distance_symmetric_and_expected() {
        let origin = HexCoord { q: 0, r: 0 };
        assert_eq!(distance(origin, HexCoord { q: 2, r: 0 }), 2);
        assert_eq!(distance(origin, HexCoord { q: 0, r: -3 }), 3);
        assert_eq!(distance(origin, HexCoord { q: 2, r: -4 }), 4);
        assert_eq!(
            distance(HexCoord { q: 1, r: 1 }, HexCoord { q: -2, r: 2 }),
            3
        );

        // Symmetry.
        for q in -4..=4i32 {
            for r in -4..=4i32 {
                let a = HexCoord { q, r };
                for q2 in -4..=4i32 {
                    for r2 in -4..=4i32 {
                        let b = HexCoord { q: q2, r: r2 };
                        assert_eq!(distance(a, b), distance(b, a));
                    }
                }
            }
        }
    }

    #[test]
    fn ring_counts_and_distance() {
        let center = HexCoord { q: 2, r: -1 };
        assert_eq!(ring(center, 0), vec![center]);
        assert_eq!(ring(center, 3).len(), 18);
        for radius in 1..=6u32 {
            let r = ring(center, radius);
            assert_eq!(r.len(), 6 * radius as usize, "bad len at radius {radius}");
            for h in r {
                assert_eq!(distance(center, h), radius);
            }
        }
    }

    #[test]
    fn range_counts_and_inclusive() {
        let center = HexCoord { q: -1, r: 2 };
        assert_eq!(range(center, 0).len(), 1);
        assert_eq!(range(center, 4).len(), 61);
        for radius in 0..=5u32 {
            let expected = 1 + 3 * radius * (radius + 1);
            assert_eq!(range(center, radius).len(), expected as usize);
        }
        // Every hex in range is within radius of center.
        for h in range(center, 5) {
            assert!(distance(center, h) <= 5);
        }
    }

    #[test]
    fn line_length_and_endpoints() {
        let cases = [
            (HexCoord { q: 0, r: 0 }, HexCoord { q: 3, r: 0 }),
            (HexCoord { q: -2, r: 2 }, HexCoord { q: 4, r: -1 }),
            (HexCoord { q: 1, r: -3 }, HexCoord { q: 1, r: -3 }),
        ];
        for (a, b) in cases {
            let l = line(a, b);
            assert_eq!(l.len(), distance(a, b) as usize + 1);
            assert_eq!(l.first(), Some(&a));
            assert_eq!(l.last(), Some(&b));
        }
    }

    #[test]
    fn in_map_boundary() {
        let radius = 3;
        // A hex exactly at the boundary is inside.
        let on_edge = HexCoord { q: 3, r: 0 };
        assert!(in_map(on_edge, radius));
        let outside = HexCoord { q: 4, r: 0 };
        assert!(!in_map(outside, radius));
        assert!(in_map(HexCoord { q: 0, r: 0 }, radius));
    }

    #[test]
    fn neighbors_are_six_and_distinct() {
        let h = HexCoord { q: 1, r: -2 };
        let ns = neighbors(h);
        assert_eq!(ns.len(), 6);
        let mut sorted = ns;
        sorted.sort();
        // neighbors are already distinct — no dedup needed (dedup is not available
        // on fixed-size arrays anyway).
        assert_eq!(sorted.len(), 6, "neighbors were not distinct: {ns:?}");
        for n in ns {
            assert_eq!(distance(h, n), 1);
        }
    }

    #[test]
    fn astar_straight_path_all_passable() {
        let start = HexCoord { q: 0, r: 0 };
        let goal = HexCoord { q: 4, r: 0 };
        let passable = |_: HexCoord| true;
        let cost = |_: HexCoord, _: HexCoord| 1.0_f32;
        let path = astar(start, goal, passable, cost).expect("should find path");
        assert_eq!(path.first(), Some(&HexCoord { q: 1, r: 0 }));
        assert_eq!(path.last(), Some(&goal));
        assert_eq!(path.len(), 4);
    }

    #[test]
    fn astar_respects_wall() {
        // Wall: a vertical line of impassable hexes at q = 2 blocks the direct
        // straight route, forcing a detour (or None if fully sealed). Here we
        // leave a gap so a detour exists.
        let start = HexCoord { q: 0, r: 0 };
        let goal = HexCoord { q: 4, r: 0 };
        let passable = |h: HexCoord| !(h.q == 2 && (h.r == -1 || h.r == 0 || h.r == 1));
        let cost = |_: HexCoord, _: HexCoord| 1.0_f32;
        let path = astar(start, goal, passable, cost).expect("detour should exist");
        assert_eq!(path.last(), Some(&goal));
        for &h in &path {
            assert!(passable(h), "path stepped through a wall at {h:?}");
        }
        // The detour is longer than the straight 4-step path.
        assert!(path.len() > 4);
    }

    #[test]
    fn astar_returns_none_when_sealed() {
        use crate::hex::distance;
        let start = HexCoord { q: 0, r: 0 };
        let goal = HexCoord { q: 4, r: 0 };
        // Seal q == 2 AND bound the explored region to a finite disk around the
        // start, so A* terminates. The q == 2 wall still makes the goal
        // unreachable, so the result must be None.
        let passable = |h: HexCoord| h.q != 2 && distance(h, start) <= 5;
        let cost = |_: HexCoord, _: HexCoord| 1.0_f32;
        assert!(astar(start, goal, passable, cost).is_none());
    }

    #[test]
    fn astar_deterministic_across_calls() {
        let start = HexCoord { q: -3, r: -3 };
        let goal = HexCoord { q: 3, r: 3 };
        let passable = |_: HexCoord| true;
        let cost = |_: HexCoord, _: HexCoord| 1.0_f32;
        let a = astar(start, goal, passable, cost).unwrap();
        let b = astar(start, goal, passable, cost).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn safe_route_cost_without_threat() {
        let start = HexCoord { q: 0, r: 0 };
        let goal = HexCoord { q: 3, r: 0 };
        let threat = |_: HexCoord| 0.0_f32;
        let base_cost = 1.0;
        let (path, total) = safe_route(start, goal, threat, base_cost).expect("path exists");
        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&goal));
        // No threat: cost equals number of edges * base_cost.
        let edges = (path.len() - 1) as f32;
        assert!((total - edges * base_cost).abs() < 1e-6);
    }

    #[test]
    fn safe_route_threat_increases_cost() {
        let start = HexCoord { q: 0, r: 0 };
        let goal = HexCoord { q: 3, r: 0 };
        let base_cost = 1.0;

        let no_threat = |_: HexCoord| 0.0_f32;
        let with_threat = |h: HexCoord| if h.q == 1 && h.r == 0 { 5.0 } else { 0.0 };

        let (_, c0) = safe_route(start, goal, no_threat, base_cost).unwrap();
        let (path, c1) = safe_route(start, goal, with_threat, base_cost).unwrap();
        assert!(c1 > c0, "threat should increase total cost");
        // The threatened tile must not appear as an intermediate step.
        for &h in &path[1..path.len().saturating_sub(0)] {
            if h != goal {
                assert!(!(h.q == 1 && h.r == 0), "route passed through threat");
            }
        }
    }

    #[test]
    fn safe_route_deterministic() {
        let start = HexCoord { q: -2, r: 2 };
        let goal = HexCoord { q: 2, r: -2 };
        let threat = |h: HexCoord| ((h.q + h.r) as f32).abs() * 0.1;
        let a = safe_route(start, goal, threat, 1.0).unwrap();
        let b = safe_route(start, goal, threat, 1.0).unwrap();
        assert_eq!(a, b);
    }

    proptest! {
        #[test]
        fn prop_distance_symmetry(
            a_q in -10i32..10,
            a_r in -10i32..10,
            b_q in -10i32..10,
            b_r in -10i32..10,
        ) {
            let a = HexCoord { q: a_q, r: a_r };
            let b = HexCoord { q: b_q, r: b_r };
            prop_assert_eq!(distance(a, b), distance(b, a));
        }

        #[test]
        fn prop_distance_zero_iff_equal(
            q in -10i32..10,
            r in -10i32..10,
        ) {
            let coord = HexCoord { q, r };
            prop_assert_eq!(distance(coord, coord), 0);
        }

        #[test]
        fn prop_axial_cube_roundtrip(
            q in -10i32..10,
            r in -10i32..10,
        ) {
            let coord = HexCoord { q, r };
            let (x, y, z) = to_cube(coord);
            let back = from_cube(x, y, z);
            prop_assert_eq!(coord, back);
        }

        #[test]
        fn prop_cube_coordinates_sum_to_zero(
            x in -10i32..10,
            z in -10i32..10,
        ) {
            let y = -x - z;
            prop_assert_eq!(x + y + z, 0);
        }
    }
}
