//! The AI of groups and units: waypoints, formations, the units' formation FSM, paths and what a
//! group knows about its enemies, turned into each unit's [`ManInput`] once per frame.
//!
//! Phase 5 of [`World::simulate`]. Only groups whose [`Locality`](crate::Locality) is local think
//! here — a group owned by another machine is driven there, and its units on this machine only
//! replay the state that arrives over the network. Nothing in this module reads a clock:
//! the tick uses `dt` of World time and the World's own seeded random generator
//! ([`EngineRng`]), so a mission replays the same way at any frame rate.
//!
//! The engine's model and how sure we are of each part: `docs/re/ai.md` (groups, waypoints,
//! formations, movement), `docs/re/ai-fsm.md` (the unit FSMs).

mod formation;
mod movement;
mod rng;
mod target;
mod unit_fsm;
mod waypoint;

use std::collections::HashMap;
use std::sync::Arc;

pub use formation::{FormationEntry, FormationShape, FormationSlot, FormationTable};
pub use movement::{Pace, UnitPath};
pub use rng::{DEFAULT_SEED, EngineRng};
pub use target::{
    EYE_HEIGHT, FORGET_TIME, KNOWLEDGE_PER_SECOND, TargetKnowledge, Targets, VIEW_RANGE, target_key,
};
pub use unit_fsm::CoverState;
pub use waypoint::{
    Behaviour, CombatMode, Completion, DEFAULT_COMPLETION_RADIUS, FORMATION_SPACING, Formation,
    LoiterType, SpeedMode, Waypoint, WaypointQueue, WaypointType,
};

use a3_fsm::{Fsm, Machine};
use a3_physics::ObjectKey;
use glam::DVec3;

use crate::{EntityId, Error, GroupId, UnitPos, World};

/// How close a unit must be to the end of an order of his own (`doMove`) before it is done, in
/// metres, when his type gives no `precision`.
pub const ARRIVE_RADIUS: f64 = 1.0;

/// The AI state the World keeps for everyone: the formation table, the native FSMs units can
/// run, the navigator paths are planned on, and the random generator.
#[derive(Default)]
pub struct AiWorld {
    pub(crate) formations: FormationTable,
    pub(crate) native_fsms: HashMap<String, Arc<Fsm>>,
    pub(crate) navigator: Option<a3_nav::Navigator>,
    pub(crate) rng: EngineRng,
}

impl std::fmt::Debug for AiWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiWorld")
            .field("native_fsms", &self.native_fsms.keys().collect::<Vec<_>>())
            .field("navigator", &self.navigator.is_some())
            .field("rng", &self.rng)
            .finish()
    }
}

/// The AI state of a group: its waypoint queue, the modes it moves under, its formation, and
/// what it knows.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupAi {
    /// The queue, and the waypoint the group is working on.
    pub waypoints: Vec<Waypoint>,
    /// Which waypoint is active, since when, and how far through it the group is.
    pub queue: WaypointQueue,
    /// `behaviour`: how the group moves and how alert it is.
    pub behaviour: Behaviour,
    /// `combatMode`: what the group does about an enemy it knows about.
    pub combat_mode: CombatMode,
    /// `speedMode`: how fast the group moves.
    pub speed_mode: SpeedMode,
    /// `formation`: the shape the group moves in.
    pub formation: Formation,
    /// The formation direction (a flat unit vector): reset to the leader's facing by
    /// `setFormation`, set by `setFormDir` and towards each waypoint the group turns to. `None`
    /// until first set; the leader's facing is used then.
    pub formation_direction: Option<DVec3>,
    /// The leader's share of his top speed (`formationCoef`, 0.1..1.5): slews towards what
    /// keeps the slowest follower in his slot.
    pub formation_coef: f64,
    /// What the group knows about its enemies (`knowsAbout`); shared by its units.
    pub targets: Targets,
    /// The last time the group met danger (a new contact), for the formation FSM's delays.
    pub last_danger: Option<f64>,
}

impl Default for GroupAi {
    fn default() -> Self {
        Self {
            waypoints: Vec::new(),
            queue: WaypointQueue::default(),
            behaviour: Behaviour::default(),
            combat_mode: CombatMode::default(),
            speed_mode: SpeedMode::default(),
            formation: Formation::default(),
            formation_direction: None,
            formation_coef: 1.0,
            targets: Targets::default(),
            last_danger: None,
        }
    }
}

/// The stance a [`UnitPos`] asks for.
fn stance_of(pos: UnitPos) -> a3_moves::Stance {
    match pos {
        UnitPos::Auto => a3_moves::Stance::Undefined,
        UnitPos::Up => a3_moves::Stance::Stand,
        UnitPos::Middle => a3_moves::Stance::Crouch,
        UnitPos::Down => a3_moves::Stance::Prone,
    }
}

/// The AI features `disableAI` / `enableAI` switch off, as the bits of the engine's enum
/// (`docs/re/sqf-object-state.md`, [`crate::AI_FEATURES`]). They live in the unit's
/// [`ObjectState`](crate::ObjectState); this is the AI's view of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AiFeatures(pub u32);

impl AiFeatures {
    pub const TARGET: u32 = 0x1;
    pub const MOVE: u32 = 0x2;
    pub const AUTOTARGET: u32 = 0x4;
    pub const ANIM: u32 = 0x8;
    pub const TEAMSWITCH: u32 = 0x10;
    /// No formation or danger FSM.
    pub const FSM: u32 = 0x40;
    pub const WEAPONAIM: u32 = 0x80;
    pub const AIMINGERROR: u32 = 0x100;
    pub const SUPPRESSION: u32 = 0x200;
    pub const CHECKVISIBLE: u32 = 0x400;
    pub const COVER: u32 = 0x800;
    pub const AUTOCOMBAT: u32 = 0x1000;
    pub const PATH: u32 = 0x2000;
    pub const MINEDETECTION: u32 = 0x4000;
    pub const NVG: u32 = 0x8000;
    pub const LIGHTS: u32 = 0x1_0000;
    pub const RADIOPROTOCOL: u32 = 0x2_0000;
    pub const FIREWEAPON: u32 = 0x4_0000;
    pub const COMMAND: u32 = 0x8_0000;
    pub const HEARING: u32 = 0x10_0000;
    pub const ALL: u32 = 0xffff_ffff;

    /// The bits of a `disableAI` feature name (any case), `None` for an unknown one.
    pub fn bit(name: &str) -> Option<u32> {
        crate::object_state::ai_feature(name)
    }

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }

    /// Whether the AI leaves the unit's movement to the script: `MOVE`, `PATH` or `ANIM` off.
    pub fn script_moves(self) -> bool {
        self.has(Self::MOVE | Self::PATH | Self::ANIM)
    }
}

/// How a unit's path is planned (`AIBrain+0x234`, `docs/re/ai.md` §4): what the formation FSM's
/// `formationIsLeader` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlanningMode {
    #[default]
    DoNotPlan,
    DoNotPlanFormation,
    LeaderPlanned,
    LeaderDirect,
    FormationPlanned,
    VehiclePlanned,
}

/// A unit's formation FSM, running.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitFsm {
    pub fsm: Arc<Fsm>,
    pub machine: Machine,
}

/// The AI state of one unit: what he was ordered to do outside his group's waypoints, the stance
/// he is asked to keep, his path and his formation FSM.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManAi {
    /// `doMove`/`moveTo`/`commandMove`: walk to this point instead of following the group.
    pub move_order: Option<DVec3>,
    /// `doStop`: stay here, out of the formation, until a waypoint or `doFollow` moves him.
    pub stopped: bool,
    /// The stance the formation FSM asks for (the engine's "weak" request).
    pub fsm_unit_pos: UnitPos,
    /// `forceSpeed`, in metres per second; `None` when not forced.
    pub force_speed: Option<f64>,
    /// How his path is planned.
    pub planning: PlanningMode,
    /// The path he is following, if he plans one.
    pub path: Option<UnitPath>,
    /// His formation FSM, once created.
    pub fsm: Option<UnitFsm>,
    /// Whether the formation FSM was looked up already (so a type without one is not asked
    /// again every frame).
    pub fsm_looked_up: bool,
    /// What the formation FSM keeps about covering and hiding.
    pub cover: CoverState,
    /// His last positions, newest first, for the men behind him in a SAFE file
    /// (`docs/re/ai.md` §4.3).
    pub trail: Vec<DVec3>,
}

impl World {
    /// Phase 5 of a frame: every local group works its waypoint queue, updates what it knows
    /// about its enemies, runs its units' formation FSMs and fills the [`ManInput`] of each of
    /// its units.
    pub(crate) fn perform_ai(&mut self, dt: f64) {
        for group in self.all_groups().map(|g| g.id()).collect::<Vec<_>>() {
            self.tick_group(group, dt);
        }
    }

    fn tick_group(&mut self, group: GroupId, dt: f64) {
        let Some(g) = self.group(group) else {
            return;
        };
        if !g.is_local() || g.units.is_empty() {
            return;
        }
        let units = g.units.clone();
        let behaviour = g.ai.behaviour;

        // What the group sees, before it decides where to go or what to look at.
        self.update_targets(group, &units, behaviour, dt);
        // The waypoint the group works on: turn to it, give the leader the move.
        self.turn_to_waypoint(group);
        // Where everyone's place is, and how fast the leader may go to keep them in it.
        let slots = self.formation_positions(group, &units);
        let leader_speed = self.leader_speed(group, &units, &slots, dt);
        // Each unit: his formation FSM, then his steering.
        for (index, unit) in units.iter().enumerate() {
            self.think_unit(group, *unit);
            let input = self.steer_unit(group, *unit, index, &units, &slots, leader_speed);
            if let Some(input) = input
                && let Some(man) = self.man_mut(*unit)
            {
                man.input = input;
            }
            self.record_trail(*unit);
        }
        // Arrivals, conditions and countdowns move the queue on.
        self.check_waypoint(group);
    }

    /// Updates what the group knows about its enemies: what it can see gains knowledge, what it
    /// cannot see is kept until [`FORGET_TIME`] runs out, and the gone are dropped.
    fn update_targets(
        &mut self,
        group: GroupId,
        units: &[EntityId],
        behaviour: Behaviour,
        dt: f64,
    ) {
        let now = self.time();
        let Some(g) = self.group(group) else {
            return;
        };
        let side = g.side;
        let Some(observer) = units.first().and_then(|u| self.entity(*u)) else {
            return;
        };
        let eye = observer.position() + DVec3::Y * EYE_HEIGHT;
        let range = VIEW_RANGE * behaviour.view_scale();
        let ignore: Vec<ObjectKey> = units.iter().copied().map(target_key).collect();
        let known: Vec<(EntityId, f64, f64, DVec3)> = self
            .group(group)
            .map(|g| {
                g.ai.targets
                    .iter()
                    .map(|k| (k.target, k.knowledge, k.last_seen, k.position))
                    .collect()
            })
            .unwrap_or_default();

        let candidates: Vec<(EntityId, DVec3)> = self
            .entities()
            .filter(|e| e.is_alive())
            .filter(|e| self.side_of(e.id()).is_some_and(|s| self.is_enemy(side, s)))
            .filter_map(|e| {
                let position = e.position() + DVec3::Y * EYE_HEIGHT;
                (flat_distance(position, eye) <= range).then(|| (e.id(), position))
            })
            .collect();

        let mut updates: Vec<TargetKnowledge> = Vec::new();
        let mut new_contact = false;
        for (target, position) in &candidates {
            let visible = self.is_visible(eye, *position, &ignore, *target);
            match known.iter().find(|(id, _, _, _)| id == target) {
                None if !visible => continue,
                None => {
                    new_contact = true;
                    updates.push(TargetKnowledge {
                        target: *target,
                        knowledge: (KNOWLEDGE_PER_SECOND * dt).min(4.0),
                        position: *position - DVec3::Y * EYE_HEIGHT,
                        last_seen: now,
                    })
                }
                Some(&(_, knowledge, last_seen, old_position)) => {
                    let (position, last_seen) = if visible {
                        (*position - DVec3::Y * EYE_HEIGHT, now)
                    } else {
                        (old_position, last_seen)
                    };
                    updates.push(TargetKnowledge {
                        target: *target,
                        knowledge: if visible {
                            (knowledge + KNOWLEDGE_PER_SECOND * dt).min(4.0)
                        } else {
                            knowledge
                        },
                        position,
                        last_seen,
                    });
                }
            }
        }

        let gone: Vec<EntityId> = self
            .group(group)
            .map(|g| {
                g.ai.targets
                    .iter()
                    .filter(|k| !self.entity(k.target).is_some_and(|e| e.is_alive()))
                    .map(|k| k.target)
                    .collect()
            })
            .unwrap_or_default();
        let Some(g) = self.group_mut(group) else {
            return;
        };
        for target in gone {
            g.ai.targets.forget(target);
        }
        let forgotten = g.ai.targets.retain_recent(now);
        for update in updates {
            if !forgotten.contains(&update.target) {
                g.ai.targets.insert(update);
            }
        }
        if new_contact {
            g.ai.last_danger = Some(now);
        }
    }

    /// Whether the eye at `eye` can see `position`: not hidden by terrain, a building or a wall
    /// (the view layer of a3-physics; `docs/re/ai.md`).
    fn is_visible(
        &self,
        eye: DVec3,
        position: DVec3,
        ignore: &[ObjectKey],
        target: EntityId,
    ) -> bool {
        let Some(collision) = self.collision_world() else {
            return false;
        };
        let mut keys = Vec::with_capacity(ignore.len() + 1);
        keys.extend_from_slice(ignore);
        keys.push(target_key(target));
        collision.visibility(eye, position, &keys) > 0.0
    }

    // ---- What the AI needs installed. ----

    /// Installs the formation table (`cfgFormations >> <side>`); the shipped one is the
    /// default.
    pub fn set_formation_table(&mut self, table: FormationTable) {
        self.ai.formations = table;
    }

    /// Loads every native FSM of `CfgFSMs` (the soldiers' `Formation`), so units whose
    /// `fsmFormation` names one run it. Returns the loader's warnings.
    pub fn load_native_fsms(&mut self, cfg_fsms: &a3_config::ConfigRef<'_>) -> Vec<String> {
        let mut warnings = Vec::new();
        for class in cfg_fsms.entries() {
            if !class.is_class() {
                continue;
            }
            match Fsm::from_native_config(&class) {
                Ok(loaded) => {
                    warnings.extend(loaded.warnings);
                    self.ai
                        .native_fsms
                        .insert(class.name().to_ascii_lowercase(), Arc::new(loaded.fsm));
                }
                Err(e) => warnings.push(format!("CfgFSMs >> {}: {e}", class.name())),
            }
        }
        warnings
    }

    /// Loads what the AI reads from the game config: the native FSMs of `CfgFSMs` and the
    /// formation table of `cfgFormations` (every side ships the same; `West` is used). Returns
    /// the FSM loader's warnings.
    pub fn load_ai_config(&mut self, config: &a3_config::ConfigTree) -> Vec<String> {
        let side = config.root() >> "cfgFormations" >> "West";
        if side.is_class() {
            self.set_formation_table(FormationTable::from_config(&side));
        }
        self.load_native_fsms(&(config.root() >> "CfgFSMs"))
    }

    /// Adds one native FSM by name (tests and tools).
    pub fn add_native_fsm(&mut self, fsm: Fsm) {
        self.ai
            .native_fsms
            .insert(fsm.name.to_ascii_lowercase(), Arc::new(fsm));
    }

    /// Installs the navigator paths are planned on. Without one, units walk straight at their
    /// goals.
    pub fn set_navigator(&mut self, navigator: a3_nav::Navigator) {
        self.ai.navigator = Some(navigator);
    }

    /// The navigator, to patch its grid.
    pub fn navigator_mut(&mut self) -> Option<&mut a3_nav::Navigator> {
        self.ai.navigator.as_mut()
    }

    /// Seeds the World's random generator (every AI choice that is random draws from it).
    pub fn seed_random(&mut self, seed: u32) {
        self.ai.rng = EngineRng::new(seed);
    }

    /// The next number in `0..1` from the World's random generator.
    pub fn random(&mut self) -> f64 {
        self.ai.rng.next_unit()
    }

    // ---- The waypoint queue, as the mission scripts and the network layer see it. ----

    /// `waypoints`: every waypoint of a group, in order.
    pub fn waypoints(&self, group: GroupId) -> &[Waypoint] {
        self.group(group)
            .map(|g| g.ai.waypoints.as_slice())
            .unwrap_or_default()
    }

    /// The waypoint at `index`, if the group has one there.
    pub fn waypoint(&self, group: GroupId, index: usize) -> Option<&Waypoint> {
        self.group(group)?.ai.waypoints.get(index)
    }

    /// The waypoint at `index`, to change (`setWaypoint*`).
    pub fn waypoint_mut(&mut self, group: GroupId, index: usize) -> Option<&mut Waypoint> {
        self.group_mut(group)?.ai.waypoints.get_mut(index)
    }

    /// `addWaypoint`: appends `waypoint` to the group's queue and returns its index. The first
    /// waypoint of a group becomes active at once, and applies its modes, as in the engine.
    pub fn add_waypoint(&mut self, group: GroupId, waypoint: Waypoint) -> Result<usize, Error> {
        let index = self
            .group(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .waypoints
            .len();
        self.insert_waypoint(group, index, waypoint)
    }

    /// `addWaypoint` with an index: inserts `waypoint` at `index` (the ones after it move up),
    /// appending when the index is past the end.
    pub fn insert_waypoint(
        &mut self,
        group: GroupId,
        index: usize,
        waypoint: Waypoint,
    ) -> Result<usize, Error> {
        let now = self.time();
        let (was_idle, index) = {
            let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
            let was_idle = g.ai.queue.current >= g.ai.waypoints.len();
            let index = index.min(g.ai.waypoints.len());
            g.ai.waypoints.insert(index, waypoint);
            // A waypoint inserted before the active one pushes it along.
            if !was_idle && index < g.ai.queue.current {
                g.ai.queue.current += 1;
            }
            if was_idle {
                g.ai.queue.current = index;
                g.ai.queue.started = now;
                g.ai.queue.turned = false;
                g.ai.queue.deadline = None;
            }
            (was_idle, index)
        };
        if was_idle {
            self.apply_waypoint_modes(group, index);
        }
        Ok(index)
    }

    /// `deleteWaypoint`: removes the waypoint at `index`; the ones after it move down one place
    /// (`docs/re/ai.md` on deleting the active one).
    pub fn delete_waypoint(&mut self, group: GroupId, index: usize) -> Result<(), Error> {
        let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
        if index >= g.ai.waypoints.len() {
            return Err(Error::NoSuchWaypoint { group, index });
        }
        g.ai.waypoints.remove(index);
        if index < g.ai.queue.current {
            g.ai.queue.current -= 1;
        } else if index == g.ai.queue.current {
            g.ai.queue.turned = false;
            g.ai.queue.deadline = None;
        }
        Ok(())
    }

    /// The index of the waypoint the group is working on — `Some(0)` for a group with waypoints
    /// and none done yet. It equals the number of waypoints once all of them are done.
    pub fn current_waypoint(&self, group: GroupId) -> Option<usize> {
        Some(self.group(group)?.ai.queue.current)
    }

    /// `setCurrentWaypoint`: makes the waypoint at `index` the active one and applies its modes;
    /// an index equal to the count means "every waypoint is done".
    pub fn set_current_waypoint(&mut self, group: GroupId, index: usize) -> Result<(), Error> {
        let count = self
            .group(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .waypoints
            .len();
        if index > count {
            return Err(Error::NoSuchWaypoint { group, index });
        }
        let now = self.time();
        let g = self.group_mut(group).expect("checked");
        g.ai.queue = WaypointQueue {
            current: index,
            started: now,
            turned: false,
            deadline: None,
        };
        self.apply_waypoint_modes(group, index);
        Ok(())
    }

    /// `copyWaypoints`: `to` gets a copy of `from`'s waypoints, replacing its own, and starts
    /// over at the first one.
    pub fn copy_waypoints(&mut self, from: GroupId, to: GroupId) -> Result<(), Error> {
        let waypoints = self
            .group(from)
            .ok_or(Error::NoSuchGroup(from))?
            .ai
            .waypoints
            .clone();
        self.group_mut(to)
            .ok_or(Error::NoSuchGroup(to))?
            .ai
            .waypoints = waypoints;
        self.set_current_waypoint(to, 0)
    }

    /// `move`: drop what the group was doing and walk to `position` — one MOVE waypoint, active
    /// at once (`docs/re/ai.md`).
    pub fn move_group(&mut self, group: GroupId, position: DVec3) -> Result<(), Error> {
        let now = self.time();
        let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
        g.ai.waypoints.clear();
        g.ai.waypoints
            .push(Waypoint::new(WaypointType::Move, position));
        g.ai.queue = WaypointQueue {
            current: 0,
            started: now,
            turned: false,
            deadline: None,
        };
        Ok(())
    }

    /// Applies what the waypoint at `index` says about the group's modes. A field the waypoint
    /// does not set means "no change", as `UNCHANGED` does in the engine.
    fn apply_waypoint_modes(&mut self, group: GroupId, index: usize) {
        let Some(g) = self.group_mut(group) else {
            return;
        };
        let Some(waypoint) = g.ai.waypoints.get(index) else {
            return;
        };
        if let Some(behaviour) = waypoint.behaviour {
            g.ai.behaviour = behaviour;
        }
        if let Some(combat_mode) = waypoint.combat_mode {
            g.ai.combat_mode = combat_mode;
        }
        if let Some(speed_mode) = waypoint.speed_mode {
            g.ai.speed_mode = speed_mode;
        }
        if let Some(formation) = waypoint.formation {
            g.ai.formation = formation;
        }
    }

    // ---- The group's modes. ----

    /// `behaviour`.
    pub fn group_behaviour(&self, group: GroupId) -> Option<Behaviour> {
        Some(self.group(group)?.ai.behaviour)
    }

    /// `setBehaviour`.
    pub fn set_group_behaviour(
        &mut self,
        group: GroupId,
        behaviour: Behaviour,
    ) -> Result<(), Error> {
        self.group_mut(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .behaviour = behaviour;
        Ok(())
    }

    /// `combatMode`.
    pub fn group_combat_mode(&self, group: GroupId) -> Option<CombatMode> {
        Some(self.group(group)?.ai.combat_mode)
    }

    /// `setCombatMode`.
    pub fn set_group_combat_mode(
        &mut self,
        group: GroupId,
        combat_mode: CombatMode,
    ) -> Result<(), Error> {
        self.group_mut(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .combat_mode = combat_mode;
        Ok(())
    }

    /// `speedMode`.
    pub fn group_speed_mode(&self, group: GroupId) -> Option<SpeedMode> {
        Some(self.group(group)?.ai.speed_mode)
    }

    /// `setSpeedMode`.
    pub fn set_group_speed_mode(&mut self, group: GroupId, speed: SpeedMode) -> Result<(), Error> {
        self.group_mut(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .speed_mode = speed;
        Ok(())
    }

    /// `formation`.
    pub fn group_formation(&self, group: GroupId) -> Option<Formation> {
        Some(self.group(group)?.ai.formation)
    }

    /// `setFormation`: the new shape, and the formation direction reset to where the leader
    /// faces (`AISubgroup_SetFormation`).
    pub fn set_group_formation(
        &mut self,
        group: GroupId,
        formation: Formation,
    ) -> Result<(), Error> {
        let facing = self.leader_facing(group);
        let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
        g.ai.formation = formation;
        if let Some(facing) = facing {
            g.ai.formation_direction = Some(facing);
        }
        Ok(())
    }

    /// `setFormDir`: the formation direction, as a heading in degrees.
    pub fn set_formation_direction(&mut self, group: GroupId, heading: f64) -> Result<(), Error> {
        let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
        g.ai.formation_direction = Some(movement::direction_of(heading));
        Ok(())
    }

    /// The formation direction of the group as a heading in degrees.
    pub fn formation_direction(&self, group: GroupId) -> Option<f64> {
        let direction = self.group_direction(group)?;
        Some(movement::heading_of(direction))
    }

    // ---- Orders on a single unit. ----

    /// `doMove`/`moveTo`/`commandMove`: this unit walks to `position`, whatever his group is
    /// doing, and stays there until something else moves him.
    pub fn order_move(&mut self, unit: EntityId, position: DVec3) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.move_order = Some(position);
            man.ai.stopped = false;
            man.ai.path = None;
        }
    }

    /// The point the unit was ordered to, until he arrives and his group takes over again
    /// (`moveToCompleted`, `docs/re/ai.md`).
    pub fn unit_move_order(&self, unit: EntityId) -> Option<DVec3> {
        self.man(unit)?.ai.move_order
    }

    /// Clears an order on one unit once he has arrived, so he returns to his formation slot.
    pub fn clear_move_order(&mut self, unit: EntityId) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.move_order = None;
            man.ai.path = None;
        }
    }

    /// `doStop`: the unit stays where he is and leaves the formation until a waypoint or
    /// `doFollow` moves him.
    pub fn stop_unit(&mut self, unit: EntityId) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.stopped = true;
            man.ai.move_order = None;
            man.ai.path = None;
        }
    }

    /// `stopped`.
    pub fn unit_stopped(&self, unit: EntityId) -> bool {
        self.man(unit).is_some_and(|man| man.ai.stopped)
    }

    /// `doFollow`: the unit falls back in with his group lead, dropping any order of his own.
    pub fn follow_unit(&mut self, unit: EntityId) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.stopped = false;
            man.ai.move_order = None;
            man.ai.path = None;
        }
    }

    /// `enableAI`/`disableAI` of every feature at once: whether the group drives this unit at
    /// all.
    pub fn set_man_ai_enabled(&mut self, unit: EntityId, enabled: bool) {
        self.set_ai_feature(unit, AiFeatures::ALL, enabled);
    }

    /// `enableAI` / `disableAI` of the features in `bits`.
    pub fn set_ai_feature(&mut self, unit: EntityId, bits: u32, enabled: bool) {
        if self.man(unit).is_none() {
            return;
        }
        let state = self.object_state_mut(unit);
        if enabled {
            state.ai_disabled &= !bits;
        } else {
            state.ai_disabled |= bits;
        }
    }

    /// The AI features switched off for the unit.
    pub fn ai_disabled(&self, unit: EntityId) -> AiFeatures {
        AiFeatures(self.object_state(unit).map_or(0, |s| s.ai_disabled))
    }

    /// `checkAIFeature`: whether every feature in `bits` is enabled for the unit.
    pub fn ai_feature_enabled(&self, unit: EntityId, bits: u32) -> bool {
        self.man(unit).is_some() && !self.ai_disabled(unit).has(bits)
    }

    /// `setUnitPos`.
    pub fn set_unit_pos(&mut self, unit: EntityId, pos: UnitPos) {
        if self.man(unit).is_some() {
            self.object_state_mut(unit).unit_pos = pos;
        }
    }

    /// The stance the unit is asked to keep: the script's (`setUnitPos`), else the formation
    /// FSM's own ("weak") request.
    pub fn effective_unit_pos(&self, unit: EntityId) -> UnitPos {
        let script = self
            .object_state(unit)
            .map_or(UnitPos::Auto, |s| s.unit_pos);
        if script != UnitPos::Auto {
            return script;
        }
        self.man(unit).map_or(UnitPos::Auto, |m| m.ai.fsm_unit_pos)
    }

    /// `forceSpeed`: caps the unit's speed in metres per second; negative removes the cap.
    pub fn force_speed(&mut self, unit: EntityId, speed: f64) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.force_speed = (speed >= 0.0).then_some(speed);
        }
    }

    /// `unitReady`: false only for a group leader whose group still has a move to make
    /// (`docs/re/ai.md` §5); a unit with an order of his own is ready once he arrived.
    pub fn unit_ready(&self, unit: EntityId) -> bool {
        let Some(man) = self.man(unit) else {
            return true;
        };
        if man.ai.move_order.is_some() {
            return false;
        }
        let Some(group) = self.group_of(unit) else {
            return true;
        };
        let Some(g) = self.group(group) else {
            return true;
        };
        if g.leader != Some(unit) {
            return true;
        }
        g.ai.waypoints
            .get(g.ai.queue.current)
            .is_none_or(|_| g.ai.queue.deadline.is_some())
    }

    /// `moveToCompleted`: the unit's own move is done (he has none left).
    pub fn move_to_completed(&self, unit: EntityId) -> bool {
        self.man(unit).is_none_or(|man| man.ai.move_order.is_none())
    }

    // ---- Target knowledge. ----

    /// The knowledge of `group` about `target`, 0..=4 (`knowsAbout`).
    pub fn knows_about_group(&self, group: GroupId, target: EntityId) -> f64 {
        // Units of a group always know about each other.
        if self.group_of(target) == Some(group) {
            return 4.0;
        }
        self.group(group)
            .map_or(0.0, |g| g.ai.targets.knowledge(target))
    }

    /// The knowledge of `unit` (and so of his group) about `target`, 0..=4 (`knowsAbout`).
    pub fn knows_about(&self, unit: EntityId, target: EntityId) -> f64 {
        if unit == target {
            return 4.0;
        }
        match self.group_of(unit) {
            Some(group) => self.knows_about_group(group, target),
            None => 0.0,
        }
    }

    /// What the group knows about each of its contacts (`targetsQuery`), empty for a group with
    /// no contacts yet and `None` for a gone group.
    pub fn group_targets(&self, group: GroupId) -> Option<&Targets> {
        Some(&self.group(group)?.ai.targets)
    }

    /// `reveal`: the group that owns `to_whom` learns about `target` — with `accuracy` when the
    /// script gives one, otherwise the best knowledge of any group on the revealing side, or 1
    /// when the side knows nothing about it (`reveal`).
    pub fn reveal(&mut self, to_whom: EntityId, target: EntityId, accuracy: Option<f64>) {
        let Some(group) = self.group_of(to_whom).or(self.group_of(target)) else {
            return;
        };
        self.reveal_group_to(group, target, accuracy);
    }

    /// `reveal` on a group directly.
    pub fn reveal_group(&mut self, group: GroupId, target: EntityId, accuracy: Option<f64>) {
        self.reveal_group_to(group, target, accuracy);
    }

    fn reveal_group_to(&mut self, group: GroupId, target: EntityId, accuracy: Option<f64>) {
        let Some(side) = self.group(group).map(|g| g.side) else {
            return;
        };
        let knowledge = match accuracy {
            Some(value) => value.clamp(0.0, 4.0),
            None => {
                let best = self
                    .all_groups()
                    .filter(|g| g.side == side)
                    .map(|g| g.ai.targets.knowledge(target))
                    .fold(0.0, f64::max);
                if best > 0.0 { best } else { 1.0 }
            }
        };
        if knowledge <= 0.0 {
            return;
        }
        let position = self.entity(target).map_or(DVec3::ZERO, |e| e.position());
        let now = self.time();
        if let Some(g) = self.group_mut(group) {
            g.ai.targets.insert(TargetKnowledge {
                target,
                knowledge,
                position,
                last_seen: now,
            });
        }
    }

    /// `forgetTarget`: the group drops everything it knows about `target`.
    pub fn forget_target(&mut self, group: GroupId, target: EntityId) {
        if let Some(g) = self.group_mut(group) {
            g.ai.targets.forget(target);
        }
    }
}

/// The flat (ground plane) distance between two points, in metres.
pub(crate) fn flat_distance(a: DVec3, b: DVec3) -> f64 {
    DVec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}
