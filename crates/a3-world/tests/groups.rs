//! Groups, sides and side relations through the World interface.

use std::sync::Arc;

use a3_world::{
    ClientId, Create, EntityId, EntityType, Error, Locality, NetworkId, Side, SimulationClass,
    World, WorldEvent, default_group_name,
};
use glam::DVec3;

fn man(world: &mut World) -> EntityId {
    let ty = Arc::new(EntityType::new("B_Soldier_F", SimulationClass::Soldier));
    world.create(Create::new(ty, DVec3::ZERO)).unwrap()
}

#[test]
fn groups_get_network_ids_from_the_same_serial_as_objects() {
    let mut world = World::new(ClientId::SERVER);
    let a = man(&mut world);

    let g = world.create_group(Side::West, false);

    let group = world.group(g).unwrap();
    assert_eq!(group.network_id(), NetworkId::new(2, 2));
    assert_eq!(
        world.entity(a).unwrap().network_id(),
        Some(NetworkId::new(2, 1))
    );
    assert_eq!(world.resolve_group(group.network_id()), Some(g));
    assert!(group.is_local());
}

#[test]
fn default_names_count_per_side() {
    let mut world = World::new(ClientId::SERVER);

    let w1 = world.create_group(Side::West, false);
    let w2 = world.create_group(Side::West, false);
    let e1 = world.create_group(Side::East, false);

    assert_eq!(world.group(w1).unwrap().name(), "Alpha 1-1");
    assert_eq!(world.group(w2).unwrap().name(), "Alpha 1-2");
    assert_eq!(world.group(e1).unwrap().name(), "Alpha 1-1");
    assert_eq!(default_group_name(6), "Alpha 2-1");
    assert_eq!(default_group_name(24), "Bravo 1-1");
}

#[test]
fn joining_moves_units_between_groups_and_picks_leaders() {
    let mut world = World::new(ClientId::SERVER);
    let (a, b) = (man(&mut world), man(&mut world));
    let g1 = world.create_group(Side::West, false);
    let g2 = world.create_group(Side::West, true);

    world.join(a, g1).unwrap();
    world.join(b, g1).unwrap();
    assert_eq!(world.group(g1).unwrap().units(), [a, b]);
    assert_eq!(world.group(g1).unwrap().leader(), Some(a));

    world.join(a, g2).unwrap();
    assert_eq!(world.group(g1).unwrap().units(), [b]);
    assert_eq!(world.group(g1).unwrap().leader(), Some(b));
    assert_eq!(world.group_of(a), Some(g2));

    world.set_leader(g1, b).unwrap();
    assert!(matches!(world.set_leader(g1, a), Err(Error::NotInGroup(_))));
}

#[test]
fn delete_when_empty_groups_go_away_with_their_last_unit() {
    let mut world = World::new(ClientId::SERVER);
    let a = man(&mut world);
    let g = world.create_group(Side::East, true);
    world.join(a, g).unwrap();

    world.delete(a);
    world.flush_deletions();

    assert!(world.group(g).is_none());
}

#[test]
fn only_empty_groups_can_be_deleted() {
    let mut world = World::new(ClientId::SERVER);
    let a = man(&mut world);
    let g = world.create_group(Side::East, false);
    world.join(a, g).unwrap();

    assert!(matches!(
        world.delete_group(g),
        Err(Error::GroupNotEmpty(_))
    ));
    world.leave_group(a);
    world.delete_group(g).unwrap();
    assert!(world.group(g).is_none());
    assert!(world.group_of(a).is_none());
}

#[test]
fn units_take_their_groups_locality() {
    let mut world = World::new(ClientId::SERVER);
    let (a, b) = (man(&mut world), man(&mut world));
    let g = world.create_group(Side::West, false);
    world.join(a, g).unwrap();
    world.join(b, g).unwrap();
    world.drain_events();

    let remote = Locality::Remote {
        owner: Some(ClientId(5000)),
    };
    world.set_group_locality(g, remote).unwrap();

    assert_eq!(world.group(g).unwrap().locality(), remote);
    assert_eq!(world.entity(a).unwrap().locality(), remote);
    assert_eq!(
        world.drain_events(),
        [
            WorldEvent::LocalityChanged {
                entity: a,
                local: false
            },
            WorldEvent::LocalityChanged {
                entity: b,
                local: false
            },
        ]
    );
}

#[test]
fn units_take_their_groups_side() {
    let mut world = World::new(ClientId::SERVER);
    let a = man(&mut world);
    let g = world.create_group(Side::East, false);
    // `EntityType::new` types are civilians (config side 3).
    assert_eq!(world.side_of(a), Some(Side::Civilian));

    world.join(a, g).unwrap();

    assert_eq!(world.side_of(a), Some(Side::East));
}

#[test]
fn side_relations_have_the_original_defaults_and_can_change() {
    let mut world = World::new(ClientId::SERVER);

    assert!(world.is_enemy(Side::West, Side::East));
    assert!(!world.is_enemy(Side::Independent, Side::West));
    assert!(world.is_enemy(Side::Independent, Side::East));
    assert!(!world.is_enemy(Side::Civilian, Side::East));

    world.set_friendship(Side::Independent, Side::West, 0.0);

    assert!(world.is_enemy(Side::Independent, Side::West));
    assert!(
        !world.is_enemy(Side::West, Side::Independent),
        "one direction only"
    );
}

#[test]
fn remote_groups_keep_their_announced_network_id() {
    let mut world = World::new(ClientId(5000));
    let net = NetworkId::new(2, 40);

    let g = world
        .spawn_remote_group(Side::West, "Alpha 1-1".into(), net, Some(ClientId::SERVER))
        .unwrap();

    assert_eq!(world.resolve_group(net), Some(g));
    assert!(!world.group(g).unwrap().is_local());
    assert!(matches!(
        world.spawn_remote_group(Side::West, "x".into(), net, None),
        Err(Error::DuplicateNetworkId(_))
    ));
}
