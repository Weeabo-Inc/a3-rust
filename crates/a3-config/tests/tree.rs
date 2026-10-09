//! Merged config tree: patching across addons, inheritance and the query API.

use a3_config::{ConfigTree, ExportMode, Value, parse_text, write_text};

fn tree(sources: &[&str]) -> ConfigTree {
    let mut tree = ConfigTree::new();
    for src in sources {
        tree.merge(&parse_text(src).unwrap_or_else(|e| panic!("{e}")));
    }
    tree
}

#[test]
fn lookup_walks_the_base_chain_case_insensitively() {
    let t = tree(&[
        "class CfgVehicles { class Man { scope = 0; side = 1; }; class B_Soldier_F: Man { scope = 2; }; };",
    ]);
    let soldier = t.root() >> "cfgvehicles" >> "B_SOLDIER_F";
    assert!(soldier.is_class());
    assert_eq!(soldier.name(), "B_Soldier_F");
    assert_eq!((&soldier >> "scope").number(), 2.0);
    assert_eq!((&soldier >> "side").number(), 1.0);
    assert!((&soldier >> "missing").is_null());
    assert_eq!(soldier.inherits_from().name(), "Man");
}

#[test]
fn nested_base_with_same_name_resolves_in_the_enclosing_class_base() {
    let t = tree(&[r#"
        class CfgVehicles {
            class Car {
                class Turrets { class MainTurret { gunner = "a"; ammo = 1; }; };
            };
            class MyCar: Car {
                class Turrets: Turrets {
                    class MainTurret: MainTurret { gunner = "b"; };
                };
            };
        };"#]);
    let turret = t.root() >> "CfgVehicles" >> "MyCar" >> "Turrets" >> "MainTurret";
    assert_eq!((&turret >> "gunner").text(), "b");
    assert_eq!((&turret >> "ammo").number(), 1.0);
    let base = turret.inherits_from();
    assert_eq!(
        base.path_string(),
        "bin\\config.bin/CfgVehicles/Car/Turrets/MainTurret"
    );
}

#[test]
fn external_declaration_refers_to_the_inherited_class() {
    let t = tree(&[r#"
        class CfgVehicles {
            class All { class NewTurret { x = 7; }; };
            class Car: All {
                class NewTurret;
                class Turrets { class MainTurret: NewTurret {}; };
            };
        };"#]);
    let turret = t.root() >> "CfgVehicles" >> "Car" >> "Turrets" >> "MainTurret";
    assert_eq!((&turret >> "x").number(), 7.0);
    assert_eq!(
        turret.inherits_from().path_string(),
        "bin\\config.bin/CfgVehicles/All/NewTurret"
    );
}

#[test]
fn inherited_subclass_keeps_the_access_path() {
    let t = tree(&["class A { class Sub { v = 1; }; }; class B: A {};"]);
    let sub = t.root() >> "B" >> "Sub";
    assert_eq!(sub.path_string(), "bin\\config.bin/B/Sub");
    let names: Vec<_> = sub
        .hierarchy()
        .iter()
        .map(|c| c.name().to_owned())
        .collect();
    assert_eq!(names, ["", "B", "Sub"]);
}

#[test]
fn later_configs_patch_earlier_classes() {
    let t = tree(&[
        "class CfgX { class Base {}; class Other {}; class Item: Base { a = 1; b = 2; }; };",
        "class CfgX { class Item: Other { b = 3; c = 4; }; class New {}; };",
    ]);
    let item = t.root() >> "CfgX" >> "Item";
    assert_eq!((&item >> "a").number(), 1.0);
    assert_eq!((&item >> "b").number(), 3.0);
    assert_eq!((&item >> "c").number(), 4.0);
    assert_eq!(item.inherits_from().name(), "Other");
    let own: Vec<_> = item.entries().iter().map(|c| c.name().to_owned()).collect();
    assert_eq!(own, ["a", "b", "c"], "overrides keep their position");
    assert!((t.root() >> "CfgX" >> "New").is_class());
    assert!(
        t.warnings()
            .iter()
            .any(|w| w.contains("Updating base class Base->Other")),
        "{:?}",
        t.warnings()
    );
}

#[test]
fn patch_without_base_removes_inheritance() {
    let t = tree(&["class A { x = 1; }; class B: A {};", "class B { y = 2; };"]);
    assert!((t.root() >> "B" >> "x").is_null());
    assert!(t.root().get("B").inherits_from().is_null());
}

#[test]
fn external_declaration_becomes_real_when_defined_later() {
    let t = tree(&[
        "class CfgX { class Later; class Uses: Later {}; };",
        "class CfgX { class Later { v = 5; }; };",
    ]);
    assert_eq!((t.root() >> "CfgX" >> "Uses" >> "v").number(), 5.0);
}

#[test]
fn unresolved_external_reads_as_missing() {
    let t = tree(&["class CfgX { class Nowhere; };"]);
    assert!((t.root() >> "CfgX" >> "Nowhere").is_null());
}

#[test]
fn entry_count_and_at_match_entries() {
    let t = tree(&["class CfgX { class First {}; n = 3; class Ghost; class Second {}; };"]);
    let cfg = t.root() >> "CfgX";
    let entries = cfg.entries();
    let names: Vec<_> = entries.iter().map(|e| e.name()).collect();
    assert_eq!(
        names,
        ["First", "n", "Second"],
        "Ghost is unresolved, so hidden"
    );
    assert_eq!(cfg.entry_count(), entries.len());
    for (i, e) in entries.iter().enumerate() {
        assert_eq!(cfg.entry_at(i).node_path(), e.node_path());
        assert_eq!(cfg.entry_at(i).name(), e.name());
    }
    assert!(
        cfg.entry_at(entries.len()).is_null(),
        "out of range is null"
    );
}

#[test]
fn delete_removes_unreferenced_classes_only() {
    let t = tree(&[
        "class CfgX { class Gone {}; class Base {}; class Child: Base {}; };",
        "class CfgX { delete Gone; delete Base; };",
    ]);
    assert!((t.root() >> "CfgX" >> "Gone").is_null());
    assert!((t.root() >> "CfgX" >> "Base").is_class());
    assert!(
        t.warnings()
            .iter()
            .any(|w| w.contains("Cannot delete class Base"))
    );
}

#[test]
fn array_append_extends_own_and_inherited_arrays() {
    let t = tree(&[
        "class A { list[] = {1, 2}; }; class B: A { list[] += {3}; };",
        "class A { list[] += {\"x\"}; };",
    ]);
    assert_eq!(
        (t.root() >> "A" >> "list").array(),
        vec![Value::Int(1), Value::Int(2), Value::String("x".into())]
    );
    assert_eq!(
        (t.root() >> "B" >> "list").array(),
        vec![
            Value::Int(1),
            Value::Int(2),
            Value::String("x".into()),
            Value::Int(3)
        ]
    );
}

#[test]
fn value_accessors_follow_engine_coercions() {
    let t = tree(&[r#"
        class C {
            i = 3; f = 0.5; big = 3000000000;
            s = "text"; n = "1.5"; yes = "true"; no = "False"; hex = "0x10";
            arr[] = {1, "a"};
            class Sub {};
        };"#]);
    let c = t.root() >> "C";
    let get = |n: &str| c.get(n);
    assert_eq!(get("i").number(), 3.0);
    assert_eq!(get("f").number(), 0.5);
    assert_eq!(get("big").number(), 3.0e9);
    assert_eq!(get("n").number(), 1.5);
    assert_eq!(get("yes").number(), 1.0);
    assert_eq!(get("no").number(), 0.0);
    assert_eq!(get("hex").number(), 16.0);
    assert_eq!(get("s").number(), 0.0);
    assert_eq!(get("arr").number(), 0.0);
    assert_eq!(get("missing").number(), 0.0);

    assert_eq!(get("s").text(), "text");
    assert_eq!(get("i").text(), "3");
    assert_eq!(get("f").text(), "0.5");
    assert_eq!(get("arr").text(), "");

    assert_eq!(
        get("arr").array(),
        vec![Value::Int(1), Value::String("a".into())]
    );
    assert!(get("s").array().is_empty());

    assert!(get("i").is_number() && get("f").is_number() && get("big").is_number());
    assert!(!get("n").is_number() && get("n").is_text());
    assert!(get("arr").is_array() && !get("arr").is_text());
    assert!(get("Sub").is_class() && !get("Sub").is_number());
    assert!(!get("missing").is_class() && get("missing").is_null());
}

#[test]
fn base_cycles_do_not_hang() {
    let t = tree(&["class A: B { }; class B: A { };"]);
    assert!((t.root() >> "A" >> "x").is_null());
}

fn exported(t: &ConfigTree, path: &[&str], mode: ExportMode) -> String {
    let mut c = t.root();
    for p in path {
        c = c.get(p);
    }
    write_text(&c.export(mode).expect("entry exists"))
}

#[test]
fn export_merged_shows_own_entries_as_patched() {
    let t = tree(&[
        "class A { x = 1; class S { y = 1; }; }; class B: A { z = 2; list[] += {3}; class Ext; };",
        "class B: A { z = 4; };",
    ]);
    assert_eq!(
        exported(&t, &["B"], ExportMode::Merged),
        "class B: A\n{\n    z = 4;\n    list[] += {3};\n    class Ext;\n};\n"
    );
}

#[test]
fn export_resolved_flattens_inheritance() {
    let t = tree(&[
        "class A { x = 1; list[] = {1}; class S { y = 1; }; }; class B: A { z = 2; list[] += {3}; };",
    ]);
    assert_eq!(
        exported(&t, &["B"], ExportMode::Resolved),
        "class B\n{\n    z = 2;\n    list[] = {1, 3};\n    x = 1;\n    class S\n    {\n        y = 1;\n    };\n};\n"
    );
    assert_eq!(exported(&t, &["B", "x"], ExportMode::Resolved), "x = 1;\n");
    assert!(t.root().get("nope").export(ExportMode::Merged).is_none());
}
