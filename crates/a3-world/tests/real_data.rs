//! Loads shipped terrains into a World. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_pbo::Pbo;
use a3_world::{ClientId, ObjectRef, World};
use a3_wrp::Terrain;

/// Every creatable class of the shipped config builds an Entity type: CfgVehicles and
/// CfgNonAIVehicles classes with `scope >= 1`, and every CfgAmmo class with a `simulation`
/// (ammo is created by firing, whatever its scope).
#[test]
fn every_creatable_config_class_has_a_known_simulation() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let mut bank = a3_world::TypeBank::new(data.config.clone());
    let mut unknown = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut built = 0;
    for source in a3_world::TypeSource::ALL {
        for class in data.config.root().get(source.root_name()).entries() {
            if !class.is_class() {
                continue;
            }
            let simulation = class.get("simulation").text();
            let creatable = match source {
                a3_world::TypeSource::Ammo => !simulation.is_empty(),
                _ => class.get("scope").number() >= 1.0,
            };
            if !creatable {
                continue;
            }
            match bank.get(class.name()) {
                Ok(_) => built += 1,
                Err(a3_world::Error::UnknownSimulation { simulation, .. }) => unknown
                    .entry(simulation)
                    .or_default()
                    .push(class.name().to_owned()),
                Err(e) => panic!("{}: {e}", class.name()),
            }
        }
    }
    eprintln!("built {built} entity types");
    for (simulation, classes) in &unknown {
        eprintln!(
            "simulation {simulation:?}: {} classes, e.g. {:?}",
            classes.len(),
            &classes[..classes.len().min(5)]
        );
    }
    // Public classes without a `simulation` are config leftovers the engine cannot create
    // either; every non-empty value must be known.
    unknown.remove("");
    assert!(unknown.is_empty(), "unknown simulation values: {unknown:?}");
}

fn load(root: &Path, pbo: &str, wrp: &str) -> Terrain {
    let pbo = Pbo::open(root.join("Addons").join(pbo)).unwrap();
    Terrain::parse(&pbo.read(wrp).unwrap()).unwrap()
}

#[test]
fn shipped_terrains_load_as_static_objects_findable_by_object_id() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    for (pbo, wrp) in [
        ("map_stratis.pbo", "Stratis.wrp"),
        ("map_altis.pbo", "Altis.wrp"),
    ] {
        let terrain = load(Path::new(&root), pbo, wrp);
        let mut world = World::new(ClientId::SERVER);

        let terrain = std::sync::Arc::new(terrain);
        world.load_terrain(terrain.clone()).unwrap();

        assert_eq!(world.static_object_count(), terrain.objects.len(), "{wrp}");
        let step = (terrain.objects.len() / 2000).max(1);
        for object in terrain.objects.iter().step_by(step) {
            let near = object.transform.position().as_dvec3();
            let Some(ObjectRef::Static(key)) = world.find_static(near, object.id) else {
                panic!("{wrp}: object {} not found", object.id);
            };
            assert_eq!(world.static_object(key).unwrap().object_id, object.id);
            assert_eq!(
                world.resolve(key.network_id()),
                Some(ObjectRef::Static(key))
            );
        }
        eprintln!(
            "{wrp}: {} static objects, land grid {}x{}",
            world.static_object_count(),
            terrain.land_grid.width,
            terrain.land_grid.height
        );
    }
}
