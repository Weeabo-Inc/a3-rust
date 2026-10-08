//! The Man family: the ground he walks on, his motion, the moves state machine and the input
//! a player controller fills in.
//!
//! The ground is the collision world of `a3-physics` (ADR 0008): a synthetic terrain and
//! synthetic Roadway models placed as Static objects, so the tests run everywhere.

mod common;

use std::sync::Arc;

use a3_physics::{CollisionWorld, Interest};
use a3_world::{ClientId, Create, EntityType, Motion, NetworkId, SimulationClass, World};
use a3_wrp::TerrainBuilder;
use common::{at, collision_world, create_man, flat_terrain, model, roadway_lod, terrain_with};
use glam::DVec3;

/// The terrain surface every fixture below stands on.
const GROUND: f64 = 100.0;

/// The collision world of a flat terrain at [`GROUND`] with nothing on it.
fn flat_ground() -> CollisionWorld {
    collision_world(Arc::new(flat_terrain(GROUND as f32)), &[], &[])
}

/// The collision world of a flat terrain at [`GROUND`] with one Roadway platform `height`
/// metres over it, from world `edge` in +z: a kerb (0.3), a floor, or a bridge deck (3.0) with
/// the terrain running on beside and under it.
fn ground_with_platform(height: f64, edge: f32) -> CollisionWorld {
    let terrain = terrain_with(GROUND as f32, "platform.p3d", at(0.0, GROUND as f32, 0.0));
    let platform = model(vec![roadway_lod(
        [0.0, edge],
        [200.0, 200.0],
        height as f32,
    )]);
    collision_world(
        terrain,
        &[("platform.p3d", platform)],
        &[Interest {
            center: DVec3::new(100.0, GROUND, 100.0),
            radius: 400.0,
        }],
    )
}

#[test]
fn a_man_standing_still_keeps_his_feet_on_the_ground() {
    let ground = flat_ground();
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(10.0, GROUND, 20.0),
        DVec3::ZERO,
        &ground,
        1.0 / 15.0,
    );

    assert_eq!(feet, DVec3::new(10.0, GROUND, 20.0));
    assert!(motion.on_ground);
    assert_eq!(motion.vertical_speed, 0.0);
}

#[test]
fn walking_moves_the_feet_and_follows_the_ground() {
    // The terrain rises 1 m per 50 m cell eastwards: a 1:50 slope.
    let ground = collision_world(
        Arc::new(
            TerrainBuilder::new(4, 4, 50.0)
                .heights(|i, _| i as f32)
                .build(),
        ),
        &[],
        &[],
    );
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(60.0, 1.2, 60.0),
        DVec3::new(1.5, 0.0, 0.0),
        &ground,
        1.0,
    );

    assert!((feet.x - 61.5).abs() < 1e-9, "{feet:?}");
    assert!((feet.y - 1.23).abs() < 1e-9, "{feet:?}");
    assert!(motion.on_ground);
}

#[test]
fn walking_off_a_ledge_starts_a_fall() {
    // A bridge deck 3 m over the terrain, ending where the terrain runs on.
    let ground = ground_with_platform(3.0, 100.0);
    let mut motion = Motion::default();
    let feet = DVec3::new(100.0, GROUND + 3.0, 110.0);
    assert!(
        ground
            .surface_below(feet.with_y(feet.y + 0.5), 1.0)
            .is_some_and(|s| (s.y - feet.y).abs() < 1e-6),
        "the fixture puts him on the deck"
    );

    let feet = motion.step(feet, DVec3::new(0.0, 0.0, -4.0), &ground, 3.0);

    assert!(!motion.on_ground, "a drop taller than a step starts a fall");
    assert_eq!(
        feet.y,
        GROUND + 3.0,
        "he keeps his height as the fall starts"
    );
    assert!(feet.z < 100.0, "he walked off the deck: {feet:?}");
}

#[test]
fn a_falling_man_accelerates_with_gravity() {
    let ground = flat_ground();
    let mut motion = Motion {
        on_ground: false,
        vertical_speed: 0.0,
    };

    let feet = motion.step(DVec3::new(10.0, 200.0, 20.0), DVec3::ZERO, &ground, 1.0);

    assert!(motion.vertical_speed < 0.0);
    assert!(feet.y < 200.0);
    assert!(!motion.on_ground);
}

#[test]
fn a_falling_man_lands_on_the_ground() {
    let ground = flat_ground();
    let mut motion = Motion {
        on_ground: false,
        vertical_speed: -2.0,
    };

    // 2 m above the ground, falling at 2 m/s: the step of 1 s takes him through the surface.
    let feet = motion.step(
        DVec3::new(0.0, GROUND + 2.0, 0.0),
        DVec3::ZERO,
        &ground,
        1.0,
    );

    assert!(motion.on_ground);
    assert_eq!(motion.vertical_speed, 0.0);
    assert_eq!(feet.y, GROUND);
}

#[test]
fn a_step_within_reach_is_climbed() {
    // A 0.3 m kerb from z = 100 on.
    let ground = ground_with_platform(0.3, 100.0);
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(100.0, GROUND, 90.0),
        DVec3::new(0.0, 0.0, 20.0),
        &ground,
        0.6,
    );

    assert!(motion.on_ground);
    assert_eq!(feet.z, 102.0);
    // The kerb top is an f32 in the model, so it comes back a hair off 100.3.
    assert!((feet.y - (GROUND + 0.3)).abs() < 1e-6, "{feet:?}");
}

#[test]
fn a_ledge_too_high_to_step_onto_does_not_lift_him() {
    // A 3 m deck from z = 100 on, tall enough to walk under.
    let ground = ground_with_platform(3.0, 100.0);
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(100.0, GROUND, 90.0),
        DVec3::new(0.0, 0.0, 20.0),
        &ground,
        0.6,
    );

    assert!(motion.on_ground, "the terrain under the deck carries him");
    assert_eq!(feet.z, 102.0, "he walked under the deck, not onto it");
    assert_eq!(feet.y, GROUND);
}

#[test]
fn a_step_down_within_reach_is_walked_down() {
    // A 0.3 m kerb up to z = 100, then the terrain again.
    let ground = ground_with_platform(0.3, 100.0);
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(100.0, GROUND + 0.3, 110.0),
        DVec3::new(0.0, 0.0, -20.0),
        &ground,
        0.6,
    );

    assert!(motion.on_ground, "a small drop is walked down, not fallen");
    assert_eq!(feet.z, 98.0);
    assert_eq!(feet.y, GROUND);
}

#[test]
fn a_man_who_has_no_ground_under_him_falls() {
    // 100 m above the terrain: nothing is within a step's reach.
    let ground = flat_ground();
    let mut motion = Motion::default();

    // The step that loses the ground only enters the fall; gravity acts from the next one.
    let feet = motion.step(DVec3::new(10.0, 200.0, 20.0), DVec3::ZERO, &ground, 0.1);
    assert!(!motion.on_ground);
    assert_eq!(feet.y, 200.0);

    let feet = motion.step(feet, DVec3::ZERO, &ground, 0.1);
    assert!(motion.vertical_speed < 0.0);
    assert!(feet.y < 200.0);
}

/// A Man created in the air is stepped by [`World::simulate`] like any other Entity of his
/// family, and the collision world decides where he ends up.
#[test]
fn a_man_in_the_air_falls_to_the_terrain() {
    let mut world = World::new(ClientId::SERVER);
    world
        .load_terrain(Arc::new(flat_terrain(GROUND as f32)))
        .unwrap();
    world.set_collision_world(flat_ground());
    let ty = Arc::new(EntityType::new("B_Soldier_F", SimulationClass::Soldier));
    let man = world
        .create(Create::new(ty, DVec3::new(10.0, 110.0, 20.0)))
        .unwrap();

    for _ in 0..90 {
        world.simulate(1.0 / 15.0);
    }

    // 10 m of falling is over in ~1.4 s; afterwards he stands on the surface.
    let feet = world.entity(man).unwrap().position();
    assert!(
        (feet - DVec3::new(10.0, GROUND, 20.0)).length() < 1e-6,
        "{feet:?}"
    );
}

/// Under a bridge is not on it: the deck is the surface he lands on when he comes down over it,
/// and the terrain is the surface he walks on when he comes to it from beside the deck. Only the
/// collision world can tell the two apart (the terrain alone has one height per spot).
#[test]
fn a_man_falls_onto_the_bridge_deck_over_the_terrain() {
    let mut world = World::new(ClientId::SERVER);
    let terrain = terrain_with(GROUND as f32, "bridge.p3d", at(0.0, GROUND as f32, 0.0));
    world.load_terrain(terrain.clone()).unwrap();
    // A deck 3 m over the terrain, over the whole map from z = 100 on.
    let deck = model(vec![roadway_lod([0.0, 100.0], [200.0, 200.0], 3.0)]);
    world.set_collision_world(collision_world(
        terrain,
        &[("bridge.p3d", deck)],
        &[Interest {
            center: DVec3::new(100.0, GROUND, 100.0),
            radius: 400.0,
        }],
    ));
    let ty = Arc::new(EntityType::new("B_Soldier_F", SimulationClass::Soldier));
    let on_deck = world
        .create(Create::new(ty.clone(), DVec3::new(10.0, 110.0, 120.0)))
        .unwrap();
    let under = world
        .create(Create::new(ty, DVec3::new(10.0, GROUND, 20.0)))
        .unwrap();

    for _ in 0..90 {
        world.simulate(1.0 / 15.0);
    }

    let feet = world.entity(on_deck).unwrap().position();
    assert!(
        (feet - DVec3::new(10.0, GROUND + 3.0, 120.0)).length() < 1e-6,
        "he lands on the deck: {feet:?}"
    );
    let feet = world.entity(under).unwrap().position();
    assert_eq!(
        feet,
        DVec3::new(10.0, GROUND, 20.0),
        "and the man beside the deck stands on the terrain"
    );
}

/// A remote Man's position is his owner's to send: this machine only advances his local state
/// (the family rule in `sim::man`).
#[test]
fn a_remote_man_is_not_moved_by_this_machine() {
    let mut world = World::new(ClientId(5000));
    world
        .load_terrain(Arc::new(flat_terrain(GROUND as f32)))
        .unwrap();
    world.set_collision_world(flat_ground());
    let ty = Arc::new(EntityType::new("B_Soldier_F", SimulationClass::Soldier));
    let position = DVec3::new(10.0, 110.0, 20.0);
    let man = world
        .spawn_remote(ty, position, NetworkId::new(2, 9), None)
        .unwrap();

    world.simulate(1.0);

    assert_eq!(world.entity(man).unwrap().position(), position);
}

/// Without a collision world there is no surface to stand on or land on: the World leaves a Man
/// where he is (in the original a World always has one).
#[test]
fn a_world_without_a_collision_world_does_not_move_men() {
    let mut world = World::new(ClientId::SERVER);
    world
        .load_terrain(Arc::new(flat_terrain(GROUND as f32)))
        .unwrap();
    world.load_moves(common::moves());
    let man = create_man(&mut world, DVec3::new(10.0, 110.0, 20.0));

    world.simulate(1.0);

    assert_eq!(
        world.entity(man).unwrap().position(),
        DVec3::new(10.0, 110.0, 20.0)
    );
}
