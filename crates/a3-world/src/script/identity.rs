//! Object and group variables, the player, identities, vehicle variable names, synchronization
//! and dynamic simulation flags. Behaviour from the decompiled handlers; see
//! `docs/re/sqf-object-commands.md` (handler RVAs per command there and in
//! `docs/fidelity/sqf-verified.tsv`).

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, SqfError, Sym, Type, TypeSet, Value};

use super::groups::{GRP, group_arg};
use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, null_object, object_arg, object_value};
use crate::{EntityClass, EntityId, ObjectRef, VarOwner, World};

/// The engine's DIM error: "`got` elements provided, `expected` expected".
fn dim(got: usize, expected: usize) -> SqfError {
    SqfError::generic(format!("{got} elements provided, {expected} expected"))
}

fn items(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .map(|a| a.borrow().clone())
        .unwrap_or_default()
}

fn expect_string(value: &Value) -> Result<String, SqfError> {
    match value.as_str() {
        Some(s) => Ok(s.to_owned()),
        None => Err(SqfError::type_error(value, TypeSet::of(Type::String))),
    }
}

/// The Entity a value refers to, if it is one whose class derives from `class`.
fn entity_of_class(world: &World, value: &Value, class: EntityClass) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => world
            .entity(id)
            .filter(|e| e.class().is_kind_of(class))
            .map(|_| id),
        ObjectRef::Static(_) => None,
    }
}

fn object_owner(world: &World, value: &Value) -> Option<VarOwner> {
    object_arg(world, value).map(VarOwner::Object)
}

fn group_owner(world: &World, value: &Value) -> Option<VarOwner> {
    group_arg(world, value).map(VarOwner::Group)
}

/// `setVariable [name, value, public]` on an Object or group (handlers 0x540020 / 0x4b9030,
/// shared body 0x46d220 / 0x46bfb0). A null target does nothing, before any argument check.
fn set_variable<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    owner: Option<VarOwner>,
    args: &Value,
) -> Result<Value, SqfError> {
    let Some(owner) = owner else {
        return Ok(Value::Nothing);
    };
    let args = items(args);
    if args.is_empty() {
        return Err(dim(0, 1));
    }
    let name = expect_string(&args[0])?;
    match args.len() {
        2 => {}
        3 => {
            let public = &args[2];
            if !matches!(public.ty(), Type::Bool | Type::Number | Type::Array) {
                return Err(SqfError::type_error(
                    public,
                    TypeSet::of(Type::Bool)
                        .union(TypeSet::NUMBER)
                        .union(TypeSet::of(Type::Array)),
                ));
            }
            // A public flag (true, a client id or a list of them) also broadcasts the value;
            // stub until the network layer exists (#131).
        }
        n => return Err(dim(n, 3)),
    }
    if name.is_empty() {
        return Ok(Value::Nothing);
    }
    ctx.host
        .world_mut()
        .variables_mut(owner)
        .set(Sym::new(&name), args[1].clone());
    Ok(Value::Nothing)
}

/// `getVariable name` (handlers 0x533450 / 0x4b8ab0): the value, nil when unset or null.
fn get_variable_named(world: &World, owner: Option<VarOwner>, name: &str) -> Value {
    owner
        .and_then(|o| world.variables(o))
        .and_then(|v| v.get(Sym::new(name)).cloned())
        .unwrap_or(Value::Nil)
}

/// `getVariable [name, default]`: exactly two elements; the default for a null target, an
/// unset variable or a nil value.
fn get_variable_default(
    world: &World,
    owner: Option<VarOwner>,
    args: &Value,
) -> Result<Value, SqfError> {
    let args = items(args);
    if args.len() != 2 {
        return Err(dim(args.len(), 2));
    }
    let name = expect_string(&args[0])?;
    Ok(match get_variable_named(world, owner, &name) {
        Value::Nil => args[1].clone(),
        value => value,
    })
}

/// `allVariables` (0x49b2e0 / 0x49a4d0): the names, lower case. The original lists them in
/// hash-table order; we sort them.
fn all_variables(world: &World, owner: Option<VarOwner>) -> Value {
    let mut names: Vec<&str> = owner
        .and_then(|o| world.variables(o))
        .map(|v| v.iter().map(|(k, _)| k.as_str()).collect())
        .unwrap_or_default();
    names.sort_unstable();
    Value::array(names.into_iter().map(Value::from))
}

fn entity_value(world: &World, id: Option<EntityId>) -> Value {
    id.map_or_else(null_object, |id| object_value(world, ObjectRef::Entity(id)))
}

/// Whether a unit is controlled by a player. Single player: the local player only.
fn is_player_unit(world: &World, id: EntityId) -> bool {
    world.player() == Some(id)
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    register_variables(r);
    register_player(r);
    register_identity(r);
    register_sync(r);
}

fn register_variables<H: WorldHost>(r: &mut Registry<H>) {
    r.binary("setVariable", OBJ, ARR, NOTHING, |ctx, a, b| {
        let owner = object_owner(ctx.host.world(), &a);
        set_variable(ctx, owner, &b)
    });
    r.binary("setVariable", GRP, ARR, NOTHING, |ctx, a, b| {
        let owner = group_owner(ctx.host.world(), &a);
        set_variable(ctx, owner, &b)
    });
    r.binary("getVariable", OBJ, STR, TypeSet::ANYTHING, |ctx, a, b| {
        let w = ctx.host.world();
        Ok(get_variable_named(
            w,
            object_owner(w, &a),
            b.as_str().unwrap_or_default(),
        ))
    });
    r.binary("getVariable", GRP, STR, TypeSet::ANYTHING, |ctx, a, b| {
        let w = ctx.host.world();
        Ok(get_variable_named(
            w,
            group_owner(w, &a),
            b.as_str().unwrap_or_default(),
        ))
    });
    r.binary("getVariable", OBJ, ARR, TypeSet::ANYTHING, |ctx, a, b| {
        let w = ctx.host.world();
        get_variable_default(w, object_owner(w, &a), &b)
    });
    r.binary("getVariable", GRP, ARR, TypeSet::ANYTHING, |ctx, a, b| {
        let w = ctx.host.world();
        get_variable_default(w, group_owner(w, &a), &b)
    });
    r.unary("allVariables", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        Ok(all_variables(w, object_owner(w, &a)))
    });
    r.unary("allVariables", GRP, ARR, |ctx, a| {
        let w = ctx.host.world();
        Ok(all_variables(w, group_owner(w, &a)))
    });
}

fn register_player<H: WorldHost>(r: &mut Registry<H>) {
    // 0x8b1120: the player's Person, objNull without one.
    r.nular("player", OBJ, |ctx| {
        let w = ctx.host.world();
        Ok(entity_value(w, w.player()))
    });
    // 0x8a5bf0: the Object the camera is on.
    r.nular("cameraOn", OBJ, |ctx| {
        let w = ctx.host.world();
        Ok(entity_value(w, w.camera_on()))
    });
    // 0x542510: the vehicle a unit is in, or the object itself when it is in none (which is what
    // the engine does for anything that is not a unit, `objNull` included). A seat is recorded on
    // the *vehicle* (`ObjectState`'s `driver`, `gunner`, `cargo_seats`), so the lookup walks the
    // entities that hold one. That is linear in the world's objects per call, which is what the
    // engine's own seat lookup costs too.
    r.unary("vehicle", OBJ, OBJ, |ctx, a| {
        let w = ctx.host.world();
        let Some(unit) = entity_of_class(w, &a, EntityClass::EntityAi) else {
            return Ok(a);
        };
        let vehicle = w.entities().find_map(|e| {
            let state = w.object_state(e.id())?;
            let in_seat = state.driver == Some(unit)
                || state.gunner == Some(unit)
                || state.cargo_seats.iter().any(|(_, u)| *u == unit);
            in_seat.then_some(e.id())
        });
        Ok(vehicle.map_or(a, |v| object_value(w, ObjectRef::Entity(v))))
    });
    // 0x535b00: an EntityAI whose brain is player-controlled.
    r.unary("isPlayer", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            entity_of_class(w, &a, EntityClass::EntityAi).is_some_and(|id| is_player_unit(w, id)),
        ))
    });
    // 0x535b70: `isPlayer [unit]` checks the Person (also once dead).
    r.unary("isPlayer", ARR, BOOL, |ctx, a| {
        let w = ctx.host.world();
        let first = items(&a).into_iter().next().unwrap_or(Value::Nil);
        Ok(Value::Bool(
            entity_of_class(w, &first, EntityClass::Person).is_some_and(|id| is_player_unit(w, id)),
        ))
    });
}

fn register_identity<H: WorldHost>(r: &mut Registry<H>) {
    // 0x553560. AG EG _(the original broadcasts the identity)_.
    r.binary("setName", OBJ, STR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Person) {
            w.identity_mut(id).name = b.as_str().unwrap_or_default().to_owned();
        }
        Ok(Value::Nothing)
    });
    // 0x553610: [name, firstName, lastName]; null does nothing before the checks.
    r.binary("setName", OBJ, ARR, NOTHING, |ctx, a, b| {
        if object_arg(ctx.host.world(), &a).is_none() {
            return Ok(Value::Nothing);
        }
        let parts = items(&b);
        if parts.len() != 3 {
            return Err(dim(parts.len(), 3));
        }
        let name = expect_string(&parts[0])?;
        let first = expect_string(&parts[1])?;
        let last = expect_string(&parts[2])?;
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Person) {
            let identity = w.identity_mut(id);
            identity.name = name;
            identity.first_name = first;
            identity.last_name = last;
        }
        Ok(Value::Nothing)
    });
    // 0x538610.
    r.unary("name", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let text = match object_arg(w, &a) {
            None => "Error: No vehicle".to_owned(),
            Some(ObjectRef::Entity(id)) => {
                let class = w.entity(id).expect("exists").class();
                if class.is_kind_of(EntityClass::Person) {
                    // The original gives every unit a random name at creation; until that
                    // exists an unnamed unit has "".
                    w.identity(id).map(|i| i.name.clone()).unwrap_or_default()
                } else if class.is_kind_of(EntityClass::EntityAi) {
                    // The name of the commander, driver or gunner; vehicles have no crew yet.
                    "Error: No unit".to_owned()
                } else {
                    "Error: No vehicle".to_owned()
                }
            }
            Some(ObjectRef::Static(_)) => "Error: No vehicle".to_owned(),
        };
        Ok(Value::string(text))
    });

    // 0x53ba30: a Man's face. "custom" picks the player's custom face _(stored as given)_.
    r.binary("setFace", OBJ, STR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Man) {
            w.identity_mut(id).face = b.as_str().unwrap_or_default().to_owned();
        }
        Ok(Value::Nothing)
    });
    // 0x528110.
    r.unary("face", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(person_text(w, &a, |i| &i.face)))
    });
    // 0x553880.
    r.binary("setPitch", OBJ, NUM, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Person) {
            w.identity_mut(id).pitch = b.as_number().unwrap_or(0.0);
        }
        Ok(Value::Nothing)
    });
    // 0x538940: a number, although the command table declares STRING; -1 for non-Persons.
    r.unary("pitch", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        let pitch = match entity_of_class(w, &a, EntityClass::Person) {
            Some(id) => w.identity(id).map_or(1.0, |i| i.pitch),
            None => -1.0,
        };
        Ok(Value::Number(pitch))
    });
    // 0x553940.
    r.binary("setSpeaker", OBJ, STR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Person) {
            w.identity_mut(id).speaker = b.as_str().unwrap_or_default().to_owned();
        }
        Ok(Value::Nothing)
    });
    // 0x5418f0.
    r.unary("speaker", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(person_text(w, &a, |i| &i.speaker)))
    });
    // 0x5537d0.
    r.binary("setNameSound", OBJ, STR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Person) {
            w.identity_mut(id).name_sound = b.as_str().unwrap_or_default().to_owned();
        }
        Ok(Value::Nothing)
    });
    // 0x5388b0.
    r.unary("nameSound", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(person_text(w, &a, |i| &i.name_sound)))
    });
    // 0x53d440: CfgIdentities in missionConfigFile, then campaignConfigFile, then configFile.
    r.binary("setIdentity", OBJ, STR, NOTHING, |ctx, a, b| {
        let Some(id) = entity_of_class(ctx.host.world(), &a, EntityClass::Person) else {
            return Ok(Value::Nothing);
        };
        let class_name = b.as_str().unwrap_or_default().to_owned();
        let Some(identity) = find_identity(ctx.host, &class_name) else {
            return Ok(Value::Nothing);
        };
        *ctx.host.world_mut().identity_mut(id) = identity;
        Ok(Value::Nothing)
    });

    // 0x571f00: Entities only. AG EG _(the original broadcasts it)_.
    r.binary("setVehicleVarName", OBJ, STR, NOTHING, |ctx, a, b| {
        let w = ctx.host.world_mut();
        if let Some(id) = entity_of_class(w, &a, EntityClass::Entity) {
            w.set_var_name(id, b.as_str().unwrap_or_default());
        }
        Ok(Value::Nothing)
    });
    // 0x56ca10.
    r.unary("vehicleVarName", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        let name = entity_of_class(w, &a, EntityClass::Entity).map_or("", |id| w.var_name(id));
        Ok(Value::string(name))
    });
}

/// An identity text of a Person (`face`, `speaker`, `nameSound`), `""` for anything else.
fn person_text(
    world: &World,
    value: &Value,
    field: impl Fn(&crate::Identity) -> &String,
) -> String {
    entity_of_class(world, value, EntityClass::Person)
        .and_then(|id| world.identity(id))
        .map(|i| field(i).clone())
        .unwrap_or_default()
}

/// The identity class `name` of `CfgIdentities`, from the first config that has it.
fn find_identity<H: WorldHost>(host: &mut H, name: &str) -> Option<crate::Identity> {
    let read = |tree: &a3_config::ConfigTree| {
        let class = tree.root().get("CfgIdentities").get(name);
        class.is_class().then(|| crate::Identity {
            name: class.get("name").text(),
            first_name: String::new(),
            last_name: String::new(),
            face: class.get("face").text(),
            glasses: class.get("glasses").text(),
            speaker: class.get("speaker").text(),
            pitch: class.get("pitch").number(),
            name_sound: class.get("nameSound").text(),
        })
    };
    for tree in host.mission_configs() {
        if let Some(identity) = read(&tree) {
            return Some(identity);
        }
    }
    read(host.types().config())
}

fn register_sync<H: WorldHost>(r: &mut Registry<H>) {
    // 0x5244f0: every element must be an Object (nothing changes otherwise). A link is added in
    // both directions where the kinds allow it (see `sync_kind`).
    r.binary("synchronizeObjectsAdd", OBJ, ARR, NOTHING, |ctx, a, b| {
        change_sync(ctx, &a, &b, true)
    });
    // 0x539680.
    r.binary(
        "synchronizeObjectsRemove",
        OBJ,
        ARR,
        NOTHING,
        |ctx, a, b| change_sync(ctx, &a, &b, false),
    );
    // 0x532f20.
    r.unary("synchronizedObjects", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        let list = match object_arg(w, &a) {
            Some(ObjectRef::Entity(id)) if sync_kind(w, id) != SyncKind::None => w
                .synchronized(id)
                .iter()
                .filter(|&&e| w.entity(e).is_some_and(|e| !e.is_deleted()))
                .map(|&e| object_value(w, ObjectRef::Entity(e)))
                .collect(),
            _ => Vec::new(),
        };
        Ok(Value::array(list))
    });

    // 0x17e980: EntityAI only.
    r.binary(
        "enableDynamicSimulation",
        OBJ,
        BOOL,
        NOTHING,
        |ctx, a, b| {
            let w = ctx.host.world_mut();
            if let Some(id) = entity_of_class(w, &a, EntityClass::EntityAi) {
                w.set_dynamic_simulation(id, b.as_bool().unwrap_or(false));
            }
            Ok(Value::Nothing)
        },
    );
    // 0x17e8c0.
    r.binary(
        "enableDynamicSimulation",
        GRP,
        BOOL,
        NOTHING,
        |ctx, a, b| {
            let w = ctx.host.world_mut();
            if let Some(g) = group_arg(w, &a) {
                w.set_group_dynamic_simulation(g, b.as_bool().unwrap_or(false));
            }
            Ok(Value::Nothing)
        },
    );
    // 0x17eab0: any Entity.
    r.unary("dynamicSimulationEnabled", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            entity_of_class(w, &a, EntityClass::Entity).is_some_and(|id| w.dynamic_simulation(id)),
        ))
    });
    // 0x17ea50.
    r.unary("dynamicSimulationEnabled", GRP, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            group_arg(w, &a).is_some_and(|g| w.group_dynamic_simulation(g)),
        ))
    });
}

/// What an Object is for synchronization (`FUN_1404b3f90`): an AI unit (its list lives on the
/// unit), a trigger, or anything else (no list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncKind {
    Unit,
    Trigger,
    None,
}

fn sync_kind(world: &World, id: EntityId) -> SyncKind {
    let Some(e) = world.entity(id) else {
        return SyncKind::None;
    };
    if e.class().is_kind_of(EntityClass::EntityAi) && world.group_of(id).is_some() {
        SyncKind::Unit
    } else if e.class().is_kind_of(EntityClass::Detector) {
        SyncKind::Trigger
    } else {
        SyncKind::None
    }
}

/// Which kinds may be linked, from the table at 0x141acc7e0 (`[from][to]`): a unit with anything,
/// a trigger with a unit.
fn sync_allowed(from: SyncKind, to: SyncKind) -> bool {
    matches!(
        (from, to),
        (SyncKind::Unit, _) | (SyncKind::Trigger, SyncKind::Unit)
    )
}

fn change_sync<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    b: &Value,
    add: bool,
) -> Result<Value, SqfError> {
    let w = ctx.host.world();
    let source = object_arg(w, a);
    // `synchronizeObjectsAdd` returns for a null source before checking the list; the removal
    // checks it first.
    if add && source.is_none() {
        return Ok(Value::Nothing);
    }
    let others = items(b);
    if let Some(bad) = others.iter().find(|v| v.ty() != Type::Object) {
        return Err(SqfError::type_error(bad, OBJ));
    }
    let Some(ObjectRef::Entity(source)) = source else {
        return Ok(Value::Nothing);
    };
    let from = sync_kind(w, source);
    let targets: Vec<(EntityId, SyncKind)> = others
        .iter()
        .filter_map(|v| match object_arg(w, v) {
            Some(ObjectRef::Entity(id)) => Some((id, sync_kind(w, id))),
            _ => None,
        })
        .collect();
    let w = ctx.host.world_mut();
    for (target, to) in targets {
        if add {
            if !sync_allowed(from, to) {
                continue;
            }
            if from != SyncKind::None {
                w.add_sync(source, target);
            }
            if to != SyncKind::None {
                w.add_sync(target, source);
            }
        } else {
            w.remove_sync(source, target);
            w.remove_sync(target, source);
        }
    }
    Ok(Value::Nothing)
}
