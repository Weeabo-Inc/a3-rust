//! SQF marker commands, run as scripts against a synthetic World.

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

fn nums(vm: &mut Vm<ScriptWorld>, code: &str) -> Vec<f64> {
    let v = eval(vm, code);
    v.as_array()
        .unwrap_or_else(|| panic!("{code}: not an array"))
        .borrow()
        .iter()
        .map(|n| f64::from(n.as_number().unwrap_or(f32::NAN)))
        .collect()
}

fn num(vm: &mut Vm<ScriptWorld>, code: &str) -> f64 {
    eval(vm, code)
        .as_number()
        .unwrap_or_else(|| panic!("{code}: not a number"))
        .into()
}

#[test]
fn create_marker_returns_its_name_and_refuses_a_taken_one() {
    let mut vm = vm();

    assert_eq!(text(&mut vm, r#"createMarker ["m1", [100, 200]]"#), "m1");
    // A name that is taken: the command is ignored and reports "".
    assert_eq!(text(&mut vm, r#"createMarker ["m1", [0, 0]]"#), "");
    assert_eq!(text(&mut vm, r#"createMarkerLocal ["m2", [0, 0]]"#), "m2");
    assert_eq!(nums(&mut vm, r#"markerPos "m1""#), vec![100.0, 200.0, 0.0]);
}

#[test]
fn a_new_marker_has_the_engine_defaults() {
    let mut vm = vm();
    eval(&mut vm, r#"createMarker ["m1", [100, 200]]"#);

    assert_eq!(text(&mut vm, r#"markerShape "m1""#), "ICON");
    assert_eq!(nums(&mut vm, r#"markerSize "m1""#), vec![1.0, 1.0]);
    assert_eq!(num(&mut vm, r#"markerAlpha "m1""#), 1.0);
    assert_eq!(num(&mut vm, r#"markerDir "m1""#), 0.0);
    assert!(truth(&mut vm, r#"markerShadow "m1""#));
    // A marker draws nothing until a type is set.
    assert_eq!(text(&mut vm, r#"markerType "m1""#), "");
}

#[test]
fn marker_fields_set_and_get() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        createMarker ["m1", [100, 200]];
        "m1" setMarkerType "hd_dot";
        "m1" setMarkerText "Objective";
        "m1" setMarkerColor "ColorRed";
        "m1" setMarkerShape "rectangle";
        "m1" setMarkerSize [100, 50];
        "m1" setMarkerDir 90;
        "m1" setMarkerAlpha 0.5;
        "m1" setMarkerBrush "Border";
        "m1" setMarkerPos [300, 400, 5];
        "m1" setMarkerShadow false;
    "#,
    );

    assert_eq!(text(&mut vm, r#"markerType "m1""#), "hd_dot");
    assert_eq!(text(&mut vm, r#"markerText "m1""#), "Objective");
    assert_eq!(text(&mut vm, r#"markerColor "m1""#), "ColorRed");
    // The shape is stored and reported uppercased.
    assert_eq!(text(&mut vm, r#"markerShape "m1""#), "RECTANGLE");
    assert_eq!(nums(&mut vm, r#"markerSize "m1""#), vec![100.0, 50.0]);
    assert_eq!(num(&mut vm, r#"markerDir "m1""#), 90.0);
    assert_eq!(num(&mut vm, r#"markerAlpha "m1""#), 0.5);
    assert_eq!(text(&mut vm, r#"markerBrush "m1""#), "Border");
    assert!(!truth(&mut vm, r#"markerShadow "m1""#));

    // `getMarkerPos` reports [x, y, 0] unless the elevation is asked for.
    assert_eq!(
        nums(&mut vm, r#"getMarkerPos "m1""#),
        vec![300.0, 400.0, 0.0]
    );
    assert_eq!(
        nums(&mut vm, r#"getMarkerPos ["m1", true]"#),
        vec![300.0, 400.0, 5.0]
    );
}

#[test]
fn a_marker_can_be_created_at_an_objects_position() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [10, 20, 0], [], 0, "NONE"];
        createMarker ["m1", a];
    "#,
    );
    let pos = nums(&mut vm, r#"markerPos "m1""#);
    assert!((pos[0] - 10.0).abs() < 0.001, "{pos:?}");
    assert!((pos[1] - 20.0).abs() < 0.001, "{pos:?}");
}

#[test]
fn all_map_markers_lists_by_draw_priority() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        createMarker ["a", [0, 0]];
        createMarker ["b", [0, 0]];
        createMarker ["c", [0, 0]];
        "a" setMarkerDrawPriority 10;
        "b" setMarkerDrawPriority -5;
        "c" setMarkerDrawPriority 0;
    "#,
    );

    assert_eq!(text(&mut vm, r#"str allMapMarkers"#), r#"["b","c","a"]"#);
    assert_eq!(num(&mut vm, r#"markerDrawPriority "a""#), 10.0);
}

#[test]
fn delete_marker_removes_it_and_its_accessors_go_empty() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        createMarker ["m1", [1, 2]];
        createMarkerLocal ["m2", [3, 4]];
        "m1" setMarkerType "hd_dot";
        deleteMarker "m1";
    "#,
    );

    assert_eq!(text(&mut vm, r#"str allMapMarkers"#), r#"["m2"]"#);
    assert_eq!(text(&mut vm, r#"markerType "m1""#), "");
    assert_eq!(nums(&mut vm, r#"markerPos "m1""#), vec![0.0, 0.0, 0.0]);

    eval(&mut vm, r#"deleteMarkerLocal "m2""#);
    assert!(truth(&mut vm, "allMapMarkers isEqualTo []"));
}

#[test]
fn unknown_marker_accessors_are_empty_and_setters_do_nothing() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        "nope" setMarkerType "hd_dot";
        "nope" setMarkerPos [1, 2];
        "nope" setMarkerSize [5, 5];
    "#,
    );

    assert_eq!(text(&mut vm, r#"markerType "nope""#), "");
    assert_eq!(text(&mut vm, r#"markerText "nope""#), "");
    assert_eq!(text(&mut vm, r#"markerBrush "nope""#), "");
    assert_eq!(text(&mut vm, r#"markerColor "nope""#), "");
    assert_eq!(nums(&mut vm, r#"markerSize "nope""#), vec![0.0, 0.0]);
    assert_eq!(num(&mut vm, r#"markerAlpha "nope""#), 0.0);
    assert!(!truth(&mut vm, r#"markerShadow "nope""#));
    assert!(truth(&mut vm, "allMapMarkers isEqualTo []"));
}

#[test]
fn polyline_points_round_trip() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"
        createMarker ["shape", [0, 0]];
        "shape" setMarkerShape "polyline";
        "shape" setMarkerPolyline [[0, 0], [100, 50], [200, 0]];
    "#,
    );

    assert_eq!(text(&mut vm, r#"markerShape "shape""#), "POLYLINE");
    assert_eq!(
        text(&mut vm, r#"str markerPolyline "shape""#),
        "[[0,0,0],[100,50,0],[200,0,0]]"
    );
}
