//! Event-handler commands: `addEventHandler` / `addMissionEventHandler` and the removals, plus
//! the dispatch that fires the handlers of the World's events.
//!
//! The stored handlers live in [`crate::handlers`]; this module is the script face of it. An
//! addition takes `[event, expression(, args)]` and returns the handler's id (a Number); a
//! handler expression is Code or a String statement, which is compiled here. Ids are per target
//! and event type and a removed id is re-used, as in the engine.
//!
//! Raising: [`dispatch_events`] drains the World's [`WorldEvent`] queue and runs the handlers
//! that event maps to — mission `"EntityCreated"` / `"EntityDeleted"` and the object's
//! `"Local"` — and the entity commands call the same dispatch as they run, so an event raised
//! by a script fires before the command returns. An embedder that raises events of its own (the
//! simulation, damage, weapons) calls it after the tick that produced them. Handler code runs
//! unscheduled with `_this` (the event's arguments), `_thisEvent` (the event type) and
//! `_thisEventHandler` (the handler's id); mission handlers added with a third element also see
//! it as `_thisArgs`.
//!
//! An object's own handlers are forgotten when it is deleted ([`crate::handlers`] drops them at
//! the delete flush), so the object `"Deleted"` event does not fire yet; the mission
//! `"EntityDeleted"` does. Group lifecycle events (`"GroupCreated"`, `"GroupDeleted"`) and the
//! events owned by systems that do not exist yet (damage, weapons, UI) are follow-ups.

use a3_sqf::vm::Ctx;
use a3_sqf::{Code, Namespace, Registry, Sym, Value, Vm};

use super::groups::GRP;
use super::{ARR, NOTHING, NUM, OBJ, STR, WorldHost, group_arg, object_arg, object_value};
use crate::handlers::Handler;
use crate::{GroupId, ObjectRef, World, WorldEvent};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // `addEventHandler` and `addMPEventHandler` share the object's table; MP events are told
    // apart by their names (`"MPKilled"`, ...), exactly as the original stores them.
    for name in ["addEventHandler", "addMPEventHandler"] {
        r.binary(name, OBJ, ARR, NUM.union(NOTHING), |ctx, t, a| {
            Ok(add_object(ctx, &t, &a))
        });
    }
    r.binary(
        "addEventHandler",
        GRP,
        ARR,
        NUM.union(NOTHING),
        |ctx, t, a| Ok(add_group(ctx, &t, &a)),
    );
    // `addMissionEventHandler [type, code(, args)]`.
    r.unary("addMissionEventHandler", ARR, NUM, |ctx, a| {
        Ok(add_mission(ctx, &a))
    });

    for name in ["removeEventHandler", "removeMPEventHandler"] {
        r.binary(name, OBJ, ARR, NOTHING, |ctx, t, a| {
            remove_object(ctx, &t, &a);
            Ok(Value::Nothing)
        });
    }
    r.binary("removeEventHandler", GRP, ARR, NOTHING, |ctx, t, a| {
        remove_group(ctx, &t, &a);
        Ok(Value::Nothing)
    });
    // `removeAllEventHandlers [target, type]`; the engine has no "remove every type" form.
    for name in ["removeAllEventHandlers", "removeAllMPEventHandlers"] {
        r.binary(name, OBJ, STR, NOTHING, |ctx, t, a| {
            if let (Some(target), Some(event_type)) = (object_arg(ctx.host.world(), &t), a.as_str())
            {
                ctx.host
                    .world_mut()
                    .handlers_mut()
                    .remove_all_object(target, event_type);
            }
            Ok(Value::Nothing)
        });
    }
    r.binary("removeAllEventHandlers", GRP, STR, NOTHING, |ctx, t, a| {
        if let (Some(target), Some(event_type)) = (group_arg(ctx.host.world(), &t), a.as_str()) {
            ctx.host
                .world_mut()
                .handlers_mut()
                .remove_all_group(target, event_type);
        }
        Ok(Value::Nothing)
    });

    // Mission handlers: `removeMissionEventHandler [type, id]` and
    // `removeAllMissionEventHandlers type`.
    r.unary("removeMissionEventHandler", ARR, NOTHING, |ctx, a| {
        if let Some((event_type, id)) = id_arg(&a) {
            ctx.host
                .world_mut()
                .handlers_mut()
                .remove_mission(&event_type, id);
        }
        Ok(Value::Nothing)
    });
    r.unary("removeAllMissionEventHandlers", STR, NOTHING, |ctx, a| {
        if let Some(event_type) = a.as_str() {
            ctx.host
                .world_mut()
                .handlers_mut()
                .remove_all_mission(event_type);
        }
        Ok(Value::Nothing)
    });

    // `getEventHandlerInfo [type, id]`, and `target getEventHandlerInfo [type, id]`.
    r.unary("getEventHandlerInfo", ARR, ARR, |ctx, a| {
        Ok(match id_arg(&a) {
            Some((event_type, id)) => {
                Value::array(ctx.host.world().handlers().mission_info(&event_type, id))
            }
            None => Value::array([]),
        })
    });
    r.binary("getEventHandlerInfo", OBJ, ARR, ARR, |ctx, t, a| {
        Ok(match (object_arg(ctx.host.world(), &t), id_arg(&a)) {
            (Some(target), Some((event_type, id))) => Value::array(
                ctx.host
                    .world()
                    .handlers()
                    .object_info(target, &event_type, id),
            ),
            _ => Value::array([]),
        })
    });
    r.binary("getEventHandlerInfo", GRP, ARR, ARR, |ctx, t, a| {
        Ok(match (group_arg(ctx.host.world(), &t), id_arg(&a)) {
            (Some(target), Some((event_type, id))) => Value::array(
                ctx.host
                    .world()
                    .handlers()
                    .group_info(target, &event_type, id),
            ),
            _ => Value::array([]),
        })
    });
}

/// The elements of an array argument, or empty.
fn items(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .map(|a| a.borrow().clone())
        .unwrap_or_default()
}

/// The handler expression of an addition: Code, or a String statement compiled here.
fn code_arg<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value) -> Option<Code> {
    match value {
        Value::Code(code) => Some(code.clone()),
        Value::String(text) => ctx.compile("", text).ok(),
        _ => None,
    }
}

/// `[event, expression(, args)]` as an event type and the handler to store.
fn added_arg<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value) -> Option<(String, Handler)> {
    let items = items(value);
    let event_type = items.first()?.as_str()?.to_owned();
    let code = code_arg(ctx, items.get(1)?)?;
    let args = items.get(2).filter(|a| !a.is_nil()).cloned();
    Some((event_type, Handler { code, args }))
}

/// `[event, id]` of a removal or a query, as an event type and a non-negative id.
fn id_arg(value: &Value) -> Option<(String, usize)> {
    let items = items(value);
    let event_type = items.first()?.as_str()?.to_owned();
    let id = items.get(1)?.as_number()?;
    (id >= 0.0).then_some((event_type, id as usize))
}

fn add_object<H: WorldHost>(ctx: &mut Ctx<'_, H>, target: &Value, args: &Value) -> Value {
    let Some(object) = object_arg(ctx.host.world(), target) else {
        return Value::Nothing;
    };
    let Some((event_type, handler)) = added_arg(ctx, args) else {
        return Value::Nothing;
    };
    let id = ctx
        .host
        .world_mut()
        .handlers_mut()
        .add_object(object, &event_type, handler);
    Value::Number(id as f32)
}

fn add_group<H: WorldHost>(ctx: &mut Ctx<'_, H>, target: &Value, args: &Value) -> Value {
    let Some(group) = group_arg(ctx.host.world(), target) else {
        return Value::Nothing;
    };
    let Some((event_type, handler)) = added_arg(ctx, args) else {
        return Value::Nothing;
    };
    let id = ctx
        .host
        .world_mut()
        .handlers_mut()
        .add_group(group, &event_type, handler);
    Value::Number(id as f32)
}

fn add_mission<H: WorldHost>(ctx: &mut Ctx<'_, H>, args: &Value) -> Value {
    let Some((event_type, handler)) = added_arg(ctx, args) else {
        return Value::Nothing;
    };
    let id = ctx
        .host
        .world_mut()
        .handlers_mut()
        .add_mission(&event_type, handler);
    Value::Number(id as f32)
}

fn remove_object<H: WorldHost>(ctx: &mut Ctx<'_, H>, target: &Value, args: &Value) {
    if let (Some(target), Some((event_type, id))) =
        (object_arg(ctx.host.world(), target), id_arg(args))
    {
        ctx.host
            .world_mut()
            .handlers_mut()
            .remove_object(target, &event_type, id);
    }
}

fn remove_group<H: WorldHost>(ctx: &mut Ctx<'_, H>, target: &Value, args: &Value) {
    if let (Some(target), Some((event_type, id))) =
        (group_arg(ctx.host.world(), target), id_arg(args))
    {
        ctx.host
            .world_mut()
            .handlers_mut()
            .remove_group(target, &event_type, id);
    }
}

/// One handler about to run, with the magic variables its code sees.
struct Call {
    code: Code,
    id: usize,
    event: String,
    this: Value,
    args: Option<Value>,
}

impl Call {
    /// `_thisEvent` and `_thisEventHandler`, plus `_thisArgs` when the handler has them.
    fn locals(&self) -> Vec<(Sym, Value)> {
        let mut locals = vec![
            (Sym::new("_thisEvent"), Value::from(self.event.clone())),
            (Sym::new("_thisEventHandler"), Value::Number(self.id as f32)),
        ];
        if let Some(args) = &self.args {
            locals.push((Sym::new("_thisArgs"), args.clone()));
        }
        locals
    }
}

/// The calls an event type's handlers on one list amount to, in id order.
fn calls_of(list: Option<&[Option<Handler>]>, event_type: &str, this: Value) -> Vec<Call> {
    list.into_iter()
        .flatten()
        .flatten()
        .enumerate()
        .map(|(id, handler)| Call {
            code: handler.code.clone(),
            id,
            event: event_type.to_owned(),
            this: this.clone(),
            args: handler.args.clone(),
        })
        .collect()
}

/// Runs an Object's handlers for one event type, `_this` being `args`.
pub fn raise_object_event<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    target: ObjectRef,
    event_type: &str,
    args: Value,
) {
    let calls = calls_of(
        ctx.host.world().handlers().object_list(target, event_type),
        event_type,
        args,
    );
    run_in_ctx(ctx, calls);
}

/// Runs a Group's handlers for one event type.
pub fn raise_group_event<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    target: GroupId,
    event_type: &str,
    args: Value,
) {
    let calls = calls_of(
        ctx.host.world().handlers().group_list(target, event_type),
        event_type,
        args,
    );
    run_in_ctx(ctx, calls);
}

/// Runs the mission's handlers for one event type.
pub fn raise_mission_event<H: WorldHost>(ctx: &mut Ctx<'_, H>, event_type: &str, args: Value) {
    let calls = calls_of(
        ctx.host.world().handlers().mission_list(event_type),
        event_type,
        args,
    );
    run_in_ctx(ctx, calls);
}

/// The handlers the World's queued events amount to, taking the queue.
fn take_calls(world: &mut World) -> Vec<Call> {
    let mut calls = Vec::new();
    for event in world.drain_events() {
        match event {
            WorldEvent::EntityCreated(entity) => {
                let this = Value::array([object_value(world, ObjectRef::Entity(entity))]);
                push_mission(world, "EntityCreated", this, &mut calls);
            }
            WorldEvent::EntityDeleted { entity, .. } => {
                // The Entity is gone, so its value is the null object of that id.
                let this = Value::array([object_value(world, ObjectRef::Entity(entity))]);
                push_mission(world, "EntityDeleted", this, &mut calls);
            }
            WorldEvent::LocalityChanged { entity, local } => {
                let target = ObjectRef::Entity(entity);
                let this = Value::array([object_value(world, target), Value::Bool(local)]);
                calls.extend(calls_of(
                    world.handlers().object_list(target, "Local"),
                    "Local",
                    this,
                ));
            }
            WorldEvent::WaypointCompleted { group, index } => {
                // `[group, waypointIndex]`, the index one-based as scripts address waypoints
                // (the engine's implicit start waypoint is 0).
                let this =
                    Value::array([super::group_value(group), Value::Number((index + 1) as f32)]);
                calls.extend(calls_of(
                    world.handlers().group_list(group, "WaypointComplete"),
                    "WaypointComplete",
                    this,
                ));
            }
        }
    }
    calls
}

fn push_mission(world: &World, event_type: &str, this: Value, calls: &mut Vec<Call>) {
    calls.extend(calls_of(
        world.handlers().mission_list(event_type),
        event_type,
        this,
    ));
}

/// Runs the handlers of every event the World queued since the last dispatch, in order. An
/// embedder calls this after the tick (or the command) that raised them; the entity commands
/// call it themselves. The queue is consumed even when nothing handles the events.
pub fn dispatch_events<H: WorldHost>(vm: &mut Vm<H>) {
    for call in take_calls(vm.host.world_mut()) {
        let mut locals = call.locals();
        locals.push((Sym::THIS, call.this.clone()));
        // A handler's own error is reported by the VM and does not stop the rest.
        let _ = vm.call_with_locals(&call.code, Namespace::Mission, locals);
    }
}

/// [`dispatch_events`] while a command holds a [`Ctx`].
pub(crate) fn dispatch_events_in<H: WorldHost>(ctx: &mut Ctx<'_, H>) {
    let calls = take_calls(ctx.host.world_mut());
    run_in_ctx(ctx, calls);
}

fn run_in_ctx<H: WorldHost>(ctx: &mut Ctx<'_, H>, calls: Vec<Call>) {
    for call in calls {
        let locals = call.locals();
        let _ = ctx.call_unscheduled_with_locals(&call.code, Some(call.this), locals);
    }
}
