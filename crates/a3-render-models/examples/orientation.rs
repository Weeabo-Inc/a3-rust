//! Which way is model space? Compare each house map symbol's footprint centre (computed by
//! Binarize) with the model's bbox centre placed raw or rotated 180 degrees about Y.
use std::collections::HashMap;

use glam::{Vec2, Vec3};

fn main() {
    let root = std::env::var("A3_ROOT").unwrap();
    let vfs = a3_vfs::Vfs::new();
    vfs.mount_game(std::path::Path::new(&root), &[]);
    let terrain = a3_wrp::Terrain::parse(&vfs.open(r"a3\map_altis\altis.wrp").unwrap()).unwrap();
    let by_id: HashMap<u32, &a3_wrp::ObjectInstance> =
        terrain.objects.iter().map(|o| (o.id, o)).collect();
    let mut centres: HashMap<u32, Option<(Vec3, Vec3)>> = HashMap::new();
    let (mut raw, mut rotated, mut tie) = (0, 0, 0);
    let mut shown = 0;
    for mo in &terrain.map_objects {
        if !matches!(mo.kind, a3_wrp::MapType::House | a3_wrp::MapType::Building) {
            continue;
        }
        let a3_wrp::MapShape::RectColored { corners, .. } = &mo.shape else {
            continue;
        };
        let Some(o) = by_id.get(&mo.object_id) else {
            continue;
        };
        let bounds = centres.entry(o.model_index).or_insert_with(|| {
            let path = terrain.models[o.model_index as usize].as_str().to_owned();
            let m = a3_p3d::Model::from_bytes(&vfs.open(&path).ok()?).ok()?;
            let lod = m.lods.iter().find(|l| l.resolution.is_visual())?;
            let mut min = Vec3::MAX;
            let mut max = Vec3::MIN;
            for p in &lod.vertices.positions {
                min = min.min(*p);
                max = max.max(*p);
            }
            Some((min, max))
        });
        let Some((min, max)) = *bounds else { continue };
        let c = (min + max) * 0.5;
        if Vec2::new(c.x, c.z).length() < 1.0 {
            continue; // centred models cannot tell
        }
        let rect = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
        let t = o.transform.to_affine();
        let a = t.transform_point3(c);
        let b = t.transform_point3(Vec3::new(-c.x, c.y, -c.z));
        let da = (Vec2::new(a.x, a.z) - rect).length();
        let db = (Vec2::new(b.x, b.z) - rect).length();
        if shown < 8 {
            println!(
                "{}: offset {:.2},{:.2} raw err {da:.2} m, rotated err {db:.2} m",
                terrain.models[o.model_index as usize].as_str(),
                c.x,
                c.z
            );
            shown += 1;
        }
        if (da - db).abs() < 0.5 {
            tie += 1;
        } else if da < db {
            raw += 1;
        } else {
            rotated += 1;
        }
    }
    println!("raw closer: {raw}, rotated closer: {rotated}, undecided: {tie}");
}
