//! The move state machine of a Man: his move, the blend to the next one, the input a player
//! controller fills in and the movement that comes out of the animation.
//!
//! Everything runs on the synthetic moves config in `tests/common`, so it runs everywhere.

mod common;

use a3_world::{ManInput, World};
use glam::DVec3;

use common::{create_man, world};

/// Steps the World `frames` times at 1/15 s: one step of a Man per call, the step these tests
/// are written in.
fn run(world: &mut World, frames: usize) {
    for _ in 0..frames {
        world.simulate(1.0 / 15.0);
    }
}

/// A freshly created Man plays the idle move of his stance.
#[test]
fn a_man_stands_in_the_idle_move_of_his_stance() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));

    world.simulate(1.0 / 15.0);

    assert_eq!(world.animation_state(man), "stand");
}

/// Without a moves type there is nothing to play: he still stands on the ground (the motion of
/// `sim::man::ground` does not depend on the animation).
#[test]
fn without_moves_a_man_has_no_move() {
    let mut world = a3_world::World::new(a3_world::ClientId::SERVER);
    world
        .load_terrain(std::sync::Arc::new(common::flat_terrain(100.0)))
        .unwrap();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));

    world.simulate(1.0 / 15.0);

    assert_eq!(world.animation_state(man), "");
    assert_eq!(world.man(man).unwrap().input, ManInput::default());
}

/// Pushing forward, he plays the walk move and moves north — his front — at the speed the
/// animation gives him: the RTM step of the walk (1.62 m per cycle) times its phase rate
/// (0.85 cycles/s), i.e. 1.377 m/s (`docs/re/sim-man-movement.md` §3).
#[test]
fn walking_forward_moves_him_at_the_speed_of_his_move() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let man = create_man(&mut world, start);
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );

    for _ in 0..15 {
        world.simulate(1.0 / 15.0);
    }

    assert_eq!(world.animation_state(man), "walk");
    let entity = world.entity(man).unwrap();
    let speed = 1.62 * 0.85;
    assert!(
        (entity.velocity().z - speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - start;
    // A little short of the walk's speed: the first steps are slower while the walk blends in.
    assert!((moved.z - speed).abs() < 0.1, "{moved:?}");
    assert!(moved.x.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// Pushing backward he plays the walk-back move and moves south, behind his front: 1.12 m per
/// cycle × 0.8 cycles/s. `Stand` has no direct edge to `WalkBack`, so the request goes through
/// the route `Stand → Walk → WalkBack` (`docs/re/sim-man-movement.md` §2).
#[test]
fn walking_backward_moves_him_backwards() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let man = create_man(&mut world, start);
    world.set_man_input(
        man,
        ManInput {
            forward: -1.0,
            ..ManInput::default()
        },
    );

    // The route takes two hops and the blend has to reverse his velocity, so let him settle
    // first.
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "walkback");
    assert!(world.entity(man).unwrap().position().z < start.z, "he went back");

    let before = world.entity(man).unwrap().position();
    run(&mut world, 15);
    let entity = world.entity(man).unwrap();
    let speed = 1.12 * 0.8;
    assert!(
        (entity.velocity().z + speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - before;
    assert!((moved.z + speed).abs() < 1e-3, "{moved:?}");
    assert!(moved.x.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// Strafing left plays the left move and moves him west, his left while he faces north: 1.0 m
/// per cycle × 0.5 cycles/s. He keeps his front.
#[test]
fn strafing_left_moves_him_left() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            strafe: -1.0,
            ..ManInput::default()
        },
    );

    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "walkleft");

    let before = world.entity(man).unwrap().position();
    run(&mut world, 15);
    let entity = world.entity(man).unwrap();
    let speed = 1.0 * 0.5;
    assert!(
        (entity.velocity().x + speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - before;
    assert!((moved.x + speed).abs() < 1e-3, "{moved:?}");
    assert!(moved.z.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// He does not jump from standing to the walk's speed: the blend runs over the walk's
/// `interpolationSpeed`, so the speed builds up from the idle he was in.
#[test]
fn starting_to_walk_blends_in() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );

    world.simulate(1.0 / 15.0);
    let first = world.entity(man).unwrap().velocity().z;
    let walk = 1.62 * 0.85;
    assert!(first > 0.0 && first < walk, "{first}");

    for _ in 0..15 {
        world.simulate(1.0 / 15.0);
    }
    let later = world.entity(man).unwrap().velocity().z;
    assert!(
        later > first,
        "the blend must build up: {first} then {later}"
    );
    assert!((later - walk).abs() < 1e-3, "{later}");
}
