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
mission says which terrain it belongs on. `a3-config`'s real-data test reads and rap-round-trips
every `mission.sqm` (`docs/re/config.md`).

Two editors wrote the shipped files _(verified by the scenario sweep, `docs/fidelity/`)_: of the
238 scenarios (Mission fragments left out), about half are the 2D editor's `version=12` and half
the 3D editor's (Eden) `version=52`, `53` or `54`, which has a different layout (below).

## File structure (2D editor, `version=12`)

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
| `position[]={x, y, z}` | east, **height above sea level**, north | world space as it is: X east, Y up, Z north (ADR 0003) _(verified: `boot_m02` `{6687.77, 48.0, 15982.78}`)_; `{x, y}` (two elements, east and north) means "place on the surface" _(assumed)_ |
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

## File structure (3D editor, `version=52..54`)

_(Verified in `showcase_combined_arms.stratis`, `tanks_m01.altis`, `faction_blufor.altis`,
`malden_intro.malden`; implemented in `crates/a3-mission/src/mission.rs`.)_ The header moves to
the root (`addons[]`, `randomSeed`, `class EditorData`, `class ScenarioData`,
`class CustomAttributes` for scenario attributes), and each scene has one list:

```
version=53;
addons[]={...};
randomSeed=1028852;
class Mission
{
    class Intel { briefingName=...; year=...; ... };
    class Entities
    {
        items=51;
        class Item0 { dataType="Marker"; position[]={3881.63,20.69,5505.40}; name="BIS_insertion"; type="mil_start"; id=262; };
        class Item1
        {
            dataType="Group"; side="West"; id=288;
            class Entities { items=2;
                class Item0 { dataType="Object"; class PositionInfo { position[]={...}; angles[]={0,4.1089,0}; };
                    side="West"; flags=6; class Attributes { name="BIS_x"; init="..."; isPlayer=1; skill=0.6; rank="SERGEANT"; };
                    id=289; type="B_Soldier_F"; atlOffset=2.46; class CustomAttributes {...}; };
                class Item1 { dataType="Waypoint"; position[]={...}; type="Move"; id=300; };
            };
            class Attributes {};
            class CrewLinks { class Links { class Item0 { item0=289; item1=288; class CustomData { role=1; }; }; }; };
        };
        class Item2 { dataType="Trigger"; position[]={...}; class Attributes { condition="..."; onActivation="..."; sizeA=...; sizeB=...; activationBy=...; repeatable=1; type="SWITCH"; }; type="EmptyDetector"; id=...; };
        class Item3 { dataType="Logic"; class PositionInfo {...}; name="BIS_logic"; init="..."; presenceCondition="false"; type="ModuleDoorOpen_F"; id=...; };
        class Item4 { dataType="Layer"; name="Empty Vehicles"; class Entities {...}; };
        class Item5 { dataType="Comment"; ... };
    };
    class Connections { ... };   // synchronisations, trigger owners (not read yet)
};
```

- `dataType` says what an item is: `Group` (its `class Entities` holds `Object`/`Logic` units and
  `Waypoint`s), `Object`/`Logic` outside a group, `Marker`, `Trigger`, `Layer` (an editor folder
  with its own `class Entities`, read recursively), `Comment` (ignored).
- `position[]` is `{east, height, north}` like the 2D editor's, with the height of the **surface
  under the entity**; `atlOffset` lifts the entity above it (helicopter crew: `atlOffset=54`;
  markers on the sea floor have negative heights) _(verified in the files; that the sum is the
  ASL height is assumed)_. `angles[]` are radians, `angles[1]` the heading.
- Crew seated in a vehicle may have no `position[]` at all (`faction_blufor`: a UAV's AI);
  `class CrewLinks` links unit `item0` to vehicle `item1`, with the seat in `CustomData`
  (`role`, `turretPath[]`). `a3-mission` puts such a unit at its vehicle's position; seating is
  not modelled.
- Unit settings live in `class Attributes` (`name`, `init`, `isPlayer`, `isPlayable`, `skill`,
  `rank`, `presence`, `class Inventory`); a `Logic` keeps `name`/`init` on the item. Bit 2 of
  `flags` marks the group leader _(assumed: leaders carry 2, 6 or 7, the others 4, 5 or 0)_.
- `presenceCondition="false"` (switched-off modules) means the entity is not created; other
  conditions are evaluated by the engine at start _(not yet in `a3-mission`)_.
- `class CustomAttributes { class AttributeK { property; expression; class Value { class data {
  class type { type[]={"BOOL"|"SCALAR"|"STRING"|"ARRAY"}; }; value=...; }; }; }; }`: each
  expression runs at start with `_this` the entity and `_value` the typed value (`ARRAY` holds a
  `class value` list of further `class data`).
- A cutscene saved with only `class Intro` (`malden_intro.malden`, `enoch_intro1.enoch`) has no
  `class Mission`; `a3-mission` loads the intro then _(assumed: what the engine plays)_.

## Start-up order

The engine's order (community wiki "Initialisation Order", Arma 3 table; implemented in
`crates/a3-mission/src/run.rs`, `start_mission`):

1. the function library's mission start (`initFunctions.sqf`, `docs/re/functions-init.md`):
   campaign and mission functions, `preInit` functions; the `postInit` sequence
   (`initServer.sqf`, `initPlayerLocal.sqf`, `postInit` functions) is spawned,
2. object init event handlers _(not yet)_,
3. each placed unit's `init` field, **unscheduled** — so `sleep`/`waitUntil` in one is an error
   ("Suspending not allowed in this context"),
4. the 3D editor's entity attribute expressions (`_this`, `_value`),
5. `init.sqs`, then `init.sqf`, **scheduled** (they may `sleep` and `waitUntil`).

`missionName` is the folder name without the world (`boot_m02`) and `worldName` the CfgWorlds
class (`Altis`) while a mission runs. The `missionName` handler (0x8b0c00) copies a global string
(DAT_14225ec48) whose writers were not traced, so its content is _(assumed, from the wiki)_.

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
  intro/outro plays _(assumed: the latter; `a3-mission` parses `class Mission` only, or
  `class Intro` when there is no `class Mission`)_.
- The 3D editor's `flags` bits other than the leader bit, `class Connections` (synchronisation
  and trigger-owner links) and `class Inventory` (loadouts applied at start) are not read yet.
