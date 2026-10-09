//! Loading a mission from the VFS: `mission.sqm`, `description.ext` and the folder's terrain.

use a3_config::ConfigTree;
use a3_gamedata::{ConfigHost, ConfigRoot, load_text_config};
use a3_sqf::{Host, Vm};
use a3_vfs::Vfs;

use crate::mission::{Mission, MissionError};
use crate::sqm::{SqmError, parse_sqm};

/// Errors from loading a mission.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The folder has no `mission.sqm`.
    #[error("no mission.sqm at {0}")]
    NoSqm(String),
    /// A file could not be read from the VFS.
    #[error("cannot read {path}: {source}")]
    Vfs {
        /// The virtual path.
        path: String,
        /// The VFS error.
        source: a3_vfs::Error,
    },
    /// `mission.sqm` did not parse.
    #[error("{path}: {source}")]
    Sqm {
        /// The virtual path.
        path: String,
        /// The parse error.
        source: SqmError,
    },
    /// The parsed `mission.sqm` is not usable.
    #[error("{0}")]
    Mission(#[from] MissionError),
}

/// Normalises a virtual folder path: no leading or trailing separators.
fn normalize(folder: &str) -> String {
    folder
        .trim()
        .trim_matches(|c| c == '\\' || c == '/')
        .replace('/', "\\")
}

/// The terrain a mission folder names through its extension: `...\boot_m02.altis` → `altis`.
fn terrain_of(folder: &str) -> Option<String> {
    let name = folder.rsplit('\\').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    // Mission folder names are `<name>.<world>`, e.g. `boot_m02.altis`; a misspelt world is still
    // what the folder says it is.
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Reads `folder\mission.sqm` and, when it exists, `folder\description.ext`, and returns the
/// [`Mission`].
///
/// A `description.ext` that does not preprocess or parse is logged (as the engine logs
/// `Preprocessor failed on file '<path>'`) and the mission is loaded without it: the engine starts
/// the mission with an empty mission config, it does not drop the mission
/// (`tools/oracle/probes/missing_include.py`, #348).
///
/// `folder` is a virtual path (`a3\missions_f_bootcamp\campaign\missions\boot_m02.altis`); a
/// trailing separator and forward slashes are accepted. Folders on disk are loaded by mounting
/// them first ([`Vfs::mount_dir`] or [`Vfs::mount_pbo_file`]).
pub fn load_mission<H: Host>(
    vfs: &Vfs,
    folder: &str,
    vm: &mut Vm<H>,
) -> Result<Mission, LoadError> {
    let folder = normalize(folder);
    let sqm_path = format!("{folder}\\mission.sqm");
    let bytes = vfs.open(&sqm_path).map_err(|source| match source {
        a3_vfs::Error::NotFound(_) => LoadError::NoSqm(sqm_path.clone()),
        source => LoadError::Vfs {
            path: sqm_path.clone(),
            source,
        },
    })?;
    let config = parse_sqm(&bytes).map_err(|source| LoadError::Sqm {
        path: sqm_path.clone(),
        source,
    })?;
    let mut mission = Mission::from_config(&config)?;
    mission.folder = folder.clone();
    mission.terrain = terrain_of(&folder);
    let description_path = format!("{folder}\\description.ext");
    if vfs.exists(&description_path) {
        match load_text_config(vfs, vm, &description_path) {
            Ok(description) => mission.description = Some(description),
            // Nothing of the failed config is installed, not even the entries before the error.
            Err(message) => vm
                .host
                .diag_log(&format!("Warning: preprocessor: {message}")),
        }
    }
    Ok(mission)
}

/// Installs the mission's `description.ext` as `missionConfigFile` of the VM's config host, so
/// scripts (`getMissionConfigValue`, `missionConfigFile >> ...`) see it. Leaves an undefined
/// (empty) mission config root when the mission has none.
pub fn install_mission_config<H: ConfigHost>(vm: &mut Vm<H>, mission: &Mission) {
    let tree = match &mission.description {
        Some(config) => ConfigTree::from_config(config),
        None => ConfigTree::with_root_name("description.ext"),
    };
    vm.host
        .configs_mut()
        .set(ConfigRoot::Mission, std::sync::Arc::new(tree));
}
