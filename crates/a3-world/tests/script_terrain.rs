//! Terrain and environment commands (`worldSize`, `getTerrainHeightASL`, `surfaceIsWater`,
//! `surfaceNormal`, the ASL/AGL conversions, ray casts, `date`/`overcast`/`fog`) over a synthetic
//! terrain. The expectations are the ones the server oracle recorded for the same commands
//! (`tools/oracle/probes/95_world_stratis.probes`, `96_world_vr.probes`).

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, TypeBank, World};
use a3_wrp::{Terrain, TerrainBuilder};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class Thing: All { simulation = "thing"; };
    class Land_Crate_F: Thing { scope = 2; };
};
"#;

/// A 4 x 4 land grid of 50 m cells (200 m square) whose height samples rise 1 m per sample
/// eastwards: 25 m per metre, so the surface normal is the same everywhere. The middle is a
/// ridge: the sample heights also rise 1 m per sample northwards, then the far half drops.
fn terrain() -> Arc<Terrain> {
    Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .heights(|i, _| i as f32)
            .build(),
    )
}

fn vm() -> Vm<ScriptWorld> {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut world = World::new(ClientId::SERVER);
    world.load_terrain(terrain()).unwrap();
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(ScriptWorld::new(world, types), Rc::new(registry))
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

fn array(vm: &mut Vm<ScriptWorld>, code: &str) -> Vec<Value> {
    match eval(vm, code) {
        Value::Array(items) => items.borrow().clone(),
        other => panic!("{code}: {other:?}"),
    }
}

fn bool_of(vm: &mut Vm<ScriptWorld>, code: &str) -> bool {
    match eval(vm, code) {
        Value::Bool(b) => b,
        other => panic!("{code}: {other:?}"),
    }
}

fn close(got: f64, expected: f64) {
    assert!((got - expected).abs() <= 1e-4, "{got} is not {expected}");
}

#[test]
fn worlds_size_is_the_terrain_edge() {
    let mut vm = vm();

    // 4 land cells of 50 m.
    close(number(&mut vm, "worldSize"), 200.0);
}

#[test]
fn terrain_height_ignores_the_height_argument() {
    let mut vm = vm();

    // Heights rise 1 m per sample, samples are 25 m apart: 100 m east is +4 m.
    close(number(&mut vm, "getTerrainHeightASL [100, 0]"), 4.0);
    close(number(&mut vm, "getTerrainHeightASL [100, 0, 1000]"), 4.0);
    close(number(&mut vm, "getTerrainHeightASL [50, 50]"), 2.0);
}

#[test]
fn surface_is_water_below_sea_level() {
    let mut vm = vm();
    let submerged = Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .heights(|i, _| i as f32 - 5.0)
            .build(),
    );
    vm.host.world.load_terrain(submerged).unwrap();

    assert!(bool_of(&mut vm, "surfaceIsWater [0, 0]"));
    assert!(!bool_of(&mut vm, "surfaceIsWater [190, 0]"));
}

#[test]
fn surface_normal_follows_the_slope() {
    let mut vm = vm();

    // The height rises 1 m per sample eastwards (25 m), so the normal tilts west.
    let n = array(&mut vm, "surfaceNormal [100, 100]");
    let got: Vec<f64> = n
        .iter()
        .map(|v| match v {
            Value::Number(n) => f64::from(*n),
            other => panic!("{other:?}"),
        })
        .collect();
    let len = (1.0 + 25.0f64 * 25.0).sqrt();
    close(got[0], -1.0 / len);
    close(got[1], 0.0);
    close(got[2], 25.0 / len);
}

#[test]
fn agl_and_asl_conversions_are_inverse() {
    let mut vm = vm();

    // wstr.agl_to_asl / wstr.asl_to_agl / wstr.atl_to_asl / wstr.asl_to_atl, on our terrain
    // (the height at [100, 0] is 4 m).
    let asl = array(&mut vm, "AGLToASL [100, 0, 0]");
    close(number_of(&asl[2]), 4.0);
    let agl = array(&mut vm, "ASLToAGL [100, 0, 300]");
    close(number_of(&agl[2]), 296.0);
    let atl = array(&mut vm, "ATLToASL [100, 0, 2]");
    close(number_of(&atl[2]), 6.0);
    let back = array(&mut vm, "ASLToATL [100, 0, 300]");
    close(number_of(&back[2]), 296.0);
}

fn number_of(value: &Value) -> f64 {
    match value {
        Value::Number(n) => f64::from(*n),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_clock_and_weather_start_at_the_oracle_values() {
    let mut vm = vm();

    // wstr.date, wstr.overcast, wstr.fog, wstr.sunormoon.
    let date = array(&mut vm, "date");
    let date: Vec<f64> = date.iter().map(number_of).collect();
    assert_eq!(date, [2035.0, 6.0, 24.0, 12.0, 0.0]);
    close(number(&mut vm, "overcast"), 0.0);
    close(number(&mut vm, "fog"), 0.0);
    close(number(&mut vm, "sunOrMoon"), 1.0);
}

#[test]
fn a_vertical_line_hits_the_terrain_from_above() {
    let mut vm = vm();

    // wvr.lis_ground: the hit is the surface height, the normal its surface normal, and the
    // Object is objNull (terrain).
    let hits = array(
        &mut vm,
        "lineIntersectsSurfaces [[100, 100, 1000], [100, 100, -100]]",
    );
    assert_eq!(hits.len(), 1, "one terrain hit");
    let hit = match &hits[0] {
        Value::Array(items) => items.borrow().clone(),
        other => panic!("{other:?}"),
    };
    let position: Vec<f64> = match &hit[0] {
        Value::Array(items) => items.borrow().iter().map(number_of).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!([position[0], position[1]], [100.0, 100.0]);
    close(position[2], 4.0);
    assert!(
        matches!(hit[2], Value::Handle(h) if h.is_null()),
        "terrain has no Object"
    );
}

#[test]
fn a_line_that_misses_the_terrain_reports_nothing() {
    let mut vm = vm();

    // Both ends far above the surface.
    assert!(!bool_of(
        &mut vm,
        "terrainIntersect [[10, 10, 300], [190, 190, 300]]"
    ));
    assert!(
        array(
            &mut vm,
            "lineIntersectsSurfaces [[10, 10, 1000], [190, 190, 900]]"
        )
        .is_empty()
    );
}

#[test]
fn terrain_intersect_sees_a_hill_in_the_way() {
    let mut vm = vm();

    // The sample heights rise to 7 m at the east edge, so a line at 2 m above the ground at
    // [0, 0] and [25, 0] (heights 0 and 1) passes under the ground further east.
    assert!(bool_of(
        &mut vm,
        "terrainIntersectASL [[0, 0, 1], [190, 0, 1]]"
    ));
    assert!(!bool_of(
        &mut vm,
        "terrainIntersectASL [[0, 0, 100], [190, 0, 100]]"
    ));
}
