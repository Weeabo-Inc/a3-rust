# AI navigation: A*, the oper map, roads and building paths

How Arma 3 2.22 plans a path for an AI unit. Source: `arma3_x64.exe` (RVAs below; the reader is
`docs/re/roads.md`, `docs/re/wrp.md`, `docs/re/sim-man-movement.md`). **Status: the class
structure is verified from RTTI (high); the cell level is medium; the cost numbers are low.**
Follow-ups are listed in §8.

## 1. Shape of the system (high)

The engine has one search engine and two cases that use it:

- `AStar<ASOField, ASOCostFunctions, ASOReadyFunction, ASOInterruptFunction,
  AStarEndDefault<ASOField>, ASOContext, ASOIterator, ASOClosedList, ASOOpenList, ASOParams,
  IAStarOperative, AStarCostFunctionsWrapped<ASOCostFunctions>>` — vtable `0x141cb5530`. This is
  the **path** search: a template A* whose node is a cell of an operational field
  (`ASOField`), with pluggable cost functions (`ASOCostFunctions`), a readiness predicate, an
  interrupt predicate, an end test, an open list, a closed list and an iterator over the
  neighbours. `IAStarOperative` (vtable `0x141cb5490`) is the interface the field implements so
  the search can walk it.
- a second instantiation with `ASOCoverCostFunctions`, `AStarCoverEnd` and `AStarCoverContext`
  (vtable `0x141cb5710`) — the separate **cover** search (`AStarCover`, `AStarCoverContext`),
  the same machinery with different costs and a different end test.
- `AIPathPlanner` (vtable `0x141cb5c88`) drives one search: its virtual
  `AIPathPlanner::ProcessSearching(void)` (there is an `InvokerLambda<…ProcessSearching…>`
  RTTI entry, so the search is invoked through a lambda with an `unsigned int` budget) names
  the search as **incremental**: it advances a bounded number of nodes per call instead of
  running to completion in one frame. This is how many units plan in the same frame without a
  spike.

The path search never leaves the field: `AStarCostFunctionsDef<ASOField>` and the wrapped
`AStarCostFunctionsWrapped<…>` compute a cell's g and the heuristic from the field, and
`GetFieldCost` is the field's cost lookup (`" @GetFieldCost has a broken operField!
[lx,lz]={%d;%d}, [x,z]={%d;%d}"` `0x141cb588e` is the assertion of that function; it prints both
the field-local and the world cell coordinate, so field cells are indexed in a local
`lx`/`lz` space with a world `x`/`z` next to it).

## 2. The oper map and the oper fields (medium)

`OperMap` (vtable `0x141cb53d8`), `OperMapField`, `OperField`, `OperFieldSimple` (vtable
`0x141cb5208`), `OperFieldFull` (`0x141cb5278`) and `OperFieldFullExt` (`0x141cb52f0`) are the
AI's **operational map** — a grid over the terrain that stores, per cell, how good the cell is
for combat movement. The vtable RTTI gives the entry points:

| Where | What |
|---|---|
| `OperMap::CreateFields(int,int,int,int,const EntityAIType*,CombatMode,FlagEnum<OperFieldMask>,FlagEnum<OperMap::CreateFieldsMTFlags>)` | builds the fields of a rectangular region for one AI type and combat mode; the four ints are the region rectangle |
| `OperFieldFull::CreateField(FlagEnum<OperFieldMask>)` | builds one field of the full set, spread over two lambdas (`InvokerLambda<…CreateField…lambda_1/lambda_2>`) |
| `OperFieldFull::RemoveFalseCover(void)` | prunes cover the unit cannot use |
| `OperCache::GetOperField(int,int,FlagEnum<OperFieldMask>,…)` | the cache that hands out a field for a cell |
| `OperField`/`OperFieldSimple`/`OperFieldFullExt` | the cheap and the detailed variants |

`OperFieldMask` selects which layers of the field to build (the flag is a bitset); a dedicated
`costMap diag tool` string ("Reload oper map visible in costMap diag tool.",
`0x141b554a0`, in `FUN_1408ab890`) shows the fields to a developer, so a field is per-cell
displayable like a cost map.

The unit of the field is a **cell**, and the AI's positions in it are **oper positions**: the
engine keeps `lastOperPosCost`, `lastOperPosType`, `lastOperPosClearance`, `lastOperPosX`,
`lastOperPosZ`, `lastOperPosHouse`, `lastOperPosHousePos`, `lastOperPosRoad`,
`lastOperPosRoadIndex` (strings at `0x141cb4918`..`0x141cb49b8`). So an oper position carries

- a cell coordinate `x`/`z` and a **type**,
- the cost the planner gave it and a **clearance** (room to move around it),
- optionally a **house** (index and the position inside it) or a **road** (index and the
  position on it).

The types are the kinds of place the path search can aim at: open ground, road, house, cover,
water edge (`WaterOperfieldRadiusCollision` / `WaterOperfieldRadiusLogical`,
`0x141b32108`/`0x141b32128`, are the radii at which a cell counts as water). A path is a list
of such positions; a path that ends at a house or a ladder is finished by the building's own
paths (§5).

`noPath` (`0x141cb266c`) is what two functions (`FUN_141357130`, `FUN_141317340`) report when
the search fails.

**Not established:** the cell size in metres, the actual cost values, and the ordering of the
type enum. Everything above is from RTTI, vtables and format strings; the numbers did not get
read out of the code.

## 3. Where the field's data comes from (medium)

The field is terrain-derived. What a cell contributes is what the WRP and the world config
already hold — this is why the engine can bake it once per terrain and cache it:

- the **land grid** cell (`docs/re/wrp.md`): its `Geography` flags give water depth, forest,
  road, object counts and a gradient class;
- the **heightmap**: slope across the cell;
- the **surface type** (`CfgSurfaces`, `docs/re/landscape.md`): `isWater`, `rough`,
  `maxSpeedCoef`, `friction`, `grassCover`, `AIAvoidStance`;
- the **roads** (`docs/re/roads.md`): road cells are cheaper for the AI, which is what makes a
  long path follow the road network;
- the **objects** placed in the cell (buildings, walls, trees) and the **roadway** LODs that
  make bridges and building floors walkable (`docs/re/sim-man-movement.md` §5).

A vehicle's path planner uses the same search with vehicle cost functions; `calculatePath`
(`0x141adf8e8`, example `"veh = CalculatePath [\"car\", \"SAFE\", [7000, 5000, 0], [7100,
5100, 0]]"` `0x141adf840`; error `"Error: CalculatePath: wrong combat mode: '%s'"`
`0x141ace7e8`) is the script view of it — and it takes a **combat mode** (`"SAFE"` in the
example), so the combat mode is an input to the costs, exactly as `OperMap::CreateFields`
takes it.

## 4. Roads (medium)

Road-following is not a separate graph search in the engine: the road network is one of the
things the field encodes (a cheap cell), so the same A* prefers roads, and `RoadsLib`'s
`AIpathOffset` (a per-road-type lateral offset) says where on the width the AI walks. The
`RoadsLib` `color[]` entry, described as the "AI cost map colour" in `docs/re/roads.md`, is the
same field drawn to a developer. The `lastOperPosRoad` / `lastOperPosRoadIndex` stat names show
a path position can name the road it is on.

`docs/re/roads.md` "how the engine's own `roadsConnectedTo` treats shapefile roads is not yet
reverse engineered": the same open point applies to the road part of the field.

## 5. Buildings: the Paths LOD and `IPaths` (high for the classes, medium for the use)

A building's indoor navigation is its own thing, not a terrain cell search:

- the model's **Paths LOD** (`LodKind::Paths`, resolution `4e15`, "AI paths through buildings",
  `crates/a3-p3d/src/resolution.rs`) is the floor plan: its points and faces are the walkable
  positions inside the building (and the model's **Roadway LOD**, `3e15`, is the surface the man
  stands on).
- `IPaths` (vtable `0x141be4008`) is the interface a building exposes to the AI: a list of
  positions with, per position, the actions that are possible there. `PathAction`
  (`0x142148870`) is the base class, with `PathActionLadderTop` (`0x141be8f40`) and
  `PathActionLadderBottom` (`0x141be8f78`) as concrete ones: **the actions are how a path
  changes floor** — a ladder bottom is entered from the ground floor and left at the ladder top.
  The vtable pairs are 7 entries apart in `0x141be8f40`/`0x141be8f78`, so top and bottom share
  the same interface shape with different behaviour.
- the engine keeps a terrain-wide index of these paths (a "house path index"), so an outdoor
  path can be aimed at a house's entrance position, and the indoor path takes over from there.

There is a `UserActionBegin` / `UserActionEnd`-like family as well (the engine's own action
names in the `PathAction` hierarchy); only the ladder pair could be read off RTTI here.

## 6. Man movement over the result (high, see `sim-man-movement.md`)

The planner returns positions; the man's own movement is built from the moves graph, and slopes
are limited by `CfgSlopeLimits`: `maxRun 0.6` / `minRun −0.8`, `maxSprint 0.3` /
`minSprint −0.5`. A planned segment steeper than `maxRun` cannot be run; the engine falls back
to walking (and refuses movement above the walk limit). Ground under a path position is
whatever the world surface query returns, terrain or a Roadway LOD face — that is how a path
over a bridge or a floor works.

## 7. What this means for our implementation

- Search: **grid A\*** (not a triangle navmesh), with per-cell cost and an octile/euclidean
  heuristic, one search object with reusable scratch per unit (the engine's incremental
  `ProcessSearching` is the same idea: bounded work per frame over a persisting search state).
- Cost grid: land cell sized, from land grid geography + heightmap slope + `CfgSurfaces` +
  roads. Roads cheap. Water and steep cells impassable for men.
- Obstacles: the op field is baked from the terrain's objects; dynamic obstacles (vehicles,
  units) are not part of it and are handled by the movement/steering layer. Our grid bakes
  geography and can be patched from the physics colliders where they are loaded.
- Buildings: a separate small triangle graph from the Paths/Roadway LOD, entered from a door
  position, not a terrain cell search.
- Smoothing: the engine's paths are positions, not cells, so something pulls the cell path
  straight; our implementation does a visibility-based string pull.

## 8. Open points

- The oper field's cell size, its cost values and the `OperFieldType` enum order.
- How `AIPathPlanner` schedules the incremental search (budget per frame, priority between
  units) and how a search is interrupted and resumed.
- How `IPaths` positions and `PathAction`s are stored in the Paths LOD (its points, faces and
  named selections) — reading that LOD would let us build a real house path mesh.
- What `Clearance` and the cover field mean numerically for movement cost.
- How the engine's `roadsConnectedTo` works on shapefile roads (`docs/re/roads.md`).
