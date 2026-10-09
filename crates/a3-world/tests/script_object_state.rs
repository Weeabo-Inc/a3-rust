//! Unit and vehicle state commands and vehicle cargo against a synthetic config. The expectations
//! mirror the oracle runs of `tools/oracle/probes/98_object_state_vr.probes`.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; side = 1; };
    class Car: All { simulation = "carx"; };
    class B_MRAP_01_F: Car {
        scope = 2; side = 1; fuelCapacity = 50;
        hiddenSelections[] = {"Camo1", "Camo2", "Camo3"};
        hiddenSelectionsTextures[] = {"\A3\Soft_F\MRAP_01\Data\MRAP_01_base_CO.paa", "\A3\Soft_F\MRAP_01\Data\MRAP_01_adds_CO.paa"};
        class TransportItems { class _xx_FirstAidKit { name = "FirstAidKit"; count = 2; }; };
        class TransportMagazines { class _xx_mag { magazine = "30Rnd_65x39_caseless_mag"; count = 2; }; };
        class TransportWeapons { class _xx_mx { weapon = "arifle_MX_F"; count = 1; }; };
    };
    class B_Truck_01_fuel_F: B_MRAP_01_F { transportFuel = 3000; };
    class Thing: All { simulation = "thing"; };
    class Box_NATO_Ammo_F: Thing { scope = 2; };
    class B_AssaultPack_mcamo: All { scope = 2; isBackpack = 1; maximumLoad = 160; mass = 20; };
};
class CfgMagazines {
    class Default {};
    class 30Rnd_65x39_caseless_mag: Default { count = 30; mass = 10; };
};
class CfgWeapons {
    class Default { type = 0; };
    class ItemCore: Default { type = 131072; };
    class FirstAidKit: ItemCore { class ItemInfo { type = 401; mass = 8; }; };
    class ItemGPS: ItemCore { simulation = "ItemGPS"; class ItemInfo { mass = 8; }; };
    class arifle_MX_F: Default { type = 1; class WeaponSlotsInfo { mass = 65; }; };
};
"#;

fn vm() -> Vm<ScriptWorld> {
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    let mut registry = Registry::with_core();
    register_world_commands(&mut registry);
    let mut vm = Vm::with_registry(
        ScriptWorld::new(World::new(ClientId::SERVER), types),
        Rc::new(registry),
    );
    eval(
        &mut vm,
        r#"u = (createGroup west) createUnit ["B_Soldier_F", [0, 0, 0], [], 0, "NONE"];
           car = "B_MRAP_01_F" createVehicle [20, 0, 0];
           truck = "B_Truck_01_fuel_F" createVehicle [40, 0, 0];
           box = "Box_NATO_Ammo_F" createVehicle [60, 0, 0];"#,
    );
    vm
}

fn eval(vm: &mut Vm<ScriptWorld>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

fn sqf(vm: &mut Vm<ScriptWorld>, code: &str) -> String {
    match eval(vm, code) {
        Value::String(s) => s.to_string(),
        other => other.to_sqf_string(),
    }
}

#[test]
fn captive_makes_a_unit_civilian() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "[captive u, side u]"), "[false,WEST]");
    eval(&mut vm, "u setCaptive true");
    assert_eq!(
        sqf(&mut vm, "[captive u, side u, side group u, captiveNum u]"),
        "[true,CIV,WEST,1]"
    );
    eval(&mut vm, "u setCaptive 5");
    assert_eq!(sqf(&mut vm, "[captive u, captiveNum u]"), "[true,5]");
    eval(&mut vm, "u setCaptive false; car setCaptive true");
    assert_eq!(
        sqf(&mut vm, "[captive u, side u, captive car, captive objNull]"),
        "[false,WEST,false,false]"
    );
}

#[test]
fn unit_pos_ai_features_rank_skill() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "unitPos u"), "Auto");
    eval(&mut vm, "u setUnitPos 'down'");
    assert_eq!(sqf(&mut vm, "unitPos u"), "Down");
    // An unknown stance runs on and keeps the stance (oracle: the original logs the enum error).
    eval(&mut vm, "u setUnitPos 'bad'");
    assert_eq!(sqf(&mut vm, "unitPos u"), "Down");
    eval(&mut vm, "u disableAI 'MOVE'; u disableAI 'path'");
    assert_eq!(
        sqf(
            &mut vm,
            "[u checkAIFeature 'MOVE', u checkAIFeature 'PATH', u checkAIFeature 'TARGET']"
        ),
        "[false,false,true]"
    );
    eval(&mut vm, "u disableAI 'ALL'; u enableAI 'MOVE'");
    assert_eq!(
        sqf(
            &mut vm,
            "[u checkAIFeature 'MOVE', u checkAIFeature 'TARGET']"
        ),
        "[true,false]"
    );
    // An unknown feature name runs on too (oracle: `disableAI "NOSUCH"`).
    eval(&mut vm, "u disableAI 'NOSUCH'");
    assert_eq!(sqf(&mut vm, "[rank u, rankId u]"), r#"["PRIVATE",0]"#);
    eval(&mut vm, "u setRank 'captain'");
    assert_eq!(sqf(&mut vm, "[rank u, rankId u]"), r#"["CAPTAIN",4]"#);
    // An unknown rank sets PRIVATE and runs on (oracle: `setRank "bogus"`).
    eval(&mut vm, "u setUnitRank 'bogus'");
    assert_eq!(sqf(&mut vm, "[rank u, rank objNull]"), r#"["PRIVATE",""]"#);
    assert_eq!(sqf(&mut vm, "skill u"), "0.5");
    eval(
        &mut vm,
        "u setSkill 0.3; u setSkill ['aimingAccuracy', 0.9]",
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[skill u, u skill 'aimingAccuracy', u skill 'spotTime']"
        ),
        "[0.3,0.9,0.3]"
    );
    eval(&mut vm, "u setSkill 2");
    assert_eq!(sqf(&mut vm, "[skill u, u skill 'aimingAccuracy']"), "[1,1]");
}

#[test]
fn fuel_and_supply_cargo() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "fuel car"), "1");
    eval(&mut vm, "car setFuel 0.3");
    assert_eq!(sqf(&mut vm, "fuel car"), "0.3");
    eval(&mut vm, "car setFuel 2");
    assert_eq!(sqf(&mut vm, "fuel car"), "1");
    eval(&mut vm, "car setFuel -1; u setFuel 0.5");
    assert_eq!(sqf(&mut vm, "[fuel car, fuel u, fuel objNull]"), "[0,1,0]");
    assert_eq!(
        sqf(
            &mut vm,
            "[getFuelCargo car, getFuelCargo truck, getAmmoCargo truck, getFuelCargo u]"
        ),
        "[-1,1,-1,-1]"
    );
    eval(&mut vm, "truck setFuelCargo 0.25; truck setAmmoCargo 0.5");
    assert_eq!(
        sqf(&mut vm, "[getFuelCargo truck, getAmmoCargo truck]"),
        "[0.25,-1]"
    );
}

#[test]
fn textures_and_materials() {
    let mut vm = vm();
    assert_eq!(
        sqf(&mut vm, "getObjectTextures car"),
        r#"["a3\soft_f\mrap_01\data\mrap_01_base_co.paa","a3\soft_f\mrap_01\data\mrap_01_adds_co.paa",""]"#
    );
    eval(
        &mut vm,
        "car setObjectTexture [0, '#(argb,8,8,3)color(1,0,0,1)']; car setObjectTexture [5, '#(rgb,8,8,3)color(0,0,0,1)']; car setObjectTextureGlobal ['camo2', 'missing.paa']",
    );
    assert_eq!(
        sqf(&mut vm, "getObjectTextures car select 0"),
        "#(argb,8,8,3)color(1,0,0,1)"
    );
    assert_eq!(sqf(&mut vm, "count getObjectTextures car"), "3");
    assert_eq!(sqf(&mut vm, "getObjectMaterials car"), r#"["","",""]"#);
    eval(
        &mut vm,
        "car setObjectMaterial [0, 'a3\\data_f\\default.rvmat']",
    );
    assert_eq!(
        sqf(&mut vm, "getObjectMaterials car select 0"),
        r"a3\data_f\default.rvmat"
    );
}

#[test]
fn locks_and_engine() {
    let mut vm = vm();
    assert_eq!(sqf(&mut vm, "locked car"), "1");
    let mut seen = Vec::new();
    for code in [
        "car lock true",
        "car lock 3",
        "car lock 0",
        "car lock false",
        "car setVehicleLock 'LOCKEDPLAYER'",
        "car setVehicleLock 'unlocked'",
        "car lock 7",
    ] {
        eval(&mut vm, code);
        seen.push(sqf(&mut vm, "locked car"));
    }
    assert_eq!(seen, ["2", "3", "0", "1", "3", "0", "7"]);
    assert_eq!(sqf(&mut vm, "[locked objNull, locked u]"), "[-1,-1]");
    eval(
        &mut vm,
        "car lockDriver true; car lockCargo true; car lockCargo [0, false]",
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[lockedDriver car, car lockedCargo 0, car lockedCargo 1]"
        ),
        "[true,false,true]"
    );
    eval(&mut vm, "car engineOn true; car setFuel 0");
    assert_eq!(sqf(&mut vm, "isEngineOn car"), "true");
}

#[test]
fn vehicle_cargo() {
    let mut vm = vm();
    assert_eq!(
        sqf(
            &mut vm,
            "[itemCargo car, magazineCargo car, weaponCargo car, backpackCargo car]"
        ),
        r#"[["FirstAidKit","FirstAidKit"],["30Rnd_65x39_caseless_mag","30Rnd_65x39_caseless_mag"],["arifle_MX_F"],[]]"#
    );
    eval(
        &mut vm,
        "clearItemCargo car; clearMagazineCargoGlobal car; clearWeaponCargo car; clearBackpackCargoGlobal car",
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[itemCargo car, magazineCargo car, weaponCargo car]"
        ),
        "[[],[],[]]"
    );
    eval(
        &mut vm,
        "car addItemCargo ['FirstAidKit', 2]; car addItemCargoGlobal ['ItemGPS', 1]; car addItemCargo ['FirstAidKit', 1.6]; car addItemCargo ['FirstAidKit', 0]; car addMagazineCargo ['30Rnd_65x39_caseless_mag', 3]; car addWeaponCargoGlobal ['arifle_MX_F', 1]; car addBackpackCargo ['B_AssaultPack_mcamo', 1]",
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[itemCargo car, getItemCargo car, getMagazineCargo car, backpackCargo car]"
        ),
        r#"[["FirstAidKit","FirstAidKit","ItemGPS","FirstAidKit","FirstAidKit"],[["FirstAidKit","ItemGPS"],[4,1]],[["30Rnd_65x39_caseless_mag"],[3]],["B_AssaultPack_mcamo"]]"#
    );
    // addItemCargo routes magazines and weapons; addWeaponCargo takes any CfgWeapons class.
    eval(
        &mut vm,
        "clearItemCargo box; box addItemCargo ['30Rnd_65x39_caseless_mag', 1]; box addItemCargo ['arifle_MX_F', 1]; box addMagazineCargo ['FirstAidKit', 1]; box addWeaponCargo ['FirstAidKit', 1]; box addItemCargo ['B_AssaultPack_mcamo', 1]",
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[itemCargo box, magazineCargo box, weaponCargo box, backpackCargo box]"
        ),
        r#"[[],["30Rnd_65x39_caseless_mag"],["arifle_MX_F","FirstAidKit"],[]]"#
    );
    // A one-element array adds nothing and runs on (oracle: `addItemCargo ["FirstAidKit"]`).
    eval(&mut vm, "box addItemCargo ['FirstAidKit']");
    assert_eq!(sqf(&mut vm, "count itemCargo box"), "0");
    // `load` of a box is its cargo mass over the type's `maximumLoad` (oracle: 100 toolkits in an
    // ammo box give 8).
    eval(
        &mut vm,
        "clearItemCargo box; clearMagazineCargo box; clearWeaponCargo box; clearBackpackCargo box; box addItemCargo ['FirstAidKit', 125]",
    );
    assert_eq!(
        sqf(&mut vm, "[count itemCargo box, load box, loadAbs box]"),
        "[125,1,1000]"
    );
    // Units keep their gear in containers; clearing cargo does not touch it.
    assert_eq!(
        sqf(
            &mut vm,
            "[itemCargo objNull, getItemCargo objNull, getItemCargo u]"
        ),
        "[[],[[],[]],[[],[]]]"
    );
}
