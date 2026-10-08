//! Behaviour of the World through its public interface.

use std::sync::Arc;

use a3_world::{
    ClientId, Create, EntityClass, EntityType, Error, ListKind, Locality, NetworkId, ObjectRef,
    POSITION_MAX, POSITION_MIN, SimulationClass, World, WorldEvent,
};
use glam::DVec3;

const CLIENT: ClientId = ClientId(1_234_567);

fn ty(name: &str, class: SimulationClass) -> Arc<EntityType> {
    Arc::new(EntityType::new(name, class))
}

fn create(class: SimulationClass) -> Create {
    Create::new(ty("B_Quadbike_01_F", class), DVec3::new(100.0, 5.0, 200.0))
}

/// A world with one remote Entity; the creation event is drained.
fn with_remote(
    client: ClientId,
    net: NetworkId,
    owner: Option<ClientId>,
) -> (World, a3_world::EntityId) {
    let mut world = World::new(client);
    let id = world
        .spawn_remote(
            ty("C_Offroad_01_F", SimulationClass::CarX),
            DVec3::ZERO,
            net,
            owner,
        )
        .unwrap();
    world.drain_events();
    (world, id)
}

#[test]
fn created_entities_are_local_with_a_network_id_from_this_client() {
    let mut world = World::new(CLIENT);

    let a = world.create(create(SimulationClass::CarX)).unwrap();
    let b = world.create(create(SimulationClass::CarX)).unwrap();

    let a = world.entity(a).unwrap();
    let b = world.entity(b).unwrap();
    assert_eq!(a.locality(), Locality::Local);
    assert_eq!(a.network_id().unwrap().creator, CLIENT.0);
    assert_eq!(b.network_id().unwrap().creator, CLIENT.0);
    assert_ne!(a.network_id(), b.network_id());
    assert_eq!(a.class(), SimulationClass::CarX);
    assert_eq!(a.type_name(), "B_Quadbike_01_F");
    assert_eq!(a.position(), DVec3::new(100.0, 5.0, 200.0));
}

#[test]
fn local_only_entities_have_no_network_id() {
    let mut world = World::new(ClientId::SERVER);

    let id = world
        .create(create(SimulationClass::Thing).local_only())
        .unwrap();

    assert_eq!(world.entity(id).unwrap().network_id(), None);
    assert_eq!(world.entity(id).unwrap().locality(), Locality::Local);
}

#[test]
fn abstract_types_cannot_be_created() {
    let mut world = World::new(CLIENT);
    let config = a3_config::parse_text(
        "class CfgVehicles { class Car { scope = 0; simulation = \"carx\"; }; };",
    )
    .unwrap();
    let mut bank = a3_world::TypeBank::new(Arc::new(a3_config::ConfigTree::from_config(&config)));
    let car = bank.get("Car").unwrap();

    let err = world.create(Create::new(car, DVec3::ZERO)).unwrap_err();

    assert!(matches!(err, Error::AbstractType(name) if name == "Car"));
    assert_eq!(world.entities().count(), 0);
}

#[test]
fn created_positions_are_clamped_to_the_original_box() {
    let mut world = World::new(CLIENT);

    let id = world
        .create(Create::new(
            ty("T", SimulationClass::Thing),
            DVec3::new(-1e9, 1e9, 12.0),
        ))
        .unwrap();

    assert_eq!(
        world.entity(id).unwrap().position(),
        DVec3::new(POSITION_MIN, POSITION_MAX, 12.0)
    );
}

#[test]
fn entities_are_sorted_into_simulation_lists_by_class() {
    let mut world = World::new(CLIENT);

    let man = world.create(create(SimulationClass::Soldier)).unwrap();
    let car = world.create(create(SimulationClass::CarX)).unwrap();
    let bullet = world.create(create(SimulationClass::ShotBullet)).unwrap();
    let trigger = world.create(create(SimulationClass::Detector)).unwrap();

    assert_eq!(world.list(ListKind::Vehicles), [man, car]);
    assert_eq!(world.list(ListKind::Projectiles), [bullet]);
    assert_eq!(world.list(ListKind::Slow), [trigger]);
    assert_eq!(world.entity(bullet).unwrap().list(), ListKind::Projectiles);
}

#[test]
fn creation_is_reported_as_an_event() {
    let mut world = World::new(CLIENT);

    let id = world.create(create(SimulationClass::CarX)).unwrap();

    assert_eq!(world.drain_events(), [WorldEvent::EntityCreated(id)]);
    assert!(world.drain_events().is_empty());
}

#[test]
fn entities_resolve_by_network_id() {
    let mut world = World::new(CLIENT);
    let id = world.create(create(SimulationClass::Soldier)).unwrap();
    let net = world.entity(id).unwrap().network_id().unwrap();

    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));
    assert_eq!(world.resolve(NetworkId::NULL), None);
    assert_eq!(world.resolve(NetworkId::new(99, 1)), None);
}

#[test]
fn remote_entities_keep_the_network_id_and_owner_they_arrive_with() {
    let net = NetworkId::new(2, 17);

    let (world, id) = with_remote(CLIENT, net, Some(ClientId::SERVER));

    let e = world.entity(id).unwrap();
    assert_eq!(e.network_id(), Some(net));
    assert_eq!(
        e.locality(),
        Locality::Remote {
            owner: Some(ClientId::SERVER)
        }
    );
    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));
    assert_eq!(world.list(ListKind::Vehicles), [id]);
}

#[test]
fn a_network_id_cannot_be_used_twice() {
    let net = NetworkId::new(2, 17);
    let (mut world, _) = with_remote(CLIENT, net, None);

    let err = world
        .spawn_remote(ty("C", SimulationClass::Car), DVec3::ZERO, net, None)
        .unwrap_err();

    assert!(matches!(err, Error::DuplicateNetworkId(n) if n == net));
}

#[test]
fn reserved_creators_are_rejected_for_remote_entities() {
    let mut world = World::new(CLIENT);

    for creator in [0, 1] {
        let err = world
            .spawn_remote(
                ty("C", SimulationClass::Car),
                DVec3::ZERO,
                NetworkId::new(creator, 5),
                None,
            )
            .unwrap_err();
        assert!(matches!(err, Error::ReservedNetworkId(_)));
    }
}

#[test]
fn deletion_takes_effect_at_the_end_of_the_step() {
    let mut world = World::new(CLIENT);
    let id = world.create(create(SimulationClass::Thing)).unwrap();
    let net = world.entity(id).unwrap().network_id().unwrap();
    world.drain_events();

    assert!(world.delete(id));
    assert!(!world.delete(id), "already scheduled");

    assert!(world.entity(id).unwrap().is_deleted());
    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));

    world.flush_deletions();

    assert!(world.entity(id).is_none());
    assert_eq!(world.resolve(net), None);
    assert!(world.list(ListKind::Vehicles).is_empty());
    assert_eq!(
        world.drain_events(),
        [WorldEvent::EntityDeleted {
            entity: id,
            network_id: Some(net)
        }]
    );
    assert!(!world.delete(id));
}

#[test]
fn deleted_ids_are_never_reused() {
    let mut world = World::new(CLIENT);
    let old = world.create(create(SimulationClass::Thing)).unwrap();
    let net = world.entity(old).unwrap().network_id().unwrap();
    world.delete(old);
    world.flush_deletions();

    let new = world.create(create(SimulationClass::Thing)).unwrap();

    assert_ne!(old, new);
    assert!(world.entity(old).is_none());
    assert_ne!(world.entity(new).unwrap().network_id(), Some(net));
}

#[test]
fn locality_changes_are_reported_once_per_change() {
    let (mut world, id) = with_remote(CLIENT, NetworkId::new(2, 1), Some(ClientId::SERVER));

    world.set_locality(id, Locality::Local).unwrap();
    world.set_locality(id, Locality::Local).unwrap();
    world
        .set_locality(id, Locality::Remote { owner: None })
        .unwrap();

    assert_eq!(
        world.drain_events(),
        [
            WorldEvent::LocalityChanged {
                entity: id,
                local: true
            },
            WorldEvent::LocalityChanged {
                entity: id,
                local: false
            },
        ]
    );
}

#[test]
fn changing_the_owner_of_a_remote_entity_is_not_a_locality_change() {
    let (mut world, id) = with_remote(ClientId::SERVER, NetworkId::new(77, 1), Some(ClientId(77)));

    world
        .set_locality(
            id,
            Locality::Remote {
                owner: Some(ClientId(78)),
            },
        )
        .unwrap();

    assert!(world.drain_events().is_empty());
    assert_eq!(
        world.entity(id).unwrap().locality(),
        Locality::Remote {
            owner: Some(ClientId(78))
        }
    );
}

#[test]
fn object_refs_round_trip_through_sqf_handle_ids() {
    let mut world = World::new(CLIENT);
    let id = world.create(create(SimulationClass::Soldier)).unwrap();
    let r = ObjectRef::Entity(id);

    let handle = r.to_handle_id();

    assert_ne!(handle, 0, "0 is objNull");
    assert_eq!(ObjectRef::from_handle_id(handle), Some(r));
    assert_eq!(ObjectRef::from_handle_id(0), None);
}

#[test]
fn simulation_values_from_config_map_to_engine_classes() {
    assert_eq!(
        SimulationClass::from_simulation("carx"),
        Some(SimulationClass::CarX)
    );
    assert_eq!(
        SimulationClass::from_simulation("HelicopterRTD"),
        Some(SimulationClass::HelicopterRtd)
    );
    assert_eq!(
        SimulationClass::from_simulation("soldier"),
        Some(SimulationClass::Soldier)
    );
    assert_eq!(SimulationClass::from_simulation("nonsense"), None);
}

#[test]
fn engine_kind_questions_follow_the_original_class_tree() {
    use EntityClass::*;
    assert!(SimulationClass::CarX.is_kind_of(Car));
    assert!(SimulationClass::CarX.is_kind_of(TankOrCar));
    assert!(SimulationClass::CarX.is_kind_of(Transport));
    assert!(SimulationClass::CarX.is_kind_of(EntityAi));
    assert!(SimulationClass::CarX.is_kind_of(Entity));
    assert!(!SimulationClass::CarX.is_kind_of(Person));
    assert!(SimulationClass::Soldier.is_kind_of(Man));
    assert!(SimulationClass::Soldier.is_kind_of(Person));
    assert!(!SimulationClass::Soldier.is_kind_of(Transport));
    assert!(SimulationClass::HelicopterRtd.is_kind_of(PlaneOrHeli));
    assert!(SimulationClass::House.is_kind_of(Building));
    assert!(!SimulationClass::House.is_kind_of(EntityAiFull));
    assert!(SimulationClass::Thing.is_kind_of(EntityAi));
    assert!(SimulationClass::ShotBullet.is_kind_of(Shot));
}
