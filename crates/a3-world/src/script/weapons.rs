//! Weapon commands: the unit's weapons and magazines (`addWeapon`, `addMagazine(s)`,
//! `removeWeapon`, `weapons`, `magazines`, `primaryWeapon`, `currentWeapon`, `currentMuzzle`,
//! `currentWeaponMode`, `currentMagazine`, `ammo`, `setAmmo`, `selectWeapon`, `reload`) and
//! firing (`fire`, `forceWeaponFire`). See [`crate::Loadout`] and `docs/re/sim-weapons.md`.
//!
//! The Loadout commands need a local unit (AL). `forceWeaponFire` fires at once and raises the
//! `Fired` handlers before the command returns; `fire` only leaves a request, so its round goes in
//! a later simulate (`docs/re/sim-weapons.md` §3.3, §6).

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, object_arg};
use crate::{EntityId, ObjectRef};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AL EG.
    r.binary("addWeapon", OBJ, STR.union(ARR), NOTHING, |ctx, a, b| {
        if let (Some(unit), Some(weapon)) = (local_unit(ctx, &a), text_or_first(&b)) {
            ensure_config(ctx);
            let _ = ctx.host.world_mut().add_weapon(unit, &weapon);
        }
        Ok(Value::Nothing)
    });
    r.binary("removeWeapon", OBJ, STR.union(ARR), NOTHING, |ctx, a, b| {
        if let (Some(unit), Some(weapon)) = (local_unit(ctx, &a), text_or_first(&b)) {
            ctx.host.world_mut().remove_weapon(unit, &weapon);
        }
        Ok(Value::Nothing)
    });
    // `unit addMagazine "class"` / `unit addMagazine ["class", ammo]`.
    r.binary("addMagazine", OBJ, STR, NOTHING, |ctx, a, b| {
        if let (Some(unit), Some(mag)) = (local_unit(ctx, &a), b.as_str()) {
            let mag = mag.to_owned();
            ensure_config(ctx);
            let _ = ctx.host.world_mut().add_magazine(unit, &mag, None);
        }
        Ok(Value::Nothing)
    });
    r.binary("addMagazine", OBJ, ARR, NOTHING, |ctx, a, b| {
        let (mag, count) = name_and_number(&b);
        if let (Some(unit), Some(mag)) = (local_unit(ctx, &a), mag) {
            ensure_config(ctx);
            // `None` (a missing or nil count) fills the magazine; so does a negative one or one
            // above its `count` (`0x83a4a0`).
            let ammo = count.map(|c| c as f32);
            let _ = ctx.host.world_mut().add_magazine(unit, &mag, ammo);
        }
        Ok(Value::Nothing)
    });
    // `unit addMagazines ["class", count]`: `count` full magazines.
    r.binary("addMagazines", OBJ, ARR, NOTHING, |ctx, a, b| {
        let (mag, count) = name_and_number(&b);
        if let (Some(unit), Some(mag)) = (local_unit(ctx, &a), mag) {
            ensure_config(ctx);
            for _ in 0..count.unwrap_or(0.0).max(0.0) as u32 {
                if ctx.host.world_mut().add_magazine(unit, &mag, None).is_err() {
                    break;
                }
            }
        }
        Ok(Value::Nothing)
    });

    r.unary("weapons", OBJ, ARR, |ctx, a| {
        let list = unit(ctx, &a).map_or_else(Vec::new, |u| ctx.host.world().weapons_of(u));
        Ok(strings(list))
    });
    r.unary("magazines", OBJ, ARR, |ctx, a| {
        let list = unit(ctx, &a).map_or_else(Vec::new, |u| ctx.host.world().magazines_of(u));
        Ok(strings(list))
    });
    r.unary("primaryWeapon", OBJ, STR, |ctx, a| {
        Ok(Value::string(unit(ctx, &a).map_or_else(String::new, |u| {
            ctx.host.world().primary_weapon(u)
        })))
    });
    r.unary("currentWeapon", OBJ, STR, |ctx, a| {
        Ok(Value::string(unit(ctx, &a).map_or_else(String::new, |u| {
            ctx.host.world().current_weapon(u)
        })))
    });
    r.unary("currentMuzzle", OBJ, STR, |ctx, a| {
        Ok(Value::string(unit(ctx, &a).map_or_else(String::new, |u| {
            ctx.host.world().current_muzzle(u)
        })))
    });
    r.unary("currentWeaponMode", OBJ, STR, |ctx, a| {
        Ok(Value::string(unit(ctx, &a).map_or_else(String::new, |u| {
            ctx.host.world().current_weapon_mode(u)
        })))
    });
    r.unary("currentMagazine", OBJ, STR, |ctx, a| {
        Ok(Value::string(unit(ctx, &a).map_or_else(String::new, |u| {
            ctx.host.world().current_magazine(u)
        })))
    });
    r.binary("ammo", OBJ, STR, NUM, |ctx, a, b| {
        let muzzle = b.as_str().unwrap_or_default().to_owned();
        let n = unit(ctx, &a).map_or(0, |u| ctx.host.world().ammo_in(u, &muzzle));
        Ok(Value::Number(n as f32))
    });
    // AL EG: `unit setAmmo [weapon, count]`. `count` is rounded; a negative one fills the
    // magazine, as does one above its capacity (`0x1411176b0`).
    r.binary("setAmmo", OBJ, ARR, NOTHING, |ctx, a, b| {
        let (muzzle, count) = name_and_number(&b);
        if let (Some(unit), Some(muzzle), Some(count)) = (local_unit(ctx, &a), muzzle, count) {
            let count = count.round();
            let ammo = (count >= 0.0).then_some(count as u32);
            ctx.host.world_mut().set_ammo(unit, &muzzle, ammo);
        }
        Ok(Value::Nothing)
    });
    r.binary("selectWeapon", OBJ, STR, NOTHING, |ctx, a, b| {
        if let (Some(unit), Some(muzzle)) = (local_unit(ctx, &a), b.as_str()) {
            let muzzle = muzzle.to_owned();
            ctx.host.world_mut().select_weapon(unit, &muzzle);
        }
        Ok(Value::Nothing)
    });
    // `unit selectWeapon [weapon, muzzle, mode]`: true when the unit has them.
    r.binary("selectWeapon", OBJ, ARR, BOOL, |ctx, a, b| {
        let parts = text_items(&b);
        let Some(unit) = local_unit(ctx, &a) else {
            return Ok(Value::Bool(false));
        };
        let world = ctx.host.world_mut();
        let muzzle = parts
            .get(1)
            .filter(|m| !m.is_empty())
            .or(parts.first())
            .cloned()
            .unwrap_or_default();
        let mut ok = world.select_weapon(unit, &muzzle);
        if ok {
            if let Some(mode) = parts.get(2).filter(|m| !m.is_empty()) {
                ok = world.select_mode(unit, mode);
            }
        }
        Ok(Value::Bool(ok))
    });
    r.unary("reload", OBJ, NOTHING, |ctx, a| {
        if let Some(unit) = local_unit(ctx, &a) {
            ctx.host.world_mut().reload(unit);
        }
        Ok(Value::Nothing)
    });

    // `unit fire muzzle` / `unit fire [muzzle, mode(, magazine)]`: a **request**
    // (`WeaponsState+0x30`, `0x1405281a0`). It goes in a later simulate, from the selected
    // muzzle and only when the weapon is ready and aimed.
    r.binary("fire", OBJ, STR, NOTHING, |ctx, a, b| {
        let muzzle = b.as_str().map(str::to_owned);
        request_fire(ctx, &a, muzzle, None);
        Ok(Value::Nothing)
    });
    r.binary("fire", OBJ, ARR, NOTHING, |ctx, a, b| {
        let parts = text_items(&b);
        request_fire(ctx, &a, parts.first().cloned(), parts.get(1).cloned());
        Ok(Value::Nothing)
    });
    // `unit forceWeaponFire [muzzle, mode]`: fires at once, like the engine's `FireWeapon`.
    r.binary("forceWeaponFire", OBJ, ARR, NOTHING, |ctx, a, b| {
        let parts = text_items(&b);
        fire(ctx, &a, parts.first().cloned(), parts.get(1).cloned());
        Ok(Value::Nothing)
    });
}

/// The `fire` command: leaves a request on the unit.
fn request_fire<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    muzzle: Option<String>,
    mode: Option<String>,
) {
    let Some(unit) = local_unit(ctx, a) else {
        return;
    };
    ctx.host
        .world_mut()
        .request_fire(unit, muzzle.as_deref(), mode.as_deref());
}

fn fire<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    muzzle: Option<String>,
    mode: Option<String>,
) {
    let Some(unit) = local_unit(ctx, a) else {
        return;
    };
    let fired = ctx
        .host
        .world_mut()
        .fire_weapon(unit, muzzle.as_deref(), mode.as_deref());
    if matches!(fired, Ok(Some(_))) {
        super::handlers::dispatch_events_in(ctx);
    }
}

/// Installs the script host's config as the World's weapons config when it has none, so the
/// Loadout commands work in any World a script runs in.
pub(super) fn ensure_config<H: WorldHost>(ctx: &mut Ctx<'_, H>) {
    if ctx.host.world().config().is_none() {
        let config = ctx.host.types().config_arc();
        ctx.host.world_mut().set_config(config);
    }
}

/// The Entity a value names.
fn unit<H: WorldHost>(ctx: &mut Ctx<'_, H>, a: &Value) -> Option<EntityId> {
    match object_arg(ctx.host.world(), a)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

/// The Entity a value names, when it is local (an AL command does nothing on a remote unit).
fn local_unit<H: WorldHost>(ctx: &mut Ctx<'_, H>, a: &Value) -> Option<EntityId> {
    let id = unit(ctx, a)?;
    ctx.host
        .world()
        .entity(id)
        .filter(|e| e.is_local())
        .map(|e| e.id())
}

fn strings(list: Vec<String>) -> Value {
    Value::array(list.into_iter().map(Value::string))
}

/// A string, or the first element of an array.
fn text_or_first(v: &Value) -> Option<String> {
    match v.as_str() {
        Some(s) => Some(s.to_owned()),
        None => text_items(v).into_iter().next(),
    }
}

/// The string elements of an array (others as "").
fn text_items(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.borrow()
                .iter()
                .map(|x| x.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `[name, number]`.
fn name_and_number(v: &Value) -> (Option<String>, Option<f64>) {
    let Some(a) = v.as_array() else {
        return (None, None);
    };
    let a = a.borrow();
    (
        a.first().and_then(Value::as_str).map(str::to_owned),
        a.get(1).and_then(Value::as_number).map(f64::from),
    )
}
