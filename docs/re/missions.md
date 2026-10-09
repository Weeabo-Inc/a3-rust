# Missions: `mission.sqm`, `description.ext` and the mission scripts

How the engine reads a mission folder, and what `crates/a3-mission` implements from it. Findings
are marked _(verified)_ when a shipped file or a test shows them, _(assumed)_ when they come from
the community wiki or from the binary and have not been checked against data yet.

## Where missions live

Missions ship inside PBOs; there is no loose `missions/` folder in the install. A mission is a
virtual folder (verified):

```
a3\missions_f_bootcamp\campaign\missions\boot_m02.altis\
    mission.sqm              30942 B, text config (many are rapified)
    description.ext          440 B
    init.sqf                 5230 B
    intro.sqf, missionflow.fsm, missiontasks.sqf, missionconversations.sqf, setup\...
```

The folder's extension names the world (`boot_m02.altis` → Altis); it is the only place the
mission says which terrain it belongs on _(verified: 237 `mission.sqm` in the install, all
`version=12`)_. `a3-config`'s real-data test reads and rap-round-trips all 237 (`docs/re/config.md`).

## File structure

`mission.sqm` is config syntax (or its rapified form): `version=12;` followed by one class per
**scene** — the editor keeps a separate placement list for each (verified in `boot_m02.altis`):

```
version=12;
class Mission     { class Intel; class Groups; class Vehicles; class Markers; class Sensors; };
class Intro       { class Intel; ... };
class OutroWin    { class Intel; ... };
class OutroLoose  { class Intel; ... };
```

Every scene carries its own `addOns[]`, `addOnsAuto[]`, `randomSeed` and `class Intel` (the brief:
`briefingName`, date, weather, wind, fog) — so scenes can, and in `boot_m02` do, have different
seeds. Each placement list is `items=N;` plus `class Item0..ItemN-1` — **the item index is not the
entity's `id`** (verified: `Item1` can hold `id=1`, and the first group of `boot_m02` holds `id=0`
with `Item0`):

```
class Groups
{
    items=26;
    class Item0
    {
        side="WEST";
        class Vehicles { items=1; class Item0 { position[]={6687.8,48.0,15982.8}; azimut=-5;
            id=0; side="WEST"; vehicle="B_Soldier_TL_F"; player="PLAYER COMMANDER"; leader=1;
            rank="SERGEANT"; skill=0.6; text="BIS_lacey"; init="this enableSimulation false; ..."; }; };
        class Waypoints { ... };
    };
};
class Vehicles { ... }   // objects outside any group, e.g. crates and empty cars
class Markers  { ... }
class Sensors  { ... }   // triggers
```

Fields, as shipped missions use them:

| key | meaning | notes |
|---|---|---|
| `position[]={x, y, z}` | east, **height above sea level**, north | `{x, y}` (two elements) means "place on the surface" _(assumed)_; world space is `(x, z, y)` (ADR 0003) |
| `azimut` | degrees clockwise from north, `getDir` convention | negative values occur (`-5`, `-131`) and wrap _(verified)_ |
| `placement` | `"CAN_COLLIDE"` etc. | absent on most units |
| `special` | `"NONE"`, `"FLY"`, `"FORM"`, `"CARGO"` | on every placed object in `boot_m02` |
| `player` | `"PLAYER COMMANDER"` and friends | marks the player-controlled unit |
| `leader=1` | this unit is the group's leader | more than one group in `boot_m02` has `leader=1` on its vehicle only |
| `text` | the editor's variable name | becomes a `missionNamespace` variable at start |
| `init` | SQF expression run with `this` = the unit | see below |
| `presence` | editor probability of presence, 0 = absent | `boot_m02` carries briefing-only units with `presence=0` _(verified: 69 units, 64 spawned)_ |
| `lock`, `rank`, `skill` | as in the editor | not applied by the World yet |

Triggers (`class Sensors`) carry `a`/`b` (semi-axes), `angle`, `activationBy`, `activationType`,
`repeating`, `age`, `idVehicle`, `expCond`, `statements` (the On Activation expression; older
files use `expActiv`), `synchronizations[]` and `syncId`. Waypoints carry `position`,
`placement`, `type` (`"MOVE"` when absent), `speed`, `combatMode`, `behaviour`, `formation`,
`description`, `timeout`, `synchronizations[]`, `syncId`.

## Start-up order

The engine's order (community wiki "Initialisation Order", Arma 3 table; implemented in
`crates/a3-mission/src/run.rs`):

1. object init event handlers,
2. each placed unit's `init` field, **unscheduled** — so `sleep`/`waitUntil` in one is an error
   ("Suspending not allowed in this context"),
3. `init.sqs`, then `init.sqf`, **scheduled** (they may `sleep` and `waitUntil`).

Two consequences the VM must honour:

- An init field's object is bound to **`this`**. The VM's own parameter variable is `_this`
  (`Sym::LOCAL_BIT`); the engine binds `this` as well, and shipped init fields rely on it
  (`this allowDamage false;`, `this setBehaviour "SAFE";`). Outside an init field `this` is
  undefined _(verified by test: a mission whose init reads `this` fails without the binding)_.
- Scripts a mission `execVM`s are not `init.sqf`: they are spawned scheduled during start-up and
  run on after it, which is why the engine's "mission loaded" state is not "scripts finished".

## What the campaign missions need

`boot_m02.altis` (bootcamp, mission 02) is the reference mission in
`crates/a3-mission/tests/real_data.rs`: 26 groups, 69 units (37 with an `init`), 18 waypoints,
37 ungrouped objects, 12 markers, 4 triggers; 64 units spawn over the shipped config. Compiling
every `init` field and `init.sqf` and asking the registry which commands have no implementation
gives **42 commands**, headed by `allowDamage` (13×), `clearMagazineCargo`/`clearWeaponCargo`/
`clearItemCargo`/`clearBackpackCargo` (7× each), `setBehaviour` (7×), `addEventHandler` (6×),
`setMarkerPos` (6×), `enableMimics` (4×), `name`, `removeEventHandler`, `setIdentity`,
`disableAI`. These are the short list for making the bootcamp campaign run.

## Open questions

- Which `class Mission` fields the engine applies beyond `addOns`: `addOnsAuto` is used by the
  editor, and which scene's `randomSeed` seeds the mission RNG is not known _(assumed: the
  `Mission` scene's)_.
- Whether the two-element `position[]` means surface placement or "at height 0" _(assumed_; the
  World treats it as surface placement, `spawn.rs`).
- `special` ("FLY"/"FORM"/"CARGO") semantics: `CARGO` needs a carrier's cargo index, which the
  World has no model for yet.
- Whether `Intro`/`OutroWin`/`OutroLoose` scenes are loaded at mission start or only when the
  intro/outro plays _(assumed: the latter; `a3-mission` parses `class Mission` only)_.
