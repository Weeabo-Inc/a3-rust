//! Behaviour of the World through its public interface.

use a3_world::{
    ClientId, EntityClass, EntitySpec, Error, Locality, LocalityChange, NetworkId, ObjectRef,
    SimulationClass, World,
};
use glam::DVec3;

const CLIENT: ClientId = ClientId(1_234_567);

fn spec(class: SimulationClass) -> EntitySpec {
    EntitySpec::new("B_Quadbike_01_F", class, DVec3::new(100.0, 5.0, 200.0))
}

#[test]
fn spawned_entities_are_local_with_a_network_id_from_this_client() {
    let mut world = World::new(CLIENT);

    let a = world.spawn(spec(SimulationClass::CarX));
    let b = world.spawn(spec(SimulationClass::CarX));

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

    let id = world.spawn_local_only(spec(SimulationClass::Thing));

    assert_eq!(world.entity(id).unwrap().network_id(), None);
    assert_eq!(world.entity(id).unwrap().locality(), Locality::Local);
}

#[test]
fn entities_resolve_by_network_id() {
    let mut world = World::new(CLIENT);
    let id = world.spawn(spec(SimulationClass::Soldier));
    let net = world.entity(id).unwrap().network_id().unwrap();

    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));
    assert_eq!(world.resolve(NetworkId::NULL), None);
    assert_eq!(world.resolve(NetworkId::new(99, 1)), None);
}

#[test]
fn remote_entities_keep_the_network_id_and_owner_they_arrive_with() {
    let mut world = World::new(CLIENT);
    let net = NetworkId::new(2, 17);

    let id = world
        .spawn_remote(spec(SimulationClass::TankX), net, Some(ClientId::SERVER))
        .unwrap();

    let e = world.entity(id).unwrap();
    assert_eq!(e.network_id(), Some(net));
    assert_eq!(
        e.locality(),
        Locality::Remote {
            owner: Some(ClientId::SERVER)
        }
    );
    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));
}

#[test]
fn a_network_id_cannot_be_used_twice() {
    let mut world = World::new(CLIENT);
    let net = NetworkId::new(2, 17);
    world
        .spawn_remote(spec(SimulationClass::Car), net, None)
        .unwrap();

    let err = world
        .spawn_remote(spec(SimulationClass::Car), net, None)
        .unwrap_err();

    assert!(matches!(err, Error::DuplicateNetworkId(n) if n == net));
}

#[test]
fn reserved_creators_are_rejected_for_remote_entities() {
    let mut world = World::new(CLIENT);

    for creator in [0, 1] {
        let err = world
            .spawn_remote(spec(SimulationClass::Car), NetworkId::new(creator, 5), None)
            .unwrap_err();
        assert!(matches!(err, Error::ReservedNetworkId(_)));
    }
}

#[test]
fn deleted_entities_do_not_resolve_and_their_ids_are_never_reused() {
    let mut world = World::new(CLIENT);
    let old = world.spawn(spec(SimulationClass::Thing));
    let net = world.entity(old).unwrap().network_id().unwrap();

    assert!(world.delete(old));
    let new = world.spawn(spec(SimulationClass::Thing));

    assert!(world.entity(old).is_none());
    assert!(!world.delete(old));
    assert_eq!(world.resolve(net), None);
    assert_ne!(old, new);
    assert_ne!(world.entity(new).unwrap().network_id(), Some(net));
}

#[test]
fn locality_changes_are_reported_once_per_change() {
    let mut world = World::new(CLIENT);
    let id = world
        .spawn_remote(
            spec(SimulationClass::Car),
            NetworkId::new(2, 1),
            Some(ClientId::SERVER),
        )
        .unwrap();

    world.set_locality(id, Locality::Local).unwrap();
    world.set_locality(id, Locality::Local).unwrap();
    world
        .set_locality(id, Locality::Remote { owner: None })
        .unwrap();

    assert_eq!(
        world.drain_locality_changes(),
        [
            LocalityChange {
                entity: id,
                local: true
            },
            LocalityChange {
                entity: id,
                local: false
            },
        ]
    );
    assert!(world.drain_locality_changes().is_empty());
}

#[test]
fn changing_the_owner_of_a_remote_entity_is_not_a_locality_change() {
    let mut world = World::new(ClientId::SERVER);
    let id = world
        .spawn_remote(
            spec(SimulationClass::Car),
            NetworkId::new(77, 1),
            Some(ClientId(77)),
        )
        .unwrap();

    world
        .set_locality(
            id,
            Locality::Remote {
                owner: Some(ClientId(78)),
            },
        )
        .unwrap();

    assert!(world.drain_locality_changes().is_empty());
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
    let id = world.spawn(spec(SimulationClass::Soldier));
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
}
