//! Test-only helpers for reading real game data through `a3-pbo` / `a3-vfs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The game install root from `A3_ROOT`, or `None` (with a note on stderr) when unset.
pub fn game_root() -> Option<PathBuf> {
    match std::env::var_os("A3_ROOT") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("skipping: A3_ROOT not set");
            None
        }
    }
}

/// `*.pbo` files directly inside `dir`, sorted by name ignoring case.
fn pbos_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbo")))
        .collect();
    files.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
    files
}

fn child_dir(parent: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .find(|e| e.file_name().eq_ignore_ascii_case(name) && e.path().is_dir())
        .map(|e| e.path())
}

/// Every PBO of the install in engine discovery order: `Dta`, base `Addons`, the official DLC
/// folders in load order, then optional/creator DLC folders (as `-mod` would add them).
pub fn game_pbos(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dta) = child_dir(root, "dta") {
        out.extend(pbos_in(&dta));
    }
    let mut mods = vec![root.to_path_buf()];
    mods.extend(
        a3_vfs::OFFICIAL_MOD_DIRS
            .iter()
            .filter_map(|name| child_dir(root, name)),
    );
    mods.extend(a3_vfs::optional_mod_dirs(root));
    for dir in mods {
        if let Some(addons) = a3_vfs::find_addons_dir(&dir) {
            out.extend(pbos_in(&addons));
        }
    }
    out
}

/// Every `*.pbo` anywhere below `root` (including mission PBOs), sorted.
pub fn all_pbos(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pbo"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}
