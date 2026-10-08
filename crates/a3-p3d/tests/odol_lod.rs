//! ODOL LOD geometry decoding from synthetic files.

mod common;

use a3_p3d::{BoneWeights, Error, Face, Model};
use common::{Odol, empty_lod_body, sample_lod_body};
use glam::{Mat3, Vec2, Vec3};

fn sample_model() -> Model {
    let mut file = Odol::new(&[1.0, 1e15]);
    file.lod_bodies = vec![sample_lod_body(), empty_lod_body()];
    Model::from_bytes(&file.build()).unwrap()
}

#[test]
fn decodes_vertices_faces_and_render_triangles() {
    let model = sample_model();
    let lod = &model.lods[0];

    assert_eq!(
        lod.vertices.positions,
        [
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::Y,
            Vec3::new(2.0, 0.0, 0.0)
        ],
        "LZO-compressed positions"
    );
    assert_eq!(lod.vertices.normals, [Vec3::NEG_Z; 5], "filled normals");
    assert_eq!(
        lod.vertices.uv_sets,
        [vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.5, 0.5)
        ]]
    );
    assert_eq!(lod.vertices.flags, [0; 5]);
    assert_eq!(
        lod.vertices.bone_weights[4],
        BoneWeights {
            count: 1,
            pairs: [(1, 255), (0, 0), (0, 0), (0, 0)]
        }
    );
    assert_eq!(lod.faces, [Face::quad(0, 1, 2, 3), Face::triangle(1, 4, 2)]);
    assert_eq!(lod.triangles(), [0, 1, 2, 0, 2, 3, 1, 4, 2]);
}

#[test]
fn converts_section_byte_offsets_to_face_ranges() {
    let model = sample_model();
    let lod = &model.lods[0];

    assert_eq!(lod.sections.len(), 2);
    let (first, second) = (&lod.sections[0], &lod.sections[1]);
    assert_eq!(first.faces, 0..1);
    assert_eq!(first.texture, Some(0));
    assert_eq!(first.material, Some(0));
    assert_eq!(second.faces, 1..2);
    assert_eq!(second.texture, Some(1));
    assert_eq!(second.material, None);
    assert_eq!(second.flags, 0x40);
    assert_eq!(lod.section_triangles(second), [1, 4, 2]);
    let odol = first.odol.as_ref().unwrap();
    assert_eq!(odol.bone_count, 2);
    let collimator = odol.collimator.expect("section 0 stores a collimator");
    assert_eq!(collimator.origin, Vec3::new(0.0, 0.05, 0.0));
    assert_eq!(collimator.axis_a, Vec3::Z);
    assert_eq!(collimator.size_b, 0.04);
    assert_eq!(second.odol.as_ref().unwrap().collimator, None);
}

#[test]
fn decodes_textures_materials_selections_properties_and_proxies() {
    let model = sample_model();
    let lod = &model.lods[0];

    assert_eq!(lod.textures, [r"a3\data\a_co.paa", r"a3\data\b_co.paa"]);
    let material = &lod.materials[0];
    assert_eq!(material.name, r"a3\data\a.rvmat");
    assert_eq!(material.specular_power, 40.0);
    assert_eq!((material.pixel_shader, material.vertex_shader), (7, 3));
    assert_eq!(material.surface, r"a3\data\a.bisurf");
    let stages: Vec<_> = material.stages.iter().map(|s| s.texture.as_str()).collect();
    assert_eq!(stages, [r"a3\data\a_nohq.paa", r"a3\data\a_smdi.paa"]);
    assert!(material.stages[1].use_world_env_map);
    assert_eq!(material.tex_gens[0].uv_source, 1);
    assert_eq!(material.ti_stage.as_ref().unwrap().texture, "");

    let door = &lod.named_selections[0];
    assert_eq!(door.name, "door");
    assert_eq!(door.faces, [1]);
    assert_eq!(door.vertices, [1, 4, 2]);
    assert!(door.sectional);
    assert_eq!(door.sections, [1]);
    assert_eq!(
        lod.properties,
        [("lodnoshadow".to_string(), "1".to_string())]
    );

    let proxy = &lod.proxies[0];
    assert_eq!(proxy.model, r"\a3\proxies\seat");
    assert_eq!(proxy.orientation, Mat3::IDENTITY);
    assert_eq!(proxy.position, Vec3::new(0.5, 0.5, 0.0));
    assert_eq!(proxy.named_selection, 1);

    let odol = lod.odol.as_ref().unwrap();
    assert_eq!(odol.sub_skeleton, [0, 1]);
    assert_eq!(odol.bbox_max, Vec3::new(2.0, 1.0, 0.0));
    assert_eq!(odol.face_area, 1.5);
}

#[test]
fn reads_lods_with_no_geometry() {
    let model = sample_model();
    let memory = &model.lods[1];
    assert!(memory.vertices.is_empty());
    assert!(memory.faces.is_empty());
}

#[test]
fn rejects_a_lod_that_ends_before_its_table_offset() {
    let mut file = Odol::new(&[1.0]);
    let mut body = empty_lod_body();
    body.push(0);
    file.lod_bodies = vec![body];
    let err = Model::from_bytes(&file.build()).unwrap_err();
    assert!(matches!(err, Error::Malformed { .. }), "{err}");
}

#[test]
fn rejects_unknown_compressed_array_flags() {
    let mut file = Odol::new(&[1.0]);
    let mut body = sample_lod_body();
    // The first compressed array with data is the "door" face list: count 1, then the flag.
    let door = body.windows(5).position(|w| w == b"door\0").unwrap();
    body[door + 5 + 4] = 7;
    file.lod_bodies = vec![body];
    let err = Model::from_bytes(&file.build()).unwrap_err();
    assert!(err.to_string().contains("flag 7"), "{err}");
}
