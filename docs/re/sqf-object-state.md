# Unit and vehicle state commands (arma3_x64.exe 2.22.0.154103)

Captive, unit position, AI feature switches, skill, rank, fuel and supply cargo, locks, engine,
texture and material overrides, and vehicle cargo. Sources: the decompiled handlers (RVAs from
`docs/re/sqf-commands.tsv`) and oracle runs of `tools/oracle/probes/98_object_state_vr.probes` on
`arma3server_x64.exe` (`ost.*` and `cargo.*`, 26 probes, all matching). Implemented in
`crates/a3-world/src/object_state.rs` (the per-Entity state, a side table of the World) and
`src/script/object_state.rs` (the commands), with three integration points outside it: `side` of a
captive unit is civilian (`script/groups.rs`), and `load`/`loadAbs` of a vehicle or box read its
cargo mass (`script/inventory.rs`). Class checks: `0x1420ba9a0` EntityAI, `0x1420ba620` Transport,
`0x1420b98cc` Entity (see `sqf-object-commands.md`).

**An unknown enum name is logged, not raised.** `setUnitPos "bad"`, `disableAI "NOSUCH"` and
`setRank "bogus"` write the engine's "Unknown enum value" line to the RPT, leave the state alone
(`setUnitPos`, `disableAI`) or apply PRIVATE (`setRank`), and the script carries on (oracle).


## Units

| Command | Handler | Behaviour |
|---|---|---|
| `setCaptive` | 0x53b1a0 | An alive EntityAI with a brain: `brain + 0x240` = `true` 1 / `false` 0 / a number truncated. A captive unit's `side` is civilian; its group keeps its side. Vehicles: nothing. |
| `captive` / `captiveNum` | 0x526070 | `captiveNum > 0` / the number; `false`/0 otherwise. |
| `setUnitPos` / `unitPos` | 0x53fc20 / 0x533390 | Enum `UP`, `DOWN`, `MIDDLE`, `AUTO` (any case; printed `"Up"`, `"Down"`, `"Middle"`, `"Auto"`, default `"Auto"`). An unknown name keeps the stance (oracle). |
| `disableAI` / `enableAI` / `checkAIFeature` | 0x526960 / 0x527420 / 0x481de0 | Bits (enum at 0x1400e7b30): TARGET 0x1, MOVE 0x2, AUTOTARGET 0x4, ANIM 0x8, TEAMSWITCH 0x10, FSM 0x40, WEAPONAIM 0x80, AIMINGERROR 0x100, SUPPRESSION 0x200, CHECKVISIBLE 0x400, COVER 0x800, AUTOCOMBAT 0x1000, PATH 0x2000, MINEDETECTION 0x4000, NVG 0x8000, LIGHTS 0x10000, RADIOPROTOCOL 0x20000, FIREWEAPON 0x40000, COMMAND 0x80000, HEARING 0x100000, ALL 0xffffffff. An unknown name changes nothing (oracle). The unary `checkAIFeature` reads the global switches (`enableAIFeature`); `"MOVE"` is false. |
| `setRank` / `setUnitRank` / `rank` / `rankId` | 0x5660a0 / 0x5660c0 / 0x565e60 / 0x565f80 | PRIVATE 0 … COLONEL 6, any case. An unknown name sets PRIVATE (oracle). `rank objNull` is `""`. |
| `setSkill` (number) / `setUnitAbility` | 0x8b8100 / 0x566000 | General skill clamped to 0..1; default 0.5. |
| `setSkill [name, value]` | 0x53f3b0 | One sub-skill. `unit skill name` gives it, else the general skill; `skill unit` the general one (oracle: after `setSkill 0.3; setSkill ["aimingAccuracy", 0.9]`: 0.3 / 0.9 / spotTime 0.3). |
| `allowFleeing` / `fleeing` | 0x1907c0 / 0x528630 | Cowardice, 1 maximum and 0 disabling fleeing (wiki), clamped 0..1 and stored on the unit; a group sets every man of it. No morale model yet: the group tick breaks a unit whose value is above its threshold, and `fleeing` hands the stored value back rather than the engine's Boolean (`ai.md` §4, §7). |

## Vehicles

| Command | Handler | Behaviour |
|---|---|---|
| `setFuel` | 0x53c440 | Alive EntityAI: fuel = value × `fuelCapacity` (`AddFuel` of the difference, clamped 0..1 of capacity). |
| `fuel` | 0x528860 | fuel / capacity; 1 without a tank (units); 0 for non-EntityAI and null. |
| `setFuelCargo` / `setAmmoCargo` / `setRepairCargo`, `get...Cargo` | 0x53c510 ... | Fraction of `transportFuel` / `transportAmmo` / `transportRepair`; -1 when the type has none (a fuel truck's `getAmmoCargo` stays -1 after `setAmmoCargo`). |
| `setObjectTexture(Global)` / `getObjectTextures` | 0x8b8550 / 0x8b8590 → 0x8b85d0; 0x4ad560 | One entry per `hiddenSelections[]`; defaults are `hiddenSelectionsTextures[]` lower-cased without the leading `\`, `""` past their end. `[index or selection name, texture]`; an index outside the selections is ignored; a texture that is not procedural (`#(...)`) and not a file is refused ("Warning Message: Picture %s not found"). |
| `setObjectMaterial(Global)` / `getObjectMaterials` | 0x8b8260 / 0x4ac4d0 | The same, defaults `""`. |
| `lock` | 0x536850 | Transport: `true` 2, `false` 1, a number rounded and **not** clamped (`lock 7` → 7) at `+0xe5c`. |
| `setVehicleLock` | 0x570440 | `UNLOCKED` 0, `DEFAULT` 1, `LOCKED` 2, `LOCKEDPLAYER` 3. |
| `locked` | 0x537110 | The value, default 1; -1 for non-Transport and null. |
| `lockDriver` / `lockedDriver`, `lockCargo` (bool / `[index, bool]`) / `lockedCargo` | 0x56d570 / 0x56cc40, 0x56d270 / 0x56d390 / 0x56cad0 | Driver and cargo seat locks. |
| `engineOn` / `isEngineOn` | 0x569c30 / 0x56cca0 | The engine flag; an empty tank does not stop `engineOn true` at once (oracle). |
| `flyInHeight` / `flyInHeightASL` | 0x53c0b0 / 0x53c2a0 | An aircraft's commanded altitude: above the ground below it (`flyInHeight`, `[height, forced]` taking only the height) or above sea level (`flyInHeightASL`, `[standard, combat, stealth]`). Stored on the aircraft; the air step holds the higher of the two (`sim-air.md` §5). |

## Vehicle cargo

Cargo of non-Man EntityAI (vehicles, boxes), created from the type's `TransportItems` (`name`),
`TransportMagazines` (`magazine`), `TransportWeapons` (`weapon`) and `TransportBackpacks`
(`backpack`) classes (`count` each).

| Command | Behaviour (oracle) |
|---|---|
| `clear{Item,Magazine,Weapon,Backpack}Cargo(Global)` (0x83d7b0 ...) | Local EntityAI only; a unit's gear is not touched. |
| `add{Item,Magazine,Weapon,Backpack}Cargo(Global)` (0x8391e0 ...) | `[class, count]`; fewer than 2 elements adds nothing and runs on, count rounded, < 1 adds nothing; no capacity check (100 toolkits fit an ammo box). `addItemCargo` routes magazines to the magazine cargo and weapons to the weapon cargo, ignores backpacks; `addMagazineCargo` with a non-magazine logs 'Warning: "%s" is not a valid magazine name'; `addWeaponCargo` takes any CfgWeapons class (a first aid kit becomes weapon cargo). |
| `itemCargo` & co. | Every entry in the order added. A unit lists its containers' items / magazines / weapons. |
| `getItemCargo` & co. | `[[classes], [counts]]` grouped in first-seen order; `[[],[]]` for units and null. |
| `load` / `loadAbs` | A vehicle's or box's cargo mass over the type's `maximumLoad` (oracle: 100 toolkits in an ammo box give `load` 8, 8000 over 1000); a unit's worn gear, as in `sqf-inventory.md`. |
