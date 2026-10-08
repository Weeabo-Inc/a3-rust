//! Layout of a game install: which folders hold addons and in what order they load.

use std::path::{Path, PathBuf};

/// The official DLC folders the game loads by default, in load order (later overrides
/// earlier). They load after `Dta` and the base game's `Addons`.
///
/// Taken from the "Loaded mods" table of game logs, which lists the highest priority first.
/// _(uncertain: exact order; needs confirmation by reverse engineering.)_
pub const OFFICIAL_MOD_DIRS: &[&str] = &[
    "curator",
    "kart",
    "heli",
    "mark",
    "expansion",
    "jets",
    "argo",
    "orange",
    "tacops",
    "tank",
    "enoch",
    "aow",
];

/// Folders directly under `root` that hold an `addons` folder but are not loaded by default:
/// optional DLC and creator DLC folders (`Contact`, `GM`, `vn`, ...) and `@mod` folders. Sorted
/// by name, ignoring case.
pub fn optional_mod_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            !OFFICIAL_MOD_DIRS
                .iter()
                .any(|official| official.eq_ignore_ascii_case(&name))
        })
        .filter(|path| find_addons_dir(path).is_some())
        .collect();
    dirs.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
    dirs
}

/// The `addons` folder inside `mod_dir`, matched case-insensitively.
pub fn find_addons_dir(mod_dir: &Path) -> Option<PathBuf> {
    find_child_dir(mod_dir, "addons")
}

pub(crate) fn find_child_dir(parent: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .find(|e| e.file_name().eq_ignore_ascii_case(name) && e.path().is_dir())
        .map(|e| e.path())
}

/// PBO and EBO files directly inside `dir`, sorted by name ignoring case.
pub(crate) fn archives_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pbo") || e.eq_ignore_ascii_case("ebo"))
        })
        .collect();
    files.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
    files
}

pub(crate) fn is_ebo(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ebo"))
}
