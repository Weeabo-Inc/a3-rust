//! MLOD decoding from synthetic files.

mod common;

use a3_p3d::{Encoding, Error, Face, LodKind, Model};
use common::{MlodFace, MlodLod, mlod};
use glam::{Vec2, Vec3};

fn triangle_lod(resolution: f32) -> MlodLod {
    MlodLod {
        resolution,
        points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, -1.0]],
        faces: vec![MlodFace {
            corners: vec![(0, 0, 0.0, 0.0), (1, 0, 1.0, 0.0), (2, 0, 0.0, 1.0)],
            flags: 0,
            texture: r"a3\data\box_co.paa",
            material: r"a3\data\box.rvmat",
        }],
        tags: vec![],
    }
}

#[test]
fn reads_a_single_textured_triangle() {
    let model = Model::from_bytes(&mlod(&[triangle_lod(1.0)])).unwrap();

    assert_eq!(model.encoding, Encoding::Mlod);
    assert_eq!(model.version, 257);
    assert_eq!(model.lods.len(), 1);
    let lod = &model.lods[0];
    assert_eq!(lod.resolution.kind(), LodKind::Resolution(1.0));
    assert_eq!(
        lod.vertices.positions,
        [Vec3::ZERO, Vec3::X, Vec3::Y],
        "one vertex per distinct corner"
    );
    assert_eq!(lod.vertices.normals, [Vec3::NEG_Z; 3]);
    assert_eq!(
        lod.vertices.uv_sets,
        [vec![Vec2::ZERO, Vec2::X, Vec2::Y]],
        "UV set 0 from the face corners"
    );
    assert_eq!(lod.faces, [Face::triangle(0, 1, 2)]);
    assert_eq!(lod.textures, [r"a3\data\box_co.paa"]);
    assert_eq!(lod.materials.len(), 1);
    assert_eq!(lod.materials[0].name, r"a3\data\box.rvmat");
    assert_eq!(lod.sections.len(), 1);
    assert_eq!(lod.sections[0].faces, 0..1);
    assert_eq!(lod.sections[0].texture, Some(0));
    assert_eq!(lod.sections[0].material, Some(0));
    assert_eq!(model.info.bbox_min, Vec3::ZERO);
    assert_eq!(model.info.bbox_max, Vec3::new(1.0, 1.0, 0.0));
}

#[test]
fn splits_points_with_different_normals_or_uvs_into_separate_vertices() {
    let mut lod = triangle_lod(1.0);
    lod.normals.push([0.0, 0.0, 1.0]);
    lod.points.push([1.0, 1.0, 0.0]);
    // Second face shares points 1 and 2, but with another normal: four new vertices.
    lod.faces.push(MlodFace {
        corners: vec![(1, 1, 1.0, 0.0), (3, 1, 1.0, 1.0), (2, 1, 0.0, 1.0)],
        flags: 0,
        texture: r"a3\data\box_co.paa",
        material: r"a3\data\box.rvmat",
    });
    let model = Model::from_bytes(&mlod(&[lod])).unwrap();
    let lod = &model.lods[0];

    assert_eq!(lod.vertices.len(), 6);
    assert_eq!(lod.vertex_to_point, [0, 1, 2, 1, 3, 2]);
    assert_eq!(lod.faces[1], Face::triangle(3, 4, 5));
    assert_eq!(
        lod.sections.len(),
        1,
        "same texture and material: one section"
    );
    assert_eq!(lod.sections[0].faces, 0..2);
}

#[test]
fn groups_faces_into_sections_by_texture_material_and_flags() {
    let mut lod = triangle_lod(1.0);
    let face = |texture| MlodFace {
        corners: vec![(0, 0, 0.0, 0.0), (1, 0, 1.0, 0.0), (2, 0, 0.0, 1.0)],
        flags: 0,
        texture,
        material: r"a3\data\box.rvmat",
    };
    // Textures A, B, A: the two A faces end up adjacent in one section.
    lod.faces = vec![face("a_co.paa"), face("b_co.paa"), face("a_co.paa")];
    // Selection "door" holds face 1 (texture B), which moves to index 2.
    lod.tags.push(("door", vec![0, 0, 0, 0, 1, 0]));
    let model = Model::from_bytes(&mlod(&[lod])).unwrap();
    let lod = &model.lods[0];

    assert_eq!(lod.textures, ["a_co.paa", "b_co.paa"]);
    let sections: Vec<_> = lod
        .sections
        .iter()
        .map(|s| (s.faces.clone(), s.texture))
        .collect();
    assert_eq!(sections, [(0..2, Some(0)), (2..3, Some(1))]);
    assert_eq!(lod.named_selections[0].name, "door");
    assert_eq!(lod.named_selections[0].faces, [2]);
}

#[test]
fn reads_tags_selections_properties_mass_and_extra_uv_sets() {
    let mut lod = triangle_lod(1e13);
    let mut uv1 = Vec::new();
    uv1.extend_from_slice(&1u32.to_le_bytes());
    for (u, v) in [(0.5f32, 0.5f32), (0.25, 0.5), (0.5, 0.25)] {
        uv1.extend_from_slice(&u.to_le_bytes());
        uv1.extend_from_slice(&v.to_le_bytes());
    }
    let mut prop = vec![0u8; 128];
    prop[..5].copy_from_slice(b"class");
    prop[64..72].copy_from_slice(b"building");
    let mass: Vec<u8> = [1.0f32, 2.0, 3.0]
        .iter()
        .flat_map(|m| m.to_le_bytes())
        .collect();
    let edges: Vec<u8> = [0u32, 1].iter().flat_map(|i| i.to_le_bytes()).collect();
    lod.tags = vec![
        ("#UVSet#", uv1),
        ("#Property#", prop),
        ("#Mass#", mass),
        ("#SharpEdges#", edges),
        // Points 0 and 2 (point 2 with a partial weight byte), face 0.
        ("hatch", vec![1, 0, 128, 1]),
    ];
    let model = Model::from_bytes(&mlod(&[lod])).unwrap();
    let lod = &model.lods[0];

    assert_eq!(lod.resolution.kind(), LodKind::Geometry);
    assert_eq!(lod.vertices.uv_sets.len(), 2);
    assert_eq!(
        lod.vertices.uv_sets[1],
        [
            Vec2::new(0.5, 0.5),
            Vec2::new(0.25, 0.5),
            Vec2::new(0.5, 0.25)
        ]
    );
    assert_eq!(lod.properties, [("class".into(), "building".into())]);
    assert_eq!(lod.point_masses, [1.0, 2.0, 3.0]);
    assert_eq!(lod.sharp_edges, [[0, 1]]);
    let hatch = &lod.named_selections[0];
    assert_eq!(hatch.name, "hatch");
    assert_eq!(hatch.vertices, [0, 2]);
    assert_eq!(hatch.weights, [1, 128]);
    assert_eq!(hatch.faces, [0]);
}

#[test]
fn reads_several_lods_in_file_order() {
    let model = Model::from_bytes(&mlod(&[triangle_lod(1.0), triangle_lod(1e15)])).unwrap();
    let kinds: Vec<_> = model.lods.iter().map(|l| l.resolution.kind()).collect();
    assert_eq!(kinds, [LodKind::Resolution(1.0), LodKind::Memory]);
}

#[test]
fn rejects_truncated_files() {
    let bytes = mlod(&[triangle_lod(1.0)]);
    let err = Model::from_bytes(&bytes[..bytes.len() - 2]).unwrap_err();
    assert!(matches!(err, Error::Truncated { .. }), "{err}");
}

#[test]
fn rejects_unknown_signatures() {
    let err = Model::from_bytes(b"P3DX\0\0\0\0").unwrap_err();
    assert!(matches!(err, Error::UnknownSignature(s) if &s == b"P3DX"));
}
