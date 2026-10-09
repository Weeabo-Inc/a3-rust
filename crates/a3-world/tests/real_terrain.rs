//! Terrain data against the server oracle's reference values. Skipped when `A3_ROOT` is unset.
//!
//! The expectations are the recorded results of `tools/oracle/probes/95_world_stratis.probes`
//! and `96_world_vr.probes` (`getTerrainHeightASL`, `surfaceNormal`, `surfaceIsWater`).

use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree};
use a3_wrp::Terrain;

fn load(root: &std::ffi::OsStr, world: &str) -> Option<(Terrain, Arc<ConfigTree>)> {
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let classes = data.config.root().get("CfgWorlds");
    let class: ConfigRef<'_> = classes.get(world);
    if !class.is_class() {
        eprintln!("skipping: no CfgWorlds >> {world}");
        return None;
    }
    let wrp = class.get("worldName").text();
    let bytes = data.vfs.open(&wrp).unwrap();
    let terrain = Terrain::parse(&bytes).unwrap();
    eprintln!(
        "{world}: land {}x{} cell {} m, heightmap {}x{}, {} materials, {} objects",
        terrain.land_grid.width,
        terrain.land_grid.height,
        terrain.land_cell_size,
        terrain.heightmap.width(),
        terrain.heightmap.height(),
        terrain.materials.len(),
        terrain.objects.len()
    );
    Some((terrain, data.config.clone()))
}

/// The oracle prints terrain heights with six significant digits.
fn close(what: &str, got: f32, expected: f32) {
    assert!(
        (got - expected).abs() <= expected.abs() * 1e-4 + 1e-4,
        "{what}: got {got}, oracle {expected}"
    );
}

#[test]
fn stratis_heights_match_the_oracle() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some((terrain, _)) = load(&root, "Stratis") else {
        return;
    };

    for (x, y, expected) in [
        (4096.0, 4096.0, 161.18),
        (2950.0, 6050.0, 3.35),
        (1900.0, 5700.0, 5.56),
        (4300.0, 3800.0, 216.71),
        (6250.0, 5300.0, 13.05),
        (2000.0, 2700.0, 3.39),
        (3500.0, 4500.0, 160.89),
        (3501.37, 4499.81, 161.355),
        (3500.0, 4496.0, 163.12),
        (3502.0, 4498.0, 162.335),
        (500.0, 500.0, -134.49),
    ] {
        close(
            &format!("height [{x}, {y}]"),
            terrain.surface_height(x, y),
            expected,
        );
    }
}

#[test]
fn stratis_water_line_matches_the_oracle() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some((terrain, _)) = load(&root, "Stratis") else {
        return;
    };
    // wstr.water_shore_grid: surfaceIsWater from x = 1500 to 2500 step 100 at y = 5000 is
    // [false, false, false, true, true, true, true, false, false, false, false].
    let expected = [
        false, false, false, true, true, true, true, false, false, false, false,
    ];
    for (i, x) in (1500..=2500).step_by(100).enumerate() {
        let h = terrain.surface_height(x as f32, 5000.0);
        assert_eq!(h < 0.0, expected[i], "x = {x}, height {h}");
    }
}

#[test]
fn stratis_surface_normal_matches_the_oracle() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some((terrain, _)) = load(&root, "Stratis") else {
        return;
    };
    // wstr.surfacenormal / wstr.lis_ground_normal at [3500, 4500]; the script vector is
    // [x east, y north, z up].
    let n = a3_world::script::terrain_normal(&terrain, 3500.0, 4500.0);
    let script = [n.x, n.z, n.y];
    let expected = [-0.303743, 0.463934, 0.832169];
    for (got, want) in script.iter().zip(expected) {
        assert!(
            (got - want).abs() <= 1e-5,
            "surfaceNormal [3500, 4500] = {script:?}, oracle {expected:?}"
        );
    }
}

#[test]
fn vr_heights_match_the_oracle() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let Some((terrain, _)) = load(&root, "VR") else {
        return;
    };

    for (x, y, expected) in [
        (4096.0, 4096.0, 5.0),
        (0.0, 0.0, 5.0),
        (100.0, 100.0, 5.0),
        (1000.0, 2000.0, 5.0),
        (5000.0, 300.0, 5.0),
        (8000.0, 8000.0, 5.0),
    ] {
        close(
            &format!("height [{x}, {y}]"),
            terrain.surface_height(x, y),
            expected,
        );
    }
}
