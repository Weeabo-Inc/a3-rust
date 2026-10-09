//! Moving a group: turning to its waypoint, the formation slots, the leader's speed, paths, the
//! file a SAFE group walks in, and each unit's [`ManInput`] (`docs/re/ai.md` §3–§4).

use glam::{DVec3, Vec3};

use super::{
    Behaviour, Completion, PlanningMode, SpeedMode, WaypointQueue, WaypointType, flat_distance,
};
use crate::{EntityId, GroupId, ManInput, World};

/// How far behind his slot a follower runs instead of walking, in formation units (ours).
const RUN_BEHIND_SLOT: f64 = 2.0;

/// The speed cap below which a man stands rather than walks, in m/s (ours).
const STAND_BELOW: f64 = 0.3;

/// The speed cap from which a man runs rather than walks, in m/s (ours: between the walk and
/// the run of `CfgMovesMaleSdr`).
const RUN_FROM: f64 = 3.0;

/// The heading error a full turn input (`±1.0`) aims at, in degrees. Beyond this the unit stands
/// and turns first; below it he turns while walking.
const FULL_TURN_DEGREES: f64 = 90.0;

/// How many past positions a unit keeps for the men behind him (the engine's ring of 10).
const TRAIL_POINTS: usize = 10;

/// How far apart trail points are, in formation units along Z (`formationZ × 0.1`).
const TRAIL_SPACING: f64 = 0.1;

/// The radius a path is planned with: how far from the ends the grid may snap them (ours).
const PLAN_SNAP_RADIUS: f32 = 50.0;

/// How a man moves this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    Stand,
    Walk,
    Run,
}

/// A path a unit follows: points from where he planned to his goal.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitPath {
    /// What the path leads to.
    pub goal: DVec3,
    /// The points, the last being `goal` (or the nearest reachable place to it).
    pub points: Vec<DVec3>,
    /// The point he walks to now.
    pub next: usize,
}

impl UnitPath {
    /// Whether he has walked the whole path (`moveToCompleted`, the leader's state OK).
    pub fn finished(&self) -> bool {
        self.next >= self.points.len()
    }
}

/// The flat unit direction of a heading in degrees (north +Z, east +X).
pub(crate) fn direction_of(heading: f64) -> DVec3 {
    let (sin, cos) = heading.to_radians().sin_cos();
    DVec3::new(sin, 0.0, cos)
}

/// The heading in degrees `0..360` of a direction.
pub(crate) fn heading_of(direction: DVec3) -> f64 {
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

/// The turn input that brings a unit facing `heading` around to `direction`.
fn turn_towards(heading: f64, direction: DVec3) -> f32 {
    (heading_error(heading, direction) / FULL_TURN_DEGREES).clamp(-1.0, 1.0) as f32
}

/// A formation offset (right `x`, ahead `z`) turned into world metres for a formation facing
/// `direction`.
fn rotate(offset: DVec3, direction: DVec3) -> DVec3 {
    let right = DVec3::new(direction.z, 0.0, -direction.x);
    right * offset.x + direction * offset.z
}

fn flat(v: DVec3) -> DVec3 {
    DVec3::new(v.x, 0.0, v.z)
}

impl World {
    /// Where the leader faces, as a flat direction.
    pub(crate) fn leader_facing(&self, group: GroupId) -> Option<DVec3> {
        let leader = self.group(group)?.leader()?;
        Some(direction_of(self.entity(leader)?.heading()))
    }

    /// The group's formation direction: the one set, else where its leader faces.
    pub(crate) fn group_direction(&self, group: GroupId) -> Option<DVec3> {
        self.group(group)?
            .ai
            .formation_direction
            .or_else(|| self.leader_facing(group))
    }

    /// The engine's `Turn` state: when the active waypoint is new to the group, its direction
    /// turns towards it (the move command's direction, `docs/re/ai.md` §4.1) and its leader's
    /// path is dropped so he plans to it.
    pub(crate) fn turn_to_waypoint(&mut self, group: GroupId) {
        let Some(g) = self.group(group) else {
            return;
        };
        if g.ai.queue.turned {
            return;
        }
        let Some(waypoint) = g.ai.waypoints.get(g.ai.queue.current) else {
            return;
        };
        let target = waypoint.position;
        let leader = g.leader();
        let from = leader.and_then(|l| self.entity(l)).map(|e| e.position());
        let direction = from
            .map(|from| flat(target - from))
            .filter(|d| d.length_squared() > 1e-6)
            .map(|d| d.normalize());
        let g = self.group_mut(group).expect("checked");
        g.ai.queue.turned = true;
        if let Some(direction) = direction {
            g.ai.formation_direction = Some(direction);
        }
        if let Some(leader) = leader
            && let Some(man) = self.man_mut(leader)
        {
            man.ai.path = None;
        }
    }

    /// Every unit's formation slot in world space, in the order of `units` (the group's ID
    /// order): slot 0's offsets from the formation table, relative to the leader and turned by
    /// the formation direction (`AIUnit_ComputeFormationPos`).
    pub(crate) fn formation_positions(&self, group: GroupId, units: &[EntityId]) -> Vec<DVec3> {
        let Some(g) = self.group(group) else {
            return Vec::new();
        };
        let sizes: Vec<Option<(f64, f64)>> = units
            .iter()
            .map(|u| {
                self.entity(*u).map(|e| {
                    let ai = e.entity_type().ai();
                    (ai.formation_x, ai.formation_z)
                })
            })
            .collect();
        let slots = self.ai.formations.slots(g.ai.formation, &sizes);
        let leader_index = g
            .leader()
            .and_then(|l| units.iter().position(|u| *u == l))
            .unwrap_or(0);
        let leader_position = units
            .get(leader_index)
            .and_then(|l| self.entity(*l))
            .map_or(DVec3::ZERO, |e| e.position());
        let direction = self.group_direction(group).unwrap_or(DVec3::Z);
        let base = slots.get(leader_index).map_or(DVec3::ZERO, |s| s.offset);
        slots
            .iter()
            .map(|slot| leader_position + rotate(slot.offset - base, direction))
            .collect()
    }

    /// `formationPosition`: where the unit's slot is now, in world space.
    pub fn formation_position(&self, unit: EntityId) -> Option<DVec3> {
        let group = self.group_of(unit)?;
        let units = self.group(group)?.units.clone();
        let index = units.iter().position(|u| *u == unit)?;
        self.formation_positions(group, &units).get(index).copied()
    }

    /// `AISubgroup_SetLeaderSpeed`: the leader's speed cap, in m/s. He slows down when the
    /// follower furthest behind his slot lags, speeds up to 1.5x when everyone is ahead; LIMITED
    /// caps him (but not in COMBAT), FULL lets him go at 1.5x his top speed.
    pub(crate) fn leader_speed(
        &mut self,
        group: GroupId,
        units: &[EntityId],
        slots: &[DVec3],
        dt: f64,
    ) -> f64 {
        let Some(g) = self.group(group) else {
            return 0.0;
        };
        let Some(leader) = g.leader() else {
            return 0.0;
        };
        let behaviour = g.ai.behaviour;
        let speed_mode = g.ai.speed_mode;
        let coef = g.ai.formation_coef;
        let direction = self.group_direction(group).unwrap_or(DVec3::Z);
        let Some(leader_type) = self.entity(leader).map(|e| e.entity_type().clone()) else {
            return 0.0;
        };
        let v = leader_type.ai().max_speed.max(0.1);
        let (mut best_v, mut best_lag) = (v, 0.0);
        // Men walking a SAFE file follow a trail, not a slot, and do not hold the leader back.
        if !follows_trail(behaviour) {
            for (unit, slot) in units.iter().zip(slots) {
                if *unit == leader || self.unit_out_of_formation(*unit) {
                    continue;
                }
                let Some(e) = self.entity(*unit) else {
                    continue;
                };
                let lag = (*slot - e.position()).dot(direction) * 0.1;
                let vf = e.entity_type().ai().max_speed.max(0.1);
                if lag / vf > best_lag / best_v {
                    best_v = vf;
                    best_lag = lag;
                }
            }
        }
        let slack = if behaviour.is_combat() { 1.0 } else { 0.5 };
        let target = (best_v - (best_lag - slack) / 3.0) / v;
        let coef = (coef + (target - coef).clamp(-0.1 * dt, 0.1 * dt)).clamp(0.1, 1.5);
        if let Some(g) = self.group_mut(group) {
            g.ai.formation_coef = coef;
        }
        let speed = coef * v;
        match speed_mode {
            SpeedMode::Limited if behaviour != Behaviour::Combat => {
                speed.min((v * leader_type.ai().limited_speed_coef).max(0.1))
            }
            SpeedMode::Full => 1.5 * v,
            _ => speed,
        }
    }

    /// Whether a unit is out of the formation on an order of his own (or stopped, or driven by
    /// a script).
    fn unit_out_of_formation(&self, unit: EntityId) -> bool {
        self.ai_disabled(unit).script_moves()
            || self
                .man(unit)
                .is_none_or(|man| man.ai.move_order.is_some() || man.ai.stopped)
    }

    /// The input of one unit this frame; `None` leaves his input to the script.
    pub(crate) fn steer_unit(
        &mut self,
        group: GroupId,
        unit: EntityId,
        index: usize,
        units: &[EntityId],
        slots: &[DVec3],
        leader_speed: f64,
    ) -> Option<ManInput> {
        let g = self.group(group)?;
        if self.ai_disabled(unit).script_moves() {
            return None;
        }
        let stance = super::stance_of(self.effective_unit_pos(unit));
        let man = self.man(unit)?;
        let behaviour = g.ai.behaviour;
        let leader = g.leader();
        let is_leader = Some(unit) == leader;
        let entity = self.entity(unit)?;
        let ai = entity.entity_type().ai().clone();
        let position = entity.position();
        let heading = entity.heading();
        let force = man.ai.force_speed;
        let stance_input = ManInput {
            stance,
            ..Default::default()
        };

        // An order of his own: a path to it, at his group's pace.
        if let Some(order) = man.ai.move_order {
            if flat_distance(position, order) <= ai.precision.max(super::ARRIVE_RADIUS) {
                self.clear_move_order(unit);
                return Some(stance_input);
            }
            self.set_planning(unit, PlanningMode::LeaderPlanned);
            let pace = cap_pace(
                behaviour,
                force.map_or(ai.max_speed, |f| f.min(ai.max_speed)),
            );
            return Some(self.follow_path_to(unit, order, ai.precision, pace, stance));
        }
        if man.ai.stopped {
            return Some(self.face_input(group, unit, stance));
        }

        if is_leader {
            let destination =
                g.ai.waypoints
                    .get(g.ai.queue.current)
                    .filter(|_| g.ai.queue.deadline.is_none())
                    .map(|w| w.position);
            let Some(destination) = destination else {
                self.set_planning(unit, PlanningMode::DoNotPlan);
                return Some(self.face_input(group, unit, stance));
            };
            self.set_planning(unit, PlanningMode::LeaderPlanned);
            let cap = force.map_or(leader_speed, |f| f.min(leader_speed));
            let pace = cap_pace(behaviour, cap);
            return Some(self.follow_path_to(unit, destination, ai.precision, pace, stance));
        }

        // A follower: his slot, or the trail of the man ahead in a SAFE file.
        self.set_planning(unit, PlanningMode::FormationPlanned);
        let direction = self.group_direction(group).unwrap_or(DVec3::Z);
        let leader_moving = leader
            .and_then(|l| self.entity(l))
            .is_some_and(|l| flat(l.velocity()).length() > STAND_BELOW);
        let goal = if follows_trail(behaviour) {
            self.trail_goal(unit, index, units, leader)
        } else {
            slots.get(index).copied()
        };
        let Some(goal) = goal else {
            return Some(self.face_input(group, unit, stance));
        };
        let to_goal = flat(goal - position);
        let distance = to_goal.length();
        if distance <= ai.precision {
            return Some(self.face_input(group, unit, stance));
        }
        let behind = (goal - position).dot(direction);
        let mut pace = if behind > RUN_BEHIND_SLOT * ai.formation_z || !leader_moving {
            if distance > RUN_BEHIND_SLOT * ai.formation_z {
                Pace::Run
            } else {
                Pace::Walk
            }
        } else {
            cap_pace(behaviour, leader_speed)
        };
        if let Some(force) = force {
            pace = pace.min_with(cap_pace(behaviour, force));
        }
        Some(walk_input(heading, to_goal, pace, stance))
    }

    fn set_planning(&mut self, unit: EntityId, mode: PlanningMode) {
        if let Some(man) = self.man_mut(unit) {
            man.ai.planning = mode;
        }
    }

    /// Walks `unit` along his path to `goal`, planning it first (on the navigator when the
    /// World has one, straight otherwise) and again when the goal moved.
    fn follow_path_to(
        &mut self,
        unit: EntityId,
        goal: DVec3,
        precision: f64,
        pace: Pace,
        stance: a3_moves::Stance,
    ) -> ManInput {
        let Some(position) = self.entity(unit).map(|e| e.position()) else {
            return ManInput::default();
        };
        let replan = self
            .man(unit)
            .and_then(|m| m.ai.path.as_ref())
            .is_none_or(|p| flat_distance(p.goal, goal) > precision);
        if replan {
            let path = self.plan_path(position, goal);
            if let Some(man) = self.man_mut(unit) {
                man.ai.path = Some(path);
            }
        }
        let Some(man) = self.man_mut(unit) else {
            return ManInput::default();
        };
        let Some(path) = man.ai.path.as_mut() else {
            return ManInput::default();
        };
        // Points he is already at are behind him; the last needs his precision.
        while let Some(point) = path.points.get(path.next) {
            let last = path.next + 1 == path.points.len();
            let reach = if last {
                precision
            } else {
                precision.max(super::ARRIVE_RADIUS)
            };
            if flat_distance(*point, position) <= reach {
                path.next += 1;
            } else {
                break;
            }
        }
        let Some(point) = path.points.get(path.next).copied() else {
            return ManInput {
                stance,
                ..Default::default()
            };
        };
        let heading = self.entity(unit).map_or(0.0, |e| e.heading());
        walk_input(heading, flat(point - position), pace, stance)
    }

    /// A path from `from` to `goal`: the navigator's, or the straight line.
    fn plan_path(&mut self, from: DVec3, goal: DVec3) -> UnitPath {
        let planned = self.ai.navigator.as_mut().and_then(|nav| {
            nav.find_path(
                Vec3::new(from.x as f32, from.y as f32, from.z as f32),
                Vec3::new(goal.x as f32, goal.y as f32, goal.z as f32),
                PLAN_SNAP_RADIUS,
            )
        });
        let points = match planned {
            Some(points) if !points.is_empty() => points
                .into_iter()
                .skip(1)
                .map(|p| DVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)))
                .collect(),
            _ => vec![goal],
        };
        UnitPath {
            goal,
            points,
            next: 0,
        }
    }

    /// Standing in place: turning to face what the group hunts, if it does.
    fn face_input(&self, group: GroupId, unit: EntityId, stance: a3_moves::Stance) -> ManInput {
        let mut input = ManInput {
            stance,
            ..Default::default()
        };
        let Some(g) = self.group(group) else {
            return input;
        };
        if !(g.ai.combat_mode.pursues() || g.ai.behaviour.is_combat()) {
            return input;
        }
        let Some(known) = g.ai.targets.best() else {
            return input;
        };
        let Some(entity) = self.entity(unit) else {
            return input;
        };
        if flat_distance(known.position, entity.position()) > 1.0 {
            input.turn = turn_towards(entity.heading(), known.position - entity.position());
        }
        input
    }

    /// Where a man in a SAFE file walks: back along the trail of the man ahead of him (the
    /// nearest lower-numbered man walking the file, else the leader) by
    /// `formationTime × his speed × 0.6`, at least `1.5 r + 2.5` (`docs/re/ai.md` §4.3). `None`
    /// to hold: the point is close and behind him.
    fn trail_goal(
        &self,
        unit: EntityId,
        index: usize,
        units: &[EntityId],
        leader: Option<EntityId>,
    ) -> Option<DVec3> {
        let ahead = units[..index]
            .iter()
            .rev()
            .find(|u| Some(**u) != leader && !self.unit_out_of_formation(**u))
            .copied()
            .or(leader)?;
        let ahead_entity = self.entity(ahead)?;
        let me = self.entity(unit)?;
        let ai = me.entity_type().ai();
        let ahead_ai = ahead_entity.entity_type().ai();
        let speed = flat(ahead_entity.velocity()).length();
        // r: a man's radius (ours: 0.5 m).
        let look_back = (ahead_ai.formation_time * speed * 0.6).max(1.5 * 0.5 + 2.5);
        let trail = self
            .man(ahead)
            .map(|m| m.ai.trail.as_slice())
            .unwrap_or(&[]);
        let mut point = ahead_entity.position();
        let mut left = look_back;
        for next in trail {
            let step = flat_distance(point, *next);
            if step >= left {
                point += (*next - point) * (left / step);
                break;
            }
            left -= step;
            point = *next;
        }
        let hold_within = 1.25 * (ahead_ai.formation_z + ai.formation_z);
        let towards_ahead = flat(ahead_entity.position() - me.position());
        let behind_me = flat(point - me.position()).dot(towards_ahead) < 0.0;
        if flat_distance(point, me.position()) < hold_within && behind_me {
            return None;
        }
        Some(point)
    }

    /// Adds the unit's position to his trail when he has moved a trail step.
    pub(crate) fn record_trail(&mut self, unit: EntityId) {
        let Some(entity) = self.entity(unit) else {
            return;
        };
        let position = entity.position();
        let spacing = entity.entity_type().ai().formation_z * TRAIL_SPACING;
        let Some(man) = self.man_mut(unit) else {
            return;
        };
        let trail = &mut man.ai.trail;
        if trail
            .first()
            .is_none_or(|last| flat_distance(*last, position) >= spacing)
        {
            trail.insert(0, position);
            trail.truncate(TRAIL_POINTS);
        }
    }

    /// The engine's arrival test, completion condition and countdown (`docs/re/ai.md` §3): the
    /// AI leader has walked his path or is within `max(completionRadius, precision)` (3-D); the
    /// type's condition then holds; a random `min/mid/max` countdown runs; then the waypoint is
    /// done.
    pub(crate) fn check_waypoint(&mut self, group: GroupId) {
        let now = self.time();
        let Some(g) = self.group(group) else {
            return;
        };
        let index = g.ai.queue.current;
        let Some(waypoint) = g.ai.waypoints.get(index) else {
            return;
        };
        if let Some(deadline) = g.ai.queue.deadline {
            if now >= deadline {
                self.advance_waypoint(group, index);
            }
            return;
        }
        let Some(leader) = g.leader() else {
            return;
        };
        let Some(leader_entity) = self.entity(leader) else {
            return;
        };
        let precision = leader_entity.entity_type().ai().precision;
        let radius = waypoint.arrival_radius(precision);
        let near = (leader_entity.position() - waypoint.position).length() <= radius;
        let path_done = self.man(leader).is_some_and(|m| {
            m.ai.path
                .as_ref()
                .is_some_and(|p| p.finished() && flat_distance(p.goal, waypoint.position) < 1e-3)
        });
        if !(near || path_done) {
            return;
        }
        let condition = match waypoint.waypoint_type.completion() {
            Completion::Arrival => true,
            Completion::Never => false,
            Completion::IdentifiedEnemy => g.ai.targets.iter().any(|k| k.knowledge >= 1.5),
            Completion::Cleared => g.ai.targets.is_empty(),
            Completion::Combat => {
                let units = g.units.clone();
                g.ai.behaviour.is_combat() || units.is_empty()
            }
        };
        if !condition {
            return;
        }
        let [min, mid, max] = waypoint.timeout;
        let wait = self.ai.rng.min_mid_max(min, mid, max).max(0.0);
        if let Some(g) = self.group_mut(group) {
            g.ai.queue.deadline = Some(now + wait);
        }
        if wait <= 0.0 {
            self.advance_waypoint(group, index);
        }
    }

    /// Marks the waypoint at `index` done: the group starts the next one (or the first again,
    /// after a CYCLE waypoint), that waypoint's mode changes are applied, and the event goes out
    /// for the mission's scripts.
    pub(crate) fn advance_waypoint(&mut self, group: GroupId, index: usize) {
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
        g.ai.queue = WaypointQueue {
            current: next,
            started: now,
            turned: false,
            deadline: None,
        };
        self.apply_waypoint_modes(group, next);
        self.push_event(crate::WorldEvent::WaypointCompleted { group, index });
    }
}

/// Whether men of a group in `behaviour` walk in a file on the trail of the man ahead instead of
/// keeping their slots (CARELESS, SAFE; `docs/re/ai.md` §4.3; the road case is not modelled).
fn follows_trail(behaviour: Behaviour) -> bool {
    matches!(behaviour, Behaviour::Careless | Behaviour::Safe)
}

/// The pace a speed cap allows a man of a group in `behaviour` (ours, `docs/re/ai.md` §4.4: the
/// engine's gait choice was not traced). CARELESS and SAFE walk; the others run when the cap
/// allows.
fn cap_pace(behaviour: Behaviour, cap: f64) -> Pace {
    if cap < STAND_BELOW {
        Pace::Stand
    } else if cap < RUN_FROM || matches!(behaviour, Behaviour::Careless | Behaviour::Safe) {
        Pace::Walk
    } else {
        Pace::Run
    }
}

impl Pace {
    fn min_with(self, other: Pace) -> Pace {
        let rank = |p: Pace| match p {
            Pace::Stand => 0,
            Pace::Walk => 1,
            Pace::Run => 2,
        };
        if rank(other) < rank(self) {
            other
        } else {
            self
        }
    }
}

/// The input that walks a man facing `heading` along `to` at `pace`.
fn walk_input(heading: f64, to: DVec3, pace: Pace, stance: a3_moves::Stance) -> ManInput {
    if pace == Pace::Stand || to.length_squared() < 1e-9 {
        return ManInput {
            stance,
            ..Default::default()
        };
    }
    // Walking while turning sharply would carry him the wrong way: he turns on the spot until
    // the goal is ahead of him.
    let forward = if heading_error(heading, to).abs() < FULL_TURN_DEGREES {
        1.0
    } else {
        0.0
    };
    ManInput {
        forward,
        strafe: 0.0,
        turn: turn_towards(heading, to),
        sprint: pace == Pace::Run,
        stance,
    }
}
