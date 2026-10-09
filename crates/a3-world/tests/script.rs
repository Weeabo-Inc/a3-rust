//! SQF world commands, run as scripts against a synthetic World.

mod common;

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{PublicTarget, Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, EntityId, NetworkId, TypeBank, World};
use a3_wrp::{TerrainBuilder, Transform};
use glam::Vec3;

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class AllVehicles: All {};
    class Land: AllVehicles {};
    class LandVehicle: Land {};
    class Car: LandVehicle { simulation = "carx"; };
    class C_Offroad_01_F: Car {
        scope = 2;
        model = "\A3\Soft_F\Offroad_01\Offroad_01_unarmed_F.p3d";
    };
    class Man: Land { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; };
    class Thing: All { simulation = "thing"; };
    class Land_Crate_F: Thing { scope = 2; };
};
"#;

/// Heights rise 1 m per 25 m eastwards. Two Static objects: id 0 at (10, 10), id 1 at
/// (120, 160). The collision world and the moves type of the Man fixture are loaded, so a
/// script-created Soldier can be animated and walked.
fn vm(client: ClientId) -> Vm<ScriptWorld> {
    let at = |x, z| Transform::from_position(Vec3::new(x, 0.0, z));
    let terrain = Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .heights(|i, _| i as f32)
            .object(r"a3\plants_f\tree\t_pinus.p3d", at(10.0, 10.0))
            .object(r"a3\structures_f\house\house.p3d", at(120.0, 160.0))
            .build(),
    );
    let mut world = World::new(client);
    world.load_terrain(terrain.clone()).unwrap();
    world.load_moves(common::moves());
    world.set_collision_world(common::collision_world(terrain, &[], &[]));
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(ScriptWorld::new(world, types), Rc::new(registry))
}

/// Steps the World `frames` times at the sim rate (1/15 s), as the engine runs it.
fn run(vm: &mut Vm<ScriptWorld>, frames: usize) {
    for _ in 0..frames {
        vm.host.world.simulate(1.0 / 15.0);
    }
}

/// The Entity an SQF Object value refers to.
fn entity(vm: &mut Vm<ScriptWorld>, name: &str) -> EntityId {
    let value = eval(vm, name);
    match a3_world::script::object_arg(&vm.host.world, &value) {
        Some(a3_world::ObjectRef::Entity(id)) => id,
        other => panic!("{name}: {other:?}"),
    }
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn text(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    let v = eval(vm, code);
    match v {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

fn nums(vm: &mut Vm<ScriptWorld>, code: &str) -> Vec<f64> {
    let v = eval(vm, code);
    v.as_array()
        .unwrap_or_else(|| panic!("{code}: not an array"))
        .borrow()
        .iter()
        .map(|x| f64::from(x.as_number().unwrap()))
        .collect()
}

fn assert_near(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-3, "{a:?} vs {b:?}");
    }
}

fn truth(vm: &mut Vm<ScriptWorld>, code: &str) -> bool {
    eval(vm, code)
        .as_bool()
        .unwrap_or_else(|| panic!("{code}: not a bool"))
}

#[test]
fn create_vehicle_places_on_the_terrain_and_reports_heights() {
    let mut vm = vm(ClientId::SERVER);

    eval(
        &mut vm,
        r#"v = createVehicle ["C_Offroad_01_F", [50, 10, 0], [], 0, "CAN_COLLIDE"]"#,
    );

    assert_eq!(text(&mut vm, "typeOf v"), "C_Offroad_01_F");
    assert_near(&nums(&mut vm, "getPosATL v"), &[50.0, 10.0, 0.0]);
    assert_near(&nums(&mut vm, "getPosASL v"), &[50.0, 10.0, 2.0]);
    assert_near(&nums(&mut vm, "getPos v"), &[50.0, 10.0, 0.0]);
}

#[test]
fn the_old_create_vehicle_syntax_works_too() {
    let mut vm = vm(ClientId::SERVER);

    eval(&mut vm, r#"v = "B_Soldier_F" createVehicle [25, 5, 1]"#);

    assert!(!truth(&mut vm, "isNull v"));
    assert_near(&nums(&mut vm, "getPosASL v"), &[25.0, 5.0, 2.0]);
}

#[test]
fn unknown_and_abstract_types_give_obj_null() {
    let mut vm = vm(ClientId::SERVER);

    assert!(truth(
        &mut vm,
        r#"isNull ("Nope_F" createVehicle [0, 0, 0])"#
    ));
    assert!(truth(&mut vm, r#"isNull ("Car" createVehicle [0, 0, 0])"#));
}

#[test]
fn net_ids_round_trip_and_local_objects_have_none() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"a = "Land_Crate_F" createVehicle [0, 0, 0]"#);
    eval(
        &mut vm,
        r#"b = "Land_Crate_F" createVehicleLocal [0, 0, 0]"#,
    );

    assert_eq!(text(&mut vm, "netId a"), "2:1");
    assert_eq!(text(&mut vm, "netId b"), "0:0");
    assert!(truth(&mut vm, r#"objectFromNetId "2:1" isEqualTo a"#));
    assert!(truth(&mut vm, r#"isNull objectFromNetId "2:99""#));
    assert_eq!(text(&mut vm, "netId objNull"), "");
}

#[test]
fn deleted_objects_are_dead_at_once_and_null_after_the_step() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"a = "Land_Crate_F" createVehicle [0, 0, 0]; deleteVehicle a"#,
    );

    assert!(!truth(&mut vm, "alive a"));
    assert!(!truth(&mut vm, "isNull a"));

    vm.host.world.simulate(0.016);

    assert!(truth(&mut vm, "isNull a"));
}

#[test]
fn position_and_direction_round_trip() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, "v setPosATL [100, 20, 5]; v setDir 90");
    assert_near(&nums(&mut vm, "getPosATL v"), &[100.0, 20.0, 5.0]);
    assert_near(&nums(&mut vm, "getPosASL v"), &[100.0, 20.0, 9.0]);
    assert!((f64::from(eval(&mut vm, "getDir v").as_number().unwrap()) - 90.0).abs() < 1e-3);
    assert_near(&nums(&mut vm, "vectorDir v"), &[1.0, 0.0, 0.0]);

    eval(&mut vm, "v setPosASL [10, 10, 50]");
    assert_near(&nums(&mut vm, "getPosASL v"), &[10.0, 10.0, 50.0]);

    eval(&mut vm, "v setVectorDirAndUp [[0, -1, 0], [0, 0, 1]]");
    assert_near(&nums(&mut vm, "vectorDir v"), &[0.0, -1.0, 0.0]);
    assert_near(&nums(&mut vm, "vectorUp v"), &[0.0, 0.0, 1.0]);
    assert!((f64::from(eval(&mut vm, "getDir v").as_number().unwrap()) - 180.0).abs() < 1e-3);
}

#[test]
fn velocity_and_speed() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]; v setDir 0"#,
    );

    eval(&mut vm, "v setVelocity [0, 10, 0]");
    assert_near(&nums(&mut vm, "velocity v"), &[0.0, 10.0, 0.0]);
    assert!((f64::from(eval(&mut vm, "speed v").as_number().unwrap()) - 36.0).abs() < 1e-3);

    eval(&mut vm, "v setVelocity [0, -10, 0]");
    assert!((f64::from(eval(&mut vm, "speed v").as_number().unwrap()) + 36.0).abs() < 1e-3);
}

#[test]
fn damage_and_alive() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, "v setDamage 0.5");
    assert_eq!(eval(&mut vm, "damage v").as_number(), Some(0.5));
    assert!(truth(&mut vm, "alive v"));

    eval(&mut vm, "v setDamage [1, false]");
    assert!(!truth(&mut vm, "alive v"));
    assert!(!truth(&mut vm, "alive objNull"));
}

#[test]
fn is_kind_of_follows_config_inheritance() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]"#);

    assert!(truth(&mut vm, r#"v isKindOf "Car""#));
    assert!(truth(&mut vm, r#"v isKindOf "landvehicle""#));
    assert!(!truth(&mut vm, r#"v isKindOf "Man""#));
    assert!(truth(&mut vm, r#""B_Soldier_F" isKindOf "Land""#));
    assert!(!truth(&mut vm, r#""Nope" isKindOf "All""#));
}

#[test]
fn locality_on_the_server_and_on_a_client() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]"#);
    assert!(truth(&mut vm, "local v"));
    assert_eq!(eval(&mut vm, "owner v").as_number(), Some(2.0));
    assert_eq!(eval(&mut vm, "clientOwner").as_number(), Some(2.0));

    let mut client = self::vm(ClientId(5000));
    let ty = client.host.types.get("C_Offroad_01_F").unwrap();
    let id = client
        .host
        .world
        .spawn_remote(
            ty,
            glam::DVec3::ZERO,
            NetworkId::new(2, 7),
            Some(ClientId::SERVER),
        )
        .unwrap();
    let _ = id;
    eval(&mut client, r#"r = objectFromNetId "2:7""#);
    assert!(!truth(&mut client, "local r"));
    assert_eq!(
        eval(&mut client, "owner r").as_number(),
        Some(0.0),
        "clients get 0"
    );

    // enableSimulation needs a local argument.
    eval(&mut client, "r enableSimulation false");
    assert!(truth(&mut client, "simulationEnabled r"));
    eval(&mut vm, "v enableSimulation false");
    assert!(!truth(&mut vm, "simulationEnabled v"));
}

#[test]
fn hiding_objects() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "Land_Crate_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, "hideObject v");
    assert!(truth(&mut vm, "isObjectHidden v"));
    eval(&mut vm, "v hideObjectGlobal false");
    assert!(!truth(&mut vm, "isObjectHidden v"));

    let mut client = self::vm(ClientId(5000));
    eval(
        &mut client,
        r#"c = "Land_Crate_F" createVehicle [0, 0, 0]; c hideObjectGlobal true"#,
    );
    assert!(!truth(&mut client, "isObjectHidden c"), "server only");
}

#[test]
fn static_objects_from_the_terrain() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, "t = nearestObject [[0, 0, 0], 1]");

    assert!(!truth(&mut vm, "isNull t"));
    assert_eq!(text(&mut vm, "getObjectID t"), "1");
    assert_eq!(text(&mut vm, "typeOf t"), "");
    assert!(truth(&mut vm, "alive t"));
    assert!(truth(&mut vm, "local t"));
    assert_near(&nums(&mut vm, "getPosASL t"), &[120.0, 160.0, 0.0]);
    assert!(text(&mut vm, "netId t").starts_with("1:-"));
    assert!(truth(&mut vm, "objectFromNetId (netId t) isEqualTo t"));
    assert_eq!(
        eval(&mut vm, "count nearestTerrainObjects [[0, 0, 0], [], 1000]").as_number(),
        Some(2.0)
    );

    // Changing a Static object promotes it; scripts keep seeing the same object.
    eval(&mut vm, "t setDamage 1");
    assert!(!truth(&mut vm, "alive t"));
    assert_eq!(text(&mut vm, "getObjectID t"), "1");
}

#[test]
fn proximity_queries() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        car = "C_Offroad_01_F" createVehicle [12, 10, 0];
        man = "B_Soldier_F" createVehicle [30, 10, 0];
        crate = "Land_Crate_F" createVehicleLocal [90, 10, 0];
    "#,
    );

    assert!(truth(
        &mut vm,
        r#"nearestObjects [[10, 10, 0], ["Car", "Man"], 100] isEqualTo [car, man]"#
    ));
    assert!(truth(
        &mut vm,
        r#"nearestObject [[10, 10, 0], "Man"] isEqualTo man"#
    ));
    assert!(truth(
        &mut vm,
        r#"([10, 10, 0] nearestObject "Car") isEqualTo car"#
    ));
    // Within 15 m: the car and the tree at (10, 10).
    assert_eq!(
        eval(&mut vm, "count ([10, 10, 0] nearObjects 15)").as_number(),
        Some(2.0)
    );
    assert!(truth(
        &mut vm,
        r#"(car nearObjects ["Man", 50]) isEqualTo [man]"#
    ));
    assert!(truth(&mut vm, r#"allMissionObjects "Car" isEqualTo [car]"#));
    assert_eq!(
        eval(&mut vm, r#"count allMissionObjects """#).as_number(),
        Some(3.0)
    );
}

#[test]
fn attached_objects_follow_their_parent() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        car = "C_Offroad_01_F" createVehicle [0, 0, 0];
        crate = "Land_Crate_F" createVehicle [0, 0, 0];
        crate attachTo [car, [0, -2, 1]];
    "#,
    );
    assert!(truth(&mut vm, "attachedTo crate isEqualTo car"));
    assert!(truth(&mut vm, "attachedObjects car isEqualTo [crate]"));

    eval(&mut vm, "car setPosASL [100, 100, 10]; car setDir 90");
    vm.host.world.simulate(0.016);

    // Facing east, "2 m behind" is 2 m west.
    assert_near(&nums(&mut vm, "getPosASL crate"), &[98.0, 100.0, 11.0]);

    eval(&mut vm, "detach crate");
    assert!(truth(&mut vm, "isNull attachedTo crate"));
}

#[test]
fn objects_print_like_the_original() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"v = "C_Offroad_01_F" createVehicle [0, 0, 0]"#);

    let s = text(&mut vm, "str v");
    assert!(s.ends_with(": offroad_01_unarmed_f.p3d"), "{s}");
    assert_eq!(text(&mut vm, "str objNull"), "<NULL-object>");
}

/// `playMove` and `playAction` queue the move the script asks for on the Man it names, and
/// `animationState` reads back the move he plays (`docs/re/sim-man-anim-state.md` §6).
#[test]
fn animation_commands_drive_the_move_state_machine() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"u = "B_Soldier_F" createVehicle [30, 10, 0]; c = "Land_Crate_F" createVehicleLocal [0, 0, 0]"#,
    );

    // He has no move until his first step, and anything that is no Man never has one.
    assert_eq!(text(&mut vm, "animationState u"), "");
    assert_eq!(text(&mut vm, "animationState c"), "");
    assert_eq!(text(&mut vm, "animationState objNull"), "");
    run(&mut vm, 1);
    assert_eq!(text(&mut vm, "animationState u"), "stand");

    // A name that is no action of the map and no move changes nothing, and an Object that is no
    // Man takes the command without a move state to change.
    let id = entity(&mut vm, "u");
    eval(
        &mut vm,
        r#"u playMove "no such move"; u playAction "no such action"; c playMove "walk""#,
    );
    assert_eq!(vm.host.world.man(id).unwrap().moves.queue().count(), 0);
    assert_eq!(text(&mut vm, "animationState u"), "stand");

    // `playMove` queues the move; it plays on his next step.
    eval(&mut vm, r#"u playMove "walk""#);
    run(&mut vm, 1);
    assert_eq!(text(&mut vm, "animationState u"), "walk");

    // `playAction` asks through the action map of the move he plays: `StandActions` has
    // `WalkF = "Walk"`.
    eval(&mut vm, r#"u playAction "WalkF""#);
    run(&mut vm, 1);
    assert_eq!(text(&mut vm, "animationState u"), "walk");
}

/// The immediate forms and `switchMove`: `playMoveNow` and `playActionNow` drop what was queued
/// and arm their move, `switchMove` resets him where he stands, and its array form writes the
/// cycle time and the blend factor of the record (`docs/re/sim-man-anim-state.md` §6.1, §6.3).
#[test]
fn immediate_forms_and_switch_move() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"u = "B_Soldier_F" createVehicle [30, 10, 0]"#);
    let id = entity(&mut vm, "u");
    run(&mut vm, 1);

    // `playMoveNow` drops what `playMove` queued and arms its move at once.
    eval(&mut vm, r#"u playMove "walk"; u playMoveNow "run""#);
    run(&mut vm, 1);
    assert_eq!(text(&mut vm, "animationState u"), "run");
    assert_eq!(vm.host.world.man(id).unwrap().moves.queue().count(), 0);

    // `playActionNow` resolves through the action map the same way. The hop out of a move waits
    // for the blend into it, so let Run settle first.
    run(&mut vm, 3);
    eval(&mut vm, r#"u playActionNow "WalkF""#);
    run(&mut vm, 1);
    assert_eq!(text(&mut vm, "animationState u"), "walk");

    // `switchMove` resets him on the spot: the queue and the request are dropped.
    eval(&mut vm, r#"u playMove "walkback"; u switchMove "run""#);
    assert_eq!(text(&mut vm, "animationState u"), "run");
    assert_eq!(vm.host.world.man(id).unwrap().moves.phase(), 0.0);
    assert_eq!(vm.host.world.man(id).unwrap().moves.queue().count(), 0);

    // The array form writes the cycle time and the blend factor; the `resetAim` flag is accepted
    // and ignored (nothing here aims).
    eval(&mut vm, r#"u switchMove ["walk", 0.5, 0.25, false]"#);
    assert_eq!(text(&mut vm, "animationState u"), "walk");
    assert_eq!(vm.host.world.man(id).unwrap().moves.phase(), 0.5);
    assert_eq!(vm.host.world.man(id).unwrap().moves.weight(), 0.25);
    let weight = vm.host.world.man(id).unwrap().moves.weight();
    run(&mut vm, 20);
    assert!(vm.host.world.man(id).unwrap().moves.weight() > weight);

    // A name the moves type does not know is the default of his action map: the idle of his
    // stance.
    eval(&mut vm, r#"u switchMove "no such move""#);
    assert_eq!(text(&mut vm, "animationState u"), "stand");
}

/// The five move commands take a local argument: a remote Man's move state is his owner's to
/// drive, so asking for a move here leaves him alone. `animationState` reads any Man.
#[test]
fn move_commands_of_a_remote_man_do_nothing() {
    let mut client = vm(ClientId(5000));
    let ty = client.host.types.get("B_Soldier_F").unwrap();
    let id = client
        .host
        .world
        .spawn_remote(
            ty,
            glam::DVec3::ZERO,
            NetworkId::new(2, 9),
            Some(ClientId::SERVER),
        )
        .unwrap();
    eval(&mut client, r#"r = objectFromNetId "2:9""#);
    assert!(!truth(&mut client, "local r"));

    eval(
        &mut client,
        r#"
        r playMove "walk"; r playMoveNow "walk"; r playAction "WalkF";
        r playActionNow "WalkF"; r switchMove "walk"; r switchMove ["run", 0.5, 0.25];
        "#,
    );

    let state = &client.host.world.man(id).unwrap().moves;
    assert_eq!(state.current(), None);
    assert_eq!(state.queue().count(), 0);
    assert_eq!(text(&mut client, "animationState r"), "");
}

#[test]
fn script_world_records_what_it_publishes() {
    // `publicVariable` hands the name to the host; there is no transport yet
    // (#131), so the record is the whole effect of a publish.
    let mut client = vm(ClientId::SERVER);
    eval(
        &mut client,
        r#"
        myVar = 1;
        publicVariable "myVar";
        publicVariableServer "myVar";
        3 publicVariableClient "myVar";
        "#,
    );
    assert_eq!(
        client.host.published_variables(),
        &[
            ("myVar".to_string(), PublicTarget::All),
            ("myVar".to_string(), PublicTarget::Server),
            ("myVar".to_string(), PublicTarget::Client(3)),
        ]
    );
}
