//! Round-trips every rapified file in the game install. Skipped when `A3_ROOT` is unset.
//!
//! Run with `cargo test -p a3-config --release --test real_data -- --nocapture` to see stats.

mod common;

use std::collections::BTreeMap;

use a3_config::{
    AddonPatches, ConfigClass, ConfigTree, EntryKind, Value, is_rap, load_order, parse_text,
    read_rap, write_rap, write_text,
};

/// File extensions that may hold rapified config data inside PBOs.
const CANDIDATE_EXTENSIONS: &[&str] = &["bin", "rvmat", "bisurf", "sqm", "ext", "cfg", "fsm"];

#[derive(Default)]
struct Stats {
    files: usize,
    read_ok: usize,
    rap_round_trip: usize,
    byte_identical: usize,
    text_round_trip: usize,
}

#[derive(Default, Debug)]
struct Kinds {
    classes: usize,
    externals: usize,
    deletes: usize,
    appends: usize,
    strings: usize,
    floats: usize,
    ints: usize,
    int64s: usize,
    expressions: usize,
    arrays: usize,
    enums: usize,
}

fn count_value(v: &Value, k: &mut Kinds) {
    match v {
        Value::String(_) => k.strings += 1,
        Value::Float(_) => k.floats += 1,
        Value::Int(_) => k.ints += 1,
        Value::Int64(_) => k.int64s += 1,
        Value::Expression(_) => k.expressions += 1,
        Value::Array(items) => {
            k.arrays += 1;
            items.iter().for_each(|i| count_value(i, k));
        }
    }
}

fn count_kinds(class: &ConfigClass, k: &mut Kinds) {
    for e in &class.entries {
        match &e.kind {
            EntryKind::Class(c) => {
                k.classes += 1;
                count_kinds(c, k);
            }
            EntryKind::External => k.externals += 1,
            EntryKind::Delete => k.deletes += 1,
            EntryKind::Value(v) => count_value(v, k),
            EntryKind::ArrayAppend(items) => {
                k.appends += 1;
                items.iter().for_each(|i| count_value(i, k));
            }
        }
    }
}

fn extension(name: &str) -> String {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    if file.eq_ignore_ascii_case("config.bin") {
        return "config.bin".into();
    }
    file.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

#[test]
fn every_rapified_file_round_trips() {
    let Some(root) = common::game_root() else {
        return;
    };
    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut not_identical = Vec::new();
    let mut compressed = 0usize;
    let mut kinds = Kinds::default();

    for path in common::all_pbos(&root) {
        let pbo = match a3_pbo::Pbo::open(&path) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{}: PBO header: {e}", path.display()));
                continue;
            }
        };
        for entry in pbo.entries() {
            let ext = extension(entry.name());
            if ext != "config.bin" && !CANDIDATE_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let Ok(bytes) = pbo.read_entry(entry) else {
                compressed += 1;
                continue;
            };
            if !is_rap(&bytes) {
                continue;
            }
            let where_ = format!("{}:{}", path.display(), entry.name());
            let s = stats.entry(ext).or_default();
            s.files += 1;
            let config = match read_rap(&bytes) {
                Ok(c) => c,
                Err(e) => {
                    failures.push(format!("{where_}: read: {e}"));
                    continue;
                }
            };
            s.read_ok += 1;
            count_kinds(&config.root, &mut kinds);
            kinds.enums += config.enums.len();
            let written = write_rap(&config);
            match read_rap(&written) {
                Ok(again) if again == config => s.rap_round_trip += 1,
                Ok(_) => failures.push(format!("{where_}: re-read differs")),
                Err(e) => failures.push(format!("{where_}: re-read: {e}")),
            }
            let text = write_text(&config);
            match parse_text(&text) {
                Ok(again) if again == config => s.text_round_trip += 1,
                Ok(_) => failures.push(format!("{where_}: text re-parse differs")),
                Err(e) => failures.push(format!("{where_}: text re-parse: {e}")),
            }
            if written == bytes {
                s.byte_identical += 1;
            } else {
                not_identical.push(where_);
            }
        }
    }

    eprintln!("unreadable (compressed) candidate entries skipped: {compressed}");
    eprintln!("entry kinds: {kinds:?}");
    for (ext, s) in &stats {
        eprintln!(
            "{ext:>12}: {:>6} files, {:>6} read, {:>6} rap round-trip, {:>6} byte-identical, {:>6} text round-trip",
            s.files, s.read_ok, s.rap_round_trip, s.byte_identical, s.text_round_trip
        );
    }
    for f in not_identical.iter().take(10) {
        eprintln!("not byte-identical: {f}");
    }
    for f in failures.iter().take(50) {
        eprintln!("FAIL {f}");
    }
    assert!(stats.get("config.bin").is_some_and(|s| s.files > 0));
    assert!(failures.is_empty(), "{} failures", failures.len());
}

/// Loads every addon's config.bin in `requiredAddons` order into one tree, like the engine's
/// `configFile`, and spot-checks well-known entries.
#[test]
fn full_game_config_merges_and_resolves() {
    let Some(root) = common::game_root() else {
        return;
    };
    let start = std::time::Instant::now();
    let mut configs = Vec::new();
    let pbos = common::game_pbos(&root);
    for path in &pbos {
        let pbo = a3_pbo::Pbo::open(path).unwrap();
        // Every config.bin in a PBO is its own addon config, including those in subfolders.
        for entry in pbo
            .entries()
            .iter()
            .filter(|e| extension(e.name()) == "config.bin")
        {
            let bytes = pbo.read_entry(entry).unwrap();
            configs.push(read_rap(&bytes).unwrap());
        }
    }
    let read_time = start.elapsed();

    let patches: Vec<_> = configs.iter().map(AddonPatches::from_config).collect();
    let order = load_order(&patches);
    let mut tree = ConfigTree::new();
    for &i in &order.order {
        tree.merge(&configs[i]);
    }
    let merge_time = start.elapsed() - read_time;

    let soldier = tree.root() >> "CfgVehicles" >> "B_Soldier_F";
    let display_name = (&soldier >> "displayName").text();
    let bases: Vec<_> = soldier
        .bases()
        .iter()
        .map(|b| b.name().to_owned())
        .collect();

    // Every class in the big Cfg* roots should resolve its declared base.
    let mut classes = 0usize;
    let mut unresolved = Vec::new();
    for cfg in ["CfgVehicles", "CfgWeapons", "CfgAmmo", "CfgMagazines"] {
        for class in (tree.root() >> cfg).entries() {
            if !class.is_class() {
                continue;
            }
            classes += 1;
            if class.declared_base().is_some() && class.inherits_from().is_null() {
                unresolved.push(class.path_string());
            }
        }
    }

    eprintln!(
        "pbos: {}, addon configs: {}, read {:?}, merged {:?}, nodes {}, missing requirements {}, cycles {}, warnings {}",
        pbos.len(),
        configs.len(),
        read_time,
        merge_time,
        tree.node_count(),
        order.missing.len(),
        order.cycles.len(),
        tree.warnings().len()
    );
    for (i, name) in order.missing.iter().take(10) {
        eprintln!(
            "missing requirement: {:?} needs {name}",
            patches[*i].patches.first()
        );
    }
    for w in tree.warnings().iter().take(10) {
        eprintln!("warning: {w}");
    }
    eprintln!("B_Soldier_F displayName = {display_name:?}, bases = {bases:?}");
    eprintln!(
        "top-level classes checked: {classes}, unresolved bases: {}",
        unresolved.len()
    );
    for u in unresolved.iter().take(10) {
        eprintln!("unresolved base: {u}");
    }

    assert!(soldier.is_class());
    assert!(!display_name.is_empty());
    assert_eq!((&soldier >> "scope").number(), 2.0);
    assert!(bases.iter().any(|b| b.eq_ignore_ascii_case("CAManBase")));
    assert!(bases.iter().any(|b| b.eq_ignore_ascii_case("All")));
    assert!(
        (tree.root() >> "CfgPatches" >> "A3_Data_F").is_class(),
        "core addon present"
    );
    assert!(
        unresolved.is_empty(),
        "{} unresolved bases",
        unresolved.len()
    );
}
