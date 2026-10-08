//! Static objects loaded from a terrain.

use a3_world::{
    ClientId, EntitySpec, Error, Locality, NetworkId, ObjectRef, SimulationClass, StaticKey, World,
};
use a3_wrp::{Terrain, TerrainBuilder, Transform};
use glam::{DVec3, Vec3};

/// A 4 x 4 land grid of 50 m cells with three objects: two in cell (0, 0), one in cell (2, 3).
fn terrain() -> Terrain {
    let at = |x, z| Transform::from_position(Vec3::new(x, 0.0, z));
    TerrainBuilder::new(4, 8, 50.0)
        .object(r"a3\trees\pine.p3d", at(10.0, 10.0))
        .object(r"a3\houses\house.p3d", at(120.0, 160.0))
        .object(r"a3\trees\pine.p3d", at(20.0, 30.0))
        .build()
}

fn world(client: ClientId) -> World {
    let mut world = World::new(client);
    world.load_terrain(&terrain()).unwrap();
    world
}

#[test]
fn every_placed_object_becomes_a_static_object() {
    let world = world(ClientId::SERVER);

    assert_eq!(world.static_object_count(), 3);
}

#[test]
fn static_objects_are_found_by_object_id_from_any_nearby_position() {
    let world = world(ClientId::SERVER);

    let Some(ObjectRef::Static(key)) = world.find_static(DVec3::new(0.0, 0.0, 0.0), 1) else {
        panic!("object 1 not found");
    };

    let object = world.static_object(key).unwrap();
    assert_eq!(object.object_id, 1);
    assert_eq!(object.position, DVec3::new(120.0, 0.0, 160.0));
    assert_eq!(key.cell(), (2, 3));
    assert_eq!(world.find_static(DVec3::ZERO, 99), None);
}

#[test]
fn objects_in_one_cell_get_consecutive_indices() {
    let world = world(ClientId::SERVER);

    let key_of = |id| match world.find_static(DVec3::ZERO, id) {
        Some(ObjectRef::Static(key)) => key,
        other => panic!("{other:?}"),
    };

    assert_eq!((key_of(0).cell(), key_of(0).index()), ((0, 0), 0));
    assert_eq!((key_of(2).cell(), key_of(2).index()), ((0, 0), 1));
}

#[test]
fn static_objects_resolve_by_their_creator_1_network_id() {
    let world = world(ClientId::SERVER);
    let Some(ObjectRef::Static(key)) = world.find_static(DVec3::ZERO, 2) else {
        panic!()
    };
    let net = key.network_id();

    assert_eq!(net.creator, 1);
    assert_eq!(world.resolve(net), Some(ObjectRef::Static(key)));
    assert_eq!(world.resolve(NetworkId::new(1, 0x7fff_ffff)), None);
}

#[test]
fn promoted_static_objects_keep_their_network_id() {
    let mut world = world(ClientId::SERVER);
    let Some(ObjectRef::Static(key)) = world.find_static(DVec3::ZERO, 1) else {
        panic!()
    };
    let spec = EntitySpec::new(
        "Land_House_1",
        SimulationClass::House,
        DVec3::new(120.0, 0.0, 160.0),
    );

    let id = world.promote_static(key, spec.clone()).unwrap();

    assert_eq!(
        world.entity(id).unwrap().network_id(),
        Some(key.network_id())
    );
    assert_eq!(world.resolve(key.network_id()), Some(ObjectRef::Entity(id)));
    assert_eq!(
        world.find_static(DVec3::ZERO, 1),
        Some(ObjectRef::Entity(id))
    );
    assert_eq!(world.promote_static(key, spec).unwrap(), id);
    assert_eq!(world.entity(id).unwrap().locality(), Locality::Local);
}

#[test]
fn on_a_client_promoted_static_objects_are_owned_by_the_server() {
    let mut world = world(ClientId(5000));
    let Some(ObjectRef::Static(key)) = world.find_static(DVec3::ZERO, 0) else {
        panic!()
    };

    let id = world
        .promote_static(
            key,
            EntitySpec::new("Land_Pine", SimulationClass::Thing, DVec3::ZERO),
        )
        .unwrap();

    assert_eq!(
        world.entity(id).unwrap().locality(),
        Locality::Remote {
            owner: Some(ClientId::SERVER)
        }
    );
}

#[test]
fn a_deleted_promoted_static_object_is_gone_for_good() {
    let mut world = world(ClientId::SERVER);
    let Some(ObjectRef::Static(key)) = world.find_static(DVec3::ZERO, 0) else {
        panic!()
    };
    let id = world
        .promote_static(
            key,
            EntitySpec::new("Land_Pine", SimulationClass::Thing, DVec3::ZERO),
        )
        .unwrap();

    world.delete(id);

    assert_eq!(world.resolve(key.network_id()), None);
    assert_eq!(world.find_static(DVec3::ZERO, 0), None);
    assert!(matches!(
        world.promote_static(
            key,
            EntitySpec::new("Land_Pine", SimulationClass::Thing, DVec3::ZERO)
        ),
        Err(Error::NoSuchStatic(_))
    ));
}

#[test]
fn static_object_refs_round_trip_through_sqf_handle_ids() {
    let key = StaticKey::new(1023, 7, 2047).unwrap();
    let r = ObjectRef::Static(key);

    assert_eq!(ObjectRef::from_handle_id(r.to_handle_id()), Some(r));
    assert_eq!(StaticKey::new(1024, 0, 0), None);
    assert_eq!(StaticKey::new(0, 0, 2048), None);
}

#[test]
fn network_ids_print_and_parse_like_net_id() {
    let net = NetworkId::new(2, 345);
    assert_eq!(net.to_string(), "2:345");
    assert_eq!("2:345".parse::<NetworkId>().unwrap(), net);
    // Static keys have bit 31 set: the original prints them through %d, so negative.
    let key = StaticKey::new(0, 0, 5).unwrap().network_id();
    assert_eq!(key.to_string(), "1:-2147483643");
    assert_eq!(key.to_string().parse::<NetworkId>().unwrap(), key);
    assert!("2".parse::<NetworkId>().is_err());
    assert!("a:b".parse::<NetworkId>().is_err());
}
