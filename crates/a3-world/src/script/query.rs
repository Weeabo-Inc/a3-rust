//! Object queries: `nearestObject(s)`, `nearObjects`, `nearestTerrainObjects`,
//! `allMissionObjects`.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use glam::DVec3;

use super::{ARR, NUM, OBJ, STR, WorldHost, null_object, object_value, position_or_object};
use crate::{EntityId, Near, ObjectRef, SimulationClass};

/// Every entity with its type name, for the `entities` filters. With `alive_only`, dead ones are
/// dropped here rather than by the caller, because `is_alive` needs the world borrow.
fn entity_candidates<H: WorldHost>(ctx: &Ctx<'_, H>, alive_only: bool) -> Vec<(EntityId, String)> {
    ctx.host
        .world()
        .entities()
        .filter(|e| !alive_only || e.is_alive())
        .map(|e| (e.id(), e.type_name().to_owned()))
        .collect()
}

/// `nearestObject [position, type]` searches this far (the original's 50 m).
const NEAREST_OBJECT_RADIUS: f64 = 50.0;

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // `entities type`: every alive and dead entity that is kind of `type`; `""` is every entity.
    r.unary("entities", STR, ARR, |ctx, a| {
        let ty = a.as_str().unwrap_or("").to_owned();
        let candidates = entity_candidates(ctx, false);
        let ids: Vec<EntityId> = candidates
            .into_iter()
            .filter(|(_, name)| ty.is_empty() || ctx.host.types().is_kind_of(name, &ty))
            .map(|(id, _)| id)
            .collect();
        let w = ctx.host.world();
        Ok(Value::array(
            ids.into_iter()
                .map(|id| object_value(w, ObjectRef::Entity(id)))
                .collect::<Vec<_>>(),
        ))
    });
    // `entities [typesInclude, typesExclude, includeCrews, excludeDead]`. We have no crew model
    // yet, so `includeCrews` changes nothing: nobody is ever inside a vehicle.
    r.unary("entities", ARR, ARR, |ctx, a| {
        let Value::Array(args) = &a else {
            return Err(a3_sqf::SqfError::type_error(&a, ARR));
        };
        let args = args.borrow().clone();
        let strings = |v: Option<&Value>| -> Vec<String> {
            match v {
                Some(Value::Array(items)) => items
                    .borrow()
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect(),
                _ => Vec::new(),
            }
        };
        let include = strings(args.first());
        let exclude = strings(args.get(1));
        let exclude_dead = matches!(args.get(3), Some(Value::Bool(true)));
        let candidates = entity_candidates(ctx, exclude_dead);
        let ids: Vec<EntityId> = candidates
            .into_iter()
            .filter(|(_, name)| {
                let types = ctx.host.types();
                (include.is_empty() || include.iter().any(|t| types.is_kind_of(name, t)))
                    && !exclude.iter().any(|t| types.is_kind_of(name, t))
            })
            .map(|(id, _)| id)
            .collect();
        let w = ctx.host.world();
        Ok(Value::array(
            ids.into_iter()
                .map(|id| object_value(w, ObjectRef::Entity(id)))
                .collect::<Vec<_>>(),
        ))
    });

    // `nearestObject [x, y, z]`, `nearestObject [position, type]`, `nearestObject [position, id]`.
    r.unary("nearestObject", ARR, OBJ, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let (position, selector) = if items.first().and_then(Value::as_number).is_some() {
            (a.clone(), Value::string(""))
        } else {
            (
                items.first().cloned().unwrap_or(Value::Nil),
                items.get(1).cloned().unwrap_or(Value::string("")),
            )
        };
        Ok(nearest_object(ctx, &position, &selector))
    });
    r.binary("nearestObject", ARR, NUM, OBJ, |ctx, a, b| {
        Ok(nearest_object(ctx, &a, &b))
    });
    r.binary("nearestObject", ARR, STR, OBJ, |ctx, a, b| {
        Ok(nearest_object(ctx, &a, &b))
    });
    r.binary("nearestObject", OBJ, STR, OBJ, |ctx, a, b| {
        Ok(nearest_object(ctx, &a, &b))
    });

    // `nearestObjects [position, [types], radius]`: Entities and Static objects, nearest first.
    r.unary("nearestObjects", ARR, ARR, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let types = string_list(items.get(1));
        let radius = items.get(2).and_then(Value::as_number).unwrap_or(0.0);
        Ok(near(ctx, items.first(), &types, radius, Near::All))
    });
    // `position nearObjects radius` / `position nearObjects [type, radius]`.
    for left in [ARR, OBJ] {
        r.binary("nearObjects", left, NUM, ARR, |ctx, a, b| {
            Ok(near(
                ctx,
                Some(&a),
                &[],
                b.as_number().unwrap_or(0.0),
                Near::All,
            ))
        });
        r.binary("nearObjects", left, ARR, ARR, |ctx, a, b| {
            let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
            let ty = items
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let radius = items.get(1).and_then(Value::as_number).unwrap_or(0.0);
            Ok(near(ctx, Some(&a), &[ty], radius, Near::All))
        });
    }
    // `nearestTerrainObjects [position, types, radius]`. Only the empty type list (every
    // object) is supported: the terrain object categories ("TREE", "HOUSE", ...) need the map
    // object kinds (#121 follow-up).
    r.unary("nearestTerrainObjects", ARR, ARR, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let types = string_list(items.get(1));
        if !types.is_empty() {
            return Ok(Value::array([]));
        }
        let radius = items.get(2).and_then(Value::as_number).unwrap_or(0.0);
        Ok(near(ctx, items.first(), &[], radius, Near::Statics))
    });
    // Every run-time Entity of a kind ("" for all), in creation order.
    r.unary("allMissionObjects", STR, ARR, |ctx, a| {
        let base = a.as_str().unwrap_or("").to_owned();
        let candidates: Vec<(crate::EntityId, String)> = ctx
            .host
            .world()
            .entities()
            .filter(|e| !e.is_deleted() && !e.network_id().is_some_and(|n| n.is_static()))
            .map(|e| (e.id(), e.type_name().to_owned()))
            .collect();
        let kept: Vec<crate::EntityId> = {
            let types = ctx.host.types();
            candidates
                .into_iter()
                .filter(|(_, name)| base.is_empty() || types.is_kind_of(name, &base))
                .map(|(id, _)| id)
                .collect()
        };
        let w = ctx.host.world();
        Ok(Value::array(
            kept.into_iter()
                .map(|id| object_value(w, ObjectRef::Entity(id))),
        ))
    });
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.borrow()
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `object` matches one of `types` by config inheritance (any object for no types).
/// Static objects without a config class match only the empty list.
fn matches_types<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: ObjectRef, types: &[String]) -> bool {
    let types: Vec<&String> = types.iter().filter(|t| !t.is_empty()).collect();
    if types.is_empty() {
        return true;
    }
    let name = match object {
        ObjectRef::Entity(id) => {
            let e = ctx.host.world().entity(id).expect("exists");
            if e.class() == SimulationClass::Plain {
                return false;
            }
            e.type_name().to_owned()
        }
        ObjectRef::Static(_) => return false,
    };
    let bank = ctx.host.types();
    types.iter().any(|t| bank.is_kind_of(&name, t))
}

fn near<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    position: Option<&Value>,
    types: &[String],
    radius: f32,
    which: Near,
) -> Value {
    let Some(center) = position.and_then(|p| position_or_object(ctx.host.world(), p)) else {
        return Value::array([]);
    };
    let found = ctx
        .host
        .world()
        .objects_near(center, f64::from(radius), which);
    let mut out = Vec::new();
    for (object, _) in found {
        if matches_types(ctx, object, types) {
            out.push(object_value(ctx.host.world(), object));
        }
    }
    Value::array(out)
}

/// `selector` is a number (WRP Object ID, searched anywhere from `position`) or a type name
/// (nearest within 50 m; "" for any object).
fn nearest_object<H: WorldHost>(ctx: &mut Ctx<'_, H>, position: &Value, selector: &Value) -> Value {
    let Some(center) = position_or_object(ctx.host.world(), position) else {
        return null_object();
    };
    if let Some(id) = selector.as_number() {
        let w = ctx.host.world();
        return w
            .find_static(DVec3::new(center.x, center.y, center.z), id as u32)
            .map_or_else(null_object, |r| object_value(w, r));
    }
    let ty = selector.as_str().unwrap_or("").to_owned();
    let found = ctx
        .host
        .world()
        .objects_near(center, NEAREST_OBJECT_RADIUS, Near::All);
    for (object, _) in found {
        if matches_types(ctx, object, std::slice::from_ref(&ty)) {
            return object_value(ctx.host.world(), object);
        }
    }
    null_object()
}
