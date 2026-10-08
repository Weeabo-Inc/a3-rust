//! Loads every stringtable of a real game install. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_stringtable::Localizer;
use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn every_stringtable_loads_and_known_keys_resolve() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));

    let (english, report) = Localizer::load_vfs(&vfs, "English");
    eprintln!(
        "{} stringtables, {} keys, {} duplicates, {} csv skipped",
        report.tables.len(),
        english.len(),
        report.duplicates.len(),
        report.skipped_csv.len()
    );
    for (path, error) in &report.failed {
        eprintln!("FAIL {path}: {error}");
    }
    assert!(report.failed.is_empty());
    assert!(report.tables.len() >= 50, "expected the full install");
    assert!(english.len() > 40_000);

    // Keys from the base game's language addon.
    assert_eq!(english.localize("STR_DISP_OK"), "OK");
    assert_eq!(english.localize("str_disp_cancel"), "Cancel");
    assert_eq!(
        english.config_text("$STR_A3_CfgMagazines_6Rnd_RedSignal_F0"),
        "6Rnd Signal Cylinder (Red)"
    );

    let (german, _) = Localizer::load_vfs(&vfs, "German");
    assert_eq!(german.len(), english.len());
    assert_eq!(german.localize("STR_DISP_CANCEL"), "Abbrechen");
    assert_eq!(
        german.localize("STR_A3_CfgMagazines_6Rnd_RedSignal_F0"),
        "6-Schuss-Signalzylinder (Rot)"
    );
}
