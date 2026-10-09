//! Running a mission's scripts: the units' `init` fields, then `init.sqf`, in the mission
//! namespace.
//!
//! The engine's order (community wiki, "Initialisation Order", Arma 3 table, from first to last)
//! is: object init event handlers, object initialisation fields (the SQM `init` strings, run
//! unscheduled), then `init.sqs`, then `init.sqf` — which runs **scheduled**. This module follows
//! that: every created unit's `init` runs unscheduled with `this` set to the unit, then `init.sqf`
//! is spawned as a scheduled script and the scheduler is stepped (with the World stepped between
//! frames) until it finishes or the frame cap is reached.
//!
//! Named units (`text=` in the SQM) become `missionNamespace` variables, as do the markers'
//! names _(`marker` values need a marker system the World does not have yet, so markers are
//! reported, not created)_; the player unit becomes `player`.
//!
//! [`run_scripts`] runs start-up to the end of `init.sqf`. A game loop uses [`start_mission`]
//! instead, which can also run the function library's mission start
//! ([`StartOptions::functions`]) and returns once `init.sqf` is spawned; the loop then calls
//! [`step`] once per frame.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3_gamedata::{ConfigHost, ConfigRoot, VfsHost, script_registry};
use a3_sqf::{Code, FrameReport, Host, Namespace, Registry, ScriptHandle, Value, Vm};
use a3_vfs::Vfs;
use a3_world::ObjectRef;
use a3_world::script::{
    WorldHost, format_handle, is_null_handle, object_value, register_world_commands,
};
use a3_world::{TypeBank, World};

use crate::load::install_mission_config;
use crate::mission::{Mission, MissionVariable};
use crate::spawn::Spawned;

/// Frames the `init.sqf` scheduler is stepped for before it is abandoned (at 15 Hz, 20 seconds).
pub const MAX_INIT_FRAMES: usize = 300;

/// The services mission start-up needs from the engine embedding the VM.
pub trait MissionHost: WorldHost + ConfigHost {
    /// The files `init.sqf` and its includes are read from.
    fn vfs(&self) -> &Vfs;
    /// Script errors, in order.
    fn errors(&self) -> &[String];
    /// `diag_log` lines, in order.
    fn log(&self) -> &[String];
}

/// A self-contained mission host: a [`World`], its [`TypeBank`] and [`VfsHost`] file services.
/// For tools, tests and headless runs.
pub struct MissionVmHost {
    pub world: World,
    pub types: TypeBank,
    pub files: VfsHost,
}

impl MissionVmHost {
    pub fn new(world: World, types: TypeBank, files: VfsHost) -> Self {
        Self {
            world,
            types,
            files,
        }
    }

    /// A host over `world` and `types` whose files and `configFile` come from a loaded game.
    pub fn for_game(world: World, types: TypeBank, game: &a3_gamedata::GameData) -> Self {
        Self::new(world, types, VfsHost::for_game(game))
    }

    /// The VM a mission's scripts run on: this host with the mission command registry.
    pub fn vm(self) -> Vm<MissionVmHost> {
        Vm::with_registry(self, Rc::new(mission_registry()))
    }
}

impl std::fmt::Debug for MissionVmHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionVmHost")
            .field("world", &self.world)
            .field("files", &self.files)
            .finish()
    }
}

impl Host for MissionVmHost {
    fn time(&self) -> f32 {
        self.world.time() as f32
    }

    fn is_null(&self, handle: a3_sqf::Handle) -> bool {
        is_null_handle(&self.world, handle)
    }

    fn format_handle(&self, handle: a3_sqf::Handle) -> String {
        // Config handles print their path (`bin\config.bin/CfgVehicles/Car`, or the engine's
        // `<NULL-config>`), which only the configs know; everything else is an Object or Group.
        if handle.kind == a3_sqf::HandleKind::Config {
            return self.files.configs.format(handle);
        }
        format_handle(&self.world, handle)
    }

    fn localize(&self, key: &str) -> Option<String> {
        self.files.localize(key)
    }

    fn diag_log(&mut self, text: &str) {
        self.files.diag_log(text);
    }

    fn report_error(&mut self, error: &a3_sqf::ScriptError) {
        self.files.report_error(error);
    }

    fn load_file(&mut self, path: &str) -> Result<String, String> {
        self.files.load_file(path)
    }
}

impl WorldHost for MissionVmHost {
    fn world(&self) -> &World {
        &self.world
    }

    fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn types(&mut self) -> &mut TypeBank {
        &mut self.types
    }

    fn mission_configs(&self) -> Vec<std::sync::Arc<a3_config::ConfigTree>> {
        let configs = self.files.configs();
        vec![
            std::sync::Arc::clone(configs.tree(a3_gamedata::ConfigRoot::Mission)),
            std::sync::Arc::clone(configs.tree(a3_gamedata::ConfigRoot::Campaign)),
        ]
    }
}

impl ConfigHost for MissionVmHost {
    fn configs(&self) -> &a3_gamedata::SqfConfigs {
        self.files.configs()
    }

    fn configs_mut(&mut self) -> &mut a3_gamedata::SqfConfigs {
        self.files.configs_mut()
    }
}

impl a3_gamedata::ErrorLog for MissionVmHost {
    fn error_log(&self) -> &[String] {
        &self.files.errors
    }
}

impl MissionHost for MissionVmHost {
    fn vfs(&self) -> &Vfs {
        &self.files.vfs
    }

    fn errors(&self) -> &[String] {
        &self.files.errors
    }

    fn log(&self) -> &[String] {
        &self.files.log
    }
}

/// The command registry a mission runs with: the game's commands (core, config, headless game
/// state) plus the World's.
pub fn mission_registry<H: WorldHost + ConfigHost>() -> Registry<H> {
    let mut registry = script_registry::<H>();
    register_world_commands(&mut registry);
    registry
}

/// One script the mission ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRun {
    /// What ran it: `"init.sqf"` or `"init of <unit text or id>"`.
    pub name: String,
    /// Whether it finished without a script error.
    pub ok: bool,
    /// The error's report, when it failed.
    pub error: Option<String>,
}

/// What [`run_scripts`] did.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    /// Named units installed as `missionNamespace` variables.
    pub variables: usize,
    /// Markers whose name could not be installed (no marker system yet).
    pub markers: usize,
    /// The scripts that ran, in order.
    pub scripts: Vec<ScriptRun>,
    /// Script errors, in order (the same the host collected).
    pub errors: Vec<String>,
    /// Commands the mission's scripts ([`ScriptRun`]s and the code values they left in the
    /// mission namespace) used that the VM has no implementation for, most used first.
    pub missing_commands: Vec<(String, usize)>,
    /// `diag_log` lines the scripts produced.
    pub log_lines: usize,
    /// Scheduler frames `init.sqf` ran for (0 when the mission has none, and after
    /// [`start_mission`], which does not wait).
    pub frames: usize,
    /// The scheduled `init.sqf`, when [`start_mission`] spawned one.
    pub init_sqf: Option<ScriptHandle>,
    /// Wall-clock time of the run.
    pub elapsed: Duration,
}

/// Installs the mission's variables and runs its scripts: every spawned unit's `init` first
/// (unscheduled, `this` = the unit), then `init.sqf` as a scheduled script, stepping the World
/// until `init.sqf` finishes or [`MAX_INIT_FRAMES`] frames have run.
pub fn run_scripts<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    spawned: &Spawned,
) -> RunReport {
    let start = Instant::now();
    let errors_before = vm.host.errors().len();
    let log_before = vm.host.log().len();
    let mut report = RunReport::default();
    let mut codes: Vec<Code> = Vec::new();
    install_mission_config(vm, mission);
    install_variables(vm, mission, spawned, &mut report);
    run_unit_inits(vm, mission, spawned, &mut report, &mut codes);
    if let Some(handle) = spawn_init_sqf(vm, mission, &mut report, &mut codes) {
        report.frames = vm.run_until_idle(MAX_INIT_FRAMES, |host| {
            host.world_mut().simulate(1.0 / 15.0);
        });
        let done = vm.script_done(handle);
        report.scripts.push(ScriptRun {
            name: "init.sqf".to_owned(),
            ok: done,
            error: (!done).then(|| format!("still running after {MAX_INIT_FRAMES} frames")),
        });
    }
    // What the mission's own scripts used (the codes just compiled), plus whatever the scripts
    // put into the mission namespace themselves.
    let missing = a3_gamedata::unimplemented_usage_in(vm, &codes)
        .into_iter()
        .chain(a3_gamedata::unimplemented_usage(vm, Namespace::Mission));
    report.missing_commands = merge_counts(missing);
    finish(vm, &mut report, start, errors_before, log_before);
    report
}

/// How [`start_mission`] starts a mission.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartOptions {
    /// Run the function library's mission start first: the script `configFile >>
    /// "CfgFunctions" >> "init"` names (`initFunctions.sqf`), unscheduled, `_this` undefined, in
    /// missionNamespace (`docs/re/functions-init.md`). With the library already compiled at game
    /// start, it compiles the campaign's and mission's functions, runs the `preInit` functions
    /// and spawns the `postInit` sequence (`initServer.sqf`, `initPlayerLocal.sqf`, the
    /// `postInit` functions).
    pub functions: bool,
}

/// Starts a mission the way the engine does, without waiting for its scheduled scripts:
/// mission config and variables, then (with [`StartOptions::functions`]) the function library's
/// mission start, then every spawned unit's `init` (unscheduled, `this` = the unit), and last
/// `init.sqf` spawned as a scheduled script ([`RunReport::init_sqf`]). Run the mission on with
/// [`step`].
///
/// [`RunReport::missing_commands`] covers the code start-up compiled itself (the init fields and
/// `init.sqf`), not the mission namespace, which after the library's start holds every library
/// function.
pub fn start_mission<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    spawned: &Spawned,
    options: StartOptions,
) -> RunReport {
    let start = Instant::now();
    let errors_before = vm.host.errors().len();
    let log_before = vm.host.log().len();
    let mut report = RunReport::default();
    let mut codes: Vec<Code> = Vec::new();
    install_mission_config(vm, mission);
    install_variables(vm, mission, spawned, &mut report);
    if options.functions {
        run_functions_init(vm, &mut report);
    }
    run_unit_inits(vm, mission, spawned, &mut report, &mut codes);
    report.init_sqf = spawn_init_sqf(vm, mission, &mut report, &mut codes);
    report.missing_commands = merge_counts(a3_gamedata::unimplemented_usage_in(vm, &codes));
    finish(vm, &mut report, start, errors_before, log_before);
    report
}

/// One frame of a running mission: the World advances by `dt` seconds, then the scheduled
/// scripts run for at most `budget` of wall-clock time (the engine gives them about 3 ms,
/// [`a3_sqf::DEFAULT_FRAME_BUDGET`]).
pub fn step<H: MissionHost>(vm: &mut Vm<H>, dt: f64, budget: Duration) -> FrameReport {
    vm.host.world_mut().simulate(dt);
    vm.run_scheduled(budget)
}

/// Sums the counts of equal names; most used first, ties by name.
fn merge_counts(counts: impl IntoIterator<Item = (String, usize)>) -> Vec<(String, usize)> {
    let mut merged: std::collections::BTreeMap<String, usize> = Default::default();
    for (name, count) in counts {
        *merged.entry(name).or_default() += count;
    }
    let mut out: Vec<(String, usize)> = merged.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Fills in the report's errors, log line count and elapsed time.
fn finish<H: MissionHost>(
    vm: &Vm<H>,
    report: &mut RunReport,
    start: Instant,
    errors_before: usize,
    log_before: usize,
) {
    report.errors = vm.host.errors()[errors_before..].to_vec();
    report.log_lines = vm.host.log().len() - log_before;
    report.elapsed = start.elapsed();
}

/// Runs `configFile >> "CfgFunctions" >> "init"` unscheduled in missionNamespace with `_this`
/// undefined. Nothing runs when the config names no script.
fn run_functions_init<H: MissionHost>(vm: &mut Vm<H>, report: &mut RunReport) {
    let path = {
        let config = Arc::clone(vm.host.configs().tree(ConfigRoot::Game));
        let cfg = config.root() >> "CfgFunctions";
        (&cfg >> "init").text()
    };
    if path.is_empty() {
        return;
    }
    // Outside an init field `this` is undefined.
    vm.set_global("this", Value::Nil);
    let result = vm
        .host
        .preprocess_file(&path, true)
        .and_then(|text| vm.compile_file(&path, &text).map_err(|e| e.message))
        .and_then(|code| {
            vm.call_in(&code, None, Namespace::Mission)
                .map(|_| ())
                .map_err(|e| e.report)
        });
    report.scripts.push(ScriptRun {
        name: "functions init".to_owned(),
        ok: result.is_ok(),
        error: result.err(),
    });
}

fn install_variables<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    spawned: &Spawned,
    report: &mut RunReport,
) {
    for (name, variable) in mission.variables() {
        match variable {
            MissionVariable::Unit(id) => {
                let Some(&unit) = spawned.units.get(&id) else {
                    continue;
                };
                // The engine names the unit (`vehicleVarName`, which `str` prints) and sets
                // the variable.
                vm.host.world_mut().set_var_name(unit, &name);
                let value = object_value(vm.host.world(), ObjectRef::Entity(unit));
                vm.set_global(&name, value);
                report.variables += 1;
            }
            MissionVariable::Marker(_) => report.markers += 1,
        }
    }
    // `player` is a command reading the World's player.
    vm.host.world_mut().set_player(spawned.player);
}

fn run_unit_inits<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    spawned: &Spawned,
    report: &mut RunReport,
    codes: &mut Vec<Code>,
) {
    for unit in mission.units() {
        let Some(init) = &unit.init else {
            continue;
        };
        let Some(&entity) = spawned.units.get(&unit.id) else {
            continue;
        };
        let name = match &unit.text {
            Some(text) => format!("init of {text}"),
            None => format!("init of unit {}", unit.id),
        };
        let source_path = format!("{}\\mission.sqm", mission.folder);
        let code = match vm.compile_file(&source_path, init) {
            Ok(code) => code,
            Err(e) => {
                report.scripts.push(ScriptRun {
                    name,
                    ok: false,
                    error: Some(e.message.clone()),
                });
                continue;
            }
        };
        codes.push(code.clone());
        let this = object_value(vm.host.world(), ObjectRef::Entity(entity));
        // An init field's object is `this` in the engine — shipped fields are full of
        // `this allowDamage false;` — as well as `_this`, which is the VM's parameter variable.
        vm.set_global("this", this.clone());
        let result = vm.call_in(&code, Some(this), Namespace::Mission);
        report.scripts.push(ScriptRun {
            name,
            ok: result.is_ok(),
            error: result.err().map(|e| e.report),
        });
    }
}

/// Compiles `init.sqf` and spawns it as a scheduled script. A compile failure is a failed
/// [`ScriptRun`]; a mission without `init.sqf` spawns nothing.
fn spawn_init_sqf<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    report: &mut RunReport,
    codes: &mut Vec<Code>,
) -> Option<ScriptHandle> {
    if mission.folder.is_empty() {
        return None;
    }
    let path = format!("{}\\init.sqf", mission.folder);
    if !vm.host.file_exists(&path) {
        return None;
    }
    // Outside an init field `this` is undefined, as it is in the engine; only the init fields
    // above set it.
    vm.set_global("this", Value::Nil);
    let code: Code = match vm
        .host
        .preprocess_file(&path, true)
        .and_then(|text| vm.compile_file(&path, &text).map_err(|e| e.message))
    {
        Ok(code) => code,
        Err(message) => {
            report.scripts.push(ScriptRun {
                name: "init.sqf".to_owned(),
                ok: false,
                error: Some(message),
            });
            return None;
        }
    };
    codes.push(code.clone());
    Some(vm.spawn(&code, Value::Nil))
}
