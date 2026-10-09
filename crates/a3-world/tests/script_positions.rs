//! Position arithmetic commands (`distance`, `getDir`, relative `getPos`, `inArea`,
//! `inPolygon`). The expected values are the ones the server oracle recorded for the probes in
//! `tools/oracle/probes/95_world_stratis.probes` and `96_world_vr.probes`.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class AllVehicles: All {};
    class Land: AllVehicles {};
    class LandVehicle: Land {};
    class Car: LandVehicle { simulation = "carx"; };
    class C_Offroad_01_F: Car { scope = 2; };
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

fn number(vm: &mut Vm<ScriptWorld>, code: &str) -> f64 {
    match eval(vm, code) {
        Value::Number(n) => f64::from(n),
        other => panic!("{code}: {other:?}"),
    }
}

fn boolean(vm: &mut Vm<ScriptWorld>, code: &str) -> bool {
    match eval(vm, code) {
        Value::Bool(b) => b,
        other => panic!("{code}: {other:?}"),
    }
}

fn array(vm: &mut Vm<ScriptWorld>, code: &str) -> Vec<f64> {
    match eval(vm, code) {
        Value::Array(items) => items
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Number(n) => f64::from(*n),
                other => panic!("{code}: element {other:?}"),
            })
            .collect(),
        other => panic!("{code}: {other:?}"),
    }
}

/// The oracle prints values with six significant digits, our floats are f32: compare the way the
/// engine prints them.
fn close(got: f64, expected: f64) {
    assert!(
        (got - expected).abs() <= expected.abs() * 1e-5 + 1e-9,
        "{got} is not {expected}"
    );
}

#[test]
fn distance2d_ignores_the_height() {
    let mut vm = vm();

    // wvr / vec.distance2d_arrays
    close(number(&mut vm, "[0, 0, 0] distance2D [3, 4, 100]"), 5.0);
}

#[test]
fn distance_is_between_the_points() {
    let mut vm = vm();

    close(number(&mut vm, "[0, 0, 0] distance [3, 4, 0]"), 5.0);
}

#[test]
fn distancesqr_is_the_squared_distance() {
    let mut vm = vm();

    close(number(&mut vm, "[0, 0, 0] distanceSqr [3, 4, 0]"), 25.0);
}

#[test]
fn distance_takes_objects_as_well_as_positions() {
    let mut vm = vm();
    eval(&mut vm, "a = [0, 0, 0]");
    eval(&mut vm, "b = [30, 40, 0]");

    close(number(&mut vm, "a distance2D b"), 50.0);
    close(number(&mut vm, "a distance b"), 50.0);
}

#[test]
fn get_dir_between_positions_is_the_compass_direction() {
    let mut vm = vm();

    // vec.dir_to / vec.dir_to_neg
    close(number(&mut vm, "[0, 0, 0] getDir [1, 1, 0]"), 45.0);
    close(number(&mut vm, "[0, 0, 0] getDir [-1, 0, 0]"), 270.0);
    close(number(&mut vm, "[0, 0, 0] getDir [0, 1, 0]"), 0.0);
}

#[test]
fn get_pos_relative_moves_along_the_heading() {
    let mut vm = vm();

    // vec.getpos_relative: the engine's right angle is not exactly a right angle.
    let p = array(&mut vm, "[0, 0, 0] getPos [10, 90]");
    close(p[0], 10.0);
    close(p[1], -1.62921e-06);
    close(p[2], 0.0);
}

#[test]
fn get_pos_relative_is_diagonal_at_45_degrees_and_returns_the_terrain_height() {
    let mut vm = vm();

    // vec.getpos_relative_45
    let p = array(&mut vm, "[100, 100, 5] getPos [10, 45]");
    close(p[0], 107.071);
    close(p[1], 107.071);
    close(p[2], 0.0);
}

#[test]
fn in_area_checks_an_ellipse_and_a_rotated_rectangle() {
    let mut vm = vm();

    // vec.inarea_circle / vec.inarea_rect_rot
    assert!(boolean(
        &mut vm,
        "[1, 1, 0] inArea [[0, 0, 0], 2, 2, 0, false]"
    ));
    assert!(boolean(
        &mut vm,
        "[3, 0, 0] inArea [[0, 0, 0], 2, 4, 90, true]"
    ));
    assert!(!boolean(
        &mut vm,
        "[3, 0, 0] inArea [[0, 0, 0], 2, 4, 0, true]"
    ));
}

#[test]
fn in_polygon_ignores_the_height() {
    let mut vm = vm();

    // vec.inpolygon
    assert!(boolean(
        &mut vm,
        "[1, 1, 0] inPolygon [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]]"
    ));
    assert!(!boolean(
        &mut vm,
        "[3, 1, 0] inPolygon [[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]]"
    ));
}
