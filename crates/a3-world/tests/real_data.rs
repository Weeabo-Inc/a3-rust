//! Loads shipped terrains into a World. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_pbo::Pbo;
use a3_world::{ClientId, ObjectRef, World};
use a3_wrp::Terrain;

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

        world.load_terrain(&terrain).unwrap();

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
