//! Smoke tests against the shipped config (`A3_ROOT`, skipped without it): a real missile class
//! has to load, fly, hit and explode. Synthetic fixtures stay in `missiles.rs`; here the *config*
//! is the game's, so the parameters the loader reads are the real ones.

mod common;

use std::sync::Arc;

use a3_config::Value;
use a3_physics::BodyKind;
use a3_world::weapons::LockType;
use a3_world::{
    ClientId, Create, EntityType, FireRequest, ObjectRef, SimulationClass, World, WorldEvent,
};
use glam::{DQuat, DVec3};

/// The shipped config, or `None` without `A3_ROOT`.
fn game(root: &std::ffi::OsString) -> Option<Arc<a3_config::ConfigTree>> {
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    Some(data.config.clone())
}

/// The first `CfgWeapons` class of the shipped config that fires a missile with a motor, as
/// `(weapon, magazine, ammo)`.
fn a_missile_weapon(world: &mut World) -> Option<(String, String, String)> {
    // Every (weapon, magazine, ammo) triple of the config that names a magazine, collected before
    // the parameter cache is asked so the two borrows do not overlap.
    let candidates: Vec<(String, String, String)> = {
        let config = world.config()?;
        let mut out = Vec::new();
        for class in config.root().get("CfgWeapons").entries() {
            if !class.is_class() {
                continue;
            }
            for magazine in class.get("magazines").array().iter().filter_map(|v| match v {
                Value::String(s) | Value::Expression(s) => Some(s.clone()),
                _ => None,
            }) {
                let ammo = config
                    .root()
                    .get("CfgMagazines")
                    .get(&magazine)
                    .get("ammo")
                    .text();
                if !ammo.is_empty() {
                    out.push((class.name().to_owned(), magazine, ammo));
                }
            }
        }
        out
    };
    for (weapon, magazine, ammo) in candidates {
        if let Ok(ammo) = world.weapon_bank().unwrap().ammo(&ammo) {
            if ammo.lock_type() == LockType::GuidedMissile && ammo.thrust > 0.0 {
                return Some((weapon, magazine, ammo.name.clone()));
            }
        }
    }
    None
}

/// The first shipped thrown item: `(weapon, magazine, ammo)` for a `CfgAmmo` class with the
/// `shotGrenade` simulation and a magazine that launches it below 30 m/s (`sim-weapons.md` §2.1).
/// The weapon is the one carrying the magazine, or a `Throw` class, or the first `CfgWeapons`
/// class when nothing lists it (a hand grenade is thrown from the hand, not fired from a weapon).
fn a_grenade(world: &mut World) -> Option<(String, String, String)> {
    let (magazines, any_weapon): (Vec<(String, String)>, Option<String>) = {
        let config = world.config()?;
        let mut magazines = Vec::new();
        for class in config.root().get("CfgAmmo").entries() {
            if !class.is_class() || class.get("simulation").text() != "shotGrenade" {
                continue;
            }
            let ammo = class.name().to_owned();
            for magazine in config.root().get("CfgMagazines").entries() {
                if magazine.is_class()
                    && magazine.get("ammo").text() == ammo
                    && magazine.get("initSpeed").number() < 30.0
                {
                    magazines.push((magazine.name().to_owned(), ammo.clone()));
                }
            }
        }
        let weapons = config.root().get("CfgWeapons");
        let any_weapon = weapons
            .get("Throw")
            .is_class()
            .then(|| "Throw".to_owned())
            .or_else(|| {
                weapons
                    .entries()
                    .iter()
                    .find(|c| c.is_class())
                    .map(|c| c.name().to_owned())
            });
        (magazines, any_weapon)
    };
    let any_weapon = any_weapon?;
    let (magazine, ammo) = magazines.into_iter().next()?;
    let carrier = {
        let config = world.config()?;
        config
            .root()
            .get("CfgWeapons")
            .entries()
            .iter()
            .find(|c| {
                c.is_class()
                    && c.get("magazines").array().iter().any(|v| match v {
                        Value::String(s) | Value::Expression(s) => {
                            s.eq_ignore_ascii_case(&magazine)
                        }
                        _ => false,
                    })
            })
            .map(|c| c.name().to_owned())
            .unwrap_or(any_weapon)
    };
    Some((carrier, magazine, ammo))
}

/// A one-box Fire Geometry body for an Object, so a shot can hit it.
fn add_body(world: &mut World, model: &str, entity: a3_world::EntityId, position: DVec3) {
    let collision = world
        .collision_mut()
        .unwrap()
        .models()
        .get(model)
        .unwrap_or_else(|| panic!("no model {model}"));
    world.collision_mut().unwrap().add_body(
        ObjectRef::Entity(entity).to_body_key(),
        collision,
        position,
        DQuat::IDENTITY,
        BodyKind::Kinematic,
    );
}

#[test]
fn a_shipped_missile_loads_flies_and_explodes_on_its_target() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some(config) = game(&root) else {
        return;
    };
    // A flat terrain well below the flight so the missile only meets the target's box.
    let mut world = common::scene_with_models(&[], &[("box.p3d", common::box_model([6.0, 6.0, 6.0], "plank.bisurf"))]);
    world.set_config(config);
    let Some((weapon, magazine, ammo)) = a_missile_weapon(&mut world) else {
        panic!("no missile weapon in the shipped config");
    };
    eprintln!("missile smoke: {weapon} / {magazine} / {ammo}");

    let shooter = world
        .create(Create::new(
            Arc::new(EntityType::new("shooter", SimulationClass::Soldier)),
            DVec3::ZERO,
        ))
        .unwrap();
    let target = world
        .create(Create::new(
            Arc::new(EntityType::new("target", SimulationClass::Tank)),
            DVec3::new(800.0, 0.0, 0.0),
        ))
        .unwrap();
    add_body(&mut world, "box.p3d", target, DVec3::new(800.0, 0.0, 0.0));

    let shot = world
        .fire(
            FireRequest::new(shooter, &weapon, &magazine, DVec3::ZERO, DVec3::X)
                .target(ObjectRef::Entity(target)),
        )
        .unwrap();
    let state = match world.entity(shot).unwrap().class_state() {
        a3_world::ClassState::Projectile(p) => p,
        other => panic!("not a projectile: {other:?}"),
    };
    let missile = state.missile.as_ref().expect("a missile state");
    assert_eq!(missile.phase, a3_world::MotorPhase::Init);
    let launch_speed = world.entity(shot).unwrap().velocity().length();

    let mut fastest = launch_speed;
    let mut exploded = None;
    for _ in 0..1000 {
        world.simulate(0.02);
        if let Some(e) = world.entity(shot) {
            fastest = fastest.max(e.velocity().length());
        }
        for event in world.drain_events() {
            if let WorldEvent::Exploded { position, .. } = event {
                exploded = Some(position);
            }
        }
        if exploded.is_some() {
            break;
        }
    }
    let position = exploded.expect("the missile has to detonate");
    eprintln!(
        "detonated at {position:?} after reaching {fastest:.0} m/s (launched at {launch_speed:.0})"
    );
    // A guided missile of the shipped config has to accelerate on its motor and reach its target.
    // Their magazines launch them at rest (`initSpeed` 0): the motor is the whole velocity.
    assert!(
        fastest > launch_speed + 50.0,
        "the motor accelerates the missile: {fastest} vs {launch_speed}"
    );
    assert!(
        (position - DVec3::new(800.0, 0.0, 0.0)).length() < 50.0,
        "detonated {position:?}, not at the target"
    );
    assert!(
        world.entity(target).unwrap().damage() > 0.0,
        "the blast has to damage the target"
    );
}

/// Every shipped `CfgAmmo` class with a missile simulation loads into an `AmmoType` whose lock
/// type the decompiled table agrees with.
#[test]
fn every_shipped_missile_class_reads_a_lock_type() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some(config) = game(&root) else {
        return;
    };
    let mut world = World::new(ClientId::SERVER);
    world.set_config(config.clone());
    let mut missiles = 0;
    let mut guided = 0;
    for class in config.root().get("CfgAmmo").entries() {
        if !class.is_class() || class.get("simulation").text() != "shotMissile" {
            continue;
        }
        let ammo = world
            .weapon_bank()
            .unwrap()
            .ammo(class.name())
            .unwrap_or_else(|e| panic!("{}: {e}", class.name()));
        missiles += 1;
        match ammo.lock_type() {
            LockType::GuidedMissile => {
                guided += 1;
                assert!(ammo.thrust > 0.0, "{}: a guided missile has a motor", ammo.name);
                assert!(
                    ammo.thrust_time > 0.0,
                    "{}: a guided missile burns",
                    ammo.name
                );
                assert!(
                    ammo.max_control_range > 10.0,
                    "{}: lock type 4 is at most 10 m",
                    ammo.name
                );
            }
            LockType::UnguidedMissile => {
                assert!(
                    ammo.thrust_time <= 0.0,
                    "{}: 0x40 is a missile with no thrustTime",
                    ammo.name
                );
            }
            LockType::ShortRangeMissile => {
                assert!(
                    ammo.max_control_range <= 10.0 && ammo.thrust_time > 0.0,
                    "{}: lock type 4",
                    ammo.name
                );
            }
            other => panic!("{}: a missile with lock type {other:?}", ammo.name),
        }
    }
    eprintln!("{missiles} shipped missiles, {guided} of them guided");
    assert!(missiles > 0, "the shipped config has missile classes");
    assert!(guided > 0, "and guided ones among them");
}

/// A shipped grenade is thrown at its magazine's `initSpeed`, lands, and goes off when its fuse
/// runs out, blasting what is next to it (`sim-weapons.md` §2.1, `sim-ballistics.md` §4 and §6).
#[test]
fn a_shipped_grenade_is_thrown_runs_its_fuse_and_blasts() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some(config) = game(&root) else {
        return;
    };
    let mut world = common::scene(&[]);
    world.set_config(config);
    let Some((weapon, magazine, ammo)) = a_grenade(&mut world) else {
        panic!("no thrown grenade in the shipped config");
    };
    let fuse = world
        .weapon_bank()
        .unwrap()
        .ammo(&ammo)
        .unwrap()
        .explosion_time;
    let init_speed = world
        .weapon_bank()
        .unwrap()
        .magazine(&magazine)
        .unwrap()
        .init_speed;
    eprintln!("grenade smoke: {weapon} / {magazine} / {ammo}: initSpeed {init_speed}, fuse {fuse}");
    assert!(fuse > 0.0, "{ammo} has to have a fuse");
    assert!(
        (0.5..30.0).contains(&init_speed),
        "{ammo} is thrown at {init_speed} m/s"
    );

    let thrower = world
        .create(Create::new(
            Arc::new(EntityType::new("thrower", SimulationClass::Soldier)),
            DVec3::new(30.0, -4.0, 30.0),
        ))
        .unwrap();
    // The `Throw` weapon's own muzzle, which is not the weapon body.
    let muzzle = {
        let bank = world.weapon_bank().unwrap();
        let weapon_type = bank
            .weapon(&weapon)
            .unwrap_or_else(|e| panic!("{weapon}: {e}"));
        weapon_type
            .muzzles
            .first()
            .map(|m| m.name.clone())
            .expect("a muzzle")
    };
    let shot = world
        .fire(
            FireRequest::new(
                thrower,
                &weapon,
                &magazine,
                DVec3::new(30.0, -4.0, 30.0),
                // A 45-degree throw: it arcs and comes down inside the fixture terrain.
                DVec3::new(0.7, 0.7, 0.0),
            )
            .muzzle(muzzle)
            .throw_intensity(1.0),
        )
        .unwrap();
    let speed = world.entity(shot).unwrap().velocity().length();
    assert!(
        (speed - init_speed).abs() < 1e-6,
        "a full throw is the magazine's initSpeed: {speed} vs {init_speed}"
    );

    // Fly until it settles, then put the blast's victim where it came to rest.
    let mut resting = None;
    let mut victim = None;
    let mut exploded_at = None;
    for step in 0..600 {
        world.simulate(0.02);
        let elapsed = (step + 1) as f64 * 0.02;
        if victim.is_none()
            && let Some(entity) = world.entity(shot)
            && entity.velocity().length() < 0.5
        {
            let position = entity.position();
            resting = Some(position);
            victim = Some(
                world
                    .create(Create::new(
                        Arc::new(EntityType::new("victim", SimulationClass::Tank)),
                        position + DVec3::new(1.0, 0.0, 0.0),
                    ))
                    .unwrap(),
            );
        }
        for event in world.drain_events() {
            if let WorldEvent::Exploded { position, .. } = event {
                exploded_at = Some((elapsed, position));
            }
        }
        if exploded_at.is_some() {
            break;
        }
    }
    let (elapsed, position) = exploded_at.expect("the fuse has to run out");
    eprintln!("grenade went off after {elapsed:.2} s at {position:?}, resting {resting:?}");
    assert!(
        (elapsed - fuse).abs() < 0.1,
        "the fuse is {fuse} s, it went off after {elapsed}"
    );
    let victim = victim.expect("the grenade has to settle before its fuse runs out");
    assert!(
        world.entity(victim).unwrap().damage() > 0.0,
        "the blast has to damage what is next to it"
    );
}
