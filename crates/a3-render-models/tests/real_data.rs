//! Real game data: every model placed on Altis prepares for rendering and its textures resolve.
//! Skips without `A3_ROOT`.

use std::collections::BTreeMap;
use std::path::Path;

use a3_paa::Procedural;
use a3_render_models::{PreparedModel, ShaderFamily, Slot};

#[test]
fn altis_models_prepare_and_their_textures_exist() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let vfs = a3_vfs::Vfs::new();
    vfs.mount_game(Path::new(&root), &[]);
    let terrain = a3_wrp::Terrain::parse(&vfs.open(r"a3\map_altis\altis.wrp").unwrap()).unwrap();

    let mut families: BTreeMap<String, usize> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut sections = 0;
    let mut invisible = 0;
    for path in &terrain.models {
        let bytes = vfs.open(path.as_str()).expect("placed model exists");
        let model = a3_p3d::Model::from_bytes(&bytes).expect("placed model decodes");
        let prepared = PreparedModel::new(&model);
        // Invisible helpers (bridge path LODs, ...) have resolution LODs without faces.
        let faces: u32 = prepared.lods.iter().map(|l| l.faces).sum();
        if faces == 0 {
            invisible += 1;
            continue;
        }
        assert!(prepared.radius > 0.0, "{path} has no extent");
        for lod in &prepared.lods {
            assert_eq!(lod.indices.len() % 3, 0);
            assert!(
                lod.indices
                    .iter()
                    .all(|&i| (i as usize) < lod.vertices.len())
            );
            for section in &lod.sections {
                sections += 1;
                let m = &section.material;
                *families.entry(format!("{:?}", m.family)).or_default() += 1;
                assert_ne!(m.family, ShaderFamily::Unsupported);
                assert!(m.texture(Slot::Diffuse).is_some());
                for (_, t) in m.textures() {
                    if !Procedural::is_procedural(&t.path) && !vfs.exists(&t.path) {
                        missing.push(t.path.clone());
                    }
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    eprintln!(
        "{} models ({invisible} without faces), {sections} sections by family {families:?}, \
         missing textures {missing:?}",
        terrain.models.len()
    );
    assert!(invisible < 20, "{invisible} models draw nothing");
    assert!(missing.len() < 10, "missing textures: {missing:?}");
}

/// B_Soldier_F's model (the class inherits the BLUFOR soldier).
const SOLDIER: &str = r"a3\characters_f\blufor\b_soldier_01.p3d";

/// Every vertex a soldier's visual LODs draw is weighted to its Skeleton, with the influence
/// weights summing to 255 (the ODOL weight byte range). The soldier has no unweighted rigid
/// parts, so a skinned draw never falls back to the rest pose for its visible geometry; other
/// models do (a helicopter hull or a parachute canopy stays in the rest pose).
#[test]
fn drawn_vertices_of_a_soldier_are_all_weighted() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let vfs = a3_vfs::Vfs::new();
    vfs.mount_game(Path::new(&root), &[]);
    let model = a3_p3d::Model::from_bytes(&vfs.open(SOLDIER).expect("the soldier model opens"))
        .expect("the soldier model decodes");
    assert!(model.skeleton.is_some(), "a soldier is skinned");

    let mut skinned = 0;
    for lod in &model.lods {
        if !lod.resolution.is_visual() {
            continue;
        }
        let weights = &lod.vertices.bone_weights;
        if weights.is_empty() {
            continue;
        }
        skinned += 1;
        for section in &lod.sections {
            // Proxy triangles are placeholders for the LOD's proxies, not geometry.
            if section.is_proxy() {
                continue;
            }
            for i in lod.section_triangles(section) {
                let w = weights.get(i as usize).unwrap_or_else(|| {
                    panic!(
                        "lod {} draws vertex {i} without bone weights",
                        lod.resolution.0
                    )
                });
                assert!(
                    w.count > 0,
                    "lod {} draws the unweighted vertex {i}",
                    lod.resolution.0
                );
                let sum: u32 = w.pairs[..w.count.min(4) as usize]
                    .iter()
                    .map(|&(_, weight)| u32::from(weight))
                    .sum();
                assert_eq!(sum, 255, "lod {} vertex {i} weighs {sum}", lod.resolution.0);
            }
        }
    }
    assert!(
        skinned >= 4,
        "the soldier's visual LODs are skinned: {skinned}"
    );
}
