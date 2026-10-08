//! Process-wide engine state shared by every subsystem.

use std::path::PathBuf;

/// Engine-wide settings and paths, created once at startup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineContext {
    /// Root of the user's Arma 3 installation (`--game-dir` or `A3_ROOT`), if known. Content
    /// loading (VFS, config) mounts from here once those subsystems are wired in.
    pub game_dir: Option<PathBuf>,
}
