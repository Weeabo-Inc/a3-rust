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
| `position[]={x, y, z}` | east, height, north | world space as it is: X east, Y up, Z north (ADR 0003) _(verified: `boot_m02` `{6687.77, 48.0, 15982.78}`)_. The 2D editor's `y` is **not** the entity's place: the engine stands every entity of this format on the ground at its `x`/`z` (see "Placement" below); `{x, y}` (two elements, east and north) has no height at all |
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
- `position[]` is `{east, height, north}` and the height is the entity's **own place**; the item's
  `atlOffset` records how far the entity stands above the terrain there and is **not** added when
  the mission loads _(verified against the Oracle: a `Key_F` 130 m above the terrain sits at its
  `position[]`; see "Where a placed entity ends up")_. `angles[]` are radians, `angles[1]` the
  heading.
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

## Placement: where a placed entity ends up

_(Verified against the Oracle, `arma3server_x64.exe` 2.22.0.154103: copies of shipped Missions
and synthetic ones started with `-autoInit` and read back from `initServer.sqf` with
`getPosASL`, `getPosATL` and `getTerrainHeightASL`.)_ The two editors store different things in
the same key, and the engine uses them differently:

| SQM | stored `position[]` | what the engine does |
|---|---|---|
| 2D editor (`version=12`) | `{east, y, north}` | **ignores `y`**: the entity is placed on the ground at its `x`/`z` — and on the sea surface, not the sea floor, where the terrain lies below sea level |
| 3D editor (`version=52..54`) | `{east, y, north}`, the entity's own place, plus `atlOffset`, its height above the terrain there | places the entity at `position[]`; `atlOffset` is **not** added |

The measurements (Stratis; "ground" is the engine's `getTerrainHeightASL`, which never differed
from `a3-wrp`'s `Terrain::surface_height` by more than 0.000 m at any probed spot — our height
sampling is not the problem):

| case | SQM `y` | ground there | engine `getPosASL` |
|---|---:|---:|---:|
| 2D `Land_CanisterFuel_F`, `special="NONE"` | 200 | 60.44 | **60.44** |
| 2D crate, `placement="CAN_COLLIDE"` | 200 | 70.16 | **70.16** |
| 2D crate over deep water | 0 | −26.45 (sea floor) | **0** (sea surface) |
| 2D `B_Soldier_F` | 200 | 71.04 | **71.04** |
| 2D crate, `y` = −100 | −100 | 68.10 | **68.10** |
| 2D helicopter, `special="FLY"` | 200 (and 0) | 71.94 | 48.2209 m above the ground, either way |
| 3D `Key_F`, `atlOffset=130.111` | 241.087 | 110.976 | **241.087** (= `y`) |
| 3D `Land_ClutterCutter_small_F`, `atlOffset=3e-5` | 203.986 | 203.986 | **203.986** |
| 3D crate, `atlOffset=50`, `y` = 60 | 60 | 60.44 | **59.73** (`y`, not `ground+atlOffset` = 110.4) |

Shipped content agrees. In `b_m02_2.stratis` (2D) 126 of its 305 placed entities carry `y=0`; in
the camp at (2977, 1873) the terrain is 171.4 m and every probed unit there stands on the surface
(`getPosASL` 171.401, `getPosATL` ≈ 0.001 m), which is the 97 entities the sweep reports as below
terrain for that Mission. Its boats (`y ≈ 0` over a sea floor at −63 m) are the other half: a
crate placed over deep water in the controlled run sat at y = 0, the sea surface, not on the
floor. Issue #349's 388 "below terrain" entities are this one mistake: a stored height the engine
does not use, read as the entity's place. In `Intro2.Enoch` (3D) every object with an `atlOffset` from −18 to +173 that was
probed sits at its `position[]`; a few objects the editor left stale (a `Land_Smokestack_F` whose
`y` is 29 m above the terrain, `atlOffset=0`; one `CargoPlaftorm` 2 m below it) do not — they are
29 m and 3 m lower than `y`, which is either their settling onto the ground during the (heavy)
mission load or a `ground + atlOffset` placement; the controlled 3D Mission, read 0.2 s after
creation with nothing yet settled, shows `y` alone, so `position[]` is the place.

What `a3-mission` does with that (`crates/a3-mission/src/spawn.rs`):

- a 2D-editor entity or a two-element position sets `Unit::on_surface` and is created at
  `max(surface_height(x, z), 0)` — the ground, or the sea surface over sea, where the whole map
  is at y = 0;
- a 3D-editor entity is created at its `position[]`; `atlOffset` is parsed past, not added;
- `special="FLY"` (the one 2D case whose stored height the engine reads) keeps the stored `y` for
  now: the engine's own lift to 48.2209 m above the ground is issue #126's air work, and the
  author's `y` is closer to the intent than the ground.

Open: the collision world's Roadway surfaces (a bridge deck, a house floor) are the engine's real
"ground" (`CONTEXT.md` §Ground); `spawn.rs` uses the terrain, since a Mission is spawned before
its land cells are streamed. Whether the script paths (`createVehicle`, `createUnit`,
`crates/a3-world/src/script/`) clamp to the sea surface the same way is unmeasured: they still
add the terrain height to the script's own z (`Placement::OnSurface`).

The other candidate causes issue #349 listed are ruled out by the same probes: the engine's
`getPosASL` keeps the file's `x`/`z` exactly, so nothing is swapped on the Y/Z axis (ADR 0003),
and `Terrain::surface_height` matched the engine's `getTerrainHeightASL` to 0.000 m at every spot
probed, so our WRP sampling is not at fault. The entities counted are Mission entities, not the
WRP's static objects, so a building placed by its model pivot is not involved either. How far the
fix moves things: of eight 2D Missions checked with this rule, `boot_m02`, `boot_m03`,
`b_m02_1`, `mp_coop_m04` and `showcase_helicopters` move 0–1 % of their entities (their stored
heights already are the ground), while the two with the sweep's below-terrain counts move most of
them — `c_in1.stratis` 136 of 149, `b_m02_2.stratis` 233 of 305. On the 3D side the entities that
move are those with a negative `atlOffset`, and their count per Mission matches the sweep's:
`exp_m05.tanoa` 12 (sweep 12), `exp_m01.tanoa` 12 (12), `showcase_future.altis` 6 (12),
`orange_hub.altis` 4 (6); the 1–5 m ones add the remainder.

## What the engine refuses to create

_(Verified against the Oracle.)_ Both kinds of unit the sweep reports as unspawned are refused by
the engine itself, so they are abstract or stale Mission content and nothing is missing here:

| class | scenarios | the engine's own behaviour |
|---|---|---|
| `WeaponHolder` | `mp_coop_m04.stratis` (β and Curator) | logs `Cannot create entity with abstract type WeaponHolder (scope = private?)`, the placed variable stays nil, and `createVehicle ["WeaponHolder", …]` adds `Cannot create non-ai vehicle 'WeaponHolder',''` |
| `ModuleHvtObjectiveObjectsManager_F`, `ModuleHvtTwinObjective_F`, `ModuleHvtEndGameTwinObjective_F` | `mp_marksmen_02.altis` (Mark DLC) | the class is nowhere: `isClass (configFile >> "CfgVehicles" >> "ModuleHvtObjectiveObjectsManager_F")` is `false` on the engine, and a scan of all 508 mounted PBOs (1432 addon configs, Mark DLC included) finds the string only in that Mission's own `mission.sqm` |

`CfgVehicles/WeaponHolder` resolves to `scope = 0` (inherited from `Static`); the creatable ground
holders are `GroundWeaponHolder` and `GroundWeaponHolder_Scripted`. See `docs/re/config.md`
§`scope` for the rule. `a3-world`'s `TypeBank` refuses the same two ways
(`Error::AbstractType`, `Error::UnknownType`), so `Spawned::unspawned` names exactly the units the
engine drops, and `Unspawned::engine_skips` marks them so a report can tell stale content from a
class we failed to create.

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
- The two-element `position[]`: the engine places the entity on the ground either way, so "no
  height" and "height 0" are the same thing for the 2D editor.
- `special="FLY"` lifts an aircraft to 48.2209 m above the ground whatever its stored height
  (measured twice); whether that number is a config value (`flyInHeight`?) or the engine's own
  default is not known. `FORM` and `CARGO` need a formation slot and a carrier's cargo index,
  which the World has no model for yet.
- Whether `Intro`/`OutroWin`/`OutroLoose` scenes are loaded at mission start or only when the
  intro/outro plays _(assumed: the latter; `a3-mission` parses `class Mission` only, or
  `class Intro` when there is no `class Mission`)_.
- The 3D editor's `flags` bits other than the leader bit, `class Connections` (synchronisation
  and trigger-owner links) and `class Inventory` (loadouts applied at start) are not read yet.
