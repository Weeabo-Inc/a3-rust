//! Object animation commands: `animate`, `animateSource` and `animateDoor` set the phase of a
//! named selection, source or door, and `animationPhase`, `animationSourcePhase` and `doorPhase`
//! read it back. `action` is a unit's action: the Man move state machine plays it (as
//! `playAction` does), anything else keeps its name.
//!
//! From the decompiled handlers (RVAs from `docs/re/sqf-commands.tsv`) and the offline wiki for
//! the argument layout; `docs/re/sqf-object-animation.md` records the phases we verified against
//! the oracle.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{ARR, NOTHING, NUM, OBJ, STR, WorldHost, object_arg};
use crate::object_state::ObjectState;
use crate::{ClassState, EntityClass, EntityId, ObjectRef, World};

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn items(v: &Value) -> Vec<Value> {
    v.as_array().map(|a| a.borrow().clone()).unwrap_or_default()
}

/// The phase in the second element; 0 without one.
fn phase(args: &[Value]) -> f32 {
    args.get(1).and_then(Value::as_number).unwrap_or(0.0)
}

/// The Entity a value refers to, if its class derives from `class`.
fn entity(world: &World, value: &Value, class: EntityClass) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => world
            .entity(id)
            .filter(|e| e.class().is_kind_of(class))
            .map(|_| id),
        ObjectRef::Static(_) => None,
    }
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    register_phases(r);
    register_action(r);
}

/// `animate`, `animateSource`, `animateDoor` and the three phase getters.
fn register_phases<H: WorldHost>(r: &mut Registry<H>) {
    // 0x8b6310: `object animate [selection, phase(, speed)]`.
    r.binary("animate", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if let (Some(name), Some(id)) = (
            args.first().map(text),
            entity(ctx.host.world(), &a, EntityClass::Entity),
        ) {
            let value = phase(&args);
            let state = ctx.host.world_mut().object_state_mut(id);
            ObjectState::set_phase(&mut state.animations, &name, value);
        }
        Ok(Value::Nothing)
    });
    // 0x8b6700: `object animateSource [source, phase(, speed)]`.
    r.binary("animateSource", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if let (Some(name), Some(id)) = (
            args.first().map(text),
            entity(ctx.host.world(), &a, EntityClass::Entity),
        ) {
            let value = phase(&args);
            let state = ctx.host.world_mut().object_state_mut(id);
            ObjectState::set_phase(&mut state.sources, &name, value);
        }
        Ok(Value::Nothing)
    });
    // 0x54faf0: `object animateDoor [door, phase(, instant)]`.
    r.binary("animateDoor", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if let (Some(name), Some(id)) = (
            args.first().map(text),
            entity(ctx.host.world(), &a, EntityClass::Entity),
        ) {
            let value = phase(&args);
            let state = ctx.host.world_mut().object_state_mut(id);
            ObjectState::set_phase(&mut state.doors, &name, value);
        }
        Ok(Value::Nothing)
    });
    // 0x8b6720.
    r.binary("animationPhase", OBJ, STR, NUM, |ctx, a, b| {
        Ok(Value::Number(phase_of(ctx, &a, &b, 0)))
    });
    // 0x8b67b0.
    r.binary("animationSourcePhase", OBJ, STR, NUM, |ctx, a, b| {
        Ok(Value::Number(phase_of(ctx, &a, &b, 1)))
    });
    // 0x4a0570.
    r.binary("doorPhase", OBJ, STR, NUM, |ctx, a, b| {
        Ok(Value::Number(phase_of(ctx, &a, &b, 2)))
    });
}

/// The phase of `name`, read from the selection animations (0), the sources (1) or the doors (2).
fn phase_of<H: WorldHost>(ctx: &mut Ctx<'_, H>, a: &Value, b: &Value, list: usize) -> f32 {
    let name = text(b);
    let w = ctx.host.world();
    let Some(id) = entity(w, a, EntityClass::Entity) else {
        return 0.0;
    };
    let Some(state) = w.object_state(id) else {
        return 0.0;
    };
    match list {
        0 => ObjectState::phase_at(&state.animations, &name),
        1 => ObjectState::phase_at(&state.sources, &name),
        _ => ObjectState::phase_at(&state.doors, &name),
    }
}

/// `action`: `unit action ["Name", target, ...]` and the array form `action ["Name", unit, ...]`.
fn register_action<H: WorldHost>(r: &mut Registry<H>) {
    // 0x56e4d0.
    r.binary("action", OBJ, ARR, NOTHING, |ctx, a, b| {
        play_action(ctx, &a, &items(&b));
        Ok(Value::Nothing)
    });
    // 0x560420: the unit is the second element of the action array.
    r.unary("action", ARR, NOTHING, |ctx, a| {
        let args = items(&a);
        let unit = args.get(1).cloned().unwrap_or(Value::Nil);
        play_action(ctx, &unit, &args);
        Ok(Value::Nothing)
    });
}

/// Plays the action's first element on the unit: a Man's move state machine runs it (`playAction`),
/// anything else keeps its name.
fn play_action<H: WorldHost>(ctx: &mut Ctx<'_, H>, unit: &Value, args: &[Value]) {
    let Some(name) = args.first().map(text).filter(|n| !n.is_empty()) else {
        return;
    };
    let Some(id) = object_arg(ctx.host.world(), unit).and_then(|o| match o {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }) else {
        return;
    };
    let is_man = ctx
        .host
        .world()
        .entity(id)
        .is_some_and(|e| matches!(e.class_state(), ClassState::Man(_)));
    if is_man {
        ctx.host.world_mut().play_action(id, &name);
    } else {
        ctx.host.world_mut().object_state_mut(id).action = Some(name);
    }
}
