//! Navigation over the shipped terrains: baking Altis and planning across it, and the path
//! meshes of its houses. Skipped when `A3_ROOT` is unset.

use std::sync::Arc;
use std::time::Instant;

use a3_gamedata::{GameData, LoadOptions};
use a3_landscape::{RoadGraph, RoadNetwork, WorldConfig, world_classes};
use a3_nav::{Navigator, PathMesh, Planner, cost};
use a3_wrp::Terrain;
use glam::Vec3;

fn altis(game: &GameData) -> Arc<Terrain> {
    let class = world_classes(&game.config)
        .into_iter()
        .find(|c| c.eq_ignore_ascii_case("altis") || c.eq_ignore_ascii_case("altis_"))
        .expect("Altis is shipped");
    let world = WorldConfig::load(&game.config, &class).expect("the Altis world config");
    Arc::new(
        Terrain::parse(&game.vfs.open(world.wrp.as_str()).expect("the WRP")).expect("a terrain"),
    )
}

/// The grid bakes from the real geography, and paths across the island are found; prints the
/// bake time and the planning throughput.
#[test]
fn altis_bakes_and_plans() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    let terrain = altis(&game);

    // Bake without the roads, then with them.
    let start = Instant::now();
    let plain = Navigator::new(terrain.clone());
    let plain_bake = start.elapsed();
    let world = world_classes(&game.config)
        .into_iter()
        .find(|c| c.eq_ignore_ascii_case("altis") || c.eq_ignore_ascii_case("altis_"))
        .expect("Altis");
    let shp = WorldConfig::load(&game.config, &world)
        .unwrap()
        .roads_shape
        .expect("Altis names its roads shape");
    let net = RoadNetwork::load(&game.vfs, &shp).unwrap();
    let graph = RoadGraph::new(&net);
    let start = Instant::now();
    let nav = Navigator::with_roads(terrain.clone(), &graph);
    let road_bake = start.elapsed();

    let size = terrain.land_grid;
    let cells = size.width as usize * size.height as usize;
    let walkable = (0..size.height)
        .map(|z| {
            (0..size.width)
                .filter(|&x| nav.grid().is_walkable(x, z))
                .count()
        })
        .sum::<usize>();
    let roads = (0..size.height)
        .map(|z| {
            (0..size.width)
                .filter(|&x| nav.grid().cost(x, z) == cost::ROAD)
                .count()
        })
        .sum::<usize>();
    eprintln!(
        "Altis: {cells} land cells, {:.1}% walkable, {roads} road cells; \
         grid baked in {plain_bake:.2?} (geography + slope), {road_bake:.2?} more with {} roads",
        walkable as f64 / cells as f64 * 100.0,
        net.roads.len()
    );
    assert_eq!(plain.grid().width(), size.width);
    // Altis is an island in a 30.7 km square: most of the grid is sea.
    assert!(walkable * 10 > cells, "Altis has land");

    // Deterministic pairs of walkable cells, 300 m to 2 km apart — the length of an AI order.
    // SplitMix64: an LCG's low bits would repeat long before 100k draws.
    let mut state = 0x5eed_1234_5678_9abcu64;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut pairs: Vec<(Vec3, Vec3)> = Vec::new();
    let mut tries = 0;
    while pairs.len() < 60 && tries < 100_000 {
        tries += 1;
        let (a, b) = (next(), next());
        let a = ((a >> 32) as u32 % size.width, a as u32 % size.height);
        let b = ((b >> 32) as u32 % size.width, b as u32 % size.height);
        if !nav.grid().is_walkable(a.0, a.1) || !nav.grid().is_walkable(b.0, b.1) {
            continue;
        }
        let (from, to) = (
            nav.grid().cell_center(a.0, a.1),
            nav.grid().cell_center(b.0, b.1),
        );
        let distance = (from - to).length();
        if (300.0..=2000.0).contains(&distance) {
            pairs.push((from, to));
        }
    }
    assert_eq!(pairs.len(), 60, "seeded pairs across Altis");

    let mut planner = Planner::new();
    let mut waypoints = 0usize;
    let mut on_road = 0usize;
    let mut expansions = 0usize;
    let mut longest = 0.0f32;
    let start = Instant::now();
    for &(from, to) in &pairs {
        let path = nav
            .find_path_with(&mut planner, from, to, 60.0)
            .unwrap_or_else(|| panic!("no path from {from} to {to}"));
        expansions += planner.expanded();
        waypoints += path.len();
        for p in &path {
            assert_ne!(nav.grid().cost_at(*p), cost::IMPASSABLE, "{p:?}");
            if nav.grid().cost_at(*p) == cost::ROAD {
                on_road += 1;
            }
        }
        let walked = path.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f32>();
        longest = longest.max(walked);
    }
    let elapsed = start.elapsed();
    eprintln!(
        "Altis: {} paths planned in {elapsed:.2?} ({:.2} paths/ms), {:.0} waypoints and {expansions} cells expanded per path, {:.1}% of waypoints on road cells, longest walk {:.0} m",
        pairs.len(),
        pairs.len() as f64 / elapsed.as_secs_f64() / 1000.0,
        waypoints as f64 / pairs.len() as f64,
        on_road as f64 / waypoints as f64 * 100.0,
        longest,
    );
}

/// The houses of Altis ship path meshes: a Paths LOD (or a Roadway fallback) that routes
/// through the building.
#[test]
fn altis_houses_have_path_meshes() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let game = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    let terrain = altis(&game);

    let start = Instant::now();
    let (mut checked, mut meshes, mut triangles, mut points_only) = (0, 0, 0, 0);
    for model_path in terrain
        .models
        .iter()
        .filter(|m| m.as_str().to_ascii_lowercase().contains("\\households\\"))
        .take(60)
    {
        let Ok(bytes) = game.vfs.open(model_path.as_str()) else {
            continue;
        };
        let Ok(model) = a3_p3d::Model::from_bytes(&bytes) else {
            continue;
        };
        checked += 1;
        match PathMesh::from_model(&model) {
            Some(mesh) if !mesh.is_empty() => {
                meshes += 1;
                triangles += mesh.triangle_count();
            }
            _ => points_only += 1,
        }
    }
    eprintln!(
        "Altis: {checked} house models in {:.2?}: {meshes} with a path mesh ({triangles} triangles), {points_only} without",
        start.elapsed()
    );
    assert!(checked > 10, "Altis ships house models");
    assert!(
        meshes * 2 > checked,
        "{points_only} of {checked} houses have no path mesh"
    );
    // A path mesh routes across itself: any two of its path positions that are connected.
    let bytes = terrain
        .models
        .iter()
        .filter(|m| m.as_str().to_ascii_lowercase().contains("\\households\\"))
        .find_map(|m| game.vfs.open(m.as_str()).ok())
        .expect("a house");
    let model = a3_p3d::Model::from_bytes(&bytes).expect("a model");
    let mesh = PathMesh::from_model(&model).expect("a path mesh");
    let end = mesh.positions().len() - 1;
    if let Some(path) = mesh.find_path_between(0, end) {
        assert!(path.len() >= 2);
        eprintln!(
            "first house: {end} path positions, {} triangles",
            mesh.triangle_count()
        );
    }
}
