//! Road mesh generation and the road feature.

mod common;

use a3_render::roads::{
    RoadFeature, RoadMaterial, RoadMeshSettings, RoadSegment, build_road_meshes,
};
use a3_render::{Camera, DrawList, MeshData, MeshDraw, Renderer, TextureData};
use glam::{DVec3, Vec2, Vec3};

/// A straight piece from `a` to `b` (controls at thirds).
fn straight(road: u32, a: Vec2, b: Vec2, open_start: bool, open_end: bool) -> RoadSegment {
    RoadSegment {
        road,
        p0: a,
        c1: a + (b - a) / 3.0,
        c2: a + (b - a) * 2.0 / 3.0,
        p1: b,
        width: 10.0,
        material: 0,
        open_start,
        open_end,
    }
}

#[test]
fn a_straight_road_is_a_flat_strip_at_the_road_width() {
    let seg = straight(
        0,
        Vec2::new(1000.0, 1000.0),
        Vec2::new(1000.0, 1100.0),
        false,
        false,
    );
    let chunks = build_road_meshes(&[seg], |_, _| 5.0, &RoadMeshSettings::default());
    assert_eq!(chunks.len(), 1);
    let chunk = &chunks[0];
    // The midpoint (1000, 1050) lies in chunk (1, 2) of 512 m.
    assert_eq!(chunk.origin, DVec3::new(512.0, 0.0, 1024.0));
    assert_eq!(chunk.batches.len(), 1);
    let batch = &chunk.batches[0];
    assert!(!batch.end);
    for v in &batch.vertices {
        let world = chunk.origin.as_vec3() + Vec3::from_array(v.position);
        assert!(
            (world.y - 5.04).abs() < 1e-4,
            "lifted 4 cm above the terrain"
        );
        assert!(
            (world.x - 1000.0).abs() <= 5.0 + 1e-3,
            "within half the width"
        );
        // u = 0 is the left edge (west when driving north).
        let expected_x = 995.0 + v.uv[0] * 10.0;
        assert!((world.x - expected_x).abs() < 1e-3, "{world} u {}", v.uv[0]);
        assert!(
            (v.uv[1] - (world.z - 1000.0) / 10.0).abs() < 1e-3,
            "v in road widths"
        );
        assert!((Vec3::from_array(v.normal) - Vec3::Y).length() < 1e-5);
    }
    let max = batch.vertices.len() as u32;
    assert!(batch.indices.iter().all(|&i| i < max));
    assert_eq!(batch.indices.len() % 3, 0);
}

#[test]
fn open_ends_use_the_end_texture_for_one_width() {
    let seg = straight(0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), true, true);
    let chunks = build_road_meshes(&[seg], |_, _| 0.0, &RoadMeshSettings::default());
    let ends: Vec<_> = chunks[0].batches.iter().filter(|b| b.end).collect();
    assert_eq!(ends.len(), 1);
    // Both ends: v runs from 0 at the road end to 1 one width (10 m) inside.
    for v in &ends[0].vertices {
        let x = v.position[0];
        let inside = if x < 50.0 { x } else { 100.0 - x };
        assert!(inside <= 10.0 + 1e-3);
        assert!((v.uv[1] - inside / 10.0).abs() < 1e-3);
    }
    let straight = chunks[0].batches.iter().find(|b| !b.end).unwrap();
    assert!(
        straight
            .vertices
            .iter()
            .all(|v| (10.0 - 1e-3..=90.0 + 1e-3).contains(&v.position[0]))
    );
}

#[test]
fn texture_continues_across_pieces_of_one_road_and_drapes_on_slopes() {
    let a = straight(7, Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0), false, false);
    let b = straight(7, Vec2::new(30.0, 0.0), Vec2::new(60.0, 0.0), false, false);
    let slope = |x: f32, z: f32| 0.1 * x + 0.05 * z;
    let chunks = build_road_meshes(&[a, b], slope, &RoadMeshSettings::default());
    let batch = &chunks[0].batches[0];
    let at_60 = batch
        .vertices
        .iter()
        .find(|v| (v.position[0] - 60.0).abs() < 1e-3)
        .unwrap();
    assert!(
        (at_60.uv[1] - 6.0).abs() < 1e-3,
        "v carried over: {}",
        at_60.uv[1]
    );
    for v in &batch.vertices {
        let [x, y, z] = v.position;
        assert!((y - (slope(x, z) + 0.04)).abs() < 1e-3);
    }
}

#[test]
fn road_is_drawn_over_the_ground_below_the_camera() {
    let Some(gpu) = common::gpu() else { return };
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let centre = Vec2::new(20_000.0, 20_000.0);
    let seg = straight(
        0,
        centre - Vec2::new(0.0, 50.0),
        centre + Vec2::new(0.0, 50.0),
        false,
        false,
    );
    let chunks = build_road_meshes(&[seg], |_, _| 10.0, &RoadMeshSettings::default());
    let material = RoadMaterial {
        straight: TextureData::solid_rgba8([255, 0, 0, 255]),
        end: TextureData::solid_rgba8([0, 0, 255, 255]),
    };
    let feature =
        RoadFeature::new(&gpu.device, &gpu.queue, &renderer, &[material], &chunks).unwrap();
    renderer.add_feature(Box::new(feature));
    let camera = Camera {
        position: DVec3::new(f64::from(centre.x), 30.0, f64::from(centre.y)),
        pitch: -std::f32::consts::FRAC_PI_2 + 0.01,
        ..Camera::default()
    };
    // Opaque grey ground under the road (roads do not write depth; empty depth is sky).
    let ground = renderer.upload_mesh(&gpu, &MeshData::ground_plane(400.0, 1.0));
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw::at(
        ground,
        DVec3::new(f64::from(centre.x), 10.0, f64::from(centre.y)),
        [0.3, 0.3, 0.3, 1.0],
    ));
    let image = renderer
        .render_to_image(&gpu, 64, 64, &camera, &draws, 1.0 / 60.0)
        .expect("render");
    let at = |x: usize, y: usize| {
        let i = (y * 64 + x) * 4;
        [image[i], image[i + 1], image[i + 2]].map(u16::from)
    };
    let [r, g, b] = at(32, 32);
    assert!(r > 60 && r > 2 * g && r > 2 * b, "road below: {r} {g} {b}");
    // 10 m wide from 30 m up: the left and right image edges are off the road.
    let [r, g, _] = at(1, 32);
    assert!(r < g + 20, "left edge is grey ground, not road: {r} {g}");
}
