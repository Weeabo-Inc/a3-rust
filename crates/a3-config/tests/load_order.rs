//! Addon load order from CfgPatches `requiredAddons`.

use a3_config::{AddonPatches, load_order, parse_text};

fn addon(patches: &[&str], required: &[&str]) -> AddonPatches {
    AddonPatches {
        patches: patches.iter().map(|s| s.to_string()).collect(),
        required: required.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn dependencies_load_first_and_independent_addons_keep_discovery_order() {
    let addons = [
        addon(&["c"], &["b"]),
        addon(&["a"], &[]),
        addon(&["b"], &["A"]),
        addon(&["d"], &[]),
    ];
    let order = load_order(&addons);
    assert_eq!(order.order, [1, 2, 0, 3]);
    assert!(order.missing.is_empty());
    assert!(order.cycles.is_empty());
}

#[test]
fn missing_requirements_are_reported_and_ignored() {
    let addons = [addon(&["x"], &["not_here"]), addon(&["y"], &[])];
    let order = load_order(&addons);
    assert_eq!(order.order, [0, 1]);
    assert_eq!(order.missing, [(0, "not_here".to_string())]);
}

#[test]
fn cycles_are_broken_in_discovery_order() {
    let addons = [
        addon(&["z"], &[]),
        addon(&["p"], &["q"]),
        addon(&["q"], &["p"]),
    ];
    let order = load_order(&addons);
    assert_eq!(order.order, [0, 1, 2]);
    assert_eq!(order.cycles, [1]);
}

#[test]
fn addon_with_several_patches_is_one_unit() {
    let addons = [
        addon(&["needs"], &["second"]),
        addon(&["first", "second"], &["first"]),
    ];
    assert_eq!(load_order(&addons).order, [1, 0]);
}

#[test]
fn patches_are_read_from_cfgpatches() {
    let config = parse_text(
        r#"class CfgPatches {
            class A3_Foo { requiredAddons[] = {"A3_Data_F", "A3_Bar"}; };
            class A3_Foo_Extra { requiredAddons[] = {"A3_Bar"}; };
        };"#,
    )
    .unwrap();
    let p = AddonPatches::from_config(&config);
    assert_eq!(p.patches, ["A3_Foo", "A3_Foo_Extra"]);
    assert_eq!(p.required, ["A3_Data_F", "A3_Bar"]);
}
