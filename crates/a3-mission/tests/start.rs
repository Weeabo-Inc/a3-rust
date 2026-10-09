//! Starting a mission without waiting for it: the function library's mission start, the unit
//! `init` fields, then `init.sqf` spawned; afterwards the mission runs frame by frame.

mod common;

use std::sync::Arc;
use std::time::Duration;

use a3_config::{ConfigTree, parse_text};
use a3_gamedata::{SqfConfigs, VfsHost};
use a3_mission::{MissionVmHost, StartOptions, load_mission, spawn_mission, start_mission, step};
use a3_sqf::{Value, Vm};

const FOLDER: &str = "missions\\start.altis";

const SQM: &str = r#"version=12;
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
					player="PLAYER COMMANDER";
					leader=1;
					init="if (isNil ""order"") then {order = []}; order pushBack ""unit init"";";
				};
			};
		};
	};
};
"#;

/// `init.sqf`: records its turn, then sleeps for two seconds of mission time.
const INIT_SQF: &str = r#"order pushBack "init.sqf";
sleep 2;
woke = true;
"#;

/// The function library's init script, as `configFile >> "CfgFunctions" >> "init"` names it.
/// It runs before the unit inits, unscheduled, with `_this` undefined, in missionNamespace.
const FUNCTIONS_INIT: &str = r#"order = ["functions"];
this_was_nil = isNil "_this";
[] spawn { post_init = true; };
"#;

const GAME_CONFIG: &str = r#"
class CfgFunctions { init = "fn\initFunctions.sqf"; };
"#;

fn started(options: StartOptions) -> (Vm<MissionVmHost>, a3_mission::RunReport, tempfile::TempDir) {
    let (dir, vfs) = common::mount(&[("mission.sqm", SQM), ("init.sqf", INIT_SQF)], FOLDER);
    // The init script lives outside the mission, in an addon.
    let functions = dir.path().join("fn");
    std::fs::create_dir_all(&functions).unwrap();
    std::fs::write(functions.join("initFunctions.sqf"), FUNCTIONS_INIT).unwrap();
    vfs.mount_dir(&functions, a3_vfs::VfsPath::new("fn"))
        .unwrap();

    let mut loader = Vm::new(VfsHost::new(vfs.clone()));
    let mission = load_mission(&vfs, FOLDER, &mut loader).expect("the fixture loads");
    let (mut world, mut types) = common::world_and_types();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    let mut files = VfsHost::new(vfs);
    let config = parse_text(GAME_CONFIG).unwrap();
    files.configs = SqfConfigs::new(Arc::new(ConfigTree::from_config(&config)));
    let mut vm = MissionVmHost::new(world, types, files).vm();
    let report = start_mission(&mut vm, &mission, &spawned, options);
    (vm, report, dir)
}

fn strings(value: Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .borrow()
            .iter()
            .map(|v| match v {
                Value::String(s) => s.to_string(),
                other => panic!("not a string: {other:?}"),
            })
            .collect(),
        other => panic!("not an array: {other:?}"),
    }
}

#[test]
fn the_function_library_starts_before_the_unit_inits() {
    let (vm, report, _dir) = started(StartOptions { functions: true });

    assert_eq!(
        strings(vm.get_global("order")),
        ["functions", "unit init"],
        "{report:?}"
    );
    assert_eq!(vm.get_global("this_was_nil"), Value::Bool(true));
    assert_eq!(report.scripts[0].name, "functions init");
    assert!(report.scripts[0].ok, "{report:?}");
}

#[test]
fn without_functions_the_init_script_does_not_run() {
    let (vm, report, _dir) = started(StartOptions::default());

    assert_eq!(strings(vm.get_global("order")), ["unit init"], "{report:?}");
    assert!(report.scripts.iter().all(|s| s.name != "functions init"));
}

#[test]
fn init_sqf_is_spawned_but_not_waited_for() {
    let (mut vm, report, _dir) = started(StartOptions { functions: true });

    let handle = report.init_sqf.expect("init.sqf was spawned");
    assert!(!vm.script_done(handle));
    assert_eq!(report.frames, 0);
    // The postInit spawn and init.sqf are both waiting for their first frame.
    assert!(vm.scheduled_count() >= 2);

    // One frame: init.sqf runs to its sleep, the spawned post-init code finishes.
    step(&mut vm, 0.1, Duration::from_millis(50));
    assert_eq!(
        strings(vm.get_global("order")),
        ["functions", "unit init", "init.sqf"]
    );
    assert_eq!(vm.get_global("post_init"), Value::Bool(true));
    assert!(vm.get_global("woke") == Value::Nil);

    // Mission time advances with the World; after two seconds the sleep is over.
    for _ in 0..25 {
        step(&mut vm, 0.1, Duration::from_millis(50));
    }
    assert!(vm.host.world.time() > 2.0);
    assert_eq!(vm.get_global("woke"), Value::Bool(true));
    assert!(vm.script_done(handle));
}
