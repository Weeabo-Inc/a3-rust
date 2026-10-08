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

**Animation phase**:
A position in an RTM's cycle, from 0 (start) to 1 (end). Keyframes and keystones sit at phases.
_Avoid_: time, frame (a keyframe is the pose stored at one phase)

**Animation keystone**:
A named marker at a phase of an RTM, such as `StepSound` (a footstep), that the engine fires
while the animation plays.
_Avoid_: event (alone), animation event

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

**Sound shader / sound set**:
Config-defined descriptions of how a sound is played (sample choice, volume curves, range) used
by the modern sound system.

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

**Locality**:
Whether an Object is _local_ (its Owner is this machine, which simulates it and is authoritative
for its state) or _remote_ (another machine owns it; this machine only receives its updates).
Script commands differ in whether their arguments must be local and whether their effects are
global.
_Avoid_: authority (alone), ownership (that is the Owner relation)

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

**Curator (Zeus)**:
The real-time game-master mode in which a player places and commands entities during a running
mission. The engine name is Curator; Zeus is the product name.

**3DEN (Eden editor)**:
The in-engine 3D mission editor that produces mission.sqm files.
