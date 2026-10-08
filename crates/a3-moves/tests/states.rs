//! Move parameters, action maps and RTM headers, on hand-written moves configs.

use a3_config::{ConfigTree, parse_text};
use a3_moves::{ActionTarget, ManPos, Moves, Stance};
use glam::Vec3;

const CONFIG: &str = r#"
class CfgMovesBasic {
    class Default {
        actions = "NoActions"; file = ""; looped = 1; speed = 0.5; minPlayTime = 0;
        interpolationSpeed = 6; equivalentTo = ""; relSpeedMin = 1; relSpeedMax = 1;
        variantsPlayer[] = {}; variantAfter[] = {5, 10, 20}; connectTo[] = {};
    };
    class ManActions { Stop = ""; WalkF = ""; Down = ""; ReloadMagazine = ""; };
    class Actions {
        class NoActions: ManActions { turnSpeed = 1; upDegree = -1; stance = "ManStanceUndefined"; };
    };
};
class CfgMovesTest: CfgMovesBasic {
    skeletonName = "OFP2_ManSkeleton";
    gestures = "CfgGesturesMale";
    class States {
        class Stand: Default {
            actions = "StandActions";
            file = "\A3\anims\Stand";
            speed = -30;
            minPlayTime = 1.5;
            variantsPlayer[] = {"Idle", 0.25, "Missing", 0.75};
        };
        class Walk: Stand { file = "a3\anims\walk.rtm"; speed = 0.85; looped = 1; relSpeedMin = 0.8; };
        class Idle: Stand { looped = 0; terminal = 1; };
        class Prone: Default { actions = "ProneActions"; };
    };
    class Actions: Actions {
        class StandActions: NoActions {
            Stop = "Stand"; WalkF = "Walk"; Down = "Prone";
            ReloadMagazine[] = {"GestureReloadMX", "Gesture"};
            turnSpeed = 8; upDegree = "ManPosCombat"; stance = "ManStanceStand"; limitFast = 5.5;
        };
        class ProneActions: StandActions { Down = "Stand"; WalkF = ""; stance = "ManStanceProne"; upDegree = 4; };
    };
};
"#;

fn moves() -> Moves {
    let tree = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
    Moves::from_config(&tree.root().get("CfgMovesTest")).unwrap()
}

#[test]
fn reads_move_parameters_and_converts_negative_speeds_to_durations() {
    let m = moves();
    let stand = m.get(m.find("Stand").unwrap());
    let walk = m.get(m.find("Walk").unwrap());

    assert_eq!(m.skeleton_name(), "OFP2_ManSkeleton");
    assert_eq!(m.gestures(), "CfgGesturesMale");
    assert_eq!(stand.file, "a3\\anims\\stand.rtm");
    assert_eq!(walk.file, "a3\\anims\\walk.rtm");
    assert!((stand.speed - 1.0 / 30.0).abs() < 1e-7);
    assert_eq!(walk.speed, 0.85);
    assert_eq!(stand.min_play_time, 1.0, "clamped to a phase");
    assert_eq!(walk.rel_speed_min, 0.8);
    assert_eq!(stand.variant_after, [5.0, 10.0, 20.0]);
    assert_eq!(stand.variants_player, [(m.find("Idle").unwrap(), 0.25)]);
    let idle = m.get(m.find("Idle").unwrap());
    assert!(!idle.looped && idle.terminal);
}

#[test]
fn action_maps_map_actions_to_moves_and_gestures() {
    let m = moves();
    let stand = m.find("Stand").unwrap();
    let prone = m.find("Prone").unwrap();

    assert_eq!(
        m.action(stand, "walkf"),
        Some(&ActionTarget::Move(m.find("Walk").unwrap()))
    );
    assert_eq!(m.action(stand, "Down"), Some(&ActionTarget::Move(prone)));
    assert_eq!(
        m.action(stand, "ReloadMagazine"),
        Some(&ActionTarget::Gesture("GestureReloadMX".into()))
    );
    assert_eq!(m.action(prone, "Down"), Some(&ActionTarget::Move(stand)));
    assert_eq!(m.action(prone, "WalkF"), None, "emptied in the derived map");
}

#[test]
fn action_maps_carry_stance_turn_speed_and_posture() {
    let m = moves();
    let stand = m.action_map(m.get(m.find("Stand").unwrap()).actions.unwrap());
    let prone = m.action_map(m.find_action_map("proneactions").unwrap());

    assert_eq!(stand.turn_speed, 8.0);
    assert_eq!(stand.limit_fast, 5.5);
    assert_eq!(stand.stance, Stance::Stand);
    assert_eq!(stand.up_degree, Some(ManPos::Combat));
    assert_eq!(prone.stance, Stance::Prone);
    assert_eq!(prone.up_degree, Some(ManPos::Lying));
    assert_eq!(
        m.action_map(m.find_action_map("NoActions").unwrap())
            .up_degree,
        None
    );
}

#[test]
fn unknown_moves_are_reported() {
    let m = moves();
    assert!(
        m.warnings().iter().any(|w| w.contains("\"Missing\"")),
        "{:?}",
        m.warnings()
    );
}

#[test]
fn rtm_headers_give_each_move_its_step_once_per_file() {
    let mut m = moves();
    // A plain RTM with step (0, 0, -1.6) and no bones or frames.
    let mut rtm = b"RTM_0101".to_vec();
    for v in [0.0f32, 0.0, -1.6] {
        rtm.extend_from_slice(&v.to_le_bytes());
    }
    rtm.extend_from_slice(&0u32.to_le_bytes());
    rtm.extend_from_slice(&0u32.to_le_bytes());
    let mut opened = Vec::new();

    let report = m.load_rtm_headers(|path| {
        opened.push(path.to_owned());
        (path == "a3\\anims\\walk.rtm").then(|| rtm.clone())
    });

    assert_eq!(report.loaded, 1);
    assert_eq!(report.missing, ["a3\\anims\\stand.rtm"]);
    assert_eq!(opened.len(), 2, "{opened:?}");
    assert_eq!(
        m.get(m.find("Walk").unwrap()).step,
        Vec3::new(0.0, 0.0, -1.6)
    );
    assert_eq!(m.get(m.find("Stand").unwrap()).step, Vec3::ZERO);
}
