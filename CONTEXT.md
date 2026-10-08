# Arma 3 / Real Virtuality

The vocabulary of the Real Virtuality 4 engine and the Arma 3 game data it runs. Definitions
marked _(uncertain)_ are best current understanding and need confirmation by reverse engineering.

## Packaging and file system

**PBO**:
An archive file holding a set of game files plus a header of string properties (such as `prefix`).
The unit in which all game content ships.
_Avoid_: archive, pak

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

**CfgPatches**:
The root config class in which each addon declares itself, its `requiredAddons` (which decides
merge order) and the units/weapons it adds.

**CfgVehicles / CfgWeapons / CfgAmmo / CfgMagazines**:
Root config classes defining every entity type, weapon, projectile and magazine by name.

**Stringtable**:
A `stringtable.xml` file mapping `STR_` keys to localised text in each supported language.

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
`_dt` detail, `_sat` satellite, `_mask` surface mask.

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

**Proxy**:
A placeholder in a LOD that references another P3D, placed at a position and orientation;
used for crew seats, weapons on a vehicle, and attached decorations.

**Skeleton**:
The ordered list of bones (with parents) that a model's animations act on, defined in
CfgSkeletons.

**RTM**:
An animation file holding per-frame bone transforms for a Skeleton, used mainly for character
moves.

**Model animation**:
A config-defined transform (rotation, translation, hide) of a named selection, driven by an
Animation source such as a door state, wheel rotation or gun elevation.

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

**Entity**:
A dynamic, simulated object in the World (soldier, vehicle, projectile).
_Avoid_: actor, game object

**Object**:
Any instance placed in the World with a model and position, simulated or static. Every Entity is
an Object; buildings and trees on the Landscape are Objects but not Entities _(uncertain: exact
class split)_.

**Simulation**:
The behaviour class that drives an Entity type each frame, selected by the `simulation` config
entry (e.g. `soldier`, `carx`, `tankx`, `helicopterrtd`, `airplanex`, `shipx`, `house`, `thing`).

**Locality**:
Which machine in a multiplayer session owns and simulates a given Object. Script commands differ
in whether their arguments must be local and whether their effects are global.

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
