//! Unit inventory commands: containers, worn items, assigned items, weapons, attachments,
//! magazines. From the decompiled handlers and the oracle probes
//! (`tools/oracle/probes/97_inventory_vr.probes`); see `docs/re/sqf-inventory.md`.
//!
//! A unit's gear is created from its config default on first use. Commands on anything that is
//! not a Man return `""` / `[]` / nothing. RPT-only warnings of the original ("Inventory item
//! with given name: [%s] not found", ...) are not reproduced.

use a3_config::ConfigTree;
use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, SqfError, Value};

use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, object_arg};
use crate::inventory::{
    ANY_CONTAINER, ContainerSlot, Gear, ItemInfo, ItemKind, LinkSlot, Stored, Weapon, WeaponSlot,
    classify, compatible_magazines, slot,
};
use crate::gear::{
    default_gear, load_from_containers, make_container, make_magazine, make_weapon, stored, wear,
};
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

/// Runs `f` on a unit's gear (created from the config default on first use).
fn with_gear<H: WorldHost, R>(
    host: &mut H,
    id: EntityId,
    f: impl FnOnce(&ConfigTree, &mut Gear) -> R,
) -> R {
    let config = host.types().config_arc();
    let mut gear = match host.world_mut().take_gear(id) {
        Some(g) => g,
        None => {
            let type_name = host
                .world()
                .entity(id)
                .map(|e| e.type_name().to_owned())
                .unwrap_or_default();
            default_gear(&config, &type_name).0
        }
    };
    let result = f(&config, &mut gear);
    host.world_mut().set_gear(id, gear);
    result
}

/// A gear query on a Man; `default` for anything else.
fn query<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    default: Value,
    f: impl FnOnce(&ConfigTree, &Gear) -> Value,
) -> Value {
    match man(ctx.host.world(), unit) {
        Some(id) => with_gear(ctx.host, id, |c, g| f(c, g)),
        None => default,
    }
}

/// A gear change on a local Man (the original forwards a remote unit's change to its owner).
fn change<H: WorldHost, R: Default>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    f: impl FnOnce(&ConfigTree, &mut Gear) -> R,
) -> R {
    let w = ctx.host.world();
    match man(w, unit) {
        Some(id) if w.entity(id).is_some_and(|e| e.is_local()) => with_gear(ctx.host, id, f),
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

fn weapon_items(gear: &Gear, slot: WeaponSlot) -> Value {
    let att = gear
        .weapon(slot)
        .map(|w| w.attachments.clone())
        .unwrap_or_default();
    string_list(att)
}

fn weapon_magazine(gear: &Gear, slot: WeaponSlot) -> Value {
    string_list(
        gear.weapon(slot)
            .and_then(|w| w.magazine.as_ref())
            .map(|m| m.class.clone()),
    )
}

/// Every worn item's and attachment's mass, for `load`.
fn worn_mass(config: &ConfigTree, gear: &Gear) -> f32 {
    let mass = |c: &str| classify(config, c).map_or(0.0, |i| i.mass);
    let attachments: f32 = gear
        .weapons
        .iter()
        .flat_map(|w| w.attachments.iter())
        .filter(|a| !a.is_empty())
        .map(|a| mass(a))
        .sum();
    gear.total_mass(mass) + attachments
}

fn register_getters<H: WorldHost>(r: &mut Registry<H>) {
    // 0x83e540 → 0x82e950: equipped weapons in the order added, then the weapons stored in the
    // containers. `Throw` and `Put` are not listed.
    r.unary("weapons", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            string_list(
                g.weapons.iter().map(|w| w.class.clone()).chain(
                    g.containers()
                        .flat_map(|c| c.weapons.iter().map(|w| w.class.clone())),
                ),
            )
        }))
    });
    r.unary("primaryWeapon", OBJ, STR, |ctx, a| {
        slot_name(ctx, a, WeaponSlot::Primary)
    });
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
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_items(g, WeaponSlot::Primary)
        }))
    });
    r.unary("secondaryWeaponItems", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_items(g, WeaponSlot::Secondary)
        }))
    });
    r.unary("handgunItems", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_items(g, WeaponSlot::Handgun)
        }))
    });
    r.unary("primaryWeaponMagazine", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_magazine(g, WeaponSlot::Primary)
        }))
    });
    r.unary("secondaryWeaponMagazine", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_magazine(g, WeaponSlot::Secondary)
        }))
    });
    r.unary("handgunMagazine", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            weapon_magazine(g, WeaponSlot::Handgun)
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
    // 0x83dfa0: stored magazines that are not empty, uniform first; loaded magazines are not
    // listed.
    r.unary("magazines", OBJ, ARR, |ctx, a| {
        Ok(query(ctx, &a, Value::array([]), |_, g| {
            string_list(g.containers().flat_map(|c| {
                c.magazines
                    .iter()
                    .filter(|m| m.ammo > 0)
                    .map(|m| m.class.clone())
                    .collect::<Vec<_>>()
            }))
        }))
    });
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
    // 0x83c530: map, compass, watch, radio, GPS, NVG; `[unit, goggles, headgear]` (exactly three
    // elements) adds goggles and headgear when the first flag is set — the original checks only
    // that flag for both.
    r.unary("assignedItems", OBJ, ARR, |ctx, a| {
        Ok(assigned(ctx, &a, false))
    });
    r.unary("assignedItems", ARR, ARR, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        if items.len() < 3 {
            return Err(dim(items.len(), 3));
        }
        let worn = items[1].as_bool().unwrap_or(false);
        Ok(assigned(ctx, &items[0], worn))
    });
    // 0x5353e0: weapons, stored weapons and pseudo weapons, any case.
    r.binary("hasWeapon", OBJ, STR, BOOL, |ctx, a, b| {
        let name = text(&b);
        Ok(query(ctx, &a, Value::Bool(false), |_, g| {
            let eq = |c: &String| c.eq_ignore_ascii_case(&name);
            Value::Bool(
                g.weapons.iter().any(|w| eq(&w.class))
                    || g.pseudo_weapons.iter().any(eq)
                    || g.containers()
                        .any(|c| c.weapons.iter().any(|w| eq(&w.class))),
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
        Ok(query(ctx, &a, Value::Number(0.0), |c, g| {
            Value::Number(worn_mass(c, g))
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
        Ok(query(ctx, &a, Value::Number(0.0), |c, g| {
            Value::Number(if max > 0.0 {
                worn_mass(c, g) / max
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
    Ok(query(ctx, &a, Value::from(""), |_, g| {
        Value::string(g.weapon(slot).map_or(String::new(), |w| w.class.clone()))
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
/// 0x8438c0): the slot is emptied first; an item of another type is the script error "Tried
/// to add inventory item with type 'X' into slot of type 'Y'" and the slot stays empty.
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
    let error = with_gear(ctx.host, id, |config, gear| {
        clear_slot(gear, expected);
        if info.item_type() != expected {
            return Some(SqfError::generic(format!(
                "Tried to add inventory item with type '{}' into slot of type '{}'",
                slot::name(info.item_type()),
                slot::name(expected)
            )));
        }
        if check_side && !uniform_allowed(config, &info.class, unit_side) {
            return None;
        }
        wear(config, gear, &info);
        None
    });
    match error {
        Some(e) => Err(e),
        None => Ok(Value::Nothing),
    }
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
        change(ctx, &a, |config, gear| link(config, gear, &name));
        Ok(Value::Nothing)
    });
    // 0x847510 → 0x84a0f0: removes an assigned or worn item.
    r.binary("unlinkItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |_, gear| {
            unassign(gear, &name);
        });
        Ok(Value::Nothing)
    });
    // 0x847170: an assigned item back into the containers (dropped when nothing has room).
    r.binary("unassignItem", OBJ, STR, NOTHING, |ctx, a, b| {
        let name = text(&b);
        change(ctx, &a, |config, gear| {
            if let Some(class) = unassign(gear, &name) {
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

/// `linkItem` (and `addWeapon` with an item): link slot items, headgear, goggles, vest and
/// uniform go to their slot; anything else is ignored.
fn link(config: &ConfigTree, gear: &mut Gear, name: &str) {
    if let Some(info) = classify(config, name).filter(ItemInfo::is_inventory_item) {
        wear(config, gear, &info);
    }
}

/// Removes an assigned or worn item (link slots, headgear, goggles) by class; returns it.
fn unassign(gear: &mut Gear, name: &str) -> Option<String> {
    for s in LinkSlot::ALL {
        if gear.link(s).is_some_and(|c| c.eq_ignore_ascii_case(name)) {
            let class = gear.link(s).map(str::to_owned);
            gear.set_link(s, None);
            return class;
        }
    }
    if gear
        .headgear
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case(name))
    {
        return gear.headgear.take();
    }
    if gear
        .goggles
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case(name))
    {
        return gear.goggles.take();
    }
    None
}

fn register_weapons<H: WorldHost>(r: &mut Registry<H>) {
    // 0x83b700 → 0x83bc10: a weapon replaces the one of its slot (dropped with its magazine
    // and attachments), goes to the end of the list, gets its linked attachments and loads the
    // first compatible stored magazine. An inventory item is linked instead (as `linkItem`).
    r.binary("addWeapon", OBJ, STR, NOTHING, |ctx, a, b| {
        add_weapon(ctx, &a, &text(&b));
        Ok(Value::Nothing)
    });
    r.binary("addWeapon", OBJ, ARR, NOTHING, |ctx, a, b| {
        let first = b
            .as_array()
            .and_then(|x| x.borrow().first().cloned())
            .unwrap_or(Value::Nil);
        add_weapon(ctx, &a, &text(&first));
        Ok(Value::Nothing)
    });
    // 0x8462d0: a weapon with its loaded magazine; an inventory item is unlinked instead.
    r.binary("removeWeapon", OBJ, STR, NOTHING, |ctx, a, b| {
        remove_weapon(ctx, &a, &text(&b));
        Ok(Value::Nothing)
    });
    r.binary("removeWeapon", OBJ, ARR, NOTHING, |ctx, a, b| {
        let first = b
            .as_array()
            .and_then(|x| x.borrow().first().cloned())
            .unwrap_or(Value::Nil);
        remove_weapon(ctx, &a, &text(&first));
        Ok(Value::Nothing)
    });
    // 0x844790: every weapon, every stored magazine and item _(the oracle: `items` is empty
    // afterwards too)_; worn items stay.
    r.unary("removeAllWeapons", OBJ, NOTHING, |ctx, a| {
        change(ctx, &a, |_, g| {
            g.weapons.clear();
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

fn add_weapon<H: WorldHost>(ctx: &mut Ctx<'_, H>, unit: &Value, name: &str) {
    change(ctx, unit, |config, gear| {
        let Some(info) = classify(config, name) else {
            return;
        };
        match info.kind {
            ItemKind::Weapon(slot) => {
                let weapon = make_weapon(config, &info, slot);
                gear.equip(weapon);
                let last = gear.weapons.len() - 1;
                load_from_containers(config, gear, last);
            }
            ItemKind::PseudoWeapon => {
                if !gear
                    .pseudo_weapons
                    .iter()
                    .any(|w| w.eq_ignore_ascii_case(&info.class))
                {
                    gear.pseudo_weapons.push(info.class.clone());
                }
            }
            _ if info.is_inventory_item() => link(config, gear, name),
            _ => {}
        }
    });
}

fn remove_weapon<H: WorldHost>(ctx: &mut Ctx<'_, H>, unit: &Value, name: &str) {
    change(ctx, unit, |config, gear| {
        if classify(config, name).is_some_and(|i| i.is_inventory_item()) {
            unassign(gear, name);
            return;
        }
        if let Some(i) = gear
            .weapons
            .iter()
            .position(|w| w.class.eq_ignore_ascii_case(name))
        {
            gear.weapons.remove(i);
        } else {
            gear.pseudo_weapons
                .retain(|w| !w.eq_ignore_ascii_case(name));
        }
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
    change(ctx, &a, |config, gear| {
        let Some(info) = classify(config, &name) else {
            return;
        };
        let compatible = gear
            .weapon(slot)
            .map(|w| compatible_magazines(config, &w.class))
            .unwrap_or_default();
        let Some(weapon) = gear.weapon_mut(slot) else {
            return;
        };
        match info.kind {
            ItemKind::Magazine { count } => {
                if compatible.contains(&info.class.to_ascii_lowercase()) {
                    weapon.magazine = Some(make_magazine(&info, count));
                }
            }
            ItemKind::Item(t) => {
                if let Some(i) = Weapon::attachment_index(t) {
                    weapon.attachments[i] = info.class.clone();
                }
            }
            _ => {}
        }
    });
    Ok(Value::Nothing)
}

fn remove_weapon_item<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    a: Value,
    b: Value,
    slot: WeaponSlot,
) -> Result<Value, SqfError> {
    let name = text(&b);
    change(ctx, &a, |_, gear| {
        let Some(weapon) = gear.weapon_mut(slot) else {
            return;
        };
        if let Some(att) = weapon
            .attachments
            .iter_mut()
            .find(|c| c.eq_ignore_ascii_case(&name))
        {
            att.clear();
        } else if weapon
            .magazine
            .as_ref()
            .is_some_and(|m| m.class.eq_ignore_ascii_case(&name))
        {
            weapon.magazine = None;
        }
    });
    Ok(Value::Nothing)
}

fn register_magazines<H: WorldHost>(r: &mut Registry<H>) {
    // 0x839ba0: a full magazine into the first container with room.
    r.binary("addMagazine", OBJ, STR, NOTHING, |ctx, a, b| {
        add_magazines(ctx, &a, &text(&b), None, 1);
        Ok(Value::Nothing)
    });
    // 0x83a4a0: `[class, ammo]`; ammo above the magazine's count, or negative, is the count.
    r.binary("addMagazine", OBJ, ARR, NOTHING, |ctx, a, b| {
        let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let name = items.first().map(text).unwrap_or_default();
        let ammo = items.get(1).and_then(Value::as_number);
        add_magazines(ctx, &a, &name, ammo, 1);
        Ok(Value::Nothing)
    });
    // 0x83b290: `[class, count]`.
    r.binary("addMagazines", OBJ, ARR, NOTHING, |ctx, a, b| {
        let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let name = items.first().map(text).unwrap_or_default();
        let count = items
            .get(1)
            .and_then(Value::as_number)
            .unwrap_or(0.0)
            .max(0.0) as usize;
        add_magazines(ctx, &a, &name, None, count);
        Ok(Value::Nothing)
    });
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

fn add_magazines<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    unit: &Value,
    name: &str,
    ammo: Option<f32>,
    times: usize,
) {
    change(ctx, unit, |config, gear| {
        // "Warning: "%s" is not a valid magazine name" for anything else.
        let Some(info) = classify(config, name) else {
            return;
        };
        let ItemKind::Magazine { count } = info.kind else {
            return;
        };
        // Negative or more than the magazine holds: full; 0 adds an empty magazine.
        let rounds = match ammo {
            Some(n) if n >= 0.0 && (n as u32) < count => n as u32,
            _ => count,
        };
        for _ in 0..times {
            gear.store(
                Stored::Magazine(make_magazine(&info, rounds)),
                &info.allowed,
                &ANY_CONTAINER,
            );
        }
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
