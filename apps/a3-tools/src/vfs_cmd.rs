//! `a3-tools vfs ...`: browse the VFS of a game install.

use std::io::Write;
use std::path::{Path, PathBuf};

use a3_vfs::Vfs;
use anyhow::{Context, Result};

/// Mounts the game at `game_dir` plus `mods` (and every optional mod folder when `all_mods`).
pub fn mount(game_dir: &Path, mods: &[PathBuf], all_mods: bool) -> Result<Vfs> {
    let mut mods = mods.to_vec();
    if all_mods {
        mods.splice(0..0, a3_vfs::optional_mod_dirs(game_dir));
    }
    let vfs = Vfs::new();
    let report = vfs.mount_game(game_dir, &mods);
    eprintln!(
        "mounted {} PBOs ({} files) in {:.2?}; {} encrypted skipped, {} failed",
        report.pbos,
        vfs.len(),
        report.elapsed,
        report.encrypted.len(),
        report.failed.len()
    );
    for (path, reason) in &report.failed {
        eprintln!("  failed: {}: {reason}", path.display());
    }
    if vfs.is_empty() {
        anyhow::bail!("no PBOs found under {}", game_dir.display());
    }
    Ok(vfs)
}

/// Reads `input`: an OS file when one exists at that path, otherwise a VFS path in the game
/// at `game_dir` (mounted with every optional mod folder).
pub fn read_file_or_vfs(input: &str, game_dir: Option<&Path>) -> Result<Vec<u8>> {
    let on_disk = Path::new(input);
    if on_disk.is_file() {
        return std::fs::read(on_disk).with_context(|| format!("cannot read {input}"));
    }
    let Some(game_dir) = game_dir else {
        anyhow::bail!(
            "{input} is not a file, and no --game-dir (or A3_ROOT) is set to look it up in the VFS"
        );
    };
    let vfs = mount(game_dir, &[], true)?;
    Ok(vfs.open(input)?.to_vec())
}

/// Lists the children of `dir`, directories marked with a trailing `\`.
pub fn ls(vfs: &Vfs, dir: &str) -> Result<()> {
    let entries = vfs.list_dir(dir);
    if entries.is_empty() && vfs.exists(dir) {
        let info = vfs.stat(dir).expect("file exists");
        println!("{:>12}  {dir}", info.size);
        return Ok(());
    }
    let base = a3_vfs::VfsPath::new(dir);
    for entry in entries {
        if entry.is_dir {
            println!("{:>12}  {}\\", "", entry.name);
        } else {
            let size = vfs
                .stat(base.join(&entry.name).as_str())
                .map_or(0, |i| i.size);
            println!("{size:>12}  {}", entry.name);
        }
    }
    Ok(())
}

/// Writes the content of the file at `path` to standard output.
pub fn cat(vfs: &Vfs, path: &str) -> Result<()> {
    let data = vfs.open(path)?;
    let mut out = std::io::stdout().lock();
    match out.write_all(&data).and_then(|()| out.flush()) {
        // The reader (for example `head`) closed the pipe early; that is not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        result => result.context("cannot write to stdout"),
    }
}

/// Prints every file path matching `pattern`.
pub fn find(vfs: &Vfs, pattern: &str) -> Result<()> {
    for path in vfs.glob(pattern) {
        println!("{path}");
    }
    Ok(())
}

/// Prints the size and origin of the file at `path`.
pub fn stat(vfs: &Vfs, path: &str) -> Result<()> {
    let info = vfs
        .stat(path)
        .with_context(|| format!("file not found in VFS: {path}"))?;
    println!("size   {}", info.size);
    match info.source {
        Some(source) => println!("source {}", source.display()),
        None => println!("source <memory>"),
    }
    Ok(())
}
