//! An Entity and what it is created from.

use std::sync::Arc;

use glam::DVec3;

use crate::{ClientId, EntityId, EntityType, NetworkId, SimulationClass};

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

/// What to create: a type and where.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpec {
    pub entity_type: Arc<EntityType>,
    /// World-space position (ADR 0003).
    pub position: DVec3,
}

impl EntitySpec {
    pub fn new(entity_type: Arc<EntityType>, position: DVec3) -> Self {
        Self {
            entity_type,
            position,
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
}

impl Entity {
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

    pub fn position(&self) -> DVec3 {
        self.position
    }

    pub fn set_position(&mut self, position: DVec3) {
        self.position = position;
    }
}
