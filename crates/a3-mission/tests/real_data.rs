//! Loads a shipped campaign mission out of the real game install. Skipped when `A3_ROOT` is
//! unset.
//!
//! `cargo test -p a3-mission --release --test real_data -- --nocapture` prints the mission's
//! structure, what spawned, and which commands the mission's scripts need that the VM does not
//! implement yet.

use a3_gamedata::{GameData, LoadOptions, VfsHost, unimplemented_usage};
use a3_mission::{MAX_INIT_FRAMES, MissionVmHost, load_mission, run_scripts, spawn_mission};
use a3_sqf::{Host, Namespace, Value, Vm};
use a3_world::{ClientId, TypeBank, World};

/// "Bootcamp" (Operation Flashpoint-style campaign), mission 02 — a small combined-arms mission
/// with vehicles, waypoints and triggers.
const FOLDER: &str = r"a3\missions_f_bootcamp\campaign\missions\boot_m02.altis";

#[test]
fn loads_a_shipped_campaign_mission() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root)).expect("the game loads");

    let mut loader = Vm::new(VfsHost::for_game(&game));
    let mission = load_mission(&game.vfs, FOLDER, &mut loader).expect("the mission loads");

    // Structure: the file parsed, the folder named the terrain, and the brief survived.
    assert_eq!(mission.version, 12);
    assert_eq!(mission.terrain.as_deref(), Some("altis"));
    assert!(!mission.addons.is_empty(), "addOns[]");
    assert!(!mission.groups.is_empty(), "class Groups");
    assert!(!mission.markers.is_empty(), "class Markers");
    assert!(mission.description.is_some(), "description.ext");
    let player = mission.player().expect("a player unit");
    assert!(
        mission.group_of(player.id).is_some(),
        "the player is in a group"
    );

    let units = mission.units().count();
    let waypoints: usize = mission.groups.iter().map(|g| g.waypoints.len()).sum();
    let scripts = mission.units().filter(|u| u.init.is_some()).count();
    eprintln!(
        "{}: {} groups, {} units ({} with init), {} waypoints, {} markers, {} triggers, \
         briefing name {:?}",
        mission.folder,
        mission.groups.len(),
        units,
        scripts,
        waypoints,
        mission.markers.len(),
        mission.triggers.len(),
        mission.intel.briefing_name,
    );
    assert!(units > 20, "a mission worth running: {units} units");
    assert!(waypoints > 0, "the mission has waypoints");

    // Spawning: real config types into a World. Some classes may be DLC the install lacks, so
    // only the bulk has to spawn.
    let mut world = World::new(ClientId::SERVER);
    let mut types = TypeBank::new(game.config.clone());
    let spawned = spawn_mission(&mut world, &mut types, &mission);
    eprintln!(
        "spawned {} of {} units, {} groups, player {:?}, {} unspawned",
        spawned.units.len(),
        units,
        spawned.groups.len(),
        spawned.player,
        spawned.unspawned.len(),
    );
    for unit in &spawned.unspawned {
        eprintln!(
            "  unspawned id {} ({}): {}",
            unit.id, unit.class, unit.reason
        );
    }
    assert!(spawned.player.is_some(), "the player unit spawned");
    assert!(
        spawned.units.len() * 2 > units,
        "most units spawned: {} of {units}",
        spawned.units.len()
    );

    // What the mission's scripts need from the VM: compile every `init` field and `init.sqf` and
    // let the registry say which commands have no implementation.
    let mut vm = MissionVmHost::new(world, types, VfsHost::for_game(&game)).vm();
    let mut probe = 0;
    let mut compile_failures = Vec::new();
    for unit in mission.units() {
        let Some(init) = &unit.init else {
            continue;
        };
        match vm.compile_file("mission.sqm", init) {
            Ok(code) => {
                vm.set_global(&format!("__init_{probe}"), Value::Code(code));
                probe += 1;
            }
            Err(e) => compile_failures.push(format!("unit {}: {}", unit.id, e.message)),
        }
    }
    let init_path = format!("{}\\init.sqf", mission.folder);
    if vm.host.file_exists(&init_path) {
        let text = vm
            .host
            .preprocess_file(&init_path, true)
            .expect("init.sqf preprocesses");
        match vm.compile_file(&init_path, &text) {
            Ok(code) => vm.set_global("__init_sqf", Value::Code(code)),
            Err(e) => compile_failures.push(format!("init.sqf: {}", e.message)),
        }
    }
    let missing = unimplemented_usage(&vm, Namespace::Mission);
    eprintln!("{} commands missing from the VM:", missing.len());
    for (name, count) in missing.iter().take(40) {
        eprintln!("  {name} x{count}");
    }
    assert!(
        compile_failures.is_empty(),
        "the mission's scripts compile: {compile_failures:?}"
    );
    assert!(
        !missing.is_empty(),
        "the VM cannot run a whole campaign mission yet — if this ever passes, pick another"
    );

    // And the whole pipeline once, over the real config and files.
    let report = run_scripts(&mut vm, &mission, &spawned);
    eprintln!(
        "run: {} scripts ({} ok), {} logs, {} frames, {:?}, {} errors",
        report.scripts.len(),
        report.scripts.iter().filter(|s| s.ok).count(),
        report.log_lines,
        report.frames,
        report.elapsed,
        report.errors.len(),
    );
    for script in report.scripts.iter().filter(|s| !s.ok).take(6) {
        eprintln!(
            "  failed: {}: {}",
            script.name,
            script.error.as_deref().unwrap_or("")
        );
    }
    assert!(
        report.scripts.iter().any(|s| s.name == "init.sqf"),
        "{:?}",
        report.scripts
    );
    assert!(report.frames <= MAX_INIT_FRAMES);
}
