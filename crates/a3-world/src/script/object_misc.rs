//! Object, environment and global-script commands that do not belong to the unit, vehicle or
//! container families: `direction`, `setOvercast`, the simple-object pair, and the commands whose
//! contract we can honour while the subsystem behind them does not exist yet (`enableStamina`,
//! `enableTeamSwitch`, `enableAttack`, `createDiaryRecord`). From the decompiled handlers (RVAs
//! from `docs/re/sqf-commands.tsv`) and the offline wiki for the syntax; the stubs are recorded
//! as `stub` in `docs/fidelity/sqf-verified.tsv`.

use a3_sqf::{Handle, HandleKind, Registry, Type, TypeSet, Value};

use super::groups::{GRP, group_arg};
use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, WorldHost, null_object, object_arg, object_value, script_position,
};
use crate::{Create, EntityClass, EntityId, ObjectRef, World};

/// `createDiaryRecord`'s return value.
const DIARY: TypeSet = TypeSet::of(Type::DiaryRecord);

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn items(v: &Value) -> Vec<Value> {
    v.as_array().map(|a| a.borrow().clone()).unwrap_or_default()
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
    register_object(r);
    register_simple(r);
    register_globals(r);
}

/// `direction` and `setOvercast`.
fn register_object<H: WorldHost>(r: &mut Registry<H>) {
    // 0x52b330 (`direction`): the object's heading in degrees, 0..360. The LOCATION form
    // (0xd05170) needs values we do not have yet.
    r.unary("direction", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Number(
            entity(w, &a, EntityClass::Entity)
                .map_or(0.0, |id| w.entity(id).expect("exists").heading() as f32),
        ))
    });
    // 0x5545b0: `time setOvercast overcast` (`0 setOvercast 0.5` sets it at once). The
    // environment has no weather ramp yet, so the value is applied at once whatever `time` is.
    r.binary("setOvercast", NUM, NUM, NOTHING, |ctx, _, b| {
        let overcast = b.as_number().unwrap_or(0.0);
        ctx.host
            .world_mut()
            .environment_mut()
            .set_overcast(overcast);
        Ok(Value::Nothing)
    });
}

/// `isSimpleObject` and `createSimpleObject`.
fn register_simple<H: WorldHost>(r: &mut Registry<H>) {
    // 0x518720.
    r.unary("isSimpleObject", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(entity(w, &a, EntityClass::Entity).is_some_and(
            |id| w.object_state(id).is_some_and(|s| s.simple),
        )))
    });
    // 0x5262d0: `createSimpleObject [shapeName, positionWorld, local]`. `shapeName` is a
    // CfgVehicles class or a model path (oracle: `Box_NATO_Ammo_F` and
    // `a3\structures_f\civ\market\crateswooden_f.p3d` both create one, `Land_Barrel_empty` does
    // not). The engine's simple objects are local-only render objects with no simulation; we
    // create an Entity that is local, does not simulate and is marked simple (a stand-in for the
    // render path, not for the contract: `isSimpleObject` reports it, `deleteVehicle` removes it).
    r.unary("createSimpleObject", ARR, OBJ, |ctx, a| {
        let args = items(&a);
        let (Some(name), Some(position)) = (
            args.first().and_then(Value::as_str),
            args.get(1).and_then(script_position),
        ) else {
            return Ok(null_object());
        };
        let ty = match ctx.host.types().get(name) {
            Ok(ty) => ty,
            Err(_) => {
                let Some(class) = ctx.host.types().class_of_model(name) else {
                    return Ok(null_object());
                };
                match ctx.host.types().get(&class) {
                    Ok(ty) => ty,
                    Err(_) => return Ok(null_object()),
                }
            }
        };
        let Ok(id) = ctx
            .host
            .world_mut()
            .create(Create::new(ty, position).local_only())
        else {
            return Ok(null_object());
        };
        if let Some(e) = ctx.host.world_mut().entity_mut(id) {
            e.simulation_enabled = false;
        }
        ctx.host.world_mut().object_state_mut(id).simple = true;
        Ok(object_value(ctx.host.world(), ObjectRef::Entity(id)))
    });
}

/// The commands whose subsystem does not exist yet: each keeps the state the engine keeps and
/// records itself as a `stub` in `docs/fidelity/sqf-verified.tsv`. (`fadeMusic`, once of this
/// family, now lives in `audio_cmds`, against the host's audio engine.)
fn register_globals<H: WorldHost>(r: &mut Registry<H>) {
    // 0x55f850: there is no team-switch UI, so the flag is only kept _(stub)_.
    r.unary("enableTeamSwitch", BOOL, NOTHING, |ctx, a| {
        ctx.host
            .world_mut()
            .set_team_switch(a.as_bool().unwrap_or(false));
        Ok(Value::Nothing)
    });
    // 0x896820: no stamina model exists, so the flag is only kept _(stub)_.
    r.binary("enableStamina", OBJ, BOOL, NOTHING, |ctx, a, b| {
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Man) {
            let enabled = b.as_bool().unwrap_or(false);
            ctx.host.world_mut().object_state_mut(id).stamina = enabled;
        }
        Ok(Value::Nothing)
    });
    // 0x1908e0: the AI does not read the flag yet, so it is kept per unit (a group sets its
    // units') _(stub)_.
    r.binary(
        "enableAttack",
        OBJ.union(GRP),
        BOOL,
        NOTHING,
        |ctx, a, b| {
            let enabled = b.as_bool().unwrap_or(false);
            let w = ctx.host.world();
            let units: Vec<EntityId> = if let Some(id) =
                entity(w, &a, EntityClass::EntityAi).filter(|&id| w.group_of(id).is_none())
            {
                vec![id]
            } else if let Some(group) = group_arg(w, &a) {
                w.group(group)
                    .map(|g| g.units().to_vec())
                    .unwrap_or_default()
            } else if let Some(id) = entity(w, &a, EntityClass::EntityAi) {
                vec![id]
            } else {
                Vec::new()
            };
            for unit in units {
                ctx.host.world_mut().object_state_mut(unit).attack = enabled;
            }
            Ok(Value::Nothing)
        },
    );
    // 0x847190: `uniformContainer unit` returns the uniform's container Object. Container
    // objects do not exist in our inventory (a container is a value inside `Gear`), so this gives
    // objNull _(stub)_.
    r.unary("uniformContainer", OBJ, OBJ, |_, _| Ok(null_object()));
    // 0x8471c0: `unitBackpack unit` returns the backpack's container Object, with the same
    // limitation as `uniformContainer`: a container is a value inside `Gear`, not an Object, so
    // this gives objNull _(stub)_.
    r.unary("unitBackpack", OBJ, OBJ, |_, _| Ok(null_object()));
    // 0xdfd850: `unit createDiaryRecord [subject, text]`. The record is kept; there is no diary
    // UI and no record handle table yet, so the returned handle is null _(stub)_.
    r.binary("createDiaryRecord", OBJ, ARR, DIARY, |ctx, a, b| {
        let args = items(&b);
        let subject = args.first().map(text).unwrap_or_default();
        let body = args.get(1).map(text).unwrap_or_default();
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::EntityAi) {
            ctx.host
                .world_mut()
                .object_state_mut(id)
                .diary
                .push((subject, body));
        }
        Ok(Value::Handle(Handle::null(HandleKind::DiaryRecord)))
    });
    // 0xdfea70: `unit createDiarySubject [[subject, display name(, picture)]]`. The subject is
    // kept and its index returned; there is no diary UI yet _(stub)_.
    r.binary("createDiarySubject", OBJ, ARR, NUM, |ctx, a, b| {
        let args = items(&b);
        let subject = args.first().map(text).unwrap_or_default();
        let display = args.get(1).map(text).unwrap_or_default();
        let Some(id) = entity(ctx.host.world(), &a, EntityClass::EntityAi) else {
            return Ok(Value::Number(-1.0));
        };
        let state = ctx.host.world_mut().object_state_mut(id);
        state.diary_subjects.push((subject, display));
        Ok(Value::Number(state.diary_subjects.len() as f32 - 1.0))
    });
}
