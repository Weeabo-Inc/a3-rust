//! The network-facing World operations (issue #131): remote create, update, delete and owner
//! change, the updates this machine owes a receiver, and the state a joining client needs.
//!
//! There is no transport here. Each test builds the decoded contents of a message as a fixture
//! and checks what the World does with it; encoding and decoding the bytes is Phase 6's job
//! (`docs/re/net-message-formats.tsv`).

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_world::{
    ClientId, Create, DAMAGE_ERROR_SCALE, EntityId, Error, JipObject, ListKind, Locality,
    NetworkId, ObjectRef, OwedUpdate, RemoteCreate, RemoteUpdate, ReplicatedState,
    STATE_FLAG_ERROR, SimulationClass, TypeBank, UpdateClass, UpdateError, UpdateOutcome, World,
    WorldEvent,
};
use glam::{DQuat, DVec3};
use proptest::prelude::*;

const SERVER: ClientId = ClientId::SERVER;
/// Another machine's id (`NetworkServer` hands out large ids).
const CLIENT: ClientId = ClientId(1_234_567);

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class C_Offroad_01_F: All { scope = 2; simulation = "car"; };
    class B_MRAP_01_F: All { scope = 2; simulation = "tankx"; };
    class B_Soldier_F: All { scope = 2; simulation = "soldier"; };
    class B_762x51_Ball: All { scope = 2; simulation = "shotbullet"; };
};
"#;

fn bank() -> TypeBank {
    let config = parse_text(CONFIG).unwrap();
    TypeBank::new(Arc::new(ConfigTree::from_config(&config)))
}

/// Creates a local Entity through [`World::create`], the `createVehicle` path.
fn create_local(world: &mut World, types: &mut TypeBank, name: &str, position: DVec3) -> EntityId {
    let ty = types.get(name).unwrap();
    world.create(Create::new(ty, position)).unwrap()
}

/// A remote create message's contents for `name`, at `position`.
fn create(net: NetworkId, name: &str, position: DVec3) -> RemoteCreate {
    RemoteCreate {
        network_id: net,
        type_name: name.to_owned(),
        state: ReplicatedState::at(position),
    }
}

/// The Entity a Network object ID resolves to on this machine (`None` for a Static object).
fn entity_of(world: &World, net: NetworkId) -> Option<EntityId> {
    match world.resolve(net)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

/// The replicated state of the Entity a Network object ID resolves to.
fn entity_state(world: &World, net: NetworkId) -> Option<ReplicatedState> {
    world
        .entity(entity_of(world, net)?)
        .map(|e| e.replicated_state())
}

// -- remote create ----------------------------------------------------------------------------

#[test]
fn a_remote_create_builds_a_remote_entity_of_the_named_type() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let state = ReplicatedState {
        position: DVec3::new(1.0, 2.0, 3.0),
        orientation: DQuat::from_rotation_y(0.5),
        velocity: DVec3::new(0.0, 0.0, 5.0),
        damage: 0.25,
        hidden: true,
    };

    let id = world
        .apply_remote_create(
            &mut types,
            RemoteCreate {
                network_id: net,
                type_name: "C_Offroad_01_F".into(),
                state,
            },
        )
        .unwrap();

    let e = world.entity(id).unwrap();
    assert_eq!(e.network_id(), Some(net));
    assert_eq!(e.type_name(), "C_Offroad_01_F");
    assert_eq!(e.class(), SimulationClass::Car);
    assert_eq!(e.list(), ListKind::Vehicles);
    assert_eq!(e.replicated_state(), state);
    // The creating machine owns what it created (`docs/re/world-object-model.md`, "The initial
    // owner: the machine that creates the object").
    assert_eq!(
        e.locality(),
        Locality::Remote {
            owner: Some(ClientId(7))
        }
    );
    assert_eq!(world.resolve(net), Some(ObjectRef::Entity(id)));
    assert_eq!(world.drain_events(), vec![WorldEvent::EntityCreated(id)]);
}

#[test]
fn a_remote_create_of_an_unknown_type_is_refused() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let err = world
        .apply_remote_create(
            &mut types,
            create(NetworkId::new(7, 1), "C_Not_In_The_Config", DVec3::ZERO),
        )
        .unwrap_err();
    assert_eq!(err, Error::UnknownType("C_Not_In_The_Config".into()));
    assert!(world.entities().next().is_none());
}

#[test]
fn a_remote_create_refuses_reserved_and_duplicate_network_ids() {
    let mut types = bank();
    let mut world = World::new(SERVER);

    // Creator 0 is null and creator 1 is a Static object: neither is created by a message.
    assert_eq!(
        world
            .apply_remote_create(
                &mut types,
                create(NetworkId::NULL, "C_Offroad_01_F", DVec3::ZERO)
            )
            .unwrap_err(),
        Error::ReservedNetworkId(NetworkId::NULL)
    );
    let stat = NetworkId::new(NetworkId::STATIC_CREATOR, 0x4000_0000);
    assert_eq!(
        world
            .apply_remote_create(&mut types, create(stat, "C_Offroad_01_F", DVec3::ZERO))
            .unwrap_err(),
        Error::ReservedNetworkId(stat)
    );

    let net = NetworkId::new(7, 3);
    world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    assert_eq!(
        world
            .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
            .unwrap_err(),
        Error::DuplicateNetworkId(net)
    );
}

#[test]
fn a_remote_create_for_an_id_this_machine_created_is_refused() {
    // The network layer must not feed this machine's own create back to it: that would build a
    // second, never-simulated copy of an object we own.
    let mut types = bank();
    let mut world = World::new(CLIENT);
    let own = create_local(&mut world, &mut types, "C_Offroad_01_F", DVec3::ZERO);
    let net = world.entity(own).unwrap().network_id().unwrap();

    assert_eq!(
        world
            .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
            .unwrap_err(),
        Error::OwnNetworkId(net)
    );
    assert_eq!(world.entities().count(), 1);
}

// -- remote update ----------------------------------------------------------------------------

#[test]
fn a_remote_update_applies_the_items_the_message_carries() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    world.drain_events();

    let outcome = world
        .apply_remote_update(
            RemoteUpdate::new(net)
                .position(DVec3::new(10.0, 0.0, 20.0))
                .velocity(DVec3::new(0.0, 0.0, 8.0))
                .damage(0.5),
        )
        .unwrap();

    assert_eq!(outcome, UpdateOutcome::Applied(id));
    let e = world.entity(id).unwrap();
    assert_eq!(e.position(), DVec3::new(10.0, 0.0, 20.0));
    assert_eq!(e.velocity(), DVec3::new(0.0, 0.0, 8.0));
    assert_eq!(e.damage(), 0.5);
    // Items the message does not carry are untouched.
    assert_eq!(e.orientation(), DQuat::IDENTITY);
    assert!(!e.is_hidden());
    // Applying an update is not a World event.
    assert!(world.drain_events().is_empty());
}

#[test]
fn a_remote_update_of_an_entity_this_machine_owns_is_ignored() {
    // The original logs "Client: Object (id %d:%d, type %s) is local - update is ignored."
    // (`docs/re/net-message-dispatch.tsv`).
    let mut types = bank();
    let mut world = World::new(SERVER);
    let id = create_local(
        &mut world,
        &mut types,
        "C_Offroad_01_F",
        DVec3::new(1.0, 0.0, 1.0),
    );
    let net = world.entity(id).unwrap().network_id().unwrap();

    let outcome = world
        .apply_remote_update(RemoteUpdate::new(net).position(DVec3::new(99.0, 0.0, 99.0)))
        .unwrap();

    assert_eq!(outcome, UpdateOutcome::IgnoredLocal(id));
    assert_eq!(
        world.entity(id).unwrap().position(),
        DVec3::new(1.0, 0.0, 1.0)
    );
}

#[test]
fn a_remote_update_of_an_unknown_or_static_id_is_refused() {
    let mut world = World::new(SERVER);
    let unknown = NetworkId::new(7, 3);
    assert_eq!(
        world
            .apply_remote_update(RemoteUpdate::new(unknown).position(DVec3::ZERO))
            .unwrap_err(),
        Error::NoSuchObject(unknown)
    );
    // Static objects (creator 1) are never updated by a message: every machine has them from the
    // WRP (`docs/re/net-object-model.md`).
    let stat = NetworkId::new(NetworkId::STATIC_CREATOR, 0x4000_0000);
    assert_eq!(
        world
            .apply_remote_update(RemoteUpdate::new(stat).damage(1.0))
            .unwrap_err(),
        Error::NoSuchObject(stat)
    );
}

// -- remote delete ----------------------------------------------------------------------------

#[test]
fn a_remote_delete_takes_effect_at_the_end_of_the_step() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    world.drain_events();

    assert_eq!(world.apply_remote_delete(net).unwrap(), id);
    // Like `deleteVehicle`, the Entity stays until the deletions are flushed.
    assert!(world.entity(id).unwrap().is_deleted());
    // A duplicate delete message before the flush is not an error.
    assert_eq!(world.apply_remote_delete(net).unwrap(), id);

    world.flush_deletions();
    assert!(world.entity(id).is_none());
    assert_eq!(world.resolve(net), None);
    assert_eq!(
        world.drain_events(),
        vec![WorldEvent::EntityDeleted {
            entity: id,
            network_id: Some(net),
        }]
    );
    // The id is never reused.
    assert_eq!(
        world.apply_remote_delete(net).unwrap_err(),
        Error::NoSuchObject(net)
    );
}

#[test]
fn a_remote_delete_of_an_unknown_id_is_refused() {
    let mut world = World::new(SERVER);
    let unknown = NetworkId::new(7, 3);
    assert_eq!(
        world.apply_remote_delete(unknown).unwrap_err(),
        Error::NoSuchObject(unknown)
    );
}

// -- owner change -----------------------------------------------------------------------------

#[test]
fn an_owner_change_flips_locality_and_records_it() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    world.drain_events();

    assert_eq!(world.owner(net), Some(ClientId(7)));
    assert_eq!(world.apply_owner_change(net, SERVER).unwrap(), id);

    assert_eq!(world.entity(id).unwrap().locality(), Locality::Local);
    assert_eq!(world.owner(net), Some(SERVER));
    assert_eq!(
        world.drain_events(),
        vec![WorldEvent::LocalityChanged {
            entity: id,
            local: true
        }]
    );

    // Giving it up to another machine records the loss of ownership.
    assert_eq!(world.apply_owner_change(net, CLIENT).unwrap(), id);
    assert_eq!(
        world.entity(id).unwrap().locality(),
        Locality::Remote {
            owner: Some(CLIENT)
        }
    );
    assert_eq!(world.owner(net), Some(CLIENT));
    assert_eq!(
        world.drain_events(),
        vec![WorldEvent::LocalityChanged {
            entity: id,
            local: false
        }]
    );

    // A change between two other machines is not a change here.
    assert_eq!(world.apply_owner_change(net, ClientId(9)).unwrap(), id);
    assert_eq!(world.owner(net), Some(ClientId(9)));
    assert!(world.drain_events().is_empty());

    let unknown = NetworkId::new(7, 9);
    assert_eq!(
        world.apply_owner_change(unknown, CLIENT).unwrap_err(),
        Error::NoSuchObject(unknown)
    );
}

#[test]
fn the_server_accepts_an_owner_change_only_from_the_owner() {
    // Message 355 is C→S and the server checks the sender owns the object
    // ("Server: OwnerChanged of %d:%d arrived from non owner %d", `docs/re/world-object-model.md`).
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();

    let err = world.receive_owner_change(CLIENT, net, CLIENT).unwrap_err();
    assert_eq!(
        err,
        Error::NotOwner {
            network_id: net,
            sender: CLIENT
        }
    );
    assert_eq!(world.owner(net), Some(ClientId(7)));

    assert_eq!(
        world
            .receive_owner_change(ClientId(7), net, SERVER)
            .unwrap(),
        id
    );
    assert_eq!(world.entity(id).unwrap().locality(), Locality::Local);
}

// -- update error metrics ---------------------------------------------------------------------

#[test]
fn the_update_error_follows_the_documented_metrics() {
    let base = ReplicatedState::at(DVec3::ZERO);
    assert!(UpdateError::between(&base, &base).is_zero());

    // Each changed state flag adds 10000 (`docs/re/world-object-model.md`, static objects).
    let hidden = ReplicatedState {
        hidden: true,
        ..base
    };
    let error = UpdateError::between(&base, &hidden);
    assert_eq!(error.of(UpdateClass::State), STATE_FLAG_ERROR);
    assert_eq!(error.total(), STATE_FLAG_ERROR);

    // Damage: 10 * |Δdamage| (same rule).
    let hurt = ReplicatedState {
        damage: 0.5,
        ..base
    };
    let error = UpdateError::between(&base, &hurt);
    assert_eq!(error.of(UpdateClass::Damage), DAMAGE_ERROR_SCALE * 0.5);
    assert_eq!(error.of(UpdateClass::State), 0.0);
    assert_eq!(error.of(UpdateClass::Transform), 0.0);

    // The transform metric is ours: the RE does not fix its coefficients (issue #117).
    let moved = ReplicatedState::at(DVec3::new(3.0, 4.0, 0.0));
    let error = UpdateError::between(&base, &moved);
    assert!(error.of(UpdateClass::Transform) > 0.0);
    assert_eq!(error.of(UpdateClass::Damage), 0.0);
}

#[test]
fn only_local_entities_are_owed_and_only_against_the_receiver_s_last_sent_state() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let local = create_local(&mut world, &mut types, "C_Offroad_01_F", DVec3::ZERO);
    let own_net = world.entity(local).unwrap().network_id().unwrap();
    let remote_net = NetworkId::new(9, 1);
    world
        .apply_remote_create(
            &mut types,
            create(remote_net, "C_Offroad_01_F", DVec3::ZERO),
        )
        .unwrap();

    // Nothing is owed until the receiver has a baseline for the object.
    world
        .entity_mut(local)
        .unwrap()
        .set_position(DVec3::new(1.0, 0.0, 0.0));
    assert!(world.updates_owed(CLIENT).is_empty());

    let baseline = world.entity(local).unwrap().replicated_state();
    world.mark_sent(CLIENT, own_net, baseline);
    assert!(world.updates_owed(CLIENT).is_empty());

    // A local change is owed; the remote object is not ours to send.
    world
        .entity_mut(local)
        .unwrap()
        .set_position(DVec3::new(4.0, 0.0, 0.0));
    let owed = world.updates_owed(CLIENT);
    assert_eq!(owed.len(), 1);
    assert_eq!(owed[0].entity, local);
    assert_eq!(owed[0].network_id, own_net);
    assert_eq!(owed[0].state.position, DVec3::new(4.0, 0.0, 0.0));
    assert!(owed[0].error.of(UpdateClass::Transform) > 0.0);
    // Another receiver has no baseline for it either.
    assert!(world.updates_owed(SERVER).is_empty());

    // Sending it settles that receiver's baseline; the next call has nothing to send.
    let update = owed[0];
    world.mark_sent(CLIENT, update.network_id, update.state);
    assert!(world.updates_owed(CLIENT).is_empty());

    // Baselines are per receiver: what the client has been sent is still owed to the server.
    world.mark_sent(SERVER, own_net, ReplicatedState::at(DVec3::ZERO));
    assert_eq!(world.updates_owed(SERVER).len(), 1);
}

#[test]
fn an_update_is_owed_again_after_a_state_flag_changes() {
    // The error is measured against the state the receiver was last sent, so a flag flip is owed
    // even though the transform did not move.
    let mut types = bank();
    let mut world = World::new(SERVER);
    let id = create_local(&mut world, &mut types, "C_Offroad_01_F", DVec3::ZERO);
    let net = world.entity(id).unwrap().network_id().unwrap();
    let baseline = world.entity(id).unwrap().replicated_state();
    world.mark_sent(CLIENT, net, baseline);
    assert!(world.updates_owed(CLIENT).is_empty());

    world.entity_mut(id).unwrap().set_hidden(true);
    let owed = world.updates_owed(CLIENT);
    assert_eq!(owed.len(), 1);
    assert_eq!(owed[0].error.of(UpdateClass::State), STATE_FLAG_ERROR);
}

#[test]
fn owed_updates_are_ordered_by_error_then_network_id() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = create_local(&mut world, &mut types, "C_Offroad_01_F", DVec3::ZERO);
        let net = world.entity(id).unwrap().network_id().unwrap();
        world.mark_sent(CLIENT, net, ReplicatedState::at(DVec3::ZERO));
        ids.push((id, net));
    }

    // The first and the third move the same distance; the second moves far.
    world
        .entity_mut(ids[0].0)
        .unwrap()
        .set_position(DVec3::new(10.0, 0.0, 0.0));
    world
        .entity_mut(ids[1].0)
        .unwrap()
        .set_position(DVec3::new(1000.0, 0.0, 0.0));
    world
        .entity_mut(ids[2].0)
        .unwrap()
        .set_position(DVec3::new(0.0, 10.0, 0.0));

    let owed: Vec<OwedUpdate> = world.updates_owed(CLIENT);
    assert_eq!(
        owed.iter().map(|u| u.network_id).collect::<Vec<_>>(),
        vec![ids[1].1, ids[0].1, ids[2].1]
    );
    assert!(owed[0].error.total() > owed[1].error.total());
    // A tie keeps the Network object ID order, so two machines compute the same message order.
    assert_eq!(owed[1].error.total(), owed[2].error.total());
    assert_eq!(
        world
            .updates_owed(CLIENT)
            .iter()
            .map(|u| u.network_id)
            .collect::<Vec<_>>(),
        owed.iter().map(|u| u.network_id).collect::<Vec<_>>()
    );
}

#[test]
fn local_only_entities_are_never_owed() {
    // `createVehicleLocal` objects have no Network object ID, so no receiver can be told about one.
    let mut types = bank();
    let mut world = World::new(SERVER);
    let ty = types.get("B_MRAP_01_F").unwrap();
    let id = world
        .create(Create::new(ty, DVec3::ZERO).local_only())
        .unwrap();
    world
        .entity_mut(id)
        .unwrap()
        .set_position(DVec3::new(5.0, 0.0, 0.0));

    assert!(world.updates_owed(CLIENT).is_empty());
    assert!(world.jip_snapshot().is_empty());
}

#[test]
fn a_received_transform_is_drawn_where_it_says() {
    // A step gives the renderer an interpolation history; a received transform anchors it there
    // instead of blending from the state before it (interpolation between received states is open,
    // issue #117).
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(7, 3);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    world.simulate(1.0 / 15.0);
    assert!(world.entity(id).unwrap().visual_state().last_step > 0.0);

    world
        .apply_remote_update(RemoteUpdate::new(net).position(DVec3::new(100.0, 0.0, 0.0)))
        .unwrap();

    assert_eq!(
        world.entity(id).unwrap().render_position(),
        DVec3::new(100.0, 0.0, 0.0)
    );
}

#[test]
fn an_update_the_receiver_never_had_a_baseline_for_is_owed_after_the_create() {
    // The network layer's flow: create, mark sent, then only changes are owed.
    let mut types = bank();
    let mut world = World::new(SERVER);
    let net = NetworkId::new(9, 1);
    let id = world
        .apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    // The object is another machine's: this machine owes nothing for it either way.
    assert!(world.updates_owed(CLIENT).is_empty());

    // A local object the receiver has been told about (a create this machine sent).
    let local = create_local(
        &mut world,
        &mut types,
        "C_Offroad_01_F",
        DVec3::new(1.0, 0.0, 0.0),
    );
    let local_net = world.entity(local).unwrap().network_id().unwrap();
    world.mark_sent(
        CLIENT,
        local_net,
        world.entity(local).unwrap().replicated_state(),
    );
    assert!(world.updates_owed(CLIENT).is_empty());
    assert_eq!(world.entity(id).unwrap().network_id(), Some(net));

    // Marking an unknown object sent is harmless.
    world.mark_sent(
        CLIENT,
        NetworkId::new(9, 999),
        ReplicatedState::at(DVec3::ZERO),
    );
    assert!(world.updates_owed(CLIENT).is_empty());
}

// -- JIP --------------------------------------------------------------------------------------

#[test]
fn a_jip_snapshot_holds_every_networked_object_in_arena_order() {
    let mut types = bank();
    let mut world = World::new(SERVER);
    let a = create_local(
        &mut world,
        &mut types,
        "C_Offroad_01_F",
        DVec3::new(1.0, 0.0, 0.0),
    );
    // Local-only objects are nobody else's business.
    let hidden = types.get("B_MRAP_01_F").unwrap();
    world
        .create(Create::new(hidden, DVec3::new(2.0, 0.0, 0.0)).local_only())
        .unwrap();
    let remote_net = NetworkId::new(9, 1);
    let b = world
        .apply_remote_create(
            &mut types,
            create(remote_net, "B_Soldier_F", DVec3::new(3.0, 0.0, 0.0)),
        )
        .unwrap();
    let a_net = world.entity(a).unwrap().network_id().unwrap();

    let snapshot: Vec<JipObject> = world.jip_snapshot();
    assert_eq!(
        snapshot.iter().map(|o| o.network_id).collect::<Vec<_>>(),
        vec![a_net, remote_net]
    );
    assert_eq!(snapshot[0].type_name, "C_Offroad_01_F");
    assert_eq!(snapshot[0].owner, Some(SERVER));
    assert_eq!(snapshot[0].state.position, DVec3::new(1.0, 0.0, 0.0));
    assert_eq!(snapshot[1].type_name, "B_Soldier_F");
    assert_eq!(snapshot[1].owner, Some(ClientId(9)));
    assert_eq!(snapshot[1].state.position, DVec3::new(3.0, 0.0, 0.0));

    // Objects on their way out are not part of the world a client joins.
    world.delete(b);
    assert_eq!(world.jip_snapshot().len(), 1);
}

#[test]
fn a_joining_client_can_be_built_from_the_snapshot_and_then_follows_updates() {
    let mut types = bank();
    let mut server = World::new(SERVER);
    let a = create_local(
        &mut server,
        &mut types,
        "C_Offroad_01_F",
        DVec3::new(1.0, 0.0, 2.0),
    );
    let b = create_local(
        &mut server,
        &mut types,
        "B_Soldier_F",
        DVec3::new(3.0, 0.0, 4.0),
    );

    // The server sends the snapshot and remembers it as each object's per-receiver baseline.
    let mut client = World::new(CLIENT);
    let mut client_types = bank();
    for object in server.jip_snapshot() {
        server.mark_sent(CLIENT, object.network_id, object.state);
        client
            .apply_remote_create(
                &mut client_types,
                RemoteCreate {
                    network_id: object.network_id,
                    type_name: object.type_name.clone(),
                    state: object.state,
                },
            )
            .unwrap();
    }
    assert_eq!(client.entities().count(), 2);
    assert!(server.updates_owed(CLIENT).is_empty());
    for id in [a, b] {
        let net = server.entity(id).unwrap().network_id().unwrap();
        assert_eq!(
            client
                .entity(entity_of(&client, net).unwrap())
                .unwrap()
                .position(),
            server.entity(id).unwrap().position()
        );
    }

    // The server moves both; the client applies what it is sent and ends up in step.
    server
        .entity_mut(a)
        .unwrap()
        .set_position(DVec3::new(11.0, 0.0, 12.0));
    server
        .entity_mut(b)
        .unwrap()
        .set_position(DVec3::new(30.0, 0.0, 40.0));
    server.entity_mut(b).unwrap().set_damage(0.75);

    let owed: Vec<OwedUpdate> = server.updates_owed(CLIENT);
    assert_eq!(owed.len(), 2);
    for update in &owed {
        // Phase 6 encodes `update` here; this test is the World side of that.
        server.mark_sent(CLIENT, update.network_id, update.state);
        client
            .apply_remote_update(
                RemoteUpdate::new(update.network_id)
                    .position(update.state.position)
                    .orientation(update.state.orientation)
                    .velocity(update.state.velocity)
                    .damage(update.state.damage)
                    .hidden(update.state.hidden),
            )
            .unwrap();
    }
    assert!(server.updates_owed(CLIENT).is_empty());
    for id in [a, b] {
        let net = server.entity(id).unwrap().network_id().unwrap();
        assert_eq!(
            entity_state(&client, net),
            Some(server.entity(id).unwrap().replicated_state())
        );
    }
}

// -- determinism ------------------------------------------------------------------------------

#[test]
fn operations_apply_in_call_order_and_record_events_oldest_first() {
    // What the network layer relies on: no deferred reordering inside the World. Updates land
    // when they are applied, and the events of a batch of creates, deletes and owner changes come
    // out in the order the messages were applied.
    let mut types = bank();
    let mut world = World::new(SERVER);
    let a_net = NetworkId::new(7, 1);
    let b_net = NetworkId::new(7, 2);
    let a = world
        .apply_remote_create(&mut types, create(a_net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();
    let b = world
        .apply_remote_create(&mut types, create(b_net, "C_Offroad_01_F", DVec3::ZERO))
        .unwrap();

    world
        .apply_remote_update(RemoteUpdate::new(a_net).position(DVec3::new(1.0, 0.0, 0.0)))
        .unwrap();
    world
        .apply_remote_update(RemoteUpdate::new(a_net).position(DVec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(
        world.entity(a).unwrap().position(),
        DVec3::new(2.0, 0.0, 0.0)
    );

    world.apply_remote_delete(a_net).unwrap();
    world.apply_owner_change(b_net, SERVER).unwrap();
    assert_eq!(
        world.drain_events(),
        vec![
            WorldEvent::EntityCreated(a),
            WorldEvent::EntityCreated(b),
            WorldEvent::LocalityChanged {
                entity: b,
                local: true
            },
        ]
    );

    // The deletion's event lands when the deletion takes effect, at the flush.
    world.flush_deletions();
    assert_eq!(
        world.drain_events(),
        vec![WorldEvent::EntityDeleted {
            entity: a,
            network_id: Some(a_net),
        }]
    );
}

proptest! {
    /// The error of two equal states is zero, and any difference in a replicated field makes it
    /// non-zero, so no change is silently un-owed.
    #[test]
    fn update_error_is_zero_exactly_for_equal_states(a in state(), b in state()) {
        let error = UpdateError::between(&a, &b);
        prop_assert_eq!(error.is_zero(), a == b);
        prop_assert_eq!(error.is_zero(), error.total() == 0.0);
    }

    /// A client that applies a full update holds exactly what the server holds, field by field.
    #[test]
    fn a_client_that_applies_the_servers_state_holds_it(state in state()) {
        let mut types = bank();
        let mut server = World::new(SERVER);
        let mut client = World::new(CLIENT);
        let net = NetworkId::new(9, 1);
        server.apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO)).unwrap();
        client.apply_remote_create(&mut types, create(net, "C_Offroad_01_F", DVec3::ZERO)).unwrap();
        let baseline = entity_state(&client, net).unwrap();

        server.apply_remote_update(RemoteUpdate::new(net)
            .position(state.position)
            .orientation(state.orientation)
            .velocity(state.velocity)
            .damage(state.damage)
            .hidden(state.hidden)).unwrap();
        // What the receiver was last sent is far enough behind to be owed.
        prop_assert!(!UpdateError::between(&baseline, &entity_state(&server, net).unwrap()).is_zero());

        client.apply_remote_update(RemoteUpdate::new(net)
            .position(state.position)
            .orientation(state.orientation)
            .velocity(state.velocity)
            .damage(state.damage)
            .hidden(state.hidden)).unwrap();

        prop_assert_eq!(entity_state(&client, net), entity_state(&server, net));
        prop_assert_eq!(entity_state(&client, net), Some(state));
    }
}

/// Replicated states with finite values (the float arithmetic in the metrics is exact for them).
fn state() -> impl Strategy<Value = ReplicatedState> {
    (
        (-1000.0f64..1000.0, -1000.0f64..1000.0, -1000.0f64..1000.0),
        (-3.0f64..3.0, -3.0f64..3.0, -3.0f64..3.0),
        (-10.0f64..10.0, -10.0f64..10.0, -10.0f64..10.0),
        0.0f32..1.0,
        any::<bool>(),
    )
        .prop_map(|(p, r, v, damage, hidden)| ReplicatedState {
            position: DVec3::new(p.0, p.1, p.2),
            orientation: DQuat::from_euler(glam::EulerRot::YXZ, r.0, r.1, r.2),
            velocity: DVec3::new(v.0, v.1, v.2),
            damage,
            hidden,
        })
}
