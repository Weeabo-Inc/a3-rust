//! A unit's gear: uniform, vest and backpack containers, the equipment slots (headgear,
//! goggles, map, compass, watch, radio, GPS, NVG), the weapons with their attachments and
//! loaded magazines. See `docs/re/sqf-inventory.md` for what comes from the engine and from the
//! oracle (`tools/oracle/probes/97_inventory_vr.probes`).
//!
//! Items are config classes; what an item is and where it goes is read from the config
//! ([`classify`]): `CfgMagazines`, `CfgWeapons` (`type`, `simulation`, `ItemInfo >> type`),
//! `CfgGlasses` and the `CfgVehicles` backpacks (`isBackpack = 1`).

use a3_config::{ConfigRef, ConfigTree};

/// The engine's inventory item / slot type numbers (`ItemInfo >> type`; the item factory at
/// 0x141688430, the type names at 0x14084c400).
pub mod slot {
    pub const DEFAULT: i32 = 0;
    pub const MUZZLE: i32 = 101;
    pub const OPTIC: i32 = 201;
    pub const POINTER: i32 = 301;
    pub const BIPOD: i32 = 302;
    pub const FIRST_AID: i32 = 401;
    pub const NVG: i32 = 602;
    pub const GOGGLES: i32 = 603;
    pub const SCUBA: i32 = 604;
    pub const HEADGEAR: i32 = 605;
    pub const MAP: i32 = 608;
    pub const COMPASS: i32 = 609;
    pub const WATCH: i32 = 610;
    pub const RADIO: i32 = 611;
    pub const GPS: i32 = 612;
    pub const PARA: i32 = 613;
    pub const HMD: i32 = 616;
    pub const BINOCULAR: i32 = 617;
    pub const MINE_DETECTOR: i32 = 618;
    pub const MEDIKIT: i32 = 619;
    pub const TOOLKIT: i32 = 620;
    pub const UAV_TERMINAL: i32 = 621;
    pub const VEST: i32 = 701;
    pub const UNIFORM: i32 = 801;
    pub const BACKPACK: i32 = 901;

    /// The engine's name of a type, as its error messages print it (0x14084c400).
    pub fn name(t: i32) -> &'static str {
        match t {
            DEFAULT => "Default",
            MUZZLE => "Muzzle",
            OPTIC => "Optics",
            POINTER => "Flashlight",
            BIPOD => "UnderBarrel",
            FIRST_AID => "FirstAidKit",
            NVG => "NVG",
            GOGGLES => "Goggles",
            SCUBA => "Scuba",
            HEADGEAR => "Headgear",
            MAP => "Map",
            COMPASS => "Compass",
            WATCH => "Watch",
            RADIO => "Radio",
            GPS => "GPS",
            PARA => "Para",
            HMD => "HMD",
            BINOCULAR => "Binocular",
            MINE_DETECTOR => "MineDetector",
            MEDIKIT => "Medikit",
            TOOLKIT => "Toolkit",
            UAV_TERMINAL => "UavTerminal",
            VEST => "Vest",
            UNIFORM => "Uniform",
            BACKPACK => "Backpack",
            1001 => "Backpack2",
            _ => "Invalid",
        }
    }
}

/// The weapon slots of a unit, by the `CfgWeapons >> type` bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponSlot {
    Primary,
    Handgun,
    Secondary,
    Binocular,
}

/// The assigned-item slots (`assignedItems`, `linkItem`), in the engine's listing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkSlot {
    Map,
    Compass,
    Watch,
    Radio,
    Gps,
    Hmd,
}

impl LinkSlot {
    pub const ALL: [LinkSlot; 6] = [
        LinkSlot::Map,
        LinkSlot::Compass,
        LinkSlot::Watch,
        LinkSlot::Radio,
        LinkSlot::Gps,
        LinkSlot::Hmd,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// The link slot an item type goes to.
    pub fn for_type(item_type: i32) -> Option<LinkSlot> {
        Some(match item_type {
            slot::MAP => LinkSlot::Map,
            slot::COMPASS => LinkSlot::Compass,
            slot::WATCH => LinkSlot::Watch,
            slot::RADIO => LinkSlot::Radio,
            slot::GPS | slot::UAV_TERMINAL => LinkSlot::Gps,
            slot::HMD | slot::NVG => LinkSlot::Hmd,
            _ => return None,
        })
    }
}

/// What a config class is to the inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A `CfgWeapons` weapon for one of the weapon slots.
    Weapon(WeaponSlot),
    /// A pseudo weapon such as `Throw` and `Put` (`type = 0`).
    PseudoWeapon,
    /// A `CfgWeapons` item with its inventory type (`ItemInfo >> type`, or the type its
    /// `simulation` implies for map, compass, watch, radio, GPS and NVG).
    Item(i32),
    /// A `CfgMagazines` magazine with its ammo count.
    Magazine { count: u32 },
    /// A `CfgGlasses` class.
    Goggles,
    /// A `CfgVehicles` backpack.
    Backpack,
}

/// A config class classified for the inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    /// The class name as the config spells it.
    pub class: String,
    pub kind: ItemKind,
    /// Weight in inventory mass units.
    pub mass: f32,
    /// `allowedSlots[]`: the container types (701, 801, 901) that may hold it; empty = any.
    pub allowed: Vec<i32>,
    /// For uniforms, vests and backpacks: the container's `maximumLoad`.
    pub capacity: f32,
}

impl ItemInfo {
    /// The item type for slot checks: `Item` types as they are, goggles 603, backpacks 901,
    /// everything else `DEFAULT`.
    pub fn item_type(&self) -> i32 {
        match self.kind {
            ItemKind::Item(t) => t,
            ItemKind::Goggles => slot::GOGGLES,
            ItemKind::Backpack => slot::BACKPACK,
            ItemKind::Weapon(WeaponSlot::Binocular) => slot::BINOCULAR,
            _ => slot::DEFAULT,
        }
    }

    /// Whether this is an "inventory item" (`FUN_141689a20`): a `CfgGlasses` class or a
    /// `CfgWeapons` item — not a weapon, magazine or backpack.
    pub fn is_inventory_item(&self) -> bool {
        matches!(self.kind, ItemKind::Item(_) | ItemKind::Goggles)
    }
}

fn number(c: &ConfigRef<'_>) -> f32 {
    if c.is_number() { c.number() } else { 0.0 }
}

fn int_list(c: &ConfigRef<'_>) -> Vec<i32> {
    c.array()
        .iter()
        .filter_map(|v| match v {
            a3_config::Value::Int(i) => Some(*i),
            a3_config::Value::Float(f) => Some(*f as i32),
            _ => None,
        })
        .collect()
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

/// Classifies `name` (any case) against the config; `None` when no inventory class has it.
pub fn classify(config: &ConfigTree, name: &str) -> Option<ItemInfo> {
    if name.is_empty() {
        return None;
    }
    let root = config.root();
    let weapon = root.get("CfgWeapons").get(name);
    if weapon.is_class() {
        return Some(classify_weapon(config, &weapon));
    }
    let magazine = root.get("CfgMagazines").get(name);
    if magazine.is_class() {
        return Some(ItemInfo {
            class: magazine.name().to_owned(),
            kind: ItemKind::Magazine {
                count: number(&magazine.get("count")).max(0.0) as u32,
            },
            mass: number(&magazine.get("mass")),
            allowed: int_list(&magazine.get("allowedSlots")),
            capacity: 0.0,
        });
    }
    let glasses = root.get("CfgGlasses").get(name);
    if glasses.is_class() {
        return Some(ItemInfo {
            class: glasses.name().to_owned(),
            kind: ItemKind::Goggles,
            mass: number(&glasses.get("mass")),
            allowed: Vec::new(),
            capacity: 0.0,
        });
    }
    let vehicle = root.get("CfgVehicles").get(name);
    if vehicle.is_class() && number(&vehicle.get("isBackpack")) == 1.0 {
        return Some(ItemInfo {
            class: vehicle.name().to_owned(),
            kind: ItemKind::Backpack,
            mass: number(&vehicle.get("mass")),
            allowed: Vec::new(),
            capacity: number(&vehicle.get("maximumLoad")),
        });
    }
    None
}

fn classify_weapon(config: &ConfigTree, weapon: &ConfigRef<'_>) -> ItemInfo {
    let class = weapon.name().to_owned();
    let simulation = weapon.get("simulation").text().to_ascii_lowercase();
    let weapon_type = number(&weapon.get("type")) as i32;
    let info = weapon.get("ItemInfo");
    let slots = weapon.get("WeaponSlotsInfo");
    let simulated_type = match simulation.as_str() {
        "itemmap" => Some(slot::MAP),
        "itemcompass" => Some(slot::COMPASS),
        "itemwatch" => Some(slot::WATCH),
        "itemradio" => Some(slot::RADIO),
        "itemgps" => Some(slot::GPS),
        "itemminedetector" => Some(slot::MINE_DETECTOR),
        "nvgoggles" => Some(slot::HMD),
        _ => None,
    };
    let weapon_slot = if weapon_type & 1 != 0 {
        Some(WeaponSlot::Primary)
    } else if weapon_type & 2 != 0 {
        Some(WeaponSlot::Handgun)
    } else if weapon_type & 4 != 0 {
        Some(WeaponSlot::Secondary)
    } else if weapon_type == 4096 {
        Some(WeaponSlot::Binocular)
    } else {
        None
    };
    let (kind, from) = match (simulated_type, weapon_slot) {
        (Some(t), _) => (ItemKind::Item(t), &info),
        (None, Some(s)) => (ItemKind::Weapon(s), &slots),
        (None, None) if weapon_type == 0 => (ItemKind::PseudoWeapon, &info),
        (None, None) => (ItemKind::Item(number(&info.get("type")) as i32), &info),
    };
    let capacity = match kind {
        ItemKind::Item(slot::VEST | slot::UNIFORM) => {
            let container = info.get("containerClass").text();
            number(
                &config
                    .root()
                    .get("CfgVehicles")
                    .get(&container)
                    .get("maximumLoad"),
            )
        }
        _ => 0.0,
    };
    ItemInfo {
        class,
        kind,
        mass: number(&from.get("mass")),
        allowed: int_list(&from.get("allowedSlots")),
        capacity,
    }
}

/// The magazines a weapon's main muzzle takes: `magazines[]` and the `magazineWell[]`
/// lists of `CfgMagazineWells`, lower case.
pub fn compatible_magazines(config: &ConfigTree, weapon: &str) -> Vec<String> {
    let root = config.root();
    let w = root.get("CfgWeapons").get(weapon);
    let mut out: Vec<String> = strings(&w.get("magazines"));
    for well in strings(&w.get("magazineWell")) {
        let class = root.get("CfgMagazineWells").get(&well);
        for entry in class.entries() {
            out.extend(strings(&entry));
        }
    }
    out.iter().map(|m| m.to_ascii_lowercase()).collect()
}

/// A magazine instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Magazine {
    pub class: String,
    pub ammo: u32,
    pub mass: f32,
}

/// A weapon instance with its attachments and loaded magazine.
#[derive(Debug, Clone, PartialEq)]
pub struct Weapon {
    pub class: String,
    pub slot: WeaponSlot,
    pub mass: f32,
    /// Muzzle, pointer (side rail), optic, bipod; `""` when empty.
    pub attachments: [String; 4],
    pub magazine: Option<Magazine>,
}

impl Weapon {
    /// The attachment index of an item type.
    pub fn attachment_index(item_type: i32) -> Option<usize> {
        Some(match item_type {
            slot::MUZZLE => 0,
            slot::POINTER => 1,
            slot::OPTIC => 2,
            slot::BIPOD => 3,
            _ => return None,
        })
    }

    /// The attachment index of a `LinkedItems` slot name.
    pub fn linked_slot_index(slot: &str) -> Option<usize> {
        Some(match slot.to_ascii_lowercase().as_str() {
            "muzzleslot" => 0,
            "pointerslot" => 1,
            "cowsslot" => 2,
            "underbarrelslot" => 3,
            _ => return None,
        })
    }

    /// Mass with the loaded magazine (attachments are not weighed _(uncertain)_).
    pub fn total_mass(&self) -> f32 {
        self.mass + self.magazine.as_ref().map_or(0.0, |m| m.mass)
    }
}

/// An item stored in a container.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredItem {
    pub class: String,
    pub mass: f32,
}

/// A uniform, vest or backpack and what it holds: items, weapons and magazines in separate
/// lists, listed in that order.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub class: String,
    /// The container's own mass.
    pub mass: f32,
    /// `maximumLoad`.
    pub capacity: f32,
    pub items: Vec<StoredItem>,
    pub weapons: Vec<Weapon>,
    pub magazines: Vec<Magazine>,
}

impl Container {
    pub fn new(info: &ItemInfo) -> Self {
        Self {
            class: info.class.clone(),
            mass: info.mass,
            capacity: info.capacity,
            items: Vec::new(),
            weapons: Vec::new(),
            magazines: Vec::new(),
        }
    }

    /// Mass of the contents.
    pub fn load(&self) -> f32 {
        self.items.iter().map(|i| i.mass).sum::<f32>()
            + self.weapons.iter().map(Weapon::total_mass).sum::<f32>()
            + self.magazines.iter().map(|m| m.mass).sum::<f32>()
    }

    /// Whether `mass` more fits: the engine's `load + mass <= maximumLoad` (0x1416af5d0).
    pub fn fits(&self, mass: f32) -> bool {
        self.load() + mass <= self.capacity
    }

    /// Item and weapon names, then magazine names (`uniformItems` and siblings).
    pub fn listing(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|i| i.class.clone())
            .chain(self.weapons.iter().map(|w| w.class.clone()))
            .chain(self.magazines.iter().map(|m| m.class.clone()))
            .collect()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.weapons.clear();
        self.magazines.clear();
    }
}

/// The container slots, with the engine's container type numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerSlot {
    Uniform,
    Vest,
    Backpack,
}

impl ContainerSlot {
    /// The container type number items list in `allowedSlots[]`.
    pub fn slot_type(self) -> i32 {
        match self {
            ContainerSlot::Uniform => slot::UNIFORM,
            ContainerSlot::Vest => slot::VEST,
            ContainerSlot::Backpack => slot::BACKPACK,
        }
    }
}

/// The containers `addItem` and its siblings try, in order (`FUN_141681fe0`, mask 0x80):
/// uniform, vest, backpack.
pub const ANY_CONTAINER: [ContainerSlot; 3] = [
    ContainerSlot::Uniform,
    ContainerSlot::Vest,
    ContainerSlot::Backpack,
];

/// What one container entry is, for [`Gear::store`].
#[derive(Debug, Clone, PartialEq)]
pub enum Stored {
    Item(StoredItem),
    Weapon(Weapon),
    Magazine(Magazine),
}

impl Stored {
    pub fn mass(&self) -> f32 {
        match self {
            Stored::Item(i) => i.mass,
            Stored::Weapon(w) => w.total_mass(),
            Stored::Magazine(m) => m.mass,
        }
    }
}

/// A unit's gear.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gear {
    pub uniform: Option<Container>,
    pub vest: Option<Container>,
    pub backpack: Option<Container>,
    pub headgear: Option<String>,
    pub goggles: Option<String>,
    links: [Option<String>; 6],
    /// Equipped weapons in the order they were added (`weapons` lists them so).
    pub weapons: Vec<Weapon>,
    /// `Throw`, `Put` and other pseudo weapons (`hasWeapon` sees them, `weapons` does not).
    pub pseudo_weapons: Vec<String>,
}

impl Gear {
    pub fn container(&self, slot: ContainerSlot) -> Option<&Container> {
        match slot {
            ContainerSlot::Uniform => self.uniform.as_ref(),
            ContainerSlot::Vest => self.vest.as_ref(),
            ContainerSlot::Backpack => self.backpack.as_ref(),
        }
    }

    pub fn container_mut(&mut self, slot: ContainerSlot) -> &mut Option<Container> {
        match slot {
            ContainerSlot::Uniform => &mut self.uniform,
            ContainerSlot::Vest => &mut self.vest,
            ContainerSlot::Backpack => &mut self.backpack,
        }
    }

    pub fn containers(&self) -> impl Iterator<Item = &Container> {
        [&self.uniform, &self.vest, &self.backpack]
            .into_iter()
            .flatten()
    }

    pub fn containers_mut(&mut self) -> impl Iterator<Item = &mut Container> {
        [&mut self.uniform, &mut self.vest, &mut self.backpack]
            .into_iter()
            .flatten()
    }

    pub fn link(&self, slot: LinkSlot) -> Option<&str> {
        self.links[slot.index()].as_deref()
    }

    pub fn set_link(&mut self, slot: LinkSlot, class: Option<String>) {
        self.links[slot.index()] = class;
    }

    /// The equipped weapon of a slot.
    pub fn weapon(&self, slot: WeaponSlot) -> Option<&Weapon> {
        self.weapons.iter().find(|w| w.slot == slot)
    }

    pub fn weapon_mut(&mut self, slot: WeaponSlot) -> Option<&mut Weapon> {
        self.weapons.iter_mut().find(|w| w.slot == slot)
    }

    /// Equips `weapon`, replacing (and dropping) the weapon of its slot; the new one goes to the
    /// end of the list.
    pub fn equip(&mut self, weapon: Weapon) {
        self.weapons.retain(|w| w.slot != weapon.slot);
        self.weapons.push(weapon);
    }

    /// Stores `entry` in the first container of `order` that allows and fits it.
    pub fn store(&mut self, entry: Stored, allowed: &[i32], order: &[ContainerSlot]) -> bool {
        let mass = entry.mass();
        for &slot in order {
            if !allowed.is_empty() && !allowed.contains(&slot.slot_type()) {
                continue;
            }
            if let Some(c) = self.container_mut(slot) {
                if c.fits(mass) {
                    match entry {
                        Stored::Item(i) => c.items.push(i),
                        Stored::Weapon(w) => c.weapons.push(w),
                        Stored::Magazine(m) => c.magazines.push(m),
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Takes the first stored magazine (uniform, vest, backpack) that `accept` accepts.
    pub fn take_magazine(&mut self, accept: impl Fn(&Magazine) -> bool) -> Option<Magazine> {
        for c in self.containers_mut() {
            if let Some(i) = c.magazines.iter().position(&accept) {
                return Some(c.magazines.remove(i));
            }
        }
        None
    }

    /// Takes the first stored item of class `class` (any case).
    pub fn take_item(&mut self, class: &str) -> Option<StoredItem> {
        for c in self.containers_mut() {
            if let Some(i) = c
                .items
                .iter()
                .position(|e| e.class.eq_ignore_ascii_case(class))
            {
                return Some(c.items.remove(i));
            }
        }
        None
    }

    /// Takes the first stored weapon of class `class` (any case).
    pub fn take_weapon(&mut self, class: &str) -> Option<Weapon> {
        for c in self.containers_mut() {
            if let Some(i) = c
                .weapons
                .iter()
                .position(|e| e.class.eq_ignore_ascii_case(class))
            {
                return Some(c.weapons.remove(i));
            }
        }
        None
    }

    /// Mass of everything carried: containers with contents, weapons, worn items.
    pub fn total_mass(&self, item_mass: impl Fn(&str) -> f32) -> f32 {
        let containers: f32 = self.containers().map(|c| c.mass + c.load()).sum();
        let weapons: f32 = self.weapons.iter().map(Weapon::total_mass).sum();
        let worn: f32 = self
            .headgear
            .iter()
            .chain(self.goggles.iter())
            .chain(self.links.iter().flatten())
            .map(|c| item_mass(c))
            .sum();
        containers + weapons + worn
    }
}
