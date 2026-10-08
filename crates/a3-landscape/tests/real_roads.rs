//! Loads the road network of every shipped world. Skipped when `A3_ROOT` is unset.

use std::collections::BTreeMap;

use a3_gamedata::{GameData, LoadOptions};
use std::time::Instant;

use a3_landscape::{RoadGraph, RoadNetwork, WorldConfig, world_classes};
use a3_wrp::Terrain;

#[test]
fn every_world_road_network_lies_on_its_land() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    let mut checked = 0;
    for class in world_classes(&game.config) {
        let world = WorldConfig::load(&game.config, &class).unwrap();
        let Ok(bytes) = game.vfs.open(world.wrp.as_str()) else {
            continue;
        };
        let terrain = Terrain::parse(&bytes).unwrap();
        // VR names no roads shapefile in its config but ships one next to its WRP.
        let shp = world.roads_shape.clone().unwrap_or_else(|| {
            world
                .wrp
                .parent()
                .expect("wrp folder")
                .join("data\\roads\\roads.shp")
        });
        let net = RoadNetwork::load(&game.vfs, &shp).unwrap_or_else(|e| panic!("{class}: {e}"));

        let size = terrain.world_size();
        let mut on_land = 0usize;
        let mut points = 0usize;
        let mut by_type: BTreeMap<String, (usize, f32)> = BTreeMap::new();
        for road in &net.roads {
            let t = net.road_type(road);
            assert!(
                net.library.get(road.id).is_some(),
                "{class}: road {} has ID {} without a RoadsLib class",
                road.record,
                road.id
            );
            let e = by_type
                .entry(format!("{} {} w{}", t.class, t.map, t.width))
                .or_default();
            e.0 += 1;
            e.1 += road.length() / 1000.0;
            for p in &road.points {
                assert!(
                    (-10.0..=size + 10.0).contains(&p.x) && (-10.0..=size + 10.0).contains(&p.y),
                    "{class}: road {} point {p} more than 10 m outside the {size} m terrain",
                    road.record
                );
                points += 1;
                if terrain.surface_height(p.x, p.y) > -1.0 {
                    on_land += 1;
                }
            }
        }
        let land_share = on_land as f64 / points.max(1) as f64;
        eprintln!(
            "{class}: {shp}: {} roads, {points} points, {:.1} km, {:.1}% on land, {} types; {by_type:?}",
            net.roads.len(),
            net.roads.iter().map(|r| r.length()).sum::<f32>() / 1000.0,
            land_share * 100.0,
            net.library.types.len(),
        );
        assert!(!net.roads.is_empty(), "{class}: no roads");

        let start = Instant::now();
        let graph = RoadGraph::new(&net);
        let built = start.elapsed();
        let junctions = graph.nodes.iter().filter(|n| n.is_junction()).count();
        let dead_ends = graph.nodes.iter().filter(|n| n.is_dead_end()).count();
        let continued = graph
            .continuations
            .iter()
            .flatten()
            .filter(|c| c.is_some())
            .count();
        // Every road's own midpoint is on a road, and its nearest centre line is (f32-)0 m away.
        for (i, road) in net.roads.iter().enumerate() {
            let mid = (road.points[0] + road.points[1]) * 0.5;
            let near = graph.nearest(mid, 1.0).unwrap();
            assert!(near.distance < 0.01, "{class}: road {i}: {}", near.distance);
            if net.road_type(road).width > 0.0 {
                assert!(
                    graph.is_on_road(mid),
                    "{class}: road {i} midpoint not on road"
                );
            }
            for seg in graph.curve(i) {
                assert!(seg.point(1.0).distance(seg.p1) < 1e-2);
            }
        }
        eprintln!(
            "  graph: {} nodes ({junctions} junctions, {dead_ends} dead ends), {continued} of {} road ends continue, {} T attachments, built in {built:.2?}",
            graph.nodes.len(),
            2 * net.roads.len(),
            graph.attachments.len(),
        );
        assert!(land_share > 0.95, "{class}: roads mostly in the sea?");
        checked += 1;
    }
    assert_eq!(checked, 6);
}
