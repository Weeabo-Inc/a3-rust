//! RTM bone frames on a synthetic two-bone arm.

use std::f32::consts::FRAC_PI_2;

use a3_anim::{Pose, RtmBinding, SkeletonPivots, blend};
use a3_p3d::{
    Bone, Encoding, Lod, LodResolution, Model, ModelInfo, NamedSelection, Skeleton, Vertices,
};
use a3_rtm::{Animation as Rtm, BoneTransform, Frame};
use glam::{Affine3A, Quat, Vec3};

fn skeleton() -> Skeleton {
    Skeleton {
        name: "Arm".into(),
        bones: vec![
            Bone {
                name: "upper".into(),
                parent: None,
                parent_name: String::new(),
            },
            Bone {
                name: "lower".into(),
                parent: Some(0),
                parent_name: "upper".into(),
            },
            Bone {
                name: "tip".into(),
                parent: Some(1),
                parent_name: "lower".into(),
            },
        ],
        ..Skeleton::default()
    }
}

/// A pivots model whose Memory LOD has points `upper` at the origin and `lower` at (1, 0, 0);
/// `tip` has none and inherits from its parent.
fn pivots_model() -> Model {
    let memory = Lod {
        resolution: LodResolution(1e15),
        vertices: Vertices {
            positions: vec![Vec3::ZERO, Vec3::X],
            ..Vertices::default()
        },
        named_selections: vec![
            NamedSelection {
                name: "Upper".into(),
                vertices: vec![0],
                ..NamedSelection::default()
            },
            NamedSelection {
                name: "lower".into(),
                vertices: vec![1],
                ..NamedSelection::default()
            },
        ],
        ..Lod::default()
    };
    Model {
        encoding: Encoding::Odol,
        version: 73,
        info: ModelInfo::default(),
        skeleton: None,
        animations: vec![],
        lods: vec![memory],
    }
}

fn rtm(frames: Vec<(f32, Vec<BoneTransform>)>) -> Rtm {
    Rtm {
        encoding: a3_rtm::Encoding::Plain,
        step: Vec3::ZERO,
        bones: vec!["LOWER".into(), "upper".into()],
        frames: frames
            .into_iter()
            .map(|(phase, transforms)| Frame { phase, transforms })
            .collect(),
        keystones: vec![],
        extra_names: vec![],
    }
}

#[test]
fn reads_pivots_with_parent_fallback() {
    let pivots = SkeletonPivots::from_model(&skeleton(), &pivots_model(), "");
    assert_eq!(pivots.positions, [Vec3::ZERO, Vec3::X, Vec3::X]);
    assert_eq!(pivots.weapon_bone, None);
    let with_weapon = SkeletonPivots::from_model(&skeleton(), &pivots_model(), "TIP");
    assert_eq!(with_weapon.weapon_bone, Some(2));
}

#[test]
fn binds_rtm_bones_by_name_ignoring_case() {
    let anim = rtm(vec![]);
    let binding = RtmBinding::new(&skeleton(), &anim);
    assert_eq!(binding.rtm_bone, [Some(1), Some(0), None]);
    assert_eq!(binding.bound(), 2);
}

#[test]
fn builds_engine_frames_with_conjugate_rotation_and_pivot_translation() {
    let q = Quat::from_rotation_z(FRAC_PI_2);
    let t = Vec3::new(0.0, 0.5, 0.0);
    let anim = rtm(vec![(
        0.0,
        vec![
            BoneTransform {
                rotation: q,
                translation: t,
            },
            BoneTransform::IDENTITY,
        ],
    )]);
    let pivots = SkeletonPivots::from_model(&skeleton(), &pivots_model(), "");
    let frames = RtmBinding::new(&skeleton(), &anim).frames(&anim, 0.0, &pivots);
    // Bone `lower` (pivot (1,0,0)): rotation is the conjugate (-90 degrees about Z), and the
    // translation becomes R * pivot + t = (0, -1, 0) + (0, 0.5, 0).
    let expected = Affine3A::from_rotation_translation(q.conjugate(), Vec3::new(0.0, -0.5, 0.0));
    assert!(frames[1].abs_diff_eq(expected, 1e-6), "{:?}", frames[1]);
    assert_eq!(frames[0], Affine3A::IDENTITY);
    assert_eq!(frames[2], Affine3A::IDENTITY, "unbound bones stay put");
}

#[test]
fn blends_keyframes_matrix_by_matrix() {
    let shift = |x: f32| BoneTransform {
        rotation: Quat::IDENTITY,
        translation: Vec3::new(x, 0.0, 0.0),
    };
    let anim = rtm(vec![
        (0.0, vec![shift(0.0), shift(0.0)]),
        (1.0, vec![shift(2.0), shift(0.0)]),
    ]);
    let pivots = SkeletonPivots::from_model(&skeleton(), &pivots_model(), "");
    let binding = RtmBinding::new(&skeleton(), &anim);
    let mid = binding.frames(&anim, 0.25, &pivots);
    assert!(
        mid[1]
            .translation
            .abs_diff_eq(Vec3::new(1.5, 0.0, 0.0).into(), 1e-6)
    );
    // Clamped outside the keyframes.
    let after = binding.frames(&anim, 2.0, &pivots);
    assert!(
        after[1]
            .translation
            .abs_diff_eq(Vec3::new(3.0, 0.0, 0.0).into(), 1e-6)
    );

    let a = [Affine3A::from_translation(Vec3::X)];
    let b = [Affine3A::from_translation(Vec3::Y)];
    let half = blend(&a, &b, 0.5);
    assert!(
        half[0]
            .translation
            .abs_diff_eq(Vec3::new(0.5, 0.5, 0.0).into(), 1e-6)
    );
}

#[test]
fn rtm_pose_composes_local_frames_down_the_chain_in_mirrored_space() {
    let pivots = SkeletonPivots::from_model(&skeleton(), &pivots_model(), "");
    assert_eq!(pivots.parents, [None, Some(0), Some(1)]);
    // In RTM space `upper` turns +90 degrees about Z at the origin; `lower` has an identity
    // local transform (frame = [I | pivot]), so it follows its parent.
    let upper = Affine3A::from_quat(Quat::from_rotation_z(FRAC_PI_2));
    let lower = Affine3A::from_translation(Vec3::X);
    // `tip` inherits the pivot (1, 0, 0) from `lower`; its identity local transform is that.
    let frames = [upper, lower, lower];
    let pose = Pose::from_rtm_frames(&frames, &pivots, Vec3::ZERO);
    // Model space is RTM space mirrored in X: model (-2, 0, 0) is RTM (2, 0, 0), which turns
    // to RTM (0, 2, 0), model (0, 2, 0).
    let p = pose.bones[1].transform_point3(Vec3::new(-2.0, 0.0, 0.0));
    assert!(p.abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-6), "{p}");
    // The tip (identity local transform) follows too.
    let tip = pose.bones[2].transform_point3(Vec3::new(-2.0, 0.0, 0.0));
    assert!(tip.abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-6), "{tip}");

    // Model space offset from pivot space by +1 in Y.
    let shifted = Pose::from_rtm_frames(&frames, &pivots, Vec3::Y);
    let q = shifted.bones[1].transform_point3(Vec3::new(-2.0, -1.0, 0.0));
    assert!(q.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6), "{q}");

    // A config animation applies first, in the rest pose; the RTM then moves the result.
    let mut config = Pose::identity(3);
    config.bones[1] = Affine3A::from_translation(Vec3::Z);
    config.hidden[2] = true;
    let both = pose.compose(&config);
    let r = both.bones[1].transform_point3(Vec3::new(-2.0, 0.0, 0.0));
    assert!(r.abs_diff_eq(Vec3::new(0.0, 2.0, 1.0), 1e-6), "{r}");
    assert_eq!(both.hidden, [false, false, true]);
}