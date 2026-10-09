//! Network-facing World operations (issue #131): the interface the network layer (Phase 6) calls.
//!
//! The network layer owns the bytes: it decodes a message, calls one of these operations, and
//! encodes what they return. Nothing here knows a wire layout — the message ids, item orders and
//! `/e<type>:<coef>` error annotations live in `docs/re/net-message-formats.tsv` and
//! `docs/re/net-object-model.md`.
//!
//! # The operations
//!
//! | message | operation |
//! | --- | --- |
//! | object create | [`World::apply_remote_create`] (the original's `type[%s]` names the config class) |
//! | object update | [`World::apply_remote_update`], ignored for an Entity this machine owns |
//! | object delete | [`World::apply_remote_delete`] |
//! | owner change (id 355) | [`World::receive_owner_change`] on the server, [`World::apply_owner_change`] elsewhere |
//! | JIP | [`World::jip_snapshot`]: everything a joining client must be told about |
//!
//! Sending is the other half: [`World::updates_owed`] lists what this machine's Local Entities
//! owe one receiver, ordered by the update error against what that receiver was last sent, and
//! [`World::mark_sent`] records what was actually sent. The per-receiver bookkeeping mirrors the
//! original's `NetworkObjectInfo`, which tracks one error per update class per receiving player
//! and "decides when and how often each object is sent to each player"
//! (`docs/re/world-object-model.md`).
//!
//! # Ordering guarantees
//!
//! - Operations apply immediately, in call order; only deletions are deferred (to
//!   [`World::flush_deletions`], as [`World::delete`] is). A batch of messages therefore records
//!   its [`WorldEvent`](crate::WorldEvent)s in the order it was applied.
//! - [`World::updates_owed`] is sorted by the total update error, highest first, ties broken by
//!   Network object ID ascending, so two machines holding the same state compute the same order.
//! - [`World::jip_snapshot`] is in arena order (ascending Entity slot index): stable between calls
//!   as long as nothing is created or deleted.
//!
//! # What the RE does not say (yet)
//!
//! Where a number would have to be invented, this module says so instead of guessing:
//!
//! - The object create and delete message ids and their item layouts are not identified
//!   ("Create / delete ... is open", `docs/re/net-object-model.md`; issues #105/#106/#107).
//!   [`RemoteCreate`] and [`World::apply_remote_delete`] take the decoded contents, whatever the
//!   ids turn out to be.
//! - The six per-update-class slots of the original's `NetworkObjectInfo` are not identified
//!   (issue #117). The three [`UpdateClass`]es here are our split of the state the World holds;
//!   only the damage and state-flag errors are the original's numbers.
//! - The transform error coefficients are ours (issue #117).
//! - Remote motion between updates — interpolation or dead reckoning, and the snap threshold — is
//!   open (#117). Here a received update replaces the transform and anchors the render
//!   interpolation at it; [`simulate`](World::simulate) does not extrapolate a remote Entity.
//! - The JIP world-state message and queue replay are open (#28); [`World::jip_snapshot`] is the
//!   state, not the message.
//! - Group replication is not part of this module.

use std::collections::HashMap;

use glam::{DQuat, DVec3};

use crate::{ClientId, Entity, EntityId, Error, Locality, NetworkId, ObjectRef, TypeBank, World};

// -- replicated state -------------------------------------------------------------------------

/// The state of one Entity that the network replicates, and what an update's error is measured
/// against.
///
/// The original compares an object's current state with the state the receiving player was last
/// sent, per update class (`NetworkObject` vtable slots 17/18 at `+0x88`/`+0x90`, the current and
/// the initial error metric; `docs/re/world-object-model.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplicatedState {
    pub position: DVec3,
    pub orientation: DQuat,
    pub velocity: DVec3,
    pub damage: f32,
    pub hidden: bool,
}

impl ReplicatedState {
    /// A state at `position`, everything else default.
    pub fn at(position: DVec3) -> Self {
        Self {
            position,
            orientation: DQuat::IDENTITY,
            velocity: DVec3::ZERO,
            damage: 0.0,
            hidden: false,
        }
    }
}

impl Entity {
    /// The Entity's replicated state: what a create or update message carries, and what the
    /// update error is computed from.
    pub fn replicated_state(&self) -> ReplicatedState {
        ReplicatedState {
            position: self.position,
            orientation: self.orientation,
            velocity: self.velocity,
            damage: self.damage(),
            hidden: self.hidden,
        }
    }
}

// -- update classes and their error metrics ----------------------------------------------------

/// A class of replicated state with its own error metric, after the original's per-class update
/// errors.
///
/// The original keeps six such slots per object per player (`NetworkObjectInfo`); which six is
/// not identified (issue #117). These three are the groups the World can compute an error for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateClass {
    /// Position, orientation and velocity.
    Transform,
    /// Damage.
    Damage,
    /// The destroyed/hidden state flags.
    State,
}

impl UpdateClass {
    pub const COUNT: usize = 3;
    pub const ALL: [UpdateClass; Self::COUNT] = [
        UpdateClass::Transform,
        UpdateClass::Damage,
        UpdateClass::State,
    ];
}

/// The error one changed state flag adds (a destroyed/hidden flag flipped adds 10000,
/// `docs/re/world-object-model.md`).
pub const STATE_FLAG_ERROR: f32 = 10_000.0;

/// The damage error per unit of damage changed: `10 * |Δdamage|` (measured on static objects in
/// `docs/re/world-object-model.md`).
pub const DAMAGE_ERROR_SCALE: f32 = 10.0;

/// Transform error per metre between the current state and the state last sent. *Our* scale: the
/// RE does not fix the transform coefficients (issue #117).
pub const TRANSFORM_ERROR_PER_METRE: f32 = 1.0;

/// Transform error per radian turned. *Our* scale (issue #117).
pub const TRANSFORM_ERROR_PER_RADIAN: f32 = 100.0;

/// Velocity error per m/s. *Our* scale (issue #117). The transform class carries it because a
/// velocity change is only useful to a receiver together with the transform it belongs to.
pub const TRANSFORM_ERROR_PER_MPS: f32 = 1.0;

/// The error of one Entity's state change, one value per [`UpdateClass`]: what the original's
/// per-class error metric carries (`docs/re/world-object-model.md`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UpdateError {
    errors: [f32; UpdateClass::COUNT],
}

impl UpdateError {
    /// The error of the state a receiver was last sent versus the state now.
    pub fn between(sent: &ReplicatedState, now: &ReplicatedState) -> Self {
        let mut errors = [0.0f32; UpdateClass::COUNT];
        errors[UpdateClass::Transform as usize] = (now.position - sent.position).length() as f32
            * TRANSFORM_ERROR_PER_METRE
            + now.orientation.angle_between(sent.orientation) as f32 * TRANSFORM_ERROR_PER_RADIAN
            + (now.velocity - sent.velocity).length() as f32 * TRANSFORM_ERROR_PER_MPS;
        errors[UpdateClass::Damage as usize] =
            DAMAGE_ERROR_SCALE * (now.damage - sent.damage).abs();
        errors[UpdateClass::State as usize] = if now.hidden != sent.hidden {
            STATE_FLAG_ERROR
        } else {
            0.0
        };
        Self { errors }
    }

    /// The error of one class.
    pub fn of(&self, class: UpdateClass) -> f32 {
        self.errors[class as usize]
    }

    /// The error over all classes: what [`World::updates_owed`] orders by.
    pub fn total(&self) -> f32 {
        self.errors.iter().sum()
    }

    /// No class reports an error: the receiver's state is current.
    pub fn is_zero(&self) -> bool {
        self.errors.iter().all(|e| *e == 0.0)
    }
}

// -- received messages -------------------------------------------------------------------------

/// The decoded contents of an object create message.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteCreate {
    /// The announced Network object ID. Its `creator` is the machine that created the object and
    /// is its first [`Owner`](crate::ClientId) (`docs/re/world-object-model.md`).
    pub network_id: NetworkId,
    /// The config class name the message carries (the original's log prints `type[%s]`).
    pub type_name: String,
    /// The state the message carries.
    pub state: ReplicatedState,
}

/// The decoded contents of one object update message.
///
/// Which items a message carries depends on its format (`docs/re/net-message-formats.tsv`, the
/// `/e<type>:<coef>` annotations): a field left `None` is not touched, a field set is applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteUpdate {
    pub network_id: NetworkId,
    pub position: Option<DVec3>,
    pub orientation: Option<DQuat>,
    pub velocity: Option<DVec3>,
    pub damage: Option<f32>,
    pub hidden: Option<bool>,
}

impl RemoteUpdate {
    /// An update that carries nothing yet; add the items the message had.
    pub fn new(network_id: NetworkId) -> Self {
        Self {
            network_id,
            position: None,
            orientation: None,
            velocity: None,
            damage: None,
            hidden: None,
        }
    }

    pub fn position(mut self, position: DVec3) -> Self {
        self.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: DQuat) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn velocity(mut self, velocity: DVec3) -> Self {
        self.velocity = Some(velocity);
        self
    }

    pub fn damage(mut self, damage: f32) -> Self {
        self.damage = Some(damage);
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = Some(hidden);
        self
    }
}

/// What [`World::apply_remote_update`] did with an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Applied to the Entity.
    Applied(EntityId),
    /// The Entity is Local here, so the update was dropped: the original logs
    /// `"Client: Object (id %d:%d, type %s) is local - update is ignored."`
    /// (`docs/re/net-message-dispatch.tsv`).
    IgnoredLocal(EntityId),
}

// -- what this machine owes --------------------------------------------------------------------

/// One update this machine owes one receiver: the Entity's current state and the error against the
/// state that receiver was last sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwedUpdate {
    pub entity: EntityId,
    pub network_id: NetworkId,
    /// The state to send.
    pub state: ReplicatedState,
    /// The error against the receiver's last sent state, per update class.
    pub error: UpdateError,
}

/// One object of the world state a joining client needs.
#[derive(Debug, Clone, PartialEq)]
pub struct JipObject {
    pub network_id: NetworkId,
    /// The config class name the client builds the Entity from.
    pub type_name: String,
    /// The current owner; `None` when this machine does not know it (a client usually does not).
    pub owner: Option<ClientId>,
    pub state: ReplicatedState,
}

// -- World operations --------------------------------------------------------------------------

impl World {
    /// Applies an object create received from the network: builds the remote copy of an Entity
    /// another machine created.
    ///
    /// The Entity's owner is the Network object ID's `creator` — the machine that created the
    /// object owns it (`docs/re/world-object-model.md`, "The initial owner"). The message's type
    /// name is resolved through `types`; an unknown or unknown-simulation class is refused. The
    /// class's `scope` is not re-checked: the sender already did, and the RE does not show the
    /// network path checking it (#106).
    ///
    /// Refused for a Network object ID with a reserved creator (0, or 1 for Static objects: every
    /// machine has those from the WRP and they are never created by a message,
    /// `docs/re/net-object-model.md`), for an ID already in use, and for an ID this machine
    /// created itself (the network layer must not echo our own create back).
    pub fn apply_remote_create(
        &mut self,
        types: &mut TypeBank,
        create: RemoteCreate,
    ) -> Result<EntityId, Error> {
        let network_id = create.network_id;
        if network_id.creator == self.local_client().0 {
            return Err(Error::OwnNetworkId(network_id));
        }
        let entity_type = types.get(&create.type_name)?;
        // `spawn_remote` forces the owner from the argument; the message's creator is the owner.
        let owner = Some(ClientId(network_id.creator));
        let id = self.spawn_remote(entity_type, create.state.position, network_id, owner)?;
        let entity = self.entity_mut(id).expect("the Entity was just inserted");
        apply_state(entity, &create.state);
        Ok(id)
    }

    /// Applies an update received from the network to the Entity its Network object ID names.
    ///
    /// Only the items the message carries are applied. An Entity this machine owns is left alone
    /// ([`UpdateOutcome::IgnoredLocal`]). An ID that names no Entity — including a Static object
    /// (creator 1), which is never updated by a message — is [`Error::NoSuchObject`].
    pub fn apply_remote_update(&mut self, update: RemoteUpdate) -> Result<UpdateOutcome, Error> {
        let id = self.entity_of(update.network_id)?;
        let entity = self.entity_mut(id).expect("just resolved");
        if entity.is_local() {
            return Ok(UpdateOutcome::IgnoredLocal(id));
        }
        if let Some(position) = update.position {
            entity.set_position(position);
        }
        if let Some(orientation) = update.orientation {
            entity.set_orientation(orientation);
        }
        if let Some(velocity) = update.velocity {
            entity.set_velocity(velocity);
        }
        if let Some(damage) = update.damage {
            entity.set_damage(damage);
        }
        if let Some(hidden) = update.hidden {
            entity.set_hidden(hidden);
        }
        // A received transform is where the Entity is; do not blend the renderer from the state
        // before it (interpolation between received states is open, #117).
        if update.position.is_some() || update.orientation.is_some() {
            entity.anchor_visual_state();
        }
        Ok(UpdateOutcome::Applied(id))
    }

    /// Applies an object delete received from the network: schedules the Entity for deletion, as
    /// [`World::delete`] does, so it disappears when the deletions are flushed. Deleting an
    /// already scheduled Entity again is not an error; an ID that names no Entity is
    /// [`Error::NoSuchObject`].
    pub fn apply_remote_delete(&mut self, network_id: NetworkId) -> Result<EntityId, Error> {
        let id = self.entity_of(network_id)?;
        self.delete(id);
        Ok(id)
    }

    /// Applies an owner change: the Entity belongs to `new_owner` from now on. Locality follows
    /// (this machine owns it when it is the new owner), so the change records
    /// [`WorldEvent::LocalityChanged`](crate::WorldEvent::LocalityChanged) when ownership moves
    /// to or away from this machine.
    pub fn apply_owner_change(
        &mut self,
        network_id: NetworkId,
        new_owner: ClientId,
    ) -> Result<EntityId, Error> {
        let id = self.entity_of(network_id)?;
        let locality = if new_owner == self.local_client() {
            Locality::Local
        } else {
            Locality::Remote {
                owner: Some(new_owner),
            }
        };
        self.set_locality(id, locality)?;
        Ok(id)
    }

    /// The server's receive path for an owner change (message 355, C→S): applies it only when
    /// `sender` owns the object, and otherwise reports [`Error::NotOwner`] — the original logs
    /// "Server: OwnerChanged of %d:%d arrived from non owner %d" and does nothing
    /// (`docs/re/world-object-model.md`).
    pub fn receive_owner_change(
        &mut self,
        sender: ClientId,
        network_id: NetworkId,
        new_owner: ClientId,
    ) -> Result<EntityId, Error> {
        if self.owner(network_id) != Some(sender) {
            // The object may not exist at all; the ownership check covers both refusals.
            self.entity_of(network_id)?;
            return Err(Error::NotOwner { network_id, sender });
        }
        self.apply_owner_change(network_id, new_owner)
    }

    /// The machine that owns the Object a Network object ID names, as far as this machine knows:
    /// the owner of a remote Entity, this machine for one it owns, and this machine for a Static
    /// object (the original's Static `Object`s are always local). `None` for an ID that names
    /// nothing.
    pub fn owner(&self, network_id: NetworkId) -> Option<ClientId> {
        match self.resolve(network_id)? {
            ObjectRef::Entity(id) => Some(match self.entity(id)?.locality() {
                Locality::Local => self.local_client(),
                Locality::Remote { owner } => owner?,
            }),
            ObjectRef::Static(_) => Some(self.local_client()),
        }
    }

    /// The updates owed to `receiver` by this machine's Local Entities, highest total error
    /// first, ties broken by Network object ID ascending.
    ///
    /// An Entity is owed an update when its state differs from the state [`mark_sent`]
    /// (Self::mark_sent) last recorded for that receiver, so a receiver with no baseline for an
    /// object is owed nothing: it does not know the object yet and must be sent a create (see
    /// [`jip_snapshot`](Self::jip_snapshot)) first. Entities without a Network object ID
    /// (`createVehicleLocal`) and Entities scheduled for deletion are never owed.
    pub fn updates_owed(&self, receiver: ClientId) -> Vec<OwedUpdate> {
        let mut owed = Vec::new();
        for entity in self.entities() {
            let (Some(network_id), false) = (entity.network_id(), entity.is_deleted()) else {
                continue;
            };
            if !entity.is_local() {
                continue;
            }
            let Some(sent) = self.sent.get(&entity.id()).and_then(|by| by.get(&receiver)) else {
                continue;
            };
            let state = entity.replicated_state();
            let error = UpdateError::between(sent, &state);
            if error.is_zero() {
                continue;
            }
            owed.push(OwedUpdate {
                entity: entity.id(),
                network_id,
                state,
                error,
            });
        }
        owed.sort_by(|a, b| {
            b.error
                .total()
                .total_cmp(&a.error.total())
                .then_with(|| a.network_id.cmp(&b.network_id))
        });
        owed
    }

    /// Records the state `receiver` has now been sent for the object `network_id`: the baseline
    /// [`updates_owed`](Self::updates_owed) measures against. Called after a create or an update
    /// is sent. Unknown IDs are ignored.
    pub fn mark_sent(&mut self, receiver: ClientId, network_id: NetworkId, state: ReplicatedState) {
        let Ok(id) = self.entity_of(network_id) else {
            return;
        };
        self.sent.entry(id).or_default().insert(receiver, state);
    }

    /// The state a joining client needs: every networked Entity, in arena order, with the type
    /// name to build it from and its current owner.
    ///
    /// Static objects (creator 1) are left out — every machine has them from the WRP and they need
    /// no create message (`docs/re/net-object-model.md`) — as are Entities without a Network
    /// object ID and Entities scheduled for deletion. The client applies each as a create
    /// ([`apply_remote_create`](Self::apply_remote_create)); the sender should then
    /// [`mark_sent`](Self::mark_sent) each so later changes are owed.
    pub fn jip_snapshot(&self) -> Vec<JipObject> {
        self.entities()
            .filter(|e| !e.is_deleted())
            .filter_map(|e| {
                let network_id = e.network_id()?;
                Some(JipObject {
                    network_id,
                    type_name: e.type_name().to_owned(),
                    owner: self.owner(network_id),
                    state: e.replicated_state(),
                })
            })
            .collect()
    }

    /// The Entity a Network object ID names on this machine: `None` for an unknown ID, and for a
    /// Static object that is not (or no longer) a promoted Entity.
    fn entity_of(&self, network_id: NetworkId) -> Result<EntityId, Error> {
        match self.resolve(network_id) {
            Some(ObjectRef::Entity(id)) => Ok(id),
            Some(ObjectRef::Static(_)) | None => Err(Error::NoSuchObject(network_id)),
        }
    }
}

/// Applies every part of a created Entity's state.
fn apply_state(entity: &mut Entity, state: &ReplicatedState) {
    entity.set_position(state.position);
    entity.set_orientation(state.orientation);
    entity.set_velocity(state.velocity);
    entity.set_damage(state.damage);
    entity.set_hidden(state.hidden);
    entity.anchor_visual_state();
}

/// The per-receiver send state of this machine's Entities: the original's `NetworkObjectInfo`,
/// which "decides when and how often each object is sent to each player"
/// (`docs/re/world-object-model.md`).
pub(crate) type SentStates = HashMap<EntityId, HashMap<ClientId, ReplicatedState>>;
