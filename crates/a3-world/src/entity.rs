//! An Entity and its simulation list.

use std::sync::Arc;

use glam::DVec3;

use crate::{ClientId, EntityClass, EntityId, EntityType, NetworkId, SimulationClass};

/// Whether this machine owns an Entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locality {
    /// This machine simulates the Entity authoritatively and sends its updates.
    Local,
    /// Another machine owns it. The server always knows the owner; a client usually does not
    /// (the original's `owner` command returns 0 on clients).
    Remote { owner: Option<ClientId> },
}

impl Locality {
    pub fn is_local(self) -> bool {
        matches!(self, Locality::Local)
    }
}

/// The World list an Entity is simulated in, after the original's World containers
/// (`docs/re/world-object-model.md`, "World containers").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// AI-capable Entities: people, vehicles, buildings and things (`EntityAI`). The original's
    /// "vehicles".
    Vehicles = 0,
    /// Shots and other projectiles. The original's "fast vehicles", simulated in parallel jobs.
    Projectiles = 1,
    /// Every other Entity or Object created at run time (triggers, cameras, lamps, proxies).
    Slow = 2,
    /// Static objects promoted to Entities.
    Static = 3,
}

impl ListKind {
    pub(crate) const COUNT: usize = 4;
    pub const ALL: [ListKind; 4] = [
        ListKind::Vehicles,
        ListKind::Projectiles,
        ListKind::Slow,
        ListKind::Static,
    ];

    /// The list a created Entity of `class` goes to.
    pub fn for_class(class: SimulationClass) -> ListKind {
        if class.is_kind_of(EntityClass::Shot) {
            ListKind::Projectiles
        } else if class.is_kind_of(EntityClass::EntityAi) {
            ListKind::Vehicles
        } else {
            ListKind::Slow
        }
    }
}

/// A simulated Object in the World.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub(crate) id: EntityId,
    pub(crate) network_id: Option<NetworkId>,
    pub(crate) locality: Locality,
    pub(crate) entity_type: Arc<EntityType>,
    pub(crate) position: DVec3,
    pub(crate) list: ListKind,
    pub(crate) deleted: bool,
}

impl Entity {
    pub(crate) fn new(
        id: EntityId,
        network_id: Option<NetworkId>,
        locality: Locality,
        entity_type: Arc<EntityType>,
        position: DVec3,
        list: ListKind,
    ) -> Self {
        Self {
            id,
            network_id,
            locality,
            entity_type,
            position,
            list,
            deleted: false,
        }
    }

    pub fn id(&self) -> EntityId {
        self.id
    }

    /// `None` for Entities created local-only (`createVehicleLocal`).
    pub fn network_id(&self) -> Option<NetworkId> {
        self.network_id
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn is_local(&self) -> bool {
        self.locality.is_local()
    }

    pub fn entity_type(&self) -> &Arc<EntityType> {
        &self.entity_type
    }

    /// The config class name (`typeOf`).
    pub fn type_name(&self) -> &str {
        self.entity_type.name()
    }

    pub fn class(&self) -> SimulationClass {
        self.entity_type.class()
    }

    /// The simulation list it lives in.
    pub fn list(&self) -> ListKind {
        self.list
    }

    /// Scheduled for deletion; it disappears at the end of the step.
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub fn position(&self) -> DVec3 {
        self.position
    }

    pub fn set_position(&mut self, position: DVec3) {
        self.position = position;
    }
}
