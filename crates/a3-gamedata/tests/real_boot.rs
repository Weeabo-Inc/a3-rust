//! Runs the real function-library initialisation headless. Skipped when `A3_ROOT` is unset.
//!
//! `cargo test -p a3-gamedata --release --test real_boot -- --nocapture` prints the report.

use std::collections::BTreeMap;

use a3_gamedata::{GameData, LoadOptions, init_functions, script_vm, unimplemented_usage};
use a3_sqf::Namespace;

#[test]
fn init_functions_compiles_the_function_library() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    let mut vm = script_vm(&data);

    let report = init_functions(&mut vm);

    eprintln!(
        "{}: {} of {} CfgFunctions compiled into uiNamespace in {:.2?}; bis_fnc_init = {}",
        report.init_script, report.compiled, report.declared, report.elapsed, report.finished
    );
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    for error in &report.errors {
        let line = error
            .lines()
            .find(|l| l.trim_start().starts_with("Error "))
            .unwrap_or(error.as_str())
            .trim()
            .to_owned();
        *categories.entry(line).or_default() += 1;
    }
    eprintln!("{} script errors", report.errors.len());
    for (message, count) in &categories {
        eprintln!("    {count:5}  {message}");
    }
    for error in &report.errors {
        eprintln!("{error}");
    }
    let usage = unimplemented_usage(&vm, Namespace::Ui);
    eprintln!(
        "{} unimplemented commands used by the compiled functions (static uses):",
        usage.len()
    );
    for (name, count) in usage.iter().take(40) {
        eprintln!("    {count:5}  {name}");
    }
    for name in [
        "bis_fnc_timetostring",
        "bis_fnc_functionmeta",
        "bis_fnc_log",
    ] {
        let ui = vm
            .namespace(Namespace::Ui)
            .get(a3_sqf::Sym::new(name))
            .is_some();
        let mission = vm
            .namespace(Namespace::Mission)
            .get(a3_sqf::Sym::new(name))
            .is_some();
        eprintln!("    {name}: uiNamespace {ui}, missionNamespace {mission}");
    }

    assert!(
        report.finished,
        "initFunctions did not finish: {:#?}",
        report.errors.first()
    );
    eprintln!("missing: {:?}", report.missing);
    assert!(report.missing.is_empty(), "{:?}", report.missing);
}
