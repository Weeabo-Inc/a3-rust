//! The shipped moves types. Skipped when `A3_ROOT` is unset.

use a3_moves::{EdgeKind, Moves};

use std::sync::OnceLock;

/// The game data and `CfgMovesMaleSdr`, loaded once for all tests.
fn load() -> Option<&'static (a3_gamedata::GameData, Moves)> {
    static LOADED: OnceLock<(a3_gamedata::GameData, Moves)> = OnceLock::new();
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    Some(LOADED.get_or_init(|| {
        let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
        let moves = Moves::from_config(&data.config.root().get("CfgMovesMaleSdr")).unwrap();
        (data, moves)
    }))
}

#[test]
fn the_soldier_move_graph_resolves_every_transition() {
    let Some((_, m)) = load() else { return };
    let edges = m.edge_count();
    let actions: usize = m.action_maps().iter().map(|a| a.len()).sum();
    eprintln!(
        "CfgMovesMaleSdr: {} moves, {edges} edges, {} action maps ({actions} actions), {} warnings",
        m.len(),
        m.action_maps().len(),
        m.warnings().len()
    );
    for w in m.warnings().iter().take(20) {
        eprintln!("  {w}");
    }

    assert!(m.len() > 5000, "{} moves", m.len());
    assert!(edges > 40_000, "{edges} edges");
    assert!(m.action_maps().len() > 300);
    assert_eq!(m.skeleton_name(), "OFP2_ManSkeleton");
    // Every transition list names moves that exist.
    let transitions: Vec<_> = m
        .warnings()
        .iter()
        .filter(|w| {
            let w = w.to_ascii_lowercase();
            w.contains(".connect") || w.contains(".interpolate") || w.contains("transition")
        })
        .collect();
    assert!(transitions.is_empty(), "{transitions:?}");
}

#[test]
fn standing_to_prone_goes_through_the_drop_move() {
    let Some((_, m)) = load() else { return };
    let stand = m.find("AmovPercMstpSrasWrflDnon").unwrap();
    let prone = m.find("AmovPpneMstpSrasWrflDnon").unwrap();

    let path = m.find_path(stand, prone).unwrap();
    let names: Vec<&str> = path.iter().map(|&i| m.get(i).name.as_str()).collect();
    eprintln!("stand -> prone: {names:?}");

    assert_eq!(
        names,
        [
            "AmovPercMstpSrasWrflDnon_AmovPpneMstpSrasWrflDnon",
            "AmovPpneMstpSrasWrflDnon"
        ]
    );
    let first = m.edge(stand, path[0]).unwrap();
    assert_eq!(first.kind, EdgeKind::Interpolate);
}

#[test]
fn rifle_stand_actions_walk_and_go_prone() {
    let Some((_, m)) = load() else { return };
    let stand = m.find("AmovPercMstpSrasWrflDnon").unwrap();

    let walk = m.action_move(stand, "WalkF").unwrap();
    let down = m.action_move(stand, "Down").unwrap();

    assert_eq!(m.get(walk).name, "AmovPercMwlkSrasWrflDf");
    assert_eq!(m.get(down).name, "AmovPpneMstpSrasWrflDnon");
    assert!(m.find_path(stand, walk).is_some());
}

#[test]
fn rtm_headers_give_walking_a_forward_step() {
    let Some((data, m)) = load() else { return };
    let mut m = m.clone();

    let report = m.load_rtm_headers(|p| data.vfs.open(p).ok().map(|b| b.to_vec()));
    eprintln!(
        "RTM headers: {} loaded, {} missing (e.g. {:?}), {} failed",
        report.loaded,
        report.missing.len(),
        &report.missing[..report.missing.len().min(5)],
        report.failed.len()
    );

    assert!(report.loaded > 3000);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let walk = m.get(m.find("AmovPercMwlkSrasWrflDf").unwrap());
    assert!(walk.step.z < -1.0, "{:?}", walk.step);
    assert!(!walk.step_sounds.is_empty());
    let speed = -walk.step.z * walk.speed;
    eprintln!(
        "rifle walk: step {:?}, speed {} -> {speed} m/s",
        walk.step, walk.speed
    );
    assert!((1.0..2.0).contains(&speed), "{speed} m/s");
}
