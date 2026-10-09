//! The World container: the Entity arena, the Network object ID index, the simulation lists,
//! creation, deferred deletion and the events scripts observe.

use std::collections::HashMap;
use std::sync::Arc;

use a3_moves::Moves;
use a3_physics::CollisionWorld;
use a3_wrp::Terrain;
use glam::DVec3;

use crate::groups::Groups;
use crate::statics::StaticObjects;
use crate::{
    ClientId, Entity, EntityId, EntityType, Error, GroupId, ListKind, Locality, NetworkId,
    ObjectRef, Scope, StaticKey,
};

/// The original clamps created positions to this box on every axis (`World_CreateVehicleImpl`).
pub const POSITION_MIN: f64 = -50_000.0;
pub const POSITION_MAX: f64 = 500_000.0;

/// How [`World::create`] places the new Entity vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// At `position` exactly; `y` is above sea level (`CAN_COLLIDE`).
    #[default]
    Exact,
    /// `y` is height above the terrain surface at `x`/`z`.
    OnSurface,
}

/// A request to create an Entity on this machine (the `createVehicle` path).
#[derive(Debug, Clone, PartialEq)]
pub struct Create {
    pub entity_type: Arc<EntityType>,
    /// World space (ADR 0003); see [`Placement`] for `y`.
    pub position: DVec3,
    pub placement: Placement,
    /// `createVehicleLocal`: exists on this machine only, without a Network object ID.
    pub local_only: bool,
}

impl Create {
    /// A networked Entity at an exact position.
    pub fn new(entity_type: Arc<EntityType>, position: DVec3) -> Self {
        Self {
            entity_type,
            position,
            placement: Placement::Exact,
            local_only: false,
        }
    }

    pub fn on_surface(mut self) -> Self {
        self.placement = Placement::OnSurface;
        self
    }

    pub fn local_only(mut self) -> Self {
        self.local_only = true;
        self
    }
}

/// Something that happened in the World that scripts or the network layer react to. Drained
/// with [`World::drain_events`], oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldEvent {
    /// An Entity was created on this machine, locally or as the copy of a remote one
    /// (`EntityCreated` mission event).
    EntityCreated(EntityId),
    /// A deletion took effect (at [`World::flush_deletions`]).
    EntityDeleted {
        entity: EntityId,
        network_id: Option<NetworkId>,
    },
    /// This machine gained (`local: true`) or lost ownership (`Local` event handler).
    LocalityChanged { entity: EntityId, local: bool },
    /// A group finished the waypoint at `index` and moved on (#129); a CYCLE waypoint reports
    /// the waypoint it left, so a mission sees each completion once.
    WaypointCompleted { group: GroupId, index: usize },
}

#[derive(Debug, Default)]
struct Slot {
    generation: u32,
    entity: Option<Entity>,
}

/// The session state of the World: every Entity, indexed by Entity ID and by Network object ID,
/// sorted into simulation lists, plus the terrain and its Static objects.
#[derive(Debug)]
pub struct World {
    local_client: ClientId,
    next_serial: u32,
    slots: Vec<Slot>,
    free: Vec<u32>,
    by_network_id: HashMap<NetworkId, EntityId>,
    lists: [Vec<EntityId>; ListKind::COUNT],
    pending_deletions: Vec<EntityId>,
    terrain: Option<Arc<Terrain>>,
    /// The collision world of this World: the terrain, its Static objects and every Entity's
    /// body (ADR 0008). A Man walks on its surfaces, so without one no Man moves.
    collision_world: Option<CollisionWorld>,
    /// The moves type every Man of this World is animated by (`CfgMovesMaleSdr`); see
    /// [`World::load_moves`].
    pub(crate) moves: Option<Arc<Moves>>,
    statics: StaticObjects,
    promoted: HashMap<StaticKey, EntityId>,
    events: Vec<WorldEvent>,
    time: f64,
    groups: Groups,
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
            lists: Default::default(),
            pending_deletions: Vec::new(),
            terrain: None,
            collision_world: None,
            moves: None,
            statics: StaticObjects::default(),
            promoted: HashMap::new(),
            events: Vec::new(),
            time: 0.0,
            groups: Groups::default(),
        }
    }

    /// This machine's client id: the creator of every Network object ID it allocates.
    pub fn local_client(&self) -> ClientId {
        self.local_client
    }

    /// Creates a local Entity (`createVehicle` / `createVehicleLocal`).
    ///
    /// Refuses abstract types (`scope = 0`). Clamps the position to the original's box, applies
    /// the [`Placement`], gives a networked Entity a new Network object ID
    /// `{local client, serial}`, sorts it into its simulation list and records
    /// [`WorldEvent::EntityCreated`].
    pub fn create(&mut self, request: Create) -> Result<EntityId, Error> {
        let ty = request.entity_type;
        if ty.scope() == Scope::Private {
            return Err(Error::AbstractType(ty.name().to_owned()));
        }
        let mut position = request
            .position
            .clamp(DVec3::splat(POSITION_MIN), DVec3::splat(POSITION_MAX));
        if request.placement == Placement::OnSurface {
            position.y += self.surface_height(position.x, position.z);
        }
        let network_id = (!request.local_only).then(|| self.allocate_network_id());
        let list = ListKind::for_class(ty.class());
        Ok(self.insert(ty, position, network_id, Locality::Local, list))
    }

    /// The next Network object ID `{local client, serial}`. Objects and groups share the serial,
    /// as in the original (`NetworkClient_RegisterObject`).
    pub(crate) fn allocate_network_id(&mut self) -> NetworkId {
        let id = NetworkId::new(self.local_client.0, self.next_serial);
        self.next_serial += 1;
        id
    }

    pub(crate) fn groups(&self) -> &Groups {
        &self.groups
    }

    pub(crate) fn groups_mut(&mut self) -> &mut Groups {
        &mut self.groups
    }

    /// Creates the copy of an Entity that another machine created and announced.
    pub fn spawn_remote(
        &mut self,
        entity_type: Arc<EntityType>,
        position: DVec3,
        network_id: NetworkId,
        owner: Option<ClientId>,
    ) -> Result<EntityId, Error> {
        if network_id.is_null() || network_id.is_static() {
            return Err(Error::ReservedNetworkId(network_id));
        }
        if self.by_network_id.contains_key(&network_id) {
            return Err(Error::DuplicateNetworkId(network_id));
        }
        let list = ListKind::for_class(entity_type.class());
        Ok(self.insert(
            entity_type,
            position,
            Some(network_id),
            Locality::Remote { owner },
            list,
        ))
    }

    pub(crate) fn insert(
        &mut self,
        entity_type: Arc<EntityType>,
        position: DVec3,
        network_id: Option<NetworkId>,
        locality: Locality,
        list: ListKind,
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
        slot.entity = Some(Entity::new(
            id,
            network_id,
            locality,
            entity_type,
            position,
            list,
        ));
        if let Some(net) = network_id {
            self.by_network_id.insert(net, id);
        }
        self.lists[list as usize].push(id);
        self.events.push(WorldEvent::EntityCreated(id));
        id
    }

    /// The Entity, or `None` once its deletion took effect.
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

    /// Every Entity, in arena order (Entities scheduled for deletion included).
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.slots.iter().filter_map(|s| s.entity.as_ref())
    }

    /// The Entities of one simulation list, in creation order.
    pub fn list(&self, kind: ListKind) -> &[EntityId] {
        &self.lists[kind as usize]
    }

    /// Schedules an Entity for deletion (`deleteVehicle`). Like the original, the Entity stays
    /// until the end of the step ([`flush_deletions`](Self::flush_deletions)); until then
    /// [`Entity::is_deleted`] is true. Returns `false` if it is gone or already scheduled.
    pub fn delete(&mut self, id: EntityId) -> bool {
        match self.entity_mut(id) {
            Some(e) if !e.deleted => {
                e.deleted = true;
                self.pending_deletions.push(id);
                true
            }
            _ => false,
        }
    }

    /// Applies the scheduled deletions: their Entity IDs and Network object IDs stop resolving
    /// and are never reused. Records [`WorldEvent::EntityDeleted`] for each.
    pub fn flush_deletions(&mut self) {
        for id in std::mem::take(&mut self.pending_deletions) {
            self.leave_group(id);
            let slot = &mut self.slots[id.index as usize];
            let Some(entity) = slot.entity.take() else {
                continue;
            };
            slot.generation = slot.generation.wrapping_add(1) & ENTITY_GENERATION_MASK;
            self.free.push(id.index);
            self.lists[entity.list as usize].retain(|&e| e != id);
            if let Some(net) = entity.network_id {
                self.by_network_id.remove(&net);
                if let Some(key) = StaticKey::from_network_id(net) {
                    // A deleted Static object stays gone; it does not fall back to the WRP record.
                    self.promoted.remove(&key);
                    self.statics.remove(key);
                }
            }
            self.events.push(WorldEvent::EntityDeleted {
                entity: id,
                network_id: entity.network_id,
            });
        }
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

    /// Sets an Entity's locality, recording [`WorldEvent::LocalityChanged`] when this machine
    /// gains or loses ownership. An owner change between two other machines is not a change here.
    pub fn set_locality(&mut self, id: EntityId, locality: Locality) -> Result<(), Error> {
        let entity = self.entity_mut(id).ok_or(Error::NoSuchEntity(id))?;
        let was_local = entity.locality.is_local();
        entity.locality = locality;
        if was_local != locality.is_local() {
            self.events.push(WorldEvent::LocalityChanged {
                entity: id,
                local: locality.is_local(),
            });
        }
        Ok(())
    }

    /// Takes the events recorded since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<WorldEvent> {
        std::mem::take(&mut self.events)
    }

    /// Records an event for the next [`World::drain_events`]. For the simulation modules, which
    /// cannot reach the private event list.
    pub(crate) fn push_event(&mut self, event: WorldEvent) {
        self.events.push(event);
    }

    /// Simulated time since the World was created, in seconds (`time`).
    pub fn time(&self) -> f64 {
        self.time
    }

    pub(crate) fn advance_time(&mut self, dt: f64) {
        self.time += dt;
    }

    pub(crate) fn slots_mut(&mut self) -> impl Iterator<Item = &mut Option<Entity>> {
        self.slots.iter_mut().map(|s| &mut s.entity)
    }

    /// Takes a live Entity out of its slot for its step; [`put_entity`](Self::put_entity)
    /// returns it. Meanwhile the slot looks empty but keeps its generation.
    pub(crate) fn take_entity(&mut self, id: EntityId) -> Option<Entity> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.entity.take()
    }

    pub(crate) fn put_entity(&mut self, entity: Entity) {
        let slot = &mut self.slots[entity.id.index as usize];
        debug_assert_eq!(slot.generation, entity.id.generation);
        slot.entity = Some(entity);
    }

    /// The terrain surface height at world `x`/`z` (0 without a terrain).
    pub fn surface_height(&self, x: f64, z: f64) -> f64 {
        self.terrain
            .as_ref()
            .map_or(0.0, |t| f64::from(t.surface_height(x as f32, z as f32)))
    }

    /// The loaded terrain.
    pub fn terrain(&self) -> Option<&Arc<Terrain>> {
        self.terrain.as_ref()
    }

    /// Installs the collision world every collision query of this World runs in, and in which
    /// every Entity's body lives (ADR 0008). The caller builds it — this crate owns no file
    /// system — and keeps it streamed: [`CollisionWorld::stream`] per interest.
    pub fn set_collision_world(&mut self, world: CollisionWorld) {
        self.collision_world = Some(world);
    }

    /// The collision world, once [`set_collision_world`](Self::set_collision_world) installed
    /// one.
    pub fn collision_world(&self) -> Option<&CollisionWorld> {
        self.collision_world.as_ref()
    }

    pub(crate) fn set_terrain(&mut self, terrain: Arc<Terrain>, statics: StaticObjects) {
        self.terrain = Some(terrain);
        self.statics = statics;
        self.promoted.clear();
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
        entity_type: Arc<EntityType>,
        position: DVec3,
        locality: Locality,
    ) -> EntityId {
        let id = self.insert(
            entity_type,
            position,
            Some(key.network_id()),
            locality,
            ListKind::Static,
        );
        self.promoted.insert(key, id);
        id
    }
}

/// Entity generations stay below 2^31 so that an encoded Entity handle never sets bit 63, which
/// marks Static objects.
pub(crate) const ENTITY_GENERATION_MASK: u32 = 0x7fff_ffff;
