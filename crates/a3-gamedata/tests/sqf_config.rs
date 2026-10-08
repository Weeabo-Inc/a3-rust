//! SQF config commands over a synthetic config.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_gamedata::{ConfigRoot, Localizer, SqfConfigs, VfsHost, register_config_commands};
use a3_sqf::{Registry, Vm};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; side = 7; };
    class Car: All {
        scope = 2;
        displayName = "$STR_Car";
        maxSpeed = 100;
        expr = "(2 + 3) * 2";
        flag = "true";
        threat[] = {1, 0.5, "x", {2, 3}};
        class Turrets { class Main { gun = "cannon"; }; };
    };
    class Truck: Car { maxSpeed = 80; transportSoldier = 12; };
    class Bike: All { scope = 1; transportSoldier = 0; };
};
class RscText { idc = -1; };
"#;

struct Strings;

impl Localizer for Strings {
    fn localize(&self, key: &str) -> Option<String> {
        key.eq_ignore_ascii_case("STR_Car")
            .then(|| "Car".to_owned())
    }
}

fn vm() -> Vm<VfsHost> {
    let tree = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
    let mut host = VfsHost {
        configs: SqfConfigs::new(Arc::new(tree)),
        localizer: Some(Arc::new(Strings)),
        ..VfsHost::default()
    };
    let mission =
        parse_text("respawnDelay = 15; author = \"me\"; class Header { gameType = \"Coop\"; };")
            .unwrap();
    let mut mtree = ConfigTree::with_root_name("description.ext");
    mtree.merge(&mission);
    host.configs.set(ConfigRoot::Mission, Arc::new(mtree));
    let mut reg = Registry::with_core();
    register_config_commands(&mut reg);
    Vm::with_registry(host, Rc::new(reg))
}

fn s(src: &str) -> String {
    let mut vm = vm();
    match vm.eval(src) {
        Ok(v) => vm.host.format_value(&v),
        Err(e) => panic!("{src}\n{}", e.report),
    }
}

trait Format {
    fn format_value(&self, v: &a3_sqf::Value) -> String;
}

impl Format for VfsHost {
    fn format_value(&self, v: &a3_sqf::Value) -> String {
        use a3_sqf::Host;
        v.to_sqf_string_with(&|h| self.format_handle(h))
    }
}

const CAR: &str = "(configFile >> \"CfgVehicles\" >> \"Car\")";

#[test]
fn paths_and_str() {
    assert_eq!(
        s(&format!("str {CAR}")),
        "\"bin\\config.bin/CfgVehicles/Car\""
    );
    assert_eq!(
        s("str (configFile / \"CfgVehicles\")"),
        "\"bin\\config.bin/CfgVehicles\""
    );
    assert_eq!(s("isNull (configFile >> \"Nope\")"), "true");
    assert_eq!(
        s(&format!(
            "{CAR} == (configFile >> \"cfgvehicles\" >> \"CAR\")"
        )),
        "true"
    );
    assert_eq!(s(&format!("configName {CAR}")), "\"Car\"");
}

#[test]
fn values() {
    assert_eq!(s(&format!("getNumber ({CAR} >> \"maxSpeed\")")), "100");
    assert_eq!(s(&format!("getNumber ({CAR} >> \"missing\")")), "0");
    assert_eq!(s(&format!("getNumber ({CAR} >> \"expr\")")), "10");
    assert_eq!(s(&format!("getNumber ({CAR} >> \"flag\")")), "1");
    assert_eq!(s(&format!("getText ({CAR} >> \"displayName\")")), "\"Car\"");
    assert_eq!(
        s(&format!("getTextRaw ({CAR} >> \"displayName\")")),
        "\"$STR_Car\""
    );
    assert_eq!(
        s(&format!("getArray ({CAR} >> \"threat\")")),
        "[1,0.5,\"x\",[2,3]]"
    );
    assert_eq!(
        s(&format!(
            "getNumber ({CAR} >> \"Turrets\" >> \"Main\" >> \"gun\")"
        )),
        "0"
    );
}

#[test]
fn kinds() {
    assert_eq!(
        s(&format!(
            "[isClass {CAR}, isNumber ({CAR} >> \"scope\"), isText ({CAR} >> \"displayName\"), isArray ({CAR} >> \"threat\")]"
        )),
        "[true,true,true,true]"
    );
    assert_eq!(s(&format!("isClass ({CAR} >> \"scope\")")), "false");
}

#[test]
fn inheritance() {
    let truck = "(configFile >> \"CfgVehicles\" >> \"Truck\")";
    assert_eq!(s(&format!("getNumber ({truck} >> \"scope\")")), "2");
    assert_eq!(
        s(&format!("str inheritsFrom {truck}")),
        "\"bin\\config.bin/CfgVehicles/Car\""
    );
    assert_eq!(
        s(&format!(
            "configHierarchy {truck} apply {{ configName _x }}"
        )),
        "[\"\",\"CfgVehicles\",\"Truck\"]"
    );
    assert_eq!(
        s(&format!("str ({truck} >> \"Turrets\" >> \"Main\")")),
        "\"bin\\config.bin/CfgVehicles/Truck/Turrets/Main\""
    );
}

#[test]
fn count_and_select() {
    assert_eq!(s("count (configFile >> \"CfgVehicles\")"), "4");
    assert_eq!(
        s("configName ((configFile >> \"CfgVehicles\") select 1)"),
        "\"Car\""
    );
    assert_eq!(
        s("isNull ((configFile >> \"CfgVehicles\") select 9)"),
        "true"
    );
}

#[test]
fn config_classes_and_properties() {
    assert_eq!(
        s("(\"true\" configClasses (configFile >> \"CfgVehicles\")) apply { configName _x }"),
        "[\"All\",\"Car\",\"Truck\",\"Bike\"]"
    );
    assert_eq!(
        s(
            "(\"getNumber (_x >> 'scope') > 1\" configClasses (configFile >> \"CfgVehicles\")) apply { configName _x }"
        ),
        "[\"Car\",\"Truck\"]"
    );
    assert_eq!(
        s(
            "(configProperties [configFile >> \"CfgVehicles\" >> \"Truck\", \"isNumber _x\"]) apply { configName _x }"
        ),
        "[\"maxSpeed\",\"transportSoldier\",\"scope\",\"side\"]"
    );
    assert_eq!(
        s(
            "(configProperties [configFile >> \"CfgVehicles\" >> \"Truck\", \"true\", false]) apply { configName _x }"
        ),
        "[\"maxSpeed\",\"transportSoldier\"]"
    );
}

#[test]
fn mission_config() {
    assert_eq!(s("getMissionConfigValue \"respawnDelay\""), "15");
    assert_eq!(s("getMissionConfigValue [\"missing\", 7]"), "7");
    assert_eq!(s("isNil { getMissionConfigValue \"Header\" }"), "true");
    assert_eq!(
        s("getText (getMissionConfig \"Header\" >> \"gameType\")"),
        "\"Coop\""
    );
    assert_eq!(s("getText (missionConfigFile >> \"author\")"), "\"me\"");
}

#[test]
fn configs_are_hash_map_keys() {
    assert_eq!(
        s(&format!(
            "_m = createHashMap; _m set [{CAR}, 1]; _m get (configFile >> \"CfgVehicles\" >> \"Car\")"
        )),
        "1"
    );
}
