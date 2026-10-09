//! Synthetic poses, no game data: the conventions `docs/re/model-animations.md` records.
//! `real_data.rs` runs the same API on the shipped soldier skeleton and moves.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use a3_anim::SkeletonPivots;
use a3_p3d::{Bone, Skeleton};
use a3_pose::{BonePose, ManPose, ManRig, MoveSample, MoveState};
use a3_rtm::{Animation, BoneTransform, Encoding, Frame};
use glam::{Affine3A, Quat, Vec3};

/// The pelvis, at the origin.
const PELVIS: Vec3 = Vec3::ZERO;
/// The spine, 1 m above the pelvis.
const SPINE: Vec3 = Vec3::new(0.0, 1.0, 0.0);
/// The head, 1.6 m above the pelvis.
const HEAD: Vec3 = Vec3::new(0.0, 1.6, 0.0);

/// A three-bone Man: `pelvis` (root) -> `spine` -> `head`.
fn skeleton() -> Skeleton {
    let bone = |name: &str, parent: Option<usize>, parent_name: &str| Bone {
        name: name.to_string(),
        parent,
        parent_name: parent_name.to_string(),
    };
    Skeleton {
        name: "SyntheticMan".to_string(),
        inherited: false,
        bones: vec![
            bone("pelvis", None, ""),
            bone("spine", Some(0), "pelvis"),
            bone("head", Some(1), "spine"),
        ],
        pivots_model: "syntheticpivots".to_string(),
    }
}

/// A rig over the synthetic skeleton, `weapon_bone` being the index of the `weaponBone`.
fn rig(weapon_bone: Option<usize>) -> ManRig {
    ManRig::with_pivots(
        &skeleton(),
        SkeletonPivots {
            positions: vec![PELVIS, SPINE, HEAD],
            weapon_bone,
        },
    )
}

/// An animation of `bones` with the given keyframes (phase, one transform per bone). The
/// transforms are given in the model's space and stored [reversed](a3_anim::reversed), as the
/// shipped files are, so the tests read in model terms.
fn animation(bones: &[&str], step: Vec3, frames: &[(f32, Vec<BoneTransform>)]) -> Animation {
    Animation {
        encoding: Encoding::Binarized { version: 5 },
        step,
        bones: bones.iter().map(|s| (*s).to_string()).collect(),
        frames: frames
            .iter()
            .map(|(phase, transforms)| Frame {
                phase: *phase,
                transforms: transforms.iter().map(|&t| a3_anim::reversed(t)).collect(),
            })
            .collect(),
        keystones: Vec::new(),
        extra_names: Vec::new(),
    }
}

/// A one-frame move in which no bone moves.
fn still(bones: &[&str]) -> Animation {
    let transforms = vec![BoneTransform::IDENTITY; bones.len()];
    animation(bones, Vec3::ZERO, &[(0.0, transforms)])
}

/// A one-frame move translating every listed bone by `t`, no rotation.
fn offset(bones: &[&str], t: Vec3) -> Animation {
    let transforms = vec![
        BoneTransform {
            rotation: Quat::IDENTITY,
            translation: t,
        };
        bones.len()
    ];
    animation(bones, Vec3::ZERO, &[(0.0, transforms)])
}

/// A stored bone transform; the pose uses its conjugate as the rotation.
fn stored(rotation: Quat, translation: Vec3) -> BoneTransform {
    BoneTransform {
        rotation,
        translation,
    }
}

#[track_caller]
fn close(a: Vec3, b: Vec3) {
    assert!((a - b).length() < 1e-5, "{a} is not {b}");
}

#[track_caller]
fn same_rotation(a: Quat, b: Quat) {
    assert!(a.dot(b).abs() > 1.0 - 1e-6, "{a} is not {b}");
}

#[test]
fn a_still_move_poses_the_rest_pose() {
    let rig = rig(None);
    let idle = still(&["pelvis", "spine", "head"]);
    let state = MoveState::single(MoveSample::new(&idle, 0.37));
    let pose = rig.pose(&state);

    assert_eq!(pose.bones, vec![BonePose::REST; 3]);
    assert_eq!(pose, ManPose::identity(3));
}

#[test]
fn translation_is_the_bones_own_moved_with_the_rest_pivot_taken_out() {
    // No rotation, so the emitted translation is the record's own translation: `R * Q + t - R * Q`.
    let rig = rig(None);
    let move_ = animation(
        &["pelvis", "spine", "head"],
        Vec3::ZERO,
        &[(
            0.0,
            vec![
                BoneTransform::IDENTITY,
                BoneTransform::IDENTITY,
                stored(Quat::IDENTITY, Vec3::new(0.1, 0.0, 0.2)),
            ],
        )],
    );
    let pose = rig.pose(&MoveState::single(MoveSample::new(&move_, 0.0)));

    close(pose.bones[2].translation, Vec3::new(0.1, 0.0, 0.2));
    same_rotation(pose.bones[2].rotation, Quat::IDENTITY);
    assert_eq!(pose.bones[0], BonePose::REST);
    assert_eq!(pose.bones[1], BonePose::REST);
}

#[test]
fn a_rotation_turns_the_bone_about_its_rest_pivot() {
    let rig = rig(None);
    let stored_rotation = Quat::from_rotation_x(1.2);
    let move_ = animation(
        &["pelvis", "spine", "head"],
        Vec3::ZERO,
        &[(
            0.0,
            vec![
                BoneTransform::IDENTITY,
                BoneTransform::IDENTITY,
                stored(stored_rotation, Vec3::ZERO),
            ],
        )],
    );
    let pose = rig.pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    let head = pose.bones[2];

    // The stored quaternion is used as its conjugate (the engine's decoder reads the rows for
    // the columns).
    same_rotation(head.rotation, stored_rotation.conjugate());
    // The joint lands on `R * pivot`, so taking the pivot out leaves nothing behind.
    close(head.posed_pivot(HEAD), stored_rotation.conjugate() * HEAD);
    assert_eq!(head.translation, Vec3::ZERO);
}

#[test]
fn keyframes_interpolate_within_one_move() {
    let rig = rig(None);
    let bones = ["pelvis", "spine", "head"];
    let frame = |t: f32| {
        vec![
            BoneTransform::IDENTITY,
            BoneTransform::IDENTITY,
            stored(Quat::IDENTITY, Vec3::new(t, 0.0, 0.0)),
        ]
    };
    // Frames at phases 0 and 0.6: the blend weight is the position between the two phases.
    let move_ = animation(&bones, Vec3::ZERO, &[(0.0, frame(0.0)), (0.6, frame(0.6))]);

    let at = |phase: f32| {
        rig.pose(&MoveState::single(MoveSample::new(&move_, phase)))
            .bones[2]
            .translation
    };
    close(at(0.0), Vec3::ZERO);
    close(at(0.3), Vec3::new(0.3, 0.0, 0.0));
    close(at(0.6), Vec3::new(0.6, 0.0, 0.0));
}

#[test]
fn phases_outside_the_keyframes_clamp() {
    let rig = rig(None);
    let bones = ["pelvis", "spine", "head"];
    let frame = |t: f32| {
        vec![
            BoneTransform::IDENTITY,
            BoneTransform::IDENTITY,
            stored(Quat::IDENTITY, Vec3::new(t, 0.0, 0.0)),
        ]
    };
    let move_ = animation(
        &bones,
        Vec3::ZERO,
        &[(0.25, frame(0.0)), (0.75, frame(1.0))],
    );

    let at = |phase: f32| {
        rig.pose(&MoveState::single(MoveSample::new(&move_, phase)))
            .bones[2]
            .translation
    };
    close(at(-1.0), Vec3::ZERO);
    close(at(0.0), Vec3::ZERO);
    close(at(1.0), Vec3::new(1.0, 0.0, 0.0));
    close(at(2.0), Vec3::new(1.0, 0.0, 0.0));
}

#[test]
fn the_blend_lerps_joints_and_slerps_rotations() {
    let rig = rig(None);
    let bones = ["pelvis", "spine", "head"];
    let previous = still(&bones);
    // The stored rotation is RotX(-90), so the pose rotation is RotX(+90): the head swings
    // upright about its pivot, and the joint moves.
    let current = animation(
        &bones,
        Vec3::ZERO,
        &[(
            0.0,
            vec![
                BoneTransform::IDENTITY,
                BoneTransform::IDENTITY,
                stored(Quat::from_rotation_x(-FRAC_PI_2), Vec3::new(0.0, 0.0, 2.0)),
            ],
        )],
    );
    let state = MoveState::new(
        MoveSample::new(&previous, 0.0),
        MoveSample::new(&current, 0.0),
        0.5,
    );
    let pose = rig.pose(&state);
    let head = pose.bones[2];

    // The joint is halfway from the rest pivot to `RotX(90) * pivot + (0, 0, 2)`:
    // (0, 0.8, 1.8).
    close(head.posed_pivot(HEAD), Vec3::new(0.0, 0.8, 1.8));
    // The rotation is halfway from no rotation to RotX(90).
    same_rotation(head.rotation, Quat::from_rotation_x(FRAC_PI_4));
    assert_eq!(pose.bones[0], BonePose::REST);
}

#[test]
fn a_blend_phase_outside_the_moves_clamps() {
    let rig = rig(None);
    let bones = ["pelvis", "spine", "head"];
    let a = still(&bones);
    let b = offset(&bones, Vec3::new(0.0, 0.0, 4.0));
    let at = |phase: f32| {
        rig.pose(&MoveState::new(
            MoveSample::new(&a, 0.0),
            MoveSample::new(&b, 0.0),
            phase,
        ))
        .bones[2]
            .translation
    };

    close(at(0.0), Vec3::ZERO);
    close(at(0.25), Vec3::new(0.0, 0.0, 1.0));
    close(at(1.0), Vec3::new(0.0, 0.0, 4.0));
    close(at(-2.0), Vec3::ZERO);
    close(at(3.0), Vec3::new(0.0, 0.0, 4.0));
}

#[test]
fn a_bone_the_move_does_not_animate_stays_at_rest() {
    let rig = rig(None);
    // The move turns the pelvis and the head but knows nothing of the spine: no parent
    // composition, so the spine is untouched.
    let move_ = animation(
        &["pelvis", "head"],
        Vec3::ZERO,
        &[(
            0.0,
            vec![
                stored(Quat::from_rotation_y(-FRAC_PI_2), Vec3::ZERO),
                stored(Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.5)),
            ],
        )],
    );
    let pose = rig.pose(&MoveState::single(MoveSample::new(&move_, 0.0)));

    assert_eq!(pose.bones[1], BonePose::REST);
    same_rotation(pose.bones[0].rotation, Quat::from_rotation_y(FRAC_PI_2));
    close(pose.bones[2].translation, Vec3::new(0.0, 0.0, 0.5));
}

#[test]
fn the_weapon_bone_keeps_its_raw_translation() {
    let bones = ["pelvis", "spine", "head"];
    let move_ = animation(
        &bones,
        Vec3::ZERO,
        &[(
            0.0,
            vec![
                BoneTransform::IDENTITY,
                BoneTransform::IDENTITY,
                stored(Quat::from_rotation_y(-FRAC_PI_2), Vec3::new(0.0, 0.0, 0.5)),
            ],
        )],
    );

    // A normal bone: the load conversion made `t` a joint, and taking the pivot out of
    // `RotY(90) * pivot + t` leaves the record's own `t` (RotY keeps `y`).
    let normal = rig(None).pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    close(normal.bones[2].translation, Vec3::new(0.0, 0.0, 0.5));

    // The weapon bone skips that conversion, so its translation stays an offset from the rest
    // pivot in the emitted frame.
    let weapon = rig(Some(2)).pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    close(weapon.bones[2].posed_pivot(HEAD), Vec3::new(0.0, 0.0, 0.5));
    close(
        weapon.bones[2].translation,
        Vec3::new(0.0, 0.0, 0.5) - Quat::from_rotation_y(FRAC_PI_2) * HEAD,
    );
}

#[test]
fn step_is_the_move_vector() {
    let move_ = animation(
        &["pelvis"],
        Vec3::new(0.0, 0.0, -1.25),
        &[(0.0, vec![BoneTransform::IDENTITY])],
    );
    let sample = MoveSample::new(&move_, 0.5);
    assert_eq!(sample.step(), Vec3::new(0.0, 0.0, -1.25));
    assert_eq!(MoveState::single(sample).current.step(), sample.step());
}

#[test]
fn the_rig_knows_its_bones_and_root() {
    let rig = rig(None);
    assert_eq!(rig.bone_count(), 3);
    assert_eq!(rig.bone_index("head"), Some(2));
    assert_eq!(rig.bone_index("HEAD"), Some(2));
    assert_eq!(rig.bone_index("weapon"), None);
    assert_eq!(rig.root_bone(), Some(0));
    assert_eq!(rig.pivots().positions, vec![PELVIS, SPINE, HEAD]);
}

#[test]
fn children_follow_their_parents_in_the_skinning_pose() {
    let rig = rig(None);
    // Only the pelvis moves: up 0.1 m. Spine and head follow it.
    let move_ = offset(&["pelvis"], Vec3::new(0.0, 0.1, 0.0));
    let pose = rig.pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    let up = Affine3A::from_translation(Vec3::new(0.0, 0.1, 0.0));
    for m in rig.compose(&pose) {
        assert!(m.abs_diff_eq(up, 1e-6), "{m:?}");
    }

    // The model's origin sits at `offset` in pivot space; a lift stays a lift.
    let offset_ = Vec3::new(0.3, 0.8, -0.4);
    let skinning = rig.skinning_pose(&pose, offset_);
    assert_eq!(skinning.hidden, vec![false; 3]);
    assert_eq!(skinning.bones.len(), rig.bone_count());
    for m in &skinning.bones {
        assert!(m.abs_diff_eq(up, 1e-6), "{m:?}");
    }
    assert_eq!(ManRig::ground_offset(offset_), -offset_);
}

#[test]
fn stored_records_are_reversed_into_model_space() {
    // Built by hand, not through `animation`, so the record is what the file stores.
    let mut move_ = still(&["pelvis"]);
    move_.frames[0].transforms[0] = BoneTransform {
        rotation: Quat::from_rotation_y(0.5),
        translation: Vec3::new(0.1, 0.2, 0.3),
    };
    let pose = rig(None).pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    // A half turn about Y negates x and z; a turn about Y itself is unchanged by it, and the
    // engine uses the conjugate.
    assert!(
        pose.bones[0]
            .translation
            .abs_diff_eq(Vec3::new(-0.1, 0.2, -0.3), 1e-6)
    );
    assert!(
        pose.bones[0]
            .rotation
            .abs_diff_eq(Quat::from_rotation_y(-0.5), 1e-6)
    );
}

#[test]
fn a_turned_parent_carries_its_child_around_its_joint() {
    let rig = rig(None);
    // The spine turns a quarter about Z (the record holds the conjugate). Its translation keeps
    // the spine's joint in place: t = Q - R * Q.
    let r = Quat::from_rotation_z(FRAC_PI_2);
    let stored = BoneTransform {
        rotation: r.conjugate(),
        translation: SPINE - r * SPINE,
    };
    let move_ = animation(
        &["pelvis", "spine", "head"],
        Vec3::ZERO,
        &[(
            0.0,
            vec![BoneTransform::IDENTITY, stored, BoneTransform::IDENTITY],
        )],
    );
    let pose = rig.pose(&MoveState::single(MoveSample::new(&move_, 0.0)));
    let world = rig.compose(&pose);
    // The head's pivot swings about the spine's joint with the spine.
    let head = world[2].transform_point3(HEAD);
    let expected = SPINE + r * (HEAD - SPINE);
    assert!(head.abs_diff_eq(expected, 1e-4), "{head} vs {expected}");
}
