//! Config commands, headless game state and the function-library boot on a synthetic install.

use std::path::Path;

use a3_config::{parse_text, write_rap};
use a3_gamedata::{
    GameData, LoadOptions, compile_all, engine_command_table, init_functions, script_vm,
    unimplemented_usage,
};
use a3_pbo::PboWriter;
use a3_sqf::{Namespace, Sym, Value};

fn write_pbo(path: &Path, prefix: &str, files: &[(&str, Vec<u8>)]) {
    let mut writer = PboWriter::new().property("prefix", prefix);
    for (name, bytes) in files {
        writer = writer.file(*name, bytes.clone());
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, writer.to_bytes()).unwrap();
}

const CONFIG: &str = r#"
class CfgPatches { class Test { requiredAddons[] = {}; }; };
class CfgX {
    num = 1.5;
    text = "hello";
    list[] = {1, "two", {3}};
    class Base { v = 3; };
    class Derived: Base { w = 4; };
};
class CfgFunctions {
    init = "\x\test\init.sqf";
    class TAG {
        class Main {
            file = "\x\test\functions";
            class double {};
            class broken {};
        };
    };
    class Other {
        tag = "OTH";
        class Misc {
            class hello { file = "\x\test\hello.sqf"; };
        };
    };
};
"#;

/// A miniature initFunctions.sqf: walks CfgFunctions and compiles every function into
/// uiNamespace with compileScript, as the real one does.
const INIT: &str = r#"
private _cfg = configFile >> "CfgFunctions";
for "_t" from 0 to (count _cfg - 1) do {
    private _tagCfg = _cfg select _t;
    if (isClass _tagCfg) then {
        private _tag = getText (_tagCfg >> "tag");
        if (_tag == "") then { _tag = configName _tagCfg };
        for "_c" from 0 to (count _tagCfg - 1) do {
            private _cat = _tagCfg select _c;
            for "_f" from 0 to (count _cat - 1) do {
                private _fn = _cat select _f;
                if (isClass _fn) then {
                    private _path = getText (_fn >> "file");
                    if (_path == "") then {
                        _path = getText (_cat >> "file") + "\fn_" + configName _fn + ".sqf";
                    };
                    private _var = _tag + "_fnc_" + configName _fn;
                    uiNamespace setVariable [_var, compileScript [_path, true]];
                    missionNamespace setVariable [_var, uiNamespace getVariable _var];
                };
            };
        };
    };
};
uiNamespace setVariable ["bis_fnc_init", true];
"#;

fn game(root: &Path) {
    let config = write_rap(&parse_text(CONFIG).unwrap());
    write_pbo(
        &root.join("Addons/test.pbo"),
        r"x\test",
        &[
            ("config.bin", config),
            ("init.sqf", INIT.as_bytes().to_vec()),
            (r"functions\fn_double.sqf", b"_this * 2".to_vec()),
            (r"functions\fn_broken.sqf", b"x = (;".to_vec()),
            ("hello.sqf", b"\"hello \" + _this".to_vec()),
        ],
    );
}

fn eval(vm: &mut a3_sqf::Vm<a3_gamedata::VfsHost>, src: &str) -> String {
    match vm.eval(src) {
        Ok(v) => v.to_sqf_string_with(&|h| a3_sqf::Host::format_handle(&vm.host, h)),
        Err(e) => panic!("{src}\n{}", e.report),
    }
}

#[test]
fn config_commands_read_the_merged_config() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = script_vm(&data);

    assert_eq!(
        eval(&mut vm, r#"getNumber (configFile >> "CfgX" >> "num")"#),
        "1.5"
    );
    assert_eq!(
        eval(&mut vm, r#"getText (configFile >> "cfgx" >> "TEXT")"#),
        "\"hello\""
    );
    assert_eq!(
        eval(&mut vm, r#"getArray (configFile >> "CfgX" >> "list")"#),
        "[1,\"two\",[3]]"
    );
    assert_eq!(
        eval(&mut vm, r#"isClass (configFile >> "CfgX" >> "Base")"#),
        "true"
    );
    assert_eq!(
        eval(&mut vm, r#"isNumber (configFile >> "CfgX" >> "num")"#),
        "true"
    );
    assert_eq!(
        eval(&mut vm, r#"isText (configFile >> "CfgX" >> "num")"#),
        "false"
    );
    assert_eq!(
        eval(&mut vm, r#"configName (configFile >> "cfgx")"#),
        "\"CfgX\""
    );
    assert_eq!(eval(&mut vm, r#"count (configFile >> "CfgX")"#), "5");
    assert_eq!(
        eval(&mut vm, r#"configName ((configFile >> "CfgX") select 1)"#),
        "\"text\""
    );
    assert_eq!(
        eval(
            &mut vm,
            r#"getNumber (configFile >> "CfgX" >> "Derived" >> "v")"#
        ),
        "3",
        "inherited entry"
    );
    assert_eq!(
        eval(
            &mut vm,
            r#"configName inheritsFrom (configFile >> "CfgX" >> "Derived")"#
        ),
        "\"Base\""
    );
    assert_eq!(
        eval(&mut vm, r#"isNull (configFile >> "Nope" >> "x")"#),
        "true"
    );
    assert_eq!(eval(&mut vm, r#"getNumber (configFile >> "Nope")"#), "0");
    assert_eq!(
        eval(
            &mut vm,
            r#"(configFile >> "CfgX") isEqualTo (configFile / "cfgx")"#
        ),
        "true"
    );
    assert_eq!(
        eval(
            &mut vm,
            r#"count configHierarchy (configFile >> "CfgX" >> "Base")"#
        ),
        "3"
    );
    assert_eq!(
        eval(&mut vm, r#"str (configFile >> "CfgX" >> "Base")"#),
        "\"bin\\config.bin/CfgX/Base\""
    );
    assert_eq!(eval(&mut vm, "isNull (missionConfigFile >> \"x\")"), "true");
}

#[test]
fn headless_state_is_a_retail_main_menu() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = script_vm(&data);
    assert_eq!(
        eval(
            &mut vm,
            "[cheatsEnabled, is3DEN, isServer, isDedicated, isMultiplayer]"
        ),
        "[false,false,true,false,false]"
    );
    assert_eq!(eval(&mut vm, "isNull findDisplay 46"), "true");
}

/// Mission start-up (`BIS_fnc_startLoadingScreen`, `BIS_fnc_progressLoadingScreen`) drives the
/// loading screen; headless there is none to show, and the commands return nothing.
#[test]
fn loading_screen_commands_do_nothing_headless() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = script_vm(&data);
    assert_eq!(
        eval(
            &mut vm,
            "startLoadingScreen [\"\"]; startLoadingScreen [\"x\", \"RscDisplayLoadMission\"]; \
             progressLoadingScreen 0.5; endLoadingScreen; 1"
        ),
        "1"
    );
}

/// `disableSerialization` (handler 0x2d2bf0) clears the running script's "serializable" flag, so
/// a saved game skips it; headless there are no saved games, and only the return (Nothing) and
/// the absence of an error show. The function library calls it before touching displays.
#[test]
fn disable_serialization_returns_nothing() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = script_vm(&data);
    assert_eq!(eval(&mut vm, "isNil {disableSerialization}"), "true");
    assert_eq!(eval(&mut vm, "disableSerialization; 1 + 1"), "2");
}

#[test]
fn init_functions_compiles_every_declared_function() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = script_vm(&data);

    let report = init_functions(&mut vm);

    assert_eq!(report.init_script, r"\x\test\init.sqf");
    assert_eq!(report.declared, 3);
    assert_eq!(report.compiled, 3);
    assert!(report.missing.is_empty());
    assert!(report.finished);
    // fn_broken.sqf does not compile: reported, and the function is empty code.
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert!(
        report.errors[0].contains("fn_broken.sqf"),
        "{}",
        report.errors[0]
    );
    assert_eq!(eval(&mut vm, "21 call TAG_fnc_double"), "42");
    assert_eq!(
        eval(&mut vm, "\"there\" call OTH_fnc_hello"),
        "\"hello there\""
    );
    assert!(matches!(
        vm.namespace(Namespace::Ui).get(Sym::new("tag_fnc_double")),
        Some(Value::Code(c)) if c.is_final()
    ));
    assert!(unimplemented_usage(&vm, Namespace::Ui).is_empty());
}

#[test]
fn compile_all_reports_statistics() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let stats = compile_all(&data.vfs, &engine_command_table());
    assert_eq!(stats.ok, 3);
    assert_eq!(stats.failed(), 1);
    assert!(stats.failures.contains_key("Missing )") || stats.failures.len() == 1);
}
