//! Real-data checks on Altis. Skip when `A3_ROOT` is not set.

use std::time::Instant;

use a3_landscape::TerrainLayers;
use a3_landscape_render::detail::NO_LAYER;
use a3_landscape_render::lod::{distance_to_box, farthest_in_box};
use a3_landscape_render::{Landscape, LodQuadtree, LodSettings, TileCoord, TileFormat};
use a3_render::TextureFormat;
use a3_vfs::Vfs;
use a3_wrp::Terrain;
use glam::DVec3;

fn altis() -> Option<(Vfs, Terrain)> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let vfs = Vfs::new();
    vfs.mount_game(std::path::Path::new(&root), &[]);
    let bytes = vfs.open(r"a3\map_altis\altis.wrp").expect("altis.wrp");
    Some((vfs, Terrain::parse(&bytes).expect("parse altis.wrp")))
}

#[test]
fn altis_satellite_tiles_overview_and_heights() {
    let Some((vfs, terrain)) = altis() else {
        return;
    };
    let start = Instant::now();
    let layers = TerrainLayers::load(&vfs, &terrain);
    let loaded = start.elapsed();
    let landscape = Landscape::from_terrain(&terrain, &layers, |p| vfs.open(p.as_str()).ok());
    eprintln!(
        "layer materials {loaded:.2?}, landscape {:.2?}",
        start.elapsed() - loaded
    );

    let grid = landscape.tiles.grid.expect("Altis has a satellite grid");
    assert_eq!((grid.tiles, grid.step, grid.size), (64, 480.0, 512.0));
    let own = landscape
        .tiles
        .tiles
        .iter()
        .filter(|t| t.satellite.is_some())
        .count();
    assert_eq!(own, 1863, "tiles with their own satellite image");
    let overview = landscape.overview.as_ref().expect("overview");
    assert_eq!((overview.width, overview.height), (3840, 3840));

    // Kavala (x 3600, z 13000) lies in tile column 7, row (30720 - 13000) / 480 = 36.
    let cell = |x: f32, z: f32| {
        let (cx, cz) = (
            (x / landscape.land_cell) as u32,
            (z / landscape.land_cell) as u32,
        );
        let t = *landscape.tiles.cell_tiles.get(cx, cz).unwrap();
        landscape.tiles.tiles[usize::from(t)].coord
    };
    assert_eq!(cell(3600.0, 13000.0), TileCoord { col: 7, row: 36 });

    // Every tile streams in the same shape: 512 px DXT1 with 8 mips.
    let kavala = landscape
        .tiles
        .tiles
        .iter()
        .find(|t| t.coord == TileCoord { col: 7, row: 36 })
        .and_then(|t| t.satellite.clone())
        .unwrap();
    let bytes = vfs.open(kavala.as_str()).unwrap();
    let format = TileFormat::probe(&bytes, true).unwrap();
    assert_eq!(
        format,
        TileFormat {
            format: TextureFormat::Bc1,
            size: 512,
            mips: 8
        }
    );
    assert!(format.decode(&bytes).is_some());

    // Its mask streams as BC3 of the same shape.
    let kavala_tile = landscape
        .tiles
        .tiles
        .iter()
        .find(|t| t.coord == TileCoord { col: 7, row: 36 })
        .unwrap();
    let mask = vfs
        .open(kavala_tile.mask.as_ref().unwrap().as_str())
        .unwrap();
    let mask_format = TileFormat::probe(&mask, true).unwrap();
    assert_eq!(
        (mask_format.format, mask_format.size),
        (TextureFormat::Bc3, 512)
    );

    // Detail layers: every gdt_* surface once, each material pointing at its tile.
    let detail = &landscape.detail;
    assert_eq!(detail.materials.len(), terrain.materials.len());
    assert!(
        (15..=40).contains(&detail.textures.len()),
        "{} detail textures",
        detail.textures.len()
    );
    assert!(detail.textures.iter().all(|t| {
        let name = t.color.file_name().unwrap_or_default();
        name.starts_with("gdt_") && t.normal.is_some()
    }));
    let (cx, cz) = (
        (3600.0 / landscape.land_cell) as u32,
        (13000.0 / landscape.land_cell) as u32,
    );
    let material = *landscape.material_indices.get(cx, cz).unwrap();
    let slots = detail.materials[usize::from(material)];
    assert_eq!(slots.tile, *landscape.tiles.cell_tiles.get(cx, cz).unwrap());
    assert!(slots.layers.iter().any(|&l| l != NO_LAYER));

    for (x, z) in [(3600.0, 13000.0), (14382.4, 15924.6), (25000.3, 21000.7)] {
        let (ours, engine) = (landscape.surface_height(x, z), terrain.surface_height(x, z));
        assert!((ours - engine).abs() < 1e-3, "{ours} vs {engine}");
    }
}

#[test]
fn altis_lod_selection_is_crack_free_over_real_heights() {
    let Some((vfs, terrain)) = altis() else {
        return;
    };
    let layers = TerrainLayers::load(&vfs, &terrain);
    let landscape = Landscape::from_terrain(&terrain, &layers, |p| vfs.open(p.as_str()).ok());
    let tree = LodQuadtree::new(&landscape.heights, LodSettings::default());
    assert_eq!(tree.levels(), 8, "4096 cells in 32-cell patches");
    let mut out = Vec::new();
    for camera in [
        DVec3::new(3600.0, 120.0, 13000.0),
        DVec3::new(14382.0, 30.0, 15924.0),
        DVec3::new(20000.0, 1500.0, 20000.0),
        DVec3::new(10000.0, 400.0, 18000.0),
    ] {
        tree.select(camera, None, &mut out);
        let area: u64 = out.iter().map(|n| u64::from(n.cells).pow(2)).sum();
        assert_eq!(area, 4096 * 4096, "the selection covers Altis once");
        for n in &out {
            let (min, max) = tree.bounds(n);
            if n.level + 1 < tree.levels() {
                assert!(farthest_in_box(camera, min, max) <= tree.morph_range(n.level + 1).0);
            }
            if n.level > 0 {
                assert!(distance_to_box(camera, min, max) >= tree.range(n.level - 1));
            }
        }
    }
}
