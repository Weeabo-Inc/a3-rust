//! Queries on a collision world built from synthetic terrains and models.

mod common;

use a3_physics::{
    Interest, Layer, LayerMask, ObjectKey, QueryShape, RayQuery, StaticSource, TerrainStatics,
};
use a3_wrp::{TerrainBuilder, Transform};
use common::*;
use glam::{DQuat, DVec3, Vec3};

/// A 4 m concrete cube (Geometry), a slightly larger metal Fire Geometry and a glass View
/// Geometry, placed at (40, 0, 40) on flat ground at 0.
fn house() -> a3_p3d::Model {
    model(vec![
        box_lod(
            GEOMETRY,
            &[part([-2.0, 0.0, -2.0], [2.0, 4.0, 2.0], "concrete.bisurf")],
        ),
        box_lod(
            FIRE,
            &[part([-2.5, 0.0, -2.5], [2.5, 4.0, 2.5], "metal.bisurf")],
        ),
        box_lod(
            VIEW,
            &[part([-2.0, 0.0, -2.0], [2.0, 4.0, 2.0], "glass.bisurf")],
        ),
    ])
}

fn stream_everything(w: &mut a3_physics::CollisionWorld, statics: &TerrainStatics) {
    w.stream(
        statics,
        &[Interest {
            center: DVec3::new(64.0, 0.0, 64.0),
            radius: 100.0,
        }],
    );
}

fn the_key(statics: &TerrainStatics) -> u32 {
    let mut key = None;
    let grid = statics.land_grid();
    for z in 0..grid.height {
        for x in 0..grid.width {
            statics.for_each_in_cell(x, z, &mut |p| key = Some(p.key));
        }
    }
    key.expect("one object")
}

#[test]
fn a_ray_down_onto_a_house_reports_object_component_surface_normal_and_distance() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);

    let q = RayQuery::new(
        DVec3::new(40.5, 10.0, 39.0),
        DVec3::new(40.5, -10.0, 39.0),
        Layer::Geometry,
    );
    let hit = w.ray_cast(&q).expect("hits the roof");
    assert_eq!(hit.object, ObjectKey::Static(the_key(&statics)));
    assert_eq!(hit.layer, Some(Layer::Geometry));
    assert_eq!(hit.component, Some(0));
    assert!((hit.distance - 6.0).abs() < 1e-6, "{hit:?}");
    assert!((hit.position.y - 4.0).abs() < 1e-6);
    assert!((hit.normal - DVec3::Y).length() < 1e-6, "{:?}", hit.normal);
    let surface = w.surface(hit.surface.expect("has a surface"));
    assert_eq!(surface.sound_environ, "concrete");
}

#[test]
fn each_layer_answers_with_its_own_lod() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);
    // Horizontal ray from the west at 1 m height.
    let from = DVec3::new(30.0, 1.0, 40.0);
    let to = DVec3::new(50.0, 1.0, 40.0);
    let geometry = w
        .ray_cast(&RayQuery::new(from, to, Layer::Geometry))
        .unwrap();
    let fire = w
        .ray_cast(&RayQuery::new(from, to, Layer::FireGeometry))
        .unwrap();
    assert!((geometry.position.x - 38.0).abs() < 1e-6);
    assert!((fire.position.x - 37.5).abs() < 1e-6);
    assert_eq!(w.surface(fire.surface.unwrap()).sound_environ, "metal");
    // Nothing on the Roadway layer.
    assert!(
        w.ray_cast(&RayQuery::new(from, to, Layer::Roadway))
            .is_none()
    );
}

#[test]
fn fire_and_view_queries_fall_back_to_the_geometry_lod() {
    let crate_model = model(vec![box_lod(
        GEOMETRY,
        &[part([-1.0, 0.0, -1.0], [1.0, 1.0, 1.0], "concrete.bisurf")],
    )]);
    let terrain = flat_terrain(0.0)
        .object("crate.p3d", at(20.0, 0.0, 20.0))
        .build();
    let (mut w, statics) = world(terrain, &[("crate.p3d", crate_model)]);
    stream_everything(&mut w, &statics);
    let q = |layer: Layer| {
        RayQuery::new(
            DVec3::new(20.0, 5.0, 20.0),
            DVec3::new(20.0, -5.0, 20.0),
            layer,
        )
    };
    for layer in [Layer::Geometry, Layer::FireGeometry, Layer::ViewGeometry] {
        let hit = w.ray_cast(&q(layer)).expect("hit");
        assert!((hit.position.y - 1.0).abs() < 1e-6, "{layer:?}");
        assert_eq!(hit.layer, Some(layer));
    }
}

#[test]
fn the_terrain_is_hit_where_no_object_is_and_can_be_left_out() {
    let terrain = flat_terrain(3.0).build();
    let (w, _) = world(terrain, &[]);
    let q = RayQuery::new(
        DVec3::new(10.0, 50.0, 10.0),
        DVec3::new(10.0, -50.0, 10.0),
        LayerMask::ALL,
    );
    let hit = w.ray_cast(&q).unwrap();
    assert_eq!(hit.object, ObjectKey::Terrain);
    assert!((hit.position.y - 3.0).abs() < 1e-9);
    assert!(w.ray_cast(&q.without_terrain()).is_none());
}

#[test]
fn objects_of_cells_outside_every_interest_are_not_loaded_until_streamed_in() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(8.0, 0.0, 8.0))
        .object("house.p3d", at(120.0, 0.0, 120.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    w.set_keep_generations(2);
    let near = |x: f64| Interest {
        center: DVec3::new(x, 0.0, x),
        radius: 10.0,
    };
    let down = |x: f64| {
        RayQuery::new(
            DVec3::new(x, 10.0, x),
            DVec3::new(x, -10.0, x),
            Layer::Geometry,
        )
        .without_terrain()
    };
    w.stream(&statics, &[near(8.0)]);
    assert!(w.ray_cast(&down(8.0)).is_some());
    assert!(w.ray_cast(&down(120.0)).is_none(), "far cell not loaded");

    // Interest moves away: the first house survives `keep` streams, then goes.
    for _ in 0..2 {
        w.stream(&statics, &[near(120.0)]);
        assert!(w.ray_cast(&down(8.0)).is_some());
    }
    let report = w.stream(&statics, &[near(120.0)]);
    assert!(report.cells_unloaded > 0);
    assert!(w.ray_cast(&down(8.0)).is_none());
    assert!(w.ray_cast(&down(120.0)).is_some());
}

#[test]
fn a_scaled_object_collides_at_its_scaled_size() {
    let m = model(vec![box_lod(
        GEOMETRY,
        &[part([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0], "concrete.bisurf")],
    )]);
    let mut t = at(64.0, 0.0, 64.0);
    for c in [0, 4, 8] {
        t.0[c] = 2.0;
    }
    let terrain = flat_terrain(0.0).object("rock.p3d", t).build();
    let (mut w, statics) = world(terrain, &[("rock.p3d", m)]);
    stream_everything(&mut w, &statics);
    let hit = w
        .ray_cast(&RayQuery::new(
            DVec3::new(64.0, 10.0, 64.0),
            DVec3::new(64.0, -10.0, 64.0),
            Layer::Geometry,
        ))
        .unwrap();
    assert!((hit.position.y - 4.0).abs() < 1e-6, "{hit:?}");
}

#[test]
fn a_rotated_object_is_hit_where_its_rotated_geometry_is() {
    // A 1 x 1 x 8 m beam along model z, turned 90 degrees to lie along world x.
    let m = model(vec![box_lod(
        GEOMETRY,
        &[part([-0.5, 0.0, -4.0], [0.5, 1.0, 4.0], "concrete.bisurf")],
    )]);
    // Columns: aside = -z, up = y, dir = +x.
    let t = Transform([
        0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 64.0, 0.0, 64.0,
    ]);
    let terrain = flat_terrain(0.0).object("beam.p3d", t).build();
    let (mut w, statics) = world(terrain, &[("beam.p3d", m)]);
    stream_everything(&mut w, &statics);
    let down = |x: f64, z: f64| {
        RayQuery::new(
            DVec3::new(x, 5.0, z),
            DVec3::new(x, -5.0, z),
            Layer::Geometry,
        )
        .without_terrain()
    };
    assert!(w.ray_cast(&down(67.0, 64.0)).is_some());
    assert!(w.ray_cast(&down(64.0, 67.0)).is_none());
}

#[test]
fn every_component_crossed_is_reported_in_order() {
    // Two walls 0.5 m thick, 4 m apart, in one model.
    let m = model(vec![box_lod(
        GEOMETRY,
        &[
            part([-0.25, 0.0, -3.0], [0.25, 3.0, 3.0], "concrete.bisurf"),
            part([3.75, 0.0, -3.0], [4.25, 3.0, 3.0], "metal.bisurf"),
        ],
    )]);
    let terrain = flat_terrain(0.0)
        .object("walls.p3d", at(60.0, 0.0, 60.0))
        .build();
    let (mut w, statics) = world(terrain, &[("walls.p3d", m)]);
    stream_everything(&mut w, &statics);
    let q = RayQuery::new(
        DVec3::new(50.0, 1.0, 60.0),
        DVec3::new(70.0, 1.0, 60.0),
        Layer::Geometry,
    );
    let hits = w.ray_cast_all(&q);
    let xs: Vec<f64> = hits.iter().map(|h| h.position.x).collect();
    assert_eq!(hits.len(), 2, "{xs:?}");
    assert!(
        (xs[0] - 59.75).abs() < 1e-6 && (xs[1] - 63.75).abs() < 1e-6,
        "{xs:?}"
    );
    assert_eq!(hits[0].component, Some(0));
    assert_eq!(hits[1].component, Some(1));

    let p = w.penetrations(&q);
    assert_eq!(p.len(), 2);
    assert!((p[0].depth() - 0.5).abs() < 1e-6, "{p:?}");
    assert!((p[1].entry.distance - 13.75).abs() < 1e-6);
    assert_eq!(
        w.surface(p[1].entry.surface.unwrap())
            .penetration_resistance,
        1e6 / 10.0
    );

    // A segment starting inside the first wall enters it at 0.
    let inside = RayQuery::new(
        DVec3::new(60.0, 1.0, 60.0),
        DVec3::new(70.0, 1.0, 60.0),
        Layer::Geometry,
    );
    let p = w.penetrations(&inside);
    assert_eq!(p[0].entry.distance, 0.0);
    assert!((p[0].exit_distance - 0.25).abs() < 1e-6);
}

#[test]
fn the_ignore_list_lets_rays_pass_through_objects() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);
    let key = ObjectKey::Static(the_key(&statics));
    let ignore = [key];
    let q = RayQuery::new(
        DVec3::new(40.0, 10.0, 40.0),
        DVec3::new(40.0, -10.0, 40.0),
        Layer::Geometry,
    );
    assert_eq!(w.ray_cast(&q).unwrap().object, key);
    assert_eq!(
        w.ray_cast(&q.ignoring(&ignore)).unwrap().object,
        ObjectKey::Terrain
    );
}

#[test]
fn surface_below_finds_a_bridge_deck_above_the_terrain_and_the_terrain_beside_it() {
    // A bridge deck 2 x 20 m at 10 m over terrain at 0, surfaced by its texture.
    let bridge = model(vec![
        box_lod(
            GEOMETRY,
            &[part(
                [-1.0, 9.0, -10.0],
                [1.0, 10.0, 10.0],
                "concrete.bisurf",
            )],
        ),
        roadway_lod(
            [-1.0, -10.0],
            [1.0, 10.0],
            10.0,
            "a3\\data_f\\surfaces\\betonout.paa",
        ),
    ]);
    let terrain = flat_terrain(0.0)
        .object("bridge.p3d", at(64.0, 0.0, 64.0))
        .build();
    let (mut w, statics) = world(terrain, &[("bridge.p3d", bridge)]);
    stream_everything(&mut w, &statics);

    let on = w.surface_below(DVec3::new(64.0, 11.0, 70.0), 50.0).unwrap();
    assert!((on.y - 10.0).abs() < 1e-6, "{on:?}");
    assert!(matches!(on.object, ObjectKey::Static(_)));
    assert!(on.normal.y > 0.99);
    assert_eq!(w.surface(on.surface.unwrap()).name, "#Concrete");

    let beside = w.surface_below(DVec3::new(70.0, 11.0, 70.0), 50.0).unwrap();
    assert_eq!(beside.object, ObjectKey::Terrain);
    assert_eq!(beside.y, 0.0);

    // Under the deck the terrain is the surface.
    let under = w.surface_below(DVec3::new(64.0, 5.0, 70.0), 50.0).unwrap();
    assert_eq!(under.object, ObjectKey::Terrain);
}

#[test]
fn a_capsule_moving_into_a_wall_stops_at_it() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);
    let capsule = QueryShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    // From x = 30 towards the wall at x = 38, 10 m of motion.
    let hit = w
        .shape_cast(
            capsule,
            DVec3::new(30.0, 1.2, 40.0),
            DQuat::IDENTITY,
            DVec3::new(10.0, 0.0, 0.0),
            LayerMask::GEOMETRY,
            true,
            &[],
        )
        .expect("touches the wall");
    assert!((hit.fraction - 0.77).abs() < 1e-3, "{hit:?}");
    assert!((hit.normal - DVec3::NEG_X).length() < 1e-3);
    assert!(matches!(hit.object, ObjectKey::Static(_)));
    assert_eq!(hit.component, Some(0));

    // Overlaps: a sphere half inside the wall, then one in the open.
    let o = w.overlaps(
        QueryShape::Sphere { radius: 0.5 },
        DVec3::new(38.2, 1.0, 40.0),
        DQuat::IDENTITY,
        LayerMask::GEOMETRY,
        false,
        &[],
    );
    assert_eq!(o.len(), 1);
    assert!(
        w.overlaps(
            QueryShape::Sphere { radius: 0.5 },
            DVec3::new(30.0, 1.0, 40.0),
            DQuat::IDENTITY,
            LayerMask::GEOMETRY,
            false,
            &[],
        )
        .is_empty()
    );
}

#[test]
fn contacts_tell_how_deep_a_shape_is_in_a_wall_and_which_way_is_out() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);
    // A bone capsule from the hip to the shoulder, 0.1 m into the west wall (x = 38).
    let bone = QueryShape::Segment {
        a: DVec3::new(0.0, -0.4, 0.0),
        b: DVec3::new(0.0, 0.4, 0.0),
        radius: 0.2,
    };
    let c = w.contacts(
        bone,
        DVec3::new(37.9, 1.2, 40.0),
        DQuat::IDENTITY,
        0.05,
        LayerMask::GEOMETRY,
        false,
        &[],
    );
    assert_eq!(c.len(), 1, "{c:?}");
    assert!((c[0].distance + 0.1).abs() < 1e-6, "{c:?}");
    assert!((c[0].normal - DVec3::NEG_X).length() < 1e-6);
    assert_eq!(c[0].component, Some(0));
    let name = &w.component(c[0].shape.unwrap(), 0).unwrap().name;
    assert_eq!(name, "Component01");
    // Within the margin but not touching: a positive distance.
    let near = w.contacts(
        bone,
        DVec3::new(37.77, 1.2, 40.0),
        DQuat::IDENTITY,
        0.05,
        LayerMask::GEOMETRY,
        false,
        &[],
    );
    assert!((near[0].distance - 0.03).abs() < 1e-6, "{near:?}");
    assert!(
        w.contacts(
            bone,
            DVec3::new(37.0, 1.2, 40.0),
            DQuat::IDENTITY,
            0.05,
            LayerMask::GEOMETRY,
            false,
            &[]
        )
        .is_empty()
    );
}

#[test]
fn a_shape_cast_down_lands_on_the_loaded_terrain() {
    let terrain = flat_terrain(2.0).build();
    let (mut w, statics) = world(terrain, &[]);
    stream_everything(&mut w, &statics);
    let hit = w
        .shape_cast(
            QueryShape::Sphere { radius: 0.5 },
            DVec3::new(10.0, 10.0, 10.0),
            DQuat::IDENTITY,
            DVec3::new(0.0, -20.0, 0.0),
            LayerMask::NONE,
            true,
            &[],
        )
        .expect("lands");
    assert_eq!(hit.object, ObjectKey::Terrain);
    // Centre stops 0.5 m above 2 m: travelled 7.5 of 20.
    assert!((hit.fraction - 7.5 / 20.0).abs() < 1e-3, "{hit:?}");
}

#[test]
fn a_shape_cast_lands_on_the_sloped_heightfield_where_the_analytic_terrain_is() {
    // 128 m square, 4 m height cells. Heights rise east and north at different rates, and the
    // height sample at grid (2, 2) is raised 8 m, so cell (1, 1)..(2, 2) has its two triangles
    // far apart: a chunk whose axes are swapped or split along the other diagonal sits elsewhere.
    let terrain = TerrainBuilder::new(8, 32, 16.0)
        .heights(|i, j| {
            let h = (i + 2 * j) as f32;
            if (i, j) == (2, 2) { h + 8.0 } else { h }
        })
        .build();
    let analytic = a3_physics::TerrainShape::new(std::sync::Arc::new(terrain.clone()));
    let (mut w, statics) = world(terrain, &[]);
    stream_everything(&mut w, &statics);
    // A small sphere stops with its centre `radius / n.y` above the surface under it, where `n`
    // is the surface normal there: the touch point of a plane is that far below the centre.
    let probe = QueryShape::Sphere { radius: 0.05 };
    // Inside the steep upper-right triangle of cell (1, 1), where the two possible splits of the
    // cell differ by 4 m, and a grid over the whole terrain for chunk placement and the far edge.
    let mut points = vec![(5.9, 6.9)];
    for k in 0..=6 {
        for l in 0..=6 {
            points.push((0.7 + 21.0 * f64::from(k), 0.7 + 21.0 * f64::from(l)));
        }
    }
    for &(x, z) in &points {
        let hit = w
            .shape_cast(
                probe,
                DVec3::new(x, 200.0, z),
                DQuat::IDENTITY,
                DVec3::new(0.0, -200.0, 0.0),
                LayerMask::NONE,
                true,
                &[],
            )
            .unwrap_or_else(|| panic!("({x}, {z}): no hit"));
        assert_eq!(hit.object, ObjectKey::Terrain, "({x}, {z})");
        let centre = 200.0 - hit.fraction * 200.0;
        let expected = analytic.height(x, z) + 0.05 / analytic.normal(x, z).y;
        assert!(
            (centre - expected).abs() < 1e-3,
            "({x}, {z}): centre at {centre}, surface at {expected}"
        );
    }
}

#[test]
fn points_inside_a_building_find_it() {
    let terrain = flat_terrain(0.0)
        .object("house.p3d", at(40.0, 0.0, 40.0))
        .build();
    let (mut w, statics) = world(terrain, &[("house.p3d", house())]);
    stream_everything(&mut w, &statics);
    let inside = w.objects_at(DVec3::new(40.0, 1.0, 40.0), LayerMask::GEOMETRY);
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].component, Some(0));
    assert!(
        w.objects_at(DVec3::new(45.0, 1.0, 40.0), LayerMask::GEOMETRY)
            .is_empty()
    );
}

#[test]
fn visibility_is_cut_by_terrain_and_opaque_view_geometry_and_reduced_by_transparent() {
    let bush = model(vec![
        box_lod(
            GEOMETRY,
            &[part([-0.1, 0.0, -0.1], [0.1, 0.5, 0.1], "concrete.bisurf")],
        ),
        box_lod(
            VIEW,
            &[part([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0], "leaves.bisurf")],
        ),
    ]);
    let wall = model(vec![box_lod(
        GEOMETRY,
        &[part([-0.2, 0.0, -2.0], [0.2, 3.0, 2.0], "concrete.bisurf")],
    )]);
    let terrain = flat_terrain(0.0)
        .object("bush.p3d", at(40.0, 0.0, 20.0))
        .object("wall.p3d", at(40.0, 0.0, 60.0))
        .build();
    let (mut w, statics) = world(terrain, &[("bush.p3d", bush), ("wall.p3d", wall)]);
    stream_everything(&mut w, &statics);
    let eye = |z: f64| DVec3::new(30.0, 1.0, z);
    let target = |z: f64| DVec3::new(50.0, 1.0, z);
    assert_eq!(w.visibility(eye(100.0), target(100.0), &[]), 1.0);
    assert!((w.visibility(eye(20.0), target(20.0), &[]) - 0.8).abs() < 1e-6);
    assert_eq!(w.visibility(eye(60.0), target(60.0), &[]), 0.0);
    // Below ground.
    assert_eq!(
        w.visibility(
            DVec3::new(30.0, -1.0, 100.0),
            DVec3::new(50.0, 1.0, 100.0),
            &[]
        ),
        0.0
    );
}

#[test]
fn an_entity_body_is_hit_by_queries_until_removed() {
    let (mut w, _) = world(flat_terrain(0.0).build(), &[]);
    let m = w.models().insert_model(
        "car.p3d",
        &model(vec![box_lod(
            GEOMETRY,
            &[part([-1.0, 0.0, -2.0], [1.0, 1.5, 2.0], "metal.bisurf")],
        )]),
    );
    w.add_body(
        7,
        m,
        DVec3::new(30.0, 0.0, 30.0),
        DQuat::IDENTITY,
        a3_physics::BodyKind::Kinematic,
    );
    let q = RayQuery::new(
        DVec3::new(30.0, 5.0, 30.0),
        DVec3::new(30.0, -5.0, 30.0),
        Layer::FireGeometry,
    );
    let hit = w.ray_cast(&q).unwrap();
    assert_eq!(hit.object, ObjectKey::Entity(7));
    assert!((hit.position.y - 1.5).abs() < 1e-6);
    // Moved: the colliders follow at once.
    w.set_body_pose(7, DVec3::new(35.0, 0.0, 30.0), DQuat::IDENTITY);
    assert_eq!(w.ray_cast(&q).unwrap().object, ObjectKey::Terrain);
    w.remove_body(7);
    let moved = RayQuery::new(
        DVec3::new(35.0, 5.0, 30.0),
        DVec3::new(35.0, -5.0, 30.0),
        Layer::FireGeometry,
    );
    assert_eq!(w.ray_cast(&moved).unwrap().object, ObjectKey::Terrain);
}

#[test]
fn the_terrain_statics_key_finds_its_wrp_object() {
    let terrain = flat_terrain(0.0)
        .object("a.p3d", at(1.0, 0.0, 1.0))
        .object("b.p3d", at(100.0, 0.0, 20.0))
        .object("c.p3d", at(2.0, 0.0, 2.0))
        .build();
    let statics = TerrainStatics::new(std::sync::Arc::new(terrain));
    let mut seen = Vec::new();
    statics.for_each_in_cell(0, 0, &mut |p| seen.push((p.key, p.model.to_owned())));
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].0, TerrainStatics::key(0, 0, 1));
    assert_eq!(statics.object(seen[1].0).unwrap().id, 2);
    assert_eq!(
        statics
            .object(TerrainStatics::key(6, 1, 0))
            .unwrap()
            .transform
            .position(),
        Vec3::new(100.0, 0.0, 20.0)
    );
}
