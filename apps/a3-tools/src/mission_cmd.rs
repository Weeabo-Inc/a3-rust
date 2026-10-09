//! `a3-tools mission ...`: load a mission out of a game install and print what it places, and
//! with `--run` what happens when it is spawned and its scripts are run.

use std::path::PathBuf;

use clap::Args;

use a3_gamedata::{GameData, LoadOptions, VfsHost};
use a3_mission::{Mission, MissionVmHost, load_mission, run_scripts, spawn_mission};
use a3_sqf::Vm;
use a3_world::{ClientId, TypeBank, World};

#[derive(Args)]
pub struct MissionArgs {
    /// Game install folder.
    #[arg(long, env = "A3_ROOT")]
    game_dir: PathBuf,
    /// Mod folder to load after the game, in order; repeatable.
    #[arg(long = "mod", value_name = "DIR")]
    mods: Vec<PathBuf>,
    /// Also load every optional DLC and `@mod` folder found in the game folder.
    #[arg(long)]
    all_mods: bool,
    /// The mission folder inside the game, e.g.
    /// `a3\missions_f_bootcamp\campaign\missions\boot_m02.altis`; several run one after
    /// another over one loaded game, with a summary of their script errors.
    #[arg(required = true)]
    folders: Vec<String>,
    /// Spawn the mission into a World and run its scripts (unit `init` fields, then `init.sqf`),
    /// printing the report.
    #[arg(long)]
    run: bool,
    /// With `--run`: how many unimplemented commands to list.
    #[arg(long, default_value_t = 20)]
    missing: usize,
}

pub fn run(args: MissionArgs) -> anyhow::Result<()> {
    let data = load(&args)?;
    if let [folder] = args.folders.as_slice() {
        let mut vm = Vm::new(VfsHost::for_game(&data));
        let mission = load_mission(&data.vfs, folder, &mut vm)?;
        print_mission(&mission);
        if args.run {
            run_it(&data, &mission, args.missing);
        }
        return Ok(());
    }
    // Several missions: script errors by first line, over all of them.
    let mut errors: std::collections::BTreeMap<String, usize> = Default::default();
    let mut ran = 0;
    for folder in &args.folders {
        let mut vm = Vm::new(VfsHost::for_game(&data));
        let mission = match load_mission(&data.vfs, folder, &mut vm) {
            Ok(mission) => mission,
            Err(e) => {
                println!("{folder}: not loaded: {e}");
                continue;
            }
        };
        print_mission(&mission);
        if args.run {
            ran += 1;
            for error in run_it(&data, &mission, args.missing) {
                *errors.entry(error).or_default() += 1;
            }
        }
    }
    if args.run {
        let total: usize = errors.values().sum();
        let unimplemented: usize = errors
            .iter()
            .filter(|(e, _)| e.contains("Unimplemented command"))
            .map(|(_, n)| n)
            .sum();
        println!(
            "== {ran} missions run: {total} script error(s), {unimplemented} of them              \"Unimplemented command\""
        );
        let mut sorted: Vec<_> = errors.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (error, count) in sorted {
            println!("{count:5}  {error}");
        }
    }
    Ok(())
}

fn load(args: &MissionArgs) -> anyhow::Result<GameData> {
    let options = LoadOptions::new(&args.game_dir)
        .with_mods(&args.mods)
        .with_optional_mods(args.all_mods);
    Ok(GameData::load(&options)?)
}

/// Prints the mission's brief, then its groups (with units and waypoints), markers and triggers.
fn print_mission(mission: &Mission) {
    let (groups, units, waypoints) = (
        mission.groups.len(),
        mission.units().count(),
        mission
            .groups
            .iter()
            .map(|g| g.waypoints.len())
            .sum::<usize>(),
    );
    println!(
        "{} (terrain {}, version {}, seed {})",
        mission.folder,
        mission.terrain.as_deref().unwrap_or("-"),
        mission.version,
        mission.random_seed,
    );
    let intel = &mission.intel;
    println!(
        "brief {:?}  date {}  time {}:{}  weather {:.2}  addons {}",
        intel.briefing_name.as_deref().unwrap_or("-"),
        [intel.year, intel.month, intel.day]
            .into_iter()
            .flatten()
            .map(|part| part.to_string())
            .collect::<Vec<String>>()
            .join("-"),
        intel.hour.unwrap_or(0),
        intel.minute.unwrap_or(0),
        intel.start_weather.unwrap_or(0.0),
        mission.addons.len(),
    );
    println!(
        "{groups} groups, {units} units, {waypoints} waypoints, {} ungrouped objects, \
         {} markers, {} triggers",
        mission.objects.len(),
        mission.markers.len(),
        mission.triggers.len(),
    );

    for (index, group) in mission.groups.iter().enumerate() {
        println!(
            "group {index}: {:?} ({} units, {} waypoints)",
            group.side,
            group.units.len(),
            group.waypoints.len()
        );
        for unit in &group.units {
            print_unit(unit, "  ");
        }
        for (index, waypoint) in group.waypoints.iter().enumerate() {
            println!(
                "  waypoint {index}: {} at ({:.1}, {:.1}, {:.1}){}",
                waypoint.kind.as_deref().unwrap_or("MOVE"),
                waypoint.position.x,
                waypoint.position.y,
                waypoint.position.z,
                waypoint
                    .description
                    .as_deref()
                    .map(|d| format!(" {:?}", d))
                    .unwrap_or_default(),
            );
        }
    }
    for unit in &mission.objects {
        print_unit(unit, "ungrouped ");
    }
    for marker in &mission.markers {
        println!(
            "marker {:?}: {} at ({:.1}, {:.1}, {:.1}){}",
            marker.name,
            marker.kind.as_deref().unwrap_or("-"),
            marker.position.x,
            marker.position.y,
            marker.position.z,
            marker
                .text
                .as_deref()
                .map(|t| format!(" {:?}", t))
                .unwrap_or_default(),
        );
    }
    for trigger in &mission.triggers {
        println!(
            "trigger {:?}: {} {}x{} at ({:.1}, {:.1}, {:.1}) activ {:?} {:?}{}",
            trigger.name.as_deref().unwrap_or("-"),
            trigger.kind.as_deref().unwrap_or("trigger"),
            trigger.a.unwrap_or(0.0),
            trigger.b.unwrap_or(0.0),
            trigger.position.x,
            trigger.position.y,
            trigger.position.z,
            trigger.activation_by.as_deref().unwrap_or("ANY"),
            trigger.activation_type.as_deref().unwrap_or("PRESENT"),
            trigger
                .exp_activ
                .as_deref()
                .map(|s| format!(" on activation {:?}", s))
                .unwrap_or_default(),
        );
    }
}

fn print_unit(unit: &a3_mission::Unit, indent: &str) {
    let mut marks = Vec::new();
    if unit.leader {
        marks.push("leader");
    }
    if unit.player.is_some() {
        marks.push("player");
    }
    if unit.is_absent() {
        marks.push("absent");
    }
    println!(
        "{indent}unit {}: {} at ({:.1}, {:.1}, {:.1}){} {}",
        unit.id,
        unit.class,
        unit.position.x,
        unit.position.y,
        unit.position.z,
        unit.azimut
            .map(|a| format!(" azimut {:.0}", a))
            .unwrap_or_default(),
        marks.join(" "),
    );
    if let Some(text) = &unit.text {
        println!("{indent}  name {text:?}");
    }
    if let Some(init) = &unit.init {
        println!("{indent}  init {init:?}");
    }
}

/// Spawns the mission over the real config and runs its scripts, printing what happened.
/// Returns the first line of each script error.
fn run_it(data: &GameData, mission: &Mission, missing: usize) -> Vec<String> {
    let mut world = World::new(ClientId::SERVER);
    let mut types = TypeBank::new(data.config.clone());
    let spawned = spawn_mission(&mut world, &mut types, mission);
    println!(
        "spawned {} units in {} groups, player {:?}, {} not spawned",
        spawned.units.len(),
        spawned.groups.len(),
        spawned
            .player
            .and_then(|id| world.entity(id).map(|e| e.entity_type().name().to_owned())),
        spawned.unspawned.len(),
    );
    for unit in &spawned.unspawned {
        println!(
            "  not spawned: id {} ({}): {}",
            unit.id, unit.class, unit.reason
        );
    }

    let mut vm = MissionVmHost::for_game(world, types, data).vm();
    let report = run_scripts(&mut vm, mission, &spawned);
    println!(
        "ran {} scripts ({:.2?}), {} ok, {} log line(s), {} frame(s), {} error(s)",
        report.scripts.len(),
        report.elapsed,
        report.scripts.iter().filter(|s| s.ok).count(),
        report.log_lines,
        report.frames,
        report.errors.len(),
    );
    for script in &report.scripts {
        match (&script.ok, &script.error) {
            (true, _) => println!("  ok      {}", script.name),
            // The first line of the report; the rest repeats the source and position.
            (false, Some(error)) => {
                let first = error.lines().next().unwrap_or_default();
                println!("  failed  {}: {first}", script.name);
            }
            (false, None) => println!("  failed  {}", script.name),
        }
    }
    println!(
        "{} commands the VM does not implement yet:",
        report.missing_commands.len()
    );
    for (name, count) in report.missing_commands.iter().take(missing) {
        println!("  {name} x{count}");
    }
    let errors: Vec<String> = report
        .errors
        .iter()
        .map(|e| error_message(e).to_owned())
        .collect();
    for error in &errors {
        println!("  error: {error}");
    }
    errors
}

/// The message of an engine-style error report (the `Error <message>` line after the expression
/// and position lines), or its first line.
fn error_message(report: &str) -> &str {
    report
        .lines()
        .map(str::trim)
        .find(|l| {
            l.starts_with("Error ")
                && !l.starts_with("Error in expression <")
                && !l.starts_with("Error position: <")
        })
        .or_else(|| report.lines().next())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn error_message_is_the_error_line() {
        let report = "Error in expression <a = b c>\n  Error position: <c>\n  Error Missing ;\nfile.sqf, line 1";
        assert_eq!(super::error_message(report), "Error Missing ;");
        assert_eq!(
            super::error_message("Script x not found"),
            "Script x not found"
        );
    }
}
