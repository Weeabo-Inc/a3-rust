//! `Weather >> LightingNew` tables: colour parsing and interpolation.

use a3_config::{ConfigTree, parse_text};
use a3_environment::{LightingTable, ev_color};
use glam::Vec3;

const CONFIG: &str = r#"
class LightingNew {
    class L0 { height = 0; overcast = 0.25; sunAngle = -10; sunOrMoon = 0;
        diffuse[] = {0.1, 0.2, 0.3}; ambient[] = {{1, 1, 1}, 0};
        groundReflection[] = {0.3, 0.3, 0.3};
        sky[] = {0.1, 0.1, 0.1}; skyAroundSun[] = {0.1, 0.1, 0.1}; fogColor[] = {0.1, 0.1, 0.1};
        apertureMin = 2; apertureStandard = 4; apertureMax = 8; standardAvgLum = 4; };
    class L1 { height = 0; overcast = 0.25; sunAngle = 30; sunOrMoon = 1;
        diffuse[] = {{1, 1, 1}, 4}; ambient[] = {{1, 1, 1}, 2};
        groundReflection[] = {0.5, 0.5, 0.5}; ambientMid[] = {1, 1, 1};
        sky[] = {{0.2, 0.4, 0.8}, 3}; skyAroundSun[] = {1, 1, 1}; fogColor[] = {1, 1, 1};
        apertureMin = 20; apertureStandard = 40; apertureMax = 80; standardAvgLum = 400; };
    class L2 { height = 0; overcast = 0.85; sunAngle = 30; sunOrMoon = 1;
        diffuse[] = {{1, 1, 1}, 2}; ambient[] = {{1, 1, 1}, 2};
        groundReflection[] = {0.5, 0.5, 0.5};
        sky[] = {1, 1, 1}; skyAroundSun[] = {1, 1, 1}; fogColor[] = {1, 1, 1};
        apertureMin = 10; apertureStandard = 20; apertureMax = 40; standardAvgLum = 100; };
};
"#;

fn table() -> LightingTable {
    let tree = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
    LightingTable::from_config(&tree.root().get("LightingNew"))
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).abs().max_element() < 1e-3
}

#[test]
fn ev_colours_are_scaled_to_a_luminance_of_two_to_the_ev() {
    // Rec. 601 luma of (0.2, 0.4, 0.8) is 0.4888; scaled to 2^3 = 8.
    let c = ev_color(Vec3::new(0.2, 0.4, 0.8), 3.0);
    assert!((c.x * 0.299 + c.y * 0.587 + c.z * 0.114 - 8.0).abs() < 1e-3);
    assert!(close(c / c.x, Vec3::new(1.0, 2.0, 4.0)));
}

#[test]
fn reads_plain_and_ev_colours_and_defaults_ambient_mid() {
    let t = table();
    assert_eq!(t.len(), 3);
    let l0 = t.sample(0.0, 0.25, (-10f32).to_radians().sin());
    assert!(close(l0.diffuse, Vec3::new(0.1, 0.2, 0.3)));
    assert!(close(l0.ambient, Vec3::ONE));
    // No ambientMid: halfway between ambient and groundReflection.
    assert!(close(l0.ambient_mid, Vec3::splat(0.65)));
    assert_eq!(l0.aperture_standard, 4.0);
}

#[test]
fn interpolates_linearly_in_the_sine_of_the_sun_angle() {
    let t = table();
    let (s0, s1) = ((-10f32).to_radians().sin(), 30f32.to_radians().sin());
    let mid = t.sample(0.0, 0.25, (s0 + s1) / 2.0);
    assert!(close(
        mid.diffuse,
        (Vec3::new(0.1, 0.2, 0.3) + Vec3::splat(16.0)) / 2.0
    ));
    assert!((mid.sun_or_moon - 0.5).abs() < 1e-5);
    assert!((mid.aperture_standard - 22.0).abs() < 1e-4);
    // Clamped outside the table.
    let above = t.sample(0.0, 0.25, 1.0);
    assert!(close(above.diffuse, Vec3::splat(16.0)));
}

#[test]
fn interpolates_between_overcast_groups_and_clamps_outside() {
    let t = table();
    let s = 30f32.to_radians().sin();
    let half = t.sample(0.0, 0.55, s);
    assert!(close(half.diffuse, Vec3::splat(10.0)));
    let clear = t.sample(0.0, 0.0, s);
    assert!(close(clear.diffuse, Vec3::splat(16.0)));
    let rainy = t.sample(0.0, 1.0, s);
    assert!(close(rainy.diffuse, Vec3::splat(4.0)));
    // The 0.85 group has a single entry: used for every sun angle.
    let night_rainy = t.sample(0.0, 0.85, -1.0);
    assert!(close(night_rainy.diffuse, Vec3::splat(4.0)));
}
