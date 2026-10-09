//! Object state: type, damage, locality, simulation, visibility, network ids.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, is_server, null_object, object_arg, object_value,
};
use crate::{
    ClientId, DamageHit, EntityId, Locality, NetworkId, ObjectRef, SimulationClass, World,
};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // `typeOf`: the config class; "" for plain Static objects (trees, rocks, walls).
    r.unary("typeOf", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let name = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => {
                let e = w.entity(id).expect("exists");
                if e.class() == SimulationClass::Plain {
                    String::new()
                } else {
                    e.type_name().to_owned()
                }
            }
            _ => String::new(),
        };
        Ok(Value::string(name))
    });
    // `isKindOf`: config inheritance.
    r.binary("isKindOf", OBJ, STR, BOOL, |ctx, a, b| {
        let w = ctx.host.world();
        let name = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").type_name().to_owned(),
            _ => return Ok(Value::Bool(false)),
        };
        let base = b.as_str().unwrap_or_default().to_owned();
        Ok(Value::Bool(ctx.host.types().is_kind_of(&name, &base)))
    });
    r.binary("isKindOf", STR, STR, BOOL, |ctx, a, b| {
        let (name, base) = (
            a.as_str().unwrap_or_default().to_owned(),
            b.as_str().unwrap_or_default().to_owned(),
        );
        Ok(Value::Bool(ctx.host.types().is_kind_of(&name, &base)))
    });

    // Damage. The formulas and the locality rules are in `docs/re/sim-damage.md`; a hit point
    // is addressed by config name (`HitHead`) or by model selection (`head`).
    r.unary("alive", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").is_alive(),
            Some(ObjectRef::Static(_)) => true,
            None => false,
        }))
    });
    for name in ["damage", "getDammage"] {
        r.unary(name, OBJ, NUM, |ctx, a| {
            let w = ctx.host.world();
            Ok(Value::Number(match object_arg(w, &a) {
                Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").damage(),
                _ => 0.0,
            }))
        });
    }
    // AG EG: the argument is local (only the owner applies it) and the effect is global.
    r.binary("setDamage", OBJ, NUM, NOTHING, |ctx, a, b| {
        set_damage(
            ctx,
            &a,
            DamageHit::total(b.as_number().unwrap_or(0.0)).scripted(),
        );
        Ok(Value::Nothing)
    });
    // `setDamage [damage, useEffects, killer, instigator, allowResurrection, shot]`. The extra
    // arguments are optional; `allowResurrection` and `shot` are not used yet.
    r.binary("setDamage", OBJ, ARR, NOTHING, |ctx, a, b| {
        let arguments = b
            .as_array()
            .map(|x| (*x.borrow()).clone())
            .unwrap_or_default();
        let (damage, use_effects, killer, instigator) = {
            let w = ctx.host.world();
            let object = |i: usize| {
                arguments
                    .get(i)
                    .and_then(|v| object_arg(w, v))
                    .and_then(|r| match r {
                        ObjectRef::Entity(id) => Some(id),
                        ObjectRef::Static(_) => None,
                    })
            };
            (
                arguments.first().and_then(Value::as_number).unwrap_or(0.0),
                arguments.get(1).and_then(Value::as_bool).unwrap_or(true),
                object(2),
                object(3),
            )
        };
        let mut hit = DamageHit::total(damage).scripted();
        if !use_effects {
            hit = hit.without_effects();
        }
        if let Some(killer) = killer {
            hit = hit.caused_by(killer);
        }
        if let Some(instigator) = instigator {
            hit = hit.instigated_by(instigator);
        }
        set_damage(ctx, &a, hit);
        Ok(Value::Nothing)
    });
    // The hit point's damage, 0 for a name the Object's type does not have.
    r.binary("getHitPointDamage", OBJ, STR, NUM, |ctx, a, b| {
        let w = ctx.host.world();
        let name = b.as_str().unwrap_or_default().to_owned();
        let damage = object_arg(w, &a)
            .and_then(|object| w.hit_point_damage(object, &name))
            .unwrap_or(0.0);
        Ok(Value::Number(damage))
    });
    // AL EG: `setHitPointDamage [name, damage]`. The command's own page says it has no effect
    // while `allowDamage` is false, unlike `setDamage`/`setHit`/`setHitIndex` (which
    // `allowDamage` does not stop, per its page).
    r.binary("setHitPointDamage", OBJ, ARR, NOTHING, |ctx, a, b| {
        let Some((target, damage)) = hit_arguments(&b) else {
            return Ok(Value::Nothing);
        };
        if let Some(id) = entity_for_change(ctx, &a) {
            if ctx.host.world().damage_allowed(ObjectRef::Entity(id)) == Some(false) {
                return Ok(Value::Nothing);
            }
            let index = hit_target_index(ctx.host.world(), id, &target);
            ctx.host
                .world_mut()
                .apply_damage_to(id, DamageHit::default().scripted().at_name(index, damage));
        }
        Ok(Value::Nothing)
    });
    // AL EG: `setHit [selection, damage]`, by model selection.
    r.binary("setHit", OBJ, ARR, NOTHING, |ctx, a, b| {
        if let Some((target, damage)) = hit_arguments(&b) {
            set_hit_point(ctx, &a, &target, damage);
        }
        Ok(Value::Nothing)
    });
    // AL EG: `setHitIndex [index, damage]`, by hit point index.
    r.binary("setHitIndex", OBJ, ARR, NOTHING, |ctx, a, b| {
        if let Some((target, damage)) = hit_arguments(&b) {
            set_hit_point(ctx, &a, &target, damage);
        }
        Ok(Value::Nothing)
    });
    // The selection's damage, 0 for a selection the Object's type does not have.
    r.binary("getHit", OBJ, STR, NUM, |ctx, a, b| {
        let w = ctx.host.world();
        let selection = b.as_str().unwrap_or_default().to_owned();
        let damage = object_arg(w, &a)
            .and_then(|object| w.hit_point_damage_of_selection(object, &selection))
            .unwrap_or(0.0);
        Ok(Value::Number(damage))
    });
    // The hit point's damage by index, 0 for an index out of range.
    r.binary("getHitIndex", OBJ, NUM, NUM, |ctx, a, b| {
        let w = ctx.host.world();
        let index = b.as_number().unwrap_or(-1.0).max(0.0) as usize;
        let damage = object_arg(w, &a)
            .and_then(|object| w.hit_point_damage_of_index(object, index))
            .unwrap_or(0.0);
        Ok(Value::Number(damage))
    });
    // `[hitpointNames, selectionNames, damageValues]`, in hit point order.
    r.unary("getAllHitPointsDamage", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        let all = object_arg(w, &a).and_then(|object| w.all_hit_points_damage(object));
        let (hit_points, selections, damage) = match all {
            Some(all) => (
                all.hit_points,
                all.selections,
                all.damage
                    .into_iter()
                    .map(Value::Number)
                    .collect::<Vec<Value>>(),
            ),
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        Ok(Value::array([
            Value::array(
                hit_points
                    .into_iter()
                    .map(Value::string)
                    .collect::<Vec<Value>>(),
            ),
            Value::array(
                selections
                    .into_iter()
                    .map(Value::string)
                    .collect::<Vec<Value>>(),
            ),
            Value::array(damage),
        ]))
    });
    // AL EG.
    r.binary("allowDamage", OBJ, BOOL, NOTHING, |ctx, a, b| {
        let allowed = b.as_bool().unwrap_or(true);
        let w = ctx.host.world_mut();
        // Unlike the other damage commands this one does not work on a Static object (terrain
        // vegetation included), so it does not promote one either.
        if let Some(object) = object_arg(w, &a) {
            w.set_damage_allowed(object, allowed);
        }
        Ok(Value::Nothing)
    });
    // False for an Object that is not local.
    r.unary("isDamageAllowed", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        let allowed = object_arg(w, &a).is_some_and(|object| {
            w.damage_allowed(object) == Some(true)
                && match object {
                    ObjectRef::Entity(id) => w.entity(id).is_some_and(|e| e.is_local()),
                    ObjectRef::Static(_) => false,
                }
        });
        Ok(Value::Bool(allowed))
    });

    // Locality.
    r.unary("local", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").is_local(),
            // The original's plain `Object` is always local.
            Some(ObjectRef::Static(_)) => true,
            None => false,
        }))
    });
    // Server only: clients get 0.
    r.unary("owner", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        let owner = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) if is_server(w) => {
                match w.entity(id).expect("exists").locality() {
                    Locality::Local => ClientId::SERVER.0,
                    Locality::Remote { owner } => owner.map_or(0, |c| c.0),
                }
            }
            _ => 0,
        };
        Ok(Value::Number(owner as f32))
    });
    r.nular("clientOwner", NUM, |ctx| {
        Ok(Value::Number(ctx.host.world().local_client().0 as f32))
    });

    // Network ids (only an encoding until the network layer exists).
    r.unary("netId", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let text = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w
                .entity(id)
                .expect("exists")
                .network_id()
                .unwrap_or(NetworkId::NULL)
                .to_string(),
            Some(ObjectRef::Static(key)) => key.network_id().to_string(),
            None => String::new(),
        };
        Ok(Value::string(text))
    });
    r.unary("objectFromNetId", STR, OBJ, |ctx, a| {
        let w = ctx.host.world();
        Ok(a.as_str()
            .and_then(|s| s.parse::<NetworkId>().ok())
            .and_then(|n| w.resolve(n))
            .map_or_else(null_object, |r| object_value(w, r)))
    });
    // The WRP Object ID. Run-time Entities print -1 _(uncertain: the original prints its
    // `+0xa4` field, whose value for created objects is unknown)_.
    r.unary("getObjectID", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let id = match object_arg(w, &a) {
            Some(ObjectRef::Static(key)) => {
                i64::from(w.static_object(key).expect("exists").object_id)
            }
            Some(ObjectRef::Entity(id)) => static_object_id(w, id).unwrap_or(-1),
            None => -1,
        };
        Ok(Value::string(id.to_string()))
    });

    // Simulation. AL EG.
    r.binary("enableSimulation", OBJ, BOOL, NOTHING, |ctx, a, b| {
        set_simulation(ctx, &a, b.as_bool().unwrap_or(true), false);
        Ok(Value::Nothing)
    });
    // Server only.
    r.binary("enableSimulationGlobal", OBJ, BOOL, NOTHING, |ctx, a, b| {
        set_simulation(ctx, &a, b.as_bool().unwrap_or(true), true);
        Ok(Value::Nothing)
    });
    r.unary("simulationEnabled", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").simulation_enabled(),
            _ => false,
        }))
    });

    // Visibility. AG EL.
    r.unary("hideObject", OBJ, NOTHING, |ctx, a| {
        hide(ctx, &a, true, false);
        Ok(Value::Nothing)
    });
    r.binary("hideObject", OBJ, BOOL, NOTHING, |ctx, a, b| {
        hide(ctx, &a, b.as_bool().unwrap_or(true), false);
        Ok(Value::Nothing)
    });
    // Server only.
    r.unary("hideObjectGlobal", OBJ, NOTHING, |ctx, a| {
        hide(ctx, &a, true, true);
        Ok(Value::Nothing)
    });
    r.binary("hideObjectGlobal", OBJ, BOOL, NOTHING, |ctx, a, b| {
        hide(ctx, &a, b.as_bool().unwrap_or(true), true);
        Ok(Value::Nothing)
    });
    r.unary("isObjectHidden", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) => w.entity(id).expect("exists").is_hidden(),
            _ => false,
        }))
    });
}

/// The Object ID of a promoted Static object.
fn static_object_id(world: &World, id: EntityId) -> Option<i64> {
    let key = crate::StaticKey::from_network_id(world.entity(id)?.network_id()?)?;
    world.static_object(key).map(|o| i64::from(o.object_id))
}

/// The Entity a state change applies to, promoting a Static object if needed. A Static object is
/// promoted with the `CfgVehicles` class its model path belongs to when the config knows one —
/// a house gets its hit points, its armor and its ruin — and with a plain type otherwise.
fn entity_for_change<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value) -> Option<EntityId> {
    match object_arg(ctx.host.world(), value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(key) => {
            let model = ctx
                .host
                .world()
                .static_model(key)
                .map(|m| m.as_str().to_owned());
            match model.and_then(|model| ctx.host.types().for_model(&model)) {
                Some(ty) => ctx.host.world_mut().promote_static(key, ty).ok(),
                None => ctx
                    .host
                    .world_mut()
                    .promote_static_with_model_type(key)
                    .ok(),
            }
        }
    }
}

/// Applies a hit (already built) to the Object a command was given, promoting a Static object.
fn set_damage<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, hit: DamageHit) {
    if let Some(id) = entity_for_change(ctx, object) {
        ctx.host.world_mut().apply_damage_to(id, hit);
    }
}

/// `setHitPointDamage [name, damage]`, `setHit [selection, damage]`, `setHitIndex [index, damage]`:
/// the target and the damage, when the argument is an array with a first element.
fn hit_arguments(value: &Value) -> Option<(Value, f32)> {
    let array = value.as_array()?;
    let array = array.borrow();
    let target = array.first()?.clone();
    let damage = array.get(1).and_then(Value::as_number).unwrap_or(0.0);
    Some((target, damage))
}

/// The hit point a script's target names: a config name, a model selection, or an index.
fn hit_target_index(world: &World, id: EntityId, target: &Value) -> Option<usize> {
    let model = world.entity(id)?.entity_type().damage();
    if let Some(name) = target.as_str() {
        return model
            .hit_point_index(name)
            .or_else(|| model.hit_point_index_of_selection(name));
    }
    target.as_number().map(|index| index.max(0.0) as usize)
}

/// Sets one hit point of the Object a command was given, by config name, selection or index.
/// The value is the new damage, 0..1; an unknown hit point does nothing.
fn set_hit_point<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, target: &Value, damage: f32) {
    if let Some(id) = entity_for_change(ctx, object) {
        let index = hit_target_index(ctx.host.world(), id, target);
        ctx.host
            .world_mut()
            .apply_damage_to(id, DamageHit::default().scripted().at_name(index, damage));
    }
}

fn set_simulation<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, enabled: bool, global: bool) {
    let w = ctx.host.world_mut();
    if global && !is_server(w) {
        return;
    }
    let Some(ObjectRef::Entity(id)) = object_arg(w, object) else {
        return;
    };
    if let Some(e) = w.entity_mut(id) {
        // `enableSimulation` needs a local argument; the global form runs on the server.
        if global || e.is_local() {
            e.set_simulation_enabled(enabled);
        }
    }
}

fn hide<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, hidden: bool, global: bool) {
    if global && !is_server(ctx.host.world()) {
        return;
    }
    if let Some(id) = entity_for_change(ctx, object) {
        if let Some(e) = ctx.host.world_mut().entity_mut(id) {
            e.set_hidden(hidden);
        }
    }
}
