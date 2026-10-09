//! Unit inventory commands against a synthetic config. The expectations mirror the oracle runs
//! of `tools/oracle/probes/97_inventory_vr.probes` on the original server.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Registry, Value, Vm};
use a3_world::script::{ScriptWorld, register_world_commands};
use a3_world::{ClientId, TypeBank, World};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; side = 3; };
    class Man: All { simulation = "soldier"; maximumLoad = 1000; };
    class Supply40 { maximumLoad = 40; };
    class Supply140 { maximumLoad = 140; };
    class B_Soldier_F: Man {
        scope = 2; side = 1; modelSides[] = {3, 1};
        uniformClass = "U_B_CombatUniform_mcam";
        weapons[] = {"arifle_MX_ACO_pointer_F", "hgun_P07_F", "Throw", "Put"};
        magazines[] = {"30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag",
            "30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag",
            "30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag", "30Rnd_65x39_caseless_mag",
            "30Rnd_65x39_caseless_mag", "16Rnd_9x21_Mag", "16Rnd_9x21_Mag", "16Rnd_9x21_Mag",
            "SmokeShell", "SmokeShellGreen", "Chemlight_green", "Chemlight_green", "HandGrenade", "HandGrenade"};
        items[] = {"FirstAidKit"};
        linkedItems[] = {"V_PlateCarrier1_rgr", "H_HelmetB", "ItemMap", "ItemCompass", "ItemWatch", "ItemRadio", "NVGoggles"};
        backpack = "";
    };
    class O_Soldier_F: B_Soldier_F { side = 0; modelSides[] = {3, 0}; };
    class B_AssaultPack_mcamo: All { scope = 2; isBackpack = 1; maximumLoad = 160; mass = 20; };
    class B_AssaultPack_mcamo_Ammo: B_AssaultPack_mcamo {
        class TransportItems { class _xx_FirstAidKit { name = "FirstAidKit"; count = 2; }; };
        class TransportMagazines { class _xx_mag { magazine = "30Rnd_65x39_caseless_mag"; count = 3; }; };
    };
    class Car: All { simulation = "carx"; };
    class C_Offroad_01_F: Car { scope = 2; };
};
class CfgMagazines {
    class Default {};
    class CA_Magazine: Default { count = 30; mass = 8; };
    class 30Rnd_65x39_caseless_mag: CA_Magazine { count = 30; mass = 10; };
    class 100Rnd_65x39_caseless_mag: CA_Magazine { count = 100; mass = 25; };
    class 16Rnd_9x21_Mag: CA_Magazine { count = 16; mass = 6; };
    class HandGrenade: CA_Magazine { count = 1; mass = 10; };
    class SmokeShell: HandGrenade { mass = 4; };
    class SmokeShellGreen: SmokeShell {};
    class Chemlight_green: SmokeShell { mass = 2; };
};
class CfgMagazineWells { class MX_65x39 { BI_Magazines[] = {"100Rnd_65x39_caseless_mag"}; }; };
class CfgGlasses { class None {}; class G_Shades_Black { mass = 2; }; };
class CfgWeapons {
    class Default { type = 0; };
    class Throw: Default {};
    class Put: Default {};
    class ItemCore: Default { type = 131072; };
    class ItemMap: ItemCore { simulation = "ItemMap"; class ItemInfo { mass = 2; }; };
    class ItemCompass: ItemCore { simulation = "ItemCompass"; class ItemInfo { mass = 2; }; };
    class ItemWatch: ItemCore { simulation = "ItemWatch"; class ItemInfo { mass = 2; }; };
    class ItemRadio: ItemCore { simulation = "ItemRadio"; class ItemInfo { mass = 8; }; };
    class ItemGPS: ItemCore { simulation = "ItemGPS"; class ItemInfo { mass = 8; }; };
    class Binocular: Default { type = 4096; simulation = "Binocular"; class WeaponSlotsInfo { mass = 10; }; };
    class NVGoggles: Binocular { simulation = "NVGoggles"; class ItemInfo { type = 616; mass = 20; }; };
    class FirstAidKit: ItemCore { class ItemInfo { type = 401; mass = 8; }; };
    class ToolKit: ItemCore { class ItemInfo { type = 620; mass = 80; allowedSlots[] = {901}; }; };
    class H_HelmetB: ItemCore { class ItemInfo { type = 605; mass = 30; }; };
    class H_Cap_red: ItemCore { class ItemInfo { type = 605; mass = 4; }; };
    class V_PlateCarrier1_rgr: ItemCore { class ItemInfo { type = 701; mass = 80; containerClass = "Supply140"; }; };
    class V_Rangemaster_belt: ItemCore { class ItemInfo { type = 701; mass = 10; containerClass = "Supply40"; }; };
    class U_B_CombatUniform_mcam: ItemCore { class ItemInfo { type = 801; mass = 40; containerClass = "Supply40"; uniformClass = "B_Soldier_F"; }; };
    class U_O_CombatUniform_ocamo: ItemCore { class ItemInfo { type = 801; mass = 40; containerClass = "Supply40"; uniformClass = "O_Soldier_F"; }; };
    class optic_Aco: ItemCore { class ItemInfo { type = 201; mass = 4; }; };
    class optic_Hamr: ItemCore { class ItemInfo { type = 201; mass = 6; }; };
    class acc_pointer_IR: ItemCore { class ItemInfo { type = 301; mass = 2; }; };
    class muzzle_snds_H: ItemCore { class ItemInfo { type = 101; mass = 10; }; };
    class Rifle_Base_F: Default { type = 1; };
    class arifle_MX_F: Rifle_Base_F {
        magazines[] = {"30Rnd_65x39_caseless_mag"}; magazineWell[] = {"MX_65x39"};
        class WeaponSlotsInfo { mass = 65; };
    };
    class arifle_MX_ACO_pointer_F: arifle_MX_F {
        class LinkedItems {
            class LinkedItemsOptic { slot = "CowsSlot"; item = "optic_ACO"; };
            class LinkedItemsAcc { slot = "PointerSlot"; item = "acc_pointer_IR"; };
        };
    };
    class arifle_Katiba_F: Rifle_Base_F { magazines[] = {"30Rnd_65x39_caseless_green"}; class WeaponSlotsInfo { mass = 70; }; };
    class hgun_P07_F: Default { type = 2; magazines[] = {"16Rnd_9x21_Mag"}; class WeaponSlotsInfo { mass = 14; }; };
    class launch_NLAW_F: Default { type = 4; class WeaponSlotsInfo { mass = 100; }; };
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
           car = "C_Offroad_01_F" createVehicle [20, 0, 0];"#,
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
fn default_loadout_follows_the_config() {
    let mut vm = vm();
    assert_eq!(
        sqf(&mut vm, "weapons u"),
        r#"["arifle_MX_ACO_pointer_F","hgun_P07_F"]"#
    );
    assert_eq!(
        sqf(
            &mut vm,
            "[primaryWeapon u, secondaryWeapon u, handgunWeapon u, binocular u, uniform u, vest u, backpack u, headgear u]"
        ),
        r#"["arifle_MX_ACO_pointer_F","","hgun_P07_F","","U_B_CombatUniform_mcam","V_PlateCarrier1_rgr","","H_HelmetB"]"#
    );
    assert_eq!(
        sqf(&mut vm, "primaryWeaponItems u"),
        r#"["","acc_pointer_IR","optic_Aco",""]"#
    );
    // Magazines stored first-fit, then the weapons load from the uniform first.
    assert_eq!(
        sqf(&mut vm, "uniformItems u"),
        r#"["FirstAidKit","30Rnd_65x39_caseless_mag","30Rnd_65x39_caseless_mag","Chemlight_green"]"#
    );
    assert_eq!(sqf(&mut vm, "count vestItems u"), "14");
    assert_eq!(sqf(&mut vm, "count magazines u"), "17");
    assert_eq!(
        sqf(&mut vm, "[primaryWeaponMagazine u, handgunMagazine u]"),
        r#"[["30Rnd_65x39_caseless_mag"],["16Rnd_9x21_Mag"]]"#
    );
    assert_eq!(sqf(&mut vm, "items u"), r#"["FirstAidKit"]"#);
    assert_eq!(
        sqf(&mut vm, "assignedItems u"),
        r#"["ItemMap","ItemCompass","ItemWatch","ItemRadio","NVGoggles"]"#
    );
    assert_eq!(
        sqf(&mut vm, "assignedItems [u, true, false]"),
        r#"["ItemMap","ItemCompass","ItemWatch","ItemRadio","NVGoggles","H_HelmetB"]"#
    );
    // An array shorter than three elements: `[]`, not a DIM error (oracle).
    assert_eq!(sqf(&mut vm, "assignedItems [u]"), "[]");
    assert_eq!(sqf(&mut vm, "[loadUniform u, loadVest u]"), "[0.75,0.8]");
    assert_eq!(
        sqf(
            &mut vm,
            "[u hasWeapon 'ARIFLE_MX_ACO_POINTER_F', u hasWeapon 'Throw', u hasWeapon 'ItemMap']"
        ),
        "[true,true,false]"
    );
}

#[test]
fn removing_everything() {
    let mut vm = vm();
    eval(&mut vm, "removeAllWeapons u");
    assert_eq!(
        sqf(&mut vm, "[weapons u, magazines u, items u]"),
        "[[],[],[]]"
    );
    let mut vm = self::vm();
    eval(&mut vm, "removeAllItems u");
    assert_eq!(sqf(&mut vm, "[items u, count magazines u]"), "[[],17]");
    eval(&mut vm, "removeAllItemsWithMagazines u");
    assert_eq!(sqf(&mut vm, "magazines u"), "[]");
    eval(&mut vm, "removeAllAssignedItems u");
    assert_eq!(
        sqf(&mut vm, "[assignedItems u, headgear u]"),
        r#"[[],"H_HelmetB"]"#
    );
    eval(&mut vm, "removeAllAssignedItems [u, false, true]");
    assert_eq!(sqf(&mut vm, "headgear u"), "");
    eval(&mut vm, "removeAllContainers u");
    assert_eq!(
        sqf(&mut vm, "[uniform u, vest u, backpack u]"),
        r#"["","",""]"#
    );
}

#[test]
fn link_unlink_assign() {
    let mut vm = vm();
    eval(
        &mut vm,
        "u unassignItem 'ItemMap'; u unlinkItem 'NVGoggles'; u unlinkItem 'ItemGPS'",
    );
    assert_eq!(
        sqf(&mut vm, "[assignedItems u, items u]"),
        r#"[["ItemCompass","ItemWatch","ItemRadio"],["FirstAidKit","ItemMap"]]"#
    );
    eval(
        &mut vm,
        "u assignItem 'ItemMap'; u linkItem 'ItemGPS'; u linkItem 'H_Cap_red'; u linkItem 'G_Shades_Black'; u linkItem 'FirstAidKit'; u linkItem 'arifle_MX_F'",
    );
    assert_eq!(
        sqf(&mut vm, "[assignedItems u, items u, headgear u, goggles u]"),
        r#"[["ItemMap","ItemCompass","ItemWatch","ItemRadio","ItemGPS"],["FirstAidKit"],"H_Cap_red","G_Shades_Black"]"#
    );
}

#[test]
fn worn_slots() {
    let mut vm = vm();
    // Wrong type: the slot is emptied and nothing else happens (the original logs the type
    // mismatch to the RPT only; the oracle's `addHeadgear "ItemMap"` probe runs on).
    eval(&mut vm, "u addHeadgear 'ItemMap'");
    assert_eq!(sqf(&mut vm, "headgear u"), "");
    eval(
        &mut vm,
        "u addHeadgear 'H_Cap_red'; u addGoggles 'G_Shades_Black'",
    );
    assert_eq!(
        sqf(&mut vm, "[headgear u, goggles u]"),
        r#"["H_Cap_red","G_Shades_Black"]"#
    );
    eval(&mut vm, "removeHeadgear u; removeGoggles u");
    assert_eq!(sqf(&mut vm, "[headgear u, goggles u]"), r#"["",""]"#);
    // A new vest is empty; the old contents are gone.
    eval(&mut vm, "u addVest 'V_Rangemaster_belt'");
    assert_eq!(
        sqf(&mut vm, "[vest u, vestItems u, count magazines u]"),
        r#"["V_Rangemaster_belt",[],3]"#
    );
    // An OPFOR uniform is refused for BLUFOR, but the old one is gone.
    eval(&mut vm, "u addUniform 'U_O_CombatUniform_ocamo'");
    assert_eq!(sqf(&mut vm, "uniform u"), "");
    eval(&mut vm, "u forceAddUniform 'U_O_CombatUniform_ocamo'");
    assert_eq!(sqf(&mut vm, "uniform u"), "U_O_CombatUniform_ocamo");
    eval(&mut vm, "u addBackpack 'B_AssaultPack_mcamo_Ammo'");
    assert_eq!(
        sqf(&mut vm, "[backpack u, backpackItems u]"),
        r#"["B_AssaultPack_mcamo_Ammo",["FirstAidKit","FirstAidKit","30Rnd_65x39_caseless_mag","30Rnd_65x39_caseless_mag","30Rnd_65x39_caseless_mag"]]"#
    );
    eval(&mut vm, "removeBackpack u");
    assert_eq!(sqf(&mut vm, "backpack u"), "");
}

#[test]
fn items_fill_containers_in_order() {
    let mut vm = vm();
    eval(
        &mut vm,
        "removeAllItemsWithMagazines u; for '_i' from 1 to 10 do { u addItemToUniform 'FirstAidKit' }",
    );
    assert_eq!(
        sqf(&mut vm, "[count uniformItems u, loadUniform u]"),
        "[5,1]"
    );
    eval(&mut vm, "u addItem 'ToolKit'");
    assert_eq!(
        sqf(&mut vm, "items u"),
        r#"["FirstAidKit","FirstAidKit","FirstAidKit","FirstAidKit","FirstAidKit"]"#
    );
    eval(
        &mut vm,
        "u addBackpack 'B_AssaultPack_mcamo'; u addItem 'ToolKit'; u addItem 'FirstAidKit'",
    );
    assert_eq!(
        sqf(&mut vm, "[vestItems u, backpackItems u]"),
        r#"[["FirstAidKit"],["ToolKit"]]"#
    );
    eval(&mut vm, "u removeItem 'FirstAidKit'");
    assert_eq!(sqf(&mut vm, "count items u"), "6");
    eval(&mut vm, "u removeItems 'FirstAidKit'");
    assert_eq!(sqf(&mut vm, "items u"), r#"["ToolKit"]"#);
}

#[test]
fn weapons_and_attachments() {
    let mut vm = vm();
    // A new primary replaces the old one (dropped with its magazine) and goes last.
    eval(&mut vm, "u addWeapon 'arifle_Katiba_F'");
    assert_eq!(
        sqf(&mut vm, "weapons u"),
        r#"["hgun_P07_F","arifle_Katiba_F"]"#
    );
    assert_eq!(
        sqf(&mut vm, "[primaryWeaponMagazine u, count magazines u]"),
        "[[],17]"
    );
    eval(
        &mut vm,
        "removeAllWeapons u; u addMagazine '30Rnd_65x39_caseless_mag'; u addWeapon 'arifle_MX_F'; u addWeapon 'launch_NLAW_F'; u addWeapon 'Binocular'; u addWeapon 'ItemGPS'",
    );
    assert_eq!(
        sqf(&mut vm, "weapons u"),
        r#"["arifle_MX_F","launch_NLAW_F","Binocular"]"#
    );
    assert_eq!(
        sqf(&mut vm, "[primaryWeaponMagazine u, magazines u]"),
        r#"[["30Rnd_65x39_caseless_mag"],[]]"#
    );
    assert_eq!(sqf(&mut vm, "'ItemGPS' in assignedItems u"), "true");
    eval(
        &mut vm,
        "u addPrimaryWeaponItem 'optic_Hamr'; u addPrimaryWeaponItem 'muzzle_snds_H'; u addPrimaryWeaponItem 'FirstAidKit'",
    );
    assert_eq!(
        sqf(&mut vm, "primaryWeaponItems u"),
        r#"["muzzle_snds_H","","optic_Hamr",""]"#
    );
    eval(&mut vm, "u removePrimaryWeaponItem 'optic_Hamr'");
    assert_eq!(
        sqf(&mut vm, "primaryWeaponItems u"),
        r#"["muzzle_snds_H","","",""]"#
    );
    // A magazine-well magazine loads, replacing the loaded one.
    eval(
        &mut vm,
        "u addPrimaryWeaponItem '100Rnd_65x39_caseless_mag'",
    );
    assert_eq!(
        sqf(&mut vm, "primaryWeaponMagazine u"),
        r#"["100Rnd_65x39_caseless_mag"]"#
    );
    eval(
        &mut vm,
        "u removePrimaryWeaponItem '100Rnd_65x39_caseless_mag'",
    );
    assert_eq!(sqf(&mut vm, "primaryWeaponMagazine u"), "[]");
    eval(
        &mut vm,
        "u removeWeapon 'launch_NLAW_F'; u removeWeapon 'ItemGPS'",
    );
    assert_eq!(
        sqf(&mut vm, "[weapons u, 'ItemGPS' in assignedItems u]"),
        r#"[["arifle_MX_F","Binocular"],false]"#
    );
}

#[test]
fn magazines() {
    let mut vm = vm();
    eval(
        &mut vm,
        "removeAllItemsWithMagazines u; u addMagazine '30Rnd_65x39_caseless_mag'; u addMagazine ['30Rnd_65x39_caseless_mag', 5]; u addMagazine ['30Rnd_65x39_caseless_mag', 500]; u addMagazine ['30Rnd_65x39_caseless_mag', -2]; u addMagazine ['30Rnd_65x39_caseless_mag', 0]; u addMagazine 'FirstAidKit'",
    );
    assert_eq!(
        sqf(&mut vm, "magazinesAmmo u"),
        r#"[["30Rnd_65x39_caseless_mag",30],["30Rnd_65x39_caseless_mag",5],["30Rnd_65x39_caseless_mag",30],["30Rnd_65x39_caseless_mag",30]]"#
    );
    assert_eq!(sqf(&mut vm, "count magazinesAmmo [u, true]"), "5");
    eval(
        &mut vm,
        "u addMagazines ['HandGrenade', 2]; u removeMagazine ['30Rnd_65x39_caseless_mag', 5]",
    );
    assert_eq!(sqf(&mut vm, "count magazines u"), "5");
    eval(&mut vm, "u removeMagazines '30Rnd_65x39_caseless_mag'");
    assert_eq!(
        sqf(&mut vm, "magazines u"),
        r#"["HandGrenade","HandGrenade"]"#
    );
    // Loaded magazines stay.
    assert_eq!(
        sqf(&mut vm, "primaryWeaponMagazine u"),
        r#"["30Rnd_65x39_caseless_mag"]"#
    );
}

#[test]
fn non_units_have_no_gear() {
    let mut vm = vm();
    assert_eq!(
        sqf(
            &mut vm,
            "[weapons objNull, primaryWeapon objNull, magazines car, items car, uniform car, assignedItems objNull]"
        ),
        r#"[[],"",[],[],"",[]]"#
    );
    eval(
        &mut vm,
        "car addItem 'FirstAidKit'; removeAllWeapons objNull",
    );
}
