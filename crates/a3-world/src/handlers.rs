//! Event handlers: the tables `addEventHandler`, `addMissionEventHandler` and their removals
//! drive.
//!
//! An event handler is a piece of script code stored against a target — an Object, a Group, or
//! the mission itself — under an event type (`"Killed"`, `"EachFrame"`, ...). When the engine
//! raises the event it runs the code unscheduled with `_this` (the event's arguments),
//! `_thisEvent` (the event type) and `_thisEventHandler` (the handler's id) set; mission
//! handlers also get `_thisArgs` (the optional third element of `addMissionEventHandler`).
//!
//! Ids are per target and event type, starting at 0, and a removed id is re-used by the next
//! addition — `getEventHandlerInfo [type, id]` reports `[exists, isLast, total]`.
//!
//! Raising is the job of the system that owns the event. What this module provides is the
//! storage and the [`Handlers::take`] accessor the raiser uses to walk a target's handlers for
//! one event type.

use std::collections::HashMap;

use a3_sqf::{Code, Value};

use crate::{GroupId, ObjectRef, World};

/// One stored event handler.
#[derive(Debug, Clone)]
pub struct Handler {
    /// The expression the event runs.
    pub code: Code,
    /// `addMissionEventHandler`'s optional third element, seen as `_thisArgs`.
    pub args: Option<Value>,
}

/// One `(target, event type)` list: entries by id, `None` where an id was removed.
#[derive(Debug, Default, Clone)]
struct HandlerList {
    entries: Vec<Option<Handler>>,
}

impl HandlerList {
    fn add(&mut self, handler: Handler) -> usize {
        match self.entries.iter().position(Option::is_none) {
            Some(id) => {
                self.entries[id] = Some(handler);
                id
            }
            None => {
                self.entries.push(Some(handler));
                self.entries.len() - 1
            }
        }
    }

    fn remove(&mut self, id: usize) -> bool {
        match self.entries.get_mut(id) {
            Some(slot @ Some(_)) => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    fn total(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// `[exists, isLast, total]`, or `[]` when there is no such list.
    fn info(&self, id: usize) -> Vec<Value> {
        let exists = self.entries.get(id).is_some_and(Option::is_some);
        let last = self
            .entries
            .iter()
            .rposition(Option::is_some)
            .is_some_and(|last| last == id);
        vec![
            Value::Bool(exists),
            Value::Bool(exists && last),
            Value::Number(self.total() as f32),
        ]
    }
}

/// Every event handler of the session.
#[derive(Debug, Default, Clone)]
pub struct Handlers {
    objects: HashMap<ObjectRef, HashMap<String, HandlerList>>,
    groups: HashMap<GroupId, HashMap<String, HandlerList>>,
    mission: HashMap<String, HandlerList>,
}

impl Handlers {
    /// `addEventHandler` on an Object; returns the handler id.
    pub fn add_object(&mut self, target: ObjectRef, event_type: &str, handler: Handler) -> usize {
        Self::add_in(&mut self.objects, target, event_type, handler)
    }

    /// `addEventHandler` on a Group.
    pub fn add_group(&mut self, target: GroupId, event_type: &str, handler: Handler) -> usize {
        Self::add_in(&mut self.groups, target, event_type, handler)
    }

    /// `addMissionEventHandler`.
    pub fn add_mission(&mut self, event_type: &str, handler: Handler) -> usize {
        self.mission
            .entry(event_type.to_owned())
            .or_default()
            .add(handler)
    }

    /// `removeEventHandler` on an Object or Group.
    pub fn remove_object(&mut self, target: ObjectRef, event_type: &str, id: usize) -> bool {
        Self::remove_in(&mut self.objects, &target, event_type, id)
    }

    pub fn remove_group(&mut self, target: GroupId, event_type: &str, id: usize) -> bool {
        Self::remove_in(&mut self.groups, &target, event_type, id)
    }

    /// `removeMissionEventHandler`.
    pub fn remove_mission(&mut self, event_type: &str, id: usize) -> bool {
        self.mission
            .get_mut(event_type)
            .is_some_and(|list| list.remove(id))
    }

    /// `removeAllEventHandlers`: one event type, or every handler when `event_type` is empty.
    pub fn remove_all_object(&mut self, target: ObjectRef, event_type: &str) {
        Self::remove_all_in(&mut self.objects, &target, event_type);
    }

    pub fn remove_all_group(&mut self, target: GroupId, event_type: &str) {
        Self::remove_all_in(&mut self.groups, &target, event_type);
    }

    /// `removeAllMissionEventHandlers`: the engine requires the type (no "all" form).
    pub fn remove_all_mission(&mut self, event_type: &str) {
        self.mission.remove(event_type);
    }

    /// The Object's handlers for one event type, by id; empty when neither exists.
    pub fn object_list(&self, target: ObjectRef, event_type: &str) -> Option<&[Option<Handler>]> {
        self.objects
            .get(&target)
            .and_then(|types| types.get(event_type))
            .map(|list| list.entries.as_slice())
    }

    pub fn group_list(&self, target: GroupId, event_type: &str) -> Option<&[Option<Handler>]> {
        self.groups
            .get(&target)
            .and_then(|types| types.get(event_type))
            .map(|list| list.entries.as_slice())
    }

    /// The mission's handlers for one event type, by id.
    pub fn mission_list(&self, event_type: &str) -> Option<&[Option<Handler>]> {
        self.mission.get(event_type).map(|list| list.entries.as_slice())
    }

    /// `getEventHandlerInfo [type, id]` for an Object.
    pub fn object_info(&self, target: ObjectRef, event_type: &str, id: usize) -> Vec<Value> {
        Self::info_of(&self.objects, &target, event_type, id)
    }

    pub fn group_info(&self, target: GroupId, event_type: &str, id: usize) -> Vec<Value> {
        Self::info_of(&self.groups, &target, event_type, id)
    }

    /// `getEventHandlerInfo [type, id]` without an object: the mission handlers.
    pub fn mission_info(&self, event_type: &str, id: usize) -> Vec<Value> {
        self.mission
            .get(event_type)
            .map_or_else(Vec::new, |list| list.info(id))
    }

    /// Forgets every handler of an Object that left the World.
    pub fn forget_object(&mut self, target: ObjectRef) {
        self.objects.remove(&target);
    }

    /// Forgets every handler of a deleted Group.
    pub fn forget_group(&mut self, target: GroupId) {
        self.groups.remove(&target);
    }

    pub fn object_event_types(&self, target: ObjectRef) -> Vec<&str> {
        self.objects
            .get(&target)
            .map(|types| types.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn mission_event_types(&self) -> Vec<&str> {
        self.mission.keys().map(String::as_str).collect()
    }

    fn add_in<K: std::hash::Hash + Eq>(
        map: &mut HashMap<K, HashMap<String, HandlerList>>,
        target: K,
        event_type: &str,
        handler: Handler,
    ) -> usize {
        map.entry(target)
            .or_default()
            .entry(event_type.to_owned())
            .or_default()
            .add(handler)
    }

    fn remove_in<K: std::hash::Hash + Eq>(
        map: &mut HashMap<K, HashMap<String, HandlerList>>,
        target: &K,
        event_type: &str,
        id: usize,
    ) -> bool {
        map.get_mut(target)
            .and_then(|types| types.get_mut(event_type))
            .is_some_and(|list| list.remove(id))
    }

    fn remove_all_in<K: std::hash::Hash + Eq>(
        map: &mut HashMap<K, HashMap<String, HandlerList>>,
        target: &K,
        event_type: &str,
    ) {
        let Some(types) = map.get_mut(target) else {
            return;
        };
        if event_type.is_empty() {
            types.clear();
        } else {
            types.remove(event_type);
        }
    }

    fn info_of<K: std::hash::Hash + Eq>(
        map: &HashMap<K, HashMap<String, HandlerList>>,
        target: &K,
        event_type: &str,
        id: usize,
    ) -> Vec<Value> {
        map.get(target)
            .and_then(|types| types.get(event_type))
            .map_or_else(Vec::new, |list| list.info(id))
    }
}

impl World {
    pub fn handlers(&self) -> &Handlers {
        &self.handlers
    }

    pub fn handlers_mut(&mut self) -> &mut Handlers {
        &mut self.handlers
    }
}
