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
//!
//! # A frame
//!
//! ```text
//! world.create(Create::new(bank.get("C_Offroad_01_F")?, pos).on_surface())?;  // createVehicle
//! world.simulate(dt);              // steps, deferred commands, deletions (see `sim` order)
//! for event in world.drain_events() { /* EntityCreated / EntityDeleted / LocalityChanged */ }
//! ```
//!
//! # Multiplayer
//!
//! The network layer (Phase 6) drives the World through [`net`]: it applies remote create, update,
//! delete and owner-change messages, collects the updates this machine owes each receiver
//! ([`World::updates_owed`], ordered by [`UpdateError`]) and builds the state a joining client
//! needs ([`World::jip_snapshot`]).
//!
//! # Contributor guide
//!
//! ## Add an entity type (a new `simulation` value)
//!
//! 1. Add the variant to [`SimulationClass`] and its lower-case `simulation` string to the
//!    `SIMULATIONS` table in `class.rs`. Map it to its node of the original class tree in
//!    [`SimulationClass::engine_class`]; add an [`EntityClass`] node only if the original has a
//!    class there (see `docs/re/rtti-classes.md`). Mark `is_physx` for `...x` / `*EPE` values.
//! 2. The family follows from the engine class (`sim::Family::of`): people → `man`,
//!    `TankOrCar` → `ground`, `PlaneOrHeli`/`Parachute` → `air`, `Shot` → `projectile`, the rest
//!    `generic`. The World list follows from [`ListKind::for_class`].
//! 3. Run the real-data test `every_creatable_config_class_has_a_known_simulation`
//!    (`A3_ROOT`): every creatable class must still build a type.
//!
//! Config parameters a family needs are read from [`EntityType::config`] with the
//! [`TypeBank::config`] tree; cache them in the family state, not in [`EntityType`], unless every
//! family needs them.
//!
//! ## Add a simulation step (fill in a family)
//!
//! Each family lives in `src/sim/<family>.rs` (`man.rs` #124, `ground.rs` #125, `air.rs` #126,
//! `projectile.rs` #127; others go to `generic.rs` or a new family). It has:
//!
//! - a state struct (`ManState`, `GroundState`, ...) held in [`ClassState`]; put the family's
//!   per-Entity state there;
//! - `fn simulate(entity: &mut Entity, ctx: &mut StepContext, dt: f64)`, called once per step
//!   with exactly the Entity's step length, on **every** machine.
//!
//! Rules for a step:
//!
//! - Change only `entity` (position, orientation, velocity, class state, simulation step). Read
//!   other Entities through `ctx.world()`; the stepping Entity is absent from it during its own
//!   step. Create or delete other Entities with `ctx.create` / `ctx.delete`; they are applied
//!   after the phase.
//! - Do authoritative work (forces, damage, AI decisions, firing) only when
//!   `entity.is_local()`. A remote Entity advances only from its last received state.
//! - Change the step length with `entity.set_simulation_step` when the original does (it adjusts
//!   it per class at run time).
//! - Test through `World::simulate` (see `tests/simulate.rs`): create the Entity, simulate
//!   frames, assert on the Entity.
//!
//! ## Add an SQF world command
//!
//! World commands live in [`script`] (ADR 0005), one file per area (`create.rs`, `state.rs`,
//! `transform.rs`, `query.rs`; add a file for a new area and call it from
//! [`script::register_world_commands`]). They are generic over [`script::WorldHost`], which gives
//! `world()`, `world_mut()` and `types()`; [`script::ScriptWorld`] is a ready host for tests and
//! tools. Follow the existing commands:
//!
//! - Decode Object arguments with [`script::object_arg`] (it resolves promoted Static objects and
//!   treats deleted ones as `objNull`); return Objects with [`script::object_value`], which keeps
//!   a promoted Static object's handle stable.
//! - Positions cross the script boundary with Y and Z swapped (ADR 0003); use the helpers in
//!   `script/mod.rs` (`script_position`, `position_value`, `position_or_object`).
//! - Respect the command's locality and note it as `AG/AL EG/EL` on the command: "local
//!   argument" commands do nothing for a remote Entity, server-only commands do nothing on
//!   clients; global effects will also queue a network message once #131 exists.
//! - Changing a Static object promotes it (`promote_static_with_model_type`).
//! - Creation goes through [`World::create`]; deletion through [`World::delete`] (deferred);
//!   ownership changes through [`World::set_locality`].
//! - Test with SQF snippets against a synthetic World (`tests/script.rs`).

mod ai;
mod class;
mod entity;
mod groups;
mod id;
mod moves;
pub mod net;
mod object_ref;
mod query;
pub mod script;
mod sim;
mod statics;
mod terrain;
mod types;
mod world;

pub use ai::{
    ARRIVE_RADIUS, Behaviour, CombatMode, DEFAULT_COMPLETION_RADIUS, EYE_HEIGHT, FORGET_TIME,
    FORMATION_SPACING, Formation, GroupAi, KNOWLEDGE_PER_SECOND, LoiterType, ManAi, SpeedMode,
    TargetKnowledge, Targets, VIEW_RANGE, Waypoint, WaypointQueue, WaypointType, target_key,
};
pub use class::{EntityClass, SimulationClass};
pub use entity::{Attachment, Entity, ListKind, Locality, VisualState};
pub use groups::{ENEMY_THRESHOLD, Group, GroupId, Side, default_group_name, side_from_config};
pub use id::{ClientId, EntityId, NetworkId, ParseNetworkIdError};
pub use net::{
    DAMAGE_ERROR_SCALE, JipObject, OwedUpdate, RemoteCreate, RemoteUpdate, ReplicatedState,
    STATE_FLAG_ERROR, TRANSFORM_ERROR_PER_METRE, TRANSFORM_ERROR_PER_MPS,
    TRANSFORM_ERROR_PER_RADIAN, UpdateClass, UpdateError, UpdateOutcome,
};
pub use object_ref::ObjectRef;
pub use query::Near;
pub use sim::{
    AirState, ClassState, GRAVITY, GroundState, MAX_STEP_DOWN, MAX_STEP_UP, ManInput, ManState,
    Motion, MoveState, ProjectileState, SUB_STEP,
};
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
    /// The group was deleted (or never existed).
    #[error("no group {0:?}")]
    NoSuchGroup(GroupId),
    /// Only empty groups can be deleted.
    #[error("group {0:?} still has units")]
    GroupNotEmpty(GroupId),
    /// The unit is not in the group.
    #[error("{0:?} is not in the group")]
    NotInGroup(EntityId),
    /// The group has no waypoint at this index.
    #[error("group {group:?} has no waypoint {index}")]
    NoSuchWaypoint { group: GroupId, index: usize },
    /// No Object — Entity or Static object — has this Network object ID.
    #[error("no object with network id {0}")]
    NoSuchObject(NetworkId),
    /// This machine created the object the message names; the network layer must not echo our own
    /// create back ([`World::apply_remote_create`]).
    #[error("network id {0} was created by this machine")]
    OwnNetworkId(NetworkId),
    /// An owner change arrived from a machine that does not own the object (message 355 is C→S and
    /// the server checks the sender: "Server: OwnerChanged of %d:%d arrived from non owner %d").
    #[error("client {sender:?} does not own {network_id}")]
    NotOwner {
        network_id: NetworkId,
        sender: ClientId,
    },
    /// No Static object has this key.
    #[error("no static object {0:?}")]
    NoSuchStatic(StaticKey),
    /// A land cell holds more objects than the static key packing allows, or the land grid is
    /// larger than 1024 cells per side.
    #[error("static object key overflow in land cell ({cell_x}, {cell_z})")]
    StaticKeyOverflow { cell_x: u32, cell_z: u32 },
}
