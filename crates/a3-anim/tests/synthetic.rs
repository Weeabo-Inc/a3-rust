//! Posing synthetic models: a hinged door with a handle, and a wheel.

use std::f32::consts::{FRAC_PI_2, TAU};

use a3_anim::{Pose, Sources, hidden_sections, interpolate, pose, skin};
use a3_p3d::{
    Animation, AnimationAxis, AnimationBinding, AnimationTransform, Bone, BoneWeights, Encoding,
    Face, Lod, Model, ModelInfo, Section, Skeleton, SourceAddress, Vertices,
};
use glam::Vec3;

fn anim(name: &str, source: &str, transform: AnimationTransform, bone: u32) -> Animation {
    Animation {
        name: name.into(),
        source: source.into(),
        transform,
        min_value: 0.0,
        max_value: 1.0,
        min_phase: 0.0,
        max_phase: 1.0,
        anim_period: 0.0,
        init_phase: 0.0,
        source_address: SourceAddress::Clamp,
        bindings: vec![Some(AnimationBinding {
            bone,
            axis: Some((Vec3::new(1.0, 0.0, 0.0), Vec3::Y)),
        })],
    }
}

fn weights(bone: u8) -> BoneWeights {
    BoneWeights {
        count: 1,
        pairs: [(bone, 255), (0, 0), (0, 0), (0, 0)],
    }
}

/// A door hinged on the vertical line x = 1, z = 0, with a handle bone parented to it.
/// Vertices: 0..4 the door panel (bone 0), 4 the handle (bone 1), 5 the frame (no bone).
fn door_model() -> Model {
    let skeleton = Skeleton {
        name: "Door".into(),
        bones: vec![
            Bone {
                name: "door".into(),
                parent: None,
                parent_name: String::new(),
            },
            Bone {
                name: "handle".into(),
                parent: Some(0),
                parent_name: "door".into(),
            },
        ],
        ..Skeleton::default()
    };
    let positions = vec![
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(3.0, 2.0, 0.0),
        Vec3::new(1.0, 2.0, 0.0),
        Vec3::new(2.8, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ];
    let mut bone_weights = vec![weights(0); 4];
    bone_weights.push(weights(1));
    bone_weights.push(BoneWeights::default());
    let lod = Lod {
        vertices: Vertices {
            normals: vec![Vec3::NEG_Z; positions.len()],
            positions,
            bone_weights,
            ..Vertices::default()
        },
        faces: vec![
            Face::quad(0, 1, 2, 3),
            Face::triangle(4, 4, 4),
            Face::triangle(5, 5, 5),
        ],
        sections: vec![
            Section {
                faces: 0..1,
                ..Section::default()
            },
            Section {
                faces: 1..2,
                ..Section::default()
            },
            Section {
                faces: 2..3,
                ..Section::default()
            },
        ],
        bone_animations: vec![vec![0, 2], vec![1]],
        ..Lod::default()
    };
    let door = anim(
        "door",
        "door",
        AnimationTransform::Rotation {
            axis: AnimationAxis::Custom,
            angle0: 0.0,
            angle1: FRAC_PI_2,
        },
        0,
    );
    let mut handle = anim(
        "handle",
        "handle",
        AnimationTransform::Hide {
            hide_value: 0.5,
            unhide_value: -1.0,
        },
        1,
    );
    handle.bindings[0].as_mut().unwrap().axis = None;
    let mut slide = anim(
        "slide",
        "slide",
        AnimationTransform::Translation {
            axis: AnimationAxis::Custom,
            offset0: 0.0,
            offset1: 1.0,
        },
        0,
    );
    // The custom translation axis vector is 2 m long: offset 1 moves 2 m.
    slide.bindings[0].as_mut().unwrap().axis = Some((Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0)));
    Model {
        encoding: Encoding::Odol,
        version: 73,
        info: ModelInfo::default(),
        skeleton: Some(skeleton),
        animations: vec![door, handle, slide],
        lods: vec![lod],
    }
}

fn posed(model: &Model, sources: &Sources) -> Vec<Vec3> {
    let p = pose(model, 0, sources);
    skin(&model.lods[0], &p.skinning(&model.lods[0])).positions
}

fn assert_near(a: Vec3, b: Vec3) {
    assert!(a.abs_diff_eq(b, 1e-5), "{a} != {b}");
}

#[test]
fn rest_pose_leaves_vertices_in_place() {
    let model = door_model();
    let positions = posed(&model, &Sources::new());
    assert_eq!(positions, model.lods[0].vertices.positions);
}

#[test]
fn opens_a_door_about_its_hinge() {
    let model = door_model();
    let positions = posed(&model, &Sources::new().with("door", 1.0));
    // +90 degrees about +Y through (1, 0, 0): x offset 2 -> z offset -2.
    assert_near(positions[1], Vec3::new(1.0, 0.0, -2.0));
    assert_near(positions[0], Vec3::new(1.0, 0.0, 0.0));
    // The handle (child bone) follows the door.
    assert_near(positions[4], Vec3::new(1.0, 1.0, -1.8));
    // The frame has no bone and stays.
    assert_near(positions[5], Vec3::ZERO);
    // Half open: 45 degrees.
    let half = posed(&model, &Sources::new().with("door", 0.5));
    let s = std::f32::consts::FRAC_1_SQRT_2 * 2.0;
    assert_near(half[1], Vec3::new(1.0 + s, 0.0, -s));
}

#[test]
fn composes_animations_on_one_bone_in_list_order() {
    let model = door_model();
    // Rotate first (about the hinge), then slide up 2 m.
    let positions = posed(&model, &Sources::new().with("door", 1.0).with("slide", 1.0));
    assert_near(positions[1], Vec3::new(1.0, 2.0, -2.0));
}

#[test]
fn hides_a_bone_and_its_sections() {
    let model = door_model();
    let lod = &model.lods[0];
    let p = pose(&model, 0, &Sources::new().with("handle", 0.5));
    assert_eq!(p.hidden, [false, true]);
    let skinning = p.skinning(lod);
    assert_eq!(hidden_sections(lod, &skinning), [false, true, false]);
    let shown = pose(&model, 0, &Sources::new().with("handle", 0.49));
    assert_eq!(shown.hidden, [false, false]);
}

#[test]
fn parent_hide_hides_children() {
    let mut model = door_model();
    model.animations[1].bindings[0].as_mut().unwrap().bone = 0;
    model.lods[0].bone_animations = vec![vec![0, 1], vec![]];
    let p = pose(&model, 0, &Sources::new().with("handle", 1.0));
    assert_eq!(p.hidden, [true, true]);
}

#[test]
fn maps_source_values_like_the_engine() {
    let mut a = door_model().animations[0].clone();
    a.min_value = -1.0;
    a.max_value = 2.0;
    a.min_phase = 0.0;
    a.max_phase = 1.0;
    // Clamp to min/maxValue, interpolate over min/maxPhase, hold outside.
    assert_eq!(interpolate(&a, 0.25, 0.0, 4.0), 1.0);
    assert_eq!(interpolate(&a, 1.5, 0.0, 4.0), 4.0);
    assert_eq!(interpolate(&a, -0.5, 0.0, 4.0), 0.0);
    a.source_address = SourceAddress::Loop;
    assert_eq!(interpolate(&a, 1.25, 0.0, 4.0), 1.0);
    assert_eq!(interpolate(&a, -0.75, 0.0, 4.0), 1.0);
    a.source_address = SourceAddress::Mirror;
    assert_eq!(interpolate(&a, 1.25, 0.0, 4.0), 3.0);
    assert_eq!(interpolate(&a, 0.25, 0.0, 4.0), 1.0);
    // Clamping to min/maxValue happens after the wrap.
    a.max_value = 0.5;
    assert_eq!(interpolate(&a, 1.25, 0.0, 4.0), 2.0);
}

/// A wheel of radius 1 around the X axis through (0, 1, 0), as `rotationX` with
/// `angle1 = -2 pi` over the `wheel` source.
fn wheel_model() -> Model {
    let mut model = door_model();
    let mut wheel = anim(
        "wheel",
        "wheel",
        AnimationTransform::Rotation {
            axis: AnimationAxis::X,
            angle0: 0.0,
            angle1: -TAU,
        },
        0,
    );
    wheel.bindings[0].as_mut().unwrap().axis = Some((Vec3::new(0.0, 1.0, 0.0), Vec3::ZERO));
    wheel.source_address = SourceAddress::Loop;
    model.animations = vec![wheel];
    model.lods[0].bone_animations = vec![vec![0], vec![]];
    model.lods[0].vertices.positions[1] = Vec3::new(0.0, 2.0, 0.0); // top of the wheel
    model
}

#[test]
fn turns_a_wheel_with_the_engines_axis_sign() {
    let model = wheel_model();
    // A quarter turn: angle -pi/2, applied by the engine as +pi/2 about +X,
    // which takes the top of the wheel (0, 1, 0 from the hub) to (0, 0, 1).
    let positions = posed(&model, &Sources::new().with("wheel", 0.25));
    assert_near(positions[1], Vec3::new(0.0, 1.0, 1.0));
    // Looping source: 1.25 turns equals a quarter turn.
    let looped = posed(&model, &Sources::new().with("wheel", 1.25));
    assert_near(looped[1], Vec3::new(0.0, 1.0, 1.0));
    let half = posed(&model, &Sources::new().with("wheel", 0.5));
    assert_near(half[1], Vec3::new(0.0, 0.0, 0.0));
}

#[test]
fn identity_pose_skins_nothing() {
    let model = door_model();
    let skinning = Pose::identity(2).skinning(&model.lods[0]);
    assert_eq!(skinning.len(), 2);
    let out = skin(&model.lods[0], &skinning);
    assert_eq!(out.normals, [Vec3::NEG_Z; 6]);
}
