//! Building gear from config: weapons with their linked attachments, containers with their
//! config contents, worn items, and the default loadout a unit starts with.

use a3_config::{ConfigRef, ConfigTree};

use crate::inventory::{
    Container, ContainerSlot, Gear, ItemInfo, ItemKind, LinkSlot, Magazine, Stored, StoredItem,
    Weapon, WeaponSlot, classify, compatible_magazines, slot,
};

fn number(c: &ConfigRef<'_>) -> f32 {
    if c.is_number() { c.number() } else { 0.0 }
}

fn strings(c: &ConfigRef<'_>) -> Vec<String> {
    c.array()
        .iter()
        .filter_map(|v| match v {
            a3_config::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// A weapon instance of a classified weapon, with the attachments of its `LinkedItems`.
pub fn make_weapon(config: &ConfigTree, info: &ItemInfo, slot: WeaponSlot) -> Weapon {
    let mut weapon = Weapon {
        class: info.class.clone(),
        slot,
        mass: info.mass,
        attachments: Default::default(),
        magazine: None,
    };
    let linked = config
        .root()
        .get("CfgWeapons")
        .get(&info.class)
        .get("LinkedItems");
    for entry in linked.entries().iter().filter(|e| e.is_class()) {
        let item = entry.get("item").text();
        if let Some(i) = Weapon::linked_slot_index(&entry.get("slot").text()) {
            let class = classify(config, &item).map_or(item, |c| c.class);
            weapon.attachments[i] = class;
        }
    }
    weapon
}

/// A magazine instance with `ammo` rounds.
pub fn make_magazine(info: &ItemInfo, ammo: u32) -> Magazine {
    Magazine {
        class: info.class.clone(),
        ammo,
        mass: info.mass,
    }
}

/// What a classified class becomes when stored in a container.
pub fn stored(config: &ConfigTree, info: &ItemInfo) -> Stored {
    match info.kind {
        ItemKind::Magazine { count } => Stored::Magazine(make_magazine(info, count)),
        ItemKind::Weapon(slot) => Stored::Weapon(make_weapon(config, info, slot)),
        _ => Stored::Item(StoredItem {
            class: info.class.clone(),
            mass: info.mass,
        }),
    }
}

/// A container instance of a uniform, vest or backpack class, with the backpack's config
/// contents (`TransportItems`, `TransportWeapons`, `TransportMagazines`).
pub fn make_container(config: &ConfigTree, info: &ItemInfo) -> Container {
    let mut container = Container::new(info);
    if info.kind != ItemKind::Backpack {
        return container;
    }
    let class = config.root().get("CfgVehicles").get(&info.class);
    for list in ["TransportItems", "TransportWeapons", "TransportMagazines"] {
        for entry in class.get(list).entries().iter().filter(|e| e.is_class()) {
            // Items name their class with `name`, magazines with `magazine`, weapons with
            // `weapon`.
            let name = ["name", "magazine", "weapon"]
                .iter()
                .map(|k| entry.get(k).text())
                .find(|n| !n.is_empty())
                .unwrap_or_default();
            let count = number(&entry.get("count")).max(0.0) as usize;
            let Some(item) = classify(config, &name) else {
                continue;
            };
            for _ in 0..count {
                match stored(config, &item) {
                    Stored::Magazine(m) => container.magazines.push(m),
                    Stored::Weapon(w) => container.weapons.push(w),
                    Stored::Item(i) => container.items.push(i),
                }
            }
        }
    }
    container
}

/// Puts a worn item (uniform, vest, backpack, headgear, goggles, link slot items) in its slot,
/// replacing what was there. Returns `false` when it has no worn slot.
pub fn wear(config: &ConfigTree, gear: &mut Gear, info: &ItemInfo) -> bool {
    match info.kind {
        ItemKind::Goggles => gear.goggles = Some(info.class.clone()),
        ItemKind::Backpack => gear.backpack = Some(make_container(config, info)),
        ItemKind::Item(slot::UNIFORM) => gear.uniform = Some(make_container(config, info)),
        ItemKind::Item(slot::VEST) => gear.vest = Some(make_container(config, info)),
        ItemKind::Item(slot::HEADGEAR) => gear.headgear = Some(info.class.clone()),
        ItemKind::Item(t) => match LinkSlot::for_type(t) {
            Some(s) => gear.set_link(s, Some(info.class.clone())),
            None => return false,
        },
        _ => return false,
    }
    true
}

/// Puts an item where `linkItem` and `addWeapon` put it: link slot items, headgear, goggles, vest
/// and uniform go to their worn slot; anything else is ignored.
pub fn link(config: &ConfigTree, gear: &mut Gear, name: &str) {
    if let Some(info) = classify(config, name).filter(ItemInfo::is_inventory_item) {
        wear(config, gear, &info);
    }
}

/// Removes an assigned or worn item (link slots, headgear, goggles) by class; returns it.
pub fn unassign(gear: &mut Gear, name: &str) -> Option<String> {
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

/// Loads `weapon`'s empty muzzle with the first compatible stored magazine (uniform, vest,
/// backpack).
pub fn load_weapon(config: &ConfigTree, gear: &mut Gear, weapon: &mut Weapon) {
    if weapon.magazine.is_some() {
        return;
    }
    let compatible = compatible_magazines(config, &weapon.class);
    if let Some(m) = gear.take_magazine(|m| compatible.contains(&m.class.to_ascii_lowercase())) {
        weapon.magazine = Some(m);
    }
}

/// Puts `weapon` in an equipped-weapon list: the weapon of its slot is dropped (with its magazine
/// and attachments) and the new one goes to the end.
pub fn equip(weapons: &mut Vec<Weapon>, weapon: Weapon) {
    weapons.retain(|w| w.slot != weapon.slot);
    weapons.push(weapon);
}

/// What a unit's CfgVehicles class starts with.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfigLoadout {
    /// The containers, their config contents, and the worn items.
    pub gear: Gear,
    /// The `weapons[]` to equip, in config order, with their linked attachments and the magazine
    /// each one loads.
    pub weapons: Vec<Weapon>,
    /// The warnings the engine logs while building it.
    pub warnings: Vec<String>,
}

/// The containers the default loadout stores into (mask 0x100: uniform, vest, backpack).
const DEFAULT_STORE: [ContainerSlot; 3] = [
    ContainerSlot::Uniform,
    ContainerSlot::Vest,
    ContainerSlot::Backpack,
];

/// What a unit of CfgVehicles class `unit` starts with.
///
/// Order (matches the oracle on `B_Soldier_F`, `B_soldier_LAT_F`, `B_Soldier_AR_F`):
/// `uniformClass`; `linkedItems[]`; `backpack` with its contents; `weapons[]` with their linked
/// attachments; `items[]`; then `magazines[]` stored first-fit into the uniform, vest and
/// backpack (`FUN_140fc6bc0`, mask 0x100); every empty weapon then loads the first compatible
/// stored magazine, else one that did not fit; finally the magazines that did not fit are tried
/// again (loading freed room) and the rest are dropped with "Some of magazines weren't stored in
/// soldier Vest or Uniform?". Random headgear and facewear (`headgearList`, `identityTypes`)
/// are not applied.
pub fn default_loadout(config: &ConfigTree, unit: &str) -> ConfigLoadout {
    let mut gear = Gear::default();
    let mut weapons: Vec<Weapon> = Vec::new();
    let mut warnings = Vec::new();
    let class = config.root().get("CfgVehicles").get(unit);
    if !class.is_class() {
        return ConfigLoadout {
            gear,
            weapons,
            warnings,
        };
    }
    let lookup = |name: &str| classify(config, name);
    if let Some(info) = lookup(&class.get("uniformClass").text()) {
        if info.kind == ItemKind::Item(slot::UNIFORM) {
            gear.uniform = Some(make_container(config, &info));
        }
    }
    for name in strings(&class.get("linkedItems")) {
        if let Some(info) = lookup(&name) {
            wear(config, &mut gear, &info);
        }
    }
    if let Some(info) = lookup(&class.get("backpack").text()) {
        if info.kind == ItemKind::Backpack {
            gear.backpack = Some(make_container(config, &info));
        }
    }
    for name in strings(&class.get("weapons")) {
        let Some(info) = lookup(&name) else {
            continue;
        };
        match info.kind {
            ItemKind::Weapon(slot) => equip(&mut weapons, make_weapon(config, &info, slot)),
            ItemKind::PseudoWeapon => gear.pseudo_weapons.push(info.class.clone()),
            _ => {}
        }
    }
    for name in strings(&class.get("items")) {
        if let Some(info) = lookup(&name) {
            gear.store(stored(config, &info), &info.allowed, &DEFAULT_STORE);
        }
    }
    let mut failed = Vec::new();
    for name in strings(&class.get("magazines")) {
        let Some(info) = lookup(&name) else {
            continue;
        };
        let ItemKind::Magazine { count } = info.kind else {
            continue;
        };
        let magazine = make_magazine(&info, count);
        if !gear.store(
            Stored::Magazine(magazine.clone()),
            &info.allowed,
            &DEFAULT_STORE,
        ) {
            failed.push((magazine, info.allowed.clone()));
        }
    }
    for weapon in weapons.iter_mut() {
        load_weapon(config, &mut gear, weapon);
        if weapon.magazine.is_none() {
            let compatible = compatible_magazines(config, &weapon.class);
            if let Some(k) = failed
                .iter()
                .position(|(m, _)| compatible.contains(&m.class.to_ascii_lowercase()))
            {
                weapon.magazine = Some(failed.remove(k).0);
            }
        }
    }
    let mut dropped = false;
    for (magazine, allowed) in failed {
        dropped |= !gear.store(Stored::Magazine(magazine), &allowed, &DEFAULT_STORE);
    }
    if dropped {
        warnings.push(format!(
            "soldier[{}]:Some of magazines weren't stored in soldier Vest or Uniform?",
            class.name()
        ));
    }
    ConfigLoadout {
        gear,
        weapons,
        warnings,
    }
}
