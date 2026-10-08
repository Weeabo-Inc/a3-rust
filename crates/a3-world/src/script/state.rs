//! Object state: type, damage, locality, simulation, visibility, network ids.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, is_server, null_object, object_arg, object_value,
};
use crate::{ClientId, EntityId, Locality, NetworkId, ObjectRef, SimulationClass, World};

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

    // Damage. Hit points and destruction effects come with #128.
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
    // AG EG.
    r.binary("setDamage", OBJ, NUM, NOTHING, |ctx, a, b| {
        set_damage(ctx, &a, b.as_number().unwrap_or(0.0));
        Ok(Value::Nothing)
    });
    r.binary("setDamage", OBJ, ARR, NOTHING, |ctx, a, b| {
        let d = b
            .as_array()
            .and_then(|x| x.borrow().first().and_then(Value::as_number))
            .unwrap_or(0.0);
        set_damage(ctx, &a, d);
        Ok(Value::Nothing)
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

/// The Entity a state change applies to, promoting a Static object if needed.
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

fn set_damage<H: WorldHost>(ctx: &mut Ctx<'_, H>, object: &Value, damage: f32) {
    if let Some(id) = entity_for_change(ctx, object) {
        if let Some(e) = ctx.host.world_mut().entity_mut(id) {
            e.set_damage(damage);
        }
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
