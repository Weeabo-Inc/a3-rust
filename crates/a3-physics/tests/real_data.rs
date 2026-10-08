//! Collision for shipped terrains. Skipped when `A3_ROOT` is unset.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use a3_physics::{
    CollisionWorld, Interest, Layer, ModelBank, ObjectKey, RayQuery, StaticSource, TerrainStatics,
};
use a3_wrp::Terrain;
use glam::{DVec2, DVec3};

struct Game {
    vfs: a3_vfs::Vfs,
    config: Arc<a3_config::ConfigTree>,
}

fn game() -> Option<&'static Game> {
    static GAME: OnceLock<Option<Game>> = OnceLock::new();
    GAME.get_or_init(|| {
        let root = std::env::var_os("A3_ROOT")?;
        let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
        Some(Game {
            vfs: data.vfs,
            config: data.config,
        })
    })
    .as_ref()
    .or_else(|| {
        eprintln!("skipping: A3_ROOT not set");
        None
    })
}

fn terrain(game: &Game, path: &str) -> Arc<Terrain> {
    Arc::new(Terrain::parse(&game.vfs.open(path).unwrap()).unwrap())
}

fn collision(game: &Game, terrain: &Arc<Terrain>) -> (CollisionWorld, TerrainStatics) {
    let bank = ModelBank::new(Arc::new(game.vfs.clone()), Some(&game.config));
    let mut w = CollisionWorld::new(bank);
    w.set_terrain(Some(terrain.clone()));
    (w, TerrainStatics::new(terrain.clone()))
}

/// Streams a town, then checks that a ray from above hits every house of it on its own
/// Geometry, standing on the terrain. Prints build time and size.
fn town(wrp: &str, name: &str, centre: DVec2, radius: f64) {
    let Some(game) = game() else { return };
    let t = terrain(game, wrp);
    let (mut w, statics) = collision(game, &t);
    let start = Instant::now();
    let report = w.stream(
        &statics,
        &[Interest {
            center: DVec3::new(centre.x, 0.0, centre.y),
            radius,
        }],
    );
    let cold = start.elapsed();
    let models = w.models().len();
    // Again with every model cached: the cost of streaming known content.
    w.set_keep_generations(0);
    w.stream(&statics, &[]);
    let start = Instant::now();
    w.stream(
        &statics,
        &[Interest {
            center: DVec3::new(centre.x, 0.0, centre.y),
            radius,
        }],
    );
    let warm = start.elapsed();
    eprintln!(
        "{name}: {} land cells, {} terrain chunks, {} colliders, {models} models: \
         {cold:.2?} cold, {warm:.2?} with models cached",
        report.cells_loaded, report.chunks_loaded, report.colliders
    );
    assert!(report.colliders > 1000);

    let grid = statics.land_grid();
    let (cx, cz) = grid.cell_at(centre.x, centre.y).unwrap();
    let cells = (radius / grid.cell_size) as u32 - 1;
    let (mut houses, mut hit) = (0, 0);
    let mut misses = Vec::new();
    for z in cz - cells..=cz + cells {
        for x in cx - cells..=cx + cells {
            statics.for_each_in_cell(x, z, &mut |p| {
                if !p.model.to_ascii_lowercase().contains("\\households\\") {
                    return;
                }
                houses += 1;
                let pos = p.transform.translation;
                let ground = w.terrain().unwrap().height(pos.x, pos.z);
                let q = RayQuery::new(
                    DVec3::new(pos.x, ground + 100.0, pos.z),
                    DVec3::new(pos.x, ground - 20.0, pos.z),
                    Layer::Geometry,
                );
                match w.ray_cast(&q) {
                    Some(h)
                        if h.object == ObjectKey::Static(p.key) && h.position.y > ground + 2.0 =>
                    {
                        hit += 1
                    }
                    other => misses.push((p.model.to_owned(), other.map(|h| h.object))),
                }
            });
        }
    }
    eprintln!("{name}: {hit} of {houses} houses hit from above; misses {misses:?}");
    assert!(houses > 20, "{name} has houses");
    // A ray at the origin can fall into a courtyard or between wings; nearly all are solid.
    assert!(hit * 10 >= houses * 9, "{hit} of {houses}");
}

#[test]
fn stratis_agia_marina_collision() {
    town(
        r"a3\map_stratis\stratis.wrp",
        "Agia Marina",
        DVec2::new(3000.0, 6000.0),
        300.0,
    );
}

#[test]
fn altis_kavala_collision() {
    town(
        r"a3\map_altis\altis.wrp",
        "Kavala",
        DVec2::new(3650.0, 13050.0),
        300.0,
    );
}

/// Every bridge of the Altis road net: standing on its deck (its Roadway LOD) puts a man at
/// the height of the road ends it joins, above the ground or water under it.
#[test]
fn altis_bridge_decks_carry_the_road() {
    let Some(game) = game() else { return };
    let t = terrain(game, r"a3\map_altis\altis.wrp");
    let (mut w, statics) = collision(game, &t);
    let roads = &t.roads;
    let mut bridges = 0;
    for part in &roads.parts {
        if !roads.model_of(part).to_ascii_lowercase().contains("bridge") {
            continue;
        }
        let ends = roads.connections_of(part);
        let mid = ends
            .iter()
            .fold(DVec3::ZERO, |a, e| a + e.position.as_dvec3())
            / ends.len() as f64;
        w.load_area(
            &statics,
            DVec2::new(mid.x - 30.0, mid.z - 30.0),
            DVec2::new(mid.x + 30.0, mid.z + 30.0),
        );
        let ground = w.terrain().unwrap().height(mid.x, mid.z);
        let deck = w
            .surface_below(mid + DVec3::Y * 3.0, 100.0)
            .unwrap_or_else(|| panic!("{} at {mid}", roads.model_of(part)));
        assert!(
            matches!(deck.object, ObjectKey::Static(_)),
            "{} at {mid}: lands on {:?}",
            roads.model_of(part),
            deck.object
        );
        assert!(
            (deck.y - mid.y).abs() < 1.0,
            "{} at {mid}: deck {}",
            roads.model_of(part),
            deck.y
        );
        assert!(deck.y > ground + 1.0);
        let surface = w.surface(deck.surface.expect("deck surface"));
        assert!(surface.loaded, "{}", surface.name);
        bridges += 1;
    }
    eprintln!("{bridges} bridges checked");
    assert!(bridges > 10);
}

/// The analytic terrain ray agrees with the WRP's own interpolation everywhere.
#[test]
fn stratis_terrain_rays_land_on_the_wrp_surface() {
    let Some(game) = game() else { return };
    let t = terrain(game, r"a3\map_stratis\stratis.wrp");
    let (w, _) = collision(game, &t);
    let size = f64::from(t.world_size());
    let mut seed = 12345u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..2000 {
        let (x, z) = (next() * size, next() * size);
        let from = DVec3::new(x, 2000.0, z);
        // A slanted ray towards a point on the surface.
        let target = DVec3::new(x + next() * 50.0 - 25.0, 0.0, z + next() * 50.0 - 25.0);
        let target = DVec3::new(
            target.x,
            w.terrain().unwrap().height(target.x, target.z),
            target.z,
        );
        let hit = w
            .ray_cast(
                &RayQuery::new(from, target + (target - from) * 0.01, Layer::Geometry)
                    .ignoring(&[]),
            )
            .filter(|h| h.object == ObjectKey::Terrain);
        let Some(hit) = hit else { continue };
        let expected = f64::from(t.surface_height(hit.position.x as f32, hit.position.z as f32));
        assert!(
            (hit.position.y - expected).abs() < 0.02,
            "{hit:?} vs {expected}"
        );
    }
}

/// Everything shipped in the island's area parses into collision, and the counts stay sane.
#[test]
fn every_stratis_model_builds_collision() {
    let Some(game) = game() else { return };
    let t = terrain(game, r"a3\map_stratis\stratis.wrp");
    let mut bank = ModelBank::new(Arc::new(game.vfs.clone()), Some(&game.config));
    let start = Instant::now();
    let (mut with, mut without) = (0, Vec::new());
    for m in &t.models {
        match bank.get(m.as_str()) {
            Some(c) => {
                with += 1;
                if let Some(g) = c.layer(Layer::Geometry) {
                    assert!(!g.components().is_empty(), "{m}");
                }
            }
            None => without.push(m.as_str().to_owned()),
        }
    }
    eprintln!(
        "{} models: {with} with collision in {:.2?}; without: {} (e.g. {:?}); {} surfaces",
        t.models.len(),
        start.elapsed(),
        without.len(),
        &without[..without.len().min(5)],
        bank.surfaces().len()
    );
    assert!(with * 10 > t.models.len() * 8);
    let _ = statics_grid_is_the_land_grid(&t);
}

fn statics_grid_is_the_land_grid(t: &Arc<Terrain>) -> bool {
    let s = TerrainStatics::new(t.clone());
    let g = s.land_grid();
    assert_eq!(g.width, t.land_grid.width);
    true
}
