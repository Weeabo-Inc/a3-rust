//! `a3-sweep`: the scenario sweep. Loads every shipped scenario headlessly with the engine, runs
//! its start-up and a fixed-dt simulation, and reports what breaks, ranked by how many scenarios
//! it affects. See `docs/fidelity/README.md`.
//!
//! ```text
//! cargo run --release -p a3-sweep -- --game-dir <install> [--filter altis] [--seconds 60]
//!     [--doc docs/fidelity/scenario-sweep.md]
//! ```

mod inventory;
mod report;
mod runner;
mod summary;
mod supervisor;
mod worker;

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use a3_config::ConfigTree;
use a3_gamedata::{GameData, LoadOptions, VfsHost};
use anyhow::Context as _;
use clap::Parser;

use crate::inventory::{Kind, Listing, Scenario, inventory, mount_loose_missions};
use crate::report::{ScenarioResult, Status, Sweep, SweepOptions};
use crate::runner::{Engine, RunOptions};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Run every shipped scenario headlessly and report what breaks"
)]
struct Args {
    /// Game install folder. Not needed with `--summarize`, which only reads a recorded sweep.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<PathBuf>,
    /// Only scenarios whose folder contains this text (case-insensitive); repeatable, any
    /// matches. The folder ends with the world, so `--filter altis` selects a world.
    #[arg(long)]
    filter: Vec<String>,
    /// Only scenarios of this kind: campaign, scenario, showcase, challenge, tutorial,
    /// multiplayer (mp), cutscene, unlisted, fragment; repeatable.
    #[arg(long = "kind")]
    kinds: Vec<String>,
    /// Include fragments (Contact sites, Old Man layers), which are skipped by default.
    #[arg(long)]
    all: bool,
    /// Run at most this many scenarios (after filtering).
    #[arg(long)]
    limit: Option<usize>,
    /// Simulated seconds per scenario after start-up.
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,
    /// Simulation frames per simulated second.
    #[arg(long, default_value_t = 20.0)]
    fps: f64,
    /// Wall-clock seconds the simulation of one scenario may take before it stops early.
    #[arg(long, default_value_t = 90.0)]
    budget: f64,
    /// Wall-clock milliseconds the scheduled scripts get per frame (the engine gives 3).
    #[arg(long, default_value_t = 10.0)]
    frame_budget_ms: f64,
    /// Wall-clock seconds after which a worker still on one scenario is killed.
    #[arg(long, default_value_t = 300.0)]
    hard_timeout: f64,
    /// Worker processes (each loads the game: about 2 GB of memory each).
    #[arg(long, default_value_t = 2)]
    jobs: usize,
    /// Do not load terrains (the World is flat at 0 m); faster.
    #[arg(long)]
    no_terrain: bool,
    /// Run the scenarios in this process instead of worker processes: no isolation from
    /// crashes and hangs, for debugging one scenario.
    #[arg(long)]
    in_process: bool,
    /// Print the inventory and the selection, then stop.
    #[arg(long)]
    list: bool,
    /// Folder for the sweep's JSON.
    #[arg(long, default_value = ".work/sweep")]
    out: PathBuf,
    /// Write the Markdown summary here (e.g. docs/fidelity/scenario-sweep.md).
    #[arg(long)]
    doc: Option<PathBuf>,
    /// Do not run: read this sweep JSON and write the summary (`--doc`) from it.
    #[arg(long, value_name = "JSON")]
    summarize: Option<PathBuf>,
    /// Internal: run as a worker process.
    #[arg(long, hide = true)]
    worker: bool,
}

impl Args {
    fn run_options(&self) -> RunOptions {
        RunOptions {
            seconds: self.seconds.max(0.0),
            fps: self.fps.max(1.0),
            budget: Duration::from_secs_f64(self.budget.max(0.0)),
            frame_budget: Duration::from_secs_f64(self.frame_budget_ms.max(0.1) / 1000.0),
            terrain: !self.no_terrain,
        }
    }

    /// The arguments that start a worker with the same run options, on the game install `run`
    /// validated.
    fn worker_args(&self, game_dir: &Path) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "--worker".into(),
            "--game-dir".into(),
            game_dir.as_os_str().to_owned(),
            "--seconds".into(),
            self.seconds.to_string().into(),
            "--fps".into(),
            self.fps.to_string().into(),
            "--budget".into(),
            self.budget.to_string().into(),
            "--frame-budget-ms".into(),
            self.frame_budget_ms.to_string().into(),
        ];
        if self.no_terrain {
            args.push("--no-terrain".into());
        }
        args
    }

    /// The game install, from `--game-dir` or `A3_ROOT`; `--summarize` does not need one.
    fn game_dir(&self) -> anyhow::Result<&Path> {
        self.game_dir.as_deref().with_context(|| {
            "no game install: pass --game-dir <DIR> or set A3_ROOT (only --summarize works \
             without one)"
        })
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.worker {
        let game_dir = args.game_dir()?.to_owned();
        let options = args.run_options();
        return on_big_stack(move || worker::serve(&game_dir, options));
    }
    if let Some(json) = &args.summarize {
        let sweep: Sweep = serde_json::from_slice(
            &std::fs::read(json).with_context(|| format!("cannot read {}", json.display()))?,
        )
        .with_context(|| format!("{} is not a sweep", json.display()))?;
        print_summary(&sweep);
        if let Some(doc) = &args.doc {
            write_doc(doc, &sweep)?;
        }
        return Ok(());
    }

    let game_dir = args.game_dir()?.to_owned();
    let started = chrono::Local::now();
    let start = Instant::now();
    let all = load_inventory(&game_dir)?;
    let selected = select(&all, &args);
    if args.list {
        println!("{}", summary::inventory_table(&all));
        println!("{} of {} scenarios selected:", selected.len(), all.len());
        for s in &selected {
            println!(
                "  {:<12} {:<10} {}{}{}",
                s.kind.as_str(),
                s.world,
                s.folder,
                if s.optional_mods {
                    " [optional DLC]"
                } else {
                    ""
                },
                s.listed_as
                    .as_deref()
                    .map(|c| format!("  ({c})"))
                    .unwrap_or_default()
            );
        }
        return Ok(());
    }
    anyhow::ensure!(!selected.is_empty(), "no scenario matches the selection");
    eprintln!(
        "sweeping {} of {} scenarios ({} s each at {} fps)",
        selected.len(),
        all.len(),
        args.seconds,
        args.fps
    );

    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("cannot create {}", args.out.display()))?;
    let stamp = started.format("%Y%m%d-%H%M%S").to_string();
    let partial_path = args.out.join(format!("{stamp}.partial.jsonl"));
    let mut partial = std::fs::File::create(&partial_path)?;
    let total = selected.len();
    let mut done = 0;
    let mut on_result = |result: &ScenarioResult| {
        done += 1;
        progress(done, total, result);
        if let Ok(line) = serde_json::to_string(result) {
            let _ = writeln!(partial, "{line}");
        }
    };

    let results = if args.in_process {
        let options = args.run_options();
        let scenarios = selected.clone();
        let results = on_big_stack(move || {
            worker::install_panic_hook();
            let mut engine = Engine::new(&game_dir, options);
            Ok(scenarios
                .iter()
                .map(|s| worker::run_caught(&mut engine, s, |_| {}))
                .collect::<Vec<_>>())
        })?;
        results.iter().for_each(&mut on_result);
        results
    } else {
        let options = supervisor::SupervisorOptions {
            jobs: args.jobs,
            hard_timeout: Duration::from_secs_f64(args.hard_timeout.max(1.0)),
            worker_args: args.worker_args(&game_dir),
        };
        supervisor::run(selected, &options, &mut on_result)?
    };

    let mut results = results;
    results.sort_by(|a, b| a.scenario.folder.cmp(&b.scenario.folder));
    let sweep = Sweep {
        started: started.format("%Y-%m-%d %H:%M:%S").to_string(),
        engine: engine_version(),
        options: SweepOptions {
            seconds: args.seconds,
            fps: args.fps,
            budget_s: args.budget,
            jobs: if args.in_process { 1 } else { args.jobs },
            terrain: !args.no_terrain,
            filters: args.filter.clone(),
            profile: build_profile().to_owned(),
        },
        elapsed_s: start.elapsed().as_secs_f64(),
        scenarios: results,
    };
    let json_path = args.out.join(format!("{stamp}.json"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(&sweep)?)
        .with_context(|| format!("cannot write {}", json_path.display()))?;
    let _ = std::fs::remove_file(&partial_path);
    print_summary(&sweep);
    eprintln!("wrote {}", json_path.display());
    if let Some(doc) = &args.doc {
        write_doc(doc, &sweep)?;
    }
    Ok(())
}

/// Runs `f` on a thread with [`worker::STACK_SIZE`] of stack and returns its result.
fn on_big_stack<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    std::thread::Builder::new()
        .name("scenarios".into())
        .stack_size(worker::STACK_SIZE)
        .spawn(f)?
        .join()
        .map_err(|_| anyhow::anyhow!("the scenario thread panicked"))?
}

/// Mounts the game and lists every mission folder, classified. The base game (with its default
/// DLC) and the game with the optional DLC are both loaded: the optional DLC add mission
/// folders and also delete listings (Contact deletes every other campaign).
fn load_inventory(game_dir: &Path) -> anyhow::Result<Vec<Scenario>> {
    let (base, base_listing) = load_listing(game_dir, false)?;
    let (full, full_listing) = load_listing(game_dir, true)?;
    let base_folders: std::collections::BTreeSet<String> =
        base.into_iter().map(|s| s.folder).collect();
    let mut listing = base_listing;
    listing.merge(full_listing);
    let mut all: Vec<Scenario> = full
        .into_iter()
        .map(|s| {
            let mut classified = listing.classify(&s.folder);
            // The loose PBOs' kind and package come from their folder, not a listing.
            if s.package == "Missions" || s.package == "MPMissions" {
                classified.kind = s.kind;
                classified.package = s.package;
            }
            classified.optional_mods = !base_folders.contains(&classified.folder);
            classified
        })
        .collect();
    all.sort_by(|a, b| a.folder.cmp(&b.folder));
    inventory::mark_nested_fragments(&mut all);
    Ok(all)
}

/// The mission folders and the `CfgMissions` listing of the game, with or without the
/// optional DLC.
fn load_listing(game_dir: &Path, optional_mods: bool) -> anyhow::Result<(Vec<Scenario>, Listing)> {
    let data = GameData::load(&LoadOptions::new(game_dir).with_optional_mods(optional_mods))
        .with_context(|| format!("cannot load the game in {}", game_dir.display()))?;
    let loose = mount_loose_missions(&data.vfs, game_dir);
    let mut vm = a3_sqf::Vm::new(VfsHost::new(data.vfs.clone()));
    let listing = Listing::from_config(&data.config, |dir| {
        let path = format!("{dir}\\description.ext");
        if !data.vfs.exists(&path) {
            return None;
        }
        match a3_gamedata::load_text_config(&data.vfs, &mut vm, &path) {
            Ok(config) => Some(ConfigTree::from_config(&config)),
            Err(e) => {
                eprintln!("warning: campaign {path} does not load: {e}");
                None
            }
        }
    });
    let scenarios = inventory(&data.vfs, &listing, &loose);
    Ok((scenarios, listing))
}

/// The scenarios the arguments select, in folder order.
fn select(all: &[Scenario], args: &Args) -> Vec<Scenario> {
    let kinds: Vec<Kind> = args.kinds.iter().filter_map(|k| Kind::parse(k)).collect();
    let filters: Vec<String> = args.filter.iter().map(|f| f.to_ascii_lowercase()).collect();
    let mut out: Vec<Scenario> = all
        .iter()
        .filter(|s| {
            if kinds.is_empty() {
                args.all || s.kind != Kind::Fragment
            } else {
                kinds.contains(&s.kind)
            }
        })
        .filter(|s| filters.is_empty() || filters.iter().any(|f| s.folder.contains(f.as_str())))
        .cloned()
        .collect();
    if let Some(limit) = args.limit {
        out.truncate(limit);
    }
    out
}

fn progress(done: usize, total: usize, r: &ScenarioResult) {
    let detail = match r.status {
        Status::Pass | Status::Errors => format!(
            "{} errors, {} blocked, {:.0} s sim, {:.2} ms/frame",
            r.error_count,
            r.unimplemented_runtime.len(),
            r.sim_seconds,
            r.timings.frame_ms_mean
        ),
        _ => r
            .failure
            .as_deref()
            .and_then(|f| f.lines().next())
            .unwrap_or("")
            .chars()
            .take(120)
            .collect(),
    };
    eprintln!(
        "[{done:>3}/{total}] {:<11} {} ({:.1} s) {detail}",
        r.status.as_str(),
        r.scenario.folder,
        r.timings.total_ms / 1000.0
    );
}

fn print_summary(sweep: &Sweep) {
    let total = sweep.scenarios.len();
    let pass = sweep
        .scenarios
        .iter()
        .filter(|r| r.status == Status::Pass)
        .count();
    println!(
        "{pass} of {total} scenarios pass ({:.1}%)",
        if total == 0 {
            0.0
        } else {
            pass as f64 * 100.0 / total as f64
        }
    );
    for status in Status::ALL {
        let n = sweep
            .scenarios
            .iter()
            .filter(|r| r.status == status)
            .count();
        println!("  {:<11} {n}", status.as_str());
    }
    println!("top unimplemented commands (scenarios blocked / using):");
    for c in summary::command_ranking(&sweep.scenarios).iter().take(15) {
        println!("  {:<32} {:>4} {:>4}", c.name, c.blocked, c.used_by);
    }
    println!("top error signatures (scenarios / reports):");
    for s in summary::signature_ranking(&sweep.scenarios).iter().take(15) {
        println!("  {:>4} {:>6}  {}", s.scenarios, s.count, s.signature);
    }
}

fn write_doc(path: &Path, sweep: &Sweep) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, summary::markdown(sweep))
        .with_context(|| format!("cannot write {}", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// The build profile of this binary: `ms/frame` is only comparable within one profile.
fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// `git describe` of the working tree, or empty.
fn engine_version() -> String {
    std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `--game-dir` used to be required, so the summary could not be regenerated
    /// from a recorded sweep on a machine without the game install.
    #[test]
    fn summarize_needs_no_game_install() {
        let args = Args::try_parse_from(["a3-sweep", "--summarize", "sweep.json"])
            .expect("`--summarize` parses without `--game-dir`");
        assert_eq!(args.summarize.as_deref(), Some(Path::new("sweep.json")));
    }

    #[test]
    fn a_sweep_without_an_install_says_so() {
        // `A3_ROOT` is unset in CI; when it is set locally this asserts the opposite branch.
        let args = Args::try_parse_from(["a3-sweep"]).expect("parses");
        let expected = std::env::var_os("A3_ROOT").is_none();
        assert_eq!(
            args.game_dir().is_err(),
            expected,
            "a run needs --game-dir or A3_ROOT: {:?}",
            args.game_dir
        );
        if expected {
            assert!(
                args.game_dir()
                    .unwrap_err()
                    .to_string()
                    .contains("--game-dir"),
                "the error names the flag"
            );
        }
    }
}
