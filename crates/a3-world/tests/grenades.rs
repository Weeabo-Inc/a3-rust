//! Thrown grenades: the throw of `sim-weapons.md` §2.1 (a shell magazine whose `initSpeed` is
//! below 30 m/s), their fuse, and the rolling branch of `sim-ballistics.md` §4.1 that a fused shot
//! takes when it lands grazing the surface.
//!
//! The fixture config is synthetic; the real classes are exercised in `weapons_real_data.rs`.

mod common;

use std::sync::Arc;

use a3_world::{
    ClassState, Create, EntityId, EntityType, FireRequest, ProjectileState, SimulationClass, World,
    WorldEvent,
};
use glam::DVec3;

/// One frame of these tests: the fixture ammo's `simulationStep`.
fn h() -> f64 {
    f64::from(0.02_f32)
}

/// Gravity of the projectile step (`9.8066`).
const GRAVITY: f64 = 9.8066;

const CONFIG: &str = r#"
class Mode_SemiAuto { dispersion = 0; };
class CfgAmmo {
    class Default { simulation = ""; };
    // A thrown grenade: a five-second fuse, a blast, no motor.
    class G_Hand: Default {
        simulation = "shotGrenade"; simulationStep = 0.02;
        hit = 1; indirectHit = 12; indirectHitRange = 6; explosive = 1;
        caliber = 0.5; deflecting = 0;
        airFriction = -0.001; coefGravity = 1; typicalSpeed = 30;
        timeToLive = 0; explosionTime = 5; fuseDistance = 0;
    };
    // A shell with a long fuse, for the rolling branch: it must slide, not explode.
    class G_Fused: Default {
        simulation = "shotShell"; simulationStep = 0.02;
        hit = 10; indirectHit = 5; indirectHitRange = 3; explosive = 0.5;
        caliber = 1; deflecting = 0;
        airFriction = 0; coefGravity = 1; typicalSpeed = 300;
        timeToLive = 20; explosionTime = 10; fuseDistance = 0;
    };
    // The same shell with a contact fuse: it stops and explodes where it lands.
    class G_Contact: Default {
        simulation = "shotShell"; simulationStep = 0.02;
        hit = 10; indirectHit = 5; indirectHitRange = 3; explosive = 0.5;
        caliber = 1; deflecting = 0;
        airFriction = 0; coefGravity = 1; typicalSpeed = 300;
        timeToLive = 20; explosionTime = 0; fuseDistance = 0;
    };
};
class CfgMagazines {
    // A thrown grenade: `initSpeed` below 30 m/s (`sim-weapons.md` §2.1).
    class Hand_Mag: Default { ammo = "G_Hand"; count = 1; initSpeed = 20;
        maxThrowHoldTime = 1; minThrowIntensityCoef = 0.5; maxThrowIntensityCoef = 1; };
    // The same ammo fired from a launcher at 100 m/s: not a throw.
    class Launched_Mag { ammo = "G_Hand"; count = 1; initSpeed = 100; };
    class Fused_Mag { ammo = "G_Fused"; count = 1; initSpeed = 30; };
    class Contact_Mag { ammo = "G_Contact"; count = 1; initSpeed = 60; };
};
class CfgWeapons {
    class thrower_F {
        muzzles[] = { "this" };
        magazines[] = { "Hand_Mag", "Launched_Mag", "Fused_Mag", "Contact_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { reloadTime = 0.1; };
    };
};
"#;

/// A World with the fixture config and the flat terrain of `common::scene` (its ground is at
/// -5 m).
fn world() -> World {
    let mut world = common::scene(&[]);
    world.set_config(Arc::new(a3_config::ConfigTree::from_config(
        &a3_config::parse_text(CONFIG).unwrap(),
    )));
    world
}

/// A person to throw from.
fn thrower(world: &mut World, position: DVec3) -> EntityId {
    world
        .create(Create::new(
            Arc::new(EntityType::new("thrower", SimulationClass::Soldier)),
            position,
        ))
        .unwrap()
}

/// Throws `magazine` along `direction` from `from`, at `intensity`, and unwraps.
fn throw(
    world: &mut World,
    thrower: EntityId,
    magazine: &str,
    from: DVec3,
    direction: DVec3,
    intensity: f64,
) -> EntityId {
    world
        .fire(
            FireRequest::new(thrower, "thrower_F", magazine, from, direction)
                .throw_intensity(intensity),
        )
        .unwrap()
}

/// The projectile state of a shot.
fn state(world: &World, shot: EntityId) -> &ProjectileState {
    match world.entity(shot).expect("the shot exists").class_state() {
        ClassState::Projectile(p) => p,
        other => panic!("not a projectile: {other:?}"),
    }
}

/// The velocity of a shot.
fn velocity(world: &World, shot: EntityId) -> DVec3 {
    world.entity(shot).expect("the shot exists").velocity()
}

/// The position of a shot.
fn position(world: &World, shot: EntityId) -> DVec3 {
    world.entity(shot).expect("the shot exists").position()
}

#[test]
fn a_thrown_grenade_flies_at_its_magazine_speed_times_the_hold_intensity() {
    let mut world = world();
    let s = thrower(&mut world, DVec3::ZERO);
    let full = throw(&mut world, s, "Hand_Mag", DVec3::ZERO, DVec3::X, 1.0);
    let half = throw(&mut world, s, "Hand_Mag", DVec3::ZERO, DVec3::X, 0.5);
    assert!((velocity(&world, full).length() - 20.0).abs() < 1e-9);
    assert!((velocity(&world, half).length() - 10.0).abs() < 1e-9);
    // The fuse is what makes a grenade a grenade: it runs from the throw.
    assert_eq!(state(&world, full).explosion_timer, 5.0);
    assert_eq!(state(&world, full).fuse_distance, 0.0);
}

#[test]
fn a_grenade_fired_from_a_launcher_is_not_a_throw() {
    let mut world = world();
    let s = thrower(&mut world, DVec3::ZERO);
    // The same ammo at 100 m/s is a shell: the hold intensity does not scale it.
    let shot = throw(&mut world, s, "Launched_Mag", DVec3::ZERO, DVec3::X, 0.5);
    assert!((velocity(&world, shot).length() - 100.0).abs() < 1e-9);
}

#[test]
fn a_grenade_lands_slides_and_explodes_when_its_fuse_runs_out() {
    let mut world = world();
    let s = thrower(&mut world, DVec3::ZERO);
    // Thrown almost flat, so it lands grazing the ground at -5 m.
    let shot = throw(
        &mut world,
        s,
        "Hand_Mag",
        DVec3::new(0.0, -4.0, 0.0),
        DVec3::new(1.0, -0.05, 0.0),
        1.0,
    );
    let mut exploded_at = None;
    let mut landed: Option<DVec3> = None;
    for step in 0..400 {
        world.simulate(h());
        for event in world.drain_events() {
            if let WorldEvent::Exploded { position, .. } = event {
                exploded_at = Some((step, position));
            }
        }
        if landed.is_none() && world.entity(shot).is_some() {
            let position = position(&world, shot);
            if position.y <= -4.9 {
                landed = Some(position);
                // A grenade does not go off on impact: the fuse is still running.
                assert!(exploded_at.is_none(), "exploded on impact at step {step}");
            }
        }
        if exploded_at.is_some() {
            break;
        }
    }
    let landed = landed.expect("the grenade has to land");
    let (step, position) = exploded_at.expect("the fuse has to run out");
    // Five seconds of fuse at 0.02 s steps.
    let seconds = step as f64 * h();
    assert!(
        (seconds - 5.0).abs() < 0.1,
        "the fuse runs five seconds, went off after {seconds}"
    );
    // It slid on after landing, and went off near where it came to rest.
    assert!(
        (position - landed).length() < 15.0,
        "exploded {position:?} far from where it landed, {landed:?}"
    );
}

#[test]
fn a_fused_shell_that_lands_grazing_slides_instead_of_stopping() {
    let mut world = world();
    let s = thrower(&mut world, DVec3::ZERO);
    let shot = throw(
        &mut world,
        s,
        "Fused_Mag",
        DVec3::new(0.0, -4.95, 0.0),
        DVec3::new(1.0, -0.033_333, 0.0),
        1.0,
    );
    // Fly until it meets the ground.
    for _ in 0..200 {
        world.simulate(h());
        if position(&world, shot).y <= -4.99 {
            break;
        }
    }
    let landed = position(&world, shot);
    assert!(landed.y > -5.2, "it landed: {landed:?}");
    let before = velocity(&world, shot);
    world.simulate(h());
    let after = velocity(&world, shot);
    assert!(
        after.y.abs() < 0.2,
        "the normal velocity goes: {before:?} -> {after:?}"
    );
    // The slide loses `0.1*v + 3` m/s per second, so about 6 m/s² at 30 m/s.
    let loss = before.x - after.x;
    let expected = (before.x * 0.1 + 3.0) * h();
    assert!(
        (loss - expected).abs() < 0.05,
        "the rolling friction: lost {loss}, expected {expected}"
    );
    // And it keeps going rather than stopping dead on the first touch.
    assert!(world.entity(shot).is_some());
    assert!(position(&world, shot).x - landed.x > 0.1, "it slid on");
    // Gravity keeps the shot on the surface without letting it sink.
    assert!(
        position(&world, shot).y >= -5.2,
        "still on the surface: {:?}",
        position(&world, shot)
    );
}

#[test]
fn a_contact_shell_stops_and_explodes_where_it_lands() {
    let mut world = world();
    let s = thrower(&mut world, DVec3::ZERO);
    // Steep enough that even a grazing rule would not catch it, and no fuse: it detonates.
    let shot = throw(
        &mut world,
        s,
        "Contact_Mag",
        DVec3::new(0.0, -4.0, 0.0),
        DVec3::new(1.0, -0.5, 0.0),
        1.0,
    );
    let mut exploded = false;
    for _ in 0..100 {
        world.simulate(h());
        for event in world.drain_events() {
            if let WorldEvent::Exploded { .. } = event {
                exploded = true;
            }
        }
        if exploded {
            break;
        }
    }
    assert!(exploded, "a contact shell explodes on impact");
    assert!(world.entity(shot).is_none(), "and is consumed");
    let _ = GRAVITY;
}