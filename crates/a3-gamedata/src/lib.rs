//! Loads a game install: the engine's "load the game" facade.
//!
//! [`GameData::load`] mounts the [`Vfs`] (game, official DLC, then mods) and builds the merged
//! config (`configFile`): every `config.bin` of every mounted PBO, including those in subfolders,
//! is read and patched into one [`ConfigTree`] in CfgPatches `requiredAddons` order, with the
//! mount order as the tie-break. Renderer, SQF and simulation code start from a [`GameData`].
//!
//! Unbinarised `config.cpp` files (common in mods) are preprocessed with [`a3_preproc`], their
//! `#include`s resolved through the VFS and their `__EXEC`/`__EVAL` run on an SQF VM in
//! `parsingNamespace`, then merged like `config.bin`. When a folder has both, `config.bin` is used
//! _(uncertain: which one the engine prefers)_. [`VfsResolver`] and [`VfsHost`] give scripts the
//! same file access.
//!
//! Localisation is pluggable through the [`Localizer`] trait so that the stringtable crate can be
//! attached without this crate depending on it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3_config::{AddonPatches, Config, ConfigTree, load_order, read_rap};
use a3_core::VfsPath;
use a3_vfs::{MountReport, Vfs};

mod boot;
mod scripts;
mod sqf_config;

pub use boot::{
    CompileStats, FunctionsReport, compile_all, engine_command_table, error_category,
    init_functions, register_headless, script_registry, script_vm, unimplemented_usage,
    unimplemented_usage_in,
};
pub use scripts::{VfsHost, VfsResolver, decode_text, load_text_config, read_text};
pub use sqf_config::{ConfigHost, ConfigRoot, SqfConfigs, register_config_commands};

/// Errors that stop a game load. Problems with individual addons are collected in
/// [`LoadReport`] instead.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Neither `--game-dir` nor the `A3_ROOT` environment variable gave a game folder.
    #[error("no game folder given: pass --game-dir or set A3_ROOT")]
    NoGameDir,
    /// The game folder does not exist.
    #[error("game folder not found: {0}")]
    GameDirNotFound(PathBuf),
    /// No PBO was found to mount.
    #[error("no PBOs found under {0}")]
    NothingMounted(PathBuf),
}

/// What to load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadOptions {
    /// The game install folder (the one holding `arma3_x64.exe`, `Addons`, `Dta`).
    pub game_dir: PathBuf,
    /// Mod folders loaded after the game, in `-mod=` order.
    pub mods: Vec<PathBuf>,
    /// Also load the optional DLC / creator DLC / `@mod` folders found in the game folder
    /// (before `mods`).
    pub optional_mods: bool,
}

impl LoadOptions {
    /// Load the game at `game_dir` with no mods.
    pub fn new(game_dir: impl Into<PathBuf>) -> Self {
        Self {
            game_dir: game_dir.into(),
            ..Self::default()
        }
    }

    /// Load the game at `game_dir`, or at `A3_ROOT` when `None`.
    pub fn from_dir_or_env(game_dir: Option<PathBuf>) -> Result<Self, Error> {
        game_dir
            .or_else(|| std::env::var_os("A3_ROOT").map(PathBuf::from))
            .map(Self::new)
            .ok_or(Error::NoGameDir)
    }

    /// Adds mod folders, loaded after the game in the given order.
    pub fn with_mods<P: AsRef<Path>>(mut self, mods: impl IntoIterator<Item = P>) -> Self {
        self.mods
            .extend(mods.into_iter().map(|m| m.as_ref().to_path_buf()));
        self
    }

    /// Also loads every optional mod folder found in the game folder.
    pub fn with_optional_mods(mut self, yes: bool) -> Self {
        self.optional_mods = yes;
        self
    }
}

/// One addon config, in the order it was merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonConfig {
    /// VFS path of the `config.bin`.
    pub path: VfsPath,
    /// The PBO file it came from (`None` for in-memory mounts).
    pub archive: Option<PathBuf>,
    /// CfgPatches classes it declares.
    pub patches: Vec<String>,
    /// Union of their `requiredAddons`.
    pub required: Vec<String>,
}

/// Wall-clock time of each load phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadTimings {
    pub mount: Duration,
    pub read_configs: Duration,
    pub merge: Duration,
    pub total: Duration,
}

/// Non-fatal problems and statistics of a load.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    /// What mounting found (PBO count, skipped EBOs, broken archives).
    pub mount: MountReport,
    /// `config.bin` files that could not be read or parsed; they are left out of the config.
    pub config_errors: Vec<(VfsPath, String)>,
    /// `(config, required addon)` for requirements no loaded addon provides.
    pub missing_requirements: Vec<(VfsPath, String)>,
    /// Configs force-loaded to break a `requiredAddons` cycle.
    pub cycles: Vec<VfsPath>,
    /// Unbinarised `config.cpp` files ignored because their folder also has a `config.bin`.
    pub skipped_config_cpp: Vec<VfsPath>,
    pub timings: LoadTimings,
}

/// Turns `$STR_...` keys into text. Implemented by the stringtable crate.
pub trait Localizer: Send + Sync {
    /// The text for `key` (e.g. `STR_A3_Rifleman`, without `$`) in the current language, or
    /// `None` when unknown. Keys compare case-insensitively.
    fn localize(&self, key: &str) -> Option<String>;
}

/// A loaded game: its files, its merged config, and how they were assembled.
pub struct GameData {
    /// Every mounted file.
    pub vfs: Vfs,
    /// The merged config (`configFile`), shared read-only.
    pub config: Arc<ConfigTree>,
    /// Addon configs in merge order.
    pub addons: Vec<AddonConfig>,
    pub report: LoadReport,
    /// Localisation for `$STR_` text; `None` until a stringtable is attached.
    pub localizer: Option<Arc<dyn Localizer>>,
}

impl std::fmt::Debug for GameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameData")
            .field("vfs", &self.vfs)
            .field("addons", &self.addons.len())
            .field("config_nodes", &self.config.node_count())
            .field("localizer", &self.localizer.is_some())
            .finish()
    }
}

impl GameData {
    /// Mounts the game and mods, then builds the merged config.
    pub fn load(options: &LoadOptions) -> Result<Self, Error> {
        let start = Instant::now();
        if !options.game_dir.is_dir() {
            return Err(Error::GameDirNotFound(options.game_dir.clone()));
        }
        let mut mods = Vec::new();
        if options.optional_mods {
            mods.extend(a3_vfs::optional_mod_dirs(&options.game_dir));
        }
        mods.extend(options.mods.iter().cloned());

        let vfs = Vfs::new();
        let mount = vfs.mount_game(&options.game_dir, &mods);
        if mount.pbos == 0 {
            return Err(Error::NothingMounted(options.game_dir.clone()));
        }
        let mount_time = start.elapsed();
        let mut data = Self::from_vfs(vfs);
        data.report.mount = mount;
        data.report.timings.mount = mount_time;
        data.report.timings.total = start.elapsed();
        Ok(data)
    }

    /// Builds the merged config from an already mounted VFS, in its archive mount order.
    pub fn from_vfs(vfs: Vfs) -> Self {
        let start = Instant::now();
        let mut report = LoadReport::default();

        // Every config.bin (or, without one, config.cpp) of every archive, in mount order.
        let mut sources = Vec::new();
        for archive in vfs.archives() {
            let bins: std::collections::HashSet<VfsPath> = archive
                .pbo
                .entries()
                .iter()
                .map(|entry| archive.prefix.join(entry.name()))
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("config.bin"))
                })
                .filter_map(|path| path.parent())
                .collect();
            for entry in archive.pbo.entries() {
                let path = archive.prefix.join(entry.name());
                match path.file_name() {
                    Some(name) if name.eq_ignore_ascii_case("config.bin") => {
                        sources.push((archive.clone(), entry.clone(), path, false));
                    }
                    Some(name) if name.eq_ignore_ascii_case("config.cpp") => {
                        if path.parent().is_some_and(|dir| bins.contains(&dir)) {
                            report.skipped_config_cpp.push(path);
                        } else {
                            sources.push((archive.clone(), entry.clone(), path, true));
                        }
                    }
                    _ => {}
                }
            }
        }

        // config.bin files are read in parallel; config.cpp files run SQF (`__EXEC`/`__EVAL`)
        // on one VM in order, so they are done afterwards on this thread.
        let parsed = parallel_map(&sources, |(archive, entry, _, is_text)| {
            if *is_text {
                return None;
            }
            let bytes = match archive.pbo.read_entry(entry) {
                Ok(bytes) => bytes,
                Err(e) => return Some(Err(e.to_string())),
            };
            Some(read_rap(&bytes).map_err(|e| e.to_string()))
        });
        let mut parsing_vm = a3_sqf::Vm::new(VfsHost::new(vfs.clone()));
        let mut configs: Vec<(AddonConfig, Config)> = Vec::new();
        for ((archive, _, path, _), result) in sources.into_iter().zip(parsed) {
            let result =
                result.unwrap_or_else(|| load_text_config(&vfs, &mut parsing_vm, path.as_str()));
            match result {
                Ok(config) => {
                    let AddonPatches { patches, required } = AddonPatches::from_config(&config);
                    let addon = AddonConfig {
                        path,
                        archive: archive.source.clone(),
                        patches,
                        required,
                    };
                    configs.push((addon, config));
                }
                Err(e) => report.config_errors.push((path, e)),
            }
        }
        report.timings.read_configs = start.elapsed();

        let merge_start = Instant::now();
        let patches: Vec<AddonPatches> = configs
            .iter()
            .map(|(a, _)| AddonPatches {
                patches: a.patches.clone(),
                required: a.required.clone(),
            })
            .collect();
        let order = load_order(&patches);
        report.missing_requirements = order
            .missing
            .iter()
            .map(|(i, name)| (configs[*i].0.path.clone(), name.clone()))
            .collect();
        report.cycles = order
            .cycles
            .iter()
            .map(|&i| configs[i].0.path.clone())
            .collect();
        let mut tree = ConfigTree::new();
        let mut slots: Vec<Option<(AddonConfig, Config)>> = configs.into_iter().map(Some).collect();
        let mut addons = Vec::with_capacity(slots.len());
        for i in order.order {
            let (addon, config) = slots[i].take().expect("each addon is ordered once");
            tree.merge(&config);
            addons.push(addon);
        }
        report.timings.merge = merge_start.elapsed();
        report.timings.total = start.elapsed();

        Self {
            vfs,
            config: Arc::new(tree),
            addons,
            report,
            localizer: None,
        }
    }

    /// Localises config text: `$STR_key` becomes the localizer's text for `STR_key`; any other
    /// text, or a key the localizer does not know, is returned unchanged.
    pub fn localize(&self, text: &str) -> String {
        let key = text
            .strip_prefix('$')
            .filter(|k| k.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("STR_")));
        match (key, &self.localizer) {
            (Some(key), Some(localizer)) => {
                localizer.localize(key).unwrap_or_else(|| text.to_owned())
            }
            _ => text.to_owned(),
        }
    }
}

/// Maps `f` over `items` on all cores, keeping order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(items.len().max(1));
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("config reader thread panicked"))
            .collect()
    })
}
