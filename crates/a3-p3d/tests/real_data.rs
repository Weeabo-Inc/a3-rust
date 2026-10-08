//! Parses every `.p3d` in a real game install. Skipped when `A3_ROOT` is unset.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use a3_p3d::{Encoding, LodKind, Model};
use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn parses_every_model_in_the_install() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));
    let paths = vfs.glob(r"**\*.p3d");
    assert!(!paths.is_empty(), "no .p3d files found");

    let start = Instant::now();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut lods = 0;
    let mut bytes = 0;
    for path in &paths {
        let data = vfs.open(path.as_str()).unwrap();
        bytes += data.len();
        let signature = String::from_utf8_lossy(&data[..4.min(data.len())]).into_owned();
        let version = data
            .get(4..8)
            .map_or(0, |v| u32::from_le_bytes(v.try_into().unwrap()));
        *versions
            .entry(format!("{signature} v{version}"))
            .or_default() += 1;
        match Model::from_bytes(&data) {
            Ok(model) => {
                assert_eq!(model.encoding, Encoding::Odol);
                lods += model.lods.len();
                for problem in special_lod_mismatches(&model) {
                    failures
                        .entry("special LOD index".into())
                        .or_default()
                        .push(format!("{path}: {problem}"));
                }
            }
            Err(e) => {
                let reason = e.to_string();
                let key = reason.split(':').next().unwrap_or(&reason).to_owned();
                failures
                    .entry(key)
                    .or_default()
                    .push(format!("{path}: {reason}"));
            }
        }
    }
    let elapsed = start.elapsed();

    let failed: usize = failures.values().map(Vec::len).sum();
    eprintln!("versions: {versions:?}");
    eprintln!(
        "{} models ({:.1} MB), {} parsed, {failed} failed, {lods} LODs, in {elapsed:.2?}",
        paths.len(),
        bytes as f64 / 1e6,
        paths.len() - failed
    );
    for (reason, files) in &failures {
        eprintln!("{} x {reason}", files.len());
        for f in files.iter().take(3) {
            eprintln!("    {f}");
        }
    }
    assert_eq!(failed, 0);
}

/// Every special LOD index in ModelInfo must point at a LOD of the matching kind. Fire geometry
/// falls back to the View Geometry and then the Geometry; View Geometry to the Geometry.
fn special_lod_mismatches(model: &Model) -> Vec<String> {
    let s = model.info.special_lods;
    let expected = [
        (s.memory, vec![LodKind::Memory]),
        (s.geometry, vec![LodKind::Geometry]),
        (s.geometry_physx, vec![LodKind::GeometryPhysx]),
        (
            s.fire_geometry,
            vec![
                LodKind::FireGeometry,
                LodKind::ViewGeometry,
                LodKind::Geometry,
            ],
        ),
        (
            s.view_geometry,
            vec![LodKind::ViewGeometry, LodKind::Geometry],
        ),
        (s.land_contact, vec![LodKind::LandContact]),
        (s.roadway, vec![LodKind::Roadway]),
        (s.paths, vec![LodKind::Paths]),
        (s.hitpoints, vec![LodKind::HitPoints]),
    ];
    let mut out = Vec::new();
    for (index, kinds) in expected {
        let Some(i) = index else { continue };
        let found = model.lods.get(usize::from(i)).map(|l| l.resolution.kind());
        if !found.is_some_and(|k| kinds.contains(&k)) {
            out.push(format!("LOD {i} is {found:?}, expected one of {kinds:?}"));
        }
    }
    out
}
