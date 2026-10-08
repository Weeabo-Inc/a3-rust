//! The move state machine of a Man: his move, the blend to the next one, the input a player
//! controller fills in and the movement that comes out of the animation.
//!
//! Everything runs on the synthetic moves config in `tests/common`, so it runs everywhere.

mod common;

use a3_world::ManInput;
use glam::DVec3;

use common::{create_man, world};

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
