//! Parses every terrain in a real game install. Skipped when `A3_ROOT` is unset.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

use a3_core::VfsPath;
use a3_vfs::{Vfs, optional_mod_dirs};
use a3_wrp::{MapType, ObjectInstance, Terrain};
use glam::Vec3;

#[test]
fn every_wrp_in_the_install_parses() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));
    let paths = vfs.glob("**\\*.wrp");
    assert!(!paths.is_empty(), "no .wrp files in the VFS");

    for path in paths {
        let data = vfs.open(path.as_str()).unwrap();
        let start = Instant::now();
        let terrain = Terrain::parse(&data).unwrap_or_else(|e| panic!("{path}: {e}"));
        let elapsed = start.elapsed();

        let heights = terrain.heightmap.as_slice();
        let (lo, hi) = heights
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
        let mut kinds: BTreeMap<MapType, usize> = BTreeMap::new();
        for m in &terrain.map_objects {
            *kinds.entry(m.kind).or_default() += 1;
        }
        eprintln!(
            "{path}: v{} app {:?}, {:.0} m, land {}x{} @ {} m, heights {}x{} @ {} m ({lo:.1}..{hi:.1}), \
             {} materials, {} models, {} objects, {} entities, {} road parts, {} map objects, \
             {:.1} MiB, parsed in {elapsed:.2?}",
            terrain.version,
            terrain.app_id,
            terrain.world_size(),
            terrain.land_grid.width,
            terrain.land_grid.height,
            terrain.land_cell_size,
            terrain.heightmap.width(),
            terrain.heightmap.height(),
            terrain.terrain_cell_size(),
            terrain.materials.len(),
            terrain.models.len(),
            terrain.objects.len(),
            terrain.entities.len(),
            terrain.roads.parts.len(),
            terrain.map_objects.len(),
            data.len() as f64 / (1 << 20) as f64,
        );
        eprintln!("  map objects: {kinds:?}");
        eprintln!(
            "  approx. memory {:.0} MiB",
            approx_memory(&terrain) as f64 / (1 << 20) as f64
        );

        assert!(heights.iter().all(|h| h.is_finite()), "{path}: heights");
        assert!(
            (-10_000.0..10_000.0).contains(&lo) && hi < 10_000.0,
            "{path}: heights"
        );
        for o in &terrain.objects {
            assert!(
                (o.model_index as usize) < terrain.models.len(),
                "{path}: object {} model index {}",
                o.id,
                o.model_index
            );
        }
        for &m in terrain.material_indices.as_slice() {
            assert!(
                usize::from(m) < terrain.materials.len(),
                "{path}: material {m}"
            );
        }
        // Static entities are also placed objects of the same model at the same x/z (their
        // heights differ by centimetres), which pins model indices as 0-based.
        let by_position: HashMap<[u32; 2], &ObjectInstance> = terrain
            .objects
            .iter()
            .map(|o| (xz_key(o.transform.position()), o))
            .collect();
        let mut matched = 0;
        for e in &terrain.entities {
            if let Some(o) = by_position.get(&xz_key(e.position)) {
                assert_eq!(
                    terrain.model_of(o).unwrap().as_str(),
                    VfsPath::new(&e.shape).as_str(),
                    "{path}: entity {}",
                    e.class_name
                );
                matched += 1;
            }
        }
        eprintln!(
            "  {matched} of {} entities match an object",
            terrain.entities.len()
        );
        assert!(
            matched * 10 >= terrain.entities.len() * 9,
            "{path}: entities"
        );
        // Objects stand on the terrain: the heightmap is row-major with rows along z.
        let mut gaps: Vec<f32> = terrain
            .objects
            .iter()
            .step_by(101)
            .map(|o| {
                let p = o.transform.position();
                (terrain.surface_height(p.x, p.z) - p.y).abs()
            })
            .collect();
        if !gaps.is_empty() {
            gaps.sort_by(f32::total_cmp);
            let median = gaps[gaps.len() / 2];
            assert!(median < 5.0, "{path}: median object height gap {median}");
        }
        // Object ids in shipped terrains are 0..=max_object_id.
        if let Some(max) = terrain.objects.iter().map(|o| o.id).max() {
            assert_eq!(max, terrain.max_object_id, "{path}: max object id");
        }
    }
}

fn xz_key(p: Vec3) -> [u32; 2] {
    [p.x.to_bits(), p.z.to_bits()]
}

/// Heap bytes of the big arrays of a terrain.
fn approx_memory(t: &Terrain) -> usize {
    use std::mem::size_of;
    let land = t.land_grid.len();
    let heights = t.heightmap.as_slice().len();
    heights * 4
        + heights * 3 // grass, primary texture, subdivision hints
        + land * (2 + 2 + 4 + 4 + 1) // geography, materials, offsets, persistent
        + t.sound_map.as_slice().len()
        + t.objects.len() * size_of::<a3_wrp::ObjectInstance>()
        + t.map_objects.len() * size_of::<a3_wrp::MapObject>()
}
