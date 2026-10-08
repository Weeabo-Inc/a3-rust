//! The move graph and its path search, on hand-written moves configs.

use a3_config::{ConfigTree, parse_text};
use a3_moves::{EdgeKind, MoveId, Moves};

fn moves(states: &str, extra: &str) -> Moves {
    let text = format!(
        r#"
class CfgMovesBasic {{
    class Default {{
        actions = "NoActions"; file = ""; looped = 1; speed = 0.5;
        connectFrom[] = {{}}; connectTo[] = {{}}; interpolateWith[] = {{}};
        interpolateTo[] = {{}}; interpolateFrom[] = {{}}; connectAs = "";
        interpolationSpeed = 6; equivalentTo = "";
    }};
    class Actions {{ class NoActions {{ stop = ""; }}; }};
    class Interpolations {{}};
    transitionsInterpolated[] = {{}};
    transitionsSimple[] = {{}};
    transitionsDisabled[] = {{}};
}};
class CfgMovesTest: CfgMovesBasic {{
    {extra}
    class States {{ {states} }};
}};
"#
    );
    let tree = ConfigTree::from_config(&parse_text(&text).unwrap());
    Moves::from_config(&tree.root().get("CfgMovesTest")).unwrap()
}

fn id(m: &Moves, name: &str) -> MoveId {
    m.find(name).unwrap_or_else(|| panic!("no move {name}"))
}

fn names(m: &Moves, path: &[MoveId]) -> Vec<String> {
    path.iter().map(|&i| m.get(i).name.clone()).collect()
}

#[test]
fn states_are_numbered_in_config_order_and_found_ignoring_case() {
    let m = moves(
        "class A: Default {}; class B: Default {}; class C: Default {};",
        "",
    );

    assert_eq!(m.len(), 3);
    assert_eq!(m.find("a"), Some(MoveId(0)));
    assert_eq!(m.find("C"), Some(MoveId(2)));
    assert_eq!(m.find("D"), None);
}

#[test]
fn connect_to_and_interpolate_to_make_edges_with_costs_in_thousandths() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"B", 0.02}; interpolateTo[] = {"C", 0.5}; };
           class B: Default {}; class C: Default {};"#,
        "",
    );
    let (a, b, c) = (id(&m, "A"), id(&m, "B"), id(&m, "C"));

    let ab = m.edge(a, b).unwrap();
    assert_eq!((ab.kind, ab.cost), (EdgeKind::Connect, 20));
    let ac = m.edge(a, c).unwrap();
    assert_eq!((ac.kind, ac.cost), (EdgeKind::Interpolate, 500));
    assert_eq!(m.edge(b, a), None);
}

#[test]
fn reverse_and_symmetric_lists_add_edges_into_the_state() {
    let m = moves(
        r#"class A: Default { connectFrom[] = {"B", 0.1}; interpolateWith[] = {"C", 0.2};
                             interpolateFrom[] = {"D", 0.3}; };
           class B: Default {}; class C: Default {}; class D: Default {};"#,
        "",
    );
    let (a, b, c, d) = (id(&m, "A"), id(&m, "B"), id(&m, "C"), id(&m, "D"));

    assert_eq!(m.edge(b, a).unwrap().kind, EdgeKind::Connect);
    assert_eq!(m.edge(a, c).unwrap().kind, EdgeKind::Interpolate);
    assert_eq!(m.edge(c, a).unwrap().kind, EdgeKind::Interpolate);
    assert_eq!(m.edge(d, a).unwrap().cost, 300);
    assert_eq!(m.edge(a, b), None);
}

#[test]
fn unknown_targets_and_self_edges_are_skipped() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"A", 0.1, "Nowhere", 0.1, "B", 0.1}; };
           class B: Default {};"#,
        "",
    );
    let a = id(&m, "A");

    assert_eq!(m.edges(a).len(), 1);
    assert!(
        m.warnings().iter().any(|w| w.contains("Nowhere")),
        "{:?}",
        m.warnings()
    );
}

#[test]
fn class_level_transition_lists_and_interpolation_groups() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"B", 0.1, "C", 0.1}; };
           class B: Default {}; class C: Default {}; class D: Default {};"#,
        r#"class Interpolations { group[] = {0.4, "B", "C", "D"}; };
           transitionsInterpolated[] = {"D", "A", 0.7};
           transitionsSimple[] = {"C", "A", 0.8};
           transitionsDisabled[] = {"A", "C"};"#,
    );
    let (a, b, c, d) = (id(&m, "A"), id(&m, "B"), id(&m, "C"), id(&m, "D"));

    for (x, y) in [(b, c), (c, b), (b, d), (d, b), (c, d), (d, c)] {
        let e = m.edge(x, y).unwrap();
        assert_eq!((e.kind, e.cost), (EdgeKind::Interpolate, 400));
    }
    assert_eq!(m.edge(d, a).unwrap().kind, EdgeKind::Interpolate);
    assert_eq!(m.edge(c, a).unwrap().kind, EdgeKind::Connect);
    assert_eq!(m.edge(a, b).unwrap().cost, 100);
    assert_eq!(m.edge(a, c), None, "disabled");
}

#[test]
fn connect_as_copies_the_edges_of_another_state() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"B", 0.1}; };
           class B: Default { interpolateTo[] = {"C", 0.2}; };
           class C: Default {};
           class X: Default { connectAs = "B"; };"#,
        "",
    );
    let (a, c, x) = (id(&m, "A"), id(&m, "C"), id(&m, "X"));

    let xc = m.edge(x, c).unwrap();
    assert_eq!((xc.kind, xc.cost), (EdgeKind::Interpolate, 200));
    let ax = m.edge(a, x).unwrap();
    assert_eq!((ax.kind, ax.cost), (EdgeKind::Connect, 100));
}

#[test]
fn ignore_min_play_time_marks_edges() {
    let m = moves(
        r#"class A: Default { interpolateTo[] = {"B", 0.1, "C", 0.1}; ignoreMinPlayTime[] = {"C"}; };
           class B: Default {}; class C: Default {};"#,
        "",
    );
    let (a, b, c) = (id(&m, "A"), id(&m, "B"), id(&m, "C"));

    assert!(!m.edge(a, b).unwrap().ignore_min_play_time);
    assert!(m.edge(a, c).unwrap().ignore_min_play_time);
}

#[test]
fn path_follows_the_graph_to_the_target() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"B", 0.1}; };
           class B: Default { interpolateTo[] = {"C", 0.1}; };
           class C: Default {};"#,
        "",
    );

    let path = m.find_path(id(&m, "A"), id(&m, "C")).unwrap();

    assert_eq!(names(&m, &path), ["B", "C"]);
}

#[test]
fn path_to_the_current_move_is_empty_and_unreachable_moves_have_none() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"B", 0.1}; }; class B: Default {}; class C: Default {};"#,
        "",
    );
    let (a, b, c) = (id(&m, "A"), id(&m, "B"), id(&m, "C"));

    assert_eq!(m.find_path(a, a), Some(vec![]));
    assert_eq!(m.find_path(a, c), None);
    assert_eq!(m.find_path(b, a), None);
}

#[test]
fn a_direct_edge_wins_whatever_its_cost() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"T", 9.0, "B", 0.01}; };
           class B: Default { connectTo[] = {"T", 0.01}; };
           class T: Default {};"#,
        "",
    );

    let path = m.find_path(id(&m, "A"), id(&m, "T")).unwrap();

    assert_eq!(names(&m, &path), ["T"]);
}

#[test]
fn path_takes_the_cheapest_way_round() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"X", 0.1, "Y", 0.3}; };
           class X: Default { connectTo[] = {"Z", 0.1}; };
           class Y: Default { connectTo[] = {"T", 0.1}; };
           class Z: Default { connectTo[] = {"T", 0.1}; };
           class T: Default {};"#,
        "",
    );

    let path = m.find_path(id(&m, "A"), id(&m, "T")).unwrap();

    assert_eq!(names(&m, &path), ["X", "Z", "T"]);
}

/// The engine stops searching as soon as the target first gets a distance, so a cheaper path
/// through a move that is expanded later is not found.
#[test]
fn path_search_stops_at_the_first_way_into_the_target() {
    let m = moves(
        r#"class A: Default { connectTo[] = {"X", 0.1, "Y", 0.05}; };
           class X: Default { connectTo[] = {"T", 0.1}; };
           class Y: Default { connectTo[] = {"T", 1.0}; };
           class T: Default {};"#,
        "",
    );

    let path = m.find_path(id(&m, "A"), id(&m, "T")).unwrap();

    assert_eq!(names(&m, &path), ["Y", "T"]);
}
