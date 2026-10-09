//! Static command usage of shipped code: `.sqf`, `.fsm` and config-embedded SQF.

use std::path::Path;

use a3_config::{ConfigTree, parse_text, write_rap};
use a3_gamedata::{
    CommandUsage, GameData, LoadOptions, UsageSource, command_usage, engine_command_table,
    fsm_code, is_code_entry,
};
use a3_pbo::PboWriter;
use a3_sqf::Form;

const FSM: &str = r#"/*%FSM<COMPILE "scriptedFSM.cfg, test">*/
/*%FSM<HEAD>*/
/*
item0[] = {"Init",0,250,0,0,0,0,0.000000,"Init"};
*//*%FSM</HEAD>*/
class FSM
{
  fsmName = "test";
  class States
  {
    /*%FSM<STATE "Init">*/
    class Init
    {
      name = "Init";
      itemno = 0;
      init = /*%FSM<STATEINIT""">*/"_a = count _this;" \n
       "hint ""x"";"/*%FSM</STATEINIT""">*/;
      precondition = /*%FSM<STATEPRECONDITION""">*/""/*%FSM</STATEPRECONDITION""">*/;
      class Links
      {
        class Go
        {
          priority = 0.000000;
          to="End";
          condition=/*%FSM<CONDITION""">*/"time > 10"/*%FSM</CONDITION""">*/;
          action=/*%FSM<ACTION""">*/"// done" \n "sleep 1"/*%FSM</ACTION""">*/;
        };
      };
    };
  };
  initState="Init";
};
"#;

#[test]
fn fsm_code_extracts_state_and_link_scripts() {
    let code = fsm_code(FSM);
    assert_eq!(
        code,
        vec![
            "_a = count _this;\nhint \"x\";".to_owned(),
            String::new(),
            "time > 10".to_owned(),
            "// done\nsleep 1".to_owned(),
        ]
    );
}

#[test]
fn snippets_count_calls_including_nested_code() {
    let table = engine_command_table();
    let mut usage = CommandUsage::default();
    assert!(usage.add_snippet(
        "if (count _x > 0) then { hint str count _x }",
        &table,
        UsageSource::Config
    ));
    assert!(!usage.add_snippet("hint (", &table, UsageSource::Config));
    assert_eq!(usage.get("COUNT", Form::Unary), [0, 0, 2]);
    assert_eq!(usage.total("then", Form::Binary), 1);
    assert_eq!(usage.total("hint", Form::Unary), 1);
    assert_eq!(usage.compiled, [0, 0, 1]);
    assert_eq!(usage.failed, [0, 0, 1]);
}

#[test]
fn code_entries_are_handlers_and_action_fields() {
    for name in [
        "onLoad",
        "onButtonClick",
        "statement",
        "Condition",
        "init",
        "action",
    ] {
        assert!(is_code_entry(name), "{name}");
    }
    for name in ["onlyForPlayer", "on", "displayName", "volume", "model"] {
        assert!(!is_code_entry(name), "{name}");
    }
}

#[test]
fn config_scan_reads_handler_texts_once() {
    let config = parse_text(
        r#"
        class RscDisplayX { onLoad = "hint 'a'"; idd = 1; };
        class CfgVehicles {
            class Car { class EventHandlers { killed = "deleteVehicle (_this select 0)"; }; displayName = "hint"; };
            class Car2 { class EventHandlers { killed = "deleteVehicle (_this select 0)"; }; };
            class Box { class UserActions { class Open { statement = "player setDamage 1"; condition = "alive player"; }; }; };
        };
        "#,
    )
    .unwrap();
    let tree = ConfigTree::from_config(&config);
    let table = engine_command_table();
    let mut usage = CommandUsage::default();
    usage.scan_config(&tree, &table);
    assert_eq!(usage.get("hint", Form::Unary), [0, 0, 1]);
    assert_eq!(usage.get("deleteVehicle", Form::Unary), [0, 0, 1]);
    assert_eq!(usage.get("setDamage", Form::Binary), [0, 0, 1]);
    assert_eq!(usage.get("alive", Form::Unary), [0, 0, 1]);
    assert_eq!(usage.get("player", Form::Nular), [0, 0, 2]);
    assert_eq!(usage.compiled[UsageSource::Config as usize], 4);
}

fn write_pbo(path: &Path, prefix: &str, files: &[(&str, Vec<u8>)]) {
    let mut writer = PboWriter::new().property("prefix", prefix);
    for (name, bytes) in files {
        writer = writer.file(*name, bytes.clone());
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, writer.to_bytes()).unwrap();
}

#[test]
fn command_usage_scans_sqf_fsm_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = parse_text(
        r#"class CfgPatches { class T { requiredAddons[] = {}; }; };
           class RscX { onUnload = "systemChat 'bye'"; };"#,
    )
    .unwrap();
    write_pbo(
        &dir.path().join("Addons/t.pbo"),
        r"a3\t",
        &[
            ("config.bin", write_rap(&config)),
            (
                "fn_a.sqf",
                b"#define N 2\nsystemChat str N; _x = [] select 0;".to_vec(),
            ),
            ("broken.sqf", b"hint (".to_vec()),
            ("test.fsm", FSM.as_bytes().to_vec()),
        ],
    );
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let usage = command_usage(&data.vfs, &data.config, &engine_command_table());
    assert_eq!(usage.get("systemChat", Form::Unary), [1, 0, 1]);
    assert_eq!(usage.get("select", Form::Binary), [1, 0, 0]);
    assert_eq!(usage.get("sleep", Form::Unary), [0, 1, 0]);
    assert_eq!(usage.compiled, [1, 3, 1]);
    assert_eq!(usage.failed, [1, 0, 0]);
}
