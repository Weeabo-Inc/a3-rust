//! Mounts a real game install. Skipped when `A3_ROOT` is unset.

use std::path::Path;
use std::time::Instant;

use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn mounts_the_whole_install_and_resolves_known_files() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let mods = optional_mod_dirs(root);

    let start = Instant::now();
    let vfs = Vfs::new();
    let report = vfs.mount_game(root, &mods);
    let elapsed = start.elapsed();

    eprintln!("optional mod folders: {mods:?}");
    eprintln!(
        "mounted {} PBOs, {} files ({} distinct paths, {} overridden) in {elapsed:.2?}",
        report.pbos,
        report.files,
        vfs.len(),
        report.overridden
    );
    eprintln!("skipped encrypted EBOs: {}", report.encrypted.len());
    for (path, reason) in &report.failed {
        eprintln!("FAIL {}: {reason}", path.display());
    }
    assert!(report.failed.is_empty());
    assert!(
        report.pbos > 400,
        "expected the full install, got {} PBOs",
        report.pbos
    );

    let config = vfs.open(r"A3\Data_F\config.bin").unwrap();
    assert_eq!(&config[..4], b"\0raP", "config.bin is a rapified config");
    assert!(vfs.exists(r"a3\ui_f\config.bin"));
    assert!(vfs.is_dir(r"a3\data_f"));
    let top: Vec<_> = vfs.list_dir("").into_iter().map(|e| e.name).collect();
    assert!(
        top.iter().any(|name| name == "a3"),
        "top-level dirs: {top:?}"
    );
}
