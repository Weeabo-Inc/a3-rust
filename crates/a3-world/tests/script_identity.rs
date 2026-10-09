//! Object and group variables, the player, identities, vehicle variable names, synchronization
//! and dynamic simulation, run as scripts against a synthetic World.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Handle, Host, Registry, ScriptError, Value, Vm};
use a3_world::script::{ScriptWorld, WorldHost, register_world_commands};
use a3_world::{ClientId, EntityId, ObjectRef, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class Car: All { simulation = "carx"; };
    class C_Offroad_01_F: Car { scope = 2; };
    class Thing: All { simulation = "thing"; };
    class Land_Crate_F: Thing { scope = 2; };
};
class CfgIdentities {
    class Miller {
        name = "Captain Miller";
        face = "WhiteHead_06";
        glasses = "G_Aviator";
        speaker = "Male01ENGB";
        pitch = 1.05;
        nameSound = "Miller";
    };
};
"#;

/// A [`ScriptWorld`] that keeps the errors the VM reports. A command error is logged and
/// the script goes on (server oracle), so a test that checks one reads it here instead of
/// expecting `eval` to fail.
#[derive(Debug)]
struct RecordingWorld {
    inner: ScriptWorld,
    errors: Vec<String>,
}

impl RecordingWorld {
    fn new(inner: ScriptWorld) -> RecordingWorld {
        RecordingWorld {
            inner,
            errors: Vec::new(),
        }
    }
}

impl std::ops::Deref for RecordingWorld {
    type Target = ScriptWorld;
    fn deref(&self) -> &ScriptWorld {
        &self.inner
    }
}

impl std::ops::DerefMut for RecordingWorld {
    fn deref_mut(&mut self) -> &mut ScriptWorld {
        &mut self.inner
    }
}

impl Host for RecordingWorld {
    fn time(&self) -> f32 {
        self.inner.time()
    }

    fn is_null(&self, handle: Handle) -> bool {
        self.inner.is_null(handle)
    }

    fn format_handle(&self, handle: Handle) -> String {
        self.inner.format_handle(handle)
    }

    fn report_error(&mut self, error: &ScriptError) {
        self.errors.push(error.report.clone());
    }
}

impl WorldHost for RecordingWorld {
    fn world(&self) -> &World {
        self.inner.world()
    }

    fn world_mut(&mut self) -> &mut World {
        self.inner.world_mut()
    }

    fn types(&mut self) -> &mut TypeBank {
        self.inner.types()
    }
}

type TestVm = Vm<RecordingWorld>;

fn vm() -> TestVm {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    let mut vm = Vm::with_registry(
        RecordingWorld::new(ScriptWorld::new(World::new(ClientId::SERVER), types)),
        Rc::new(registry),
    );
    eval(
        &mut vm,
        r#"
        g = createGroup west;
        a = g createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
        b = g createUnit ["B_Soldier_F", [5, 0, 0], [], 0, "NONE"];
        car = "C_Offroad_01_F" createVehicle [20, 0, 0];
        crate = "Land_Crate_F" createVehicle [30, 0, 0];
    "#,
    );
    vm
}

fn eval(vm: &mut TestVm, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

/// Runs `code`, which must be an error the original logs and survives, and returns the
/// report: the statement's value is the empty value and the error is on the host.
fn error(vm: &mut TestVm, code: &str) -> String {
    vm.host.errors.clear();
    let value = eval(vm, code);
    assert!(
        value.is_nil(),
        "{code}: expected the empty value, got {value:?}"
    );
    vm.host
        .errors
        .pop()
        .unwrap_or_else(|| panic!("{code}: no error reported"))
}

fn text(vm: &mut TestVm, code: &str) -> String {
    match eval(vm, code) {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

fn truth(vm: &mut TestVm, code: &str) -> bool {
    eval(vm, code)
        .as_bool()
        .unwrap_or_else(|| panic!("{code}: not a bool"))
}

fn entity(vm: &mut TestVm, name: &str) -> EntityId {
    let value = eval(vm, name);
    match a3_world::script::object_arg(&vm.host.world, &value) {
        Some(ObjectRef::Entity(id)) => id,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn object_and_group_variables() {
    let mut vm = vm();
    eval(
        &mut vm,
        r#"a setVariable ["Score", 5]; g setVariable ["task", "hold", true];
           crate setVariable ["Contents", [1, 2]];"#,
    );
    assert_eq!(text(&mut vm, "a getVariable 'score'"), "5");
    assert_eq!(text(&mut vm, "a getVariable 'SCORE'"), "5");
    assert_eq!(text(&mut vm, "g getVariable 'task'"), "hold");
    assert_eq!(text(&mut vm, "crate getVariable 'contents'"), "[1,2]");
    // Per object: b has none.
    assert!(truth(&mut vm, "isNil {b getVariable 'score'}"));
    assert_eq!(text(&mut vm, "b getVariable ['score', 7]"), "7");
    assert_eq!(text(&mut vm, "a getVariable ['score', 7]"), "5");
    // nil deletes; a nil value gives the default.
    eval(&mut vm, "a setVariable ['score', nil]");
    assert_eq!(text(&mut vm, "a getVariable ['score', 7]"), "7");
    eval(&mut vm, "a setVariable ['x', 1]; a setVariable ['b', 2]");
    assert_eq!(text(&mut vm, "allVariables a"), r#"["b","x"]"#);
    assert_eq!(text(&mut vm, "allVariables g"), r#"["task"]"#);
    // Null targets: nil / the default / nothing, and no argument checks for setVariable.
    assert!(truth(&mut vm, "isNil {objNull getVariable 'x'}"));
    assert_eq!(text(&mut vm, "objNull getVariable ['x', 3]"), "3");
    assert_eq!(text(&mut vm, "grpNull getVariable ['x', 3]"), "3");
    eval(&mut vm, "objNull setVariable []");
    assert_eq!(text(&mut vm, "allVariables objNull"), "[]");
}

#[test]
fn variable_argument_errors() {
    let mut vm = vm();
    assert!(error(&mut vm, "a setVariable []").contains("0 elements provided, 1 expected"));
    assert!(error(&mut vm, "a setVariable ['x']").contains("1 elements provided, 3 expected"));
    assert!(
        error(&mut vm, "a setVariable ['x', 1, 2, 3]").contains("4 elements provided, 3 expected")
    );
    assert!(error(&mut vm, "a setVariable [1, 2]").contains("Type Number, expected String"));
    assert!(error(&mut vm, "a setVariable ['x', 1, 'all']").contains("Type String, expected"));
    assert!(error(&mut vm, "a getVariable ['x']").contains("1 elements provided, 2 expected"));
    assert!(
        error(&mut vm, "objNull getVariable ['x', 1, 2]")
            .contains("3 elements provided, 2 expected")
    );
    // An empty name is accepted and ignored.
    eval(&mut vm, "a setVariable ['', 1]");
    assert_eq!(text(&mut vm, "allVariables a"), "[]");
}

#[test]
fn variables_go_with_their_object() {
    let mut vm = vm();
    let crate_id = entity(&mut vm, "crate");
    let owner = a3_world::VarOwner::Object(ObjectRef::Entity(crate_id));
    eval(&mut vm, "crate setVariable ['x', 1]");
    assert!(vm.host.world.variables(owner).is_some());
    eval(&mut vm, "deleteVehicle crate");
    vm.host.world.flush_deletions();
    assert!(truth(&mut vm, "isNil {crate getVariable 'x'}"));
    assert!(vm.host.world.variables(owner).is_none());
}

#[test]
fn player_camera_vehicle_and_is_player() {
    let mut vm = vm();
    assert_eq!(text(&mut vm, "player"), "<NULL-object>");
    assert_eq!(text(&mut vm, "cameraOn"), "<NULL-object>");
    let a = entity(&mut vm, "a");
    vm.host.world.set_player(Some(a));
    assert!(truth(&mut vm, "player == a"));
    assert!(truth(&mut vm, "cameraOn == a"));
    assert!(truth(&mut vm, "isPlayer a"));
    assert!(truth(&mut vm, "isPlayer [a]"));
    assert!(!truth(&mut vm, "isPlayer b"));
    assert!(!truth(&mut vm, "isPlayer car"));
    assert!(!truth(&mut vm, "isPlayer objNull"));
    assert!(!truth(&mut vm, "isPlayer []"));
    // Nobody is in a vehicle yet: everything is its own vehicle; objNull stays null.
    assert!(truth(&mut vm, "vehicle a == a"));
    assert!(truth(&mut vm, "vehicle car == car"));
    assert!(truth(&mut vm, "isNull vehicle objNull"));
    eval(&mut vm, "deleteVehicle a");
    vm.host.world.flush_deletions();
    assert!(truth(&mut vm, "isNull player"));
}

#[test]
fn names_and_identity() {
    let mut vm = vm();
    assert_eq!(text(&mut vm, "name a"), "");
    eval(&mut vm, "a setName 'John Smith'");
    assert_eq!(text(&mut vm, "name a"), "John Smith");
    eval(&mut vm, "b setName ['Jane Doe', 'Jane', 'Doe']");
    assert_eq!(text(&mut vm, "name b"), "Jane Doe");
    assert!(error(&mut vm, "b setName ['x', 'y']").contains("2 elements provided, 3 expected"));
    assert!(error(&mut vm, "b setName ['x', 'y', 3]").contains("Type Number, expected String"));
    // A null target skips the checks.
    eval(&mut vm, "objNull setName ['x']");
    assert_eq!(text(&mut vm, "name objNull"), "Error: No vehicle");
    assert_eq!(text(&mut vm, "name car"), "Error: No unit");
    // setName on a vehicle does nothing.
    eval(&mut vm, "car setName 'Betty'");
    assert_eq!(text(&mut vm, "name car"), "Error: No unit");

    eval(
        &mut vm,
        "a setFace 'AfricanHead_01'; a setSpeaker 'Male02ENG'; a setPitch 0.9; a setNameSound 'smith'",
    );
    assert_eq!(text(&mut vm, "face a"), "AfricanHead_01");
    assert_eq!(text(&mut vm, "speaker a"), "Male02ENG");
    assert_eq!(text(&mut vm, "pitch a"), "0.9");
    assert_eq!(text(&mut vm, "nameSound a"), "smith");
    assert_eq!(text(&mut vm, "pitch b"), "1");
    assert_eq!(text(&mut vm, "pitch car"), "-1");
    assert_eq!(text(&mut vm, "face car"), "");
    assert_eq!(text(&mut vm, "speaker objNull"), "");

    eval(&mut vm, "b setIdentity 'Miller'");
    assert_eq!(text(&mut vm, "name b"), "Captain Miller");
    assert_eq!(text(&mut vm, "face b"), "WhiteHead_06");
    assert_eq!(text(&mut vm, "speaker b"), "Male01ENGB");
    assert_eq!(text(&mut vm, "pitch b"), "1.05");
    assert_eq!(text(&mut vm, "nameSound b"), "Miller");
    // An unknown identity changes nothing.
    eval(&mut vm, "b setIdentity 'Nobody'");
    assert_eq!(text(&mut vm, "name b"), "Captain Miller");
}

#[test]
fn vehicle_var_name_names_the_object_in_str() {
    let mut vm = vm();
    assert_eq!(text(&mut vm, "vehicleVarName car"), "");
    eval(
        &mut vm,
        "car setVehicleVarName 'bluecar'; a setVehicleVarName 'alpha'",
    );
    assert_eq!(text(&mut vm, "vehicleVarName car"), "bluecar");
    assert_eq!(text(&mut vm, "str car"), "bluecar");
    assert_eq!(text(&mut vm, "str a"), "alpha");
    assert_eq!(text(&mut vm, "str b"), "B Alpha 1-1:2");
    eval(&mut vm, "a setVehicleVarName ''");
    assert_eq!(text(&mut vm, "str a"), "B Alpha 1-1:1");
    assert_eq!(text(&mut vm, "vehicleVarName objNull"), "");
}

#[test]
fn synchronization_links_units_both_ways() {
    let mut vm = vm();
    eval(&mut vm, "a synchronizeObjectsAdd [b, car, objNull]");
    assert!(truth(&mut vm, "synchronizedObjects a isEqualTo [b, car]"));
    assert!(truth(&mut vm, "synchronizedObjects b isEqualTo [a]"));
    // A vehicle without a unit keeps no list.
    assert!(truth(&mut vm, "synchronizedObjects car isEqualTo []"));
    // Adding twice keeps one link.
    eval(&mut vm, "b synchronizeObjectsAdd [a]");
    assert!(truth(&mut vm, "synchronizedObjects b isEqualTo [a]"));
    // A vehicle cannot start a link to another vehicle.
    eval(&mut vm, "car synchronizeObjectsAdd [crate]");
    assert!(truth(&mut vm, "synchronizedObjects crate isEqualTo []"));
    eval(&mut vm, "a synchronizeObjectsRemove [b]");
    assert!(truth(&mut vm, "synchronizedObjects a isEqualTo [car]"));
    assert!(truth(&mut vm, "synchronizedObjects b isEqualTo []"));
    assert!(
        error(&mut vm, "a synchronizeObjectsAdd [b, 1]").contains("Type Number, expected Object")
    );
    assert!(
        error(&mut vm, "objNull synchronizeObjectsRemove [1]")
            .contains("Type Number, expected Object")
    );
    // Add returns before checking when the source is null.
    eval(&mut vm, "objNull synchronizeObjectsAdd [1]");
}

#[test]
fn dynamic_simulation_flags() {
    let mut vm = vm();
    assert!(!truth(&mut vm, "dynamicSimulationEnabled car"));
    eval(
        &mut vm,
        "car enableDynamicSimulation true; g enableDynamicSimulation true",
    );
    assert!(truth(&mut vm, "dynamicSimulationEnabled car"));
    assert!(truth(&mut vm, "dynamicSimulationEnabled g"));
    eval(&mut vm, "car enableDynamicSimulation false");
    assert!(!truth(&mut vm, "dynamicSimulationEnabled car"));
    assert!(!truth(&mut vm, "dynamicSimulationEnabled objNull"));
    assert!(!truth(&mut vm, "dynamicSimulationEnabled grpNull"));
}
