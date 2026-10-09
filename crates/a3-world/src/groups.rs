//! Groups, sides and side relations (the original's AIGroup and AICenter, both NetworkObjects).
//!
//! A group has a side, units (Entities, usually Men) in order, a leader, a name (`groupId`), a
//! Network object ID and a Locality. Units are local where their group is local; moving a group
//! (`setGroupOwner`) moves its units.

use std::collections::HashMap;

pub use a3_sqf::Side;

use crate::{ClientId, EntityId, Error, Locality, NetworkId, World};

/// Our handle to a group, with the same generational rule as [`EntityId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl GroupId {
    /// The `id` of an `a3_sqf::Handle` of kind `Group`; never 0 (0 is `grpNull`).
    pub fn to_handle_id(self) -> u64 {
        u64::from(self.generation) << 32 | (u64::from(self.index) + 1)
    }

    pub fn from_handle_id(handle: u64) -> Option<GroupId> {
        let index = (handle as u32).checked_sub(1)?;
        Some(GroupId {
            index,
            generation: (handle >> 32) as u32,
        })
    }
}

/// A group of units.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub(crate) id: GroupId,
    pub(crate) network_id: NetworkId,
    pub(crate) locality: Locality,
    pub(crate) side: Side,
    pub(crate) name: String,
    pub(crate) units: Vec<EntityId>,
    pub(crate) leader: Option<EntityId>,
    pub(crate) delete_when_empty: bool,
    /// `addWaypoint`'s list; index 0 is the first waypoint.
    pub(crate) waypoints: Vec<crate::Waypoint>,
    /// `currentWaypoint`: index of the waypoint the group moves to; `waypoints.len()` means the
    /// group has run through its list.
    pub(crate) current_waypoint: usize,
}

impl Group {
    pub fn id(&self) -> GroupId {
        self.id
    }

    pub fn network_id(&self) -> NetworkId {
        self.network_id
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn is_local(&self) -> bool {
        self.locality.is_local()
    }

    pub fn side(&self) -> Side {
        self.side
    }

    /// `groupId`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Units in join order.
    pub fn units(&self) -> &[EntityId] {
        &self.units
    }

    pub fn leader(&self) -> Option<EntityId> {
        self.leader
    }

    /// The group is deleted automatically when its last unit leaves.
    pub fn delete_when_empty(&self) -> bool {
        self.delete_when_empty
    }
}

/// The side a config `side` number stands for.
pub fn side_from_config(n: i32) -> Side {
    match n {
        0 => Side::East,
        1 => Side::West,
        2 => Side::Independent,
        3 => Side::Civilian,
        5 => Side::Enemy,
        6 => Side::Friendly,
        7 => Side::Logic,
        8 => Side::Empty,
        9 => Side::AmbientLife,
        _ => Side::Unknown,
    }
}

/// Callsign letters for default group names.
const CALLSIGNS: [&str; 26] = [
    "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet",
    "Kilo", "Lima", "Mike", "November", "Oscar", "Papa", "Quebec", "Romeo", "Sierra", "Tango",
    "Uniform", "Victor", "Whiskey", "X-ray", "Yankee", "Zulu",
];

/// The default `groupId` of the `n`-th group of a side: "Alpha 1-1" … "Alpha 1-6",
/// "Alpha 2-1" … "Alpha 4-6", then "Bravo 1-1" _(uncertain: the original builds names from
/// CfgWorlds group name lists; to confirm)_.
pub fn default_group_name(n: u32) -> String {
    let squad = n % 6 + 1;
    let platoon = n / 6 % 4 + 1;
    let letter = CALLSIGNS[(n / 24) as usize % CALLSIGNS.len()];
    format!("{letter} {platoon}-{squad}")
}

#[derive(Debug, Default)]
struct GroupSlot {
    generation: u32,
    group: Option<Group>,
}

/// The groups of a World, side relations and per-side name counters.
#[derive(Debug, Default)]
pub(crate) struct Groups {
    slots: Vec<GroupSlot>,
    free: Vec<u32>,
    by_network_id: HashMap<NetworkId, GroupId>,
    names_used: HashMap<Side, u32>,
    friendship: HashMap<(Side, Side), f32>,
}

impl Groups {
    fn get(&self, id: GroupId) -> Option<&Group> {
        self.slots
            .get(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.group.as_ref())
    }

    fn get_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.group.as_mut())
    }
}

/// The original's default relations before a mission sets its own: West and East are enemies,
/// Independent is friendly to West and hostile to East, Civilians are friendly to everyone.
fn default_friendship(a: Side, b: Side) -> f32 {
    use Side::*;
    if a == b {
        return 1.0;
    }
    match (a, b) {
        (West, East) | (East, West) => 0.0,
        (Independent, West) | (West, Independent) => 1.0,
        (Independent, East) | (East, Independent) => 0.0,
        (Enemy, _) | (_, Enemy) => 0.0,
        _ => 1.0,
    }
}

impl World {
    /// Creates a local group (`createGroup`) with a new Network object ID from this machine's
    /// serial and the side's next default name.
    pub fn create_group(&mut self, side: Side, delete_when_empty: bool) -> GroupId {
        let network_id = self.allocate_network_id();
        let n = {
            let used = self.groups_mut().names_used.entry(side).or_insert(0);
            *used += 1;
            *used - 1
        };
        self.insert_group(
            side,
            default_group_name(n),
            network_id,
            Locality::Local,
            delete_when_empty,
        )
    }

    /// Creates the copy of a group another machine created.
    pub fn spawn_remote_group(
        &mut self,
        side: Side,
        name: String,
        network_id: NetworkId,
        owner: Option<ClientId>,
    ) -> Result<GroupId, Error> {
        if network_id.is_null() || network_id.is_static() {
            return Err(Error::ReservedNetworkId(network_id));
        }
        if self.groups().by_network_id.contains_key(&network_id)
            || self.resolve(network_id).is_some()
        {
            return Err(Error::DuplicateNetworkId(network_id));
        }
        Ok(self.insert_group(side, name, network_id, Locality::Remote { owner }, false))
    }

    fn insert_group(
        &mut self,
        side: Side,
        name: String,
        network_id: NetworkId,
        locality: Locality,
        delete_when_empty: bool,
    ) -> GroupId {
        let groups = self.groups_mut();
        let index = match groups.free.pop() {
            Some(i) => i,
            None => {
                groups.slots.push(GroupSlot::default());
                (groups.slots.len() - 1) as u32
            }
        };
        let slot = &mut groups.slots[index as usize];
        let id = GroupId {
            index,
            generation: slot.generation,
        };
        slot.group = Some(Group {
            id,
            network_id,
            locality,
            side,
            name,
            units: Vec::new(),
            leader: None,
            delete_when_empty,
            waypoints: Vec::new(),
            current_waypoint: 0,
        });
        groups.by_network_id.insert(network_id, id);
        id
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups().get(id)
    }

    /// The group for commands that change it (waypoints, names, locality).
    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups_mut().get_mut(id)
    }

    /// Every group, in arena order (`allGroups`).
    pub fn all_groups(&self) -> impl Iterator<Item = &Group> {
        self.groups().slots.iter().filter_map(|s| s.group.as_ref())
    }

    /// The group with this Network object ID (`groupFromNetId`).
    pub fn resolve_group(&self, network_id: NetworkId) -> Option<GroupId> {
        self.groups().by_network_id.get(&network_id).copied()
    }

    /// `setGroupId`.
    pub fn set_group_name(&mut self, id: GroupId, name: impl Into<String>) -> Result<(), Error> {
        let g = self
            .groups_mut()
            .get_mut(id)
            .ok_or(Error::NoSuchGroup(id))?;
        g.name = name.into();
        Ok(())
    }

    /// The group of a unit (`group`).
    pub fn group_of(&self, unit: EntityId) -> Option<GroupId> {
        self.entity(unit)?.group
    }

    /// Moves `unit` into `group` (`join`), leaving its old group first. The first unit of a
    /// group becomes its leader. The unit takes the group's locality.
    pub fn join(&mut self, unit: EntityId, group: GroupId) -> Result<(), Error> {
        if self.entity(unit).is_none() {
            return Err(Error::NoSuchEntity(unit));
        }
        let locality = self.group(group).ok_or(Error::NoSuchGroup(group))?.locality;
        if self.group_of(unit) == Some(group) {
            return Ok(());
        }
        self.leave_group(unit);
        let g = self.groups_mut().get_mut(group).expect("checked");
        g.units.push(unit);
        g.leader.get_or_insert(unit);
        self.entity_mut(unit).expect("checked").group = Some(group);
        self.set_locality(unit, locality)?;
        Ok(())
    }

    /// Removes `unit` from its group. A leader is replaced by the next unit; an empty group
    /// marked delete-when-empty is deleted.
    pub fn leave_group(&mut self, unit: EntityId) {
        let Some(group) = self.group_of(unit) else {
            return;
        };
        if let Some(e) = self.entity_mut(unit) {
            e.group = None;
        }
        let Some(g) = self.groups_mut().get_mut(group) else {
            return;
        };
        g.units.retain(|&u| u != unit);
        if g.leader == Some(unit) {
            g.leader = g.units.first().copied();
        }
        if g.units.is_empty() && g.delete_when_empty {
            let _ = self.delete_group(group);
        }
    }

    /// `selectLeader`: the unit must be in the group.
    pub fn set_leader(&mut self, group: GroupId, unit: EntityId) -> Result<(), Error> {
        let g = self
            .groups_mut()
            .get_mut(group)
            .ok_or(Error::NoSuchGroup(group))?;
        if !g.units.contains(&unit) {
            return Err(Error::NotInGroup(unit));
        }
        g.leader = Some(unit);
        Ok(())
    }

    /// `deleteGroup`: only empty groups can be deleted, as in the original.
    pub fn delete_group(&mut self, id: GroupId) -> Result<(), Error> {
        let groups = self.groups_mut();
        let slot = groups
            .slots
            .get_mut(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .ok_or(Error::NoSuchGroup(id))?;
        match &slot.group {
            None => return Err(Error::NoSuchGroup(id)),
            Some(g) if !g.units.is_empty() => return Err(Error::GroupNotEmpty(id)),
            Some(_) => {}
        }
        let g = slot.group.take().expect("checked");
        slot.generation = slot.generation.wrapping_add(1);
        groups.free.push(id.index);
        groups.by_network_id.remove(&g.network_id);
        self.handlers_mut().forget_group(id);
        Ok(())
    }

    /// Sets a group's locality and that of its units (`setGroupOwner` on the server; a received
    /// ownership change elsewhere). Units record their own `WorldEvent::LocalityChanged`.
    pub fn set_group_locality(&mut self, id: GroupId, locality: Locality) -> Result<(), Error> {
        let g = self
            .groups_mut()
            .get_mut(id)
            .ok_or(Error::NoSuchGroup(id))?;
        g.locality = locality;
        let units = g.units.clone();
        for unit in units {
            self.set_locality(unit, locality)?;
        }
        Ok(())
    }

    /// The side of a unit or object: its group's side, otherwise its type's config side.
    pub fn side_of(&self, entity: EntityId) -> Option<Side> {
        let e = self.entity(entity)?;
        Some(match e.group.and_then(|g| self.group(g)) {
            Some(g) => g.side,
            None => side_from_config(e.entity_type().side()),
        })
    }

    /// How much side `a` likes side `b`: below 0.6 they are enemies (`getFriend`).
    pub fn friendship(&self, a: Side, b: Side) -> f32 {
        self.groups()
            .friendship
            .get(&(a, b))
            .copied()
            .unwrap_or_else(|| default_friendship(a, b))
    }

    /// `a setFriend [b, value]` (one direction, as in the original).
    pub fn set_friendship(&mut self, a: Side, b: Side, value: f32) {
        self.groups_mut().friendship.insert((a, b), value);
    }

    /// Whether `a` treats `b` as an enemy (friendship below 0.6).
    pub fn is_enemy(&self, a: Side, b: Side) -> bool {
        self.friendship(a, b) < ENEMY_THRESHOLD
    }
}

/// Friendship below this means enemies (the original's threshold).
pub const ENEMY_THRESHOLD: f32 = 0.6;
