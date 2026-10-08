# World and object model — the in-memory side

The original engine's Object/Entity model: the class tree, the identifiers every object carries,
how locality is stored and changed, how objects are created from config, and how the World
stores and simulates them. The wire side (how a Network object ID is encoded, which message
changes ownership) is in `net-object-model.md`. The two docs must stay consistent.

Arma 3 2.22.0.154103, `arma3_x64.exe`. Addresses are VAs (RVA + 0x140000000). Field offsets are
byte offsets into the object. Functions and globals named here were renamed or labelled in the
shared Ghidra project (`a3`), so `a3re.py decompile <name>` works.

Confidence tags: **high** = read directly from code, **medium** = inferred from code shape and
naming, **low** = hypothesis to check.

## Globals

| Label | VA | What it is | Confidence |
|---|---|---|---|
| `GWorld` | `0x14220dc60` | the `World` instance (owns object lists, `SimulateAllVehicles`, creation) | high |
| `GLandscape` | `0x142237f20` | the `Landscape` (terrain heights, object grid, static object lookup) | high |
| `GNetworkManager` | `0x1421ccaa0` | the `NetworkManager`; `+0x50` = `NetworkServer*` (null on clients), `+0x58` = `NetworkClient*` | high |

`GWorld+0x1be8` (int) switches the network paths off. When it is non-zero, creation skips the
NetworkManager, `netId` formats `0:<ObjectId>`/`1:<ObjectId>`, and `objectFromNetId` parses the
string itself. When it is zero (a network session), everything goes through the
NetworkManager. **Medium**: this is probably "no network session" (the editor, or single player
without a session). What sets the flag has not been traced.

## Class tree

The full tree with descendant counts is in `rtti-classes.md`. The parts that matter for the
World model:

```
NetworkObject                 replicated base: virtual Get/SetNetworkId, IsLocal/SetLocal, update-error metrics
└── Object                    has a shape and a position; also the class of plain static map objects
    └── ObjectTyped           Object with an EntityType (config class)
        └── Entity            simulated: Simulate(dt), visual-state history, NetworkId storage, local flag
            ├── Shot, Smoke, Explosion, Mark, Detector (triggers), CParticleSource, ...
            └── EntityAI      targets, damage, inventory; fires the "Local" event on locality change
                ├── Building, Thing (ThingEPE), FlagCarrier, ...
                └── EntityAIFull
                    ├── Person → Man → Soldier, Animal, InvisibleVehicle (logic, HC, curator)
                    └── Transport  crew positions
                        ├── TankOrCar → Car (CarEPE), Motorcycle, Tank (TankEPE), Ship (ShipEPE)
                        ├── PlaneOrHeli → Airplane (AirplaneEPE), Helicopter (HelicopterEPE, HelicopterRTD)
                        └── Parachute, Paraglide, Door
AI containers (AIGroup, AISubgroup, AIUnit/AIAgent, AICenter), Turret, Weather, ... also derive from NetworkObject.
```

Every replicated thing, groups included, derives from `NetworkObject`. **High** (RTTI).

### Network virtuals (shared vtable prefix of every NetworkObject)

| Slot (offset) | Meaning | `Object` (static) | `Entity` and below | Confidence |
|---|---|---|---|---|
| 5 (`+0x28`) | `SetNetworkId(NetworkId)` | error "Cannot set network id" | store at `+0x498` | high |
| 6 (`+0x30`) | `GetNetworkId()` | `{1, ObjectId}`, only for primary (map) objects; otherwise logs "Type is not primary for object %s" | primary: as `Object`; else `+0x498` | high |
| 10 (`+0x50`) | `IsLocal()` | always `true` | bit 4 of byte `+0x452` | high |
| 11 (`+0x58`) | `SetLocal(bool)` | error "Object is always local" | sets bit 4 of `+0x452`; `EntityAI` also raises object event `0x2f` when the value changes (probably the `Local` event handler, medium) | high |
| 12 (`+0x60`) | remove/destroy | error "Cannot remove primary object" | Entity/Transport-specific | medium |
| 17, 18 (`+0x88`, `+0x90`) | network update error metric, current and initial, per update class | compares damage and state flags (each mismatch adds 10000) | class-specific | medium |

### Fields

| Offset | Class | Field | Confidence |
|---|---|---|---|
| `+0xa4` | Object | **VisitorId**: the object's id in the WRP object block (`ObjectInstance::id` in `a3-wrp`). `getObjectID` prints it; `nearestObject [pos, id]` searches for it | high |
| `+0xa8` | Object | **ObjectId**: the engine's soft-link id. Primary objects report it as their NetworkId `id` (with creator 1). Outside a network session `netId` prints it. For landscape objects it is probably the packed cell id (bit 31 set, cell z in bits 21-30, cell x in bits 11-20, index in bits 0-10; see `wrp.md` "Static entities") | high (role), medium (encoding) |
| `+0xd0` | Object | current (future) `ObjectVisualState*` | high |
| `+0x159` | Object | object kind byte. `1` = primary (map object loaded from the WRP); `2` and `0x20` are also accepted as "primary" by `Object::GetNetworkId` | medium |
| `+0x168` | Object | flag byte: bit 3 = static, printed as `1:` instead of `0:` by `netId` outside a session (medium); bits 2 and 7 enter the update error metric | medium |
| `+0x170` | ObjectTyped | `EntityType*` (the vehicle type created from the config class) | high |
| `+0x188`, `+0x190` | Entity | history of `ObjectVisualState*` and its count. Each state stores the dt it covered (`+0x50`). The renderer interpolates between them; this is visual smoothing, not network | medium |
| `+0x1bc` | Entity | simulation step (precision) in seconds. The `Entity` constructor (`0x140e6efa0`) sets 1/15 s. `SetSimulationPrecision` (`0x140e94b20`; `0x140e94b30` also sets `+0x1c0`) changes it at run time, called from the `Simulate` of `Man`, `Transport`, `Airplane` and others. Ammo types read `simulationStep` from CfgAmmo (`0x141105920`) | high |
| `+0x1c4` | Entity | accumulated time not yet simulated | high |
| `+0x1cd` | Entity | flags: `0x20` = skip simulation (simulation disabled), `0x04` = simulate now regardless of step | medium |
| `+0x452` | Entity | flags: bit 4 = **local**, bit 3 = "deleted/finished" (set after the destroy path), bits 0-2 and 6 are vehicle state used by `Car::Simulate` | high (bit 4), medium (others) |
| `+0x498` | Entity | **NetworkId** `{i32 creator, i32 id}` | high |

## Identifiers

There are three different ids. Keep them apart.

1. **NetworkId** `{creator, id}` (two `i32`s). It is what `netId` prints as `"creator:id"`, what
   `objectFromNetId` parses, and what every game message carries (wire encoding in
   `net-object-model.md`). **High.**
   - `creator == 0`: no object (null).
   - `creator == 1`: a **static map object**. `id` is its ObjectId (`+0xa8`). It resolves through
     `GLandscape` (`NetworkManager_ObjectFromNetworkId` `0x140bc1ee0`), so map objects need no
     create message. Every machine already has them from the WRP.
   - `creator >= 2`: a dynamic object. `creator` is the player id (dpnid) of the machine that
     created it, and `id` is that machine's serial. Lookup is a hash table on the NetworkClient
     (`+0x610` buckets, `+0x5fc` size, 0x18-byte entries, hash
     `(id + creator * 0x100000) % size`).
2. **ObjectId** (`+0xa8`): the engine's internal soft-link id (`SoftLinkIdTraits<ObjectId>` in
   `NetworkObject`'s bases). It lets references to streamed-out landscape objects survive. It is
   the NetworkId `id` only for map objects.
3. **VisitorId** (`+0xa4`): the WRP object record id (named after the Visitor terrain tool). It is
   only meaningful for map objects. Scripts see it through `getObjectID` and `nearestObject`.

### Allocation — `NetworkClient_RegisterObject` `0x140c2b890` — high

On the machine that creates an object:

```
creator = client+0x9b4            // this machine's player id (also clientOwner)
id      = client+0x18++           // per-machine serial, post-increment
obj.SetNetworkId({creator, id}); obj.SetLocal(true)
create a NetworkObjectInfo with six per-update-class slots
```

The initial value of `client+0x18` has not been traced (**low**: probably 0 or 1).

### Player ids — high

- The server process has its own client with player id **2**. `NetworkServer` initialises its
  "next local id" (`server+0xfb4`) to 2 (`0x140c81180`), and the local client takes it
  (`0x140bbb210`: `client+0x9b4 = server+0xfb4++`). Objects created by the server therefore have
  `creator == 2`.
- Remote clients get the id from the connect RESULT, a large number derived from the client's
  clock (`net-handshake.md`).
- `clientOwner` = `client+0x9b4`. `owner obj` works on the server only (it reads the object's
  `NetworkObjectInfo+0x18` through `NetworkManager_GetOwner` `0x140bc2050`) and returns 0 on
  clients. `+0x1c` of the same info is a second owner-like field; slot 122 reads it. Its meaning
  is unknown.

## Locality

- **Storage:** one bit per Entity (`+0x452` bit 4). Static `Object`s are always local, and their
  `SetLocal` is an error. **High.**
- **Initial owner:** the machine that creates the object. `RegisterObject` sets local = true. The
  copies that other machines create from the create message stay remote. **High** for the
  creator side; the receiver side is inferred (medium).
- **Change:** `setOwner` (non-AI objects and agents) and `setGroupOwner` (groups, with their AI
  units) are server-only commands. The server sends the change and the new owner sets local; the
  old owner clears it. Message ids and checks are in `net-object-model.md` (355 = owner change).
  `EntityAI::SetLocal` raises the object event `0x2f` when the bit flips. **Medium**: the
  server-side path from `setOwner` (`0x14053dc20`) to the messages has not been traced end to end.
- **Groups:** `local group` and `groupOwner` follow the same model. AI units are local where
  their group is local. **Medium**.

### What runs where

`Entity_Simulate` (vtable slot 374, `+0xbb0`) runs on **every machine for every entity**. Inside,
each class tests `IsLocal()` before the authoritative parts. In `Car::Simulate` (`0x140d6c2b0`),
physics forces, damage from impacts and contact effects run only when local (`param_3 < 2 &&
IsLocal()`). Remote copies keep animating and moving from the state carried by update messages.
**High** for the guarded structure; **medium** for the details of remote motion. The receive
path is `NetworkClient::UpdateObject`. Extrapolating remote motion from the received velocity
(dead reckoning) is the expected design but is **not yet confirmed**. Follow-up: trace
`NetworkClient::UpdateObject` and the remote branch of `Transport::Simulate`.

The owner sends updates. The error metric per update class (vtable slots 17/18; for example,
static objects add 10000 for a changed destroyed/hidden flag and `10 * |Δdamage|`) decides when
and how often each object is sent to each player (`NetworkObjectInfo` per player).

## Creation from config

### `simulation` → C++ class — `VehicleTypeBank_NewType_FromSimulation` `0x140ffc1c0`

Reading a `CfgVehicles`/`CfgAmmo`/`CfgNonAIVehicles` class reads its `simulation` string and
constructs the matching **type** object (`*Type`, an `EntityType` subclass). The type then
creates instances of its entity class. Unknown values log
`"Unrecognized CfgVehicles simulation %s in %s"`. Pairs read from the factory (**high**, each one
checked against the constructor's vftable):

| `simulation` | Type class | Entity class |
|---|---|---|
| `soldier` | `ManType` | `Man`/`Soldier` |
| `car` | `CarType` | `Car` |
| `carx` | `CarEPEType` | `CarEPE` |
| `motorcycle` | `MotorcycleType` | `Motorcycle` |
| `tank` | `TankType` | `Tank` |
| `ship` | `ShipType` | `Ship` |
| `hovercraftx` | `HovercraftEPEType` | `HovercraftEPE` |
| `submarinex` | `SubmarineEPEType` | `SubmarineEPE` |
| `helicopter` | `HelicopterType` | `Helicopter` |
| `helicopterx` | `HelicopterEPEType` | `HelicopterEPE` |
| `helicopterrtd` | `HelicopterRTDType` | `HelicopterRTD` |
| `airplane` | `AirplaneType` | `Airplane` |
| `airplanex` | `AirplaneEPEType` | `AirplaneEPE` |
| `parachute` | `ParachuteType` | `Parachute` |
| `paraglide` | `ParaglideType` | `Paraglide` |
| `uavpilot` | `UavPilotType` | `UavPilot` |
| `thing` | `ThingType` | `Thing` |
| `thingx` | `ThingEPEType` | `ThingEPE` |
| `fire` | `BuildingType` | `Building` (Fireplace) |
| `fountain` | `FountainType` | `Fountain` |
| `flagcarrier` | `FlagCarrierType` | `FlagCarrier` |
| `airport` | `AirportObjectType` | `AirportObject` |
| `curator` | `CuratorCommanderType` | `CuratorCommander` |
| `headlessclient` | `HeadlessClientType` | `HeadlessClientLogic` |
| `invisible` | `InvisibleVehicleType` | `InvisibleVehicle` |
| `lasertarget`, `nvmarker`, `suppresstarget`, `artillerymarker` | `LaserTargetType`, `NVMarkerTargetType`, `SuppressTargetType`, `ArtilleryMarkerTargetType` | targets |
| `seagull` | `SeaGullType` | `SeaGull` |
| `breakablehousepart`, `breakablehouseanimatedpart`, `thingeffect` | `ThingType` | `Thing` |

Not yet paired (they appear in the factory's string table, but their compares are inlined in a
way the extraction script missed): `tankx`, `shipx`, `house`, `church`, `housesimulated`,
`breakablehouseanimated`, `animal`, `zsu`, `soldierold`, `entityaiplain`, the
`CfgNonAIVehicles` set (`proxy*`, `streetlamp`, `windsock`, `detector`, `detectorflag`, `mark`,
`objview`, `soundonvehicle`, `thunderbolt`, `editcursor`, `ropesegment`, `smokesource`,
`explosion`, `crateronvehicle`, `dynamicsound`, `lightpoint`, ...), and `CfgAmmo` (`shotMissile`,
`shotRocket`, ...). By name, `tankx` → `TankEPE`, `shipx` → `ShipEPE` and `house` → `Building`
(**medium**). Follow-up: finish the table and cross-check it against every `simulation` value in
the merged game config.

Values in the shipped config (2.22 base game and DLC, merged):

- **CfgVehicles:** `soldier` (979 classes), `house` (4897), `thingx` (999), `carx` (359),
  `invisible` (291, logic), `tankx` (194), `helicopterrtd` (133), `airplanex` (82), `thing`,
  `shipx`, `flagcarrier`, `church`, `thingeffect`, `animal`, `motorcycle`, `car`, `parachute`,
  `fire`, `uavpilot`, `fountain`, `lasertarget`, `nvmarker`, `vasi`, `submarinex`, `tank`,
  `ship`, `helicopter`, `artillerymarker`, `airport`, `paraglide`, `curator`, `headlessclient`,
  `rope`, `suppresstarget`, `windanomaly`. There is no `helicopterx`, `airplane` or
  `hovercraftx` class in the base game.
- **CfgAmmo:** `shotbullet`, `shotmissile`, `shotshell`, `shotmine`, `shotilluminating`,
  `shotsmokex`, `shotsubmunitions`, `shotdeploy`, `shotrocket`, `shotcm`,
  `shotdirectionalbomb`, `shotgrenade`, `shotnvgmarker`, `shotboundingmine`, `shotspread`,
  `shotsmoke`, `shotlaser`, `shottimebomb`, `laserdesignate`.
- **CfgNonAIVehicles:** `detector`, `camera`, `camconstruct`, `camcurator`, `editcursor`,
  `objview`, `seagull`, `streetlamp`, `windsock`, `ropesegment`, `road`, `flag`, `proxy*`,
  `alwayshide`, `alwaysshow`, `magazine`, `maverickweapon`, `pylonpod`, `randomshape`, `temp`.

`a3-world`'s `TypeBank` builds a type for each of the 8320 creatable classes. 23 public
CfgVehicles classes have no `simulation` (sound sources, `placed_*_IR_grenade`).

### `createVehicle` — `World_CreateVehicleImpl` `0x1404858c0` — high

1. Create the entity from the type name (`0x141153ce0`). Abstract types (`scope = private`)
   fail.
2. Clamp the position to ±50 km. Snap to the surface or to a free spot when asked. Optionally
   randomise the heading.
3. Refuse types "with brain" (`EntityAI` whose type has AI) from `createVehicle`:
   `"Vehicles with brain cannot be created using 'createVehicle'!"`. Units come from
   `createUnit`.
4. Insert into a World list (next section) and, unless local-only (`createVehicleLocal`) or
   outside a network session, register with the NetworkManager (vtable `+0x3e8` →
   `NetworkClient_CreateVehicle` `0x140c2cb30`). That allocates the NetworkId (above) and sends
   the create message with the World list kind (0 = vehicles, 1 = slow/non-AI, 6 = see below).
5. Mission event `EntityCreated` (`0x141154d90`, with list tag `'s'`/`'a'`/`'p'`).
6. Types with an inventory get a `GroundWeaponHolder` created next to them.

## World containers — `GWorld` offsets

| Offset | Content | Filled by | Read by | Confidence |
|---|---|---|---|---|
| `+0x1d10` | **vehicles**: an entity list made of 4 sub-lists (`+0x1d18`, `+0x1de0`, `+0x1ea8`, `+0x1f70`, each 0xc8 bytes) | `World_AddVehicle` `0x1411335a0` (tag `'s'`) | `vehicles`, `agents`, `allDead` | high (role), low (why 4 sub-lists; possibly dynamic-simulation state) |
| `+0x1c48` | **fast vehicles** (projectiles, shots) | `World_AddFastVehicle` `0x1411305a0` (tag `'p'`); also feeds `+0x1978` and `+0x1970` (sound/ballistics) | `SimulateAllVehicles` runs these as parallel jobs (`EntityMTSimJob`, profile scope `"mtVehS"`) | high |
| `+0x2038` | **slow / non-AI** entities | creation path for non-`EntityAI` (tag `'a'`) | fixed-step loop | medium |
| `+0x2360` | second 5-part list (`+0x2368` … `+0x2688`). Entities whose step is above 10 s go to `+0x2688`. Likely the **out-of-simulation / dead** set | `0x1401e9f20` (create kind 6) | `allDead` | medium |
| `+0x2760`/`+0x2768` | array + count of **agents** | | `agents` | medium |
| `+0x1838` | per-type registry fed by `AddVehicle` for some types | | | low |

Static map objects are **not** in these lists. They live in `GLandscape`'s object grid (cells of
`landscape+0x738` m; `+0x73c` = 1/cell size; `+0x28` = grid mask). `nearestObject [pos, id]`
(`0x14106c1e0`) searches the cell under `pos` for the VisitorId, then rings outward, warning past
10 rings: `"Performance warning: Very large search for %d"`.

## Simulation order — `World_SimulateAllVehicles` `0x141174330`

Per frame, **medium** for ordering details:

1. Update the dynamic-simulation grid (`EntityDynamicSimulation::UpdateGridJob`).
2. Housekeeping over the vehicle sub-lists and fast vehicles.
3. **Fast vehicles** (`+0x1c48`): one `EntityMTSimJob` per entity, run in parallel. Each job
   calls the entity's step at its own precision until the frame's time is used up.
4. **Catch-up loop for long frames**: while more than 0.025 s of the frame remains, each
   iteration consumes 0.025 s of frame time (`fVar23 -= 0.025`) but calls
   `Entity_SimulateFixedStep(entity, 0.05, k)` for the entities of the four vehicle sub-lists
   and the slow-list sub-lists, where `k` is the sub-list index (0-3). `0x140e96500` uses `k`
   to decide whether the entity takes part in this iteration, so the sub-lists are probably
   interleaved: each entity gets 0.05 s every other iteration (**medium**; the gating has not
   been traced). The remaining frame time (≤ 0.025 s) then goes through a separate per-frame
   path (`0x14117a5a0` with callbacks `0x140e95fb0`/`0x140e96050`/`0x140e95eb0`, chosen by a
   global mode `DAT_1420c0c2c % 3`).

   `Entity_SimulateFixedStep` (`0x140e95dc0`) adds dt to the accumulator (`+0x1c4`). When the
   accumulator reaches the entity's step (`+0x1bc`), or the force flag is set, it pushes a
   visual state and calls `Entity_Simulate` (slot 374) with exactly one step (or the whole
   accumulator if the step is 0), then subtracts that from the accumulator. Entities flagged
   "no simulation" (`+0x1cd & 0x20`) are skipped. `a3-world` keeps the accumulator rule but cuts
   frames into time-conserving 0.025 s sub-steps (issue #117).
5. Then: attached positions (`World::UpdateAttachedPositions`), AI (`World::PerformAI`),
   cloudlets and sound (`World::SimulateCloudletsAndSound`). These are separate `World` methods
   named by lambda RTTI; their exact order inside `World::Simulate` has not been traced.

Each entity therefore simulates at its own rate (`simulation precision`), decoupled from the
frame rate, while scripts and the frame loop stay frame-coupled (ADR 0002).

## Open questions (follow-up RE)

- The remote branch of `Simulate` and `NetworkClient::UpdateObject`: interpolation versus
  extrapolation, and the snap threshold.
- The meaning of the 4/5-way sub-lists and of `GWorld+0x1be8`.
- The rest of the `simulation` table; the `CfgNonAIVehicles` and `CfgAmmo` branches.
- The initial value of the NetworkId serial. Whether ids are reused after delete (probably not:
  the serial only grows).
- The exact ObjectId packing for landscape objects, from the Landscape loader.
- The server-side handling of `setOwner`/`setGroupOwner`, and which update classes the six
  `NetworkObjectInfo` slots are.
