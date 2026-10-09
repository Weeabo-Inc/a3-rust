//! Running a loaded and spawned mission's scripts: the units' `init` fields (unscheduled, `this`
//! bound), then `init.sqf` as a scheduled script.

mod common;

use a3_gamedata::VfsHost;
use a3_mission::{
    MAX_INIT_FRAMES, MissionVmHost, RunReport, Spawned, load_mission, run_scripts, spawn_mission,
};
use a3_sqf::{Handle, Namespace, Value, Vm};
use a3_world::ObjectRef;
use a3_world::script::object_value;

const FOLDER: &str = "missions\\test.altis";

/// Two WEST units (a leader named `boss`, the player named `shooter`), one EAST unit whose `init`
/// suspends — which the engine does not allow in the unscheduled init phase — and one marker.
const SQM: &str = r#"version=12;
class Mission
{
	class Groups
	{
		items=2;
		class Item0
		{
			side="WEST";
			class Vehicles
			{
				items=2;
				class Item0
				{
					id=7;
					position[]={1000,2000,0};
					vehicle="B_Soldier_F";
					leader=1;
					text="boss";
					init="order = ""boss"";";
				};
				class Item1
				{
					id=3;
					position[]={1010,2020,0};
					vehicle="B_soldier_AR_F";
					player="PLAYER COMMANDER";
					init="order = format [""%1|player"", order]; shooter = this;";
				};
			};
		};
		class Item1
		{
			side="EAST";
			class Vehicles
			{
				items=1;
				class Item0
				{
					id=11;
					position[]={3000,4000,0};
					vehicle="O_Soldier_F";
					text="suspender";
					init="waitUntil { false };";
				};
			};
		};
	};
	class Markers
	{
		items=1;
		class Item0
		{
			position[]={100,200,0};
			name="marker_start";
			text="Start";
			type="Empty";
		};
	};
};
"#;

/// `init.sqf` of the fixture: appends to `order` (which the unit `init`s built), suspends once so
/// the scheduler must run more than one frame, then marks itself done.
const INIT_SQF: &str = r#"order = format ["%1|init.sqf", order];
sleep 0.2;
ticked = true;
diag_log "init.sqf done";
// A code value naming commands the VM has no implementation for, for the missing-command report.
BIS_fnc_probe = { mapAnimAdd [0, 0.2, 0]; onMapSingleClick "true"; };
"#;

const DESCRIPTION: &str =
    "author = \"Tester\";\nclass CfgDebriefing { class Victory { title=\"V\"; }; };\n";

/// Loads and spawns the fixture mission and runs its scripts. The World is `vm.host.world`; the
/// directory has to stay alive for the VFS's loose files.
fn load_spawn_run() -> (Vm<MissionVmHost>, Spawned, RunReport, tempfile::TempDir) {
    let (dir, vfs) = common::mount(
        &[
            ("mission.sqm", SQM),
            ("init.sqf", INIT_SQF),
            ("description.ext", DESCRIPTION),
        ],
        FOLDER,
    );
    let mut loader = Vm::new(VfsHost::new(vfs.clone()));
    let mission = load_mission(&vfs, FOLDER, &mut loader).expect("the fixture loads");

    let (mut world, mut types) = common::world_and_types();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    let mut vm = MissionVmHost::new(world, types, VfsHost::new(vfs)).vm();
    let report = run_scripts(&mut vm, &mission, &spawned);
    (vm, spawned, report, dir)
}

/// The `Value::Handle` of `name` in the mission namespace.
fn handle(vm: &Vm<MissionVmHost>, name: &str) -> Handle {
    match vm.get_global(name) {
        Value::Handle(h) => h,
        value => panic!("{name} is not an object: {value:?}"),
    }
}

/// The handle the world hands out for the Entity.
fn entity_handle(vm: &Vm<MissionVmHost>, id: a3_world::EntityId) -> Handle {
    match object_value(&vm.host.world, ObjectRef::Entity(id)) {
        Value::Handle(h) => h,
        value => panic!("not an object: {value:?}"),
    }
}

fn text(vm: &Vm<MissionVmHost>, name: &str) -> String {
    match vm.get_global(name) {
        Value::String(s) => s.to_string(),
        value => panic!("{name} is not a string: {value:?}"),
    }
}

fn flag(vm: &Vm<MissionVmHost>, name: &str) -> bool {
    match vm.get_global(name) {
        Value::Bool(b) => b,
        value => panic!("{name} is not a bool: {value:?}"),
    }
}

#[test]
fn unit_inits_run_first_with_this_bound_and_init_sqf_runs_after_them() {
    let (vm, spawned, report, _dir) = load_spawn_run();

    // Each unit's `init` ran unscheduled with `this` = that unit, so the globals are the Entities.
    assert_eq!(handle(&vm, "boss"), entity_handle(&vm, spawned.units[&7]));
    assert_eq!(
        handle(&vm, "shooter"),
        entity_handle(&vm, spawned.units[&3])
    );

    // `init.sqf` saw what the unit `init`s had written: the order field ends with its own step.
    assert_eq!(text(&vm, "order"), "boss|player|init.sqf");

    let names: Vec<&str> = report.scripts.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "init of boss",
            "init of unit 3",
            "init of suspender",
            "init.sqf"
        ]
    );
    assert!(report.scripts[0].ok && report.scripts[1].ok && report.scripts[3].ok);
}

#[test]
fn the_player_and_the_named_units_become_mission_namespace_variables() {
    let (mut vm, spawned, report, _dir) = load_spawn_run();

    // `player` is the World's player; named units also get their vehicle variable name.
    assert_eq!(vm.host.world.player(), Some(spawned.units[&3]));
    match vm.eval("player").unwrap() {
        Value::Handle(h) => assert_eq!(h, entity_handle(&vm, spawned.units[&3])),
        value => panic!("player is not an object: {value:?}"),
    }
    let _ = handle(&vm, "boss");
    assert_eq!(vm.eval("str boss").unwrap().to_sqf_string(), "\"boss\"");
    // `boss` and `suspender` come from `text=`, the player's `shooter` from its init's `this`.
    assert_eq!(report.variables, 2, "boss and suspender");
    assert_eq!(report.markers, 1, "marker_start is carried, not installed");
}

#[test]
fn a_unit_init_that_suspends_is_an_error_and_the_rest_still_runs() {
    let (_vm, _spawned, report, _dir) = load_spawn_run();

    let failed = report
        .scripts
        .iter()
        .find(|s| s.name == "init of suspender")
        .expect("the suspending unit's init is reported");
    assert!(!failed.ok);
    let error = failed.error.as_deref().expect("an error");
    assert!(error.contains("Suspend"), "{error}");

    // The error is on the host's list, and everything after it still ran.
    assert!(
        report.errors.iter().any(|e| e.contains("Suspend")),
        "{:?}",
        report.errors
    );
    let init = report
        .scripts
        .iter()
        .find(|s| s.name == "init.sqf")
        .expect("init.sqf ran");
    assert!(init.ok, "{init:?}");
}

#[test]
fn init_sqf_is_scheduled_and_the_world_is_stepped_until_it_finishes() {
    let (vm, _spawned, report, _dir) = load_spawn_run();

    assert!(flag(&vm, "ticked"), "the code after the sleep ran");
    assert!(
        report.frames >= 1,
        "init.sqf suspended, so the scheduler ran frames: {}",
        report.frames
    );
    assert!(
        report.frames < MAX_INIT_FRAMES,
        "it finished: {}",
        report.frames
    );
    // `sleep` resumes on the host's clock, which is the World's: the World was stepped.
    assert!(vm.host.world.time() > 0.0);
    assert_eq!(report.log_lines, 1, "diag_log \"init.sqf done\"");
}

#[test]
fn commands_the_vm_cannot_run_yet_are_reported_from_the_mission_namespace() {
    let (_vm, _spawned, report, _dir) = load_spawn_run();

    // The probe code value names two commands with no implementation. If both ever get one, pick
    // another obscure command for the probe: this test is about the report, not those commands.
    assert!(
        report.missing_commands.iter().any(|(name, _)| {
            name.starts_with("mapAnimAdd") || name.starts_with("onMapSingleClick")
        }),
        "{:?}",
        report.missing_commands
    );
    // Reported as `name (form)`, with the use count.
    let (name, count) = &report.missing_commands[0];
    assert!(name.contains('('), "{name}");
    assert!(*count >= 1);
}

/// A mission whose only unit's init calls a command the VM has no implementation for, and that
/// stores nothing.
const UNIMPLEMENTED_INIT_SQM: &str = r#"version=12;
class Mission
{
	class Groups
	{
		items=1;
		class Item0
		{
			side="WEST";
			class Vehicles
			{
				items=1;
				class Item0
				{
					id=1;
					position[]={100,200,0};
					vehicle="B_Soldier_F";
					init="this enableMimics false;";
				};
			};
		};
	};
};
"#;

#[test]
fn commands_the_unit_init_fields_use_are_reported_even_though_they_store_nothing() {
    let (_dir, vfs) = common::mount(&[("mission.sqm", UNIMPLEMENTED_INIT_SQM)], FOLDER);
    let mut loader = Vm::new(VfsHost::new(vfs.clone()));
    let mission = load_mission(&vfs, FOLDER, &mut loader).expect("the fixture loads");

    let (mut world, mut types) = common::world_and_types();
    let spawned = spawn_mission(&mut world, &mut types, &mission);
    let mut vm = MissionVmHost::new(world, types, VfsHost::new(vfs)).vm();
    let report = run_scripts(&mut vm, &mission, &spawned);

    let init = report
        .scripts
        .iter()
        .find(|s| s.name == "init of unit 1")
        .expect("the init ran");
    assert!(!init.ok, "the command has no implementation: {init:?}");

    // The command is in the report although the init stored no code value anywhere: the report
    // covers the compiled inits themselves.
    assert!(
        report
            .missing_commands
            .iter()
            .any(|(name, _)| name.starts_with("enableMimics")),
        "{:?}",
        report.missing_commands
    );
}

#[test]
fn description_ext_is_visible_to_mission_config_scripts() {
    let (mut vm, _spawned, _report, _dir) = load_spawn_run();

    let code = vm
        .compile_file(
            "probe.sqf",
            "author = getMissionConfigValue [\"author\", \"\"];",
        )
        .expect("compiles");
    vm.call_in(&code, None, Namespace::Mission).expect("runs");
    assert_eq!(text(&vm, "author"), "Tester");
}
