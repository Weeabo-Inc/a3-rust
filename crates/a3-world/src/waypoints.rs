//! Group waypoints: the data `addWaypoint` and the `waypoint*` commands read and write.
//!
//! A waypoint is not an Object: scripts see it as a two-element array `[group, index]`, and the
//! waypoints of a group live in a list on the group. This module is the data side; driving a
//! group along its waypoints is the AI's job (#129).
//!
//! Defaults are the engine's: an added waypoint is a `MOVE` waypoint with no description,
//! `UNCHANGED` behaviour, combat mode, formation and speed, a placement radius of 0 and a
//! timeout of `[0, 0, 0]`.

use glam::DVec3;

use crate::{GroupId, World};

/// One waypoint of a group.
#[derive(Debug, Clone, PartialEq)]
pub struct Waypoint {
    /// Position (the centre of the waypoint's area), world space.
    pub position: DVec3,
    /// `setWaypointPosition`'s radius: how far from the centre the unit may be while the
    /// waypoint counts as reached.
    pub radius: f32,
    /// `waypointName` / `setWaypointName`.
    pub name: String,
    /// `waypointType`: `"MOVE"`, `"SAD"`, `"CYCLE"`, ...; `""` until set.
    pub waypoint_type: String,
    /// `waypointDescription`.
    pub description: String,
    /// `waypointBehaviour` (`"UNCHANGED"` when the waypoint does not override the group's).
    pub behaviour: String,
    /// `waypointCombatMode`.
    pub combat_mode: String,
    /// `waypointFormation`.
    pub formation: String,
    /// `waypointSpeed`.
    pub speed: String,
    /// `waypointCompletionRadius`, in metres.
    pub completion_radius: f32,
    /// `waypointScript`: the script file the waypoint runs.
    pub script: String,
    /// `waypointStatements`: `[condition, statement]`.
    pub statements: [String; 2],
    /// `waypointTimeout`: `[min, mid, max]` seconds.
    pub timeout: [f32; 3],
    /// `waypointVisible` (the engine reports it as a number: 1 shown, 0 hidden).
    pub visible: bool,
    /// `waypointHousePosition`: which position of a building to enter.
    pub house_position: f32,
    /// `waypointLoiterRadius`, in metres.
    pub loiter_radius: f32,
    /// `waypointLoiterType`: `"CIRCLE"`, `"CIRCLE_L"`, ...
    pub loiter_type: String,
    /// `waypointForceBehaviour`: force the waypoint's behaviour even when not in combat.
    pub force_behaviour: bool,
    /// `showWaypoint` / `waypointShow`: `"AUTO"`, `"ALWAYS"`, `"NEVER"`.
    pub show: String,
}

impl Waypoint {
    /// A waypoint at `position` with the engine's defaults.
    pub fn new(position: DVec3) -> Self {
        Waypoint {
            position,
            radius: 0.0,
            name: String::new(),
            waypoint_type: String::new(),
            description: String::new(),
            behaviour: "UNCHANGED".to_owned(),
            combat_mode: "NO CHANGE".to_owned(),
            formation: "NO CHANGE".to_owned(),
            speed: "UNCHANGED".to_owned(),
            completion_radius: 0.0,
            script: String::new(),
            statements: [String::new(), String::new()],
            timeout: [0.0, 0.0, 0.0],
            visible: true,
            house_position: -1.0,
            loiter_radius: 20.0,
            loiter_type: "CIRCLE".to_owned(),
            force_behaviour: false,
            show: "AUTO".to_owned(),
        }
    }
}

impl World {
    /// The group's waypoints, first to last.
    pub fn waypoints(&self, group: GroupId) -> Option<&[Waypoint]> {
        Some(&self.group(group)?.waypoints)
    }

    /// One waypoint (`[group, index]`).
    pub fn waypoint(&self, group: GroupId, index: usize) -> Option<&Waypoint> {
        self.group(group)?.waypoints.get(index)
    }

    /// One waypoint for the `setWaypoint*` commands.
    pub fn waypoint_mut(&mut self, group: GroupId, index: usize) -> Option<&mut Waypoint> {
        self.group_mut(group)?.waypoints.get_mut(index)
    }

    /// `addWaypoint [center, radius, index, name]`: inserts at `index` (or appends when `index`
    /// is `None` or past the end) and returns the new waypoint's index.
    pub fn add_waypoint(
        &mut self,
        group: GroupId,
        index: Option<usize>,
        waypoint: Waypoint,
    ) -> Option<usize> {
        let g = self.group_mut(group)?;
        let index = index.filter(|&i| i < g.waypoints.len()).unwrap_or(g.waypoints.len());
        g.waypoints.insert(index, waypoint);
        Some(index)
    }

    /// `deleteWaypoint [group, index]`; the other waypoints re-index.
    pub fn delete_waypoint(&mut self, group: GroupId, index: usize) -> bool {
        let Some(g) = self.group_mut(group) else {
            return false;
        };
        if index >= g.waypoints.len() {
            return false;
        }
        g.waypoints.remove(index);
        true
    }

    /// `currentWaypoint`: the index the group moves to, `waypoints` count when done.
    pub fn current_waypoint(&self, group: GroupId) -> Option<usize> {
        Some(self.group(group)?.current_waypoint)
    }

    /// `setCurrentWaypoint`.
    pub fn set_current_waypoint(&mut self, group: GroupId, index: usize) -> bool {
        let Some(g) = self.group_mut(group) else {
            return false;
        };
        g.current_waypoint = index;
        true
    }

    /// `copyWaypoints`: `to` gets a copy of `from`'s waypoints; the current waypoint is kept.
    pub fn copy_waypoints(&mut self, from: GroupId, to: GroupId) -> bool {
        let Some(source) = self.group(from).map(|g| g.waypoints.clone()) else {
            return false;
        };
        let Some(target) = self.group_mut(to) else {
            return false;
        };
        target.waypoints = source;
        target.current_waypoint = 0;
        true
    }
}
