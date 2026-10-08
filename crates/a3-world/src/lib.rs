//! The World: every Entity of the session with its Network object ID and Locality, and the
//! Static objects of the terrain.
//!
//! Design: ADR 0006 (`docs/adr/0006-world-entity-model.md`). The original engine's model this
//! mirrors: `docs/re/world-object-model.md`.
//!
//! - Entities live in a generational arena; an [`EntityId`] never refers to a different Entity
//!   after deletion.
//! - Every networked Entity has a [`NetworkId`] from creation: `{this client, serial}` when created
//!   here, the announced pair when created by another machine. Creator 0 is null, creator 1 is a
//!   Static object.
//! - [`Locality`] says whether this machine owns the Entity; [`World::set_locality`] records the
//!   changes for the `Local` event ([`WorldEvent`]).
//! - Static objects stay in a compact per-cell table loaded from the WRP and become Entities only
//!   when promoted.
//! - [`ObjectRef`] is what SQF `Object` values refer to; it encodes into an `a3-sqf` handle id.

mod class;
mod entity;
mod id;
mod object_ref;
mod statics;
mod terrain;
mod types;
mod world;

pub use class::{EntityClass, SimulationClass};
pub use entity::{Entity, ListKind, Locality};
pub use id::{ClientId, EntityId, NetworkId, ParseNetworkIdError};
pub use object_ref::ObjectRef;
pub use statics::{StaticKey, StaticObject};
pub use types::{DEFAULT_SIMULATION_STEP, EntityType, Scope, TypeBank, TypeSource};
pub use world::{Create, POSITION_MAX, POSITION_MIN, Placement, World, WorldEvent};

/// Errors from World operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Another Entity already has this Network object ID.
    #[error("network id {0} is already in use")]
    DuplicateNetworkId(NetworkId),
    /// Creators 0 (null) and 1 (Static objects) cannot be used for created Entities.
    #[error("network id {0} uses a reserved creator")]
    ReservedNetworkId(NetworkId),
    /// The Entity was deleted (or never existed).
    #[error("no entity {0:?}")]
    NoSuchEntity(EntityId),
    /// The type has `scope = 0`; the original refuses: "Cannot create entity with abstract type".
    #[error("cannot create entity with abstract type {0:?} (scope = private?)")]
    AbstractType(String),
    /// No class of this name in CfgVehicles, CfgAmmo or CfgNonAIVehicles.
    #[error("no config class {0:?} in CfgVehicles, CfgAmmo or CfgNonAIVehicles")]
    UnknownType(String),
    /// The class's `simulation` value has no engine class (yet).
    #[error("config class {type_name:?} has unknown simulation {simulation:?}")]
    UnknownSimulation {
        type_name: String,
        simulation: String,
    },
    /// No Static object has this key.
    #[error("no static object {0:?}")]
    NoSuchStatic(StaticKey),
    /// A land cell holds more objects than the static key packing allows, or the land grid is
    /// larger than 1024 cells per side.
    #[error("static object key overflow in land cell ({cell_x}, {cell_z})")]
    StaticKeyOverflow { cell_x: u32, cell_z: u32 },
}
