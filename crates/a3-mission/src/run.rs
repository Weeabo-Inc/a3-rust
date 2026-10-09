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

use std::rc::Rc;
use std::time::{Duration, Instant};

use a3_gamedata::{ConfigHost, VfsHost, script_registry};
use a3_sqf::{Code, Host, Namespace, Registry, Value, Vm};
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
    /// Scheduler frames `init.sqf` ran for (0 when the mission has none).
    pub frames: usize,
    /// Wall-clock time of the run.
    pub elapsed: Duration,
}

/// Installs the mission's variables and runs its scripts: every spawned unit's `init` first
/// (unscheduled, `this` = the unit), then `init.sqf` as a scheduled script.
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
    run_init_sqf(vm, mission, &mut report, &mut codes);
    // What the mission's own scripts used (the codes just compiled), plus whatever the scripts
    // put into the mission namespace themselves.
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for (name, count) in a3_gamedata::unimplemented_usage_in(vm, &codes)
        .into_iter()
        .chain(a3_gamedata::unimplemented_usage(vm, Namespace::Mission))
    {
        *counts.entry(name).or_default() += count;
    }
    let mut missing: Vec<(String, usize)> = counts.into_iter().collect();
    missing.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    report.missing_commands = missing;
    report.errors = vm.host.errors()[errors_before..].to_vec();
    report.log_lines = vm.host.log().len() - log_before;
    report.elapsed = start.elapsed();
    report
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

fn run_init_sqf<H: MissionHost>(
    vm: &mut Vm<H>,
    mission: &Mission,
    report: &mut RunReport,
    codes: &mut Vec<Code>,
) {
    if mission.folder.is_empty() {
        return;
    }
    let path = format!("{}\\init.sqf", mission.folder);
    if !vm.host.file_exists(&path) {
        return;
    }
    // Outside an init field `this` is undefined, as it is in the engine; only the init fields
    // above set it.
    vm.set_global("this", Value::Nil);
    let name = "init.sqf".to_owned();
    let code: Code = match vm
        .host
        .preprocess_file(&path, true)
        .and_then(|text| vm.compile_file(&path, &text).map_err(|e| e.message))
    {
        Ok(code) => code,
        Err(message) => {
            report.scripts.push(ScriptRun {
                name,
                ok: false,
                error: Some(message),
            });
            return;
        }
    };
    codes.push(code.clone());
    let handle = vm.spawn(&code, Value::Nil);
    report.frames = vm.run_until_idle(MAX_INIT_FRAMES, |host| {
        host.world_mut().simulate(1.0 / 15.0);
    });
    let done = vm.script_done(handle);
    report.scripts.push(ScriptRun {
        name,
        ok: done,
        error: (!done).then(|| format!("still running after {MAX_INIT_FRAMES} frames")),
    });
}
