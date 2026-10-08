//! ODOL decoding from synthetic files.

mod common;

use a3_p3d::{
    AnimationAxis, AnimationBinding, AnimationTransform, Encoding, Error, LodKind, Model,
    SourceAddress,
};
use common::{Odol, OdolAnim};
use glam::{Mat3, Vec3};

#[test]
fn reads_header_model_info_and_lod_table() {
    let mut file = Odol::new(&[1.0, 2.0, 1e13, 1e15]);
    file.permanent = vec![false, false, true, true];
    let model = Model::from_bytes(&file.build()).unwrap();

    assert_eq!(model.encoding, Encoding::Odol);
    assert_eq!(model.version, 73);
    let kinds: Vec<_> = model.lods.iter().map(|l| l.resolution.kind()).collect();
    assert_eq!(
        kinds,
        [
            LodKind::Resolution(1.0),
            LodKind::Resolution(2.0),
            LodKind::Geometry,
            LodKind::Memory
        ]
    );
    let info = &model.info;
    assert_eq!(info.app_id, 107410);
    assert_eq!(info.muzzle_flash, r"\a3\mf\muzzle");
    assert_eq!(info.bounding_sphere, 2.5);
    assert_eq!(info.bbox_min, Vec3::new(-1.0, -2.0, -3.0));
    assert_eq!(info.bbox_max, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(info.center_of_mass, Vec3::new(0.0, 0.25, 0.0));
    assert_eq!(
        info.inv_inertia,
        Mat3::from_diagonal(Vec3::new(1.0, 2.0, 3.0))
    );
    assert!(info.auto_center && info.can_occlude && !info.ai_covers);
    assert_eq!(info.shadow_offset, f32::MAX);
    assert_eq!(info.map_type, 3);
    assert_eq!(info.mass, 250.0);
    assert_eq!(info.armor, 40.0);
    assert_eq!(info.special_lods.memory, Some(3));
    assert_eq!(info.special_lods.geometry, Some(0));
    assert_eq!(info.special_lods.fire_geometry, None);
    assert_eq!(info.class, "house");
    assert_eq!(info.damage, "building");
    assert_eq!(info.preferred_shadow_volume_lod, [-1; 4]);

    let odol = model.lods[0].odol.as_ref().unwrap();
    assert!(!odol.permanent);
    let summary = odol.summary.unwrap();
    assert_eq!((summary.faces, summary.vertices), (12, 24));
    assert_eq!(summary.face_area, 1.5);
    let memory = model.lods[3].odol.as_ref().unwrap();
    assert!(memory.permanent && memory.summary.is_none());
}

#[test]
fn resolves_skeleton_parents() {
    let mut file = Odol::new(&[1.0]);
    file.skeleton = Some((
        "Door_Skeleton",
        true,
        vec![("door", ""), ("handle", "Door"), ("lights", "zbytek")],
    ));
    let model = Model::from_bytes(&file.build()).unwrap();

    let skeleton = model.skeleton.unwrap();
    assert_eq!(skeleton.name, "Door_Skeleton");
    assert!(skeleton.inherited);
    let bones: Vec<_> = skeleton
        .bones
        .iter()
        .map(|b| (b.name.as_str(), b.parent))
        .collect();
    assert_eq!(
        bones,
        [("door", None), ("handle", Some(0)), ("lights", None)],
        "parent names match case-insensitively; unknown parents leave the bone a root"
    );
    assert_eq!(skeleton.bones[2].parent_name, "zbytek");
}

#[test]
fn reads_animations_with_per_lod_bone_bindings() {
    let mut file = Odol::new(&[1.0, 1e15]);
    file.skeleton = Some(("Skel", false, vec![("door", ""), ("light", "")]));
    file.animations = vec![
        OdolAnim {
            kind: 2,
            name: "door_rot",
            source: "door",
            params: vec![0.0, 1.5],
        },
        OdolAnim {
            kind: 9,
            name: "light_hide",
            source: "lights",
            params: vec![0.5, -1.0],
        },
        OdolAnim {
            kind: 8,
            name: "door_direct",
            source: "door",
            params: vec![0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.75, 0.1],
        },
    ];
    file.bones_to_anims = vec![vec![vec![0, 2], vec![1]], vec![]];
    file.anims_to_bones = vec![
        vec![
            (0, [1.0, 2.0, 3.0, 0.0, 1.0, 0.0]),
            (1, [0.0; 6]),
            (0, [0.0; 6]),
        ],
        vec![(-1, [0.0; 6]), (-1, [0.0; 6]), (-1, [0.0; 6])],
    ];
    let model = Model::from_bytes(&file.build()).unwrap();

    let names: Vec<_> = model.animations.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["door_rot", "light_hide", "door_direct"]);
    let rot = &model.animations[0];
    assert_eq!(rot.source, "door");
    assert_eq!(rot.max_value, 1.0);
    assert_eq!(rot.source_address, SourceAddress::Clamp);
    assert_eq!(
        rot.transform,
        AnimationTransform::Rotation {
            axis: AnimationAxis::Y,
            angle0: 0.0,
            angle1: 1.5
        }
    );
    assert_eq!(
        rot.bindings,
        [
            Some(AnimationBinding {
                bone: 0,
                axis: Some((Vec3::new(1.0, 2.0, 3.0), Vec3::Y))
            }),
            None
        ]
    );
    assert_eq!(
        model.animations[1].bindings[0],
        Some(AnimationBinding {
            bone: 1,
            axis: None
        }),
        "hide animations carry no axis"
    );
    assert_eq!(
        model.animations[2].transform,
        AnimationTransform::Direct {
            axis_pos: Vec3::Y,
            axis_dir: Vec3::Z,
            angle: 0.75,
            axis_offset: 0.1
        }
    );
    assert_eq!(
        model.animations[2].bindings[0].unwrap().axis,
        None,
        "direct animations carry no axis either"
    );
    assert_eq!(model.lods[0].bone_animations, [vec![0, 2], vec![1]]);
    assert!(model.lods[1].bone_animations.is_empty());
}

#[test]
fn reads_animations_without_bone_tables() {
    let mut file = Odol::new(&[1.0, 2.0]);
    file.animations = vec![OdolAnim {
        kind: 4,
        name: "slide",
        source: "time",
        params: vec![0.0, 2.0],
    }];
    let model = Model::from_bytes(&file.build()).unwrap();

    assert_eq!(model.animations[0].bindings, [None, None]);
    assert_eq!(model.lods[1].bone_animations, Vec::<Vec<u32>>::new());
}

#[test]
fn rejects_other_odol_versions() {
    let mut bytes = Odol::new(&[1.0]).build();
    bytes[4] = 72;
    let err = Model::from_bytes(&bytes).unwrap_err();
    assert!(matches!(
        err,
        Error::UnsupportedVersion {
            format: "ODOL",
            version: 72
        }
    ));
}

#[test]
fn rejects_lod_offsets_outside_the_file() {
    let mut file = Odol::new(&[1.0]);
    file.lod_bodies = vec![vec![]];
    let mut bytes = file.build();
    let n = bytes.len();
    // The LOD end offset is the last u32 before the permanent flag.
    bytes[n - 5..n - 1].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = Model::from_bytes(&bytes).unwrap_err();
    assert!(matches!(err, Error::Malformed { .. }), "{err}");
}
