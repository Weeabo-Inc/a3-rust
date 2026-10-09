//! Loading a mission folder out of the VFS: `mission.sqm`, `description.ext` (through the
//! preprocessor) and the terrain named by the folder's extension.

mod common;

use a3_config::{EntryKind, Value, write_rap};
use a3_gamedata::VfsHost;
use a3_mission::{LoadError, Mission, load_mission, parse_sqm};
use a3_sqf::Vm;

/// The smallest SQM that places one WEST soldier on the map.
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
					text="player_unit";
				};
			};
		};
	};
};
"#;

/// A description.ext that exercises the preprocessor (`#define`, `#ifdef`) and a class.
const DESCRIPTION: &str = r#"#define VERSION "1.0"
class CfgDebriefing
{
	class Victory
	{
		title="Victory";
		description=VERSION;
	};
};
#ifdef DEBUG
class DebugParams {};
#endif
"#;

/// Loads `files` as the mission folder `folder` (each file path is relative to that folder).
fn load(files: &[(&str, &str)], folder: &str) -> Result<Mission, LoadError> {
    load_with_log(files, folder).0
}

/// [`load`], plus the host log the load wrote to (engine warnings go there, as the RPT).
fn load_with_log(
    files: &[(&str, &str)],
    folder: &str,
) -> (Result<Mission, LoadError>, Vec<String>) {
    let (_dir, vfs) = common::mount(files, folder);
    let mut vm = Vm::new(VfsHost::new(vfs.clone()));
    let result = load_mission(&vfs, folder, &mut vm);
    (result, vm.host.log.clone())
}

fn folder_of(files: &[(&str, &str)]) -> Result<Mission, LoadError> {
    load(files, "missions\\test.altis")
}

#[test]
fn loads_the_sqm_the_description_ext_and_the_terrain() {
    let mission = folder_of(&[("mission.sqm", SQM), ("description.ext", DESCRIPTION)])
        .expect("the folder loads");

    assert_eq!(mission.folder, "missions\\test.altis");
    assert_eq!(mission.terrain.as_deref(), Some("altis"));
    assert_eq!(mission.units().count(), 1);
    assert_eq!(
        mission.unit(1).map(|u| u.class.as_str()),
        Some("B_Soldier_F")
    );

    let description = mission.description.as_ref().expect("description.ext");
    let victory = description
        .root
        .class("CfgDebriefing")
        .and_then(|c| c.class("Victory"))
        .expect("class CfgDebriefing / class Victory");
    // `description=VERSION` came from the `#define`, so the preprocessor ran.
    let entry = victory.get("description").expect("description");
    assert_eq!(
        entry.kind,
        EntryKind::Value(Value::String("1.0".to_owned()))
    );
    // `#ifdef DEBUG` is not defined, so its class is not in the output.
    assert!(description.root.class("DebugParams").is_none());
}

#[test]
fn an_include_next_to_the_mission_is_preprocessed() {
    let mission = folder_of(&[
        ("mission.sqm", SQM),
        (
            "description.ext",
            "#include \"params.hpp\"\nclass CfgDebriefing { class Victory { title=\"V\"; }; };\n",
        ),
        (
            "params.hpp",
            "class Params { class TimeLimit { title=\"Time\"; default=10; }; };\n",
        ),
    ])
    .expect("the folder loads");

    let description = mission.description.as_ref().expect("description.ext");
    assert!(description.root.class("Params").is_some());
    assert!(description.root.class("CfgDebriefing").is_some());
}

#[test]
fn forward_slashes_and_a_trailing_separator_are_accepted() {
    // Mounted at `missions/test.altis` (forward slashes) and loaded with a leading and trailing
    // separator: both normalise to the same virtual path.
    let mission = load(&[("mission.sqm", SQM)], "/missions/test.altis/").unwrap();
    assert_eq!(mission.folder, "missions\\test.altis");
    assert_eq!(mission.units().count(), 1);
}

#[test]
fn the_folder_extension_names_the_terrain() {
    let mission = load(&[("mission.sqm", SQM)], "missions\\test.altis").unwrap();
    assert_eq!(mission.terrain.as_deref(), Some("altis"));

    let mission = load(&[("mission.sqm", SQM)], "missions\\test.stratis").unwrap();
    assert_eq!(mission.terrain.as_deref(), Some("stratis"));

    // No extension, no terrain.
    let mission = load(&[("mission.sqm", SQM)], "missions\\nodot").unwrap();
    assert_eq!(mission.terrain, None);
}

#[test]
fn a_folder_without_a_mission_sqm_is_an_error() {
    let err = folder_of(&[("readme.txt", "not a mission")]).unwrap_err();
    match err {
        LoadError::NoSqm(path) => assert!(path.ends_with("mission.sqm"), "{path}"),
        err => panic!("expected NoSqm, got {err}"),
    }
}

#[test]
fn a_broken_sqm_names_its_path() {
    let err = folder_of(&[("mission.sqm", "class Mission {")]).unwrap_err();
    match err {
        LoadError::Sqm { path, .. } => assert!(path.ends_with("mission.sqm"), "{path}"),
        err => panic!("expected Sqm, got {err}"),
    }
}

#[test]
fn a_missing_include_leaves_the_mission_loaded_without_a_description() {
    // #348: the shipped `repro_objectsimulationloadgame.tanoa` includes a file that does not
    // exist. The engine logs `Preprocessor failed on file '<path>' - error 1`, drops the whole
    // config - entries before the include are gone too - and starts the mission anyway
    // (oracle: tools/oracle/probes/missing_include.py, `include` case).
    let (mission, log) = load_with_log(
        &[
            ("mission.sqm", SQM),
            (
                "description.ext",
                "class Header { gameType = \"Sandbox\"; };\n\
                 class CfgDebriefing { class Victory { title = \"V\"; }; };\n\
                 #include \"does_not_exist.inc\"\n\
                 class CfgDebriefingAfter { class Victory { title = \"V\"; }; };\n",
            ),
        ],
        "missions\\test.altis",
    );
    let mission = mission.expect("the engine starts the mission with an empty mission config");
    assert!(
        mission.description.is_none(),
        "a config that failed to preprocess installs nothing"
    );
    // Only the config is lost: the units of `mission.sqm` are all there.
    assert_eq!(mission.units().count(), 1);
    assert!(
        log.iter().any(|line| line.contains("does_not_exist.inc")),
        "the failure is logged, as `Cannot include file` is in the RPT: {log:?}"
    );
}

#[test]
fn a_broken_description_ext_leaves_the_mission_loaded_without_one() {
    // The same for a config that fails to parse: the config is dropped, the mission is not.
    let (mission, log) = load_with_log(
        &[
            ("mission.sqm", SQM),
            ("description.ext", "class CfgDebriefing {\n"),
        ],
        "missions\\test.altis",
    );
    let mission = mission.expect("the engine starts the mission with an empty mission config");
    assert!(mission.description.is_none());
    assert_eq!(mission.units().count(), 1);
    assert!(
        log.iter().any(|line| line.contains("description.ext")),
        "the failure is logged: {log:?}"
    );
}

#[test]
fn a_rapified_sqm_loads_the_same_mission() {
    let text = parse_sqm(SQM.as_bytes()).unwrap();
    let rap = write_rap(&text);
    assert!(a3_config::is_rap(&rap));

    let (_dir, vfs) = common::mount_bytes(&[("mission.sqm", &rap)], "missions\\test.altis");
    let mut vm = Vm::new(VfsHost::new(vfs.clone()));
    let mission = load_mission(&vfs, "missions\\test.altis", &mut vm).unwrap();

    assert_eq!(mission.version, 12);
    assert_eq!(mission.units().count(), 1);
    assert_eq!(
        mission.unit(1).map(|u| u.class.as_str()),
        Some("B_Soldier_F")
    );
}
