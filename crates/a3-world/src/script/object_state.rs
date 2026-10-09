//! Unit and vehicle state commands: captive, unit position, AI features, skill, rank, fuel and
//! supply cargo, locks, engine, textures and materials, and vehicle cargo. From the decompiled
//! handlers and the oracle probes (`tools/oracle/probes/98_object_state_vr.probes`); see
//! `docs/re/sqf-object-state.md`.

use a3_config::ConfigTree;
use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, SqfError, Value};

use super::groups::{GRP, group_arg};
use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, object_arg};
use crate::gear::make_magazine;
use crate::inventory::{ItemKind, classify};
use crate::object_state::{Cargo, ObjectState, RANKS, UnitPos, ai_feature};
use crate::{EntityClass, EntityId, ObjectRef, World};

fn dim(got: usize, expected: usize) -> SqfError {
    SqfError::generic(format!("{got} elements provided, {expected} expected"))
}

/// The engine's enum conversion error.
fn bad_enum(name: &str) -> SqfError {
    SqfError::generic(format!("Foreign error: Unknown enum value: \"{name}\""))
}

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

/// An alive AI unit (an EntityAI with a brain: in a group).
fn unit(world: &World, value: &Value) -> Option<EntityId> {
    entity(world, value, EntityClass::EntityAi)
        .filter(|&id| world.group_of(id).is_some())
        .filter(|&id| world.entity(id).is_some_and(|e| e.is_alive()))
}

/// A number from the Entity's config class (`fuelCapacity`, `transportFuel`, ...).
fn config_number<H: WorldHost>(host: &mut H, id: EntityId, name: &str) -> f32 {
    let Some(type_name) = host.world().entity(id).map(|e| e.type_name().to_owned()) else {
        return 0.0;
    };
    let config = host.types().config_arc();
    let entry = config.root().get("CfgVehicles").get(&type_name).get(name);
    if entry.is_number() {
        entry.number()
    } else {
        0.0
    }
}

fn state<'a, H: WorldHost>(ctx: &'a mut Ctx<'_, H>, id: EntityId) -> &'a mut ObjectState {
    ctx.host.world_mut().object_state_mut(id)
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    register_unit(r);
    register_vehicle(r);
    register_cargo(r);
}

fn register_unit<H: WorldHost>(r: &mut Registry<H>) {
    // 0x53b1a0: a unit's brain gets the captive number (true 1, false 0, a number truncated).
    // A captive unit counts as civilian (`side`).
    r.binary("setCaptive", OBJ, BOOL.union(NUM), NOTHING, |ctx, a, b| {
        if let Some(id) = unit(ctx.host.world(), &a) {
            let n = match b {
                Value::Bool(v) => i32::from(v),
                Value::Number(n) => n as i32,
                _ => 0,
            };
            state(ctx, id).captive = n;
        }
        Ok(Value::Nothing)
    });
    // 0x526070.
    r.unary("captive", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(unit(w, &a).is_some_and(|id| w.captive(id) > 0)))
    });
    r.unary("captiveNum", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Number(
            unit(w, &a).map_or(0, |id| w.captive(id)) as f32
        ))
    });
    // 0x53fc20: "UP", "DOWN", "MIDDLE", "AUTO", any case; anything else leaves the stance alone
    // (oracle: `setUnitPos "bad"` runs on and `unitPos` keeps its value; the original logs the
    // enum error to the RPT).
    r.binary("setUnitPos", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        if let (Some(pos), Some(id)) = (
            UnitPos::parse(&name),
            entity(ctx.host.world(), &a, EntityClass::Man),
        ) {
            state(ctx, id).unit_pos = pos;
        }
        Ok(Value::Nothing)
    });
    // 0x533390 -> Man vfunc +0x1c48 (0x72c650): the first of three levels that is not AUTO —
    // Man+0x228c, the script's `setUnitPos` (+0x2290), the AI's weak request (+0x2294).
    r.unary("unitPos", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(
            entity(w, &a, EntityClass::Man).map_or("", |id| w.effective_unit_pos(id).name()),
        ))
    });
    // 0x526960 / 0x527420: feature names from the enum at 0x1400e7b30, "ALL" for every bit.
    r.binary("disableAI", OBJ, STR, NOTHING, |ctx, a, b| {
        set_ai(ctx, &a, &b, true)
    });
    r.binary("enableAI", OBJ, STR, NOTHING, |ctx, a, b| {
        set_ai(ctx, &a, &b, false)
    });
    // 0x481de0.
    r.binary("checkAIFeature", OBJ, STR, BOOL, |ctx, a, b| {
        let name = text(&b);
        let bits = ai_feature(&name).ok_or_else(|| bad_enum(&name))?;
        let w = ctx.host.world();
        Ok(Value::Bool(entity(w, &a, EntityClass::Man).is_some_and(
            |id| w.object_state(id).is_none_or(|s| s.ai_disabled & bits == 0),
        )))
    });
    // 0x481cc0: the global feature switches (`enableAIFeature`), off unless enabled — no
    // global switch exists yet _(stub)_.
    r.unary("checkAIFeature", STR, BOOL, |_, _| Ok(Value::Bool(false)));
    // 0x1907c0: courage; the morale system does not exist yet _(stub)_.
    r.binary("allowFleeing", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("allowFleeing", GRP, NUM, NOTHING, |ctx, a, _| {
        let _ = group_arg(ctx.host.world(), &a);
        Ok(Value::Nothing)
    });
    // 0x528630: nobody flees yet.
    r.unary("fleeing", OBJ, BOOL, |_, _| Ok(Value::Bool(false)));

    // 0x5660a0 / 0x5660c0: rank names any case; an unknown name sets PRIVATE and runs on (oracle:
    // `setRank "bogus"` leaves PRIVATE; the original logs the enum error to the RPT).
    for name in ["setRank", "setUnitRank"] {
        r.binary(name, OBJ, STR, NOTHING, |ctx, a, b| {
            let name = text(&b);
            let rank = RANKS
                .iter()
                .position(|r| r.eq_ignore_ascii_case(&name))
                .unwrap_or(0);
            if let Some(id) = entity(ctx.host.world(), &a, EntityClass::EntityAi) {
                state(ctx, id).rank = rank;
            }
            Ok(Value::Nothing)
        });
    }
    // 0x565e60.
    r.unary("rank", OBJ, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(
            entity(w, &a, EntityClass::EntityAi)
                .map_or("", |id| RANKS[w.object_state(id).map_or(0, |s| s.rank)]),
        ))
    });
    // 0x565f80.
    r.unary("rankId", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Number(
            entity(w, &a, EntityClass::EntityAi)
                .map_or(0, |id| w.object_state(id).map_or(0, |s| s.rank)) as f32,
        ))
    });

    // 0x8b8100 / 0x566000: the general skill, clamped to 0..1; it resets the sub-skills.
    for name in ["setSkill", "setUnitAbility"] {
        r.binary(name, OBJ, NUM, NOTHING, |ctx, a, b| {
            if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Man) {
                let s = state(ctx, id);
                s.skill = Some(b.as_number().unwrap_or(0.0).clamp(0.0, 1.0));
                s.sub_skills.clear();
            }
            Ok(Value::Nothing)
        });
    }
    // 0x53f3b0: `[name, value]`.
    r.binary("setSkill", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if args.len() < 2 {
            return Err(dim(args.len(), 2));
        }
        let name = text(&args[0]);
        let value = args[1].as_number().unwrap_or(0.0).clamp(0.0, 1.0);
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Man) {
            state(ctx, id).set_sub_skill(&name, value);
        }
        Ok(Value::Nothing)
    });
    // 0x8b6840.
    r.unary("skill", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Number(
            entity(w, &a, EntityClass::Man)
                .map_or(0.0, |id| w.object_state(id).map_or(0.5, ObjectState::skill)),
        ))
    });
    // 0x532b10.
    r.binary("skill", OBJ, STR, NUM, |ctx, a, b| {
        let w = ctx.host.world();
        let name = text(&b);
        Ok(Value::Number(
            entity(w, &a, EntityClass::Man).map_or(0.0, |id| {
                w.object_state(id).map_or(0.5, |s| s.sub_skill(&name))
            }),
        ))
    });
}

fn set_ai<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    b: &Value,
    disable: bool,
) -> Result<Value, SqfError> {
    let name = text(b);
    // An unknown name changes nothing (oracle: `disableAI "NOSUCH"` runs on; the original logs
    // the enum error to the RPT).
    let Some(bits) = ai_feature(&name) else {
        return Ok(Value::Nothing);
    };
    if let Some(id) = entity(ctx.host.world(), a, EntityClass::Man) {
        let s = state(ctx, id);
        if disable {
            s.ai_disabled |= bits;
        } else {
            s.ai_disabled &= !bits;
        }
    }
    Ok(Value::Nothing)
}

/// Hidden selection names of an Entity's type.
fn hidden_selections<H: WorldHost>(host: &mut H, id: EntityId) -> (Vec<String>, Vec<String>) {
    let Some(type_name) = host.world().entity(id).map(|e| e.type_name().to_owned()) else {
        return (Vec::new(), Vec::new());
    };
    let config = host.types().config_arc();
    let class = config.root().get("CfgVehicles").get(&type_name);
    let strings = |name: &str| -> Vec<String> {
        class
            .get(name)
            .array()
            .iter()
            .filter_map(|v| match v {
                a3_config::Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    };
    (
        strings("hiddenSelections"),
        strings("hiddenSelectionsTextures"),
    )
}

/// A texture path as `getObjectTextures` prints a config one: lower case, no leading `\`.
fn texture_path(path: &str) -> String {
    path.trim_start_matches('\\').to_ascii_lowercase()
}

fn set_texture<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    b: &Value,
    material: bool,
) -> Result<Value, SqfError> {
    let args = items(b);
    if args.len() < 2 {
        return Err(dim(args.len(), 2));
    }
    let Some(id) = entity(ctx.host.world(), a, EntityClass::Entity) else {
        return Ok(Value::Nothing);
    };
    let (selections, _) = hidden_selections(ctx.host, id);
    let index = match &args[0] {
        Value::Number(n) => (*n >= 0.0).then_some(*n as usize),
        Value::String(s) => selections.iter().position(|x| x.eq_ignore_ascii_case(s)),
        _ => None,
    };
    let Some(index) = index.filter(|&i| i < selections.len()) else {
        return Ok(Value::Nothing);
    };
    let path = text(&args[1]);
    // A texture that is neither procedural (`#(...)`) nor a file is refused ("Picture %s not
    // found").
    if !material && !path.is_empty() && !path.starts_with('#') && !ctx.host.file_exists(&path) {
        return Ok(Value::Nothing);
    }
    let s = state(ctx, id);
    let list = if material {
        &mut s.materials
    } else {
        &mut s.textures
    };
    ObjectState::set_override(list, index, path);
    Ok(Value::Nothing)
}

fn get_textures<H: WorldHost>(ctx: &mut Ctx<'_, H>, a: &Value, material: bool) -> Value {
    let Some(id) = entity(ctx.host.world(), a, EntityClass::Entity) else {
        return Value::array([]);
    };
    let (selections, textures) = hidden_selections(ctx.host, id);
    let s = ctx.host.world().object_state(id);
    Value::array((0..selections.len()).map(|i| {
        let over = s.and_then(|s| {
            ObjectState::override_at(if material { &s.materials } else { &s.textures }, i)
        });
        let base = if material {
            String::new()
        } else {
            textures.get(i).map(|t| texture_path(t)).unwrap_or_default()
        };
        Value::string(over.map_or(base, str::to_owned))
    }))
}

fn register_vehicle<H: WorldHost>(r: &mut Registry<H>) {
    // 0x53c440: an EntityAI's fuel as a fraction of `fuelCapacity`, clamped; units have none.
    r.binary("setFuel", OBJ, NUM, NOTHING, |ctx, a, b| {
        let w = ctx.host.world();
        if let Some(id) = entity(w, &a, EntityClass::EntityAi)
            .filter(|&id| w.entity(id).is_some_and(|e| e.is_alive()))
        {
            if config_number(ctx.host, id, "fuelCapacity") > 0.0 {
                state(ctx, id).fuel = Some(b.as_number().unwrap_or(0.0).clamp(0.0, 1.0));
            }
        }
        Ok(Value::Nothing)
    });
    // 0x528860: 1 without a fuel tank, 0 for anything that is not an EntityAI.
    r.unary("fuel", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        let Some(id) = entity(w, &a, EntityClass::EntityAi) else {
            return Ok(Value::Number(0.0));
        };
        let fuel = w.object_state(id).and_then(|s| s.fuel).unwrap_or(1.0);
        Ok(Value::Number(fuel))
    });
    // Supply cargo: a fraction of `transportFuel` / `transportAmmo` / `transportRepair`; -1
    // for a vehicle without that cargo.
    r.binary("setFuelCargo", OBJ, NUM, NOTHING, |ctx, a, b| {
        set_supply(ctx, &a, &b, "transportFuel", |s| &mut s.fuel_cargo)
    });
    r.binary("setAmmoCargo", OBJ, NUM, NOTHING, |ctx, a, b| {
        set_supply(ctx, &a, &b, "transportAmmo", |s| &mut s.ammo_cargo)
    });
    r.binary("setRepairCargo", OBJ, NUM, NOTHING, |ctx, a, b| {
        set_supply(ctx, &a, &b, "transportRepair", |s| &mut s.repair_cargo)
    });
    r.unary("getFuelCargo", OBJ, NUM, |ctx, a| {
        Ok(get_supply(ctx, &a, "transportFuel", |s| s.fuel_cargo))
    });
    r.unary("getAmmoCargo", OBJ, NUM, |ctx, a| {
        Ok(get_supply(ctx, &a, "transportAmmo", |s| s.ammo_cargo))
    });
    r.unary("getRepairCargo", OBJ, NUM, |ctx, a| {
        Ok(get_supply(ctx, &a, "transportRepair", |s| s.repair_cargo))
    });

    // 0x8b8550 / 0x8b8590 → 0x8b85d0: `[index or selection, texture]`.
    r.binary("setObjectTexture", OBJ, ARR, NOTHING, |ctx, a, b| {
        set_texture(ctx, &a, &b, false)
    });
    r.binary("setObjectTextureGlobal", OBJ, ARR, NOTHING, |ctx, a, b| {
        set_texture(ctx, &a, &b, false)
    });
    r.binary("setObjectMaterial", OBJ, ARR, NOTHING, |ctx, a, b| {
        set_texture(ctx, &a, &b, true)
    });
    r.binary("setObjectMaterialGlobal", OBJ, ARR, NOTHING, |ctx, a, b| {
        set_texture(ctx, &a, &b, true)
    });
    // 0x4ad560 / 0x4ac4d0: one entry per hidden selection.
    r.unary("getObjectTextures", OBJ, ARR, |ctx, a| {
        Ok(get_textures(ctx, &a, false))
    });
    r.unary("getObjectMaterials", OBJ, ARR, |ctx, a| {
        Ok(get_textures(ctx, &a, true))
    });

    // 0x536850: Transport only; `true` 2, `false` 1, a number rounded (not clamped).
    r.binary("lock", OBJ, BOOL.union(NUM), NOTHING, |ctx, a, b| {
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            let lock = match b {
                Value::Bool(v) => 1 + i32::from(v),
                Value::Number(n) => n.round_ties_even() as i32,
                _ => 1,
            };
            state(ctx, id).lock = Some(lock);
        }
        Ok(Value::Nothing)
    });
    // 0x570440: "UNLOCKED" 0, "DEFAULT" 1, "LOCKED" 2, "LOCKEDPLAYER" 3.
    r.binary("setVehicleLock", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        let lock = ["UNLOCKED", "DEFAULT", "LOCKED", "LOCKEDPLAYER"]
            .iter()
            .position(|n| n.eq_ignore_ascii_case(&name))
            .ok_or_else(|| bad_enum(&name))?;
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            state(ctx, id).lock = Some(lock as i32);
        }
        Ok(Value::Nothing)
    });
    // 0x537110: -1 for anything that is not a Transport.
    r.unary("locked", OBJ, NUM, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Number(
            entity(w, &a, EntityClass::Transport).map_or(-1, |id| {
                w.object_state(id).and_then(|s| s.lock).unwrap_or(1)
            }) as f32,
        ))
    });
    r.binary("lockDriver", OBJ, BOOL, NOTHING, |ctx, a, b| {
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            state(ctx, id).driver_locked = b.as_bool().unwrap_or(false);
        }
        Ok(Value::Nothing)
    });
    r.unary("lockedDriver", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            entity(w, &a, EntityClass::Transport)
                .is_some_and(|id| w.object_state(id).is_some_and(|s| s.driver_locked)),
        ))
    });
    r.binary("lockCargo", OBJ, BOOL, NOTHING, |ctx, a, b| {
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            let s = state(ctx, id);
            s.cargo_locked = b.as_bool().unwrap_or(false);
            s.cargo_seat_locks.clear();
        }
        Ok(Value::Nothing)
    });
    r.binary("lockCargo", OBJ, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if args.len() < 2 {
            return Err(dim(args.len(), 2));
        }
        let index = args[0].as_number().unwrap_or(-1.0) as i32;
        let locked = args[1].as_bool().unwrap_or(false);
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            state(ctx, id).cargo_seat_locks.push((index, locked));
        }
        Ok(Value::Nothing)
    });
    r.binary("lockedCargo", OBJ, NUM, BOOL, |ctx, a, b| {
        let w = ctx.host.world();
        let index = b.as_number().unwrap_or(-1.0) as i32;
        Ok(Value::Bool(
            entity(w, &a, EntityClass::Transport).is_some_and(|id| {
                w.object_state(id)
                    .is_some_and(|s| s.cargo_seat_locked(index))
            }),
        ))
    });
    // 0x569c30 / 0x56cca0: the engine flag (an empty tank does not stop `engineOn`).
    r.binary("engineOn", OBJ, BOOL, NOTHING, |ctx, a, b| {
        if let Some(id) = entity(ctx.host.world(), &a, EntityClass::Transport) {
            state(ctx, id).engine_on = b.as_bool().unwrap_or(false);
        }
        Ok(Value::Nothing)
    });
    r.unary("isEngineOn", OBJ, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            entity(w, &a, EntityClass::Transport)
                .is_some_and(|id| w.object_state(id).is_some_and(|s| s.engine_on)),
        ))
    });
    // 0x53c0b0: the flight height of air AI; no air AI yet _(stub)_.
    r.binary("flyInHeight", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("flyInHeight", OBJ, ARR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
}

fn set_supply<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    b: &Value,
    config_name: &str,
    field: fn(&mut ObjectState) -> &mut Option<f32>,
) -> Result<Value, SqfError> {
    if let Some(id) = entity(ctx.host.world(), a, EntityClass::EntityAi) {
        if config_number(ctx.host, id, config_name) > 0.0 {
            *field(state(ctx, id)) = Some(b.as_number().unwrap_or(0.0).clamp(0.0, 1.0));
        }
    }
    Ok(Value::Nothing)
}

fn get_supply<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    config_name: &str,
    field: fn(&ObjectState) -> Option<f32>,
) -> Value {
    let Some(id) = entity(ctx.host.world(), a, EntityClass::EntityAi) else {
        return Value::Number(-1.0);
    };
    if config_number(ctx.host, id, config_name) <= 0.0 {
        return Value::Number(-1.0);
    }
    let v = ctx
        .host
        .world()
        .object_state(id)
        .and_then(field)
        .unwrap_or(1.0);
    Value::Number(v)
}

/// The cargo of a vehicle or box: its config contents on first use.
fn with_cargo<H: WorldHost, R>(
    ctx: &mut Ctx<'_, H>,
    id: EntityId,
    f: impl FnOnce(&ConfigTree, &mut Cargo) -> R,
) -> R {
    let config = ctx.host.types().config_arc();
    let type_name = ctx
        .host
        .world()
        .entity(id)
        .map(|e| e.type_name().to_owned())
        .unwrap_or_default();
    let s = state(ctx, id);
    if s.cargo.is_none() {
        s.cargo = Some(default_cargo(&config, &type_name));
    }
    f(&config, s.cargo.as_mut().expect("set"))
}

/// `TransportItems` (`name`), `TransportMagazines` (`magazine`), `TransportWeapons` (`weapon`),
/// `TransportBackpacks` (`backpack`).
fn default_cargo(config: &ConfigTree, type_name: &str) -> Cargo {
    let class = config.root().get("CfgVehicles").get(type_name);
    let mut cargo = Cargo::default();
    for (list, key) in [
        ("TransportItems", "name"),
        ("TransportMagazines", "magazine"),
        ("TransportWeapons", "weapon"),
        ("TransportBackpacks", "backpack"),
    ] {
        for entry in class.get(list).entries().iter().filter(|e| e.is_class()) {
            let name = entry.get(key).text();
            let count = entry.get("count");
            let count = if count.is_number() {
                count.number().max(0.0) as usize
            } else {
                0
            };
            let Some(info) = classify(config, &name) else {
                continue;
            };
            for _ in 0..count {
                match list {
                    "TransportMagazines" => {
                        if let ItemKind::Magazine { count } = info.kind {
                            cargo.magazines.push(make_magazine(&info, count));
                        }
                    }
                    "TransportWeapons" => cargo.weapons.push(info.class.clone()),
                    "TransportBackpacks" => cargo.backpacks.push(info.class.clone()),
                    _ => cargo.items.push(info.class.clone()),
                }
            }
        }
    }
    cargo
}

/// A cargo holder: a local EntityAI that is not a Man (units keep their gear in containers).
fn cargo_holder(world: &World, value: &Value) -> Option<EntityId> {
    entity(world, value, EntityClass::EntityAi).filter(|&id| {
        world
            .entity(id)
            .is_some_and(|e| !e.class().is_kind_of(EntityClass::Man))
    })
}

#[derive(Clone, Copy)]
enum CargoKind {
    Item,
    Magazine,
    Weapon,
    Backpack,
}

fn register_cargo<H: WorldHost>(r: &mut Registry<H>) {
    // 0x83d7b0 & co.: local EntityAI only; units are not affected.
    for (name, kind) in [
        ("clearItemCargo", CargoKind::Item),
        ("clearItemCargoGlobal", CargoKind::Item),
        ("clearMagazineCargo", CargoKind::Magazine),
        ("clearMagazineCargoGlobal", CargoKind::Magazine),
        ("clearWeaponCargo", CargoKind::Weapon),
        ("clearWeaponCargoGlobal", CargoKind::Weapon),
        ("clearBackpackCargo", CargoKind::Backpack),
        ("clearBackpackCargoGlobal", CargoKind::Backpack),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, SqfError> = match kind {
            CargoKind::Item => |ctx, a| clear_cargo(ctx, &a, CargoKind::Item),
            CargoKind::Magazine => |ctx, a| clear_cargo(ctx, &a, CargoKind::Magazine),
            CargoKind::Weapon => |ctx, a| clear_cargo(ctx, &a, CargoKind::Weapon),
            CargoKind::Backpack => |ctx, a| clear_cargo(ctx, &a, CargoKind::Backpack),
        };
        r.unary(name, OBJ, NOTHING, f);
    }
    // 0x8391e0 & co.: `[class, count]` (exactly two... at least two elements), the count
    // rounded; nothing for a count below 1. No capacity check.
    for (name, kind) in [
        ("addItemCargo", CargoKind::Item),
        ("addItemCargoGlobal", CargoKind::Item),
        ("addMagazineCargo", CargoKind::Magazine),
        ("addMagazineCargoGlobal", CargoKind::Magazine),
        ("addWeaponCargo", CargoKind::Weapon),
        ("addWeaponCargoGlobal", CargoKind::Weapon),
        ("addBackpackCargo", CargoKind::Backpack),
        ("addBackpackCargoGlobal", CargoKind::Backpack),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, SqfError> = match kind {
            CargoKind::Item => |ctx, a, b| add_cargo(ctx, &a, &b, CargoKind::Item),
            CargoKind::Magazine => |ctx, a, b| add_cargo(ctx, &a, &b, CargoKind::Magazine),
            CargoKind::Weapon => |ctx, a, b| add_cargo(ctx, &a, &b, CargoKind::Weapon),
            CargoKind::Backpack => |ctx, a, b| add_cargo(ctx, &a, &b, CargoKind::Backpack),
        };
        r.binary(name, OBJ, ARR, NOTHING, f);
    }
    for (name, kind) in [
        ("itemCargo", CargoKind::Item),
        ("magazineCargo", CargoKind::Magazine),
        ("weaponCargo", CargoKind::Weapon),
        ("backpackCargo", CargoKind::Backpack),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, SqfError> = match kind {
            CargoKind::Item => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Item, false)),
            CargoKind::Magazine => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Magazine, false)),
            CargoKind::Weapon => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Weapon, false)),
            CargoKind::Backpack => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Backpack, false)),
        };
        r.unary(name, OBJ, ARR, f);
    }
    for (name, kind) in [
        ("getItemCargo", CargoKind::Item),
        ("getMagazineCargo", CargoKind::Magazine),
        ("getWeaponCargo", CargoKind::Weapon),
        ("getBackpackCargo", CargoKind::Backpack),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, SqfError> = match kind {
            CargoKind::Item => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Item, true)),
            CargoKind::Magazine => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Magazine, true)),
            CargoKind::Weapon => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Weapon, true)),
            CargoKind::Backpack => |ctx, a| Ok(list_cargo(ctx, &a, CargoKind::Backpack, true)),
        };
        r.unary(name, OBJ, ARR, f);
    }
}

fn clear_cargo<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    kind: CargoKind,
) -> Result<Value, SqfError> {
    let w = ctx.host.world();
    if let Some(id) = cargo_holder(w, a).filter(|&id| w.entity(id).is_some_and(|e| e.is_local())) {
        with_cargo(ctx, id, |_, c| match kind {
            CargoKind::Item => c.items.clear(),
            CargoKind::Magazine => c.magazines.clear(),
            CargoKind::Weapon => c.weapons.clear(),
            CargoKind::Backpack => c.backpacks.clear(),
        });
    }
    Ok(Value::Nothing)
}

fn add_cargo<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    b: &Value,
    kind: CargoKind,
) -> Result<Value, SqfError> {
    let args = items(b);
    // Fewer than two elements adds nothing and runs on (oracle: `addItemCargo ["FirstAidKit"]`).
    if args.len() < 2 {
        return Ok(Value::Nothing);
    }
    let name = text(&args[0]);
    let count = args[1].as_number().unwrap_or(0.0).round_ties_even();
    let Some(id) = cargo_holder(ctx.host.world(), a) else {
        return Ok(Value::Nothing);
    };
    if count < 1.0 {
        return Ok(Value::Nothing);
    }
    let count = count as usize;
    with_cargo(ctx, id, |config, c| {
        let Some(info) = classify(config, &name) else {
            return;
        };
        for _ in 0..count {
            match (kind, &info.kind) {
                // `addItemCargo` routes magazines and weapons to their own cargo.
                (CargoKind::Item | CargoKind::Magazine, ItemKind::Magazine { count }) => {
                    c.magazines.push(make_magazine(&info, *count))
                }
                (CargoKind::Item | CargoKind::Weapon, ItemKind::Weapon(_)) => {
                    c.weapons.push(info.class.clone())
                }
                (CargoKind::Item, ItemKind::Item(_) | ItemKind::Goggles) => {
                    c.items.push(info.class.clone())
                }
                // `addWeaponCargo` takes any CfgWeapons class.
                (CargoKind::Weapon, ItemKind::Item(_) | ItemKind::PseudoWeapon) => {
                    c.weapons.push(info.class.clone())
                }
                (CargoKind::Backpack, ItemKind::Backpack) => c.backpacks.push(info.class.clone()),
                _ => {}
            }
        }
    });
    Ok(Value::Nothing)
}

/// `itemCargo` & co. list every entry; `getItemCargo` & co. `[[names], [counts]]` grouped in
/// first-seen order. A unit lists its containers' contents (`get...` gives `[[],[]]`).
fn list_cargo<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    kind: CargoKind,
    grouped: bool,
) -> Value {
    let empty = || {
        if grouped {
            Value::array([Value::array([]), Value::array([])])
        } else {
            Value::array([])
        }
    };
    let w = ctx.host.world();
    let names: Vec<String> = if let Some(id) = cargo_holder(w, a) {
        with_cargo(ctx, id, |_, c| match kind {
            CargoKind::Item => c.items.clone(),
            CargoKind::Magazine => c.magazines.iter().map(|m| m.class.clone()).collect(),
            CargoKind::Weapon => c.weapons.clone(),
            CargoKind::Backpack => c.backpacks.clone(),
        })
    } else if !grouped {
        match entity(w, a, EntityClass::Man).and_then(|id| w.gear(id)) {
            Some(g) => match kind {
                CargoKind::Item => g
                    .containers()
                    .flat_map(|c| c.items.iter().map(|i| i.class.clone()))
                    .collect(),
                CargoKind::Magazine => g
                    .containers()
                    .flat_map(|c| c.magazines.iter().map(|m| m.class.clone()))
                    .collect(),
                CargoKind::Weapon => g
                    .containers()
                    .flat_map(|c| c.weapons.iter().map(|w| w.class.clone()))
                    .collect(),
                CargoKind::Backpack => Vec::new(),
            },
            None => return empty(),
        }
    } else {
        return empty();
    };
    if !grouped {
        return Value::array(names.into_iter().map(Value::string));
    }
    let mut keys: Vec<String> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    for n in names {
        match keys.iter().position(|k| k.eq_ignore_ascii_case(&n)) {
            Some(i) => counts[i] += 1,
            None => {
                keys.push(n);
                counts.push(1);
            }
        }
    }
    Value::array([
        Value::array(keys.into_iter().map(Value::string)),
        Value::array(counts.into_iter().map(|c| Value::Number(c as f32))),
    ])
}
