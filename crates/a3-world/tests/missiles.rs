//! Missiles and guidance: the motor's `initTime`/`thrustTime` phases, the three drag terms and
//! the guidance law, pinned to the constants of `docs/re/sim-ballistics.md` §3.1.
//!
//! The fixture config is synthetic and self-contained, so the tests run everywhere; the real
//! classes are exercised in `real_data.rs` behind `A3_ROOT`.

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_world::weapons::{AmmoType, LockType};
use a3_world::{
    ClassState, ClientId, Create, EntityId, EntityType, FireRequest, MissileState, MotorPhase,
    ObjectRef, ProjectileState, SimulationClass, World,
};
use glam::DVec3;

/// The `f64` a config number literal reads as: config numbers are stored as `f32`.
fn f32v(v: f32) -> f64 {
    f64::from(v)
}

/// One frame of these tests: the fixture missiles' `simulationStep`, so one frame is exactly one
/// projectile step.
fn h() -> f64 {
    f32v(0.01)
}

/// Gravity of the missile step (`0x140e75130`).
const GRAVITY: f64 = 9.8066;

const CONFIG: &str = r#"
class Mode_SemiAuto { dispersion = 0; };
class CfgAmmo {
    class Default { simulation = ""; };
    // A missile whose motor is out from the third step on: what is left is the plain drag.
    class M_Drag: Default {
        simulation = "shotMissile"; simulationStep = 0.01;
        hit = 1; explosive = 0; caliber = 1; deflecting = 0;
        airFriction = 1; sideAirFriction = 0;
        thrust = 0; thrustTime = 0.01; initTime = 0;
        maxControlRange = 5000;
        timeToLive = 10;
    };
    // The same, with the lateral drag on and no motor at all: lock type 0x40, the unguided model.
    class M_Side: Default {
        simulation = "shotMissile"; simulationStep = 0.01;
        hit = 1; explosive = 0; caliber = 1; deflecting = 0;
        airFriction = 0; sideAirFriction = 1;
        thrust = 0; thrustTime = 0; initTime = 0;
        timeToLive = 10;
    };
    // A guided missile: a motor that waits, burns for a second, then coasts.
    class M_Guided: Default {
        simulation = "shotMissile"; simulationStep = 0.01;
        hit = 20; indirectHit = 5; indirectHitRange = 3; explosive = 0.5;
        caliber = 2; deflecting = 0;
        airFriction = 0; sideAirFriction = 0;
        thrust = 100; thrustTime = 1; initTime = 0.1;
        maneuvrability = 18; trackOversteer = 1; trackLead = 0;
        maxControlRange = 4000; maxSpeed = 500;
        timeToLive = 20;
    };
    // A bullet, to show that a target changes nothing for a shot that does not steer.
    class B_Ball: Default {
        simulation = "shotBullet"; simulationStep = 0.01;
        hit = 8; explosive = 0; caliber = 0.9; deflecting = 0;
        airFriction = 0; coefGravity = 0; typicalSpeed = 800; timeToLive = 6;
    };
};
class CfgMagazines {
    class Missile_Mag { ammo = "M_Drag"; count = 1; initSpeed = 200; };
    class Side_Mag { ammo = "M_Side"; count = 1; initSpeed = 50; };
    class Guided_Mag { ammo = "M_Guided"; count = 1; initSpeed = 100; };
    class Ball_Mag { ammo = "B_Ball"; count = 30; initSpeed = 900; };
};
class CfgWeapons {
    class launcher_F {
        muzzles[] = { "this" };
        magazines[] = { "Missile_Mag", "Side_Mag", "Guided_Mag", "Ball_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { reloadTime = 0.1; };
    };
};
"#;

fn config() -> Arc<ConfigTree> {
    Arc::new(ConfigTree::from_config(&parse_text(CONFIG).unwrap()))
}

/// An empty World with the fixture weapons config, no terrain and no collision world.
fn bare() -> World {
    let mut world = World::new(ClientId::SERVER);
    world.set_config(config());
    world
}

/// A person to shoot from.
fn shooter(world: &mut World, position: DVec3) -> EntityId {
    world
        .create(Create::new(
            Arc::new(EntityType::new("shooter", SimulationClass::Soldier)),
            position,
        ))
        .unwrap()
}

/// A thing to shoot at.
fn target(world: &mut World, position: DVec3) -> EntityId {
    world
        .create(Create::new(
            Arc::new(EntityType::new("target", SimulationClass::Tank)),
            position,
        ))
        .unwrap()
}

/// Fires along `+x` and unwraps.
fn fire(world: &mut World, shooter: EntityId, magazine: &str, from: DVec3) -> EntityId {
    world
        .fire(FireRequest::new(
            shooter,
            "launcher_F",
            magazine,
            from,
            DVec3::X,
        ))
        .unwrap()
}

/// The projectile state of a shot.
fn state(world: &World, shot: EntityId) -> &ProjectileState {
    match world.entity(shot).expect("the shot exists").class_state() {
        ClassState::Projectile(p) => p,
        other => panic!("not a projectile: {other:?}"),
    }
}

/// The missile flight state of a shot.
fn missile(world: &World, shot: EntityId) -> &MissileState {
    state(world, shot).missile.as_ref().expect("a missile")
}

/// The velocity of a shot.
fn velocity(world: &World, shot: EntityId) -> DVec3 {
    world.entity(shot).expect("the shot exists").velocity()
}

/// The ammo type of a fixture class.
fn ammo(world: &mut World, name: &str) -> AmmoType {
    world
        .weapon_bank()
        .expect("a config")
        .ammo(name)
        .expect("a known ammo")
        .as_ref()
        .clone()
}

#[test]
fn a_missile_gets_its_flight_state_and_other_shots_do_not() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "Missile_Mag", DVec3::ZERO);
    let bullet = fire(&mut world, s, "Ball_Mag", DVec3::ZERO);
    assert!(state(&world, shot).missile.is_some());
    assert!(state(&world, bullet).missile.is_none());
    let m = missile(&world, shot);
    assert_eq!(m.phase, MotorPhase::Init);
    assert_eq!(m.init_timer, 0.0);
    assert_eq!(m.thrust_timer, f32v(0.01));
    assert_eq!(m.target, None);
    assert_eq!(m.angular_velocity, DVec3::ZERO);
}

#[test]
fn the_lock_types_are_the_decompiled_table() {
    let mut world = bare();
    // `thrustTime` > 0 with a control range: a guided missile (0x10 in the engine).
    assert_eq!(
        ammo(&mut world, "M_Drag").lock_type(),
        LockType::GuidedMissile
    );
    assert_eq!(
        ammo(&mut world, "M_Side").lock_type(),
        LockType::UnguidedMissile
    );
    assert!(ammo(&mut world, "M_Side").uses_advanced_drag());
    assert_eq!(
        ammo(&mut world, "M_Guided").lock_type(),
        LockType::GuidedMissile
    );
    assert_eq!(ammo(&mut world, "B_Ball").lock_type(), LockType::Bullet);
    assert_eq!(
        ammo(&mut world, "Default").lock_type(),
        LockType::Other,
        "an empty simulation is the engine's default class, which reads 0x80"
    );
    // A control range of at most 10 m is lock type 4.
    let mut short = ammo(&mut world, "M_Drag");
    short.max_control_range = 10.0;
    assert_eq!(short.lock_type(), LockType::ShortRangeMissile);
    short.thrust_time = 0.0;
    assert_eq!(
        short.lock_type(),
        LockType::UnguidedMissile,
        "no thrustTime outranks the range"
    );
}

#[test]
fn the_axial_drag_is_the_decompiled_polynomial() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "Missile_Mag", DVec3::ZERO);
    // Steps 1 and 2 light and burn the (zero-thrust) motor; from step 3 only the drag is left.
    for _ in 0..3 {
        world.simulate(h());
    }
    assert_eq!(missile(&world, shot).phase, MotorPhase::BurntOut);
    let before = velocity(&world, shot);
    world.simulate(h());
    let after = velocity(&world, shot);
    // `(|v|*v*0.01 + v^3*1e-5 + 2v)*airFriction*k*0.1` along the body's z, plus gravity on world
    // y. A purely axial flight has no lateral drag, so nothing turns the body and the literals
    // are pinned exactly.
    let v = before.x;
    let drag = (v.abs() * v * 0.01 + v * v * v * 1e-5 + 2.0 * v) * 1.0 * 0.1;
    assert!(
        (after.x - (before.x - drag * h())).abs() < 1e-9,
        "axial drag: {} vs {}",
        after.x,
        before.x - drag * h()
    );
    assert!(
        (after.y - (before.y - GRAVITY * h())).abs() < 1e-8,
        "gravity: {} vs {}",
        after.y,
        before.y - GRAVITY * h()
    );
    assert!(after.z.abs() < 1e-9, "no lateral velocity: {}", after.z);
}

#[test]
fn the_lateral_drag_and_the_unguided_model_are_the_decompiled_terms() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "Side_Mag", DVec3::ZERO);
    // A shot fired along +x has the body's z along +x, so a world velocity along -z is the
    // body's +x: a pure lateral flight.
    let lateral = 50.0;
    world
        .entity_mut(shot)
        .unwrap()
        .set_velocity(DVec3::new(0.0, 0.0, -lateral));
    world.simulate(h());
    let after = velocity(&world, shot);
    // `((|v|*v + v)*10 + v^3*0.0005)*sideAirFriction*k*0.1` as a deceleration, plus the unguided
    // model's `(v*-0.005 - |v|*v*0.00033)*k` on the acceleration. The same lateral drag commands
    // the body's fin-stability turn, which scales the component by `cos(w*dt)` (7e-4 here), so
    // the literal is pinned to that much and no further.
    let drag = ((lateral * lateral + lateral) * 10.0 + lateral.powi(3) * 0.0005) * 1.0 * 0.1;
    let extra = lateral * -0.005 - lateral * lateral * 0.00033;
    let expected = -lateral + drag * h() - extra * h();
    assert!(
        (after.z - expected).abs() < 1e-3,
        "lateral drag: {} vs {expected}",
        after.z
    );
    assert!(
        (after.y - (-GRAVITY * h())).abs() < 1e-8,
        "gravity: {}",
        after.y
    );
    assert!(after.x.abs() < 0.5, "no axial thrust: {}", after.x);
    // The unguided model also feeds the lateral drag into the body's turn
    // (`guide.y += 0.03*drag.x`), which is how a finned rocket stabilises itself.
    assert!(
        missile(&world, shot).angular_velocity.y.abs() > 0.0,
        "the fin-stability turn: {:?}",
        missile(&world, shot).angular_velocity
    );
    assert_eq!(missile(&world, shot).guidance, [0.0, 0.0], "no target to steer at");
}

#[test]
fn the_motor_waits_out_init_time_and_ramps_its_thrust_down() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "Guided_Mag", DVec3::ZERO);
    let dt = h();
    // `initTime` 0.1: ten steps of 0.01 leave the phase on `Init`, the eleventh lights the motor
    // and only the twelfth burns.
    assert_eq!(missile(&world, shot).phase, MotorPhase::Init);
    for step in 1..=10 {
        world.simulate(dt);
        assert_eq!(
            missile(&world, shot).phase,
            MotorPhase::Init,
            "step {step} is still inside initTime"
        );
    }
    let coasting = velocity(&world, shot).x;
    world.simulate(dt);
    assert_eq!(missile(&world, shot).phase, MotorPhase::Burning);
    assert!(
        (velocity(&world, shot).x - coasting).abs() < 1e-9,
        "the step that lights the motor does not burn"
    );
    // Burn, counting the impulse the ramp gives: full thrust until a quarter of `thrustTime` is
    // left, then linear down to zero. The ramp reads the timer the step leaves behind, and a step
    // that would push it below zero does not burn at all.
    let (thrust, thrust_time) = (100.0, 1.0);
    let mut impulse = 0.0;
    for _ in 0..200 {
        if missile(&world, shot).phase == MotorPhase::BurntOut {
            break;
        }
        let burning = missile(&world, shot).phase == MotorPhase::Burning;
        world.simulate(dt);
        let left = missile(&world, shot).thrust_timer;
        if burning && missile(&world, shot).phase == MotorPhase::Burning {
            impulse += (left * 4.0 / thrust_time).min(1.0) * thrust * dt;
        }
    }
    assert_eq!(missile(&world, shot).phase, MotorPhase::BurntOut);
    let burnt_out = velocity(&world, shot).x;
    assert!(
        (burnt_out - (coasting + impulse)).abs() < 1e-6,
        "burn: {burnt_out} vs {}",
        coasting + impulse
    );
    // `thrustTime` 1 s of full thrust then a ramp: 0.875 of the ideal impulse.
    assert!((impulse - 0.875 * thrust * thrust_time).abs() < thrust * dt);
    // Burnt out, the speed stops growing.
    for _ in 0..10 {
        world.simulate(dt);
    }
    assert!((velocity(&world, shot).x - burnt_out).abs() < 1e-9);
}

#[test]
fn a_guided_missile_turns_onto_its_target() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    // Off to the side: the missile has to turn about 17 degrees to fly at it.
    let t = target(&mut world, DVec3::new(1000.0, 300.0, 0.0));
    let shot = world
        .fire(
            FireRequest::new(s, "launcher_F", "Guided_Mag", DVec3::ZERO, DVec3::X)
                .target(ObjectRef::Entity(t)),
        )
        .unwrap();
    assert_eq!(missile(&world, shot).target, Some(ObjectRef::Entity(t)));
    let mut closest = f64::MAX;
    let mut turned = false;
    for _ in 0..600 {
        world.simulate(h());
        if world.entity(shot).is_none() {
            break;
        }
        let position = world.entity(shot).unwrap().position();
        closest = closest.min((position - DVec3::new(1000.0, 300.0, 0.0)).length());
        // The launch direction is +x; steering towards the target means climbing.
        turned |= velocity(&world, shot).y > 1.0;
    }
    assert!(turned, "the guidance has to turn the missile");
    assert!(
        closest < 20.0,
        "the missile should fly into the target, closest approach {closest} m"
    );
}

#[test]
fn a_target_does_not_change_a_shot_that_does_not_steer() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let t = target(&mut world, DVec3::new(500.0, 100.0, 0.0));
    let aimed = world
        .fire(
            FireRequest::new(s, "launcher_F", "Ball_Mag", DVec3::ZERO, DVec3::X)
                .target(ObjectRef::Entity(t)),
        )
        .unwrap();
    let plain = fire(&mut world, s, "Ball_Mag", DVec3::ZERO);
    for _ in 0..10 {
        world.simulate(h());
    }
    assert_eq!(
        velocity(&world, aimed),
        velocity(&world, plain),
        "a bullet ignores the target"
    );
    assert_eq!(state(&world, aimed).missile, None);
}

#[test]
fn a_missile_without_a_target_flies_straight() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "Guided_Mag", DVec3::ZERO);
    for _ in 0..50 {
        world.simulate(h());
    }
    let m = missile(&world, shot);
    assert_eq!(m.guidance, [0.0, 0.0], "nothing to steer at");
    assert_eq!(m.angular_velocity, DVec3::ZERO, "nothing turns the body");
    let v = velocity(&world, shot);
    assert!(v.z.abs() < 1e-9, "no lateral velocity: {v:?}");
    assert!(v.x > 100.0, "the motor still accelerates along +x: {v:?}");
    assert!(v.y < 0.0, "gravity still pulls it down: {v:?}");
}
