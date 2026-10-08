//! Rigid bodies: dynamic bodies fall and rest on terrain and Static objects; kinematic bodies
//! go where they are put.

mod common;

use a3_physics::{BodyKind, Interest, Layer};
use common::*;
use glam::{DQuat, DVec3};

const DT: f64 = 1.0 / 60.0;

fn crate_model(mass: f32) -> a3_p3d::Model {
    model_with_mass(
        vec![box_lod(
            GEOMETRY,
            &[part([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5], "metal.bisurf")],
        )],
        mass,
    )
}

#[test]
fn a_dynamic_body_falls_onto_the_terrain_and_goes_to_sleep() {
    let (mut w, statics) = world(flat_terrain(5.0).build(), &[]);
    let m = w.models().insert_model("crate.p3d", &crate_model(50.0));
    w.add_body(
        1,
        m,
        DVec3::new(30.0, 9.0, 30.0),
        DQuat::IDENTITY,
        BodyKind::Dynamic,
    );
    w.stream(
        &statics,
        &[Interest {
            center: DVec3::new(30.0, 0.0, 30.0),
            radius: 20.0,
        }],
    );
    // Free fall: after 0.5 s it has dropped g t² / 2.
    for _ in 0..30 {
        w.step(DT);
    }
    let s = w.body(1).unwrap();
    assert!(
        (9.0 - s.position.y - 0.5 * 9.8066 * 0.25).abs() < 0.05,
        "{s:?}"
    );
    for _ in 0..600 {
        w.step(DT);
    }
    let s = w.body(1).unwrap();
    // The model origin is its bottom face: it rests on the ground at 5 m.
    assert!((s.position.y - 5.0).abs() < 0.05, "{s:?}");
    assert!(s.linear_velocity.length() < 0.05);
    assert!(s.sleeping, "{s:?}");
}

#[test]
fn a_dynamic_body_lands_on_a_static_object() {
    let table = model(vec![box_lod(
        GEOMETRY,
        &[part([-2.0, 0.0, -2.0], [2.0, 2.0, 2.0], "concrete.bisurf")],
    )]);
    let terrain = flat_terrain(0.0)
        .object("table.p3d", at(30.0, 0.0, 30.0))
        .build();
    let (mut w, statics) = world(terrain, &[("table.p3d", table)]);
    let m = w.models().insert_model("crate.p3d", &crate_model(50.0));
    w.stream(
        &statics,
        &[Interest {
            center: DVec3::new(30.0, 0.0, 30.0),
            radius: 20.0,
        }],
    );
    w.add_body(
        1,
        m,
        DVec3::new(30.0, 6.0, 30.0),
        DQuat::IDENTITY,
        BodyKind::Dynamic,
    );
    for _ in 0..300 {
        w.step(DT);
    }
    let y = w.body(1).unwrap().position.y;
    assert!((y - 2.0).abs() < 0.05, "rests on the table top: {y}");
}

#[test]
fn a_kinematic_body_ignores_gravity_and_follows_its_poses() {
    let (mut w, _) = world(flat_terrain(0.0).build(), &[]);
    let m = w.models().insert_model("crate.p3d", &crate_model(50.0));
    w.add_body(
        2,
        m,
        DVec3::new(10.0, 3.0, 10.0),
        DQuat::IDENTITY,
        BodyKind::Kinematic,
    );
    for _ in 0..30 {
        w.step(DT);
    }
    assert_eq!(w.body(2).unwrap().position, DVec3::new(10.0, 3.0, 10.0));
    w.set_body_pose(2, DVec3::new(11.0, 3.0, 10.0), DQuat::IDENTITY);
    w.step(DT);
    let s = w.body(2).unwrap();
    assert!((s.position.x - 11.0).abs() < 1e-9);
    // The kinematic velocity is what it moved in the step.
    assert!((s.linear_velocity.x - 60.0).abs() < 1e-6, "{s:?}");
}

#[test]
fn switching_a_body_to_kinematic_stops_its_simulation_and_back_resumes_it() {
    let (mut w, _) = world(flat_terrain(0.0).build(), &[]);
    let m = w.models().insert_model("crate.p3d", &crate_model(50.0));
    w.add_body(
        3,
        m,
        DVec3::new(10.0, 20.0, 10.0),
        DQuat::IDENTITY,
        BodyKind::Dynamic,
    );
    w.set_body_kind(3, BodyKind::Kinematic);
    for _ in 0..30 {
        w.step(DT);
    }
    assert_eq!(w.body(3).unwrap().position.y, 20.0);
    w.set_body_kind(3, BodyKind::Dynamic);
    for _ in 0..30 {
        w.step(DT);
    }
    assert!(w.body(3).unwrap().position.y < 19.0);
}

#[test]
fn mass_and_centre_of_mass_come_from_the_model() {
    let (mut w, _) = world(flat_terrain(0.0).build(), &[]);
    let mut heavy = crate_model(1000.0);
    heavy.info.center_of_mass = glam::Vec3::new(0.0, 0.25, 0.0);
    let m = w.models().insert_model("heavy.p3d", &heavy);
    assert_eq!(m.mass().mass, 1000.0);
    w.add_body(
        4,
        m,
        DVec3::new(10.0, 20.0, 10.0),
        DQuat::IDENTITY,
        BodyKind::Dynamic,
    );
    // An upward impulse of 1000 N·s at the centre of mass gives 1 m/s.
    w.apply_impulse_at(
        4,
        DVec3::new(0.0, 1000.0, 0.0),
        DVec3::new(10.0, 20.25, 10.0),
    );
    let v = w.body(4).unwrap().linear_velocity;
    assert!((v.y - 1.0).abs() < 1e-9, "{v:?}");
    assert!(w.body(4).unwrap().angular_velocity.length() < 1e-9);
    // Queries see the body's layers.
    assert!(w.body_model(4).unwrap().layer(Layer::Geometry).is_some());
}
