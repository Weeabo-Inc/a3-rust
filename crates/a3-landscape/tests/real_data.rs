//! Reads the world config and every layer material of every shipped world. Skipped when
//! `A3_ROOT` is unset.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use a3_gamedata::{GameData, LoadOptions};
use a3_landscape::{Surfaces, TerrainLayers, UvSource, WorldConfig, world_classes};
use a3_wrp::Terrain;
use glam::Vec3;

#[test]
fn every_world_resolves_its_cells() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let start = Instant::now();
    let game = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    eprintln!("game data loaded in {:.2?}", start.elapsed());
    let surfaces = Surfaces::load(&game.config);
    eprintln!(
        "{} surfaces, {} surface characters",
        surfaces.surfaces.len(),
        surfaces.characters.len()
    );
    assert!(surfaces.surfaces.len() > 50);

    let worlds = world_classes(&game.config);
    eprintln!("worlds: {worlds:?}");
    let mut checked = 0;
    for class in &worlds {
        let world = WorldConfig::load(&game.config, class).unwrap();
        let Ok(bytes) = game.vfs.open(world.wrp.as_str()) else {
            eprintln!("{class}: {} not readable (encrypted DLC?)", world.wrp);
            continue;
        };
        let terrain = Terrain::parse(&bytes).unwrap();
        let start = Instant::now();
        let layers = TerrainLayers::load(&game.vfs, &terrain);
        let elapsed = start.elapsed();
        for e in layers.errors.iter().take(5) {
            eprintln!("  {e}");
        }
        assert!(layers.errors.is_empty(), "{class}: material errors");

        if world.map_size > 0.0 {
            assert_eq!(world.map_size, terrain.world_size(), "{class}: mapSize");
        }
        assert_eq!(
            world.sound_map_size_coef * terrain.land_grid.width,
            terrain.sound_map.width(),
            "{class}: sound map"
        );

        let mut shaders = BTreeSet::new();
        let mut slots = BTreeMap::<usize, usize>::new();
        let mut layer_textures = BTreeSet::new();
        for m in layers.materials.iter().flatten() {
            shaders.insert(m.pixel_shader.clone());
            for l in &m.layers {
                *slots.entry(l.slot).or_default() += 1;
                layer_textures.insert(l.color.texture.to_ascii_lowercase());
            }
            assert_eq!(m.satellite.uv.source, UvSource::WorldPos, "{}", m.path);
        }
        let unmatched: Vec<&String> = layer_textures
            .iter()
            .filter(|t| surfaces.for_texture(t).is_none())
            .collect();

        // The cell centre lies inside its satellite and mask tiles.
        let land = terrain.land_grid.width;
        let step = (land / 64).max(1);
        let mut cells = 0;
        for z in (0..land).step_by(step as usize) {
            for x in (0..land).step_by(step as usize) {
                let Some(cell) = layers.cell(&terrain, x, z) else {
                    continue;
                };
                let size = terrain.land_cell_size;
                let p = Vec3::new((x as f32 + 0.5) * size, 0.0, (z as f32 + 0.5) * size);
                for stage in [cell.satellite(), cell.mask()] {
                    let uv = stage.uv.apply(p);
                    assert!(
                        (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y),
                        "{class}: cell ({x}, {z}) maps to {uv:?} in {}",
                        cell.material.path
                    );
                }
                cells += 1;
            }
        }
        eprintln!(
            "{class}: {} ({} m), {} materials loaded in {elapsed:.2?}, shaders {shaders:?}, \
             layer slots {slots:?}, {} layer textures ({} without a surface: {unmatched:?}), \
             {cells} cells checked, {} clutter models, {} locations, roads {:?}",
            world.wrp,
            world.map_size,
            layers.materials.iter().flatten().count(),
            layer_textures.len(),
            unmatched.len(),
            world.clutter.len(),
            world.locations.len(),
            world.roads_shape,
        );
        checked += 1;
    }
    assert_eq!(checked, 6, "expected the six readable worlds");
}

#[test]
fn altis_sea_waves_and_water_parameters() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root)).unwrap();
    let altis = WorldConfig::load(&game.config, "Altis").unwrap();
    let waves = altis.sea.waves;
    assert_eq!(waves.water_grid, 50.0);
    assert_eq!(waves.max_tide, 0.0);
    assert_eq!(waves.max_wave, 0.25);
    assert!((waves.x_scale - 2.0 / 50.0).abs() < 1e-7, "\"2.0/50\"");
    assert!((waves.z_scale - 1.0 / 50.0).abs() < 1e-7, "\"1.0/50\"");
    assert_eq!((waves.x_duration_ms, waves.z_duration_ms), (5000, 10000));
    let ex = altis.sea.water_ex;
    assert_eq!(ex.fog_density, Some(0.07));
    assert_eq!(ex.fog_gradient_coefs, Some(Vec3::new(0.35, 1.0, 1.7)));
    assert_eq!(ex.ss_reflection_strength, Some(0.85));
    assert_eq!(ex.specular_power_overcast0, Some(750.0));
    assert_eq!(ex.refraction_max_dist, Some(5.1));
    assert_eq!(
        ex.surface_opacity.unwrap_or(0.0),
        0.0,
        "an opaque surface tint is off"
    );
}
