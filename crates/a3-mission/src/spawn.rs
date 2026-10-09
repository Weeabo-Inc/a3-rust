//! Spawning a [`Mission`] into a [`World`]: Entities at the SQM positions, groups, leaders.
//!
//! What the engine does at mission start, in order: create every placed unit as an Entity at its
//! `position[]` (the second component is height above sea level, like `CAN_COLLIDE`), set its
//! heading from `azimut`, put it into its group, and make the unit with `leader=1` the group's
//! leader. Units with `presence=0` are not created; units whose class the config does not know
//! are reported in [`Spawned::unspawned`] instead of stopping the load.
//!
//! Not applied yet (no World support): `skill`, `rank`, `special` ("FLY"/"FORM"/"CARGO" vehicle
//! placement) and `lock`. Waypoints and triggers are values only.

use std::collections::BTreeMap;

use a3_world::{Create, EntityId, GroupId, Side, TypeBank, World};
use glam::DVec3;

use crate::mission::{Mission, MissionGroup, Unit};

/// What [`spawn_mission`] created.
#[derive(Debug, Clone, Default)]
pub struct Spawned {
    /// Every created Entity, by the SQM unit `id`.
    pub units: BTreeMap<i32, EntityId>,
    /// The created groups, in SQM order.
    pub groups: Vec<SpawnedGroup>,
    /// The player-controlled unit, when the mission has one.
    pub player: Option<EntityId>,
    /// Units that could not be created, in mission order, with the reason.
    pub unspawned: Vec<Unspawned>,
    /// The markers, as name and world position. Markers are not World Entities (yet); this
    /// carries them so a caller can draw or create them later.
    pub markers: Vec<(String, DVec3)>,
}

/// One group of [`Spawned`].
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnedGroup {
    /// The group's side.
    pub side: Side,
    /// The World's handle to the group.
    pub group: GroupId,
    /// The group's created units, in SQM order.
    pub units: Vec<EntityId>,
}

/// A unit that could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unspawned {
    /// The SQM `id`.
    pub id: i32,
    /// The `vehicle` config class.
    pub class: String,
    /// Why it was not created.
    pub reason: String,
}

/// Creates every unit of `mission` in `world` (types from `types`), grouped as the SQM places
/// them.
pub fn spawn_mission(world: &mut World, types: &mut TypeBank, mission: &Mission) -> Spawned {
    let mut out = Spawned {
        markers: mission
            .markers
            .iter()
            .map(|m| (m.name.clone(), m.position))
            .collect(),
        ..Spawned::default()
    };
    for group in &mission.groups {
        let spawned = spawn_group(world, types, group, &mut out);
        out.groups.push(spawned);
    }
    // Objects outside any group (`class Vehicles` at the top level).
    for unit in &mission.objects {
        if unit.is_absent() {
            continue;
        }
        if let Some(id) = create_unit(world, types, unit, &mut out) {
            out.units.insert(unit.id, id);
            if unit.player.is_some() {
                out.player = Some(id);
            }
        }
    }
    out
}

fn spawn_group(
    world: &mut World,
    types: &mut TypeBank,
    group: &MissionGroup,
    out: &mut Spawned,
) -> SpawnedGroup {
    // The group's side comes first (the group exists before its units); a unit whose own `side`
    // differs — a soldier of another side in a vehicle — keeps the group's.
    let group_id = world.create_group(group.side, false);
    let mut spawned = SpawnedGroup {
        side: group.side,
        group: group_id,
        units: Vec::new(),
    };
    let mut leader = None;
    for unit in group.units.iter().filter(|u| !u.is_absent()) {
        let Some(id) = create_unit(world, types, unit, out) else {
            continue;
        };
        if world.join(id, group_id).is_err() {
            continue;
        }
        if unit.leader {
            leader = Some(id);
        }
        spawned.units.push(id);
        out.units.insert(unit.id, id);
        if unit.player.is_some() {
            out.player = Some(id);
        }
    }
    if let Some(leader) = leader {
        let _ = world.set_leader(group_id, leader);
    }
    spawned
}

/// Creates one unit, recording failures in `out.unspawned`. Returns the Entity.
fn create_unit(
    world: &mut World,
    types: &mut TypeBank,
    unit: &Unit,
    out: &mut Spawned,
) -> Option<EntityId> {
    let entity_type = match types.get(&unit.class) {
        Ok(ty) => ty,
        Err(e) => {
            out.unspawned.push(Unspawned {
                id: unit.id,
                class: unit.class.clone(),
                reason: e.to_string(),
            });
            return None;
        }
    };
    let mut create = Create::new(entity_type, unit.position);
    if unit.on_surface {
        create = create.on_surface();
    }
    let id = match world.create(create) {
        Ok(id) => id,
        Err(e) => {
            out.unspawned.push(Unspawned {
                id: unit.id,
                class: unit.class.clone(),
                reason: e.to_string(),
            });
            return None;
        }
    };
    if let (Some(azimut), Some(e)) = (unit.azimut, world.entity_mut(id)) {
        // `azimut` and `getDir` are the same convention: degrees clockwise from north
        // (2D Editor: "0 faces north, 90 faces east"); negative values wrap, as `setDir` does.
        e.set_heading(azimut);
    }
    Some(id)
}
