//! Animation commands: the move state machine a script drives — `playMove`, `playMoveNow`,
//! `switchMove`, `playAction`, `playActionNow` — and `animationState`
//! (`docs/re/sim-man-anim-state.md` §6).
//!
//! The five mutating commands are argument local / effect global (`AL EG`): they do nothing on a
//! remote Man, whose move state his owner drives, and a local one's effect will also queue a
//! network message once the network layer exists (#131). `animationState` is argument global
//! (`AG`): it reads the state this machine has for the Man, local or not.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{ARR, NOTHING, OBJ, STR, WorldHost, object_arg};
use crate::{ClassState, EntityId, ObjectRef};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AG. The name of the move he plays, lower case; "" for anything that is no Man of this
    // World, or a Man whose first step has not run (no move yet).
    r.unary("animationState", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let state = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.animation_state(id),
            _ => String::new(),
        };
        Ok(Value::string(state))
    });

    // The five below are AL EG.

    // `playMove`: queue the move behind everything he was asked for.
    r.binary("playMove", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = b.as_str().unwrap_or_default().to_owned();
        if let Some(id) = local_man(ctx, &a) {
            ctx.host.world_mut().play_move(id, &name);
        }
        Ok(Value::Nothing)
    });
    // `playMoveNow`: drop the queue and arm the move at once.
    r.binary("playMoveNow", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = b.as_str().unwrap_or_default().to_owned();
        if let Some(id) = local_man(ctx, &a) {
            ctx.host.world_mut().play_move_now(id, &name);
        }
        Ok(Value::Nothing)
    });
    // `switchMove`: reset him to the move on the spot; an unknown name (the empty string
    // included) resets him to the default state of his current action map.
    r.binary("switchMove", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = b.as_str().unwrap_or_default().to_owned();
        switch_move(ctx, &a, &name, 0.0, 1.0);
        Ok(Value::Nothing)
    });
    // `switchMove [name, time, blendFactor, resetAim]`: the array form, with unset entries at
    // their defaults. `time` is the cycle phase the move starts at (`0.0..=1.0`), and the blend
    // ramps from `blendFactor` up to all of him. `resetAim` is accepted and ignored: nothing
    // here aims.
    r.binary("switchMove", OBJ, ARR, NOTHING, |ctx, a, b| {
        let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let name = items
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let time = items
            .get(1)
            .and_then(Value::as_number)
            .map(f64::from)
            .unwrap_or(0.0);
        let blend = items
            .get(2)
            .and_then(Value::as_number)
            .map(f64::from)
            .unwrap_or(1.0);
        switch_move(ctx, &a, &name, time, blend);
        Ok(Value::Nothing)
    });
    // `playAction`: queue the move the action asks for in the move he plays, through that move's
    // action map with the move names after it.
    r.binary("playAction", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = b.as_str().unwrap_or_default().to_owned();
        if let Some(id) = local_man(ctx, &a) {
            ctx.host.world_mut().play_action(id, &name);
        }
        Ok(Value::Nothing)
    });
    // `playActionNow`: as above, arming it at once.
    r.binary("playActionNow", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = b.as_str().unwrap_or_default().to_owned();
        if let Some(id) = local_man(ctx, &a) {
            ctx.host.world_mut().play_action_now(id, &name);
        }
        Ok(Value::Nothing)
    });
}

/// The Entity a move command applies to: a Man of this World this machine owns — argument local
/// (`AL`), a remote Man's move state is his owner's to drive. `None` for anything else, so the
/// command does nothing.
fn local_man<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value) -> Option<EntityId> {
    let id = match object_arg(ctx.host.world(), value)? {
        ObjectRef::Entity(id) => id,
        ObjectRef::Static(_) => return None,
    };
    let e = ctx.host.world().entity(id)?;
    (e.is_local() && matches!(e.class_state(), ClassState::Man(_))).then_some(id)
}

/// [`World::switch_move_at`] behind the two `switchMove` overloads, on a local Man.
fn switch_move<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    object: &Value,
    name: &str,
    time: f64,
    blend_factor: f64,
) {
    if let Some(id) = local_man(ctx, object) {
        ctx.host
            .world_mut()
            .switch_move_at(id, name, time, blend_factor);
    }
}
