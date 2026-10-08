//! The terrain surface as a collision shape: heights, normals and ray casts that follow the
//! engine's triangle split.

use std::sync::Arc;

use a3_physics::TerrainShape;
use a3_wrp::TerrainBuilder;
use glam::DVec3;

/// 8 x 8 height samples, 4 m apart (32 m square).
fn terrain(f: impl Fn(u32, u32) -> f32) -> TerrainShape {
    TerrainShape::new(Arc::new(TerrainBuilder::new(4, 8, 8.0).heights(f).build()))
}

#[test]
fn heights_match_the_wrp_interpolation_on_both_triangles_of_a_cell() {
    // Only sample (2, 2) is raised: the cell (1, 1)..(2, 2) has its upper-right corner at 8 m.
    let t = terrain(|i, j| if (i, j) == (2, 2) { 8.0 } else { 0.0 });
    let wrp = t.terrain().clone();
    for &(x, z) in &[(5.0, 5.0), (7.0, 7.0), (6.5, 5.5), (4.2, 7.9), (7.9, 7.9)] {
        let expected = f64::from(wrp.surface_height(x as f32, z as f32));
        assert!((t.height(x, z) - expected).abs() < 1e-5, "({x}, {z})");
    }
    // Lower-left triangle of the cell (fx + fz <= 1) does not see the raised corner.
    assert_eq!(t.height(4.5, 4.5), 0.0);
    // Upper-right triangle does: at the cell centre's far side.
    assert!((t.height(7.0, 7.0) - 4.0).abs() < 1e-9);
}

#[test]
fn a_ray_straight_down_hits_the_surface_height() {
    let t = terrain(|i, j| (i * 2 + j) as f32);
    for &(x, z) in &[(1.0, 1.0), (10.3, 22.7), (17.9, 3.1), (27.0, 27.0)] {
        let hit = t
            .cast_ray(DVec3::new(x, 100.0, z), DVec3::NEG_Y, 200.0)
            .expect("hit");
        assert!((hit.position.y - t.height(x, z)).abs() < 1e-6, "({x}, {z})");
        assert!((hit.distance - (100.0 - t.height(x, z))).abs() < 1e-6);
        assert!(hit.normal.y > 0.0, "normal points up: {:?}", hit.normal);
    }
}

#[test]
fn the_normal_of_a_slope_tilts_downhill() {
    // Rises 1 m per 4 m eastwards.
    let t = terrain(|i, _| i as f32);
    let n = t.normal(10.0, 10.0);
    let expected = DVec3::new(-1.0, 4.0, 0.0).normalize();
    assert!((n - expected).length() < 1e-9, "{n:?}");
}

#[test]
fn a_horizontal_ray_hits_a_hill_side_and_misses_over_flat_ground() {
    // A ridge along x = 16 m, 12 m high.
    let t = terrain(|i, _| if i == 4 { 12.0 } else { 0.0 });
    let hit = t
        .cast_ray(DVec3::new(1.0, 6.0, 10.0), DVec3::X, 100.0)
        .expect("hits the ridge");
    // The slope rises 12 m over 4 m from x = 12; 6 m high at x = 14.
    assert!((hit.position.x - 14.0).abs() < 1e-6, "{hit:?}");
    assert!(hit.normal.x < 0.0);
    assert!(
        t.cast_ray(DVec3::new(1.0, 13.0, 10.0), DVec3::X, 100.0)
            .is_none()
    );
    // Too short to reach it.
    assert!(
        t.cast_ray(DVec3::new(1.0, 6.0, 10.0), DVec3::X, 10.0)
            .is_none()
    );
}

#[test]
fn a_ray_from_below_the_surface_reports_where_it_comes_out() {
    let t = terrain(|_, _| 5.0);
    let hit = t
        .cast_ray(DVec3::new(9.0, 0.0, 9.0), DVec3::Y, 50.0)
        .expect("crosses");
    assert!((hit.distance - 5.0).abs() < 1e-9);
}

#[test]
fn a_long_diagonal_ray_crosses_many_cells_before_hitting() {
    // A bowl: 0 in the middle, rising to the edges.
    let t = terrain(|i, j| {
        let (x, z) = (i as f32 - 3.5, j as f32 - 3.5);
        x * x + z * z
    });
    let origin = DVec3::new(14.0, 1.0, 14.0);
    let dir = DVec3::new(1.0, 0.2, 0.7).normalize();
    let hit = t
        .cast_ray(origin, dir, 100.0)
        .expect("hits the bowl's side");
    assert!((hit.position.y - t.height(hit.position.x, hit.position.z)).abs() < 1e-6);
    // Nothing between the origin and the hit is under the surface.
    for k in 1..100 {
        let p = origin + dir * (hit.distance * f64::from(k) / 100.0);
        assert!(p.y >= t.height(p.x, p.z) - 1e-6, "{p:?}");
    }
}
