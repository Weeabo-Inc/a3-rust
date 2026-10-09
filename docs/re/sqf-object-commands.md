# SQF object and group state commands (arma3_x64.exe 2.22.0.154103)

Variables on objects and groups, the player, identities, vehicle variable names, synchronization
and dynamic simulation flags. Read from the decompiled handlers (RVAs from
`docs/re/sqf-commands.tsv`; VA = RVA + 0x140000000). Implemented in
`crates/a3-world/src/script/identity.rs` over `crates/a3-world/src/script_state.rs`.

Common helpers: `FUN_1408fdb70` turns a GameValue into `Object*` (null for objNull),
`FUN_1404a2f50` into `AIGroup*`. `obj->IsKindOf(type)` is vtable `+200`. Type descriptors seen
here, mapped by the vtable slots the handlers call afterwards: `0x1420b913c` Person,
`0x1420ba9a0` EntityAI (the `+0x12c0` / `+0x12a8` / `+0x12b0` brain getters are within EntityAI's
643 vtable slots), `0x1420b98cc` Entity, `0x14209dadc` Man (only in `setFace`; medium
confidence). `FUN_1402eb6d0(state, 0x16, got, expected)` is the DIM error "%d elements
provided, %d expected"; `FUN_140171db0(state, value, type)` is a type check that raises
"Type X, expected Y".

## Variables

| Command | Handler | Behaviour |
|---|---|---|
| `OBJECT setVariable ARRAY` | 0x540020 → 0x46d220 | Null object: nothing, **before** any argument check. Size < 1: DIM (n, 1). Element 0 must be STRING. Size not 2 or 3: DIM (n, 3). With 3, element 2 must be BOOL, SCALAR or ARRAY (public: all clients / one client id / a list). An empty name does nothing. The name is lower-cased (`FUN_1403b7cc0` checks, `FUN_140476d20` lowers). The variable space is `obj->GetVars()` (vtable `+0x370`); none → nothing. |
| `GROUP setVariable ARRAY` | 0x4b9030 → 0x46bfb0 | The same on the group's space (`group + 0x338`). |
| `OBJECT getVariable STRING` | 0x533450 | Null object: nil (a debug build prints "getVariable called on null object"). Else the value, nil when unset. |
| `OBJECT getVariable ARRAY` | 0x533450 | Size must be exactly 2, else DIM (n, 2) — checked before the null test. Element 0 must be STRING. Null object, unset variable or a nil value (`GameData +0x88` IsNil) → element 1. |
| `GROUP getVariable STRING/ARRAY` | 0x4b8ab0 | The same for groups ("getVariable called on null group"). |
| `allVariables OBJECT` / `GROUP` | 0x49b2e0 / 0x49a4d0 | The names in the space (lower case), hash-table order; null → `[]`. a3-world sorts them. |

The public flag's broadcast is not implemented (no network layer yet, #131). Variables of a
Static object are kept under its Static key _(uncertain: whether the original's plain `Object`
has a variable space; vtable `+0x370` was not followed)_.

## Player and vehicle

| Command | Handler | Behaviour |
|---|---|---|
| `player` | 0x8b1120 | `GWorld->PlayerOn()` (`GWorld + 0x2d88`), objNull without one. A branch for a special client mode reads the player from the network manager instead. |
| `cameraOn` | 0x8a5bf0 | `GWorld + 0x2d50`, the camera's vehicle. a3-world: set explicitly, else the player. |
| `vehicle OBJECT` | 0x542510 | An EntityAI with a brain (`+0x12c0`): the brain's vehicle (`FUN_14134ae20`). Anything else, objNull included, is returned unchanged. a3-world has no crew yet, so every object is its own vehicle. |
| `isPlayer OBJECT` | 0x535b00 | EntityAI with a brain whose player flag is set (`FUN_14134b9e0`). |
| `isPlayer ARRAY` | 0x535b70 | `[unit]`: a Person; with a brain as above, without one (dead) true when it is the local player or a remote player's person (`FUN_140e2e220`). |

## Identity

Person fields: `+0x168*8` speaker type, `+0x169*8` name, `+0x16a*8` first name, `+0x16b*8`
last name, `+0x16c*8` face, `+0x16d*8` glasses, `+0x16e*8` speaker name, `+0x16f*8` pitch
(float), `+0x174*8` name sound.

| Command | Handler | Behaviour |
|---|---|---|
| `OBJECT setName STRING` | 0x553560 | Person only; sets the name. |
| `OBJECT setName ARRAY` | 0x553610 | Null: nothing (before checks). Exactly 3 elements else DIM (n, 3); each STRING; Person only: name, first, last. |
| `name OBJECT` | 0x538610 | Null → `"Error: No vehicle"`. Person → its name (empty name logs "WARNING: Function 'name' - %s has empty name"). Other EntityAI → the name of its commander, else driver, else gunner unit; none → `"Error: No unit"` (and a warning). Anything else → `"Error: No vehicle"`. The original gives units a random name at creation; a3-world returns `""` until that exists. |
| `OBJECT setFace STRING` | 0x53ba30 | Man only; calls the virtual `SetFace(face, name)` (`+0x1c70`); the face `"custom"` (compared case-insensitively) uses the player's custom face. Stored as given. |
| `face OBJECT` | 0x528110 | EntityAI and Person → `+0x16c`; else `""`. |
| `OBJECT setPitch SCALAR` | 0x553880 | Person; stores the pitch and updates the radio voice. |
| `pitch OBJECT` | 0x538940 | Returns a **number** (the table declares STRING): the pitch, `-1` for anything that is not a Person. |
| `OBJECT setSpeaker STRING` | 0x553940 | Person; sets the speaker and updates the radio voice. |
| `speaker OBJECT` | 0x5418f0 | Person → the speaker name, else `""`. |
| `OBJECT setNameSound STRING` / `nameSound OBJECT` | 0x5537d0 / 0x5388b0 | Person; `""` for anything else. |
| `OBJECT setIdentity STRING` | 0x53d440 | Person. The class is looked up in `CfgIdentities` of the mission config (MissionManager), then the campaign config (`0x1421e91b0`), then `configFile` (`0x142161f90`) — `FUN_140490640`. Not found → nothing. Found: `name` (first/last name cleared), `face`, `glasses`, `speaker`, `pitch` (≤ 0.001 logs "Setting invalid pitch %.4f for %s" but is kept), `nameSound`. |

## Vehicle variable name

| Command | Handler | Behaviour |
|---|---|---|
| `OBJECT setVehicleVarName STRING` | 0x571f00 | Entity only (vtable `+0xa68`). |
| `vehicleVarName OBJECT` | 0x56ca10 | Entity → vtable `+0x658`; else `""`. |

`str` of an object (`GameDataObject` text → `Object::GetDebugName`, vtable `+0x20`): Man's
override `0xfb0330` returns the variable name (`+0x4f0`) when it is not empty, else the brain's
name (`"B Alpha 1-1:1"`), else `"<address># <id>: <model>"` plus `" REMOTE"` for a remote
object. A plain Entity (`0x20c010`) prints the address form only. a3-world applies the variable
name to every EntityAI. The engine sets the variable name of SQM-named units, so `str unit`
prints the mission's name for them.

## Synchronization

`FUN_1404b3f90(object)` classifies an object: an EntityAI with a brain is kind 0 (its list is
on the AI unit, `brain + 0x110`, and the object reported is the unit's person); a
`0x1420b8614` object (a trigger, `+0xe4*8` list) is kind 1; anything else kind 2 with no list.

| Command | Handler | Behaviour |
|---|---|---|
| `OBJECT synchronizeObjectsAdd ARRAY` | 0x5244f0 | Null source: nothing, before checks. Every element must be OBJECT, else a type error and nothing changes. Per element: when the table at `0x141acc7e0` (`[from * 3 + to]` = `01 01 01 / 01 00 00 / 00 00 00`) allows the pair, the element is added to the source's list (if it has one and the element is not null) and the source to the element's list (if it has one). No duplicates. |
| `OBJECT synchronizeObjectsRemove ARRAY` | 0x539680 | The element type check runs even for a null source; then each pair is removed from both lists, without the table. |
| `synchronizedObjects OBJECT` | 0x532f20 | The source's list without null entries; `[]` without a list. |

## Dynamic simulation

| Command | Handler | Behaviour |
|---|---|---|
| `GROUP enableDynamicSimulation BOOL` | 0x17e8c0 | Adds the group to / removes it from the dynamic simulation manager (`FUN_141147fa0(GWorld)`). |
| `OBJECT enableDynamicSimulation BOOL` | 0x17e980 | EntityAI only. |
| `dynamicSimulationEnabled GROUP` / `OBJECT` | 0x17ea50 / 0x17eab0 | The manager's flag; the object form accepts any Entity. |

The flags are stored; the dynamic simulation system that freezes far objects is not
implemented yet.
