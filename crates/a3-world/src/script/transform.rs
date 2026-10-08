//! Position, direction, orientation vectors, velocity and attachments.
//!
//! Height conventions: ASL (and World, ASLW) is the world `y`; ATL and the default `getPos`/
//! `setPos` (AGL) are height above the terrain surface. The original's AGL/AGLS also count water
//! and roadway surfaces; that needs #123 (collision) and the sea level model.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use glam::{DMat3, DQuat, DVec3};

use super::{
    ARR, NOTHING, NUM, OBJ, WorldHost, numbers, object_arg, object_value, position_value,
    script_position, to_world, vector_value,
};
use crate::{EntityId, ObjectRef, World};

#[derive(Clone, Copy)]
enum Height {
    /// Above the terrain surface (`getPos`, `getPosATL`, AGL).
    Terrain,
    /// Above sea level (`getPosASL`, `getPosWorld`, `getPosASLW`).
    Sea,
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    for (name, height, visual) in [
        ("getPos", Height::Terrain, false),
        ("position", Height::Terrain, false),
        ("getPosATL", Height::Terrain, false),
        ("getPosASL", Height::Sea, false),
        ("getPosASLW", Height::Sea, false),
        ("getPosWorld", Height::Sea, false),
        ("getPosVisual", Height::Terrain, true),
        ("visiblePosition", Height::Terrain, true),
        ("getPosATLVisual", Height::Terrain, true),
        ("getPosASLVisual", Height::Sea, true),
        ("visiblePositionASL", Height::Sea, true),
        ("getPosWorldVisual", Height::Sea, true),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, a3_sqf::SqfError> =
            match (height, visual) {
                (Height::Terrain, false) => {
                    |ctx, a| Ok(get_pos(ctx.host.world(), &a, Height::Terrain, false))
                }
                (Height::Sea, false) => {
                    |ctx, a| Ok(get_pos(ctx.host.world(), &a, Height::Sea, false))
                }
                (Height::Terrain, true) => {
                    |ctx, a| Ok(get_pos(ctx.host.world(), &a, Height::Terrain, true))
                }
                (Height::Sea, true) => {
                    |ctx, a| Ok(get_pos(ctx.host.world(), &a, Height::Sea, true))
                }
            };
        r.unary(name, OBJ, ARR, f);
    }
    // AG EG.
    for (name, height) in [
        ("setPos", Height::Terrain),
        ("setPosATL", Height::Terrain),
        ("setPosASL", Height::Sea),
        ("setPosASLW", Height::Sea),
        ("setPosWorld", Height::Sea),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, a3_sqf::SqfError> = match height {
            Height::Terrain => |ctx, a, b| {
                set_pos(ctx, &a, &b, Height::Terrain);
                Ok(Value::Nothing)
            },
            Height::Sea => |ctx, a, b| {
                set_pos(ctx, &a, &b, Height::Sea);
                Ok(Value::Nothing)
            },
        };
        r.binary(name, OBJ, ARR, NOTHING, f);
    }

    for name in ["getDir", "getDirVisual"] {
        r.unary(name, OBJ, NUM, |ctx, a| {
            let w = ctx.host.world();
            Ok(Value::Number(entity(w, &a).map_or(0.0, |id| {
                w.entity(id).expect("exists").heading() as f32
            })))
        });
    }
    // AL EG.
    r.binary("setDir", OBJ, NUM, NOTHING, |ctx, a, b| {
        if let Some(id) = entity_for_change(ctx, &a) {
            let e = ctx.host.world_mut().entity_mut(id).expect("exists");
            e.set_heading(f64::from(b.as_number().unwrap_or(0.0)));
        }
        Ok(Value::Nothing)
    });

    for (name, axis) in [
        ("vectorDir", DVec3::Z),
        ("vectorDirVisual", DVec3::Z),
        ("vectorUp", DVec3::Y),
        ("vectorUpVisual", DVec3::Y),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, a3_sqf::SqfError> = if axis == DVec3::Z {
            |ctx, a| Ok(axis_value(ctx.host.world(), &a, DVec3::Z))
        } else {
            |ctx, a| Ok(axis_value(ctx.host.world(), &a, DVec3::Y))
        };
        r.unary(name, OBJ, ARR, f);
    }
    // AL EG.
    r.binary("setVectorDir", OBJ, ARR, NOTHING, |ctx, a, b| {
        orient(ctx, &a, script_vector(&b), None);
        Ok(Value::Nothing)
    });
    r.binary("setVectorUp", OBJ, ARR, NOTHING, |ctx, a, b| {
        orient(ctx, &a, None, script_vector(&b));
        Ok(Value::Nothing)
    });
    r.binary("setVectorDirAndUp", OBJ, ARR, NOTHING, |ctx, a, b| {
        let pair = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let dir = pair.first().and_then(script_vector);
        let up = pair.get(1).and_then(script_vector);
        orient(ctx, &a, dir, up);
        Ok(Value::Nothing)
    });

    r.unary("velocity", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        Ok(vector_value(entity(w, &a).map_or(DVec3::ZERO, |id| {
            w.entity(id).expect("exists").velocity()
        })))
    });
    // AL EG.
    r.binary("setVelocity", OBJ, ARR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let (Some(id), Some(v)) = (entity(w, &a), script_vector(&b)) {
            let e = w.entity_mut(id).expect("exists");
            if e.is_local() {
                e.set_velocity(v);
            }
        }
        Ok(Value::Nothing)
    });
    // km/h along the heading (negative when moving backwards).
    r.unary("speed", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        let speed = entity(w, &a).map_or(0.0, |id| {
            let e = w.entity(id).expect("exists");
            let v = e.velocity();
            v.length()
                * 3.6
                * if v.dot(e.orientation() * DVec3::Z) < 0.0 {
                    -1.0
                } else {
                    1.0
                }
        });
        Ok(Value::Number(speed as f32))
    });

    // AG EG. `obj attachTo [target, offset]`; without an offset the current relative position
    // is kept. Memory points and bone following come with model animation.
    r.binary("attachTo", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let w = ctx.host.world_mut();
        let (Some(id), Some(target)) = (entity(w, &a), args.first().and_then(|t| entity(w, t)))
        else {
            return Ok(Value::Nothing);
        };
        let offset = match args.get(1).and_then(script_position) {
            Some(offset) => offset,
            None => {
                let (child, parent) = (
                    w.entity(id).expect("exists"),
                    w.entity(target).expect("exists"),
                );
                parent.orientation().inverse() * (child.position() - parent.position())
            }
        };
        let _ = w.attach(id, target, offset);
        Ok(Value::Nothing)
    });
    r.unary("detach", OBJ, NOTHING, |ctx, a| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity(w, &a) {
            w.detach(id);
        }
        Ok(Value::Nothing)
    });
    r.unary("attachedTo", OBJ, OBJ, |ctx, a| {
        let w = ctx.host.world();
        Ok(entity(w, &a)
            .and_then(|id| w.entity(id).expect("exists").attachment())
            .filter(|att| w.entity(att.to).is_some())
            .map_or_else(super::null_object, |att| {
                object_value(w, ObjectRef::Entity(att.to))
            }))
    });
    r.unary("attachedObjects", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        let ids = entity(w, &a)
            .map(|id| w.attached_to(id))
            .unwrap_or_default();
        Ok(Value::array(
            ids.into_iter()
                .map(|id| object_value(w, ObjectRef::Entity(id))),
        ))
    });
}

/// The Entity behind a value (Static objects are not Entities until promoted).
fn entity(world: &World, value: &Value) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

fn entity_for_change<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value) -> Option<EntityId> {
    match object_arg(ctx.host.world(), value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(key) => ctx
            .host
            .world_mut()
            .promote_static_with_model_type(key)
            .ok(),
    }
}

fn get_pos(world: &World, value: &Value, height: Height, visual: bool) -> Value {
    let p = match object_arg(world, value) {
        Some(ObjectRef::Entity(id)) => {
            let e = world.entity(id).expect("exists");
            if visual {
                e.render_position()
            } else {
                e.position()
            }
        }
        Some(r @ ObjectRef::Static(_)) => world.object_position(r).expect("exists"),
        None => return position_value(DVec3::ZERO),
    };
    let y = match height {
        Height::Sea => p.y,
        Height::Terrain => p.y - world.surface_height(p.x, p.z),
    };
    position_value(DVec3::new(p.x, y, p.z))
}

fn set_pos<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, position: &Value, height: Height) {
    let Some(mut p) = script_position(position) else {
        return;
    };
    let Some(id) = entity_for_change(ctx, object) else {
        return;
    };
    let w = ctx.host.world_mut();
    if let Height::Terrain = height {
        p.y += w.surface_height(p.x, p.z);
    }
    w.entity_mut(id).expect("exists").set_position(p);
}

fn script_vector(value: &Value) -> Option<DVec3> {
    match numbers(value)?.as_slice() {
        [x, y, z, ..] => Some(to_world(*x, *y, *z)),
        _ => None,
    }
}

fn axis_value(world: &World, value: &Value, axis: DVec3) -> Value {
    let orientation = match object_arg(world, value) {
        Some(ObjectRef::Entity(id)) => world.entity(id).expect("exists").orientation(),
        _ => DQuat::IDENTITY,
    };
    vector_value(orientation * axis)
}

/// Sets the orientation from a forward and/or up vector, keeping the other axis as close to
/// the current one as possible.
fn orient<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    object: &Value,
    dir: Option<DVec3>,
    up: Option<DVec3>,
) {
    let Some(id) = entity_for_change(ctx, object) else {
        return;
    };
    let e = ctx.host.world_mut().entity_mut(id).expect("exists");
    let current = e.orientation();
    let dir = dir.unwrap_or(current * DVec3::Z).normalize_or_zero();
    let up = up.unwrap_or(current * DVec3::Y).normalize_or_zero();
    let right = up.cross(dir).normalize_or_zero();
    if dir == DVec3::ZERO || right == DVec3::ZERO {
        return;
    }
    let up = dir.cross(right);
    e.set_orientation(DQuat::from_mat3(&DMat3::from_cols(right, up, dir)));
}
