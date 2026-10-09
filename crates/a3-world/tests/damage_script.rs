//! The SQF damage commands (`docs/re/sim-damage.md` §8), run as scripts against a synthetic
//! World.
//!
//! The library API behind them is covered by `tests/damage.rs`; these tests check what a script
//! sees: the command names and argument shapes, the `allowDamage` gate, and the promotion of a
//! Static object that a damage command is given.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, EntityId, TypeBank, World, WorldEvent};
use a3_wrp::{TerrainBuilder, Transform};
use glam::Vec3;

/// A breakable house with a `depends` hit point and a ruin whose class the config names, that
/// ruin class itself, a soldier with a `Total` dependency and a car.
const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class House: All { simulation = "house"; armor = 20; };
    class Land_House_Script_F: House {
        scope = 2;
        model = "\A3\Structures_F\House_Script_F.p3d";
        class HitPoints {
            class HitWall { armor = 1; name = "wall"; };
            class HitRoof { armor = 1; name = "roof"; depends = "HitWall max 0.25"; };
        };
        class DestructionEffects {
            class Ruin1 { simulation = "ruin"; type = "\A3\Structures_F\House_Script_ruins_F.p3d"; };
        };
    };
    class Land_House_Script_ruins_F: House {
        scope = 2;
        model = "\A3\Structures_F\House_Script_ruins_F.p3d";
        class HitPoints { class HitRuin { armor = 1; name = "ruin"; } };
    };
    class Man_sim: All { simulation = "soldier"; };
    class B_Soldier_Script_F: Man_sim {
        scope = 2;
        class HitPoints {
            class HitHead { armor = 1; name = "head"; };
            class HitBody { armor = 1; name = "body"; depends = "Total"; };
            class HitFace { armor = 1; name = "face"; };
        };
    };
    class Car_sim: All { simulation = "carx"; };
    class C_Car_Script_F: Car_sim {
        scope = 2;
        class HitPoints {
            class HitFuel { armor = 1; name = "fueltank"; };
            class HitHull { armor = 1; name = "hull"; };
            class HitEngine { armor = 1; name = "engine"; };
        };
    };
};
"#;

/// A World over a synthetic terrain with the config's model resolver installed — what a host
/// wires with `world.set_model_type_resolver(Some(types.resolver()))`. Two Static objects: the
/// breakable house at (120, 0, 160), WRP Object ID 0, and a rock at (30, 0, 30), ID 1, whose
/// model no config class uses.
fn vm(client: ClientId) -> Vm<ScriptWorld> {
    let at = |x, z| Transform::from_position(Vec3::new(x, 0.0, z));
    let terrain = Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .object(r"a3\structures_f\house_script_f.p3d", at(120.0, 160.0))
            .object(r"a3\rocks_f\stone.p3d", at(30.0, 30.0))
            .build(),
    );
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut world = World::new(client);
    world.load_terrain(terrain).unwrap();
    world.set_model_type_resolver(Some(types.resolver()));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    Vm::with_registry(ScriptWorld::new(world, types), Rc::new(registry))
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn text(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    match eval(vm, code) {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

fn truth(vm: &mut Vm<ScriptWorld>, code: &str) -> bool {
    eval(vm, code)
        .as_bool()
        .unwrap_or_else(|| panic!("{code}: not a bool"))
}

/// A command's NUMBER result, as `f64` (the VM carries `f32`).
fn num(vm: &mut Vm<ScriptWorld>, code: &str) -> f64 {
    f64::from(
        eval(vm, code)
            .as_number()
            .unwrap_or_else(|| panic!("{code}: not a number")),
    )
}

fn assert_close(got: f64, want: f64) {
    assert!((got - want).abs() < 1e-4, "{got} != {want}");
}

/// The strings of an SQF array value.
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value:?}: not an array"))
        .borrow()
        .iter()
        .map(|v| match v {
            Value::String(s) => s.to_string(),
            other => other.to_sqf_string(),
        })
        .collect()
}

/// The three arrays of `getAllHitPointsDamage`: hit point names, model selections, damage values.
fn all_hit_points(vm: &mut Vm<ScriptWorld>, code: &str) -> (Vec<String>, Vec<String>, Vec<f64>) {
    let value = eval(vm, code);
    let outer: Vec<Value> = (*value.as_array().expect("not an array").borrow()).clone();
    assert_eq!(outer.len(), 3, "{code}: not [names, selections, values]");
    let damage = outer[2]
        .as_array()
        .expect("not an array")
        .borrow()
        .iter()
        .map(|v| f64::from(v.as_number().unwrap()))
        .collect();
    (strings(&outer[0]), strings(&outer[1]), damage)
}

/// The Entity an SQF Object value refers to.
fn entity(vm: &mut Vm<ScriptWorld>, name: &str) -> EntityId {
    let value = eval(vm, name);
    match a3_world::script::object_arg(&vm.host.world, &value) {
        Some(a3_world::ObjectRef::Entity(id)) => id,
        other => panic!("{name}: {other:?}"),
    }
}

// ---------------------------------------------------------------- the total

#[test]
fn the_damage_commands_read_and_write_the_total() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"c = "C_Car_Script_F" createVehicle [0, 0, 0]"#);

    assert_close(num(&mut vm, "damage c"), 0.0);
    assert!(truth(&mut vm, "alive c"));

    eval(&mut vm, "c setDamage 0.5");

    assert_close(num(&mut vm, "damage c"), 0.5);
    // `getDammage` is the old name of `damage`.
    assert_close(num(&mut vm, "getDammage c"), 0.5);
    assert!(truth(&mut vm, "alive c"), "0.5 does not destroy");

    eval(&mut vm, "c setDamage 1");
    assert!(!truth(&mut vm, "alive c"));
    assert_close(num(&mut vm, "damage c"), 1.0);

    eval(&mut vm, "c setDamage 0");
    assert!(truth(&mut vm, "alive c"), "a script can restore it");
}

#[test]
fn a_total_change_reaches_the_hit_points_that_depend_on_it() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"m = "B_Soldier_Script_F" createVehicle [0, 0, 0]"#,
    );

    eval(&mut vm, "m setDamage 0.5");

    // `HitBody` depends on `Total`; `HitHead` depends on nothing.
    assert_close(num(&mut vm, r#"m getHitPointDamage "HitBody""#), 0.5);
    assert_close(num(&mut vm, r#"m getHitPointDamage "HitHead""#), 0.0);
    assert!(truth(&mut vm, "alive m"));
}

#[test]
fn set_damage_takes_the_array_form_with_the_effects_flag_and_the_killer() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"
        h = "Land_House_Script_F" createVehicle [0, 0, 0];
        k = "C_Car_Script_F" createVehicle [20, 0, 0];
        i = "C_Car_Script_F" createVehicleLocal [25, 0, 0];
    "#,
    );
    let h = entity(&mut vm, "h");
    let (k, i) = (entity(&mut vm, "k"), entity(&mut vm, "i"));
    vm.host.world.drain_events();

    // `setDamage [damage, useEffects, killer, instigator]`; false keeps the destruction effects
    // (the ruin) off.
    eval(&mut vm, "h setDamage [1, false, k, i]");

    assert!(!truth(&mut vm, "alive h"));
    assert_eq!(text(&mut vm, "typeOf h"), "Land_House_Script_F", "no ruin");
    assert_eq!(
        vm.host.world.drain_events(),
        [WorldEvent::Killed {
            entity: h,
            killer: Some(k),
            instigator: Some(i),
            use_effects: false,
        }]
    );
}

// ---------------------------------------------------------------- the hit points

#[test]
fn set_hit_point_damage_addresses_a_hit_point_by_name_or_selection() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"h = "Land_House_Script_F" createVehicle [0, 0, 0]"#,
    );

    eval(&mut vm, r#"h setHitPointDamage ["HitWall", 0.5]"#);

    assert_close(num(&mut vm, r#"h getHitPointDamage "HitWall""#), 0.5);
    // `getHit` reads the same hit point by its model selection.
    assert_close(num(&mut vm, r#"h getHit "wall""#), 0.5);
    // `HitRoof` depends on `HitWall`: `max(0.5, 0.25)`.
    assert_close(num(&mut vm, r#"h getHitPointDamage "HitRoof""#), 0.5);
    // A hit point does not move the total.
    assert_close(num(&mut vm, "damage h"), 0.0);
    assert!(truth(&mut vm, "alive h"));

    // A direct write wins over the dependency: the 0.2 is not pulled back up to 0.5.
    eval(&mut vm, r#"h setHitPointDamage ["HitRoof", 0.2]"#);
    assert_close(num(&mut vm, r#"h getHitPointDamage "HitRoof""#), 0.2);
}

#[test]
fn set_hit_and_set_hit_index_reach_the_same_hit_points() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"c = "C_Car_Script_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, r#"c setHit ["fueltank", 0.25]"#);
    assert_close(num(&mut vm, r#"c getHit "fueltank""#), 0.25);
    assert_close(num(&mut vm, r#"c getHitPointDamage "HitFuel""#), 0.25);

    eval(&mut vm, r#"c setHitIndex [1, 0.75]"#);
    assert_close(num(&mut vm, "c getHitIndex 1"), 0.75);
    assert_close(num(&mut vm, r#"c getHitPointDamage "HitHull""#), 0.75);
    // An index out of range.
    assert_close(num(&mut vm, "c getHitIndex 9"), 0.0);
}

#[test]
fn get_all_hit_points_damage_returns_names_selections_and_values() {
    let mut vm = vm(ClientId::SERVER);
    eval(
        &mut vm,
        r#"m = "B_Soldier_Script_F" createVehicle [0, 0, 0]"#,
    );

    let (names, selections, damage) = all_hit_points(&mut vm, "getAllHitPointsDamage m");
    assert_eq!(names, ["HitHead", "HitBody", "HitFace"]);
    assert_eq!(selections, ["head", "body", "face"]);
    assert_eq!(damage, [0.0, 0.0, 0.0]);

    eval(&mut vm, r#"m setHitPointDamage ["HitFace", 0.5]"#);
    let (_, _, damage) = all_hit_points(&mut vm, "getAllHitPointsDamage m");
    assert_eq!(damage, [0.0, 0.0, 0.5], "HitFace written directly");
}

#[test]
fn a_fatal_hit_point_destroys_the_object() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"c = "C_Car_Script_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, r#"c setHitPointDamage ["HitHull", 1]"#);

    assert!(!truth(&mut vm, "alive c"), "a vehicle's fatal hit point");
    assert_close(num(&mut vm, r#"c getHitPointDamage "HitHull""#), 1.0);
}

#[test]
fn unknown_hit_points_read_zero_and_setting_them_does_nothing() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"c = "C_Car_Script_F" createVehicle [0, 0, 0]"#);

    assert_close(num(&mut vm, r#"c getHitPointDamage "Nope""#), 0.0);
    assert_close(num(&mut vm, r#"c getHit "nope""#), 0.0);

    eval(
        &mut vm,
        r#"
        c setHitPointDamage ["Nope", 1];
        c setHit ["nope", 1];
    "#,
    );
    let (_, _, damage) = all_hit_points(&mut vm, "getAllHitPointsDamage c");
    assert_eq!(damage, [0.0, 0.0, 0.0]);
}

#[test]
fn allow_damage_gates_set_hit_point_damage_but_not_the_other_damage_commands() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, r#"c = "C_Car_Script_F" createVehicle [0, 0, 0]"#);

    eval(&mut vm, "c allowDamage false");
    assert!(!truth(&mut vm, "isDamageAllowed c"));

    // The command's own page: no effect while `allowDamage` is false.
    eval(&mut vm, r#"c setHitPointDamage ["HitEngine", 0.5]"#);
    assert_close(num(&mut vm, r#"c getHitPointDamage "HitEngine""#), 0.0);

    // `setHit`, `setHitIndex` and `setDamage` are not gated.
    eval(&mut vm, r#"c setHit ["hull", 0.5]"#);
    assert_close(num(&mut vm, r#"c getHit "hull""#), 0.5);
    eval(&mut vm, "c setDamage 0.5");
    assert_close(num(&mut vm, "damage c"), 0.5);

    eval(&mut vm, "c allowDamage true");
    assert!(truth(&mut vm, "isDamageAllowed c"));
    eval(&mut vm, r#"c setHitPointDamage ["HitEngine", 0.25]"#);
    assert_close(num(&mut vm, r#"c getHitPointDamage "HitEngine""#), 0.25);
}

// ---------------------------------------------------------------- Static objects

#[test]
fn a_damage_command_promotes_a_static_object_with_its_config_class() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, "h = nearestObject [[120, 0, 160], 0]");
    assert_eq!(text(&mut vm, "getObjectID h"), "0");
    assert_eq!(
        text(&mut vm, "typeOf h"),
        "",
        "a Static object has no class"
    );

    eval(&mut vm, r#"h setHitPointDamage ["wall", 0.6]"#);

    assert_eq!(
        text(&mut vm, "typeOf h"),
        "Land_House_Script_F",
        "the model path resolved to its class"
    );
    assert_close(num(&mut vm, r#"h getHitPointDamage "HitWall""#), 0.6);
    // The house's `depends` hit point came with the class.
    assert_close(num(&mut vm, r#"h getHitPointDamage "HitRoof""#), 0.6);
    assert!(!truth(&mut vm, "isNull h"), "the handle stays");
    assert_eq!(text(&mut vm, "getObjectID h"), "0");
}

#[test]
fn a_destroyed_static_object_becomes_its_ruin() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, "h = nearestObject [[120, 0, 160], 0]");

    eval(&mut vm, "h setDamage 1");

    assert!(!truth(&mut vm, "alive h"));
    assert_close(num(&mut vm, "damage h"), 1.0);
    // The class the config names for the ruin's model, not a name guessed from it.
    assert_eq!(text(&mut vm, "typeOf h"), "Land_House_Script_ruins_F");
    assert_eq!(
        all_hit_points(&mut vm, "getAllHitPointsDamage h").0,
        ["HitRuin"]
    );
    assert!(truth(&mut vm, r#"objectFromNetId (netId h) isEqualTo h"#));
    assert!(
        truth(&mut vm, "isNull (nearestObject [[120, 0, 160], 0])"),
        "the terrain object is gone; the ruin took its place"
    );
}

#[test]
fn a_static_object_without_a_config_class_is_promoted_plain() {
    let mut vm = vm(ClientId::SERVER);
    eval(&mut vm, "r = nearestObject [[30, 0, 30], 1]");
    assert_eq!(text(&mut vm, "getObjectID r"), "1");
    assert_eq!(text(&mut vm, "typeOf r"), "");

    eval(&mut vm, "r setDamage 0.5");

    assert_close(num(&mut vm, "damage r"), 0.5);
    assert_eq!(text(&mut vm, "typeOf r"), "", "a plain type has no class");
    let (names, selections, damage) = all_hit_points(&mut vm, "getAllHitPointsDamage r");
    assert!(
        names.is_empty() && selections.is_empty() && damage.is_empty(),
        "a plain type has no hit points"
    );
}
