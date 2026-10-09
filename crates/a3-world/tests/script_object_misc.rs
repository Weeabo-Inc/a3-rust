//! The world-command setters outside the unit/vehicle state family (#354): `direction`,
//! `setOvercast`, the simple-object pair, the object animation phase commands and the crew seats,
//! plus the contract-only stubs. Expectations mirror `tools/oracle/probes/99_world_setters_vr.probes`
//! where a probe exists.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, WorldHost, object_arg, register_world_commands};
use a3_world::{ClientId, EntityId, ObjectRef, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class Car: All { simulation = "carx"; };
    class B_MRAP_01_F: Car { scope = 2; side = 1; fuelCapacity = 50; };
    class Thing: All { simulation = "thing"; };
    class Box_NATO_Ammo_F: Thing { scope = 2; };
};
"#;

fn vm() -> Vm<ScriptWorld> {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    let mut vm = Vm::with_registry(
        ScriptWorld::new(World::new(ClientId::SERVER), types),
        Rc::new(registry),
    );
    eval(
        &mut vm,
        r#"u = (createGroup west) createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
           car = "B_MRAP_01_F" createVehicle [20, 0, 0];"#,
    );
    vm
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn sqf(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    match eval(vm, code) {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

fn id(vm: &mut Vm<ScriptWorld>, code: &str) -> EntityId {
    let value = eval(vm, code);
    match object_arg(vm.host.world(), &value) {
        Some(ObjectRef::Entity(id)) => id,
        other => panic!("{code}: not an object: {other:?}"),
    }
}

#[test]
fn direction_is_the_heading() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "direction car"), "0");
    eval(&mut vm, "car setDir 90");
    assert_eq!(sqf(&mut vm, "direction car"), "90");
    eval(&mut vm, "car setDir 450");
    assert_eq!(sqf(&mut vm, "direction car"), "90");
    // Anything that is not an object reads 0.
    assert_eq!(sqf(&mut vm, "direction objNull"), "0");
}

#[test]
fn set_overcast_applies_at_once() {
    let mut vm = vm();
    eval(&mut vm, "0 setOvercast 0.7");
    assert!((vm.host.world().environment().overcast - 0.7).abs() < 1e-6);
    eval(&mut vm, "0 setOvercast 2");
    assert_eq!(vm.host.world().environment().overcast, 1.0);
}

#[test]
fn simple_objects() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "isSimpleObject car"), "false");
    let simple = id(
        &mut vm,
        r#"box = createSimpleObject ["Box_NATO_Ammo_F", [5, 5, 0], false]; box"#,
    );
    assert_eq!(sqf(&mut vm, "isSimpleObject box"), "true");
    // A local object with no simulation, as the engine's simple objects are local render objects.
    let entity = vm.host.world().entity(simple).expect("created");
    assert!(entity.is_local());
    assert!(!entity.simulation_enabled());
    // An unknown shape gives objNull.
    assert_eq!(
        sqf(
            &mut vm,
            "isNull (createSimpleObject ['NoSuch', [0, 0, 0], false])"
        ),
        "true"
    );
    eval(&mut vm, "deleteVehicle box");
    // Like every deletion, the Entity goes at the end of the step.
    assert!(
        vm.host
            .world()
            .entity(simple)
            .is_none_or(|e| e.is_deleted())
    );
}

#[test]
fn animation_phases() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "car animationPhase 'door'"), "0");
    eval(&mut vm, "car animate ['door', 0.5, 1]");
    assert_eq!(sqf(&mut vm, "car animationPhase 'door'"), "0.5");
    eval(&mut vm, "car animate ['door', 1]");
    assert_eq!(sqf(&mut vm, "car animationPhase 'door'"), "1");
    assert_eq!(sqf(&mut vm, "car animationPhase 'NOPE'"), "0");
    eval(&mut vm, "car animateSource ['gear', 0.25]");
    assert_eq!(sqf(&mut vm, "car animationSourcePhase 'gear'"), "0.25");
    assert_eq!(sqf(&mut vm, "car animationPhase 'gear'"), "0");
    eval(&mut vm, "car animateDoor ['door_R', 0.75, 1]");
    assert_eq!(sqf(&mut vm, "car doorPhase 'door_R'"), "0.75");
    assert_eq!(sqf(&mut vm, "car animationPhase 'door_R'"), "0");
}

#[test]
fn action_on_a_unit_plays_it_and_on_an_object_is_kept() {
    let mut vm = vm();
    // A Man's action goes to his move state machine (`playAction`); a vehicle keeps its name.
    eval(&mut vm, r#"u action ["MoveTo", car]"#);
    eval(&mut vm, r#"action ["MoveTo", car]"#);
    let car = id(&mut vm, "car");
    assert_eq!(
        vm.host
            .world()
            .object_state(car)
            .and_then(|s| s.action.clone()),
        Some("MoveTo".to_owned())
    );
}

#[test]
fn crew_seats() {
    let mut vm = vm();
    let unit = id(&mut vm, "u");
    let car = id(&mut vm, "car");
    eval(&mut vm, "u moveInDriver car");
    eval(
        &mut vm,
        "b = (createGroup west) createUnit ['B_Soldier_F', [1, 0, 0], [], 0, 'NONE']",
    );
    eval(&mut vm, "b moveInGunner car");
    eval(&mut vm, "b moveInCargo car");
    let second = id(&mut vm, "b");
    let state = vm.host.world().object_state(car).expect("crew state");
    // The last order wins: he is in cargo, not the gunner's seat.
    assert_eq!(state.gunner, None);
    assert_eq!(state.cargo_seats, vec![(0, second)]);
    assert_eq!(state.driver, Some(unit));
    // Owning a seat in another vehicle is not owning it in two.
    eval(
        &mut vm,
        "car2 = 'B_MRAP_01_F' createVehicle [40, 0, 0]; u moveInCargo car2",
    );
    assert_eq!(
        vm.host.world().object_state(car).expect("state").driver,
        None
    );
    // assignAsCargo reserves a seat without filling it.
    eval(&mut vm, "u assignAsCargo car");
    let state = vm.host.world().object_state(car).expect("state");
    assert!(state.cargo_seats.iter().all(|(_, u)| *u != unit));
    assert_eq!(state.assigned_cargo, vec![(unit, 1)]);
    // The flags.
    eval(&mut vm, "car allowCrewInImmobile true");
    eval(&mut vm, "car setVehicleAmmo 0.5");
    let state = vm.host.world().object_state(car).expect("state");
    assert!(state.crew_in_immobile);
    assert_eq!(state.vehicle_ammo, Some(0.5));
}

#[test]
fn the_contract_only_stubs_keep_their_state() {
    let mut vm = vm();
    eval(&mut vm, "2 fadeMusic 0.4");
    assert_eq!(vm.host.world().music_volume(), 0.4);
    eval(&mut vm, "enableTeamSwitch true");
    assert!(vm.host.world().team_switch());
    eval(&mut vm, "u enableStamina false");
    eval(&mut vm, "group u enableAttack false");
    let unit = id(&mut vm, "u");
    let state = vm.host.world().object_state(unit).expect("state");
    assert!(!state.stamina);
    assert!(!state.attack);
    eval(&mut vm, r#"u createDiaryRecord ["Diary", "Hello"]"#);
    let state = vm.host.world().object_state(unit).expect("state");
    assert_eq!(state.diary, vec![("Diary".to_owned(), "Hello".to_owned())]);
    // Container objects do not exist; the stub answers objNull (recorded as a stub).
    assert_eq!(sqf(&mut vm, "isNull uniformContainer u"), "true");
}
