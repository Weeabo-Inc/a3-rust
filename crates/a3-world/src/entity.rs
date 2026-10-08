//! An Entity and what it is created from.

use glam::DVec3;

use crate::{ClientId, EntityId, NetworkId, SimulationClass};

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

/// What to create: the config class and where.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpec {
    /// The CfgVehicles (or CfgAmmo, CfgNonAIVehicles) class name.
    pub type_name: String,
    /// The engine class, from the type's `simulation` value.
    pub class: SimulationClass,
    /// World-space position (ADR 0003).
    pub position: DVec3,
}

impl EntitySpec {
    pub fn new(type_name: impl Into<String>, class: SimulationClass, position: DVec3) -> Self {
        Self {
            type_name: type_name.into(),
            class,
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
    pub(crate) type_name: String,
    pub(crate) class: SimulationClass,
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

    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    pub fn class(&self) -> SimulationClass {
        self.class
    }

    pub fn position(&self) -> DVec3 {
        self.position
    }

    pub fn set_position(&mut self, position: DVec3) {
        self.position = position;
    }
}
