//! `World::simulate`: frame sub-steps, per-Entity step accumulators and projectiles.

use std::sync::Arc;

use a3_world::{
    ClassState, ClientId, Create, EntityId, EntityType, SimulationClass, World, WorldEvent,
};
use glam::DVec3;

const EPS: f64 = 1e-9;

fn create(world: &mut World, class: SimulationClass) -> EntityId {
    let ty = Arc::new(EntityType::new("T", class));
    world.create(Create::new(ty, DVec3::ZERO)).unwrap()
}

fn with_step(world: &mut World, class: SimulationClass, step: f64) -> EntityId {
    let id = create(world, class);
    world.entity_mut(id).unwrap().set_simulation_step(step);
    id
}

fn steps(world: &World, id: EntityId) -> (u64, f64) {
    let e = world.entity(id).unwrap();
    (e.steps(), e.simulated_time())
}

#[test]
fn entities_step_only_when_their_accumulator_reaches_their_step() {
    let mut world = World::new(ClientId::SERVER);
    let car = with_step(&mut world, SimulationClass::CarX, 0.1);

    world.simulate(0.04);
    assert_eq!(steps(&world, car).0, 0);
    world.simulate(0.04);
    assert_eq!(steps(&world, car).0, 0);
    world.simulate(0.04);

    let (n, t) = steps(&world, car);
    assert_eq!(n, 1);
    assert!((t - 0.1).abs() < EPS);
}

#[test]
fn long_frames_are_cut_into_fixed_sub_steps() {
    // 0.2 s = eight 0.025 s sub-steps. With the default 1/15 s step the accumulator reaches the
    // step in sub-steps 3, 6 and 8: three steps of 1/15 s.
    let mut world = World::new(ClientId::SERVER);
    let man = create(&mut world, SimulationClass::Soldier);

    world.simulate(0.2);

    let (n, t) = steps(&world, man);
    assert_eq!(n, 3);
    assert!((t - 0.2).abs() < 1e-6);
}

#[test]
fn at_most_one_step_runs_per_sub_step() {
    // A 0.01 s step cannot keep up within one 0.016 s frame: one step, the rest carries over.
    let mut world = World::new(ClientId::SERVER);
    let car = with_step(&mut world, SimulationClass::Car, 0.01);

    world.simulate(0.016);

    assert_eq!(steps(&world, car).0, 1);
}

#[test]
fn a_zero_step_simulates_every_sub_step_with_the_accumulated_time() {
    let mut world = World::new(ClientId::SERVER);
    let thing = with_step(&mut world, SimulationClass::Thing, 0.0);

    world.simulate(0.12);

    // Four 0.025 s sub-steps, then the 0.02 remainder.
    let (n, t) = steps(&world, thing);
    assert_eq!(n, 5);
    assert!((t - 0.12).abs() < EPS);
}

#[test]
fn projectiles_simulate_the_whole_frame_in_chunks_of_their_step() {
    let mut world = World::new(ClientId::SERVER);
    let bullet = with_step(&mut world, SimulationClass::ShotBullet, 0.02);
    world
        .entity_mut(bullet)
        .unwrap()
        .set_velocity(DVec3::new(800.0, 0.0, 0.0));

    world.simulate(0.05);

    // 0.02 + 0.02 + 0.01 remainder.
    let e = world.entity(bullet).unwrap();
    assert_eq!(e.steps(), 3);
    assert!((e.simulated_time() - 0.05).abs() < EPS);
    assert!((e.position().x - 40.0).abs() < 1e-6);
}

#[test]
fn disabled_and_frozen_entities_do_not_step() {
    let mut world = World::new(ClientId::SERVER);
    let disabled = create(&mut world, SimulationClass::CarX);
    let frozen = create(&mut world, SimulationClass::CarX);
    world
        .entity_mut(disabled)
        .unwrap()
        .set_simulation_enabled(false);
    world
        .entity_mut(frozen)
        .unwrap()
        .set_dynamically_frozen(true);

    world.simulate(0.5);

    assert_eq!(steps(&world, disabled).0, 0);
    assert_eq!(steps(&world, frozen).0, 0);
}

#[test]
fn remote_entities_step_too() {
    let mut world = World::new(ClientId(5000));
    let ty = Arc::new(EntityType::new("C", SimulationClass::CarX));
    let id = world
        .spawn_remote(ty, DVec3::ZERO, a3_world::NetworkId::new(2, 9), None)
        .unwrap();

    world.simulate(0.2);

    assert_eq!(steps(&world, id).0, 3);
}

#[test]
fn deletions_take_effect_at_the_end_of_the_frame() {
    let mut world = World::new(ClientId::SERVER);
    let id = create(&mut world, SimulationClass::CarX);
    world.drain_events();
    world.delete(id);

    world.simulate(0.016);

    assert!(world.entity(id).is_none());
    assert!(matches!(
        world.drain_events().as_slice(),
        [WorldEvent::EntityDeleted { entity, .. }] if *entity == id
    ));
}

#[test]
fn time_advances_by_the_frame() {
    let mut world = World::new(ClientId::SERVER);

    world.simulate(0.25);
    world.simulate(0.5);

    assert!((world.time() - 0.75).abs() < EPS);
}

#[test]
fn the_render_position_interpolates_between_the_last_two_steps() {
    let mut world = World::new(ClientId::SERVER);
    let bullet = with_step(&mut world, SimulationClass::ShotBullet, 0.1);
    world
        .entity_mut(bullet)
        .unwrap()
        .set_velocity(DVec3::new(100.0, 0.0, 0.0));

    world.simulate(0.1);
    let e = world.entity(bullet).unwrap();
    assert!((e.position().x - 10.0).abs() < 1e-9);
    assert!((e.render_position().x - 0.0).abs() < 1e-9);

    // Half a step later (no new step yet) the render position is half-way.
    world
        .entity_mut(bullet)
        .unwrap()
        .set_simulation_enabled(false);
    world.simulate(0.05);
    let e = world.entity(bullet).unwrap();
    assert!((e.render_position().x - 5.0).abs() < 1e-9);
}

#[test]
fn entities_get_the_class_state_of_their_family() {
    let mut world = World::new(ClientId::SERVER);
    let man = create(&mut world, SimulationClass::Soldier);
    let car = create(&mut world, SimulationClass::TankX);
    let heli = create(&mut world, SimulationClass::HelicopterRtd);
    let bullet = create(&mut world, SimulationClass::ShotBullet);
    let house = create(&mut world, SimulationClass::House);

    let state = |id| world.entity(id).unwrap().class_state().clone();
    assert!(matches!(state(man), ClassState::Man(_)));
    assert!(matches!(state(car), ClassState::Ground(_)));
    assert!(matches!(state(heli), ClassState::Air(_)));
    assert!(matches!(state(bullet), ClassState::Projectile(_)));
    assert!(matches!(state(house), ClassState::Generic));
}
