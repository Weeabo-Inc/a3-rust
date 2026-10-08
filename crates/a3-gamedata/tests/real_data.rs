//! Loads the real game install. Skipped when `A3_ROOT` is unset.
//!
//! `cargo test -p a3-gamedata --release --test real_data -- --nocapture` prints load timings.

use a3_gamedata::{GameData, LoadOptions};

#[test]
fn loads_the_full_game() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let options = LoadOptions::new(root).with_optional_mods(true);
    let data = GameData::load(&options).unwrap();
    let r = &data.report;
    eprintln!(
        "PBOs {}, files {}, EBOs skipped {}, failed {}; addon configs {}, config errors {}, \
         config.cpp skipped {}, missing requirements {}, cycles {}, merge warnings {}, nodes {}",
        r.mount.pbos,
        data.vfs.len(),
        r.mount.encrypted.len(),
        r.mount.failed.len(),
        data.addons.len(),
        r.config_errors.len(),
        r.skipped_config_cpp.len(),
        r.missing_requirements.len(),
        r.cycles.len(),
        data.config.warnings().len(),
        data.config.node_count(),
    );
    eprintln!("timings: {:?}", r.timings);

    let soldier = data.config.root() >> "CfgVehicles" >> "B_Soldier_F";
    assert!(soldier.is_class());
    assert_eq!((&soldier >> "scope").number(), 2.0);
    assert!(
        (&soldier >> "displayName")
            .text()
            .starts_with("$STR_A3_CfgVehicles_B_Soldier_F")
    );
    assert!(data.vfs.exists(r"a3\characters_f\config.bin"));
    assert!(r.config_errors.is_empty(), "{:?}", r.config_errors);
    assert!(
        r.missing_requirements.is_empty(),
        "{:?}",
        r.missing_requirements
    );
    assert!(r.mount.failed.is_empty());
}
