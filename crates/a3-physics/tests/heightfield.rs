//! The terrain has two collision representations — the analytic [`TerrainShape`] that ray
//! queries use, and the parry heightfield chunks that shape casts and contacts use. They must
//! place the same surface, whatever the heights.

mod common;

use std::sync::Arc;

use a3_physics::{Interest, LayerMask, ObjectKey, QueryShape, RayQuery, TerrainShape};
use a3_wrp::TerrainBuilder;
use common::*;
use glam::{DQuat, DVec3};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn the_heightfield_chunks_and_the_analytic_terrain_are_one_surface(
        heights in proptest::collection::vec(0.0f32..8.0, 64),
        cx in 0u32..6,
        cz in 0u32..6,
        fx in 0.15f64..0.85,
        fz in 0.15f64..0.85,
    ) {
        // A 32 m square of 4 m height cells (8 x 8 height samples). Placing the probes well
        // inside a height cell and away from its diagonal keeps the sphere's contact on the
        // triangle under it, so it must stop exactly `radius / n.y` above that triangle's plane.
        prop_assume!((fx + fz - 1.0).abs() >= 0.15);
        let (x, z) = (4.0 * (f64::from(cx) + fx), 4.0 * (f64::from(cz) + fz));

        let terrain = TerrainBuilder::new(4, 8, 8.0)
            .heights(|i, j| heights[(j * 8 + i) as usize])
            .build();
        let analytic = TerrainShape::new(Arc::new(terrain.clone()));
        let (mut w, statics) = world(terrain, &[]);
        w.stream(
            &statics,
            &[Interest { center: DVec3::new(16.0, 0.0, 16.0), radius: 64.0 }],
        );

        let surface = analytic.height(x, z);
        let normal = analytic.normal(x, z);

        let ray = w
            .ray_cast(&RayQuery::new(
                DVec3::new(x, 100.0, z),
                DVec3::new(x, -10.0, z),
                LayerMask::NONE,
            ))
            .expect("the ray meets the terrain");
        prop_assert_eq!(ray.object, ObjectKey::Terrain);
        prop_assert!(
            (ray.position.y - surface).abs() < 1e-6,
            "({x}, {z}): ray at {}, surface at {surface}",
            ray.position.y
        );

        let radius = 0.05;
        let hit = w
            .shape_cast(
                QueryShape::Sphere { radius },
                DVec3::new(x, 100.0, z),
                DQuat::IDENTITY,
                DVec3::new(0.0, -110.0, 0.0),
                LayerMask::NONE,
                true,
                &[],
            )
            .expect("the sphere meets the terrain");
        prop_assert_eq!(hit.object, ObjectKey::Terrain);
        let centre = 100.0 - hit.fraction * 110.0;
        let expected = surface + radius / normal.y;
        prop_assert!(
            (centre - expected).abs() < 1e-3,
            "({x}, {z}): sphere rests at {centre}, surface at {expected}"
        );
    }
}
