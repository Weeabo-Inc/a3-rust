//! The moves-type integration, synthetic: a hand-written CfgMoves class and hand-built RTMs,
//! no game data. `real_data.rs` runs the same wiring against `CfgMovesMaleSdr`.

use a3_anim::SkeletonPivots;
use a3_config::{ConfigTree, parse_text};
use a3_moves::Moves;
use a3_p3d::{Bone, Skeleton};
use a3_pose::{ManRig, MoveBlend, MoveClips};
use glam::Vec3;

const CONFIG: &str = r#"
class CfgMovesBasic {
    class Default {
        actions = "NoActions"; file = ""; looped = 1; speed = 0.5; minPlayTime = 0;
        interpolationSpeed = 6; equivalentTo = ""; relSpeedMin = 1; relSpeedMax = 1;
        variantsPlayer[] = {}; variantAfter[] = {5, 10, 20}; connectTo[] = {};
    };
    class ManActions { Stop = ""; WalkF = ""; };
    class Actions { class NoActions: ManActions { turnSpeed = 1; }; };
};
class CfgMovesTest: CfgMovesBasic {
    skeletonName = "SyntheticMan";
    class States {
        class Stand: Default { file = "\a3\anims\stand.rtm"; };
        class Walk: Default { file = "\a3\anims\walk.rtm"; };
        class WalkAgain: Walk { };
        class Missing: Default { file = "\a3\anims\missing.rtm"; };
        class NoAnim: Default { };
    };
    class Actions: Actions {
        class StandActions: NoActions { WalkF = "Walk"; };
    };
};
"#;

const STAND_RTM: &str = r"a3\anims\stand.rtm";
const WALK_RTM: &str = r"a3\anims\walk.rtm";
const MISSING_RTM: &str = r"a3\anims\missing.rtm";

/// The pelvis, at the origin; the spine 1 m and the head 1.6 m above it.
const PELVIS: Vec3 = Vec3::ZERO;
const SPINE: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const HEAD: Vec3 = Vec3::new(0.0, 1.6, 0.0);

fn moves() -> Moves {
    let tree = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
    Moves::from_config(&tree.root().get("CfgMovesTest")).unwrap()
}

/// A three-bone Man: `pelvis` (root) -> `spine` -> `head`, like `man_pose.rs`.
fn rig() -> ManRig {
    let bone = |name: &str, parent: Option<usize>, parent_name: &str| Bone {
        name: name.to_string(),
        parent,
        parent_name: parent_name.to_string(),
    };
    let skeleton = Skeleton {
        name: "SyntheticMan".to_string(),
        inherited: false,
        bones: vec![
            bone("pelvis", None, ""),
            bone("spine", Some(0), "pelvis"),
            bone("head", Some(1), "spine"),
        ],
        pivots_model: "syntheticpivots".to_string(),
    };
    ManRig::with_pivots(
        &skeleton,
        SkeletonPivots {
            positions: vec![PELVIS, SPINE, HEAD],
            weapon_bone: None,
        },
    )
}

fn name32(out: &mut Vec<u8>, name: &str) {
    let mut field = [0u8; 32];
    field[..name.len()].copy_from_slice(name.as_bytes());
    out.extend_from_slice(&field);
}

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// The identity 4x3 (columns aside, up, dir, then position).
const IDENTITY_M: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// The identity with the position `t`.
fn moved(t: [f32; 3]) -> [f32; 12] {
    let mut m = IDENTITY_M;
    m[9..12].copy_from_slice(&t);
    m
}

/// A plain `RTM_0101` of the three synthetic bones: no rotation, the head translated per frame.
fn rtm(step: [f32; 3], head: &[(f32, f32)]) -> Vec<u8> {
    const BONES: [&str; 3] = ["pelvis", "spine", "head"];
    let mut out = b"RTM_0101".to_vec();
    floats(&mut out, &step);
    out.extend_from_slice(&(head.len() as u32).to_le_bytes());
    out.extend_from_slice(&(BONES.len() as u32).to_le_bytes());
    for bone in BONES {
        name32(&mut out, bone);
    }
    for (phase, z) in head {
        floats(&mut out, &[*phase]);
        for bone in BONES {
            name32(&mut out, bone);
            floats(&mut out, &moved([0.0, 0.0, *z]));
        }
    }
    out
}

/// `open` over the three synthetic files: stand and walk exist, `MISSING_RTM` does not.
fn open(path: &str) -> Option<Vec<u8>> {
    match path {
        p if p == STAND_RTM => Some(rtm([0.0, 0.0, -1.0], &[(0.0, 0.0), (1.0, 1.0)])),
        p if p == WALK_RTM => Some(rtm([0.0, 0.0, -1.6], &[(0.0, 0.0), (1.0, 2.0)])),
        _ => None,
    }
}

#[test]
fn move_ids_resolve_to_their_loaded_rtm() {
    let m = moves();
    let stand = m.find("Stand").unwrap();
    let walk = m.find("Walk").unwrap();
    let again = m.find("WalkAgain").unwrap();
    let missing = m.find("Missing").unwrap();
    let none = m.find("NoAnim").unwrap();

    let mut clips = MoveClips::new();
    let mut opened = Vec::new();
    let report = clips.load(&m, [stand, walk, again, missing, none], |path| {
        opened.push(path.to_owned());
        open(path)
    });

    // One read per distinct file: `WalkAgain` shares walk's, the empty `file` is never asked for.
    assert_eq!(opened, [STAND_RTM, WALK_RTM, MISSING_RTM]);
    assert_eq!(clips.len(), 2);
    assert!(!clips.is_empty());
    assert_eq!(report.loaded, 2);
    assert_eq!(report.missing, [MISSING_RTM]);
    assert!(report.failed.is_empty(), "{report:?}");

    let animation = clips.animation(&m, walk).unwrap();
    assert_eq!(animation.bones, ["pelvis", "spine", "head"]);
    assert_eq!(animation.frames.len(), 2);
    // A move without an RTM and one whose file is missing resolve to nothing, not a panic.
    assert!(clips.sample(&m, none, 0.0).is_none());
    assert!(clips.sample(&m, missing, 0.0).is_none());
    // The phase is the one asked for; the move vector is the file's own.
    let sample = clips.sample(&m, walk, 0.25).unwrap();
    assert_eq!(sample.phase, 0.25);
    assert_eq!(sample.step(), Vec3::new(0.0, 0.0, -1.6));
    assert!(MoveClips::new().is_empty());
}

#[test]
fn a_blend_of_move_ids_poses_like_the_same_moves_sampled() {
    let m = moves();
    let (stand, walk) = (m.find("Stand").unwrap(), m.find("Walk").unwrap());
    let mut clips = MoveClips::new();
    clips.load(&m, [stand, walk], open);
    let rig = rig();

    // The previous move at phase 0.25 poses the head at z 0.25, the current at phase 0.75 at
    // z 1.5 (each file's own keyframes); a blend of 0.5 lerps the joints to z 0.875.
    let head = |blend| {
        let state = MoveBlend {
            previous: stand,
            previous_phase: 0.25,
            current: walk,
            current_phase: 0.75,
            blend,
        };
        let state = state.state(&clips, &m).unwrap();
        assert_eq!(state.previous.phase, 0.25);
        assert_eq!(state.current.phase, 0.75);
        assert_eq!(state.phase, blend);
        rig.pose(&state).bones[2].translation
    };
    assert_eq!(head(0.0), Vec3::new(0.0, 0.0, 0.25));
    assert_eq!(head(0.5), Vec3::new(0.0, 0.0, 0.875));
    assert_eq!(head(1.0), Vec3::new(0.0, 0.0, 1.5));

    // A blend naming a move whose RTM is not loaded is nothing to pose.
    let missing = MoveBlend {
        previous: stand,
        previous_phase: 0.0,
        current: m.find("Missing").unwrap(),
        current_phase: 0.0,
        blend: 0.5,
    };
    assert!(missing.state(&clips, &m).is_none());
}

#[test]
fn the_move_vector_is_the_one_a3_moves_reads() {
    let m = moves();
    let walk = m.find("Walk").unwrap();
    let mut clips = MoveClips::new();
    clips.load(&m, [walk], open);

    assert_eq!(
        m.get(walk).step,
        Vec3::ZERO,
        "zero until the headers are read"
    );
    let mut m = m.clone();
    m.load_rtm_headers(open);
    assert_eq!(m.get(walk).step, Vec3::new(0.0, 0.0, -1.6));
    assert_eq!(
        clips.sample(&m, walk, 0.5).unwrap().step(),
        m.get(walk).step
    );
}
