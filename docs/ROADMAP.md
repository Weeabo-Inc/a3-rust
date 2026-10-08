# Roadmap

Phases run roughly in order; later phases may start once the parts they depend on are usable.
Each phase has a GitHub milestone and one `type:epic` issue; task issues hang off the epic.

## Phase 0 — Foundation & RE tooling

Scope: repository, CI, conventions; RE tooling (Ghidra headless + MCP, rea, IDA Free); binary
recon of the game executable — RTTI class hierarchy, imports, strings, the script command table;
inventory of the game data (PBO list, prefixes, file types and counts). **Priority:** RE and
documentation of the network protocol (client <-> Dedicated server and A2S) from
`arma3_x64.exe` / `arma3server_x64.exe` and traffic captures, per
[ADR 0004](adr/0004-official-client-compatibility.md); Phase 4 depends on its network object
model.

Exit criteria:
- CI green on ubuntu and windows; conventions documented in `AGENTS.md`.
- Ghidra analysis of the main executable reproducible from a script; findings exported to
  `docs/re/`.
- `docs/re/` contains the RTTI class hierarchy, the script command table (name, form, argument
  types), and a data inventory of the install.
- `docs/re/` documents the network protocol: transport and framing, handshake and version check,
  message type catalogue, network object model, Mission transfer, JIP sequence, and A2S answers.

## Phase 1 — Data formats & VFS

Scope: compression (LZSS, LZO, LZ4 if used); PBO reading incl. header properties and prefixes;
VFS mounting every addon by prefix with priority and overrides; rapified config read/write;
config.cpp parser (on the Phase 2 preprocessor, or a minimal one); class inheritance resolution;
merged config tree across all CfgPatches ordered by `requiredAddons`; stringtable.xml; PAA/PAC
decode (DXT1, DXT5, ARGB variants); P3D ODOL and MLOD readers; RTM; WRP (OPRW); WSS;
bisign/bikey verification; an `a3-tools` subcommand for each.

Exit criteria:
- Every PBO in the install opens; the VFS resolves any path a config references.
- The merged config of the full game loads; derap → rapify round-trips byte-identically or
  semantically on all shipped config.bin files.
- All shipped PAA, P3D (ODOL), RTM and WRP files parse without error; spot checks render/export
  correctly via `a3-tools`.
- All PBOs verify against the shipped bikeys.

## Phase 2 — Scripting (preprocessor, config, SQF VM)

Scope: preprocessor (`#define` with arguments, `#include`, `#ifdef`, `__EVAL`, `__EXEC`); SQF
lexer, parser and compiler to an instruction stream; VM with scheduled (`spawn`, `execVM`,
`sleep`, `waitUntil`) and unscheduled environments; value types; namespaces and variables;
command registry populated from the full command signature table (binary + wiki); core
non-world commands (math, strings, arrays, hashmaps, control flow, config access); SQS; FSM;
SQM mission loader.

Exit criteria:
- All SQF files in the install compile without error.
- Core command test suite matches documented results of the original.
- A mission.sqm from the install loads into an in-memory mission description.

## Phase 3 — Engine core (window, renderer, audio, input)

Scope: winit app window; wgpu renderer for terrain (heightmap, satellite and mask layers, clutter
later), ODOL models with LOD selection, rvmat materials and shaders, sky, lighting and fog;
camera; input mapping; 3D positional audio engine driven by config sound shaders; text and fonts.

Exit criteria:
- A free camera flies over Altis with terrain, placed objects and sky rendered at interactive
  frame rates.
- Positional sounds play from config definitions.

## Phase 4 — Simulation (world, entities, physics, AI, weapons)

Scope: World and Landscape object model; entity types (Man, Car, Tank, Helicopter, Plane, Ship,
StaticWeapon, Building, Thing); simulation loop; physics on rapier; animations (RTM, skeletons,
config animations and sources); weapons, ballistics and hit-point damage; AI (FSM-driven, groups,
waypoints, pathfinding); event handlers; all world-related SQF commands. The World and Entity
model mirrors the original's network object model from the start — Network object IDs, Owner and
Locality, which machine simulates what, and the object create/update/delete and
ownership-transfer message flow — per [ADR 0004](adr/0004-official-client-compatibility.md), so
Phase 6 needs no retrofit.

Exit criteria:
- A soldier walks, enters and drives a vehicle, fires weapons, and takes damage.
- AI groups follow waypoints and engage enemies.
- Mission scripts that touch the world run without unimplemented-command errors.
- Every Entity carries a Network object ID and an Owner, and its simulation runs only where it is
  local, matching the network object model documented in `docs/re/`.

## Phase 5 — UI, missions & game loop

Scope: config-driven UI (Displays, Controls, Rsc* classes); main menu; scenario loading from
`missions/` and mission PBOs; end-to-end single-player game loop. 3DEN and Zeus come later.

Exit criteria:
- From the main menu, a shipped single-player scenario starts, plays and ends.

## Phase 6 — Multiplayer & dedicated server

Goal: official Arma 3 2.22 clients join our Dedicated server via Direct connect
([ADR 0004](adr/0004-official-client-compatibility.md)).

Scope:
- Network protocol, byte-compatible with the official 2.22.0.154103 client, reverse engineered
  from the original and documented in `docs/re/` first.
- Dedicated server binary.
- A2S (A2S_INFO, A2S_RULES, A2S_PLAYER, including challenge handling) answered directly, without
  Steamworks.
- Steam auth tickets accepted but not validated.
- No BattlEye.
- No signature verification (`verifySignatures=0` behaviour).
- Authentication by Server password and Admin password (`#login`) only.
- Version/build check and addon/mod list matching as the client expects.
- Mission transfer of the mission PBO to the client.
- Locality and ownership; join in progress (JIP).
- Out of scope: Steamworks SDK, Steam ticket validation, BattlEye, signature checks; our client
  joining official servers.

Exit criteria:
- An official 2.22 client joins our server, plays a shipped MP scenario, with JIP.
