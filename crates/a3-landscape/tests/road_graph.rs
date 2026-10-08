//! The road graph on small hand-made networks.

use a3_landscape::{Road, RoadEnd, RoadGraph, RoadNetwork, RoadsLib, catmull_rom_controls};
use glam::Vec2;

fn road(record: usize, id: u32, points: &[(f32, f32)]) -> Road {
    Road {
        record,
        id,
        order: 0,
        mask: 0,
        points: points.iter().map(|&(x, z)| Vec2::new(x, z)).collect(),
    }
}

/// A T junction at (100, 0): road 0 comes from the west, road 1 continues east, road 2 leaves
/// north. Road 3 is separate, 0.05 m from road 1's far end (joined), road 4 is 1 m away (not).
fn network() -> RoadNetwork {
    let lib = RoadsLib::parse("class RoadTypesLibrary { class Road0001 { width = 10; }; class Road0002 { width = 4; }; };")
        .unwrap();
    RoadNetwork {
        roads: vec![
            road(0, 1, &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
            road(1, 1, &[(100.0, 0.0), (200.0, 0.0)]),
            road(2, 2, &[(100.0, 0.0), (100.0, 80.0)]),
            road(3, 2, &[(200.05, 0.0), (260.0, 30.0)]),
            road(4, 2, &[(500.0, 500.0), (501.0, 500.0)]),
        ],
        library: lib,
    }
}

#[test]
fn ends_closer_than_a_decimetre_share_a_node() {
    let g = RoadGraph::new(&network());
    let t = g.road_nodes[0][1];
    assert_eq!(g.road_nodes[1][0], t);
    assert_eq!(g.road_nodes[2][0], t);
    assert!(g.nodes[t].is_junction());
    assert_eq!(g.road_nodes[1][1], g.road_nodes[3][0]);
    assert!(g.nodes[g.road_nodes[0][0]].is_dead_end());
    assert_eq!(g.connected_to(1), [0, 2, 3]);
    assert_eq!(g.connected_to(4), Vec::<usize>::new());
}

#[test]
fn a_road_continues_into_the_most_opposite_neighbour() {
    let g = RoadGraph::new(&network());
    // Road 0 runs east into the junction; road 1 carries on east, road 2 turns north.
    assert_eq!(g.continuations[0][1], Some((1, RoadEnd::Start)));
    assert_eq!(g.continuations[1][0], Some((0, RoadEnd::End)));
    assert_eq!(g.continuations[0][0], None);
}

#[test]
fn curves_pass_through_points_and_stay_straight_on_straight_roads() {
    let g = RoadGraph::new(&network());
    let curve = g.curve(0);
    assert_eq!(curve.len(), 2);
    assert!(curve[0].open_start && !curve[1].open_end);
    for s in &curve {
        assert_eq!(s.point(0.0), s.p0);
        assert!(s.point(1.0).distance(s.p1) < 1e-4);
        assert!(
            s.point(0.5).y.abs() < 1e-4,
            "straight road bends: {:?}",
            s.point(0.5)
        );
    }
}

#[test]
fn catmull_rom_controls_follow_the_engine_formula() {
    // Equal spacing along a line: controls at thirds.
    let (c1, c2) = catmull_rom_controls(
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, 0.0),
        Vec2::new(30.0, 0.0),
    );
    assert!(c1.distance(Vec2::new(13.333_333, 0.0)) < 1e-3, "{c1}");
    assert!(c2.distance(Vec2::new(16.666_666, 0.0)) < 1e-3, "{c2}");
    // A corner: the tangent at p1 points along p0 -> p2.
    let (c1, _) = catmull_rom_controls(
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 20.0),
    );
    let tangent = (c1 - Vec2::new(10.0, 0.0)).normalize();
    assert!(
        (tangent - Vec2::new(1.0, 1.0).normalize()).length() < 1e-3,
        "{tangent}"
    );
}

#[test]
fn nearest_point_and_on_road_use_the_road_width() {
    let g = RoadGraph::new(&network());
    let near = g.nearest(Vec2::new(30.0, 4.0), 50.0).unwrap();
    assert_eq!((near.road, near.segment), (0, 0));
    assert!((near.distance - 4.0).abs() < 1e-4);
    assert!(near.point.distance(Vec2::new(30.0, 0.0)) < 1e-4);
    // Road 0 is 10 m wide, road 2 is 4 m wide.
    assert!(g.is_on_road(Vec2::new(30.0, 4.9)));
    assert!(!g.is_on_road(Vec2::new(30.0, 5.1)));
    assert!(g.is_on_road(Vec2::new(101.9, 40.0)));
    assert!(!g.is_on_road(Vec2::new(102.1, 40.0)));
    assert_eq!(g.road_at(Vec2::new(101.0, 40.0)).unwrap().road, 2);
    assert!(g.nearest(Vec2::new(30.0, 60.0), 50.0).is_none());
    assert_eq!(g.roads_near(Vec2::new(100.0, 5.0), 6.0), [0, 1, 2]);
}

#[test]
fn a_road_ending_on_the_side_of_another_attaches_to_it() {
    let mut net = network();
    // Ends 4 m north of road 0's centre line (road 0 is 10 m wide).
    net.roads.push(road(5, 2, &[(25.0, 60.0), (25.0, 4.0)]));
    let g = RoadGraph::new(&net);
    let a = g.attachments.iter().find(|a| a.road == 5).unwrap();
    assert_eq!(a.end, RoadEnd::End);
    assert_eq!(a.onto.road, 0);
    assert!((a.onto.distance - 4.0).abs() < 1e-4);
    assert!(g.connected_to(0).contains(&5));
    assert!(g.connected_to(5).contains(&0));
    // Ends meeting at a node are not attachments.
    assert!(
        g.attachments
            .iter()
            .all(|a| a.road != 1 || a.onto.road != 0)
    );
}
