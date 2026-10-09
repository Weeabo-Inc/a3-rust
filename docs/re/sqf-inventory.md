# Unit inventory (arma3_x64.exe 2.22.0.154103)

How the engine models a unit's gear and what the SQF inventory commands do. Sources: the
decompiled handlers (RVAs from `docs/re/sqf-commands.tsv`) and oracle runs of
`tools/oracle/probes/97_inventory_vr.probes` on `arma3server_x64.exe` (48 probes; our engine
matches 40, see the end). Implemented in `crates/a3-world/src/{inventory,loadout}.rs` and
`src/script/inventory.rs`.

## Item types and slots

Every inventory item has a type number (`ItemInfo >> type`, or implied by `simulation`). The item
factory `FUN_141688430` switches on it to create the slot item class; `FUN_14084c400` names the
types in error messages. High confidence.

| Type | Name | Item class | Config |
|---:|---|---|---|
| 0 | Default | | |
| 101 / 201 / 301 / 302 | Muzzle / Optics / Flashlight / UnderBarrel | weapon attachments | `ItemInfo >> type` |
| 401 | FirstAidKit | | |
| 602 / 603 / 604 | NVG / Goggles / Scuba | `CNVGSlotItem` / `CGogglesSlotItem` / `CScubaSlotItem` | CfgGlasses are 603 |
| 605 | Headgear | | |
| 608–612 | Map / Compass / Watch / Radio / GPS | `CMapSlotItem` ... `CGPSSlotItem` | `simulation = "ItemMap"` etc. |
| 613 | Para | `CParaSlotItem` | |
| 616 | HMD | | `NVGoggles` (`ItemInfo type = 616`) |
| 617 / 618 | Binocular / MineDetector | | |
| 619 / 620 / 621 | Medikit / Toolkit / UavTerminal | | |
| 701 / 801 | Vest / Uniform | container items | `ItemInfo >> containerClass` → `CfgVehicles >> maximumLoad` |
| 901 | Backpack | backpack container | `CfgVehicles`, `isBackpack = 1` |
| 1001 | Backpack2 | `CBackpack2SlotItem` | |

An "inventory item" (`FUN_141689a20`) is a CfgGlasses class or a CfgWeapons item; weapons,
magazines and backpacks are not. `linkItem`, `addHeadgear` & co. reject anything else with the
RPT line "Inventory item with given name: [%s] not found".

## Containers

`FUN_141681fe0(mask)` turns a container mask into the order containers are tried: `0x80`/`0x100`
→ uniform (4), vest (8), backpack (2); `0x10` → uniform, vest; `0x200` → vest, uniform,
backpack; `0x400` → backpack, vest, uniform; 2/4/8 → that one. The first container that accepts
the item takes it (`FUN_1416825d0`).

A container accepts an item (`FUN_1416af5d0`) when its type is in the item's `allowedSlots[]`
(701/801/901; no list = anywhere) and `load + mass <= maximumLoad`. (A second check against the
unit's total load exists; not reproduced.) A container lists its items, then weapons, then
magazines (oracle: an `ItemGPS` added after magazines is listed first).

## Default loadout

From the unit's CfgVehicles class: `uniformClass`; `linkedItems[]` (vest, headgear, link slots);
`backpack` with its `TransportItems` (`name`), `TransportWeapons` (`weapon`) and
`TransportMagazines` (`magazine`) contents; `weapons[]` with their `LinkedItems`
(`slot = "CowsSlot"`/`PointerSlot`/`MuzzleSlot`/`UnderBarrelSlot`); `items[]`; `magazines[]`.
Magazines are stored with mask `0x100` (`FUN_140fc6bc0`); then each weapon loads the first
compatible stored magazine (uniform first; compatible = `magazines[]` plus the lists of its
`magazineWell[]` classes in `CfgMagazineWells`); the magazines that did not fit are stored
again — loading freed room — and the rest are dropped with "soldier[%s]:Some of magazines
weren't stored in soldier Vest or Uniform?". The oracle confirms this exact distribution for
`B_Soldier_F`, `B_Soldier_AR_F` and `B_soldier_LAT_F`.

Not reproduced: random facewear and headgear (`identityTypes`, `headgearList` — `B_Soldier_F`
gets `G_Combat`, `C_man_1` a random cap or bandanna).

## Commands

| Command | Handler | Behaviour (oracle-confirmed unless marked) |
|---|---|---|
| `weapons` | 0x83e540 → 0x82e950 | Equipped weapons in the order added (a replaced primary goes last), then weapons in containers. `Throw`/`Put` are not listed. |
| `primaryWeapon` & co. | 0x841160 ... | The weapon of the slot (`type` bit 1 primary, 2 handgun, 4 secondary, 4096 binocular), `""`. |
| `primaryWeaponItems` & co. | | `[muzzle, pointer, optic, bipod]`. |
| `primaryWeaponMagazine` & co. | 0x84d700 ... | The loaded magazine as a one-element array, `[]`. |
| `items` | 0x83cc40 → 0x82c400 | Per container: items and weapons. |
| `magazines` | 0x83dfa0 | Stored, non-empty magazines; loaded ones are not listed. |
| `magazinesAmmo` | 0x83e090 | `[class, ammo]` of stored magazines; `[unit, true]` includes empty ones. |
| `uniformItems` / `vestItems` / `backpackItems` | | The container's listing. |
| `assignedItems` | 0x83c530 | Map, compass, watch, radio, GPS, HMD. `[unit, a, b]` needs 3 elements (DIM); when `a` is true goggles and headgear are appended — `b` is not checked. |
| `hasWeapon` | 0x5353e0 | Equipped, stored or pseudo weapons (`Throw` is true), any case. |
| `load` / `loadAbs` / `loadUniform` ... | 0x83dd80 ... | Mass sums; `load` = `loadAbs / maximumLoad`. `B_Soldier_F`: 476 with its `G_Combat` (472 without). |
| `addHeadgear` / `addGoggles` / `addVest` / `addUniform` / `forceAddUniform` | 0x8436e0 → 0x843710 → 0x8438c0 | Empty name / unknown: RPT only. The slot is cleared first; an item of another type then raises "Tried to add inventory item with type '%s' into slot of type '%s'" (the slot stays empty). `addUniform` refuses a uniform whose `uniformClass` soldier's `modelSides[]` lacks the unit's side (`FUN_140734490`; the old uniform is still gone). A new uniform or vest is empty. A remote unit's change is sent to its owner. |
| `addBackpack` | 0x8319f0 | Replaces the backpack (the old one goes to the ground — not reproduced); config contents included. |
| `removeHeadgear` ... `removeBackpack`, `removeAllContainers` | | Clear the slot with its contents. |
| `linkItem` | 0x842c20 | An inventory item into its slot (link slots, headgear, goggles, vest, uniform), replacing; anything else ignored. |
| `unlinkItem` | 0x847510 → 0x84a0f0 | Removes an assigned/worn item. |
| `unassignItem` | 0x847170 | Moves an assigned item into the containers. |
| `assignItem` | 0x83ccd0 | Moves a stored item into its slot; a missing one: RPT only. |
| `removeAllAssignedItems` | 0x84b460 | The link slots; `[unit, goggles, headgear]` also those. |
| `addWeapon` | 0x83b700 → 0x83bc10 | Replaces the weapon of its slot (dropped with magazine and attachments), adds the linked attachments, loads the first compatible stored magazine. An inventory item is linked (as `linkItem`). Unknown: RPT "Weapon type with given name: [%s] not found". |
| `removeWeapon` | 0x8462d0 | An inventory item is unlinked; else the weapon with its magazine. |
| `removeAllWeapons` | 0x844790 | All weapons and all container contents (items too). |
| `addPrimaryWeaponItem` & co. | 0x8396a0 ... | An attachment replaces its kind; a compatible magazine replaces the loaded one; else ignored. |
| `removePrimaryWeaponItem` & co. | 0x846290 ... | Removes the attachment or the loaded magazine. |
| `addMagazine` | 0x839ba0 / 0x83a4a0 | Mask 0x80. `[class, ammo]`: negative or above the count → full, 0 → empty magazine. Not a magazine: RPT 'Warning: "%s" is not a valid magazine name'. Thrown magazines also load the `Throw` muzzle in the original (not reproduced). |
| `addMagazines` | 0x83b290 | `[class, n]`. |
| `removeMagazine` / `removeMagazines` | 0x844c90 / 0x845940 | One / all stored magazines of the class; loaded ones stay. |
| `addItem`, `addItemToUniform` / `Vest` / `Backpack` | 0x839160 / 0x8391a0 / 0x8391c0 / 0x839180 → 0x848380 | Masks 0x80 / 4 / 8 / 2. Items, weapons and magazines. |
| `removeItem` | 0x844880 | The first stored item, weapon or magazine; assigned items stay. |
| `removeItems` | 0x844a60 | Every stored item of the class (a magazine class is looked up as a weapon and fails). |
| `removeAllItems` / `removeAllItemsWithMagazines` | 0x844240 / 0x844170 | Stored items / and magazines. |

Commands on anything that is not a Man return `""`/`[]`/nothing in a3-world. The original lists
a vehicle's turret weapons (`weapons car` → `["TruckHorn2"]`) and has vehicle cargo — next batch.

## Oracle status

40 of 48 probes match. The rest: random facewear/headgear (`default_slots`, `remove_all_assigned`,
`civilian`, `load`), handler errors that continue the script in the original (#271:
`default_assigned_array`, `add_headgear_wrong_type`), vehicle weapons and cargo
(`null_and_vehicle`, `vehicle_add`).
