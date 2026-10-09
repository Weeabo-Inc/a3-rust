//! SQF event-handler commands, run as scripts against a synthetic World.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, dispatch_events, object_arg, register_world_commands};
use a3_world::{ClientId, Locality, ObjectRef, TypeBank, World};

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

/// A group with one unit in it.
const UNIT: &str = r#"
    g = createGroup west;
    a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
"#;

#[test]
fn handlers_are_added_per_type_and_info_reports_them() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            {UNIT}
            h0 = a addEventHandler ["Killed", {{}}];
            h1 = a addEventHandler ["Killed", {{}}];
            h2 = a addEventHandler ["Hit", {{}}];
        "#
        ),
    );

    assert_eq!(num(&mut vm, "h0"), 0.0);
    assert_eq!(num(&mut vm, "h1"), 1.0);
    // Ids are per event type.
    assert_eq!(num(&mut vm, "h2"), 0.0);
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Killed", 0] isEqualTo [true, false, 2]"#
    ));
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Killed", 1] isEqualTo [true, true, 2]"#
    ));
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Hit", 0] isEqualTo [true, true, 1]"#
    ));
    // A type with no handlers reads empty; a free id reads `[false, false, total]`.
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Fired", 0] isEqualTo []"#
    ));
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Killed", 7] isEqualTo [false, false, 2]"#
    ));
}

#[test]
fn a_removed_id_is_reused_and_the_unknown_target_is_nothing() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            {UNIT}
            a addEventHandler ["Killed", {{}}];
            a addEventHandler ["Killed", {{}}];
            a removeEventHandler ["Killed", 0];
            reused = a addEventHandler ["Killed", {{}}];
        "#
        ),
    );

    assert_eq!(num(&mut vm, "reused"), 0.0);
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Killed", 0] isEqualTo [true, false, 2]"#
    ));

    // A null target returns Nothing and removes nothing.
    let value = eval(
        &mut vm,
        r#"objNull addEventHandler ["Killed", {}]; objNull removeEventHandler ["Killed", 0]"#,
    );
    assert!(
        matches!(value, Value::Nothing),
        "expected Nothing: {value:?}"
    );
}

#[test]
fn remove_all_clears_one_event_type() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            {UNIT}
            a addEventHandler ["Killed", {{}}];
            a addEventHandler ["Killed", {{}}];
            a addEventHandler ["Hit", {{}}];
            a removeAllEventHandlers "Killed";
        "#
        ),
    );

    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Killed", 0] isEqualTo []"#
    ));
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["Hit", 0] isEqualTo [true, true, 1]"#
    ));
}

#[test]
fn mp_handlers_share_the_object_table() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            {UNIT}
            a addEventHandler ["MPKilled", {{}}];
            second = a addMPEventHandler ["MPKilled", {{}}];
        "#
        ),
    );

    assert_eq!(num(&mut vm, "second"), 1.0);
    assert!(truth(
        &mut vm,
        r#"a getEventHandlerInfo ["MPKilled", 1] isEqualTo [true, true, 2]"#
    ));
}

#[test]
fn a_group_keeps_its_own_handlers() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g1 = createGroup west;
        g2 = createGroup east;
        g1 addEventHandler ["Killed", {}];
    "#,
    );

    assert!(truth(
        &mut vm,
        r#"g1 getEventHandlerInfo ["Killed", 0] isEqualTo [true, true, 1]"#
    ));
    assert!(truth(
        &mut vm,
        r#"g2 getEventHandlerInfo ["Killed", 0] isEqualTo []"#
    ));
}

#[test]
fn mission_handlers_run_when_an_entity_is_created() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            seen_count = 0;
            seen_event = "";
            seen_id = -1;
            seen_obj = objNull;
            h = addMissionEventHandler ["EntityCreated", {{
                seen_count = seen_count + 1;
                seen_event = _thisEvent;
                seen_id = _thisEventHandler;
                seen_obj = _this select 0;
            }}];
            {UNIT}
        "#
        ),
    );

    assert_eq!(num(&mut vm, "h"), 0.0);
    assert_eq!(num(&mut vm, "seen_count"), 1.0);
    assert_eq!(text(&mut vm, "seen_event"), "EntityCreated");
    assert_eq!(num(&mut vm, "seen_id"), 0.0);
    assert!(truth(&mut vm, "seen_obj isEqualTo a"));
    // Mission handler info is queried without a target.
    assert!(truth(
        &mut vm,
        r#"getEventHandlerInfo ["EntityCreated", 0] isEqualTo [true, true, 1]"#
    ));
}

#[test]
fn a_mission_handlers_third_element_is_this_args() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            seen_args = -1;
            addMissionEventHandler ["EntityCreated", {{seen_args = _thisArgs}}, 7];
            {UNIT}
        "#
        ),
    );

    assert_eq!(num(&mut vm, "seen_args"), 7.0);
}

#[test]
fn a_handler_may_be_a_string_statement() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            seen = 0;
            addMissionEventHandler ["EntityCreated", "seen = 42"];
            {UNIT}
        "#
        ),
    );

    assert_eq!(num(&mut vm, "seen"), 42.0);
}

#[test]
fn a_removed_mission_handler_stops_running() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            seen_count = 0;
            addMissionEventHandler ["EntityCreated", {{seen_count = seen_count + 1}}];
            removeMissionEventHandler ["EntityCreated", 0];
            {UNIT}
        "#
        ),
    );

    assert_eq!(num(&mut vm, "seen_count"), 0.0);
    assert!(truth(
        &mut vm,
        r#"getEventHandlerInfo ["EntityCreated", 0] isEqualTo []"#
    ));
}

#[test]
fn the_local_event_fires_on_the_object_and_drains_once() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"
            local_count = 0;
            local_event = "";
            local_id = -1;
            local_this = [];
            {UNIT}
            a addEventHandler ["Local", {{
                local_count = local_count + 1;
                local_event = _thisEvent;
                local_id = _thisEventHandler;
                local_this = _this;
            }}];
        "#
        ),
    );

    let value = eval(&mut vm, "a");
    let object = object_arg(&vm.host.world, &value).expect("the unit exists");
    let ObjectRef::Entity(id) = object else {
        panic!("the unit is not an Entity");
    };

    vm.host
        .world
        .set_locality(id, Locality::Remote { owner: None })
        .unwrap();
    dispatch_events(&mut vm);

    assert_eq!(num(&mut vm, "local_count"), 1.0);
    assert_eq!(text(&mut vm, "local_event"), "Local");
    assert_eq!(num(&mut vm, "local_id"), 0.0);
    assert!(truth(&mut vm, "local_this select 0 isEqualTo a"));
    assert!(truth(&mut vm, "local_this select 1 == false"));

    // The queue is drained: a second dispatch runs nothing until a new event is queued.
    dispatch_events(&mut vm);
    assert_eq!(num(&mut vm, "local_count"), 1.0);

    vm.host.world.set_locality(id, Locality::Local).unwrap();
    dispatch_events(&mut vm);
    assert_eq!(num(&mut vm, "local_count"), 2.0);
    assert!(truth(&mut vm, "local_this select 1"));
}

#[test]
fn a_completed_waypoint_runs_the_groups_waypoint_complete_handler() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"{UNIT}
            done = [];
            g addEventHandler ["WaypointComplete", {{ done pushBack _this }}];
            g addWaypoint [[0, 0, 0], 0];
            "#
        ),
    );

    // The leader stands on the waypoint, so the first AI steps complete it.
    for _ in 0..10 {
        vm.host.world.simulate(0.1);
    }
    dispatch_events(&mut vm);

    assert_eq!(num(&mut vm, "count done"), 1.0);
    assert!(truth(&mut vm, "(done select 0) isEqualTo [g, 1]"));
}

#[test]
fn a_destroyed_unit_runs_its_killed_handlers() {
    let mut vm = vm();
    eval(
        &mut vm,
        &format!(
            r#"{UNIT}
            killed = [];
            a addEventHandler ["Killed", {{ killed = _this }}];
            a setDamage 1;
            "#
        ),
    );
    dispatch_events(&mut vm);

    assert!(truth(&mut vm, "(killed select 0) isEqualTo a"));
    assert!(truth(&mut vm, "isNull (killed select 1)"));
    assert!(truth(&mut vm, "isNull (killed select 2)"));
    assert!(truth(&mut vm, "killed select 3"));
}
