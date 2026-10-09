//! SQF group and side commands, run as scripts against a synthetic World.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class O_Soldier_F: Man { scope = 2; side = 0; };
    class Car: All { simulation = "carx"; };
    class B_MRAP_01_F: Car { scope = 2; side = 1; };
};
"#;

fn vm(client: ClientId) -> Vm<ScriptWorld> {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(
        ScriptWorld::new(World::new(client), types),
        Rc::new(registry),
    )
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn truth(vm: &mut Vm<ScriptWorld>, code: &str) -> bool {
    eval(vm, code)
        .as_bool()
        .unwrap_or_else(|| panic!("{code}: not a bool"))
}

fn text(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    match eval(vm, code) {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

#[test]
fn create_group_and_units() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        b = g createUnit ["B_Soldier_F", [5, 0, 0], [], 0, "NONE"];
    "#,
    );

    assert!(truth(&mut vm, "units g isEqualTo [a, b]"));
    assert!(truth(&mut vm, "leader g isEqualTo a"));
    assert!(truth(&mut vm, "leader b isEqualTo a"));
    assert!(truth(&mut vm, "group b isEqualTo g"));
    assert!(truth(&mut vm, "side a isEqualTo west"));
    assert!(truth(&mut vm, "side g isEqualTo west"));
    assert_eq!(text(&mut vm, "groupId g"), "Alpha 1-1");
    assert_eq!(text(&mut vm, "str g"), "B Alpha 1-1");
    assert_eq!(text(&mut vm, "str b"), "B Alpha 1-1:2");
    assert!(truth(&mut vm, "allGroups isEqualTo [g]"));
    assert!(truth(&mut vm, "allUnits isEqualTo [a, b]"));
    assert!(truth(&mut vm, "units west isEqualTo [a, b]"));
    assert!(truth(&mut vm, "units east isEqualTo []"));
}

#[test]
fn the_old_create_unit_syntax_joins_the_given_group() {
    let mut vm = vm(ClientId::SERVER);

    eval(
        &mut vm,
        r#"g = createGroup east; "O_Soldier_F" createUnit [[0, 0, 0], g]"#,
    );

    assert_eq!(eval(&mut vm, "count units g").as_number(), Some(1.0));
}

#[test]
fn joining_and_leaders() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g1 = createGroup west;
        g2 = createGroup [west, true];
        a = g1 createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        b = g2 createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        [b] join g1;
    "#,
    );

    assert!(truth(&mut vm, "units g1 isEqualTo [a, b]"));
    assert!(truth(&mut vm, "isNull g2"), "deleted when empty");

    eval(&mut vm, "g1 selectLeader b");
    assert!(truth(&mut vm, "leader g1 isEqualTo b"));

    eval(&mut vm, "g3 = createGroup west; [a] joinSilent b");
    assert!(truth(&mut vm, "group a isEqualTo g1"));
}

#[test]
fn group_ids_names_and_deletion() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"g = createGroup west; h = createGroup west; g setGroupId ["Hammer"]"#,
    );

    assert_eq!(text(&mut vm, "groupId g"), "Hammer");
    assert_eq!(text(&mut vm, "groupId h"), "Alpha 1-2");
    assert!(truth(&mut vm, r#"groupFromNetId (netId g) isEqualTo g"#));
    assert!(text(&mut vm, "netId g").starts_with("2:"));

    eval(&mut vm, "deleteGroup g");
    assert!(truth(&mut vm, "isNull g"));
    assert_eq!(text(&mut vm, "str grpNull"), "<NULL-group>");
}

#[test]
fn group_locality_and_owner() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"g = createGroup west; a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"]"#,
    );
    assert!(truth(&mut vm, "local g"));
    assert_eq!(eval(&mut vm, "groupOwner g").as_number(), Some(2.0));

    assert!(truth(&mut vm, "g setGroupOwner 5000"));
    assert!(!truth(&mut vm, "local g"));
    assert!(!truth(&mut vm, "local a"), "AI units follow the group");
    assert_eq!(eval(&mut vm, "groupOwner g").as_number(), Some(5000.0));
    assert_eq!(eval(&mut vm, "owner a").as_number(), Some(5000.0));
    assert!(!truth(&mut vm, "g setGroupOwner 5000"), "no change");

    let mut client = self::vm(ClientId(5000));
    eval(&mut client, "c = createGroup west");
    assert!(!truth(&mut client, "c setGroupOwner 2"), "server only");
}

#[test]
fn side_relations() {
    let mut vm = vm(ClientId::SERVER);

    assert_eq!(eval(&mut vm, "west getFriend east").as_number(), Some(0.0));
    assert_eq!(
        eval(&mut vm, "independent getFriend west").as_number(),
        Some(1.0)
    );

    eval(&mut vm, "independent setFriend [west, 0]");

    assert_eq!(
        eval(&mut vm, "independent getFriend west").as_number(),
        Some(0.0)
    );
    assert_eq!(
        eval(&mut vm, "west getFriend independent").as_number(),
        Some(1.0)
    );
}

/// Oracle probes `wstr.createunit_east_in_west` and `wstr.empty_vehicle_side`: a unit reports
/// its own side even in a group of another side, and an empty vehicle is civilian.
#[test]
fn side_of_a_unit_and_of_an_empty_vehicle() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        friend = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        enemy = g createUnit ["O_Soldier_F", [5, 0, 0], [], 0, "NONE"];
        car = createVehicle ["B_MRAP_01_F", [10, 0, 0], [], 0, "NONE"];
    "#,
    );

    assert!(truth(&mut vm, "side friend isEqualTo west"));
    assert!(truth(&mut vm, "side enemy isEqualTo east"), "own side");
    assert!(
        truth(&mut vm, "side group enemy isEqualTo west"),
        "group side"
    );
    assert!(
        truth(&mut vm, "side car isEqualTo civilian"),
        "empty vehicle"
    );
}

/// Oracle probe `wvr.group_formation`: `behaviour leader grpNull` is "ERROR".
#[test]
fn the_modes_of_a_group_without_units() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, "g = createGroup west");

    assert_eq!(text(&mut vm, "behaviour leader g"), "ERROR");
    assert_eq!(text(&mut vm, "formation g"), "WEDGE");
    assert_eq!(text(&mut vm, "combatMode g"), "YELLOW");
    assert_eq!(text(&mut vm, "speedMode g"), "NORMAL");
}

/// Oracle probe `wstr.allgroups_empty`: `deleteVehicle _u; deleteGroup _g` leaves no group, even
/// though the deletion of the unit takes effect at the end of the step.
#[test]
fn delete_group_takes_units_scheduled_for_deletion() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        u = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        deleteVehicle u;
        deleteGroup g;
    "#,
    );

    assert!(truth(&mut vm, "isNull g"));
    assert_eq!(eval(&mut vm, "count allGroups").as_number(), Some(0.0));
}
