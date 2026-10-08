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
    let mut stats: BTreeMap<&'static str, usize> = BTreeMap::new();
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
                geometry_stats(&model, &mut stats);
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
    eprintln!("geometry: {stats:#?}");
    for bad in [
        "positions outside LOD bbox",
        "normals other",
        "sections with gaps",
        "proxy orientation other",
        "proxy selection other",
        "weights single 0",
        "weights single other",
        "weights multi sum 254..256",
        "weights multi other",
    ] {
        assert_eq!(stats.get(bad).copied().unwrap_or(0), 0, "{bad}");
    }
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

/// Counts of geometry sanity properties over every LOD.
fn geometry_stats(model: &Model, stats: &mut BTreeMap<&'static str, usize>) {
    for lod in &model.lods {
        let odol = lod.odol.as_ref().unwrap();
        let v = &lod.vertices;
        *stats.entry("vertices").or_default() += v.len();
        *stats.entry("faces").or_default() += lod.faces.len();
        let eps = glam::Vec3::splat(1e-3);
        let outside = v
            .positions
            .iter()
            .filter(|p| p.cmplt(odol.bbox_min - eps).any() || p.cmpgt(odol.bbox_max + eps).any())
            .count();
        *stats.entry("positions outside LOD bbox").or_default() += outside;
        for n in &v.normals {
            let len = n.length();
            let key = if len == 0.0 {
                "normals zero"
            } else if (len - 1.0).abs() < 0.01 {
                "normals unit"
            } else {
                "normals other"
            };
            *stats.entry(key).or_default() += 1;
        }
        for w in &v.bone_weights {
            let sum: u32 = w.pairs[..w.count.min(4) as usize]
                .iter()
                .map(|p| u32::from(p.1))
                .sum();
            let key = match (w.count, sum) {
                (0, _) => "weights count 0",
                (1, 255) => "weights single 255",
                (1, 0) => "weights single 0",
                (1, _) => "weights single other",
                (_, 255) => "weights multi sum 255",
                (_, 254..=256) => "weights multi sum 254..256",
                _ => "weights multi other",
            };
            *stats.entry(key).or_default() += 1;
        }
        let mut next = 0;
        let mut contiguous = true;
        for s in &lod.sections {
            contiguous &= s.faces.start == next;
            next = s.faces.end;
        }
        if contiguous && next as usize == lod.faces.len() {
            *stats.entry("sections cover all faces").or_default() += 1;
        } else {
            *stats.entry("sections with gaps").or_default() += 1;
        }
        for p in &lod.proxies {
            let det = p.orientation.determinant();
            let key = if (det.abs() - 1.0).abs() < 0.01 {
                "proxy orientation det ±1"
            } else {
                "proxy orientation other"
            };
            *stats.entry(key).or_default() += 1;
            let named = usize::try_from(p.named_selection)
                .ok()
                .and_then(|i| lod.named_selections.get(i))
                .is_some_and(|sel| {
                    let model = p.model.trim_start_matches('\\').to_ascii_lowercase();
                    let name = sel.name.to_ascii_lowercase();
                    name.starts_with("proxy:") && name.contains(&model)
                });
            let key = if named {
                "proxy selection names its model"
            } else {
                "proxy selection other"
            };
            *stats.entry(key).or_default() += 1;
        }
    }
}
