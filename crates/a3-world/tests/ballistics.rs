//! Firing and ballistics: `World::fire`, the projectile step (simulation step, time to live,
//! air friction, gravity), impact resolution (ricochet, penetration, stop), explosions, and the
//! `Fired` / `Hit` / `Exploded` World events damage will be applied from.
//!
//! Sources: `docs/re/sim-ballistics.md` §3 (the flight loop and its timers), §4 (the impact
//! order), §5 (the direct hit value), §6 (the explosion falloff). The fixtures are synthetic
//! (`tests/common`); `firing_a_real_weapon_from_the_shipped_config` needs `A3_ROOT`.

mod common;

use std::sync::Arc;

use a3_config::Value;
use a3_physics::{BodyKind, GRAVITY};
use a3_world::{
    ClassState, ClientId, Create, EntityId, EntityType, Error, FireRequest, ListKind, Locality,
    Near, ObjectRef, ProjectileState, SimulationClass, World, WorldEvent,
};
use glam::{DQuat, DVec3};

/// One frame of these tests: the fixture ammo's `simulationStep`, so one frame is exactly one
/// projectile step (`simulate` runs whole steps).
fn h() -> f64 {
    f64::from(0.05_f32)
}

/// The `f64` a config number literal reads as: config numbers are stored as `f32`.
fn f32v(v: f32) -> f64 {
    f64::from(v)
}

/// An empty World with the weapons config: firing and flight, no collision world.
fn bare() -> World {
    let mut world = World::new(ClientId::SERVER);
    world.set_config(common::config());
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

/// Fires along `+x` and unwraps; the error paths live in `firing_errors_are_reported`.
fn fire(
    world: &mut World,
    shooter: EntityId,
    weapon: &str,
    magazine: &str,
    from: DVec3,
) -> EntityId {
    world
        .fire(FireRequest::new(shooter, weapon, magazine, from, DVec3::X))
        .unwrap()
}

/// Gives an Entity a Kinematic body of the model `model` (the tests stand in for the World's own
/// body sync): `add_body(ObjectRef::Entity(id).to_body_key(), ..)`.
fn add_body(world: &mut World, model: &str, entity: EntityId, position: DVec3) {
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

/// The one Static object at `position`.
fn static_at(world: &World, position: DVec3) -> ObjectRef {
    let near = world.objects_near(position, 0.5, Near::Statics);
    assert_eq!(near.len(), 1, "one Static at {position}: {near:?}");
    near[0].0
}

/// The `Hit` events of a drained event list, as `(target, speed_in, speed_out, value)`.
fn hits(events: &[WorldEvent]) -> Vec<(ObjectRef, f64, f64, f64)> {
    events
        .iter()
        .filter_map(|e| match e {
            WorldEvent::Hit {
                target,
                speed_in,
                speed_out,
                value,
                ..
            } => Some((*target, *speed_in, *speed_out, *value)),
            _ => None,
        })
        .collect()
}

/// The `Exploded` event of a drained event list.
fn exploded(events: &[WorldEvent]) -> Option<(DVec3, f64)> {
    events.iter().find_map(|e| match e {
        WorldEvent::Exploded {
            position, radius, ..
        } => Some((*position, *radius)),
        _ => None,
    })
}

/// The projectile state of a shot.
fn state(world: &World, shot: EntityId) -> &ProjectileState {
    match world.entity(shot).expect("the shot exists").class_state() {
        ClassState::Projectile(p) => p,
        other => panic!("not a projectile: {other:?}"),
    }
}

#[test]
fn firing_creates_a_local_projectile_entity() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::new(1.0, 0.0, 2.0));
    let shot = fire(
        &mut world,
        s,
        "rifle_F",
        "Ball_Mag",
        DVec3::new(1.0, 0.0, 2.0),
    );

    let events = world.drain_events();
    assert!(
        events.contains(&WorldEvent::Fired {
            shot,
            shooter: s,
            weapon: "rifle_F".to_owned(),
        }),
        "{events:?}"
    );

    let e = world.entity(shot).expect("the shot exists");
    assert_eq!(e.list(), ListKind::Projectiles);
    assert_eq!(e.locality(), Locality::Local);
    assert_eq!(e.position(), DVec3::new(1.0, 0.0, 2.0));
    assert_eq!(e.velocity(), DVec3::new(900.0, 0.0, 0.0));

    let state = state(&world, shot);
    assert_eq!(state.ammo.as_ref().map(|a| a.name.as_str()), Some("B_Ball"));
    assert_eq!(state.shooter, Some(s));
    // `timeToLive` runs; `explosionTime = 0` means the explosion timer never fires.
    assert!((state.time_to_live - 6.0).abs() < 1e-9);
    assert!(state.explosion_timer.is_infinite());
}

#[test]
fn a_shot_falls_under_gravity() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "rifle_F", "Grav_Mag", DVec3::ZERO);

    // The velocity is integrated after the move, so the first step flies straight.
    world.simulate(h());
    let e = world.entity(shot).unwrap();
    assert_eq!(e.position().y, 0.0);
    assert!(
        (e.velocity().y + GRAVITY * h()).abs() < 1e-9,
        "{:?}",
        e.velocity()
    );

    world.simulate(h());
    world.simulate(h());
    world.simulate(h());
    let e = world.entity(shot).unwrap();
    // y after n steps is −g·h²·(0 + 1 + … + (n − 1)) = −6 g h² after four.
    let expected = -GRAVITY * h() * h() * 6.0;
    assert!(
        (e.position().y - expected).abs() < 1e-9,
        "y {}",
        e.position().y
    );
    assert!((e.velocity().y + 4.0 * GRAVITY * h()).abs() < 1e-9);
    assert!((e.position().x - 4.0 * 900.0 * h()).abs() < 1e-6);
}

#[test]
fn air_friction_slows_a_shot() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "rifle_F", "Ball_Mag", DVec3::ZERO);
    world.simulate(h());

    let k = f32v(-0.0012);
    let e = world.entity(shot).unwrap();
    assert!((e.position().x - 900.0 * h()).abs() < 1e-9);
    // a = k·|v|·v − (0, g·coefGravity, 0); dv = a·dt.
    let v = e.velocity();
    assert!(
        (v.x - (900.0 + k * 900.0 * 900.0 * h())).abs() < 1e-9,
        "v.x {}",
        v.x
    );
    assert!((v.y + GRAVITY * h()).abs() < 1e-9, "v.y {}", v.y);
}

#[test]
fn a_shot_never_reverses_and_never_goes_nan() {
    // Strong drag on a 1 m/s shot: the integration alone would reverse it.
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "rifle_F", "Drag_Mag", DVec3::ZERO);
    world.simulate(h());
    let x1 = world.entity(shot).unwrap().position().x;
    world.simulate(h());

    let e = world.entity(shot).unwrap();
    let v = e.velocity();
    assert!(v.is_finite(), "v {v:?}");
    assert!(v.x >= 0.0 && v.x < 1e-9, "it does not turn around: {v:?}");
    assert!(e.position().is_finite());
    assert!((e.position().x - x1).abs() < 1e-12, "it does not move on");

    // Fired at rest: |v| = |dv| = 0, the degenerate clamp must not make it NaN.
    let origin = DVec3::new(3.0, 4.0, 5.0);
    let shot = fire(&mut world, s, "rifle_F", "Zero_Mag", origin);
    assert_eq!(world.entity(shot).unwrap().velocity(), DVec3::ZERO);
    world.simulate(h());
    world.simulate(h());
    let e = world.entity(shot).unwrap();
    assert_eq!(e.position(), origin, "a shot at rest does not move");
    assert!(e.velocity().is_finite() && e.velocity() == DVec3::ZERO);
}

#[test]
fn dispersion_spreads_shots_within_the_cone() {
    let mut world = bare();
    world.set_random_seed(7);
    let s = shooter(&mut world, DVec3::ZERO);
    // The `Single` mode of rifle_acc_F: a 0.001 rad half-angle cone around the aim.
    let angle = f32v(0.001_f32);

    let mut directions = Vec::new();
    for _ in 0..200 {
        let shot = fire(&mut world, s, "rifle_acc_F", "Flat_Mag", DVec3::ZERO);
        let v = world.entity(shot).unwrap().velocity();
        assert!((v.length() - 900.0).abs() < 1e-9, "|v| {}", v.length());
        let cos = v.normalize().dot(DVec3::X);
        assert!(cos >= angle.cos() - 1e-12, "outside the cone: {cos}");
        directions.push(v);
    }
    assert!(
        directions.iter().any(|d| *d != directions[0]),
        "dispersion 0.001 must not fire a laser"
    );

    // dispersion = 0 fires exactly along the aim.
    let shot = fire(&mut world, s, "rifle_F", "Ball_Mag", DVec3::ZERO);
    assert_eq!(
        world.entity(shot).unwrap().velocity(),
        DVec3::new(900.0, 0.0, 0.0)
    );
}

#[test]
fn a_shot_that_cannot_penetrate_stops_and_is_deleted() {
    let wall = DVec3::new(64.0, 0.0, 64.0);
    let mut world = common::scene(&[(
        "wall.p3d",
        common::box_model([0.5, 1.0, 1.0], "plank.bisurf"),
        wall,
    )]);
    let s = shooter(&mut world, DVec3::new(50.0, 0.0, 64.0));
    let shot = fire(
        &mut world,
        s,
        "rifle_F",
        "Ball_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );
    world.simulate(h());

    let events = world.drain_events();
    let target = static_at(&world, wall);
    let hit = hits(&events)
        .into_iter()
        .find(|(t, ..)| *t == target)
        .expect("the wall was hit");
    assert_eq!(hit.1, 900.0, "v_in is the speed before the impact");
    assert_eq!(hit.2, 0.0, "nothing left of it");
    // 0.5 m of plank at R = 1e6/100 over caliber 0.9 costs 11111 m/s: e = 900/800 (typical
    // speed), value = hit 8 × e = 9.
    assert!((hit.3 - 9.0).abs() < 1e-9, "value {}", hit.3);
    assert!(world.entity(shot).is_none(), "a stopped bullet is deleted");
}

#[test]
fn a_shot_penetrates_a_thin_plate_and_keeps_going() {
    let plate = DVec3::new(64.0, 0.0, 64.0);
    let mut world = common::scene(&[(
        "plate.p3d",
        // A 10 mm sheet: `thickness` makes the loss the thickness, not the geometry's depth.
        common::box_model([0.005, 1.0, 1.0], "sheet.bisurf"),
        plate,
    )]);
    let s = shooter(&mut world, DVec3::new(50.0, 0.0, 64.0));
    let shot = fire(
        &mut world,
        s,
        "rifle_F",
        "Ball_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );
    world.simulate(h());

    // R = 1e6/100 = 1e4; loss = (R/caliber)·L, L = the plate's 10 mm thickness. `SurfaceInfo`
    // keeps `thickness` as `f32` metres (`mm * 0.001` in `f32`), so the expected loss must round
    // through `f32` the same way.
    let thickness = f32v(10.0_f32 * 0.001_f32);
    let loss = (1e4 / f32v(0.9)) * thickness;
    let events = world.drain_events();
    let target = static_at(&world, plate);
    let hit = hits(&events)
        .into_iter()
        .find(|(t, ..)| *t == target)
        .expect("the plate was hit");
    assert_eq!(hit.1, 900.0);
    assert!((hit.2 - (900.0 - loss)).abs() < 1e-6, "speed_out {}", hit.2);
    // A punch-through barely damages: hit 8 × (loss / typicalSpeed 800).
    assert!((hit.3 - 8.0 * loss / 800.0).abs() < 1e-9, "value {}", hit.3);

    let e = world.entity(shot).expect("a penetrating bullet flies on");
    assert!(e.position().x > 90.0, "x {}", e.position().x);
    assert!((e.velocity().length() - (900.0 - loss)).abs() < 1e-6);
}

#[test]
fn a_grazing_shot_ricochets() {
    // A slab whose top face is at y = 0, hit at a grazing angle.
    let slab = DVec3::new(64.0, -0.5, 64.0);
    let mut world = common::scene(&[(
        "slab.p3d",
        common::box_model([2.0, 0.5, 2.0], "plank.bisurf"),
        slab,
    )]);
    let from = DVec3::new(62.5, 0.2, 64.0);
    let s = shooter(&mut world, from);
    let direction = DVec3::new(1.0, -0.2, 0.0).normalize();
    let shot = world
        .fire(FireRequest::new(s, "rifle_F", "Ball_Mag", from, direction))
        .unwrap();
    world.simulate(h());

    let events = world.drain_events();
    let hit = hits(&events)
        .into_iter()
        .next()
        .expect("the top face was hit");
    // The direction is a normalized vector, so the muzzle speed is 900 to rounding, not exactly.
    assert!((hit.1 - 900.0).abs() < 1e-9, "speed_in {}", hit.1);
    // maxSin = sin(deflecting 15° · surfDeflect 1), sinG = −n'·v̂ = 0.2/√1.04 = 0.196.
    let sin_g = 0.2 / 1.04_f64.sqrt();
    let max_sin = 15_f64.to_radians().sin();
    assert!(sin_g < max_sin, "{sin_g} vs {max_sin}");

    let e = world
        .entity(shot)
        .expect("a ricochet does not delete the shot");
    let v = e.velocity();
    assert!(v.y > 0.0, "it bounced up: {v:?}");
    // k = min(max(1 − (sinG/maxSin)², 0), deflectionSlowDown 1) · U(0.6, 0.9).
    let k = 1.0 - (sin_g / max_sin).powi(2);
    let (lo, hi) = (900.0 * k * 0.6, 900.0 * k * 0.9);
    assert!(
        v.length() > lo - 1e-9 && v.length() < hi + 1e-9,
        "|v| {}",
        v.length()
    );
    assert!((v.length() - hit.2).abs() < 1e-9, "speed_out {}", hit.2);
    assert!(hit.3 > 0.0, "a ricochet damages: {}", hit.3);
}

#[test]
fn an_explosive_shell_explodes_on_impact_and_blasts_nearby_objects() {
    let wall = DVec3::new(64.0, 0.0, 64.0);
    let near = DVec3::new(63.5, 0.0, 66.0);
    let far = DVec3::new(64.0, 0.0, 80.0);
    let mut world = common::scene(&[
        (
            "wall.p3d",
            common::box_model([0.5, 1.0, 1.0], "plank.bisurf"),
            wall,
        ),
        (
            "crate_near.p3d",
            common::box_model([0.5, 0.5, 0.5], "plank.bisurf"),
            near,
        ),
        (
            "crate_far.p3d",
            common::box_model([0.5, 0.5, 0.5], "plank.bisurf"),
            far,
        ),
    ]);
    let s = shooter(&mut world, DVec3::new(50.0, 0.0, 64.0));
    let shot = fire(
        &mut world,
        s,
        "launcher_F",
        "HE_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );
    world.simulate(h());

    let events = world.drain_events();
    let (wall_key, near_key, far_key) = (
        static_at(&world, wall),
        static_at(&world, near),
        static_at(&world, far),
    );
    let h = hits(&events);

    // Direct hit: e = min(300/900, 2)·(1 − explosive 0.6) + 0.6 (stopped, no fuse) → 30·e ≈ 22.
    // `explosive` is a config number, stored as `f32`, so the expected value carries that
    // rounding: the arithmetic has to be laid out the same way.
    let explosive = f32v(0.6_f32);
    let direct_value = 30.0 * ((300.0 / 900.0).min(2.0) * (1.0 - explosive) + explosive);
    let direct = h
        .iter()
        .find(|(t, ..)| *t == wall_key)
        .expect("the wall was hit directly");
    assert_eq!(direct.1, 300.0);
    assert_eq!(direct.2, 0.0);
    assert!(
        (direct.3 - direct_value).abs() < 1e-9,
        "value {} (want {direct_value})",
        direct.3
    );

    // Blast: 2.0 m from a 2.5 m radius is f = 1 → 0.33·(1+1+1)·indirectHit 12 = 11.88.
    let indirect = h
        .iter()
        .find(|(t, ..)| *t == near_key)
        .expect("the near crate is in the blast");
    assert!((indirect.3 - 11.88).abs() < 1e-9, "value {}", indirect.3);
    assert!(
        h.iter().all(|(t, ..)| *t != far_key),
        "16 m away is outside 4·r = 10 m"
    );

    let (position, radius) = exploded(&events).expect("an explosive shell explodes");
    assert!(
        position.distance(DVec3::new(63.5, 0.0, 64.0)) < 1e-6,
        "{position}"
    );
    assert_eq!(radius, 2.5);
    assert!(world.entity(shot).is_none(), "the shell is consumed");
}

#[test]
fn a_timed_shell_explodes_in_the_air() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(
        &mut world,
        s,
        "launcher_timed_F",
        "Timed_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );

    // 100 m/s: one frame is 5 m; explosionTime 0.1 s is two frames.
    world.simulate(h());
    let e = world.entity(shot).expect("still flying");
    assert!((e.position().x - 55.0).abs() < 1e-6, "x {}", e.position().x);
    world.drain_events();

    world.simulate(h());
    let events = world.drain_events();
    assert!(world.entity(shot).is_none(), "the shell is consumed");
    let (position, radius) = exploded(&events).expect("the time fuse fired");
    assert!(
        position.distance(DVec3::new(55.0, 0.0, 64.0)) < 1e-6,
        "it exploded where the fuse ran out, not further: {position}"
    );
    assert_eq!(radius, 0.0);
    assert!(
        hits(&events).is_empty(),
        "nothing is in range of a 0 m blast"
    );
}

#[test]
fn time_to_live_deletes_a_shot() {
    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let shot = fire(&mut world, s, "rifle_F", "Short_Mag", DVec3::ZERO);

    world.simulate(h());
    assert!(world.entity(shot).is_some(), "alive after 0.05 of 0.1 s");
    world.simulate(h());
    assert!(world.entity(shot).is_none(), "gone after 0.1 s");
}

#[test]
fn a_shot_ignores_its_shooter_and_their_vehicle() {
    let wall = DVec3::new(64.0, 0.0, 64.0);
    let person = || common::box_model([0.3, 0.8, 0.3], "plank.bisurf");
    let mut world = common::scene_with_models(
        &[(
            "wall.p3d",
            common::box_model([0.5, 1.0, 1.0], "plank.bisurf"),
            wall,
        )],
        &[
            ("shooter.p3d", person()),
            (
                "vehicle.p3d",
                common::box_model([1.0, 1.0, 2.0], "plank.bisurf"),
            ),
            ("bystander.p3d", person()),
        ],
    );
    let s = shooter(&mut world, DVec3::new(52.0, 0.0, 64.0));
    let vehicle = world
        .create(Create::new(
            Arc::new(EntityType::new("car", SimulationClass::CarX)),
            DVec3::new(58.0, 0.0, 64.0),
        ))
        .unwrap();
    world.attach(s, vehicle, DVec3::ZERO).unwrap();
    add_body(&mut world, "shooter.p3d", s, DVec3::new(52.0, 0.0, 64.0));
    add_body(
        &mut world,
        "vehicle.p3d",
        vehicle,
        DVec3::new(58.0, 0.0, 64.0),
    );

    // The shooter's own body and the vehicle's are in the way and must be shot through.
    fire(
        &mut world,
        s,
        "rifle_F",
        "Flat_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );
    world.simulate(h());
    let events = world.drain_events();
    let hit = hits(&events).into_iter().next().expect("something was hit");
    assert_eq!(hit.0, static_at(&world, wall), "the wall, not the bodies");

    // A bystander in the way is not ignored.
    let bystander = shooter(&mut world, DVec3::new(61.0, 0.0, 64.0));
    add_body(
        &mut world,
        "bystander.p3d",
        bystander,
        DVec3::new(61.0, 0.0, 64.0),
    );
    fire(
        &mut world,
        s,
        "rifle_F",
        "Flat_Mag",
        DVec3::new(50.0, 0.0, 64.0),
    );
    world.simulate(h());
    let events = world.drain_events();
    let hit = hits(&events)
        .into_iter()
        .next()
        .expect("the bystander was hit");
    assert_eq!(hit.0, ObjectRef::Entity(bystander));
    assert_eq!(hit.2, 0.0, "the plank body stops the bullet");
}

#[test]
fn firing_errors_are_reported() {
    // Without a config there are no weapons to read.
    let mut world = World::new(ClientId::SERVER);
    let s = shooter(&mut world, DVec3::ZERO);
    assert_eq!(
        world.fire(FireRequest::new(
            s,
            "rifle_F",
            "Ball_Mag",
            DVec3::ZERO,
            DVec3::X
        )),
        Err(Error::NoConfig)
    );

    let mut world = bare();
    let s = shooter(&mut world, DVec3::ZERO);
    let at = DVec3::ZERO;
    let look = |weapon: &str, magazine: &str| FireRequest::new(s, weapon, magazine, at, DVec3::X);

    // A deleted Entity is no shooter.
    let gone = shooter(&mut world, DVec3::new(1.0, 0.0, 0.0));
    world.delete(gone);
    world.flush_deletions();
    assert_eq!(
        world.fire(FireRequest::new(gone, "rifle_F", "Ball_Mag", at, DVec3::X)),
        Err(Error::NoSuchEntity(gone))
    );

    assert_eq!(
        world.fire(look("nope", "Ball_Mag")),
        Err(Error::UnknownWeapon("nope".to_owned()))
    );
    assert_eq!(
        world.fire(look("rifle_F", "nope")),
        Err(Error::UnknownMagazine("nope".to_owned()))
    );
    // Bad_Mag names an ammo class that does not exist.
    assert_eq!(
        world.fire(look("rifle_bad_F", "Bad_Mag")),
        Err(Error::UnknownAmmo("No_Ammo".to_owned()))
    );
    assert!(matches!(
        world.fire(look("rifle_F", "Ball_Mag").muzzle("nope")),
        Err(Error::NoSuchMuzzle { .. })
    ));
    assert!(matches!(
        world.fire(look("rifle_F", "Ball_Mag").mode("Burst")),
        Err(Error::NoSuchMode { .. })
    ));
    assert_eq!(
        world.fire(FireRequest::new(s, "rifle_F", "Ball_Mag", at, DVec3::ZERO)),
        Err(Error::ZeroDirection)
    );
}

#[test]
fn firing_a_real_weapon_from_the_shipped_config() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let mut world = World::new(ClientId::SERVER);
    world.set_config(data.config.clone());
    let s = shooter(&mut world, DVec3::ZERO);

    // The first shipped weapon whose first magazine (and its ammo) resolve.
    let mut fired = None;
    for class in data.config.root().get("CfgWeapons").entries() {
        if !class.is_class() {
            continue;
        }
        let Some(magazine) = class.get("magazines").array().iter().find_map(|v| match v {
            Value::String(s) | Value::Expression(s) => Some(s.clone()),
            _ => None,
        }) else {
            continue;
        };
        let weapon = class.name().to_owned();
        let request = FireRequest::new(
            s,
            weapon.clone(),
            magazine.clone(),
            DVec3::new(0.0, 2.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        );
        match world.fire(request) {
            Ok(shot) => {
                eprintln!("fired {weapon} with {magazine}");
                fired = Some(shot);
                break;
            }
            Err(
                Error::UnknownMagazine(_) | Error::UnknownAmmo(_) | Error::UnknownSimulation { .. },
            ) => continue,
            Err(e) => panic!("{weapon}/{magazine}: {e}"),
        }
    }

    let shot = fired.expect("some shipped weapon fires");
    let ammo = state(&world, shot).ammo.clone().expect("the ammo type");
    assert!(!ammo.name.is_empty());
    assert!(ammo.typical_speed > 0.0, "{ammo:?}");
    assert!(world.entity(shot).unwrap().velocity().length() > 0.0);
    eprintln!(
        "ammo {}: hit {}, caliber {}, typicalSpeed {}",
        ammo.name, ammo.hit, ammo.caliber, ammo.typical_speed
    );
}
