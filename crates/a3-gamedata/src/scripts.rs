//! Text sources from the VFS: `#include` resolution for the preprocessor, a script host that
//! reads files from the VFS, and loading of unbinarised `config.cpp` files.

use a3_config::{Config, parse_text};
use a3_preproc::{
    IncludeError, IncludeResolver, Options, Preprocessor, ResolvedInclude, join_virtual_path,
};
use a3_sqf::{Handle, HandleKind, Host, ScriptError, SqfEvaluator, Vm};
use a3_vfs::Vfs;

use crate::Localizer;
use crate::sqf_config::{ConfigHost, SqfConfigs};
use std::sync::Arc;

/// Reads a VFS file as text: UTF-8 (invalid sequences replaced, a BOM dropped), or UTF-16 when
/// the file starts with a UTF-16 byte-order mark. The game ships at least one UTF-16 script
/// (`\a3\functions_f\GUI\editor\GUI_init.sqf`) _(uncertain: how the engine decodes it)_.
pub fn read_text(vfs: &Vfs, path: &str) -> Result<String, a3_vfs::Error> {
    let bytes = vfs.open(path)?;
    Ok(decode_text(&bytes))
}

/// Decodes file bytes as described in [`read_text`].
pub fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| from([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Resolves `#include` and `__has_include` against the [`Vfs`].
#[derive(Debug, Clone, Copy)]
pub struct VfsResolver<'a> {
    vfs: &'a Vfs,
}

impl<'a> VfsResolver<'a> {
    /// A resolver over `vfs`.
    pub fn new(vfs: &'a Vfs) -> Self {
        Self { vfs }
    }
}

impl IncludeResolver for VfsResolver<'_> {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = join_virtual_path(current_file, include);
        match read_text(self.vfs, &path) {
            Ok(source) => Ok(ResolvedInclude { path, source }),
            Err(a3_vfs::Error::NotFound(_)) => Err(IncludeError::NotFound(path)),
            Err(e) => Err(IncludeError::Io {
                path,
                message: e.to_string(),
            }),
        }
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.vfs.exists(&join_virtual_path(current_file, include))
    }
}

/// A script [`Host`] whose file commands (`loadFile`, `preprocessFile`,
/// `preprocessFileLineNumbers`, `execVM`, `compileScript`) read from the [`Vfs`]. Log output and
/// script errors are collected. Tools and tests use it directly; the game's host can delegate
/// its file loading to [`read_text`] the same way.
#[derive(Default)]
pub struct VfsHost {
    /// The files scripts can load.
    pub vfs: Vfs,
    /// Stringtable lookup for `localize`; unknown keys localize to `""` without one.
    pub localizer: Option<Arc<dyn Localizer>>,
    /// `diag_log` lines (including preprocessor warnings).
    pub log: Vec<String>,
    /// Reported script errors.
    pub errors: Vec<String>,
    /// Also write each script error into [`VfsHost::log`], in order with the `diag_log` lines,
    /// as the engine writes both to its RPT log.
    pub errors_in_log: bool,
    /// The configs the config commands read (`configFile`, `missionConfigFile`, ...).
    pub configs: SqfConfigs,
}

impl VfsHost {
    /// A host reading from `vfs`.
    pub fn new(vfs: Vfs) -> Self {
        Self {
            vfs,
            localizer: None,
            log: Vec::new(),
            errors: Vec::new(),
            errors_in_log: false,
            configs: SqfConfigs::default(),
        }
    }

    /// A host reading from `game`'s VFS whose `configFile` is `game`'s merged config.
    pub fn for_game(game: &crate::GameData) -> Self {
        let mut host = Self::new(game.vfs.clone());
        host.localizer = game.localizer.clone();
        host.configs = SqfConfigs::new(game.config.clone());
        host
    }
}

/// A host that keeps the script errors reported to it, in order.
pub trait ErrorLog {
    /// The script errors reported so far.
    fn error_log(&self) -> &[String];
}

impl ErrorLog for VfsHost {
    fn error_log(&self) -> &[String] {
        &self.errors
    }
}

impl ConfigHost for VfsHost {
    fn configs(&self) -> &SqfConfigs {
        &self.configs
    }

    fn configs_mut(&mut self) -> &mut SqfConfigs {
        &mut self.configs
    }

    fn language(&self) -> &str {
        self.localizer
            .as_ref()
            .map_or(a3_stringtable::ENGLISH, |localizer| localizer.language())
    }
}

impl std::fmt::Debug for VfsHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VfsHost")
            .field("vfs", &self.vfs)
            .field("localizer", &self.localizer.is_some())
            .field("log", &self.log.len())
            .field("errors", &self.errors.len())
            .finish()
    }
}

impl Host for VfsHost {
    fn localize(&self, key: &str) -> Option<String> {
        self.localizer.as_ref()?.localize(key)
    }

    fn diag_log(&mut self, text: &str) {
        self.log.push(text.to_owned());
    }

    fn report_error(&mut self, error: &ScriptError) {
        if self.errors_in_log {
            self.log.extend(error.report.lines().map(str::to_owned));
        }
        self.errors.push(error.report.clone());
    }

    fn load_file(&mut self, path: &str) -> Result<String, String> {
        read_text(&self.vfs, path).map_err(|_| format!("Script {path} not found"))
    }

    fn format_handle(&self, handle: Handle) -> String {
        if handle.kind == HandleKind::Config {
            self.configs.format(handle)
        } else {
            handle.to_string()
        }
    }
}

/// Preprocesses and parses the text config at `path` (config.cpp, description.ext, ...).
/// `__EXEC`/`__EVAL` run on `vm` in `parsingNamespace`, which persists across configs as in the
/// engine.
pub fn load_text_config<H: Host>(vfs: &Vfs, vm: &mut Vm<H>, path: &str) -> Result<Config, String> {
    let path = join_virtual_path("", path);
    let source = read_text(vfs, &path).map_err(|e| e.to_string())?;
    let resolver = VfsResolver::new(vfs);
    let output = Preprocessor::new(&resolver)
        .with_options(Options::config())
        .with_evaluator(SqfEvaluator::new(vm))
        .preprocess_str(&path, &source)
        .map_err(|e| e.to_string())?;
    parse_text(&output.text).map_err(|e| format!("{path}: {e}"))
}
