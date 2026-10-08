//! Process-wide engine state shared by every subsystem.

use std::path::PathBuf;

use crate::world::CameraSpec;

/// Engine-wide settings and paths, created once at startup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineContext {
    /// Root of the user's Arma 3 installation (`--game-dir` or `A3_ROOT`), if known. Content
    /// loading (VFS, config) mounts from here once those subsystems are wired in.
    pub game_dir: Option<PathBuf>,
    /// The World (CfgWorlds class) to fly over, if any.
    pub world: Option<String>,
    /// Initial camera placement over the World.
    pub camera: Option<CameraSpec>,
    /// Distance fog density override.
    pub fog: Option<f32>,
}

impl EngineContext {
    /// Apply command-line overrides of the render settings.
    pub fn configure(&self, renderer: &mut a3_render::Renderer) {
        if let Some(fog) = self.fog {
            renderer.settings.fog_density = fog;
        }
    }

    /// Load the requested World, if any.
    pub fn load_world(&self) -> anyhow::Result<Option<crate::world::LoadedWorld>> {
        let Some(name) = &self.world else {
            return Ok(None);
        };
        let Some(dir) = &self.game_dir else {
            anyhow::bail!("--world needs the game directory (--game-dir or A3_ROOT)");
        };
        crate::world::load(dir, name).map(Some)
    }
}
