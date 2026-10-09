//! Running one scenario headlessly: terrain, `mission.sqm`, spawn, start-up (function library,
//! unit inits, `init.sqf`), a fixed-dt simulation, then an inspection of what happened.
//!
//! A [`Runner`] holds what every scenario shares: the mounted game, its merged config, the
//! function library compiled once at "game start" (uiNamespace, copied into each scenario's
//! VM), and the last terrain loaded. Each scenario gets a fresh World and VM.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3_config::ConfigTree;
use a3_gamedata::{ConfigRoot, GameData, LoadOptions, VfsHost, init_functions, script_vm};
use a3_mission::{
    Mission, MissionVmHost, Spawned, StartOptions, load_mission, spawn_mission, start_mission, step,
};
use a3_sqf::{Code, Namespace, Vm};
use a3_world::{ClientId, TypeBank, World};
use a3_wrp::Terrain;
use anyhow::Context as _;

use crate::inventory::{Scenario, mount_loose_missions};
use crate::report::{Sanity, ScenarioResult, Stage, Status, Unspawned, group_errors, ranked};

/// How each scenario is run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunOptions {
    /// Simulated seconds after start-up.
    pub seconds: f64,
    /// Simulation frames per simulated second (dt = 1 / fps).
    pub fps: f64,
    /// Wall-clock time the simulation of one scenario may take; it stops early after that.
    pub budget: Duration,
    /// Wall-clock time the scheduled scripts get per frame.
    pub frame_budget: Duration,
    /// Load the world's terrain (heights, objects); without it the World is flat at 0 m.
    pub terrain: bool,
}

/// The game, loaded once, plus caches shared by the scenarios one process runs.
pub struct Runner {
    pub data: GameData,
    /// The VM the function library was compiled in at game start; its uiNamespace is copied
    /// into every scenario.
    library: Vm<VfsHost>,
    /// The last loaded terrain, by CfgWorlds class.
    terrain: Option<(String, Arc<Terrain>)>,
    pub options: RunOptions,
    /// Whether the optional DLC folders (Contact, creator DLC) are loaded.
    pub optional_mods: bool,
    /// Time [`Runner::new`] took.
    pub load_time: Duration,
    /// `<tag>_fnc_<name>` functions compiled at game start.
    pub functions: usize,
}

/// Wraps the stringtables for the script host's `localize`.
struct Strings(a3_stringtable::Localizer);

impl a3_gamedata::Localizer for Strings {
    fn localize(&self, key: &str) -> Option<String> {
        self.0.get(key).map(str::to_owned)
    }
}

/// The [`Runner`] for the game as each scenario needs it: the base game with its default DLC,
/// or with the optional DLC too ([`Scenario::optional_mods`]). Optional DLC change the game for
/// everything else (Contact, for one, deletes every other campaign from `CfgMissions`), so a
/// scenario that does not need them runs without. Switching reloads the game.
pub struct Engine {
    game_dir: std::path::PathBuf,
    options: RunOptions,
    current: Option<Runner>,
}

impl Engine {
    pub fn new(game_dir: &Path, options: RunOptions) -> Engine {
        Engine {
            game_dir: game_dir.to_owned(),
            options,
            current: None,
        }
    }

    /// The runner with or without the optional DLC, loading it when the current one differs.
    pub fn runner(&mut self, optional_mods: bool) -> anyhow::Result<&mut Runner> {
        if self
            .current
            .as_ref()
            .is_none_or(|r| r.optional_mods != optional_mods)
        {
            self.current = None;
            self.current = Some(Runner::new(&self.game_dir, self.options, optional_mods)?);
        }
        Ok(self.current.as_mut().expect("loaded above"))
    }
}

impl Runner {
    /// Mounts the game (with the optional DLC when asked), the loose mission PBOs and the
    /// stringtables, and compiles the function library.
    pub fn new(
        game_dir: &Path,
        options: RunOptions,
        optional_mods: bool,
    ) -> anyhow::Result<Runner> {
        let start = Instant::now();
        let mut data =
            GameData::load(&LoadOptions::new(game_dir).with_optional_mods(optional_mods))
                .with_context(|| format!("cannot load the game in {}", game_dir.display()))?;
        mount_loose_missions(&data.vfs, game_dir);
        let mounted = start.elapsed();
        let (strings, _) = a3_stringtable::Localizer::load_vfs(&data.vfs, "English");
        data.localizer = Some(Arc::new(Strings(strings)));
        let localized = start.elapsed();
        let mut library = script_vm(&data);
        let report = init_functions(&mut library);
        eprintln!(
            "game loaded (optional DLC {optional_mods}): files and config {:.1} s, stringtables \
             {:.1} s, {} functions {:.1} s",
            mounted.as_secs_f64(),
            (localized - mounted).as_secs_f64(),
            report.compiled,
            (start.elapsed() - localized).as_secs_f64()
        );
        Ok(Runner {
            data,
            library,
            terrain: None,
            options,
            optional_mods,
            load_time: start.elapsed(),
            functions: report.compiled,
        })
    }

    /// Runs one scenario. `on_stage` hears each stage as it starts.
    pub fn run(&mut self, scenario: &Scenario, mut on_stage: impl FnMut(Stage)) -> ScenarioResult {
        let started = Instant::now();
        let mut result = ScenarioResult::failed(
            scenario.clone(),
            Status::Errors,
            Stage::Queued,
            String::new(),
        );
        result.failure = None;
        let fail = |mut result: ScenarioResult, stage: Stage, message: String| {
            result.status = Status::LoadFailed;
            result.stage = stage;
            result.failure = Some(message);
            result.timings.total_ms = ms(started.elapsed());
            result
        };

        // Terrain.
        on_stage(Stage::Terrain);
        let t = Instant::now();
        let terrain = match self.terrain_for(&scenario.world) {
            Ok(terrain) => terrain,
            Err(e) => return fail(result, Stage::Terrain, format!("{e:#}")),
        };
        result.timings.terrain_ms = ms(t.elapsed());

        // Mission.
        on_stage(Stage::Load);
        let t = Instant::now();
        let mut world = World::new(ClientId::SERVER);
        if let Some(terrain) = terrain
            && let Err(e) = world.load_terrain(terrain)
        {
            return fail(result, Stage::Load, format!("terrain: {e}"));
        }
        let types = TypeBank::new(self.data.config.clone());
        let mut vm = MissionVmHost::for_game(world, types, &self.data).vm();
        self.copy_library(&mut vm);
        let mission = match load_mission(&self.data.vfs, &scenario.folder, &mut vm) {
            Ok(mission) => mission,
            Err(e) => return fail(result, Stage::Load, e.to_string()),
        };
        if let Some(campaign) = &scenario.campaign {
            install_campaign_config(&self.data, &mut vm, campaign);
        }
        result.timings.load_ms = ms(t.elapsed());

        // Spawn.
        on_stage(Stage::Spawn);
        let t = Instant::now();
        let host = &mut vm.host;
        let spawned = spawn_mission(&mut host.world, &mut host.types, &mission);
        result.units_declared = mission.units().filter(|u| !u.is_absent()).count();
        result.sqm_version = mission.version;
        result.units_spawned = spawned.units.len();
        result.unspawned = spawned
            .unspawned
            .iter()
            .map(|u| Unspawned {
                class: u.class.clone(),
                reason: u.reason.clone(),
            })
            .collect();
        result.timings.spawn_ms = ms(t.elapsed());

        // Start-up.
        on_stage(Stage::Start);
        let t = Instant::now();
        let report = start_mission(
            &mut vm,
            &mission,
            &spawned,
            StartOptions { functions: true },
        );
        result.failed_scripts = report
            .scripts
            .iter()
            .filter(|s| !s.ok)
            .map(|s| s.name.clone())
            .collect();
        result.timings.start_ms = ms(t.elapsed());

        // Simulation.
        on_stage(Stage::Simulate);
        let t = Instant::now();
        let dt = 1.0 / self.options.fps.max(1.0);
        let frames = (self.options.seconds * self.options.fps).round().max(0.0) as u64;
        let mut worst = Duration::ZERO;
        for _ in 0..frames {
            if t.elapsed() > self.options.budget {
                result.budget_exhausted = true;
                break;
            }
            let frame = Instant::now();
            step(&mut vm, dt, self.options.frame_budget);
            worst = worst.max(frame.elapsed());
            result.frames += 1;
        }
        result.sim_seconds = result.frames as f64 * dt;
        result.timings.simulate_ms = ms(t.elapsed());
        if result.frames > 0 {
            result.timings.frame_ms_mean = result.timings.simulate_ms / result.frames as f64;
            result.timings.frame_ms_max = ms(worst);
        }

        // Inspection.
        on_stage(Stage::Inspect);
        result.scripts_running = vm.scheduled_count();
        self.inspect(&mut vm, &mission, &spawned, &mut result);
        result.stage = Stage::Done;
        result.status = result.grade();
        result.timings.total_ms = ms(started.elapsed());
        result
    }

    /// The terrain of `world` (a folder extension), loading it unless it is the cached one.
    /// `None` when terrain loading is off.
    fn terrain_for(&mut self, world: &str) -> anyhow::Result<Option<Arc<Terrain>>> {
        let class = self.data.config.root().get("CfgWorlds").get(world);
        anyhow::ensure!(
            !world.is_empty() && class.is_class(),
            "no world `{world}` in CfgWorlds"
        );
        if !self.options.terrain {
            return Ok(None);
        }
        let name = class.name().to_owned();
        if let Some((cached, terrain)) = &self.terrain
            && cached.eq_ignore_ascii_case(&name)
        {
            return Ok(Some(terrain.clone()));
        }
        self.terrain = None;
        let config = a3_landscape::WorldConfig::load(&self.data.config, &name)
            .with_context(|| format!("CfgWorlds >> {name}"))?;
        let wrp = config.wrp.as_str();
        let bytes = self
            .data
            .vfs
            .open(wrp)
            .with_context(|| format!("cannot open {wrp}"))?;
        let terrain =
            Arc::new(Terrain::parse(&bytes).with_context(|| format!("cannot parse {wrp}"))?);
        self.terrain = Some((name, terrain.clone()));
        Ok(Some(terrain))
    }

    /// Copies the game-start uiNamespace (the compiled function library) into `vm`. Arrays and
    /// hash maps are copied deeply so no scenario changes what the next one sees.
    fn copy_library(&self, vm: &mut Vm<MissionVmHost>) {
        let ui = vm.namespace_mut(Namespace::Ui);
        for (name, value) in self.library.namespace(Namespace::Ui).iter() {
            ui.set(name, a3_sqf::value::deep_copy_value(value));
        }
    }

    /// Fills in errors, unimplemented commands, missing models and the sanity checks.
    fn inspect(
        &self,
        vm: &mut Vm<MissionVmHost>,
        mission: &Mission,
        spawned: &Spawned,
        result: &mut ScenarioResult,
    ) {
        let (errors, runtime) = group_errors(vm.host.files.errors.iter().map(String::as_str));
        result.error_count = errors.iter().map(|e| e.count).sum();
        result.errors = errors;
        result.unimplemented_runtime = runtime;

        let (codes, compile_errors) = compile_mission_scripts(vm, mission);
        result.compile_errors = compile_errors;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (name, count) in a3_gamedata::unimplemented_usage_in(vm, &codes) {
            // `name (form)`: the sweep ranks by name.
            let name = name.split(" (").next().unwrap_or(&name).to_owned();
            *counts.entry(name).or_default() += count;
        }
        result.unimplemented_static = ranked(counts);

        let world = &vm.host.world;
        let vfs = &self.data.vfs;
        let mut missing = BTreeSet::new();
        for entity in world.entities() {
            let model = model_path(entity.entity_type().model());
            if !model.is_empty() && !vfs.exists(&model) {
                missing.insert(model);
            }
        }
        result.missing_models = missing.into_iter().collect();

        let mut sanity = Sanity {
            player_expected: mission.player().is_some_and(|p| !p.is_absent()),
            ..Sanity::default()
        };
        if let Some(player) = spawned.player.and_then(|id| world.entity(id)) {
            sanity.player_present = true;
            sanity.player_alive = player.is_alive();
        }
        for entity in world.entities() {
            sanity.entities += 1;
            if entity.is_alive() {
                sanity.alive += 1;
            }
            let p = entity.position();
            if !p.is_finite() {
                sanity.non_finite_positions += 1;
                continue;
            }
            let surface = world.surface_height(p.x, p.z);
            if p.y < surface - 5.0 {
                sanity.below_terrain += 1;
            } else if p.y > surface + 2000.0 {
                sanity.far_above_terrain += 1;
            }
        }
        result.sanity = sanity;
    }
}

/// Loads `<campaign>\description.ext` as `campaignConfigFile`.
fn install_campaign_config(data: &GameData, vm: &mut Vm<MissionVmHost>, campaign: &str) {
    let path = format!("{campaign}\\description.ext");
    if !data.vfs.exists(&path) {
        return;
    }
    match a3_gamedata::load_text_config(&data.vfs, vm, &path) {
        Ok(config) => vm.host.files.configs.set(
            ConfigRoot::Campaign,
            Arc::new(ConfigTree::from_config(&config)),
        ),
        Err(message) => vm.host.files.errors.push(format!("{path}: {message}")),
    }
}

/// Compiles every script the mission brings: each `.sqf` in its folder, the unit and group
/// init fields and the trigger expressions. Returns the code and the compile errors.
fn compile_mission_scripts(
    vm: &mut Vm<MissionVmHost>,
    mission: &Mission,
) -> (Vec<Code>, Vec<(String, String)>) {
    let mut codes = Vec::new();
    let mut errors = Vec::new();
    let sqm = format!("{}\\mission.sqm", mission.folder);
    let files: Vec<String> = vm
        .host
        .files
        .vfs
        .walk(&mission.folder)
        .into_iter()
        .map(|p| p.as_str().to_owned())
        .filter(|p| p.ends_with(".sqf"))
        .collect();
    for path in files {
        let compiled = a3_sqf::Host::preprocess_file(&mut vm.host, &path, true)
            .and_then(|text| vm.compile_file(&path, &text).map_err(|e| e.message));
        match compiled {
            Ok(code) => codes.push(code),
            Err(message) => errors.push((path, message)),
        }
    }
    let mut expressions: Vec<(String, &str)> = Vec::new();
    for group in &mission.groups {
        if let Some(init) = &group.init {
            expressions.push(("group init".to_owned(), init));
        }
    }
    for unit in mission.units() {
        if let Some(init) = &unit.init {
            expressions.push((format!("init of unit {}", unit.id), init));
        }
    }
    for (i, trigger) in mission.triggers.iter().enumerate() {
        for text in [&trigger.exp_cond, &trigger.exp_activ, &trigger.exp_desactiv]
            .into_iter()
            .flatten()
        {
            expressions.push((format!("trigger {i}"), text));
        }
    }
    for (what, text) in expressions {
        match vm.compile_file(&sqm, text) {
            Ok(code) => codes.push(code),
            Err(e) => errors.push((format!("{sqm} ({what})"), e.message)),
        }
    }
    (codes, errors)
}

/// The VFS path of a config `model` value: no leading separator, `.p3d` added when missing.
fn model_path(model: &str) -> String {
    let model = model.trim().trim_start_matches(['\\', '/']);
    if model.is_empty() {
        return String::new();
    }
    let has_ext = model
        .rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.contains('.'));
    if has_ext {
        model.to_owned()
    } else {
        format!("{model}.p3d")
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths_get_their_extension_and_lose_the_leading_separator() {
        assert_eq!(model_path("\\A3\\x\\man.p3d"), "A3\\x\\man.p3d");
        assert_eq!(model_path("a3\\x\\box"), "a3\\x\\box.p3d");
        assert_eq!(model_path(""), "");
        assert_eq!(model_path("a3\\x.y\\box"), "a3\\x.y\\box.p3d");
    }
}
