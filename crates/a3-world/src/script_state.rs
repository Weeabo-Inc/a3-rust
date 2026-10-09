//! Object and group state that scripts set and read and the simulation does not drive (yet):
//! variables (`setVariable`), the player, identities (`setName`, `setFace`, ...), vehicle variable
//! names, synchronization links and the dynamic simulation flags. See
//! `docs/re/sqf-object-commands.md`.
//!
//! The original keeps these on the objects themselves (a variable space per `Object` and
//! `AIGroup`, the identity on `Person`, the sync list on the AI unit). Here they live in one side
//! table on the [`World`], keyed by Entity ID, Static key or Group ID, and are dropped with their
//! owner.

use std::collections::{HashMap, HashSet};

use a3_sqf::vm::Variables;

use crate::{EntityId, GroupId, ObjectRef, World};

/// Whose variable space a variable lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarOwner {
    Object(ObjectRef),
    Group(GroupId),
}

/// A Person's identity, as `setIdentity` / `setName` / `setFace` / ... set it.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    /// Full name (`name`).
    pub name: String,
    pub first_name: String,
    pub last_name: String,
    pub face: String,
    pub glasses: String,
    pub speaker: String,
    /// Voice pitch; the original's default is 1.
    pub pitch: f32,
    pub name_sound: String,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            face: String::new(),
            glasses: String::new(),
            speaker: String::new(),
            pitch: 1.0,
            name_sound: String::new(),
        }
    }
}

/// The script-side state of a [`World`].
#[derive(Default, Clone)]
pub struct ScriptState {
    vars: HashMap<VarOwner, Variables>,
    identities: HashMap<EntityId, Identity>,
    var_names: HashMap<EntityId, String>,
    /// Synchronization links (`synchronizeObjectsAdd`), per Entity in insertion order.
    synced: HashMap<EntityId, Vec<EntityId>>,
    dynamic_simulation: HashSet<EntityId>,
    dynamic_simulation_groups: HashSet<GroupId>,
    player: Option<EntityId>,
    camera_on: Option<EntityId>,
    /// Units' gear, created from their config default on first use (`inventory.rs`).
    gear: HashMap<EntityId, crate::inventory::Gear>,
}

impl std::fmt::Debug for ScriptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptState")
            .field("variable_spaces", &self.vars.len())
            .field("identities", &self.identities.len())
            .field("var_names", &self.var_names.len())
            .field("player", &self.player)
            .finish_non_exhaustive()
    }
}

impl ScriptState {
    /// Forgets everything about a deleted Entity.
    pub(crate) fn forget_entity(&mut self, id: EntityId) {
        self.vars.remove(&VarOwner::Object(ObjectRef::Entity(id)));
        self.identities.remove(&id);
        self.var_names.remove(&id);
        self.dynamic_simulation.remove(&id);
        self.gear.remove(&id);
        if let Some(links) = self.synced.remove(&id) {
            for other in links {
                if let Some(list) = self.synced.get_mut(&other) {
                    list.retain(|&e| e != id);
                }
            }
        }
        if self.player == Some(id) {
            self.player = None;
        }
        if self.camera_on == Some(id) {
            self.camera_on = None;
        }
    }

    /// Forgets everything about a deleted group.
    pub(crate) fn forget_group(&mut self, id: GroupId) {
        self.vars.remove(&VarOwner::Group(id));
        self.dynamic_simulation_groups.remove(&id);
    }
}

impl World {
    /// The variable space of an Object or group, if anything was ever stored in it.
    pub fn variables(&self, owner: VarOwner) -> Option<&Variables> {
        self.script.vars.get(&owner)
    }

    /// The variable space of an Object or group, created on first use.
    pub fn variables_mut(&mut self, owner: VarOwner) -> &mut Variables {
        self.script.vars.entry(owner).or_default()
    }

    /// The Entity the player controls on this machine (`player`).
    pub fn player(&self) -> Option<EntityId> {
        self.script.player.filter(|&id| self.entity(id).is_some())
    }

    /// Sets the player's Entity (mission start, `selectPlayer`).
    pub fn set_player(&mut self, id: Option<EntityId>) {
        self.script.player = id;
    }

    /// The Entity the camera is on (`cameraOn`): set explicitly, else the player.
    pub fn camera_on(&self) -> Option<EntityId> {
        self.script
            .camera_on
            .filter(|&id| self.entity(id).is_some())
            .or_else(|| self.player())
    }

    /// Puts the camera on an Entity (`switchCamera`); `None` follows the player.
    pub fn set_camera_on(&mut self, id: Option<EntityId>) {
        self.script.camera_on = id;
    }

    /// A Person's identity, if one was set.
    pub fn identity(&self, id: EntityId) -> Option<&Identity> {
        self.script.identities.get(&id)
    }

    /// A Person's identity, created with the defaults on first use.
    pub fn identity_mut(&mut self, id: EntityId) -> &mut Identity {
        self.script.identities.entry(id).or_default()
    }

    /// The Entity's vehicle variable name (`vehicleVarName`), `""` when unset.
    pub fn var_name(&self, id: EntityId) -> &str {
        self.script.var_names.get(&id).map_or("", String::as_str)
    }

    /// Sets the Entity's vehicle variable name; `""` clears it.
    pub fn set_var_name(&mut self, id: EntityId, name: &str) {
        if name.is_empty() {
            self.script.var_names.remove(&id);
        } else {
            self.script.var_names.insert(id, name.to_owned());
        }
    }

    /// The Entities synchronized with `id`, in the order they were added.
    pub fn synchronized(&self, id: EntityId) -> &[EntityId] {
        self.script.synced.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Adds `other` to `id`'s synchronization list (one direction; a link is two calls).
    pub fn add_sync(&mut self, id: EntityId, other: EntityId) {
        let list = self.script.synced.entry(id).or_default();
        if !list.contains(&other) {
            list.push(other);
        }
    }

    /// Removes `other` from `id`'s synchronization list.
    pub fn remove_sync(&mut self, id: EntityId, other: EntityId) {
        if let Some(list) = self.script.synced.get_mut(&id) {
            list.retain(|&e| e != other);
        }
    }

    /// Whether the dynamic simulation system manages the Entity (`enableDynamicSimulation`).
    pub fn dynamic_simulation(&self, id: EntityId) -> bool {
        self.script.dynamic_simulation.contains(&id)
    }

    pub fn set_dynamic_simulation(&mut self, id: EntityId, enabled: bool) {
        if enabled {
            self.script.dynamic_simulation.insert(id);
        } else {
            self.script.dynamic_simulation.remove(&id);
        }
    }

    /// A unit's gear, if it was ever created.
    pub fn gear(&self, id: EntityId) -> Option<&crate::inventory::Gear> {
        self.script.gear.get(&id)
    }

    pub fn gear_mut(&mut self, id: EntityId) -> Option<&mut crate::inventory::Gear> {
        self.script.gear.get_mut(&id)
    }

    /// Removes and returns a unit's gear (to change it with other borrows held).
    pub fn take_gear(&mut self, id: EntityId) -> Option<crate::inventory::Gear> {
        self.script.gear.remove(&id)
    }

    /// Sets a unit's gear.
    pub fn set_gear(&mut self, id: EntityId, gear: crate::inventory::Gear) {
        self.script.gear.insert(id, gear);
    }

    /// Whether the dynamic simulation system manages the group.
    pub fn group_dynamic_simulation(&self, id: GroupId) -> bool {
        self.script.dynamic_simulation_groups.contains(&id)
    }

    pub fn set_group_dynamic_simulation(&mut self, id: GroupId, enabled: bool) {
        if enabled {
            self.script.dynamic_simulation_groups.insert(id);
        } else {
            self.script.dynamic_simulation_groups.remove(&id);
        }
    }
}
