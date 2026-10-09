//! Unit inventory commands: containers, worn items, assigned items, attachments and the
//! magazines and items inside containers. From the decompiled handlers and the oracle probes
//! (`tools/oracle/probes/97_inventory_vr.probes`); see `docs/re/sqf-inventory.md`.
//!
//! The weapons and magazines themselves are the [`Loadout`](crate::Loadout)'s
//! (`crate::script::weapons`, `crate::loadout`): an equipped weapon with the magazine in its
//! muzzle, and a carried but unloaded magazine in a container ([`Gear`]). Commands on anything
//! that is not a Man return `""` / `[]` / nothing. RPT-only warnings of the original ("Inventory
//! item with given name: [%s] not found", ...) are not reproduced.

use a3_config::ConfigTree;
use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, SqfError, Value};

use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, object_arg};
use crate::gear::{make_container, stored, wear};
use crate::inventory::{
    ANY_CONTAINER, ContainerSlot, Gear, ItemInfo, ItemKind, LinkSlot, WeaponSlot, classify, slot,
};
use crate::loadout::Loadout;
use crate::{EntityClass, EntityId, ObjectRef, World};

fn string_list(items: impl IntoIterator<Item = String>) -> Value {
    Value::array(items.into_iter().map(Value::string))
}

fn dim(got: usize, expected: usize) -> SqfError {
    SqfError::generic(format!("{got} elements provided, {expected} expected"))
}

/// The Man a value refers to.
fn man(world: &World, value: &Value) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => world
            .entity(id)
            .filter(|e| e.class().is_kind_of(EntityClass::Man))
            .map(|_| id),
        ObjectRef::Static(_) => None,
    }
}

/// Gives a unit created before the World had a config its loadout now ([`World::arm_from_config`]
/// arms every later one at creation).
fn seed_loadout<H: WorldHost>(ctx: &mut Ctx<'_, H>, id: EntityId) {
    super::weapons::ensure_config(ctx);
    if ctx.host.world().gear(id).is_none_or(|g| g.initialized) {
        return;
    }
    let config = ctx.host.types().config_arc();
    let Some(name) = ctx
        .host
        .world()
        .entity(id)
        .map(|e| e.type_name().to_owned())
    else {
        return;
    };
    let start = crate::gear::default_loadout(&config, &name);
    let weapons: Vec<String> = start
        .weapons
        .iter()
        .map(|w| w.class.clone())
        .chain(start.gear.pseudo_weapons.iter().cloned())
        .collect();
    if let Some(gear) = ctx.host.world_mut().gear_mut(id) {
        *gear = start.gear;
        gear.initialized = true;
    }
    for weapon in weapons {
        let _ = ctx.host.world_mut().add_weapon(id, &weapon);
    }
}

/// Runs `f` on a unit's gear and Loadout.
fn with_unit<H: WorldHost, R>(
    ctx: &mut Ctx<'_, H>,
    id: EntityId,
    f: impl FnOnce(&ConfigTree, &mut Gear, &mut Loadout) -> R,
) -> Option<R> {
    seed_loadout(ctx, id);
    let config = ctx.host.types().config_arc();
    let (loadout, gear) = ctx.host.world_mut().loadout_and_gear_mut(id)?;
    Some(f(&config, gear, loadout))
}

/// A query on a Man's gear (containers, worn items, stored weapons and magazines); `default` for
/// anything else.
fn query<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    default: Value,
    f: impl FnOnce(&ConfigTree, &Gear) -> Value,
) -> Value {
    match man(ctx.host.world(), unit) {
        Some(id) => with_unit(ctx, id, |c, g, _| f(c, g)).unwrap_or(default),
        None => default,
    }
}

/// A change to a local Man's gear (the original forwards a remote unit's change to its owner).
fn change<H: WorldHost, R: Default>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    f: impl FnOnce(&ConfigTree, &mut Gear) -> R,
) -> R {
    let w = ctx.host.world();
    match man(w, unit) {
        Some(id) if w.entity(id).is_some_and(|e| e.is_local()) => {
            with_unit(ctx, id, |c, g, _| f(c, g)).unwrap_or_default()
        }
        _ => R::default(),
    }
}

/// A query on a Man that reads the Loadout too (equipped weapons, attachments, magazines).
fn unit_query<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    default: Value,
    f: impl FnOnce(&ConfigTree, &Gear, &Loadout) -> Value,
) -> Value {
    match man(ctx.host.world(), unit) {
        Some(id) => with_unit(ctx, id, |c, g, l| f(c, g, l)).unwrap_or(default),
        None => default,
    }
}

/// A change to a local Man's gear and Loadout.
fn unit_change<H: WorldHost, R: Default>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    f: impl FnOnce(&ConfigTree, &mut Gear, &mut Loadout) -> R,
) -> R {
    let w = ctx.host.world();
    match man(w, unit) {
        Some(id) if w.entity(id).is_some_and(|e| e.is_local()) => {
            with_unit(ctx, id, f).unwrap_or_default()
        }
        _ => R::default(),
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    register_getters(r);
    register_worn(r);
    register_links(r);
    register_weapons(r);
    register_magazines(r);
    register_items(r);
}

/// The equipped weapon of a slot (the Loadout's).
fn equipped(loadout: &Loadout, slot: WeaponSlot) -> Option<&crate::loadout::WeaponSlot> {
    loadout.weapons.iter().find(|w| w.slot == slot)
}

fn weapon_items(loadout: &Loadout, slot: WeaponSlot) -> Value {
    let att = equipped(loadout, slot)
        .map(|w| w.attachments.clone())
        .unwrap_or_default();
    string_list(att)
}

fn weapon_magazine(loadout: &Loadout, slot: WeaponSlot) -> Value {
    string_list(
        equipped(loadout, slot)
            .and_then(|w| w.muzzles.first())
            .and_then(|m| m.magazine.as_ref())
            .map(|m| m.name().to_owned()),
    )
}

/// Every worn item's, weapon's and attachment's mass, for `load`.
fn worn_mass(config: &ConfigTree, gear: &Gear, loadout: &Loadout) -> f32 {
    let mass = |c: &str| classify(config, c).map_or(0.0, |i| i.mass);
    let equipped: f32 = loadout
        .weapons
        .iter()
        .map(|w| {
            mass(&w.kind.name)
                + w.muzzles
                    .iter()
                    .filter_map(|m| m.magazine.as_ref())
                    .map(|m| mass(m.name()))
                    .sum::<f32>()
        })
        .sum();
    let attachments: f32 = loadout
        .weapons
        .iter()
        .flat_map(|w| w.attachments.iter())
        .filter(|a| !a.is_empty())
        .map(|a| mass(a))
        .sum();
    gear.total_mass(mass) + equipped + attachments
}

fn register_getters<H: WorldHost>(r: &mut Registry<H>) {
    // `weapons` and `primaryWeapon` are the Loadout's (`script::weapons`): the equipped weapons in
    // the order they were added, then the weapons stored in the containers.
    r.unary("secondaryWeapon", OBJ, STR, |ctx, a| {
        slot_name(ctx, a, WeaponSlot::Secondary)
    });
    r.unary("handgunWeapon", OBJ, STR, |ctx, a| {
        slot_name(ctx, a, WeaponSlot::Handgun)
    });
    r.unary("binocular", OBJ, STR, |ctx, a| {
        slot_name(ctx, a, WeaponSlot::Binocular)
    });
    r.unary("primaryWeaponItems", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_items(l, WeaponSlot::Primary)
        }))
    });
    r.unary("secondaryWeaponItems", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_items(l, WeaponSlot::Secondary)
        }))
    });
    r.unary("handgunItems", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_items(l, WeaponSlot::Handgun)
        }))
    });
    r.unary("primaryWeaponMagazine", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_magazine(l, WeaponSlot::Primary)
        }))
    });
    r.unary("secondaryWeaponMagazine", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_magazine(l, WeaponSlot::Secondary)
        }))
    });
    r.unary("handgunMagazine", OBJ, ARR, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::array([]), |_, _, l| {
            weapon_magazine(l, WeaponSlot::Handgun)
        }))
    });
    // Worn containers and items: the class, "" without one.
    r.unary("uniform", OBJ, STR, |ctx, a| {
        Ok(query(ctx, &a, Value::from(""), |_, g| {
            Value::string(
                g.uniform
                    .as_ref()
                    .map_or(String::new(), |c| c.class.clone()),
            )
        }))
    });
    r.unary("vest", OBJ, STR, |ctx, a| {
        Ok(query(ctx, &a, Value::from(""), |_, g| {
            Value::string(g.vest.as_ref().map_or(String::new(), |c| c.class.clone()))
        }))
    });
    r.unary("backpack", OBJ, STR, |ctx, a| {
        Ok(query(ctx, &a, Value::from(""), |_, g| {
            Value::string(
                g.backpack
                    .as_ref()
                    .map_or(String::new(), |c| c.class.clone()),
            )
        }))
    });
    r.unary("headgear", OBJ, STR, |ctx, a| {
        Ok(query(ctx, &a, Value::from(""), |_, g| {
            Value::string(g.headgear.clone().unwrap_or_default())
        }))
    });
    r.unary("goggles", OBJ, STR, |ctx, a| {
        Ok(query(ctx, &a, Value::from(""), |_, g| {
            Value::string(g.goggles.clone().unwrap_or_default())
        }))
    });
    // 0x83cc40 → 0x82c400: per container (uniform, vest, backpack) its items and weapons.
    r.unary("items", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            string_list(g.containers().flat_map(|c| {
                c.items
                    .iter()
                    .map(|i| i.class.clone())
                    .chain(c.weapons.iter().map(|w| w.class.clone()))
                    .collect::<Vec<_>>()
            }))
        }))
    });
    // `magazines` is the Loadout's (`script::weapons`): the magazines in the containers that are
    // not empty, uniform first; a loaded magazine is in a muzzle, not a container.
    // 0x83e090: `[class, ammo]` of the stored magazines; empty ones only with
    // `[unit, true]`.
    r.unary("magazinesAmmo", OBJ, ARR, |ctx, a| {
        Ok(magazines_ammo(ctx, &a, false))
    });
    r.unary("magazinesAmmo", ARR, ARR, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let unit = items.first().cloned().unwrap_or(Value::Nil);
        let empty = items.get(1).and_then(Value::as_bool).unwrap_or(false);
        Ok(magazines_ammo(ctx, &unit, empty))
    });
    r.unary("uniformItems", OBJ, ARR, |ctx, a| {
        container_items(ctx, a, ContainerSlot::Uniform)
    });
    r.unary("vestItems", OBJ, ARR, |ctx, a| {
        container_items(ctx, a, ContainerSlot::Vest)
    });
    r.unary("backpackItems", OBJ, ARR, |ctx, a| {
        container_items(ctx, a, ContainerSlot::Backpack)
    });
    // 0x83c530: map, compass, watch, radio, GPS, NVG; `[unit, goggles, headgear]` adds goggles
    // and headgear when the first flag is set — the original checks only that flag for both. An
    // array shorter than three elements gives `[]` (oracle: `assignedItems [unit]`), not a DIM
    // error.
    r.unary("assignedItems", OBJ, ARR, |ctx, a| {
        Ok(assigned(ctx, &a, false))
    });
    r.unary("assignedItems", ARR, ARR, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        if items.len() < 3 {
            return Ok(Value::array([]));
        }
        let worn = items[1].as_bool().unwrap_or(false);
        Ok(assigned(ctx, &items[0], worn))
    });
    // 0x5353e0: equipped weapons, stored weapons and pseudo weapons, any case.
    r.binary("hasWeapon", OBJ, STR, BOOL, |ctx, a, b| {
        let name = text(&b);
        Ok(unit_query(ctx, &a, Value::Bool(false), |_, g, l| {
            let eq = |c: &str| c.eq_ignore_ascii_case(&name);
            Value::Bool(
                l.weapons.iter().any(|w| eq(&w.kind.name))
                    || g.pseudo_weapons.iter().any(|w| eq(w))
                    || g.stored_weapons().any(|w| eq(&w.class)),
            )
        }))
    });
    // Loads: container contents over its capacity; the unit's total over its `maximumLoad`.
    r.unary("loadUniform", OBJ, NUM, |ctx, a| {
        container_load(ctx, a, ContainerSlot::Uniform)
    });
    r.unary("loadVest", OBJ, NUM, |ctx, a| {
        container_load(ctx, a, ContainerSlot::Vest)
    });
    r.unary("loadBackpack", OBJ, NUM, |ctx, a| {
        container_load(ctx, a, ContainerSlot::Backpack)
    });
    r.unary("loadAbs", OBJ, NUM, |ctx, a| {
        Ok(unit_query(ctx, &a, Value::Number(0.0), |c, g, l| {
            Value::Number(worn_mass(c, g, l))
        }))
    });
    r.unary("load", OBJ, NUM, |ctx, a| {
        let max = match man(ctx.host.world(), &a) {
            Some(id) => {
                let type_name = ctx
                    .host
                    .world()
                    .entity(id)
                    .map(|e| e.type_name().to_owned());
                let config = ctx.host.types().config_arc();
                let class = config
                    .root()
                    .get("CfgVehicles")
                    .get(&type_name.unwrap_or_default())
                    .get("maximumLoad");
                if class.is_number() {
                    class.number()
                } else {
                    1000.0
                }
            }
            None => 1000.0,
        };
        Ok(unit_query(ctx, &a, Value::Number(0.0), |c, g, l| {
            Value::Number(if max > 0.0 {
                worn_mass(c, g, l) / max
            } else {
                0.0
            })
        }))
    });
}

fn magazines_ammo<H: WorldHost>(ctx: &mut Ctx<'_, H>, unit: &Value, empty: bool) -> Value {
    query(ctx, unit, Value::array([]), |_, g| {
        Value::array(
            g.containers()
                .flat_map(|c| c.magazines.iter())
                .filter(|m| empty || m.ammo > 0)
                .map(|m| {
                    Value::array([Value::string(m.class.clone()), Value::Number(m.ammo as f32)])
                })
                .collect::<Vec<_>>(),
        )
    })
}

fn slot_name<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    slot: WeaponSlot,
) -> Result<Value, SqfError> {
    Ok(unit_query(ctx, &a, Value::from(""), |_, _, l| {
        Value::string(equipped(l, slot).map_or(String::new(), |w| w.kind.name.clone()))
    }))
}

fn container_items<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    slot: ContainerSlot,
) -> Result<Value, SqfError> {
    Ok(query(ctx, &a, Value::array([]), |_, g| {
        string_list(g.container(slot).map(|c| c.listing()).unwrap_or_default())
    }))
}

fn container_load<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    slot: ContainerSlot,
) -> Result<Value, SqfError> {
    Ok(query(ctx, &a, Value::Number(0.0), |_, g| {
        Value::Number(g.container(slot).map_or(0.0, |c| {
            if c.capacity > 0.0 {
                c.load() / c.capacity
            } else {
                0.0
            }
        }))
    }))
}

fn assigned<H: WorldHost>(ctx: &mut Ctx<'_, H>, unit: &Value, worn: bool) -> Value {
    query(ctx, unit, Value::array([]), |_, g| {
        let mut out: Vec<String> = LinkSlot::ALL
            .iter()
            .filter_map(|&s| g.link(s).map(str::to_owned))
            .collect();
        if worn {
            out.extend(g.goggles.clone());
            out.extend(g.headgear.clone());
        }
        string_list(out)
    })
}

/// Puts a worn item of `expected` type in its slot (`addHeadgear` & co., 0x843710 →
/// 0x8438c0): the slot is emptied first; an item of another type leaves it empty and the original
/// only logs "Tried to add inventory item with type 'X' into slot of type 'Y'" (oracle:
/// `addHeadgear "ItemMap"` runs on).
fn add_worn<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    name: &str,
    expected: i32,
    check_side: bool,
) -> Result<Value, SqfError> {
    let Some(id) = man(ctx.host.world(), unit) else {
        return Ok(Value::Nothing);
    };
    if !ctx.host.world().entity(id).is_some_and(|e| e.is_local()) {
        return Ok(Value::Nothing);
    }
    let config = ctx.host.types().config_arc();
    let Some(info) = classify(&config, name).filter(ItemInfo::is_inventory_item_or_backpack) else {
        return Ok(Value::Nothing);
    };
    let unit_side = ctx
        .host
        .world()
        .entity(id)
        .map(|e| e.entity_type().side())
        .unwrap_or(-1);
    with_unit(ctx, id, |config, gear, _| {
        clear_slot(gear, expected);
        if info.item_type() != expected {
            return;
        }
        if check_side && !uniform_allowed(config, &info.class, unit_side) {
            return;
        }
        wear(config, gear, &info);
    });
    Ok(Value::Nothing)
}

trait InventoryClass {
    fn is_inventory_item_or_backpack(&self) -> bool;
}

impl InventoryClass for ItemInfo {
    fn is_inventory_item_or_backpack(&self) -> bool {
        self.is_inventory_item() || self.kind == ItemKind::Backpack
    }
}

/// Empties the worn slot of a type.
fn clear_slot(gear: &mut Gear, item_type: i32) {
    match item_type {
        slot::HEADGEAR => gear.headgear = None,
        slot::GOGGLES => gear.goggles = None,
        slot::UNIFORM => gear.uniform = None,
        slot::VEST => gear.vest = None,
        slot::BACKPACK => gear.backpack = None,
        t => {
            if let Some(s) = LinkSlot::for_type(t) {
                gear.set_link(s, None);
            }
        }
    }
}

/// `addUniform`'s side check (`FUN_140734490`): the uniform's `uniformClass` soldier must list
/// the unit's side in `modelSides[]` _(uncertain: read from the wiki's `isUniformAllowed`; the
/// oracle confirms an OPFOR uniform is refused for BLUFOR)_.
fn uniform_allowed(config: &ConfigTree, uniform: &str, side: i32) -> bool {
    let root = config.root();
    let soldier = root
        .get("CfgWeapons")
        .get(uniform)
        .get("ItemInfo")
        .get("uniformClass")
        .text();
    let sides = root.get("CfgVehicles").get(&soldier).get("modelSides");
    if !sides.is_array() {
        return true;
    }
    sides.array().iter().any(|v| match v {
        a3_config::Value::Int(i) => *i == side,
        a3_config::Value::Float(f) => *f as i32 == side,
        _ => false,
    })
}

fn register_worn<H: WorldHost>(r: &mut Registry<H>) {
    r.binary("addHeadgear", OBJ, STR, NOTHING, |ctx, a, b| {
        add_worn(ctx, &a, &text(&b), slot::HEADGEAR, false)
    });
    r.binary("addGoggles", OBJ, STR, NOTHING, |ctx, a, b| {
        add_worn(ctx, &a, &text(&b), slot::GOGGLES, false)
    });
    r.binary("addVest", OBJ, STR, NOTHING, |ctx, a, b| {
        add_worn(ctx, &a, &text(&b), slot::VEST, false)
    });
    r.binary("addUniform", OBJ, STR, NOTHING, |ctx, a, b| {
        add_worn(ctx, &a, &text(&b), slot::UNIFORM, true)
    });
    r.binary("forceAddUniform", OBJ, STR, NOTHING, |ctx, a, b| {
        add_worn(ctx, &a, &text(&b), slot::UNIFORM, false)
    });
    // 0x8319f0: a CfgVehicles backpack with its config contents; the old one is dropped.
    r.binary("addBackpack", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |config, gear| {
            if let Some(info) = classify(config, &name).filter(|i| i.kind == ItemKind::Backpack) {
                gear.backpack = Some(make_container(config, &info));
            }
        });
        Ok(Value::Nothing)
    });
    r.unary("removeHeadgear", OBJ, NOTHING, |ctx, a| {
        remove_worn(ctx, a, slot::HEADGEAR)
    });
    r.unary("removeGoggles", OBJ, NOTHING, |ctx, a| {
        remove_worn(ctx, a, slot::GOGGLES)
    });
    r.unary("removeUniform", OBJ, NOTHING, |ctx, a| {
        remove_worn(ctx, a, slot::UNIFORM)
    });
    r.unary("removeVest", OBJ, NOTHING, |ctx, a| {
        remove_worn(ctx, a, slot::VEST)
    });
    r.unary("removeBackpack", OBJ, NOTHING, |ctx, a| {
        remove_worn(ctx, a, slot::BACKPACK)
    });
    // 0x844020: uniform, vest and backpack with their contents.
    r.unary("removeAllContainers", OBJ, NOTHING, |ctx, a| {
        change(ctx, &a, |_, g| {
            g.uniform = None;
            g.vest = None;
            g.backpack = None;
        });
        Ok(Value::Nothing)
    });
}

fn remove_worn<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    item_type: i32,
) -> Result<Value, SqfError> {
    change(ctx, &a, |_, g| clear_slot(g, item_type));
    Ok(Value::Nothing)
}

fn register_links<H: WorldHost>(r: &mut Registry<H>) {
    // 0x842c20: an inventory item into its slot, replacing (and dropping) what was there.
    r.binary("linkItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |config, gear| {
            crate::gear::link(config, gear, &name)
        });
        Ok(Value::Nothing)
    });
    // 0x847510 → 0x84a0f0: removes an assigned or worn item.
    r.binary("unlinkItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, gear| {
            crate::gear::unassign(gear, &name);
        });
        Ok(Value::Nothing)
    });
    // 0x847170: an assigned item back into the containers (dropped when nothing has room).
    r.binary("unassignItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |config, gear| {
            if let Some(class) = crate::gear::unassign(gear, &name) {
                if let Some(info) = classify(config, &class) {
                    gear.store(stored(config, &info), &info.allowed, &ANY_CONTAINER);
                }
            }
        });
        Ok(Value::Nothing)
    });
    // 0x83ccd0: a stored item into its slot; the replaced item goes back into the containers.
    r.binary("assignItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |config, gear| {
            let Some(info) = classify(config, &name).filter(ItemInfo::is_inventory_item) else {
                return;
            };
            if gear.take_item(&info.class).is_none() {
                return;
            }
            let old = current_worn(gear, info.item_type());
            wear(config, gear, &info);
            if let Some(old) = old.and_then(|o| classify(config, &o)) {
                gear.store(stored(config, &old), &old.allowed, &ANY_CONTAINER);
            }
        });
        Ok(Value::Nothing)
    });
    // 0x84b460: the link slots; `[unit, goggles, headgear]` also those.
    r.unary("removeAllAssignedItems", OBJ, NOTHING, |ctx, a| {
        change(ctx, &a, |_, g| {
            for s in LinkSlot::ALL {
                g.set_link(s, None);
            }
        });
        Ok(Value::Nothing)
    });
    r.unary("removeAllAssignedItems", ARR, NOTHING, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        if items.is_empty() {
            return Err(dim(0, 1));
        }
        let goggles = items.get(1).and_then(Value::as_bool).unwrap_or(false);
        let headgear = items.get(2).and_then(Value::as_bool).unwrap_or(false);
        change(ctx, &items[0], |_, g| {
            for s in LinkSlot::ALL {
                g.set_link(s, None);
            }
            if goggles {
                g.goggles = None;
            }
            if headgear {
                g.headgear = None;
            }
        });
        Ok(Value::Nothing)
    });
}

/// The class currently worn in the slot of an item type.
fn current_worn(gear: &Gear, item_type: i32) -> Option<String> {
    match item_type {
        slot::HEADGEAR => gear.headgear.clone(),
        slot::GOGGLES => gear.goggles.clone(),
        t => LinkSlot::for_type(t).and_then(|s| gear.link(s).map(str::to_owned)),
    }
}

fn register_weapons<H: WorldHost>(r: &mut Registry<H>) {
    // `addWeapon` and `removeWeapon` are the Loadout's (`script::weapons`).
    // 0x844790: every weapon, every stored magazine and item _(the oracle: `items` is empty
    // afterwards too)_; worn items stay.
    r.unary("removeAllWeapons", OBJ, NOTHING, |ctx, a| {
        unit_change(ctx, &a, |_, g, l| {
            l.weapons.clear();
            l.current = None;
            l.request = None;
            for c in g.containers_mut() {
                c.clear();
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("addPrimaryWeaponItem", OBJ, STR, NOTHING, |ctx, a, b| {
        add_weapon_item(ctx, a, b, WeaponSlot::Primary)
    });
    r.binary("addSecondaryWeaponItem", OBJ, STR, NOTHING, |ctx, a, b| {
        add_weapon_item(ctx, a, b, WeaponSlot::Secondary)
    });
    r.binary("addHandgunItem", OBJ, STR, NOTHING, |ctx, a, b| {
        add_weapon_item(ctx, a, b, WeaponSlot::Handgun)
    });
    r.binary("removePrimaryWeaponItem", OBJ, STR, NOTHING, |ctx, a, b| {
        remove_weapon_item(ctx, a, b, WeaponSlot::Primary)
    });
    r.binary(
        "removeSecondaryWeaponItem",
        OBJ,
        STR,
        NOTHING,
        |ctx, a, b| remove_weapon_item(ctx, a, b, WeaponSlot::Secondary),
    );
    r.binary("removeHandgunItem", OBJ, STR, NOTHING, |ctx, a, b| {
        remove_weapon_item(ctx, a, b, WeaponSlot::Handgun)
    });
}

/// `addPrimaryWeaponItem` & co.: an attachment replaces the one of its kind; a compatible
/// magazine replaces the loaded one; anything else is ignored.
fn add_weapon_item<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    b: Value,
    slot: WeaponSlot,
) -> Result<Value, SqfError> {
    let name = text(&b);
    if let Some(id) = man(ctx.host.world(), &a)
        .filter(|id| ctx.host.world().entity(*id).is_some_and(|e| e.is_local()))
    {
        ctx.host.world_mut().add_weapon_item(id, slot, &name);
    }
    Ok(Value::Nothing)
}

fn remove_weapon_item<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    b: Value,
    slot: WeaponSlot,
) -> Result<Value, SqfError> {
    let name = text(&b);
    if let Some(id) = man(ctx.host.world(), &a)
        .filter(|id| ctx.host.world().entity(*id).is_some_and(|e| e.is_local()))
    {
        ctx.host.world_mut().remove_weapon_item(id, slot, &name);
    }
    Ok(Value::Nothing)
}

fn register_magazines<H: WorldHost>(r: &mut Registry<H>) {
    // `addMagazine` and `addMagazines` are the Loadout's (`script::weapons`): a magazine the unit
    // is given goes into a container.
    // 0x844c90: one stored magazine (`[class, ammo]`: one with that ammo count).
    r.binary("removeMagazine", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, g| {
            g.take_magazine(|m| m.class.eq_ignore_ascii_case(&name));
        });
        Ok(Value::Nothing)
    });
    r.binary("removeMagazine", OBJ, ARR, NOTHING, |ctx, a, b| {
        let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let name = items.first().map(text).unwrap_or_default();
        let ammo = items.get(1).and_then(Value::as_number);
        change(ctx, &a, |_, g| {
            g.take_magazine(|m| {
                m.class.eq_ignore_ascii_case(&name) && ammo.is_none_or(|n| m.ammo == n as u32)
            });
        });
        Ok(Value::Nothing)
    });
    // 0x845940: every stored magazine of the class; loaded ones stay.
    r.binary("removeMagazines", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, g| {
            for c in g.containers_mut() {
                c.magazines.retain(|m| !m.class.eq_ignore_ascii_case(&name));
            }
        });
        Ok(Value::Nothing)
    });
}

fn register_items<H: WorldHost>(r: &mut Registry<H>) {
    // 0x839160 / 0x8391a0 / 0x8391c0 / 0x839180 → 0x848380 with a container mask: 0x80 any
    // (uniform, vest, backpack), 4 uniform, 8 vest, 2 backpack. The first container that allows
    // and fits the item takes it; otherwise nothing happens.
    r.binary("addItem", OBJ, STR, NOTHING, |ctx, a, b| {
        add_item(ctx, a, b, &ANY_CONTAINER)
    });
    r.binary("addItemToUniform", OBJ, STR, NOTHING, |ctx, a, b| {
        add_item(ctx, a, b, &[ContainerSlot::Uniform])
    });
    r.binary("addItemToVest", OBJ, STR, NOTHING, |ctx, a, b| {
        add_item(ctx, a, b, &[ContainerSlot::Vest])
    });
    r.binary("addItemToBackpack", OBJ, STR, NOTHING, |ctx, a, b| {
        add_item(ctx, a, b, &[ContainerSlot::Backpack])
    });
    // 0x844880: the first stored item, weapon or magazine of the class; assigned items stay.
    r.binary("removeItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, g| {
            if g.take_item(&name).is_none() && g.take_weapon(&name).is_none() {
                g.take_magazine(|m| m.class.eq_ignore_ascii_case(&name));
            }
        });
        Ok(Value::Nothing)
    });
    // 0x844a60: every stored item of the class (not magazines).
    r.binary("removeItems", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, g| {
            for c in g.containers_mut() {
                c.items.retain(|i| !i.class.eq_ignore_ascii_case(&name));
            }
        });
        Ok(Value::Nothing)
    });
    // 0x844240: stored items; magazines, weapons and worn items stay.
    r.unary("removeAllItems", OBJ, NOTHING, |ctx, a| {
        change(ctx, &a, |_, g| {
            for c in g.containers_mut() {
                c.items.clear();
            }
        });
        Ok(Value::Nothing)
    });
    // 0x844170: stored items and magazines.
    r.unary("removeAllItemsWithMagazines", OBJ, NOTHING, |ctx, a| {
        change(ctx, &a, |_, g| {
            for c in g.containers_mut() {
                c.items.clear();
                c.magazines.clear();
            }
        });
        Ok(Value::Nothing)
    });
}

fn add_item<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    b: Value,
    order: &[ContainerSlot],
) -> Result<Value, SqfError> {
    let name = text(&b);
    change(ctx, &a, |config, gear| {
        let Some(info) = classify(config, &name) else {
            return;
        };
        if info.kind == ItemKind::Backpack || info.kind == ItemKind::PseudoWeapon {
            return;
        }
        gear.store(stored(config, &info), &info.allowed, order);
    });
    Ok(Value::Nothing)
}
