//! SQF config commands on the real merged config. Skipped when `A3_ROOT` is unset.

use std::rc::Rc;

use a3_gamedata::{GameData, LoadOptions, VfsHost, register_config_commands};
use a3_sqf::{Registry, Vm};

#[test]
fn reads_cfg_vehicles_from_sqf() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root)).unwrap();
    let mut reg = Registry::with_core();
    register_config_commands(&mut reg);
    let mut vm = Vm::with_registry(VfsHost::for_game(&game), Rc::new(reg));
    let mut eval = |src: &str| {
        let v = vm
            .eval(src)
            .unwrap_or_else(|e| panic!("{src}: {}", e.report));
        v.to_sqf_string()
    };
    let soldier = "(configFile >> \"CfgVehicles\" >> \"B_Soldier_F\")";
    assert_eq!(eval(&format!("isClass {soldier}")), "true");
    assert_eq!(eval(&format!("getNumber ({soldier} >> \"scope\")")), "2");
    assert_eq!(eval(&format!("getNumber ({soldier} >> \"side\")")), "1");
    assert_eq!(
        eval(&format!("getText ({soldier} >> \"vehicleClass\")")),
        "\"Men\""
    );
    let parents = eval(&format!(
        "private _p = []; private _c = {soldier}; while {{ !isNull _c }} do {{ _p pushBack configName _c; _c = inheritsFrom _c }}; _p"
    ));
    assert!(
        parents.contains("\"CAManBase\"") && parents.ends_with("\"All\"]"),
        "{parents}"
    );
    let public = eval(
        "count (\"getNumber (_x >> 'scope') == 2\" configClasses (configFile >> \"CfgVehicles\"))",
    );
    let n: f32 = public.parse().unwrap();
    eprintln!("public CfgVehicles classes: {n}");
    assert!(n > 1000.0, "{n}");
    let functions = eval("count (\"true\" configClasses (configFile >> \"CfgFunctions\"))");
    eprintln!("CfgFunctions tags: {functions}");
}
