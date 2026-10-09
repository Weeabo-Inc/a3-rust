//! Turning a cell path into positions: the string pull.
//!
//! An 8-neighbour A* produces a staircase of cell centres; the engine's paths are positions,
//! not cells (`lastOperPosX`/`lastOperPosZ`, `docs/re/navigation.md` §2). [`string_pull`]
//! greedily takes the furthest waypoint a straight line can reach, which removes the
//! staircase. Whether a line is clear is the caller's business ([`Navigator`](crate::Navigator)
//! walks it at heightmap resolution and rejects a shortcut that leaves walkable ground, climbs
//! too steeply or crosses deep water).

use glam::Vec3;

/// The greedy string pull over `points` (the first is kept): each output point is the furthest
/// later point that `clear` accepts from the current one.
///
/// `clear(a, b)` must accept `b == a + one step` — the points come from a path where
/// consecutive cells are connected — and must be symmetric about its arguments.
pub fn string_pull(points: &[Vec3], clear: impl Fn(Vec3, Vec3) -> bool) -> Vec<Vec3> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len());
    out.push(points[0]);
    let mut anchor = 0usize;
    while anchor + 1 < points.len() {
        let mut reach = anchor + 1;
        while reach + 1 < points.len() && clear(points[anchor], points[reach + 1]) {
            reach += 1;
        }
        out.push(points[reach]);
        anchor = reach;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, z: f32) -> Vec3 {
        Vec3::new(x, 0.0, z)
    }

    /// A staircase along the diagonal: nothing blocks, so the pull should collapse it to its
    /// two ends.
    #[test]
    fn an_open_staircase_collapses_to_its_ends() {
        let points = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(20.0, 10.0)];
        let pulled = string_pull(&points, |_, _| true);
        assert_eq!(pulled, vec![p(0.0, 0.0), p(20.0, 10.0)]);
    }

    /// A barrier the last leg alone crosses: the pull goes as far as it can (the point in
    /// front of the barrier) and carries on from there.
    #[test]
    fn a_barrier_stops_the_pull_at_the_last_clear_point() {
        // Nothing may reach past x = 20 unless it starts there or beyond.
        let clear = |a: Vec3, b: Vec3| b.x <= 20.0 || a.x >= 20.0;
        let points = vec![p(0.0, 0.0), p(10.0, 0.0), p(20.0, 0.0), p(30.0, 0.0)];
        let pulled = string_pull(&points, clear);
        assert_eq!(pulled, vec![p(0.0, 0.0), p(20.0, 0.0), p(30.0, 0.0)]);
    }

    #[test]
    fn short_paths_come_back_unchanged() {
        assert_eq!(string_pull(&[], |_, _| false), Vec::<Vec3>::new());
        let one = vec![p(1.0, 2.0)];
        assert_eq!(string_pull(&one, |_, _| false), one);
        let two = vec![p(1.0, 2.0), p(3.0, 4.0)];
        assert_eq!(string_pull(&two, |_, _| false), two);
    }

    #[test]
    fn the_ends_are_always_kept() {
        // Nothing is clear beyond the first step: every point must survive.
        let points = vec![p(0.0, 0.0), p(10.0, 0.0), p(20.0, 0.0), p(30.0, 0.0)];
        let pulled = string_pull(&points, |a, b| (b - a).length() <= 10.5);
        assert_eq!(pulled, points);
    }

    #[test]
    fn the_pull_is_monotone_along_the_path() {
        let points: Vec<Vec3> = (0..20)
            .map(|i| p(i as f32 * 10.0, (i % 2) as f32 * 10.0))
            .collect();
        let pulled = string_pull(&points, |_, _| true);
        assert_eq!(pulled.first(), points.first());
        assert_eq!(pulled.last(), points.last());
        assert!(pulled.len() < points.len());
    }
}
