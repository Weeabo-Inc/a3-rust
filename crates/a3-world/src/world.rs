//! The World container: the Entity arena, the Network object ID index and Locality changes.

use std::collections::HashMap;

use crate::statics::StaticObjects;
use crate::{
    ClientId, Entity, EntityId, EntitySpec, Error, Locality, NetworkId, ObjectRef, StaticKey,
};

/// A change of an Entity's locality on this machine, for the `Local` event handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalityChange {
    pub entity: EntityId,
    /// The new state: `true` when this machine became the owner.
    pub local: bool,
}

#[derive(Debug, Default)]
struct Slot {
    generation: u32,
    entity: Option<Entity>,
}

/// The session state of the World: every Entity, indexed by Entity ID and by Network object ID,
/// plus the Static objects of the terrain.
#[derive(Debug)]
pub struct World {
    local_client: ClientId,
    next_serial: u32,
    slots: Vec<Slot>,
    free: Vec<u32>,
    by_network_id: HashMap<NetworkId, EntityId>,
    statics: StaticObjects,
    promoted: HashMap<StaticKey, EntityId>,
    locality_changes: Vec<LocalityChange>,
}

impl World {
    /// An empty World on the machine with client id `local_client` (`ClientId::SERVER` on a
    /// server and in single player).
    pub fn new(local_client: ClientId) -> Self {
        Self {
            local_client,
            next_serial: 1,
            slots: Vec::new(),
            free: Vec::new(),
            by_network_id: HashMap::new(),
            statics: StaticObjects::default(),
            promoted: HashMap::new(),
            locality_changes: Vec::new(),
        }
    }

    /// This machine's client id: the creator of every Network object ID it allocates.
    pub fn local_client(&self) -> ClientId {
        self.local_client
    }

    /// Creates a local Entity with a new Network object ID `{local client, serial}`.
    pub fn spawn(&mut self, spec: EntitySpec) -> EntityId {
        let network_id = NetworkId::new(self.local_client.0, self.next_serial);
        self.next_serial += 1;
        let id = self.insert(spec, Some(network_id), Locality::Local);
        self.by_network_id.insert(network_id, id);
        id
    }

    /// Creates a local Entity that exists on this machine only (`createVehicleLocal`): it has no
    /// Network object ID.
    pub fn spawn_local_only(&mut self, spec: EntitySpec) -> EntityId {
        self.insert(spec, None, Locality::Local)
    }

    /// Creates the copy of an Entity that another machine created and announced.
    pub fn spawn_remote(
        &mut self,
        spec: EntitySpec,
        network_id: NetworkId,
        owner: Option<ClientId>,
    ) -> Result<EntityId, Error> {
        if network_id.is_null() || network_id.is_static() {
            return Err(Error::ReservedNetworkId(network_id));
        }
        if self.by_network_id.contains_key(&network_id) {
            return Err(Error::DuplicateNetworkId(network_id));
        }
        let id = self.insert(spec, Some(network_id), Locality::Remote { owner });
        self.by_network_id.insert(network_id, id);
        Ok(id)
    }

    fn insert(
        &mut self,
        spec: EntitySpec,
        network_id: Option<NetworkId>,
        locality: Locality,
    ) -> EntityId {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot::default());
                (self.slots.len() - 1) as u32
            }
        };
        let slot = &mut self.slots[index as usize];
        let id = EntityId {
            index,
            generation: slot.generation,
        };
        slot.entity = Some(Entity {
            id,
            network_id,
            locality,
            type_name: spec.type_name,
            class: spec.class,
            position: spec.position,
        });
        id
    }

    /// The Entity, or `None` if `id` was deleted.
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.slots
            .get(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.entity.as_ref())
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.entity.as_mut())
    }

    /// Every live Entity, in arena order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.slots.iter().filter_map(|s| s.entity.as_ref())
    }

    /// Removes an Entity. Returns `false` if it was already gone. Its Entity ID and Network
    /// object ID stop resolving and are never reused.
    pub fn delete(&mut self, id: EntityId) -> bool {
        let Some(slot) = self.slots.get_mut(id.index as usize) else {
            return false;
        };
        if slot.generation != id.generation {
            return false;
        }
        let Some(entity) = slot.entity.take() else {
            return false;
        };
        slot.generation = slot.generation.wrapping_add(1) & ENTITY_GENERATION_MASK;
        self.free.push(id.index);
        if let Some(net) = entity.network_id {
            self.by_network_id.remove(&net);
            if let Some(key) = StaticKey::from_network_id(net) {
                // A deleted Static object stays gone; it does not fall back to the WRP record.
                self.promoted.remove(&key);
                self.statics.remove(key);
            }
        }
        true
    }

    /// The Object a Network object ID refers to: an Entity, or a Static object (creator 1).
    pub fn resolve(&self, network_id: NetworkId) -> Option<ObjectRef> {
        if network_id.is_static() {
            let key = StaticKey::from_network_id(network_id)?;
            if let Some(&id) = self.promoted.get(&key) {
                return Some(ObjectRef::Entity(id));
            }
            return self.statics.contains(key).then_some(ObjectRef::Static(key));
        }
        self.by_network_id
            .get(&network_id)
            .copied()
            .map(ObjectRef::Entity)
    }

    /// Sets an Entity's locality, recording a [`LocalityChange`] when this machine gains or
    /// loses ownership. An owner change between two other machines is not a change here.
    pub fn set_locality(&mut self, id: EntityId, locality: Locality) -> Result<(), Error> {
        let entity = self.entity_mut(id).ok_or(Error::NoSuchEntity(id))?;
        let was_local = entity.locality.is_local();
        entity.locality = locality;
        if was_local != locality.is_local() {
            self.locality_changes.push(LocalityChange {
                entity: id,
                local: locality.is_local(),
            });
        }
        Ok(())
    }

    /// Takes the locality changes recorded since the last call, oldest first.
    pub fn drain_locality_changes(&mut self) -> Vec<LocalityChange> {
        std::mem::take(&mut self.locality_changes)
    }

    pub(crate) fn statics_mut(&mut self) -> &mut StaticObjects {
        &mut self.statics
    }

    pub(crate) fn statics(&self) -> &StaticObjects {
        &self.statics
    }

    pub(crate) fn promoted(&self) -> &HashMap<StaticKey, EntityId> {
        &self.promoted
    }

    pub(crate) fn insert_promoted(
        &mut self,
        key: StaticKey,
        spec: EntitySpec,
        locality: Locality,
    ) -> EntityId {
        let network_id = key.network_id();
        let id = self.insert(spec, Some(network_id), locality);
        self.by_network_id.insert(network_id, id);
        self.promoted.insert(key, id);
        id
    }
}

/// Entity generations stay below 2^31 so that an encoded Entity handle never sets bit 63, which
/// marks Static objects.
pub(crate) const ENTITY_GENERATION_MASK: u32 = 0x7fff_ffff;
