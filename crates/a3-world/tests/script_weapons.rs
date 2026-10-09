//! The weapon commands and the Loadout behind them, run as scripts against a synthetic World:
//! carrying weapons and magazines, firing (`fire`, `forceWeaponFire`, the held trigger), the
//! rate of fire, bursts, magazine reloads and the `Fired` handler's arguments.
//!
//! Sources: `docs/re/sim-weapons.md`; the command and handler signatures from the offline Arma
//! wiki.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{
    ScriptWorld, WorldHost, dispatch_events, object_arg, register_world_commands,
};
use a3_world::{Aim, ClientId, EntityId, ListKind, ObjectRef, TypeBank, World, WorldEvent};
use glam::DVec3;

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class B_Rifleman_F: B_Soldier_F {
        weapons[] = { "rifle_F", "NoSuchWeapon_F" };
        magazines[] = { "Mag_30", "Mag_30", "Mag_30", "NoSuchMag" };
    };
};
class CfgAmmo {
    class Default { simulation = ""; };
    class B_Test: Default {
        simulation = "shotBullet"; simulationStep = 0.05;
        hit = 8; caliber = 1; airFriction = -0.001; typicalSpeed = 800; timeToLive = 6;
    };
};
class CfgMagazines {
    class Mag_30 { ammo = "B_Test"; count = 30; initSpeed = 800; lastRoundsTracer = 2; };
    class Mag_30_Tracer: Mag_30 { tracersEvery = 1; lastRoundsTracer = 0; };
    class Mag_Other { ammo = "B_Test"; count = 5; initSpeed = 500; };
};
class Mode_SemiAuto { reloadTime = 0.1; burst = 1; autoFire = 0; dispersion = 0; };
class CfgWeapons {
    class Rifle_Base_F { type = 1; };
    class rifle_F: Rifle_Base_F {
        magazines[] = { "Mag_30", "Mag_30_Tracer" };
        magazineReloadTime = 2;
        modes[] = { "Single", "Burst", "FullAuto" };
        class Single: Mode_SemiAuto {};
        class Burst: Mode_SemiAuto { burst = 3; };
        class FullAuto: Mode_SemiAuto { autoFire = 1; };
    };
    class pistol_F {
        type = 2;
        magazines[] = { "Mag_Other" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { reloadTime = 0.2; };
    };
};
"#;

fn vm() -> Vm<ScriptWorld> {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(
        ScriptWorld::new(World::new(ClientId::SERVER), types),
        Rc::new(registry),
    )
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn text(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    eval(vm, code).to_sqf_string()
}

fn num(vm: &mut Vm<ScriptWorld>, code: &str) -> f64 {
    f64::from(
        eval(vm, code)
            .as_number()
            .unwrap_or_else(|| panic!("{code}: not a number")),
    )
}

/// A rifleman `a` with a rifle and two magazines (one loads at once).
const ARMED: &str = r#"
    g = createGroup west;
    a = g createUnit ["B_Soldier_F", [100, 100, 0], [], 0, "CAN_COLLIDE"];
    a addMagazine "Mag_30";
    a addMagazine ["Mag_30", 10];
    a addWeapon "rifle_F";
"#;

fn unit(vm: &mut Vm<ScriptWorld>) -> EntityId {
    let v = eval(vm, "a");
    match object_arg(vm.host.world(), &v) {
        Some(ObjectRef::Entity(id)) => id,
        other => panic!("not a unit: {other:?}"),
    }
}

/// Simulates the World for `frames` frames of `dt` and dispatches the handlers.
fn run(vm: &mut Vm<ScriptWorld>, frames: u32, dt: f64) {
    for _ in 0..frames {
        vm.host.world_mut().simulate(dt);
        dispatch_events(vm);
    }
}

fn shots(vm: &Vm<ScriptWorld>) -> usize {
    vm.host.world().list(ListKind::Projectiles).len()
}

#[test]
fn add_weapon_loads_the_fullest_carried_magazine() {
    let mut vm = vm();
    eval(&mut vm, ARMED);

    assert_eq!(text(&mut vm, "weapons a"), r#"["rifle_F"]"#);
    assert_eq!(text(&mut vm, "primaryWeapon a"), r#""rifle_F""#);
    assert_eq!(text(&mut vm, "currentWeapon a"), r#""rifle_F""#);
    assert_eq!(text(&mut vm, "currentMuzzle a"), r#""rifle_F""#);
    assert_eq!(text(&mut vm, "currentWeaponMode a"), r#""Single""#);
    assert_eq!(text(&mut vm, "currentMagazine a"), r#""Mag_30""#);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 30.0);
    // The loaded magazine is not in `magazines`.
    assert_eq!(text(&mut vm, "magazines a"), r#"["Mag_30"]"#);
}

#[test]
fn magazines_are_added_full_or_with_a_count_clamped_to_the_capacity() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "CAN_COLLIDE"];
        a addMagazines ["Mag_Other", 3];
        a addMagazine ["Mag_30", 99];
        a addWeapon "pistol_F";
        "#,
    );
    assert_eq!(
        text(&mut vm, "magazines a"),
        r#"["Mag_Other","Mag_Other","Mag_30"]"#
    );
    assert_eq!(text(&mut vm, "primaryWeapon a"), r#""""#);
    assert_eq!(num(&mut vm, r#"a ammo "pistol_F""#), 5.0);

    // `setAmmo` is clamped to the magazine's count.
    eval(&mut vm, r#"a setAmmo ["pistol_F", 2]"#);
    assert_eq!(num(&mut vm, r#"a ammo "pistol_F""#), 2.0);
    eval(&mut vm, r#"a setAmmo ["pistol_F", 50]"#);
    assert_eq!(num(&mut vm, r#"a ammo "pistol_F""#), 5.0);
}

#[test]
fn select_weapon_switches_between_carried_weapons() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    eval(
        &mut vm,
        r#"a addMagazine "Mag_Other"; a addWeapon "pistol_F";"#,
    );
    assert_eq!(text(&mut vm, "currentWeapon a"), r#""rifle_F""#);

    eval(&mut vm, r#"a selectWeapon "pistol_F""#);
    assert_eq!(text(&mut vm, "currentWeapon a"), r#""pistol_F""#);
    assert_eq!(text(&mut vm, "currentMagazine a"), r#""Mag_Other""#);
    assert_eq!(
        text(
            &mut vm,
            r#"a selectWeapon ["rifle_F", "rifle_F", "FullAuto"]"#
        ),
        "true"
    );
    assert_eq!(text(&mut vm, "currentWeaponMode a"), r#""FullAuto""#);
    assert_eq!(
        text(&mut vm, r#"a selectWeapon ["nope", "nope", ""]"#),
        "false"
    );

    eval(&mut vm, r#"a removeWeapon "rifle_F""#);
    assert_eq!(text(&mut vm, "weapons a"), r#"["pistol_F"]"#);
    assert_eq!(text(&mut vm, "currentWeapon a"), r#""pistol_F""#);
}

#[test]
fn fire_requests_a_round_that_goes_in_the_next_step_with_the_engine_arguments() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    eval(
        &mut vm,
        r#"
        fired = [];
        a addEventHandler ["Fired", { fired = _this }];
        a fire "rifle_F";
        "#,
    );
    // `fire` only leaves a request (`WeaponsState+0x30`): no shot, no round spent.
    assert_eq!(shots(&vm), 0);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 30.0);

    run(&mut vm, 1, 0.05);
    assert_eq!(shots(&vm), 1);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 29.0);
    // `[unit, weapon, muzzle, mode, ammo, magazine, projectile, gunner]`.
    assert_eq!(num(&mut vm, "count fired"), 8.0);
    assert_eq!(text(&mut vm, "fired select 0 == a"), "true");
    assert_eq!(
        text(&mut vm, "fired select [1, 5]"),
        r#"["rifle_F","rifle_F","Single","B_Test","Mag_30"]"#
    );
    assert_eq!(text(&mut vm, "typeOf (fired select 6)"), r#""B_Test""#);
    assert_eq!(text(&mut vm, "fired select 7 == a"), "true");
}

#[test]
fn a_mode_cannot_fire_again_before_its_reload_time() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    // Two `fire` calls in one frame are one request: the engine keeps a single pending slot.
    eval(&mut vm, r#"a fire "rifle_F"; a fire "rifle_F";"#);
    run(&mut vm, 1, 0.001);
    assert_eq!(
        num(&mut vm, r#"a ammo "rifle_F""#),
        29.0,
        "one round, not two"
    );

    // The round reload is `reloadTime · U(1 ± 0.1)` (0.09..0.11 s here): a request made inside
    // that window stays pending (`§3.3`) and goes once the weapon is ready.
    run(&mut vm, 10, 0.001);
    eval(&mut vm, r#"a fire "rifle_F""#);
    run(&mut vm, 1, 0.001);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 29.0, "0.011 s later");

    run(&mut vm, 12, 0.01);
    assert_eq!(
        num(&mut vm, r#"a ammo "rifle_F""#),
        28.0,
        "the pending request fires once 0.11 s have passed"
    );
}

#[test]
fn a_held_trigger_fires_once_in_single_a_burst_in_burst_and_on_in_full_auto() {
    for (mode, rounds) in [("Single", 1.0), ("Burst", 3.0), ("FullAuto", 10.0)] {
        let mut vm = vm();
        eval(&mut vm, ARMED);
        eval(
            &mut vm,
            &format!(r#"a selectWeapon ["rifle_F", "rifle_F", "{mode}"]"#),
        );
        let a = unit(&mut vm);
        vm.host.world_mut().set_trigger(a, true);
        // One second held at 0.1 s per frame: reloadTime 0.1 lets one round go per frame.
        run(&mut vm, 10, 0.1);
        assert_eq!(30.0 - num(&mut vm, r#"a ammo "rifle_F""#), rounds, "{mode}");
    }
}

#[test]
fn an_empty_magazine_is_changed_after_the_magazine_reload_time() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    eval(&mut vm, r#"a setAmmo ["rifle_F", 1]; a fire "rifle_F";"#);
    run(&mut vm, 1, 0.05);
    assert_eq!(shots(&vm), 1);
    // §3.4: the empty magazine is dropped, the 10-round spare goes in at once, and the muzzle
    // waits `magazineReloadTime · U(1 ± 0.2)` = 1.6..2.4 s before the next round.
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 10.0);
    assert_eq!(
        text(&mut vm, "magazines a"),
        "[]",
        "the spare is in the weapon"
    );

    eval(&mut vm, r#"a fire "rifle_F""#);
    run(&mut vm, 10, 0.1);
    assert_eq!(shots(&vm), 1, "still reloading");
    run(&mut vm, 20, 0.1);
    assert_eq!(shots(&vm), 2, "the held request goes after 2.4 s");
}

#[test]
fn a_player_does_not_auto_reload_a_muzzle_that_says_so() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    let a = unit(&mut vm);
    vm.host.world_mut().set_player(Some(a));
    eval(&mut vm, r#"a setAmmo ["rifle_F", 1]; a fire "rifle_F";"#);
    run(&mut vm, 1, 0.05);

    // `0x140f95940`: `autoReload` decides for a player, while AI always reloads.
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 0.0);
    assert_eq!(text(&mut vm, "magazines a"), r#"["Mag_30"]"#);
}

#[test]
fn reload_swaps_in_a_fuller_magazine_and_keeps_the_old_one() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "CAN_COLLIDE"];
        a addMagazine ["Mag_30", 10];
        a addWeapon "rifle_F";
        a addMagazine "Mag_30";
        reload a;
        "#,
    );
    run(&mut vm, 21, 0.1);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 30.0);
    assert_eq!(text(&mut vm, "magazines a"), r#"["Mag_30"]"#);
}

#[test]
fn tracer_rounds_follow_the_magazine() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    let a = unit(&mut vm);
    // `lastRoundsTracer = 2`: the last two rounds of a Mag_30 are tracers.
    eval(&mut vm, r#"a setAmmo ["rifle_F", 3]"#);
    let mut tracers = Vec::new();
    for _ in 0..3 {
        let shot = vm
            .host
            .world_mut()
            .fire_weapon(a, None, None)
            .unwrap()
            .expect("fires");
        let tracer = match vm.host.world().entity(shot).unwrap().class_state() {
            a3_world::ClassState::Projectile(p) => p.tracer,
            _ => unreachable!(),
        };
        tracers.push(tracer);
        // A semi-automatic round reload is `reloadTime · U(1 ± 0.1)`, so 0.11 s at most.
        run(&mut vm, 2, 0.1);
    }
    assert_eq!(tracers, [false, true, true]);
}

#[test]
fn the_shot_leaves_from_the_aim() {
    let mut vm = vm();
    eval(&mut vm, ARMED);
    let a = unit(&mut vm);
    let aim = Aim {
        from: DVec3::new(100.0, 1.6, 100.5),
        direction: DVec3::new(0.0, 0.0, 2.0),
    };
    vm.host.world_mut().set_aim(a, Some(aim));
    vm.host.world_mut().drain_events();
    let shot = vm
        .host
        .world_mut()
        .fire_weapon(a, None, None)
        .unwrap()
        .expect("fires");
    let e = vm.host.world().entity(shot).unwrap();
    assert_eq!(e.position(), aim.from);
    assert_eq!(e.velocity(), DVec3::new(0.0, 0.0, 800.0));
    assert!(vm.host.world_mut().drain_events().iter().any(|e| matches!(
        e,
        WorldEvent::Fired { shooter, .. } if shooter == &a
    )));
}

#[test]
fn a_unit_is_created_with_its_class_loadout() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Rifleman_F", [0, 0, 0], [], 0, "CAN_COLLIDE"];
        "#,
    );
    // The magazines go in first, then the weapons load from them; unknown names are skipped.
    assert_eq!(text(&mut vm, "weapons a"), r#"["rifle_F"]"#);
    assert_eq!(num(&mut vm, r#"a ammo "rifle_F""#), 30.0);
    assert_eq!(text(&mut vm, "magazines a"), r#"["Mag_30","Mag_30"]"#);
}
