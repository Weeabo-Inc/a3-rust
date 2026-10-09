# Arma 3 / Real Virtuality

The vocabulary of the Real Virtuality 4 engine and the Arma 3 game data it runs. Definitions
marked _(uncertain)_ are best current understanding and need confirmation by reverse engineering.

## Packaging and file system

**PBO**:
An archive file holding a set of game files plus a header of string properties (such as `prefix`).
The unit in which all game content ships.
_Avoid_: archive, pak

**EBO**:
An encrypted PBO (`.ebo`) shipped by creator DLC. Its properties (including the Prefix) are plain
text; its entry records and data are encrypted.
_Avoid_: encrypted archive

**Addon**:
A PBO whose content is registered with the engine, normally declaring one or more CfgPatches
classes in its config.
_Avoid_: mod (a Mod is a folder of addons), package

**Mod**:
A folder (such as `@CBA_A3` or the base-game `A3`/`Expansion` dirs) containing an `addons/`
directory of PBOs and their signatures, loaded as a unit.

**Prefix**:
The virtual path at which a PBO's contents are mounted in the VFS (e.g. `a3\weapons_f`), taken
from the PBO header's `prefix` property or, failing that, the PBO file name.
_Avoid_: mount point, root

**VFS**:
The engine's single virtual file tree built by mounting every loaded PBO at its Prefix, with
backslash separators and case-insensitive lookup. Later-loaded content overrides earlier content
at the same path _(uncertain: exact priority rules)_.
_Avoid_: filesystem, file bank (the engine's own name for a mounted PBO is "bank")

**Bisign / Bikey**:
A Bikey is a publisher's public key; a Bisign is the signature of one PBO by that key. Servers use
them to verify that clients run unmodified addons.
_Avoid_: signature file, key file

## Configuration

**Config**:
The engine's global hierarchical tree of classes holding all static game definitions, merged from
every addon's config.
_Avoid_: settings, cfg

**config.cpp**:
The human-readable source form of an addon's config, written in a C-like class syntax and run
through the Preprocessor before parsing.

**config.bin**:
The binary, rapified form of a config. What shipped addons contain.

**Rapify / Derap**:
Rapify converts a text config (config.cpp, description.ext, mission.sqm) to its binary `raP`
form; Derap converts binary back to text.
_Avoid_: compile/decompile, binarize (that is the model pipeline)

**Config class**:
A named node in the Config holding entries (numbers, strings, arrays) and child classes. It may
inherit from a base class, inheriting all entries it does not override.

**External class declaration**:
`class X;` inside a config class: a forward reference to a class that is inherited or defined by
another addon. It defines nothing itself; lookups pass through it.
_Avoid_: forward declaration (alone), stub

**Config patching**:
Merging a later addon's config into the Config: same-named classes merge entry by entry, values
are overridden, `delete X;` removes classes and `x[] += {...}` extends arrays.
_Avoid_: overriding (too narrow), mod merging

**Load order**:
The order in which addon configs are patched into the Config, derived from CfgPatches
`requiredAddons` _(uncertain: exact tie-break rules)_.

**CfgPatches**:
The root config class in which each addon declares itself, its `requiredAddons` (which decides
merge order) and the units/weapons it adds.

**CfgVehicles / CfgWeapons / CfgAmmo / CfgMagazines**:
Root config classes defining every entity type, weapon, projectile and magazine by name.

**Stringtable**:
A `stringtable.xml` (or binarized `stringtable.bin`) file mapping keys, by convention `STR_...`
and compared ignoring case, to localised text in each supported language. A key shows the
selected language, else its `Original` text, else English, else its first entry.

**Localize**:
To look a key up in every loaded Stringtable for the current language, as the `localize` script
command and `$STR_...` config values do; a missing key gives an empty string.
_Avoid_: translate

## Textures and materials

**PAA / PAC**:
The engine's texture formats: DXT-compressed or raw (ARGB4444, ARGB1555, AI88, ARGB8888) mip
chains with a tagged header. PAC is the same format under an older extension.
_Avoid_: DDS

**rvmat**:
A material file in config syntax naming a pixel/vertex shader and the textures for each stage
(normal map, specular map, detail map, etc.).
_Avoid_: shader file

**Texture suffix**:
The naming convention telling the engine how to use a texture: `_co` colour, `_ca` colour with
alpha, `_nohq` normal map, `_smdi` specular/metal/detail, `_as` ambient shadow, `_mc` macro,
`_dt` detail, `_lco` satellite segment, `_mask` surface mask, `_ti_ca` thermal.

**Texture type**:
The engine's classification of a texture (diffuse, linear diffuse, detail, normal map, macro,
ambient shadow, specular, mask, thermal, ...), derived from its Texture suffix when the texture is
converted and cached in texHeaders.bin.

**TAGG**:
A tagged chunk in a PAA header (average colour, max colour, alpha flags, channel swizzle, mipmap
offsets).
_Avoid_: tag (alone), chunk

**Swizzle**:
The channel rearrangement recorded in a PAA (e.g. a `_nohq` normal map stores X in alpha); shaders
read the swizzled layout.

**texHeaders.bin**:
The per-PBO cache of every texture's header (format, size, mipmap offsets, average colour,
Texture type), so the engine can plan texture loading without opening each PAA.

**Procedural texture**:
A texture the engine generates from a string such as `#(argb,8,8,3)color(1,0,0,1)`, usable
anywhere a texture path is; generators include `color`, `fresnel`, `fresnelGlass`,
`perlinNoise`, `irradiance`.
_Avoid_: generated texture, inline texture

## Models and animation

**P3D**:
A 3D model file holding a set of LODs. Comes in two encodings: MLOD and ODOL.
_Avoid_: mesh file

**MLOD**:
The editable P3D encoding produced by Object Builder; geometry, selections and properties stored
in a straightforward tagged form.

**ODOL**:
The binarised, engine-optimised P3D encoding that ships in game PBOs.
_Avoid_: binarized model, compiled model

**LOD**:
One complete representation of a model inside a P3D, identified by its resolution value. Either a
Resolution LOD or a special-purpose LOD.
_Avoid_: level (alone), mesh

**Resolution LOD**:
A visual LOD, chosen by distance and detail settings; lower resolution values are more detailed.

**Special LOD**:
A non-visual LOD with a fixed reserved resolution value: Geometry (collision and mass), Fire
Geometry (bullet hits), View Geometry (AI and player line of sight), Memory (named points for
attachments, axes, muzzle positions), Roadway (walkable surfaces), Shadow Volume, and others.

**Named selection**:
A named set of vertices/faces in a LOD, used to animate, hide or retexture parts of a model and to
map hit points.
_Avoid_: group, vertex group

**Section**:
A run of consecutive faces in a LOD that share one texture, material and face flags; the unit
the renderer draws with one material.
_Avoid_: submesh, primitive, batch

**Proxy**:
A placeholder in a LOD that references another P3D, placed at a position and orientation;
used for crew seats, weapons on a vehicle, and attached decorations.

**Skeleton**:
The ordered list of bones (with parents) that a model's animations act on, defined in
CfgSkeletons.

**RTM**:
An animation file holding per-frame bone transforms for a Skeleton, used mainly for character
moves.

**Move**:
A named animation state a Man can be in — a `CfgMovesBasic` / `CfgMovesMaleSdr` `States` class:
an RTM with its phase rate and speed entries, plus the graph edges to the moves it can flow
into.
_Avoid_: animation (alone), anim state (SQF's `animationState` returns the current move's name)

**Move state machine**:
The per-Man state machine that plays one move at a time, blends into the next along the move
graph, and serves the queue `playMove` and friends build.
_Avoid_: anim system, animation controller (that drives Model animations)

**Animation phase**:
A position in an RTM's cycle, from 0 (start) to 1 (end). Keyframes and keystones sit at phases.
_Avoid_: time, frame (a keyframe is the pose stored at one phase)

**Animation keystone**:
A named marker at a phase of an RTM, such as `StepSound` (a footstep), that the engine fires
while the animation plays.
_Avoid_: event (alone), animation event

**Moves type**:
A CfgMoves class (`CfgMovesMaleSdr`, named by a unit's CfgVehicles `moves`): every Move a kind of
character can play, their Move graph and their Action maps.
_Avoid_: animation set, moveset

**Move**:
One class of a Moves type's `States`: an RTM with its playback parameters (speed, looping,
interpolation speed, minimum play time). What `playMove`, `switchMove` and `animationState`
name. A Man is always in exactly one Move, possibly blending out of the previous one.
_Avoid_: state (alone), animation (alone), anim

**Move graph**:
The directed graph over a Moves type's Moves whose edges (`connectTo`, `interpolateTo`, ...) say
which Move may follow which, at what cost. A **connect** edge waits for the Move to end; an
**interpolate** edge blends into the next Move at once.
_Avoid_: animation graph, state machine

**Move path**:
The Moves the engine plays to get from the current Move to a requested one, found by a cost
search over the Move graph.

**Action map**:
A class of a Moves type's `Actions`, named by each Move: it maps actions (`WalkF`, `Down`,
`Stop`, `ReloadMagazine`, ...) to the Move or gesture to play from that Move, and gives the
stance and turn speed. `playAction` goes through it.
_Avoid_: actions class, action set

**Model animation**:
A config-defined transform (rotation, translation, hide) of a named selection, driven by an
Animation source such as a door state, wheel rotation or gun elevation.

**Animation source**:
A named scalar (`door_lf`, `wheel`, `reload`, `damper`, ...) whose value drives every Model
animation that names it; the engine maps the value to an angle, offset or hide state.
_Avoid_: controller, input

**Pose**:
The model-space transform of every Skeleton bone at one moment, from Model animations and/or
an RTM; skinning applies it to the vertices bound to each bone.

**Skinning**:
Posing a model's geometry through its Skeleton: each vertex is bound to up to four bones with
weights, and follows those bones' Pose matrices.

**Bone palette**:
The matrices one skinned draw samples, one per Skeleton bone in order, plus a trailing identity
slot for vertices with no influences; a hidden bone's matrix is zero, collapsing its vertices.
_Avoid_: bone buffer, skin matrices

## Terrain

**WRP**:
The binarised terrain file (signature `OPRW`) holding the heightmap, the surface/texture grid,
and every placed map object.
_Avoid_: map file

**Landscape**:
The engine object for the loaded terrain: heightmap, surface layers, and placed static objects.
_Avoid_: map, island (except in config names such as CfgWorlds)

**Terrain grid**:
The regular grid of height samples covering the Landscape; its spacing is the terrain cell size.

**Terrain cell**:
One square of the Terrain grid; the unit of collision, surface type lookup and LOD streaming
_(uncertain)_.

**Land grid**:
The coarser square grid of land cells (30 m on Altis) over the Landscape that holds per-cell
geography flags, the surface material index and the placed objects, grouped by cell. Distinct
from the Terrain grid of height samples, which is finer (7.5 m on Altis).
_Avoid_: layer grid, texture grid

**Ground**:
The surface a Man stands on at a point: the terrain, or an object's Roadway LOD face within step
range, so he walks on bridges, ramps and house floors as readily as on land. His feet height is
the ground's height there.
_Avoid_: floor (that is a building's ground), terrain (one kind of ground)

**Map object**:
A symbol of the 2D map stored in the WRP (tree, house, fence, power line, forest cell...),
pointing at the placed Object it stands for.
_Avoid_: map marker (markers are mission-placed)

**Road net**:
The WRP's list of road parts per land cell, each with its connection ends; in Arma 3 it holds
only bridges and invisible runway roadways, while ordinary roads come from the terrain's roads
shapefile.

**Layer material**:
The rvmat (`p_XXX-YYY_*.rvmat`) of one satellite tile: its satellite and mask textures, up to
five surface layers (detail colour and normal maps) and the UV transforms that place them.
Each land cell names one through the WRP material index.
_Avoid_: tile shader, terrain texture

**Surface type**:
A `CfgSurfaces` class selected by a layer's detail texture file name; it sets friction, sounds,
dust and, through its surface character, the clutter grown on the ground.
_Avoid_: ground type, terrain type

**Road shapefile**:
A terrain's `roads.shp` polylines with a `roads.dbf` row per road whose `ID` selects a road
type in `RoadsLib.cfg` (width, textures, map symbol). The source of every ordinary road.
_Avoid_: road net (the WRP's bridge list)

**Satellite map layers**:
The large `_sat` colour image and the `_mask` layer images that blend per-surface textures
(each described by an rvmat) across the Landscape.

**Clutter**:
Small decorative objects (grass, stones) scattered automatically near the camera according to
the surface type.

## Audio

**WSS**:
The engine's legacy sound format: raw or delta-compressed PCM with a short header.

**OGG**:
Ogg Vorbis audio, the main sound format in Arma 3.

**Sound shader**:
A `CfgSoundShaders` class: the samples a sound is chosen from (each with a probability), its
volume and frequency expressions, and its audible range.
_Avoid_: sound definition (that is an old-style `sound[]` entry)

**Sound set**:
A `CfgSoundSets` class: the sound shaders played together, with the set-level volume, curve,
randomisation and 3D parameters. What `playSound` and `say3D` name, and what a CfgEnvSounds
class selects with `soundSetEnvironment`.
_Avoid_: sound group

**Sound curve**:
A sound's gain as a function of a normalised distance (0..1 of the range it is scaled by),
named in `CfgSoundCurves` or written inline as points.
_Avoid_: falloff curve, attenuation table

**Distance filter**:
A `CfgDistanceFilters` class: the low-pass a spatial sound passes through as its distance from
the listener grows.

**Sound 3D processor**:
A `CfgSound3DProcessors` class: whether a source is heard as a ring of channels around itself
(emitter) or folded into a point that pans (panner).

**Simple expression**:
The small float expression language of sound controllers, evaluated per frame against named
variables such as `forest`, `windy` or `distance`; an expression naming anything else fails to
compile.
_Avoid_: sound controller script

## Scripting

**Preprocessor**:
The C-like macro stage (`#define`, `#include`, `#ifdef`, `__EVAL`, `__EXEC`) applied to
config.cpp, description.ext and SQF files before parsing.

**SQF**:
The engine's main scripting language: expressions built from nullary, unary and binary script
commands operating on dynamically typed values.

**SQS**:
The older, line-based scripting language from Operation Flashpoint, still supported.

**FSM**:
A finite-state-machine script (`.fsm`) whose states and conditions contain SQF; used for AI
behaviour and mission logic.

**Script command**:
A named engine primitive callable from SQF, in one of three forms: nullary, unary, or binary.
_Avoid_: function (reserved for SQF functions defined in CfgFunctions)

**Namespace**:
A container for SQF global variables (missionNamespace, uiNamespace, profileNamespace, and
others). Each variable lives in exactly one namespace.

**Scheduled environment**:
Where SQF started with `spawn`/`execVM` runs: the engine time-slices it and it may suspend with
`sleep`/`waitUntil`.

**Unscheduled environment**:
Where SQF started by `call` from engine callbacks, event handlers and init fields runs: to
completion in one go, without suspension.

**Event handler**:
SQF code that the engine runs when an event happens to an object, display or mission (e.g.
`Killed`, `Fired`, `KeyDown`). Runs unscheduled.

## Missions and session

**Mission**:
A playable scenario: a folder or PBO with `mission.sqm`, `description.ext` and scripts, bound to
one terrain.
_Avoid_: map, level

**SQM**:
The mission file format (`mission.sqm`) in config syntax, often rapified, listing the placed
entities, groups, waypoints, markers and triggers.

**Campaign**:
An ordered set of missions with branching defined in a `description.ext` campaign config.

**Profile**:
The per-player data folder holding settings, keybindings and profileNamespace variables.

## World and simulation

**World**:
The engine object owning the current session state: the Landscape, all Entities, time, weather
and the simulation loop.
_Avoid_: scene, level

**World space**:
The engine's coordinate system: left-handed, X east, Y up, Z north, in metres, positions held
as `f64`. Script positions (`[x, y, z]` in SQF) are east, north, height, so Y and Z swap at the
script boundary. See ADR 0003.
_Avoid_: map coordinates, grid coordinates (those are the 100 m map grid references)

**Entity**:
A dynamic, simulated object in the World (soldier, vehicle, projectile).
_Avoid_: actor, game object

**Man**:
A `Person` Entity (soldier, animal, uavpilot, logic): a Skeleton posed by moves, standing on the
ground the World reports under his feet. His position is his feet.
_Avoid_: character, person (the config class is `Person`; the Entity is a Man)

**Object**:
Any instance placed in the World with a model and position, simulated or static. Every Entity is
an Object. Trees, rocks and walls on the Landscape are Objects but not Entities.

**Static object**:
An Object placed by the WRP rather than created during the session. Every machine loads it
from the terrain, so it is never created or deleted over the network. It can still be an Entity
(a house with doors, a street lamp).
_Avoid_: map object (that is the 2D map symbol), terrain object, primary object

**Object ID**:
The number of a Static object in the WRP, as returned by `getObjectID` and taken by
`nearestObject [position, id]`.
_Avoid_: VisitorId, netId

**Simulation**:
The behaviour class that drives an Entity type each frame, selected by the `simulation` config
entry (e.g. `soldier`, `carx`, `tankx`, `helicopterrtd`, `airplanex`, `shipx`, `house`, `thing`).

**Hit point**:
A named damageable location of an Entity type (`class HitPoints` in `CfgVehicles`): its armour,
radius, `passThrough`, `minimalHit` and the `depends` expression that derives its damage from
other hit points (`HitBody` is the worst of the four torso hit points). Hit point damage is 0..1
and separate from the total damage; hit points are addressed by config name
(`HitHead`) or by model selection (`head`). See the hit-point tables in `docs/re/sim-damage.md`.
_Avoid_: hit zone, damage part, hit location

**Total damage**:
An Object's damage value, 0 (intact) to 1 (destroyed): `damage`, `setDamage` and `Killed` speak
of it, and it is not the sum of the hit points — a hit adds to both through its `passThrough`
factor.
_Avoid_: health, hit points, HP

**Damage model**:
What an Entity type says about taking damage: its hit points, class `armor`, `armorStructural`,
`explosionShielding`, `minTotalDamageThreshold` and its `DestructionEffects`. Parsed once per
type and shared; the running damage values live on the Entity.
_Avoid_: damage type, armour model

**Destruction**:
The state of an Object that is no longer alive: total damage reached 1, or a fatal hit point of
its class was depleted (a Man's `HitHead`/`HitBody`). Destruction effects then run where the
Object is local — what is left in its place is a Ruin.
_Avoid_: death (a Man's animation state), kill (the event)

**Ruin**:
The Object created in place of a destroyed one, named by a `simulation = "ruin"` entry of its
`DestructionEffects` (a model path). A ruin is an ordinary Object of its own type standing where
the destroyed one was; a destroyed Static object is replaced by it.
_Avoid_: debris, wreck (a vehicle's destroyed model, `DestructWreck`)

**Locality**:
Whether an Object is _local_ (its Owner is this machine, which simulates it and is authoritative
for its state) or _remote_ (another machine owns it; this machine only receives its updates).
Script commands differ in whether their arguments must be local and whether their effects are
global.
_Avoid_: authority (alone), ownership (that is the Owner relation)

## AI

**Group**:
A set of Entities under one leader, all of one Side, whose AI thinks as a unit: one waypoint
queue, one set of modes, one body of knowledge.
_Avoid_: squad (that is the map label), team (that is a UI grouping)

**Side**:
One of the world's opposing alignments (West, East, Resistance, Civilian); the unit of enmity —
two Entities are enemies when their Sides are.

**Waypoint**:
One order in a Group's queue: a position, the modes it sets, and what it asks on arrival.
_Avoid_: marker (that is a map symbol)

**Waypoint queue**:
A Group's ordered Waypoints plus the index of the one it is working on. The mission's
`currentWaypoint` counts from one because index 0 is the group's start position, already done;
the index equals the count once every waypoint is done.

**Waypoint type**:
What a Waypoint asks of the Group on arrival — walk there (MOVE), wait (HOLD), hunt (SAD), board
(GETIN), start over (CYCLE) and so on.
_Avoid_: waypoint mode (the modes are the behaviour, combat, speed and formation fields)

**Group behaviour**:
How a Group moves and how alert it is (CARELESS, SAFE, AWARE, COMBAT, STEALTH); while it says
so, it overrides the combat mode and the formation.
_Avoid_: alertness, stance

**Combat mode**:
A Group's rules of engagement (BLUE, GREEN, WHITE, YELLOW, RED): whether and how it acts on an
enemy it knows about.
_Avoid_: ROE, fire mode

**Speed mode**:
How fast a Group moves (LIMITED walking, NORMAL, FULL).
_Avoid_: pace

**Formation**:
The shape a Group moves in (WEDGE, COLUMN, LINE, VEE, ...), expressed as each follower's slot
around the leader.
_Avoid_: pattern, arrangement

**Target knowledge**:
What a Group knows about one enemy: a certainty from 0 to 4 and where and when it was last
seen. Shared by every unit of the Group, not kept per unit.

## Physics and collision

**Collision world**:
The single simulation world holding the terrain, the Static objects of the loaded land cells and
every Entity's body, in which all of the engine's collision queries run (ADR 0008).
_Avoid_: physics scene (that is the rendering side), broadphase

**Query layer**:
One of the kinds of collision geometry an Object can have — Geometry (solid collision and mass),
Fire Geometry (bullet hits), View Geometry (line of sight) and Roadway (walkable surfaces) —
chosen per query, so a bullet and an eye see different shapes of the same house.
_Avoid_: collision group (that is the solver's filter, in which only Geometry and the terrain
take part)

**Surface info**:
The collision properties of a surface name — roughness, dust, sound environment, bullet
penetration, thickness, friction, restitution and density — from a `.bisurf` file or a
`CfgSurfaces` class; one per name, shared by every face that uses it.
_Avoid_: material (the visual rvmat), Surface type (the class, selected from the terrain)

**Rigid body**:
An Object's presence in the collision world: _dynamic_ when its motion is simulated here, from
forces and contacts, or _kinematic_ when its pose is set from outside (a remote Entity, a man
driven by his own movement code).
_Avoid_: collider (that is the shape alone), physics object

**Interest**:
An area around an Entity, the camera or a script query that streaming keeps loaded; Static
object colliders outside every Interest are unloaded after a while.
_Avoid_: activation range, view distance

## Navigation

**Navigation grid**:
The cell grid `a3-nav` searches a path over: one cell per land cell of the Landscape, each with
a cost (0 = impassable, 100 = open ground, road 70, forest 130, built-up 140, shallow water
200) baked from the geography flags, the heightmap slope and the roads
(ADR 0011).
_Avoid_: navmesh (that is the triangle kind we do not use), oper map (that is the engine's own
field, which also carries cover)

**Nav cell**:
One square of the Navigation grid, named `(x, z)` like the land cell it comes from; the unit a
cost, an obstacle patch and a path node are addressed by.
_Avoid_: tile, square

**Oper map**:
The engine's own name for the per-cell AI field it plans over (`OperMap`, `OperField`, built by
`OperMap::CreateFields` per AI type and combat mode, with cover and clearance layers). We keep
the word for the engine concept; our Navigation grid is the path layer of it.
_Avoid_: cost map (that is the engine's developer display of the oper map)

**Oper position**:
A position on an AI path as the engine stores it: a cell, a type (open ground, road, house,
cover), a cost and a clearance, plus the house or road it belongs to. A path is a list of oper
positions, not cell centres; ours are the smoothed positions `find_path` returns.
_Avoid_: waypoint (that is a scripted AI order, a different thing)

**Path planner**:
The object that runs one path search and keeps its scratch between queries — the open list, the
cost arrays and the generation counter — so that many units can plan without allocating. The
engine's is `AIPathPlanner` with its incremental `ProcessSearching`; ours is `a3-nav`'s
`Planner` behind `Navigator`.
_Avoid_: router, pathfinder (that is the whole crate)

**Path smoothing**:
Turning the cell path an A* returns into positions a Man can walk: a greedy furthest-visible
pull over the cell centres, using the heightmap so a shortcut never leaves walkable ground.
_Avoid_: simplification, decimation

**Building path**:
The indoor navigation of one building model, from its Paths LOD (its Roadway LOD when it has
none): a small triangle graph an outdoor path ends at (via a door position) and a ladder
(`PathActionLadderBottom`/`PathActionLadderTop`) changes floor through. The engine's interface
is `IPaths`; ours is `a3-nav`'s `PathMesh`, which routes within one floor plan — where a
`PathAction` sits in the LOD is not read yet (`docs/re/navigation.md` §8).
_Avoid_: house path index (the engine's terrain-wide lookup of building paths, not the graph)

## Multiplayer and server

**Dedicated server**:
A headless machine that hosts a multiplayer session without a player of its own; it runs the
mission and is the Owner of every Object not owned by a client.
_Avoid_: host (that is a player-hosted server), listen server

**Direct connect**:
Joining a server by entering its IP address and port in the client, rather than picking it from
the server browser.
_Avoid_: IP join

**A2S**:
The Steam server query protocol: UDP requests A2S_INFO (name, map, player counts), A2S_RULES
(key/value rules; Arma packs binary mod/DLC data into them) and A2S_PLAYER (player list), each
guarded by a challenge number. Answered on the query port (game port + 1, default 2303).
_Avoid_: server query, master server

**JIP (join in progress)**:
A client joining a mission that is already running; it must receive the current world state and
the queued persistent messages to catch up.
_Avoid_: late join, hot join

**Network object ID**:
The identifier the original engine gives every networked Object, the same on every machine in the
session, by which create, update, delete and ownership messages refer to it. It is the pair of
the creating machine's client ID and that machine's serial number; Static objects use the
reserved creator 1.
_Avoid_: netId (that is the SQF string form of it), object handle

**Entity ID**:
Our engine's local, per-process handle to an Entity in the World. A deleted Entity's ID never
refers to another Entity. It is not sent over the network.
_Avoid_: object ID, network ID

**Owner**:
The machine (identified by its client ID; the server is 2) on which an Object is local. Ownership
can move between machines during a session.
_Avoid_: host, authority

**Update error**:
The measure of how far an Object's state has moved from the state a receiving player was last
sent, tracked per update class — transform, damage, the destroyed/hidden state flags — and per
player; it decides when and how often the object is sent to that player. A changed state flag
adds 10000, damage adds 10 × |Δdamage|.
_Avoid_: priority (that is the order that comes out of it), delta

**verifySignatures**:
The server setting that decides whether clients' addons must match Bisigns under the server's
Bikeys; value 0 disables the check.
_Avoid_: signature checking, addon verification

**Server password**:
The password a client must supply to join a server.
_Avoid_: join password

**Admin password**:
The password a joined player supplies with the `#login` chat command to become server admin.
_Avoid_: login password, rcon password

**Mission transfer**:
The server sending the mission PBO to a joining client that does not already have that exact
mission.
_Avoid_: mission download

## Input

**User action**:
A named thing the player can do (`moveForward`, `defaultAction`, `cameraMoveUp`), queried by
the engine and by the `inputAction` script command. Names compare case-insensitively.
_Avoid_: command, input event

**Keybinding**:
One way to trigger a User action: an input (key, mouse button or axis, gamepad input), an
optional modifier (`LCtrl+X`) and a trigger (press, double tap, hold). Stored as integer key
codes in `CfgDefaultKeysPresets` and the Profile.
_Avoid_: shortcut, hotkey

**Key preset**:
A named set of default Keybindings in `CfgDefaultKeysPresets` (`Arma3Apex` is the default),
which the Profile's own keybindings override.
_Avoid_: key scheme (controller schemes are separate classes), layout

**DIK code**:
A DirectInput keyboard scancode (`DIK_W` = 0x11); RV's identity for keyboard keys.

## UI

**Display**:
A top-level UI screen or dialog, defined by a config class (usually derived from a `Rsc*` base),
owning Controls.
_Avoid_: window, screen, menu

**Control**:
One UI element inside a Display (button, list box, text, map), defined by a config class with a
numeric `type` and `style`.
_Avoid_: widget

**Safe zone**:
The whole screen in viewport units, the values of `safeZoneX/Y/W/H`; `[0.5, 0.5]` is always the
screen centre.
_Avoid_: screen rectangle, bounds

**Interface size**:
The UI scale setting (Very Small to Very Large, `uiScale`), which scales the 4:3 viewport the Safe
zone is measured in.
_Avoid_: DPI scale, resolution scale

**Pixel grid**:
The screen-height-derived unit that keeps UI aligned across resolutions: `pixelGrid`,
`pixelGridNoUIScale` and `pixelGridBase`, from config `uiScaleMaxGrids` and `uiScaleFactor`.
_Avoid_: pixel step

**Draw list**:
One frame of UI drawing: the Controls' quads in draw order, each a screen-space rectangle with a
colour, a texture path and an optional clip, plus the texture paths they name.
_Avoid_: render list, command buffer

**Curator (Zeus)**:
The real-time game-master mode in which a player places and commands entities during a running
mission. The engine name is Curator; Zeus is the product name.

**3DEN (Eden editor)**:
The in-engine 3D mission editor that produces mission.sqm files.
