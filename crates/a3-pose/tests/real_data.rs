//! The shipped Man: skeleton, pivots and two moves from the game install (`A3_ROOT`).
//! Skipped, with a note, when the variable is unset, so CI stays green without game data.

use std::path::Path;
use std::sync::OnceLock;

use a3_anim::{Pose, RtmBinding, SkeletonPivots};
use a3_gamedata::{GameData, LoadOptions};
use a3_moves::Moves;
use a3_p3d::{Model, Skeleton};
use a3_pose::{ManPose, ManRig, MoveBlend, MoveClips, MoveSample, MoveState};
use a3_rtm::Animation;
use a3_vfs::Vfs;
use glam::{Affine3A, Vec3};

const SOLDIER: &str = r"a3\characters_f\blufor\b_soldier_01.p3d";
const PIVOTS: &str = r"a3\anims_f\data\skeleton\skeletonpivots.p3d";
const IDLE: &str = r"a3\anims_f\data\anim\sdr\mov\erc\stp\ras\rfl\amovpercmstpsraswrfldnon.rtm";
const WALK: &str = r"a3\anims_f\data\anim\sdr\mov\erc\wlk\ras\rfl\amovpercmwlksraswrfldf.rtm";

/// The CfgSkeletonParameters `weaponBone` of `OFP2_ManSkeleton`.
const WEAPON_BONE: &str = "weapon";

/// The soldier model, the pivots model and everything the tests need, or `None` without data.
struct Data {
    soldier: Model,
    rig: ManRig,
    pivots: SkeletonPivots,
    idle: Animation,
    walk: Animation,
}

fn setup() -> Option<Data> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let vfs = Vfs::new();
    vfs.mount_archives(&Path::new(&root).join("Addons"));
    let open = |p: &str| Model::from_bytes(&vfs.open(p).unwrap()).unwrap();
    let soldier = open(SOLDIER);
    let pivots_model = open(PIVOTS);
    let skeleton: &Skeleton = soldier.skeleton.as_ref().unwrap();
    let pivots = SkeletonPivots::from_model(skeleton, &pivots_model, WEAPON_BONE);
    let rig = ManRig::with_pivots(skeleton, pivots.clone());
    let read = |p: &str| Animation::read(&vfs.open(p).unwrap()).unwrap();
    Some(Data {
        soldier,
        rig,
        pivots,
        idle: read(IDLE),
        walk: read(WALK),
    })
}

impl Data {
    fn skeleton(&self) -> &Skeleton {
        self.soldier.skeleton.as_ref().unwrap()
    }

    fn root(&self) -> usize {
        self.rig.root_bone().expect("the soldier has a root bone")
    }

    fn rest(&self, bone: usize) -> Vec3 {
        self.rig.pivots().positions[bone]
    }
}

#[track_caller]
fn pose_close(a: &ManPose, b: &ManPose, tolerance: f32, what: &str) {
    assert_eq!(a.bones.len(), b.bones.len());
    for (bone, (x, y)) in a.bones.iter().zip(&b.bones).enumerate() {
        let rotation = 1.0 - x.rotation.dot(y.rotation).abs();
        let translation = (x.translation - y.translation).length();
        assert!(
            rotation < tolerance && translation < tolerance,
            "{what}: bone {bone} differs by {rotation} / {translation} m ({} vs {})",
            x.translation,
            y.translation
        );
    }
}

#[test]
fn the_rig_is_the_shipped_soldier() {
    let Some(data) = setup() else {
        return;
    };
    let skeleton = data.skeleton();
    assert_eq!(data.rig.bone_count(), skeleton.bones.len());
    assert!(data.rig.bone_count() > 50, "a Man has many bones");
    // The name lookup is the engine's: case insensitive.
    for bone in &skeleton.bones {
        assert_eq!(
            data.rig.bone_index(&bone.name.to_uppercase()),
            skeleton.bones.iter().position(|b| b.name == bone.name)
        );
    }
    let root = data.root();
    assert_eq!(skeleton.bones[root].name, "pelvis");
    assert_eq!(data.rig.bone_index(WEAPON_BONE), data.pivots.weapon_bone);
}

#[test]
fn the_moves_cover_the_skeleton() {
    let Some(data) = setup() else {
        return;
    };
    for (label, move_) in [("idle", &data.idle), ("walk", &data.walk)] {
        let animated = data
            .skeleton()
            .bones
            .iter()
            .filter(|b| move_.bone_index(&b.name).is_some())
            .count();
        eprintln!(
            "{label}: {} frames, {} bones of the skeleton animated, step {:?}",
            move_.frames.len(),
            animated,
            move_.step
        );
        assert!(animated * 10 >= data.rig.bone_count() * 9, "most bones");
        assert!(move_.frames.len() > 2);
    }
}

#[test]
fn the_blend_ends_where_the_moves_are() {
    let Some(data) = setup() else {
        return;
    };
    let previous = MoveSample::new(&data.idle, 0.3);
    let current = MoveSample::new(&data.walk, 0.7);
    let at = |phase| data.rig.pose(&MoveState::new(previous, current, phase));

    // At the ends the blend must be the move itself. The tolerance is float noise, not slack:
    // `slerp` at weight 0 normalizes, and a few stored quaternions are non-unit by ~2e-4 (bone 53
    // of this idle), so the ends differ from the samples by that much.
    pose_close(
        &at(0.0),
        &data.rig.pose(&MoveState::single(previous)),
        1e-3,
        "phase 0",
    );
    pose_close(
        &at(1.0),
        &data.rig.pose(&MoveState::single(current)),
        1e-3,
        "phase 1",
    );

    // And the two moves differ, so the blend is not a no-op.
    let (a, b) = (at(0.0), at(1.0));
    let differ = a
        .bones
        .iter()
        .zip(&b.bones)
        .map(|(x, y)| (x.translation - y.translation).length())
        .fold(0.0f32, f32::max);
    eprintln!("idle 0.3 vs walk 0.7: furthest bone differs by {differ:.3} m");
    assert!(differ > 0.05, "the two moves pose the man differently");
}

#[test]
fn the_blend_is_continuous_and_between_its_ends() {
    let Some(data) = setup() else {
        return;
    };
    let previous = MoveSample::new(&data.idle, 0.0);
    let current = MoveSample::new(&data.walk, 0.0);

    let mut last = data.rig.pose(&MoveState::new(previous, current, 0.0));
    let mut worst_step = 0.0f32;
    for step in 1..=20 {
        let phase = step as f32 / 20.0;
        let pose = data.rig.pose(&MoveState::new(previous, current, phase));
        for (bone, (x, y)) in pose.bones.iter().zip(&last.bones).enumerate() {
            let moved = (x.translation - y.translation).length();
            assert!(
                moved.is_finite(),
                "bone {bone} went non-finite at phase {phase}"
            );
            worst_step = worst_step.max(moved);
        }
        for bone in &pose.bones {
            // The stored quaternions are f16-quantized and the engine does not renormalize, so a
            // blended rotation is unit to about 1e-4, not exactly.
            assert!((bone.rotation.length() - 1.0).abs() < 1e-3, "{bone:?}");
        }
        last = pose;
    }
    eprintln!("a 0.05 phase step moves a bone by at most {worst_step:.3} m");
    assert!(worst_step < 0.5, "the blend stays continuous");
}

#[test]
fn the_pose_matches_the_rtm_bone_frames() {
    let Some(data) = setup() else {
        return;
    };
    let skeleton = data.skeleton();
    let binding = RtmBinding::new(skeleton, &data.idle);
    let mine = data
        .rig
        .pose(&MoveState::single(MoveSample::new(&data.idle, 0.0)));
    // The same records through `a3-anim`: the emitted frame is `[M | T - M*Q]` there too, and
    // both compose it down the skeleton.
    let theirs = Pose::from_rtm_frames(
        &binding.keyframe(&data.idle, 0, &data.pivots),
        &data.pivots,
        skeleton,
        Vec3::ZERO,
    );
    let composed = data.rig.compose(&mine);
    let mut worst = 0.0f32;
    let mut compared = 0;
    for (bone, (&a, &b)) in composed.iter().zip(&theirs.bones).enumerate() {
        let Some(rtm_bone) = binding.rtm_bone[bone] else {
            continue;
        };
        if data.idle.frames[0].transforms.get(rtm_bone).is_none() {
            continue;
        }
        compared += 1;
        worst = worst.max(matrix_difference(a, b));
    }
    eprintln!("cross-checked {compared} bones against a3-anim, worst difference {worst:.2e}");
    assert!(compared * 10 >= data.rig.bone_count() * 9);
    // Not bit-exact: `a3-anim` rotates the pivot by the quaternion, `a3-pose` by its matrix, and
    // the few stored quaternions that are not unit (f16 quantization, or a degenerate source
    // matrix) are scaled differently by the two. The engine's decoder builds a matrix.
    assert!(
        worst < 1e-3,
        "a3-pose and a3-anim agree on the emitted frames"
    );
}

/// Bones that are not joints of the body: `weapon`, `launcher` and `camera` are attachment
/// points, and `face_hub`'s record undoes the head's frame (the face rig belongs to the head
/// model; see "Face rig" in `docs/re/model-animations.md`).
const NOT_JOINTS: [&str; 4] = ["weapon", "launcher", "camera", "face_hub"];

/// The composed pose keeps the body together (`docs/re/model-animations.md`, "Pose
/// coherence"): every joint stays within 1 cm of its rest distance from its parent's (f16
/// precision), over every keyframe of the idle and the walk.
#[test]
fn the_composed_pose_keeps_every_bone_length() {
    let Some(data) = setup() else {
        return;
    };
    let skeleton = data.skeleton();
    for (label, move_) in [("idle", &data.idle), ("walk", &data.walk)] {
        let mut worst = (0.0f32, String::new());
        for frame in &move_.frames {
            let pose = data
                .rig
                .pose(&MoveState::single(MoveSample::new(move_, frame.phase)));
            let world = data.rig.compose(&pose);
            let joint = |b: usize| world[b].transform_point3(data.rest(b));
            for (bone, b) in skeleton.bones.iter().enumerate() {
                let Some(parent) = b.parent else { continue };
                if NOT_JOINTS.contains(&b.name.as_str()) {
                    continue;
                }
                let rest = data.rest(bone).distance(data.rest(parent));
                let error = (joint(bone).distance(joint(parent)) - rest).abs();
                if error > worst.0 {
                    worst = (error, format!("{} at phase {}", b.name, frame.phase));
                }
            }
        }
        eprintln!(
            "{label}: worst bone length change {:.3} m ({})",
            worst.0, worst.1
        );
        assert!(worst.0 < 0.01, "{label}: {} off by {} m", worst.1, worst.0);
    }
}

/// The root record lifts the pelvis to its hip height, so the posed Man stands on `y = 0` of
/// the pivots model's space at every keyframe, and his head stays a head's height above it.
#[test]
fn the_composed_man_stands_on_the_ground() {
    let Some(data) = setup() else {
        return;
    };
    let toes = [
        data.rig.bone_index("lefttoebase").unwrap(),
        data.rig.bone_index("righttoebase").unwrap(),
    ];
    let head = data.rig.bone_index("head").unwrap();
    for (label, move_) in [("idle", &data.idle), ("walk", &data.walk)] {
        let mut lowest = (f32::MAX, f32::MIN);
        for frame in &move_.frames {
            let pose = data
                .rig
                .pose(&MoveState::single(MoveSample::new(move_, frame.phase)));
            let world = data.rig.compose(&pose);
            let toe = toes
                .iter()
                .map(|&b| world[b].transform_point3(data.rest(b)).y)
                .fold(f32::MAX, f32::min);
            lowest = (lowest.0.min(toe), lowest.1.max(toe));
            let h = world[head].transform_point3(data.rest(head)).y;
            assert!((1.2..1.8).contains(&h), "{label}: head at {h}");
        }
        eprintln!(
            "{label}: lowest toe {:.3}..{:.3} m over the cycle",
            lowest.0, lowest.1
        );
        assert!(
            lowest.0 > -0.05 && lowest.1 < 0.08,
            "{label}: a foot is on the ground at every keyframe"
        );
    }
}

/// Skinned through the composed pose, the soldier's drawn mesh (proxies left out) keeps a
/// Man's proportions: issue #247 measured a 2.0 x 3.3 x 2.2 m spike-ball with the flat pose.
#[test]
fn the_skinned_soldier_keeps_a_mans_proportions() {
    let Some(data) = setup() else {
        return;
    };
    let offset = data.soldier.info.bounding_center;
    let lod = &data.soldier.lods[0];
    for (label, move_) in [("idle", &data.idle), ("walk", &data.walk)] {
        let pose = data
            .rig
            .pose(&MoveState::single(MoveSample::new(move_, 0.0)));
        let skinning = data.rig.skinning_pose(&pose, offset);
        let posed = a3_anim::skin(lod, &skinning.skinning(lod)).positions;
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for section in lod.sections.iter().filter(|s| !s.is_proxy()) {
            for face in &lod.faces[section.faces.start as usize..section.faces.end as usize] {
                for &i in face.indices() {
                    let p = posed[i as usize] + offset;
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
        }
        let size = hi - lo;
        eprintln!("{label}: posed box {lo:.3}..{hi:.3}, size {size:.3}");
        assert!(
            (1.3..1.9).contains(&size.y),
            "{label}: {size} tall (no head: it is a proxy)"
        );
        assert!(size.x < 1.2 && size.z < 1.2, "{label}: {size}");
        assert!(
            lo.y.abs() < 0.08,
            "{label}: the boots are on the ground ({lo})"
        );
    }
}

/// `CfgMovesMaleSdr` and the game data its RTM paths resolve against, loaded once.
fn moves_type() -> Option<&'static (GameData, Moves)> {
    static LOADED: OnceLock<(GameData, Moves)> = OnceLock::new();
    let root = std::env::var_os("A3_ROOT")?;
    Some(LOADED.get_or_init(|| {
        let data = GameData::load(&LoadOptions::new(root)).unwrap();
        let moves = Moves::from_config(&data.config.root().get("CfgMovesMaleSdr")).unwrap();
        (data, moves)
    }))
}

#[test]
fn the_moves_type_resolves_its_ids_to_the_same_rtms_the_pose_uses() {
    let Some(data) = setup() else {
        return;
    };
    let Some((game, moves)) = moves_type() else {
        return;
    };
    let idle = moves.find("AmovPercMstpSrasWrflDnon").unwrap();
    let walk = moves.find("AmovPercMwlkSrasWrflDf").unwrap();
    eprintln!(
        "moves: {:?} at {:?}, {:?} at {:?}",
        moves.get(idle).name,
        moves.get(idle).file,
        moves.get(walk).name,
        moves.get(walk).file
    );

    // Loading two moves of a 5,575-move type reads two files, the ones the tests pose directly.
    let mut clips = MoveClips::new();
    let report = clips.load(moves, [idle, walk], |p| {
        game.vfs.open(p).ok().map(|b| b.to_vec())
    });
    assert_eq!(report.loaded, 2, "{report:?}");
    assert!(
        report.missing.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
    assert_eq!(clips.len(), 2);
    assert_eq!(clips.animation(moves, idle), Some(&data.idle));
    assert_eq!(clips.animation(moves, walk), Some(&data.walk));

    // The moves type's blend (ids and phases) is the same pose as the samples it names, and the
    // move vector the app advances the entity by is the one a3-moves gives the same move.
    let state = MoveBlend {
        previous: idle,
        previous_phase: 0.3,
        current: walk,
        current_phase: 0.7,
        blend: 0.5,
    }
    .state(&clips, moves)
    .unwrap();
    assert_eq!(
        data.rig.pose(&state),
        data.rig.pose(&MoveState::new(
            MoveSample::new(&data.idle, 0.3),
            MoveSample::new(&data.walk, 0.7),
            0.5,
        ))
    );
    assert_eq!(state.current.step(), data.walk.step);

    // A move with no RTM (`file = ""`) is not a pose.
    if let Some(none) = moves
        .iter()
        .find(|(_, m)| m.file.is_empty())
        .map(|(id, _)| id)
    {
        assert!(clips.sample(moves, none, 0.0).is_none());
    }
}

#[test]
fn dump_blended_pose() {
    let Some(data) = setup() else {
        return;
    };
    let previous = MoveSample::new(&data.idle, 0.25);
    let current = MoveSample::new(&data.walk, 0.75);
    let state = MoveState::new(previous, current, 0.5);
    let pose = data.rig.pose(&state);
    let skeleton = data.skeleton();

    eprintln!(
        "-- blended pose: idle at phase 0.25 -> walk at phase 0.75, blend 0.5, {} bones",
        pose.bones.len()
    );
    eprintln!(
        "   step: idle {:?}, walk {:?}",
        data.idle.step, data.walk.step
    );
    for (bone, frame) in pose.bones.iter().enumerate() {
        // `R*Q + t`: the joint the frame maps the rest pivot onto (`M*Q + t` once blended) — see
        // "Pose coherence" in `docs/re/model-animations.md` for what this does and does not mean.
        eprintln!(
            "{:<20} q ({:>8.5},{:>8.5},{:>8.5},{:>8.5})  t ({:>9.5},{:>9.5},{:>9.5})  R*Q+t ({:>8.4},{:>8.4},{:>8.4})",
            skeleton.bones[bone].name,
            frame.rotation.x,
            frame.rotation.y,
            frame.rotation.z,
            frame.rotation.w,
            frame.translation.x,
            frame.translation.y,
            frame.translation.z,
            frame.posed_pivot(data.rest(bone)).x,
            frame.posed_pivot(data.rest(bone)).y,
            frame.posed_pivot(data.rest(bone)).z,
        );
    }
    eprintln!("-- rest pivots (pivots model space)");
    for (bone, name) in skeleton.bones.iter().enumerate() {
        let rest = data.rest(bone);
        eprintln!(
            "{:<20} ({:>8.4},{:>8.4},{:>8.4})",
            name.name, rest.x, rest.y, rest.z
        );
    }
}

fn matrix_difference(a: Affine3A, b: Affine3A) -> f32 {
    let m = a.matrix3 - b.matrix3;
    let mut worst = (a.translation - b.translation).length();
    for column in m.to_cols_array() {
        worst = worst.max(column.abs());
    }
    worst
}
