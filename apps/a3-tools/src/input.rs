//! Reading a tool's input file from disk or, failing that, from the game's VFS.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;

/// An input file named either by an OS path or by a VFS path inside the game install.
#[derive(Args)]
pub struct InputArgs {
    /// File on disk, or a VFS path (such as `a3\anims_f\...\file.rtm`) when no such file exists.
    pub input: String,
    /// Game install folder used to resolve VFS paths.
    #[arg(long, env = "A3_ROOT")]
    pub game_dir: Option<PathBuf>,
}

impl InputArgs {
    /// Reads the input: the file on disk if it exists, else the VFS file of that path.
    pub fn read(&self) -> anyhow::Result<Vec<u8>> {
        let path = Path::new(&self.input);
        if path.is_file() {
            return std::fs::read(path).with_context(|| format!("reading {}", path.display()));
        }
        let Some(game_dir) = &self.game_dir else {
            bail!(
                "{} is not a file, and no --game-dir (or A3_ROOT) is set to look it up in the VFS",
                self.input
            );
        };
        let vfs = a3_vfs::Vfs::new();
        vfs.mount_game(game_dir, &a3_vfs::optional_mod_dirs(game_dir));
        let data = vfs
            .open(&self.input)
            .with_context(|| format!("{} is neither a file nor in the VFS", self.input))?;
        Ok(data.to_vec())
    }
}
