//! `createVehicle`, `createVehicleLocal`, `deleteVehicle`.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, SqfError, Value};

use super::{
    ARR, NOTHING, OBJ, STR, WorldHost, null_object, object_arg, object_value, script_position,
};
use crate::{Create, ObjectRef};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AG EG. `type createVehicle position`: the old syntax, position above the terrain.
    r.binary("createVehicle", STR, ARR, OBJ, |ctx, t, p| {
        Ok(create(ctx, &t, &p, &Value::Nil, false))
    });
    // AG EG. `createVehicle [type, position, markers, placement, special]`.
    r.unary("createVehicle", ARR, OBJ, |ctx, a| {
        create_array(ctx, &a, false)
    });
    // AG EL.
    r.binary("createVehicleLocal", STR, ARR, OBJ, |ctx, t, p| {
        Ok(create(ctx, &t, &p, &Value::Nil, true))
    });
    r.unary("createVehicleLocal", ARR, OBJ, |ctx, a| {
        create_array(ctx, &a, true)
    });
    // AG EG. Static objects cannot be deleted (the original refuses "primary" objects).
    r.unary("deleteVehicle", OBJ, NOTHING, |ctx, a| {
        if let Some(ObjectRef::Entity(id)) = object_arg(ctx.host.world(), &a) {
            ctx.host.world_mut().delete(id);
        }
        Ok(Value::Nothing)
    });
    r.unary("deleteVehicle", ARR, NOTHING, |ctx, a| {
        let items = a.as_array().map(|a| a.borrow().clone()).unwrap_or_default();
        for item in items {
            if let Some(ObjectRef::Entity(id)) = object_arg(ctx.host.world(), &item) {
                ctx.host.world_mut().delete(id);
            }
        }
        Ok(Value::Nothing)
    });
}

fn create_array<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    args: &Value,
    local: bool,
) -> Result<Value, SqfError> {
    let items = args
        .as_array()
        .map(|a| a.borrow().clone())
        .unwrap_or_default();
    let get = |i: usize| items.get(i).cloned().unwrap_or(Value::Nil);
    Ok(create(ctx, &get(0), &get(1), &get(4), local))
}

/// Creates an Entity of type `type_name` at the script position `position` (height above the
/// terrain). `special` `"CAN_COLLIDE"` skips the free-position search; `"NONE"` and `"FLY"`
/// should search for a free spot and lift aircraft (#123 collision, #126 air) and currently
/// place at the position too. Unknown and abstract types give `objNull`, as in the original.
fn create<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    type_name: &Value,
    position: &Value,
    _special: &Value,
    local_only: bool,
) -> Value {
    let (Some(name), Some(pos)) = (type_name.as_str(), script_position(position)) else {
        return null_object();
    };
    let Ok(ty) = ctx.host.types().get(name) else {
        ctx.host
            .diag_log(&format!("Cannot create non-ai vehicle {name}"));
        return null_object();
    };
    let mut request = Create::new(ty, pos).on_surface();
    if local_only {
        request = request.local_only();
    }
    match ctx.host.world_mut().create(request) {
        Ok(id) => object_value(ctx.host.world(), ObjectRef::Entity(id)),
        Err(e) => {
            ctx.host.diag_log(&e.to_string());
            null_object()
        }
    }
}
