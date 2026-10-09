//! SQF AI commands — waypoints, group modes, unit orders and target knowledge — run as scripts
//! against a synthetic World (#129).

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, WorldHost, register_world_commands};
use a3_world::{ClientId, EntityId, ObjectRef, TypeBank, World};
use glam::DVec3;

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class O_Soldier_F: Man { scope = 2; side = 0; };
};
"#;

/// Two groups: West `g` with `a` and `b`, east `g2` with `e`.
const TWO_GROUPS: &str = r#"
    g = createGroup west;
    a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
    b = g createUnit ["B_Soldier_F", [5, 0, 0], [], 0, "NONE"];
    g2 = createGroup east;
    e = g2 createUnit ["O_Soldier_F", [100, 0, 0], [], 0, "NONE"];
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

fn number(vm: &mut Vm<ScriptWorld>, code: &str) -> f32 {
    eval(vm, code)
        .as_number()
        .unwrap_or_else(|| panic!("{code}: not a number"))
}

/// The Entity an SQF Object value names.
fn entity(vm: &mut Vm<ScriptWorld>, code: &str) -> EntityId {
    let Value::Handle(h) = eval(vm, code) else {
        panic!("{code}: not an object")
    };
    match ObjectRef::from_handle_id(h.id) {
        Some(ObjectRef::Entity(id)) => id,
        _ => panic!("{code}: not an entity handle"),
    }
}

/// The point the unit `code` names was ordered to, if any.
fn move_order(vm: &mut Vm<ScriptWorld>, code: &str) -> Option<DVec3> {
    let id = entity(vm, code);
    vm.host.world().unit_move_order(id)
}

#[test]
fn add_waypoint_returns_the_engines_waypoint_array() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [10, 10, 0], [], 0, "NONE"];
        wp = g addWaypoint [[10, 50, 0], 0];
    "#,
    );

    // The engine numbers the first explicit waypoint 1: index 0 is its implicit start one.
    assert!(truth(&mut vm, "wp isEqualTo [g, 1]"));
    assert!(truth(&mut vm, "(wp select 0) isEqualTo g"));
    assert_eq!(
        number(&mut vm, "count waypoints g"),
        2.0,
        "the implicit one and ours"
    );
    assert_eq!(number(&mut vm, "currentWaypoint g"), 1.0);
    assert_eq!(text(&mut vm, "waypointType wp"), "MOVE");

    // Adding with an index inserts before that waypoint, and the ones after it move up. A
    // Waypoint array is a value, not a reference to a slot: `wp` keeps reading `[g, 1]`, which
    // now names the inserted waypoint — the live index is what moved.
    eval(
        &mut vm,
        "w2 = g addWaypoint [[10, 20, 0], 0, 1]; w3 = g addWaypoint [[10, 90, 0], 0];",
    );
    assert!(truth(&mut vm, "w2 isEqualTo [g, 1]"), "inserted first");
    assert!(
        truth(&mut vm, "wp isEqualTo [g, 1]"),
        "a Waypoint array is a snapshot"
    );
    assert!(truth(&mut vm, "w3 isEqualTo [g, 3]"), "appended last");
    assert!(truth(
        &mut vm,
        "waypointPosition [g, 1] isEqualTo [10, 20, 0]"
    ));
    assert!(
        truth(&mut vm, "waypointPosition [g, 2] isEqualTo [10, 50, 0]"),
        "pushed along"
    );
    assert!(
        truth(&mut vm, "waypointName [g, 1] isEqualTo \"\""),
        "no name given"
    );
}

#[test]
fn add_waypoint_takes_a_name() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        wp = g addWaypoint [[10, 0, 0], 0, -1, "attack"];
    "#,
    );

    assert!(truth(&mut vm, "wp isEqualTo [g, 1]"));
    assert_eq!(text(&mut vm, "waypointName wp"), "attack");
}

#[test]
fn a_group_with_no_waypoints_reports_index_zero() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        "g = createGroup west; a = g createUnit [\"B_Soldier_F\", [0, 0, 0], [], 0, \"NONE\"];",
    );

    assert_eq!(
        number(&mut vm, "currentWaypoint g"),
        0.0,
        "0 with no waypoints"
    );
    // Only the engine's implicit waypoint 0 is there; the first added one is number 1
    // (`docs/re/ai.md` on the index model).
    assert!(truth(&mut vm, "waypoints g isEqualTo [[g, 0]]"));
}

#[test]
fn waypoint_properties_round_trip() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        wp = g addWaypoint [[10, 10, 0], 0];
        wp setWaypointType "HOLD";
        wp setWaypointPosition [[30, 0, 0], 0];
        wp setWaypointBehaviour "COMBAT";
        wp setWaypointCombatMode "RED";
        wp setWaypointSpeed "LIMITED";
        wp setWaypointFormation "COLUMN";
        wp setWaypointCompletionRadius 7;
        wp setWaypointDescription "Wait here";
        wp setWaypointName "hold";
        wp setWaypointVisible false;
        wp setWaypointTimeout [1, 2, 3];
        wp setWaypointStatements ["true", "hint 'x'"];
        wp setWaypointScript "scripts\hold.sqs";
        wp setWaypointHousePosition 3;
        wp setWaypointLoiterRadius 40;
        wp setWaypointLoiterType "CIRCLE";
    "#,
    );

    assert_eq!(text(&mut vm, "waypointType wp"), "HOLD");
    assert!(truth(&mut vm, "waypointPosition wp isEqualTo [30, 0, 0]"));
    assert_eq!(text(&mut vm, "waypointBehaviour wp"), "COMBAT");
    assert_eq!(text(&mut vm, "waypointCombatMode wp"), "RED");
    assert_eq!(text(&mut vm, "waypointSpeed wp"), "LIMITED");
    assert_eq!(text(&mut vm, "waypointFormation wp"), "COLUMN");
    assert_eq!(number(&mut vm, "waypointCompletionRadius wp"), 7.0);
    assert_eq!(text(&mut vm, "waypointDescription wp"), "Wait here");
    assert_eq!(text(&mut vm, "waypointName wp"), "hold");
    assert!(!truth(&mut vm, "waypointVisible wp"));
    assert!(truth(&mut vm, "waypointTimeout wp isEqualTo [1, 2, 3]"));
    assert!(truth(
        &mut vm,
        "waypointStatements wp isEqualTo [\"true\", \"hint 'x'\"]"
    ));
    assert_eq!(text(&mut vm, "waypointScript wp"), r"scripts\hold.sqs");
    assert_eq!(number(&mut vm, "waypointHousePosition wp"), 3.0);
    assert_eq!(number(&mut vm, "waypointLoiterRadius wp"), 40.0);
    assert_eq!(text(&mut vm, "waypointLoiterType wp"), "CIRCLE");

    // A waypoint that sets nothing leaves the group alone ("UNCHANGED").
    eval(
        &mut vm,
        "wp setWaypointSpeed \"UNCHANGED\"; wp setWaypointBehaviour \"UNCHANGED\";",
    );
    assert_eq!(
        text(&mut vm, "waypointSpeed wp"),
        "",
        "UNCHANGED stores nothing"
    );
    assert_eq!(text(&mut vm, "waypointBehaviour wp"), "");
}

#[test]
fn the_queue_takes_turns_and_deleting_re_indexes() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        w1 = g addWaypoint [[10, 0, 0], 0];
        w2 = g addWaypoint [[20, 0, 0], 0];
        w3 = g addWaypoint [[30, 0, 0], 0];
    "#,
    );

    assert_eq!(number(&mut vm, "count waypoints g"), 4.0);
    assert_eq!(number(&mut vm, "currentWaypoint g"), 1.0);

    eval(&mut vm, "g setCurrentWaypoint [g, 2]");
    assert_eq!(number(&mut vm, "currentWaypoint g"), 2.0);
    assert!(truth(
        &mut vm,
        "waypointPosition [g, 2] isEqualTo [20, 0, 0]"
    ));

    // Deleting the waypoint before the active one moves the active one down with it.
    eval(&mut vm, "deleteWaypoint [g, 1]");
    assert_eq!(number(&mut vm, "count waypoints g"), 3.0);
    assert_eq!(number(&mut vm, "currentWaypoint g"), 1.0);
    assert!(truth(
        &mut vm,
        "waypointPosition [g, 1] isEqualTo [20, 0, 0]"
    ));

    // The implicit waypoint 0 cannot be deleted.
    eval(&mut vm, "deleteWaypoint [g, 0]");
    assert_eq!(number(&mut vm, "count waypoints g"), 3.0);
}

#[test]
fn move_replaces_the_queue() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        w1 = g addWaypoint [[10, 0, 0], 0];
        w2 = g addWaypoint [[20, 0, 0], 0];
        g move [50, 0, 0];
    "#,
    );

    assert_eq!(
        number(&mut vm, "count waypoints g"),
        2.0,
        "one waypoint left"
    );
    assert_eq!(number(&mut vm, "currentWaypoint g"), 1.0);
    assert!(truth(
        &mut vm,
        "waypointPosition [g, 1] isEqualTo [50, 0, 0]"
    ));
    assert_eq!(text(&mut vm, "waypointType [g, 1]"), "MOVE");

    // `move` on a unit is the same as on his group.
    eval(&mut vm, "a move [70, 0, 0]");
    assert!(truth(
        &mut vm,
        "waypointPosition [g, 1] isEqualTo [70, 0, 0]"
    ));
}

#[test]
fn group_modes_round_trip_and_units_speak_for_the_group() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, TWO_GROUPS);

    assert_eq!(
        text(&mut vm, "behaviour g"),
        "AWARE",
        "the engine's default"
    );
    assert_eq!(text(&mut vm, "combatMode g"), "YELLOW");
    assert_eq!(text(&mut vm, "speedMode g"), "NORMAL");
    assert_eq!(text(&mut vm, "formation g"), "WEDGE");

    eval(
        &mut vm,
        r#"
        g setBehaviour "COMBAT";
        g setCombatMode "RED";
        g setSpeedMode "FULL";
        g setFormation "COLUMN";
    "#,
    );
    assert_eq!(text(&mut vm, "behaviour g"), "COMBAT");
    assert_eq!(text(&mut vm, "combatMode g"), "RED");
    assert_eq!(text(&mut vm, "speedMode g"), "FULL");
    assert_eq!(text(&mut vm, "formation g"), "COLUMN");

    // A unit is his group for all of these, as the mission scripts assume.
    eval(&mut vm, "a setBehaviour \"SAFE\"; a setFormation \"FILE\";");
    assert_eq!(text(&mut vm, "behaviour g"), "SAFE");
    assert_eq!(text(&mut vm, "behaviour b"), "SAFE", "the whole group");
    assert_eq!(text(&mut vm, "formation g"), "FILE");
}

#[test]
fn unit_orders_set_and_clear() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, TWO_GROUPS);

    eval(&mut vm, "b doMove [40, 0, 0]");
    assert_eq!(
        move_order(&mut vm, "b"),
        Some(DVec3::new(40.0, 0.0, 0.0)),
        "doMove gives the unit an order of his own"
    );

    // Script [x, y, z] is world (x, z, y) (ADR 0003).
    eval(&mut vm, "a commandMove [10, 20, 0]");
    assert_eq!(move_order(&mut vm, "a"), Some(DVec3::new(10.0, 0.0, 20.0)));

    // An array left argument orders every unit of it.
    eval(&mut vm, "[a, b] moveTo [7, 8, 0]");
    let target = DVec3::new(7.0, 0.0, 8.0);
    assert_eq!(move_order(&mut vm, "a"), Some(target));
    assert_eq!(move_order(&mut vm, "b"), Some(target));

    eval(&mut vm, "doStop a");
    assert!(truth(&mut vm, "stopped a"));
    assert!(!truth(&mut vm, "stopped b"));
    assert_eq!(move_order(&mut vm, "a"), None);

    // doStop takes an array of units, in the engine's prefix form.
    eval(&mut vm, "doStop [a, b]");
    assert!(truth(&mut vm, "stopped b"));

    eval(&mut vm, "b doFollow a");
    assert!(!truth(&mut vm, "stopped b"), "doFollow puts him back in");
    assert_eq!(move_order(&mut vm, "b"), None);
}

#[test]
fn target_knowledge_rises_with_reveal_and_falls_with_forget() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, TWO_GROUPS);

    assert_eq!(
        number(&mut vm, "a knowsAbout e"),
        0.0,
        "nobody has seen him"
    );
    assert_eq!(
        number(&mut vm, "a knowsAbout b"),
        4.0,
        "his own group always"
    );
    assert_eq!(number(&mut vm, "a knowsAbout a"), 4.0, "himself");

    // Revealing to the group raises the knowledge of every man of it, and of the side.
    eval(&mut vm, "g reveal e");
    assert_eq!(number(&mut vm, "a knowsAbout e"), 1.0, "the engine's floor");
    assert_eq!(number(&mut vm, "b knowsAbout e"), 1.0);
    assert_eq!(number(&mut vm, "g knowsAbout e"), 1.0);
    assert_eq!(number(&mut vm, "west knowsAbout e"), 1.0);

    // `reveal [target, accuracy]` sets the amount; knowledge only ever rises.
    eval(&mut vm, "g reveal [e, 2.5]");
    assert_eq!(number(&mut vm, "a knowsAbout e"), 2.5);
    eval(&mut vm, "g reveal [e, 1]");
    assert_eq!(
        number(&mut vm, "a knowsAbout e"),
        2.5,
        "reveal never lowers"
    );

    // A unit as the left argument reveals to his group; a side keeps nothing of its own.
    eval(&mut vm, "g forgetTarget e");
    assert_eq!(number(&mut vm, "a knowsAbout e"), 0.0);
    assert_eq!(number(&mut vm, "west knowsAbout e"), 0.0);
    eval(&mut vm, "a reveal e");
    assert_eq!(number(&mut vm, "b knowsAbout e"), 1.0);
}

#[test]
fn copy_waypoints_replaces_the_target_queue_and_get_wp_pos_reads_the_position() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        &format!(
            r#"{TWO_GROUPS}
            g addWaypoint [[10, 0, 20], 0];
            g addWaypoint [[30, 0, 40], 0];
            g2 addWaypoint [[99, 0, 99], 0];
            g2 copyWaypoints g;
            "#
        ),
    );

    assert_eq!(
        number(&mut vm, "count waypoints g2"),
        3.0,
        "start + 2 copies"
    );
    assert!(truth(
        &mut vm,
        "getWPPos [g2, 1] isEqualTo waypointPosition [g, 1]"
    ));
    assert_eq!(number(&mut vm, "getWPPos [g2, 1] select 0"), 10.0);
    assert!(truth(
        &mut vm,
        "getWPPos [g2, 2] isEqualTo waypointPosition [g, 2]"
    ));
    assert_eq!(number(&mut vm, "currentWaypoint g2"), 1.0);
}
