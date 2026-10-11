//! The AI of groups: waypoints, formations and what a group knows about its enemies, turned
//! into each unit's [`ManInput`] once per frame (issue #129).
//!
//! Phase 5 of [`World::simulate`]. Only groups whose [`Locality`](crate::Locality) is local think
//! here — a group owned by another machine is driven there, and its units on this machine only
//! replay the state that arrives over the network. Nothing in this module reads a clock:
//! the tick uses `dt` of World time, so a mission replays the same way at any frame rate.
//!
//! The design, the constants and how sure we are of each: `docs/re/ai.md`.

mod target;
mod waypoint;

pub use target::{
    EYE_HEIGHT, FORGET_TIME, KNOWLEDGE_PER_SECOND, TargetKnowledge, Targets, VIEW_RANGE, target_key,
};
pub use waypoint::{
    Behaviour, CombatMode, DEFAULT_COMPLETION_RADIUS, FORMATION_SPACING, Formation, LoiterType,
    SpeedMode, Waypoint, WaypointQueue, WaypointType,
};

use a3_physics::ObjectKey;
use glam::DVec3;

use crate::{EntityId, Error, GroupId, ManInput, World};

/// How close a unit must be to where it is walking before it stops, in metres. A waypoint's own
/// completion radius is usually wider than this, so the group comes to a stop inside it.
pub const ARRIVE_RADIUS: f64 = 1.0;

/// The cowardice (`allowFleeing`, 0..1) above which a unit breaks off: he stops working his
/// group's waypoint and runs from the nearest contact instead. Ours, not traced — the engine's
/// own break depends on the group's losses and the leader's courage sub-skill.
const FLEEING_THRESHOLD: f32 = 0.5;

/// How far from the contact a broken unit runs, in metres.
const FLEE_DISTANCE: f64 = 100.0;

/// The heading error a full turn input (`±1.0`) aims at, in degrees. Beyond this the unit stands
/// and turns first; below it he turns while walking.
const FULL_TURN_DEGREES: f64 = 90.0;

/// The AI state of a group: its waypoint queue, the modes it moves under, and what it knows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupAi {
    /// The queue, and the waypoint the group is working on.
    pub waypoints: Vec<Waypoint>,
    /// Which waypoint is active, and since when.
    pub queue: WaypointQueue,
    /// `behaviour`: how the group moves and how alert it is.
    pub behaviour: Behaviour,
    /// `combatMode`: what the group does about an enemy it knows about.
    pub combat_mode: CombatMode,
    /// `speedMode`: how fast the group moves.
    pub speed_mode: SpeedMode,
    /// `formation`: the shape the group moves in.
    pub formation: Formation,
    /// What the group knows about its enemies (`knowsAbout`); shared by its units.
    pub targets: Targets,
}

/// The AI state of one unit: what he was ordered to do outside his group's waypoints.
///
/// The group fills his [`ManInput`] from its own orders; these are the overrides a mission script
/// set on him alone.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManAi {
    /// `doMove`/`moveTo`/`commandMove`: walk to this point instead of following the group.
    pub move_order: Option<DVec3>,
    /// `doStop`: stay here, out of the formation, until a waypoint or `doFollow` moves him.
    pub stopped: bool,
    /// `disableAI`: the mission script drives him itself, so the group leaves him alone.
    pub disabled: bool,
}

/// What a group's waypoint queue asks of it while its units are being steered.
#[derive(Debug, Clone, Copy)]
struct Orders {
    /// Index of the waypoint in the group's queue.
    index: usize,
    /// Whether arriving is enough to finish it.
    completes_on_arrival: bool,
    /// Whether it also needs the group to know about no enemies.
    needs_a_clear_area: bool,
    /// Where the group leader walks to.
    position: DVec3,
    /// How close the leader has to get to it, in metres.
    radius: f64,
    /// Seconds after which the waypoint is done whether the group got there or not.
    timeout: f64,
}

impl World {
    /// Phase 5 of a frame: every local group works its waypoint queue, updates what it knows
    /// about its enemies, and fills the [`ManInput`] of each of its units.
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
        let combat_mode = g.ai.combat_mode;

        // What the group sees, before it decides where to go or what to look at.
        self.update_targets(group, &units, behaviour, dt);

        // Where every unit should be: the active waypoint, the formation, or his own order.
        let orders = self.active_orders(group);
        match orders {
            Some(orders) => self.steer_group(group, &units, orders, combat_mode),
            None => self.stand_group(group, &units, combat_mode),
        }

        // Arrivals and timeouts move the queue on.
        if let Some(orders) = orders {
            self.check_orders(group, orders);
        }
    }

    /// The waypoint the group is working on, if any. `None` once the queue is done, and for a
    /// group with no waypoints at all.
    fn active_orders(&self, group: GroupId) -> Option<Orders> {
        let g = self.group(group)?;
        let waypoint = g.ai.waypoints.get(g.ai.queue.current)?;
        Some(Orders {
            index: g.ai.queue.current,
            completes_on_arrival: waypoint.waypoint_type.completes_on_arrival(),
            needs_a_clear_area: waypoint.waypoint_type.needs_a_clear_area(),
            position: waypoint.position,
            radius: waypoint.effective_completion_radius(),
            timeout: waypoint.timeout[1],
        })
    }

    /// Steers every unit towards his place: the leader to the active waypoint, a follower to his
    /// slot in the formation behind the leader, and a unit with an order of his own to that.
    fn steer_group(
        &mut self,
        group: GroupId,
        units: &[EntityId],
        orders: Orders,
        combat_mode: CombatMode,
    ) {
        let Some(g) = self.group(group) else {
            return;
        };
        let leader = g.leader();
        let formation = g.ai.formation;
        let speed = g.ai.speed_mode;
        let leader_pose = leader
            .and_then(|l| self.entity(l))
            .map(|e| (e.position(), e.heading()));

        for (index, unit) in units.iter().enumerate() {
            let is_leader = Some(*unit) == leader;
            let Some(man) = self.man(*unit) else {
                continue;
            };
            if man.ai.disabled {
                continue;
            }
            let order = man.ai.move_order;
            let stopped = man.ai.stopped;
            // A broken unit (`allowFleeing`) has somewhere else to be than his waypoint; with
            // nothing to run from he keeps his group's orders.
            let flee = if order.is_none() && !stopped {
                self.flee_goal(group, *unit)
            } else {
                None
            };
            let goal = match (flee, order, stopped, is_leader) {
                (Some(flee), _, _, _) => Some(flee),
                // An order of his own overrides everything the group is doing.
                (None, Some(order), _, _) => Some(order),
                (None, None, true, _) => None,
                (None, None, false, true) => Some(orders.position),
                (None, None, false, false) => leader_pose.map(|(position, heading)| {
                    position + rotate_flat(formation.offset(index), heading)
                }),
            };
            let input = match goal {
                Some(goal) => self.walk_towards(*unit, goal, speed),
                // In place: face what the group knows about, when it is hunting.
                None => self.face_target(group, *unit, combat_mode),
            };
            if let Some(order) = order {
                self.finish_move_order(*unit, order);
            }
            if let Some(man) = self.man_mut(*unit) {
                man.input = input;
            }
        }
    }

    /// Has every unit stand where he is (queue done, or a group with no waypoints yet), walking
    /// on if he has an order of his own. The group stands in formation: a follower closes up on
    /// his slot behind the leader before he comes to rest.
    fn stand_group(&mut self, group: GroupId, units: &[EntityId], combat_mode: CombatMode) {
        let Some(g) = self.group(group) else {
            return;
        };
        let leader = g.leader();
        let formation = g.ai.formation;
        let speed = g.ai.speed_mode;
        let leader_pose = leader
            .and_then(|l| self.entity(l))
            .map(|e| (e.position(), e.heading()));
        for (index, unit) in units.iter().enumerate() {
            let is_leader = Some(*unit) == leader;
            let Some(man) = self.man(*unit) else {
                continue;
            };
            if man.ai.disabled {
                continue;
            }
            let order = man.ai.move_order;
            let stopped = man.ai.stopped;
            // Broken (`allowFleeing`) even with the queue done: away from the contact.
            let flee = if order.is_none() && !stopped {
                self.flee_goal(group, *unit)
            } else {
                None
            };
            let goal = match (flee, order, stopped, is_leader) {
                (Some(flee), _, _, _) => Some(flee),
                (None, Some(order), _, _) => Some(order),
                (None, None, true, _) => None,
                (None, None, false, true) => None,
                (None, None, false, false) => leader_pose.map(|(position, heading)| {
                    position + rotate_flat(formation.offset(index), heading)
                }),
            };
            let input = match goal {
                Some(goal) => self.walk_towards(*unit, goal, speed),
                None => self.face_target(group, *unit, combat_mode),
            };
            if let Some(order) = order {
                self.finish_move_order(*unit, order);
            }
            if let Some(man) = self.man_mut(*unit) {
                man.input = input;
            }
        }
    }

    /// Ends a unit's own move order once he is there. `doMove` to a point is done when the unit
    /// arrives: he goes back to his place in the formation — the leader to what the group is
    /// doing, a follower to his slot behind him — until the next order comes.
    fn finish_move_order(&mut self, unit: EntityId, order: DVec3) {
        let arrived = self
            .entity(unit)
            .is_some_and(|e| flat_distance(e.position(), order) <= ARRIVE_RADIUS);
        if arrived {
            self.clear_move_order(unit);
        }
    }

    /// Where a unit whose courage broke (`allowFleeing`, read back by `fleeing`) runs to: straight
    /// away from the nearest contact his group knows about, [`FLEE_DISTANCE`] metres out. `None`
    /// while he holds — nobody is known, or his cowardice is at or below [`FLEEING_THRESHOLD`].
    fn flee_goal(&self, group: GroupId, unit: EntityId) -> Option<DVec3> {
        if self.fleeing(unit) <= FLEEING_THRESHOLD {
            return None;
        }
        let position = self.entity(unit)?.position();
        let threat = self
            .group(group)?
            .ai
            .targets
            .iter()
            .map(|known| known.position)
            .min_by(|a, b| flat_distance(position, *a).total_cmp(&flat_distance(position, *b)))?;
        let away = DVec3::new(position.x - threat.x, 0.0, position.z - threat.z);
        // Standing on top of him: any way out will do.
        let away = if away.length_squared() > 1e-6 {
            away.normalize()
        } else {
            DVec3::X
        };
        Some(position + away * FLEE_DISTANCE)
    }

    /// The input that stands `unit` still, turning him towards what the group knows about when
    /// it is on the offensive.
    fn face_target(&self, group: GroupId, unit: EntityId, combat_mode: CombatMode) -> ManInput {
        let Some(g) = self.group(group) else {
            return ManInput::default();
        };
        if !(combat_mode.pursues() || g.ai.behaviour.is_combat()) {
            return ManInput::default();
        }
        let Some(known) = g.ai.targets.best() else {
            return ManInput::default();
        };
        let Some(entity) = self.entity(unit) else {
            return ManInput::default();
        };
        if flat_distance(known.position, entity.position()) <= ARRIVE_RADIUS {
            return ManInput::default();
        }
        ManInput {
            turn: turn_towards(entity.heading(), known.position - entity.position()),
            ..Default::default()
        }
    }

    /// The input that walks `unit` towards `goal` at his group's speed mode.
    fn walk_towards(&self, unit: EntityId, goal: DVec3, speed: SpeedMode) -> ManInput {
        let Some(entity) = self.entity(unit) else {
            return ManInput::default();
        };
        if flat_distance(goal, entity.position()) <= ARRIVE_RADIUS {
            return ManInput::default();
        }
        let to = goal - entity.position();
        // Walking while turning sharply would carry him the wrong way: he turns on the spot
        // until the goal is ahead of him.
        let forward = if heading_error(entity.heading(), to).abs() < FULL_TURN_DEGREES {
            1.0
        } else {
            0.0
        };
        ManInput {
            forward,
            strafe: 0.0,
            turn: turn_towards(entity.heading(), to),
            // LIMITED walks; NORMAL and FULL both use the fastest move the graph has until the
            // sprint moves exist (#124).
            sprint: speed != SpeedMode::Limited,
            ..Default::default()
        }
    }

    /// Whether the waypoint's conditions are met; the queue moves on when they are.
    fn check_orders(&mut self, group: GroupId, orders: Orders) {
        let now = self.time();
        let Some(g) = self.group(group) else {
            return;
        };
        let elapsed = now - g.ai.queue.started;
        let Some(leader) = g.leader() else {
            return;
        };
        let arrived = self
            .entity(leader)
            .is_some_and(|e| flat_distance(e.position(), orders.position) <= orders.radius);
        let clear = !orders.needs_a_clear_area || g.ai.targets.is_empty();
        let timed_out = orders.timeout > 0.0 && elapsed >= orders.timeout;
        if (orders.completes_on_arrival && arrived && clear) || timed_out {
            self.advance_waypoint(group, orders.index);
        }
    }

    /// Marks the waypoint at `index` done: the group starts the next one (or the first again,
    /// after a CYCLE waypoint), that waypoint's mode changes are applied, and the event goes out
    /// for the mission's scripts.
    fn advance_waypoint(&mut self, group: GroupId, index: usize) {
        let now = self.time();
        let Some(g) = self.group_mut(group) else {
            return;
        };
        if g.ai.queue.current != index {
            return;
        }
        let Some(waypoint) = g.ai.waypoints.get(index) else {
            return;
        };
        let next = if waypoint.waypoint_type == WaypointType::Cycle {
            0
        } else {
            index + 1
        };
        g.ai.queue.current = next;
        g.ai.queue.started = now;
        self.apply_waypoint_modes(group, next);
        self.push_event(crate::WorldEvent::WaypointCompleted { group, index });
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

    /// Updates what the group knows about its enemies: what it can see gains knowledge, what it
    /// cannot see is kept until [`FORGET_TIME`] runs out, and the gone are dropped.
    ///
    /// Every candidate is checked against the group's eye (`VIEW_RANGE` scaled by the behaviour,
    /// taken from its first unit) and everything within that range is tested for line of sight
    /// through the collision world's view layer. That is the plainest reading of the engine's
    /// contact model; a real mission pays for the ray casts and `docs/re/ai.md` says so.
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
        // What the group knows before this frame: knowledge to add to, when each contact was
        // last seen, and where it was — kept while it is out of sight.
        let known: Vec<(EntityId, f64, f64, DVec3)> = self
            .group(group)
            .map(|g| {
                g.ai.targets
                    .iter()
                    .map(|k| (k.target, k.knowledge, k.last_seen, k.position))
                    .collect()
            })
            .unwrap_or_default();

        // What is out there to see: living enemies of the group's side within view range.
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
        for (target, position) in &candidates {
            let visible = self.is_visible(eye, *position, &ignore, *target);
            match known.iter().find(|(id, _, _, _)| id == target) {
                // A target never seen before is only noticed when it is in plain sight.
                None if !visible => continue,
                None => updates.push(TargetKnowledge {
                    target: *target,
                    knowledge: (KNOWLEDGE_PER_SECOND * dt).min(4.0),
                    position: *position - DVec3::Y * EYE_HEIGHT,
                    last_seen: now,
                }),
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

        // The dead and the deleted are not contact's work to keep; the forgotten have run out
        // their [`FORGET_TIME`].
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
        // A contact whose time ran out stays dropped: its own update, built from the knowledge
        // it had, would otherwise put it straight back.
        let forgotten = g.ai.targets.retain_recent(now);
        for update in updates {
            if !forgotten.contains(&update.target) {
                g.ai.targets.insert(update);
            }
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
        let was_empty = {
            let g = self.group_mut(group).ok_or(Error::NoSuchGroup(group))?;
            let was_empty = g.ai.waypoints.is_empty();
            let index = index.min(g.ai.waypoints.len());
            g.ai.waypoints.insert(index, waypoint);
            // A waypoint inserted before the active one pushes it along.
            if !was_empty && index < g.ai.queue.current {
                g.ai.queue.current += 1;
            }
            (was_empty, index)
        };
        let (was_empty, index) = was_empty;
        if was_empty {
            self.apply_waypoint_modes(group, 0);
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
        g.ai.queue.current = index;
        g.ai.queue.started = now;
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
        g.ai.queue.current = 0;
        g.ai.queue.started = now;
        Ok(())
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

    /// `setFormation`.
    pub fn set_group_formation(
        &mut self,
        group: GroupId,
        formation: Formation,
    ) -> Result<(), Error> {
        self.group_mut(group)
            .ok_or(Error::NoSuchGroup(group))?
            .ai
            .formation = formation;
        Ok(())
    }

    // ---- Orders on a single unit. ----

    /// `doMove`/`moveTo`/`commandMove` (and `commandMove`'s radio message later): this unit
    /// walks to `position`, whatever his group is doing, and stays there until something else
    /// moves him.
    pub fn order_move(&mut self, unit: EntityId, position: DVec3) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.move_order = Some(position);
            man.ai.stopped = false;
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
        }
    }

    /// `doStop`: the unit stays where he is and leaves the formation until a waypoint or
    /// `doFollow` moves him.
    pub fn stop_unit(&mut self, unit: EntityId) {
        self.set_unit_stopped(unit, true);
    }

    /// `stop unit toggle` (0x541980): the engine's scripted flag that keeps a unit from moving or
    /// turning, the one `stopped` reads. `false` lets him go again; stopping drops any order of
    /// his own, as `doStop` does.
    pub fn set_unit_stopped(&mut self, unit: EntityId, stopped: bool) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.stopped = stopped;
            if stopped {
                man.ai.move_order = None;
            }
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
        }
    }

    /// `enableAI`/`disableAI`: whether the group drives this unit at all.
    pub fn set_man_ai_enabled(&mut self, unit: EntityId, enabled: bool) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.disabled = !enabled;
        }
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
fn flat_distance(a: DVec3, b: DVec3) -> f64 {
    DVec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

/// The heading (degrees clockwise from north) of a direction, as [`crate::Entity::heading`]
/// reports one.
fn heading_of(direction: DVec3) -> f64 {
    direction
        .x
        .atan2(direction.z)
        .to_degrees()
        .rem_euclid(360.0)
}

/// How far a unit facing `heading` has to turn to face `direction`: positive to his right,
/// `-180..=180` degrees.
fn heading_error(heading: f64, direction: DVec3) -> f64 {
    let difference = heading_of(direction) - heading;
    (difference + 180.0).rem_euclid(360.0) - 180.0
}

/// The turn input that brings a unit facing `heading` around to `direction`: a full turn at
/// [`FULL_TURN_DEGREES`] or more of error, a proportional one below it.
fn turn_towards(heading: f64, direction: DVec3) -> f32 {
    (heading_error(heading, direction) / FULL_TURN_DEGREES).clamp(-1.0, 1.0) as f32
}

/// A formation offset (to the right `x`, ahead `z`) in world metres, turned into the frame of a
/// unit facing `heading` degrees.
fn rotate_flat(offset: DVec3, heading: f64) -> DVec3 {
    let (sin, cos) = heading.to_radians().sin_cos();
    // North (heading 0) is +Z and east is +X, clockwise from above.
    let right = DVec3::new(cos, 0.0, -sin);
    let forward = DVec3::new(sin, 0.0, cos);
    right * offset.x + forward * offset.z
}
