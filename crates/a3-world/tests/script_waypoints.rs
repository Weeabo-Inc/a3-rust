//! SQF waypoint commands, run as scripts against a synthetic World.

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

fn num(vm: &mut Vm<ScriptWorld>, code: &str) -> f64 {
    f64::from(
        eval(vm, code)
            .as_number()
            .unwrap_or_else(|| panic!("{code}: not a number")),
    )
}

#[test]
fn add_waypoint_appends_and_returns_the_waypoint() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w1 = g addWaypoint [[100, 200], 5];
        w2 = g addWaypoint [[300, 400], 0, -1, "second"];
    "#,
    );

    assert!(truth(&mut vm, "w1 isEqualTo [g, 0]"));
    assert!(truth(&mut vm, "w2 isEqualTo [g, 1]"));
    assert!(truth(&mut vm, "count waypoints g == 2"));
    assert_eq!(text(&mut vm, "waypointName w2"), "second");
    // The placement radius is what the waypoint was added with.
    assert!(truth(&mut vm, "waypoints g isEqualTo [w1, w2]"));
}

#[test]
fn add_waypoint_inserts_at_a_valid_index_and_appends_past_the_end() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        g addWaypoint [[0, 0], 0];
        g addWaypoint [[100, 0], 0];
        w = g addWaypoint [[50, 0], 0, 1];
        past = g addWaypoint [[200, 0], 0, 17];
    "#,
    );

    // The new waypoint went in at index 1, pushing the old second one to 2.
    assert!(truth(&mut vm, "w isEqualTo [g, 1]"));
    assert!(truth(&mut vm, "past isEqualTo [g, 3]"));
    assert!(truth(&mut vm, "count waypoints g == 4"));
    assert_eq!(text(&mut vm, "str (waypointPosition w select 0)"), "50");
}

#[test]
fn a_new_waypoint_has_the_engine_defaults() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w = g addWaypoint [[100, 200], 0];
    "#,
    );

    assert_eq!(text(&mut vm, "waypointType w"), "");
    assert_eq!(text(&mut vm, "waypointBehaviour w"), "UNCHANGED");
    assert_eq!(text(&mut vm, "waypointCombatMode w"), "NO CHANGE");
    assert_eq!(text(&mut vm, "waypointFormation w"), "NO CHANGE");
    assert_eq!(text(&mut vm, "waypointSpeed w"), "UNCHANGED");
    assert_eq!(text(&mut vm, "waypointDescription w"), "");
    assert_eq!(text(&mut vm, "waypointShow w"), "AUTO");
    assert_eq!(num(&mut vm, "waypointCompletionRadius w"), 0.0);
    assert_eq!(num(&mut vm, "waypointLoiterRadius w"), 20.0);
    assert_eq!(num(&mut vm, "waypointHousePosition w"), -1.0);
    assert_eq!(num(&mut vm, "waypointVisible w"), 1.0);
    assert!(!truth(&mut vm, "waypointForceBehaviour w"));
    assert!(truth(&mut vm, r#"waypointStatements w isEqualTo ["", ""]"#));
    assert!(truth(&mut vm, "waypointTimeout w isEqualTo [0, 0, 0]"));
}

#[test]
fn waypoint_fields_set_and_get() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w = g addWaypoint [[0, 0], 0];
        w setWaypointType "SAD";
        w setWaypointBehaviour "COMBAT";
        w setWaypointCombatMode "RED";
        w setWaypointFormation "WEDGE";
        w setWaypointSpeed "FULL";
        w setWaypointDescription "Clear the town";
        w setWaypointCompletionRadius 25;
        w setWaypointLoiterRadius 100;
        w setWaypointLoiterType "CIRCLE_L";
        w setWaypointHousePosition 3;
        w setWaypointForceBehaviour true;
        w setWaypointVisible false;
        w setWaypointScript "scripts\task.sqf";
        w showWaypoint "ALWAYS";
        w setWaypointStatements ["true", "hint 'done'"];
        w setWaypointTimeout [5, 10, 20];
    "#,
    );

    assert_eq!(text(&mut vm, "waypointType w"), "SAD");
    assert_eq!(text(&mut vm, "waypointBehaviour w"), "COMBAT");
    assert_eq!(text(&mut vm, "waypointCombatMode w"), "RED");
    assert_eq!(text(&mut vm, "waypointFormation w"), "WEDGE");
    assert_eq!(text(&mut vm, "waypointSpeed w"), "FULL");
    assert_eq!(text(&mut vm, "waypointDescription w"), "Clear the town");
    assert_eq!(num(&mut vm, "waypointCompletionRadius w"), 25.0);
    assert_eq!(num(&mut vm, "waypointLoiterRadius w"), 100.0);
    assert_eq!(text(&mut vm, "waypointLoiterType w"), "CIRCLE_L");
    assert_eq!(num(&mut vm, "waypointHousePosition w"), 3.0);
    assert!(truth(&mut vm, "waypointForceBehaviour w"));
    assert_eq!(num(&mut vm, "waypointVisible w"), 0.0);
    assert_eq!(text(&mut vm, "waypointScript w"), "scripts\\task.sqf");
    assert_eq!(text(&mut vm, "waypointShow w"), "ALWAYS");
    assert!(truth(
        &mut vm,
        r#"str waypointStatements w == "[""true"",""hint 'done'""]""#
    ));
    assert!(truth(&mut vm, "waypointTimeout w isEqualTo [5, 10, 20]"));
}

#[test]
fn waypoint_position_set_and_get() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w = g addWaypoint [[100, 200], 0];
    "#,
    );

    assert_eq!(text(&mut vm, "str waypointPosition w"), "[100,200,0]");
    assert_eq!(text(&mut vm, "str getWPPos w"), "[100,200,0]");

    eval(&mut vm, r#"w setWaypointPosition [[300, 400], 10]"#);
    assert_eq!(text(&mut vm, "str waypointPosition w"), "[300,400,0]");

    eval(&mut vm, r#"w setWPPos [500, 600]"#);
    assert_eq!(text(&mut vm, "str getWPPos w"), "[500,600,0]");
}

#[test]
fn delete_waypoint_re_indexes_the_rest() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        g addWaypoint [[0, 0], 0];
        g addWaypoint [[100, 0], 0];
        g addWaypoint [[200, 0], 0];
        deleteWaypoint [g, 0];
    "#,
    );

    assert!(truth(&mut vm, "count waypoints g == 2"));
    assert_eq!(text(&mut vm, "str waypointPosition [g, 0]"), "[100,0,0]");

    eval(&mut vm, r#"deleteWaypoint [g, 5]"#);
    assert!(truth(&mut vm, "count waypoints g == 2"));
}

#[test]
fn copy_waypoints_replaces_the_target_list() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        src = createGroup west;
        dst = createGroup east;
        src addWaypoint [[0, 0], 0];
        src addWaypoint [[100, 0], 0];
        dst addWaypoint [[7, 7], 0];
        dst copyWaypoints src;
        src addWaypoint [[200, 0], 0];
    "#,
    );

    assert!(truth(&mut vm, "count waypoints dst == 2"));
    // A copy, not an alias: adding to `src` leaves `dst` alone.
    assert!(truth(&mut vm, "count waypoints src == 3"));
    assert_eq!(text(&mut vm, "str waypointPosition [dst, 1]"), "[100,0,0]");
}

#[test]
fn current_waypoint_tracks_the_active_index() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        g addWaypoint [[0, 0], 0];
        g addWaypoint [[100, 0], 0];
    "#,
    );

    assert_eq!(num(&mut vm, "currentWaypoint g"), 0.0);
    eval(&mut vm, "g setCurrentWaypoint [g, 1]");
    assert_eq!(num(&mut vm, "currentWaypoint g"), 1.0);
    // `setCurrentWaypoint` with an index past the end does nothing.
    eval(&mut vm, "g setCurrentWaypoint [g, 9]");
    assert_eq!(num(&mut vm, "currentWaypoint g"), 1.0);
}

#[test]
fn waypoints_accepts_a_unit_and_reports_its_group() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        g addWaypoint [[0, 0], 0];
        g addWaypoint [[100, 0], 0];
    "#,
    );

    // `waypoints` takes a unit as well as a group; `currentWaypoint` takes only a group.
    assert!(truth(&mut vm, "count waypoints a == 2"));
    assert!(truth(&mut vm, "count waypoints g == 2"));
    assert_eq!(num(&mut vm, "currentWaypoint g"), 0.0);
}

#[test]
fn accessors_of_an_unknown_waypoint_are_empty_and_setters_do_nothing() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w = g addWaypoint [[0, 0], 0];
        w setWaypointTimeout [];
        missing = [g, 4];
    "#,
    );

    assert!(truth(
        &mut vm,
        "waypointPosition missing isEqualTo [0, 0, 0]"
    ));
    assert!(truth(&mut vm, "waypointStatements missing isEqualTo []"));
    assert!(truth(&mut vm, "waypointTimeout missing isEqualTo []"));
    assert_eq!(text(&mut vm, "waypointType missing"), "");
    assert_eq!(num(&mut vm, "waypointVisible missing"), 0.0);
    assert!(!truth(&mut vm, "waypointForceBehaviour missing"));
    // The valid waypoint is untouched by the empty timeout.
    assert_eq!(text(&mut vm, "waypointType w"), "");
    assert!(truth(&mut vm, "waypointTimeout w isEqualTo [0, 0, 0]"));
}

#[test]
fn waypoint_of_a_group_that_is_gone_is_empty() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        w = g addWaypoint [[0, 0], 0];
        deleteGroup g;
    "#,
    );

    assert!(truth(
        &mut vm,
        "isNil {waypoints g} or {count waypoints g == 0}"
    ));
    assert_eq!(text(&mut vm, "waypointType w"), "");
    assert!(truth(&mut vm, "waypointPosition w isEqualTo [0, 0, 0]"));
}
