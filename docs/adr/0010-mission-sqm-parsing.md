---
status: accepted
---

# `mission.sqm` is parsed by `a3-config`, into a `Mission` value that is spawned in a second step

`crates/a3-mission` loads a mission — `mission.sqm`, `description.ext` and the mission scripts —
and puts it into a `World`. It is a crate of its own, not a module of `a3-world` or `a3-gamedata`:
loading and running a mission needs the config parser and the SQF VM, and the World (and the
renderer, and `apps/arma3`) only need the result.

The pipeline has three steps, each usable on its own:

- `load_mission` reads `mission.sqm` from the VFS and, when the folder has one, `description.ext`
  through the preprocessor and parser, and returns a `Mission` value. Nothing touches a `World`.
- `spawn_mission` creates the Entities at the SQM positions, puts them into groups and applies
  their headings. Units the config does not know are reported, not fatal.
- `run_scripts` installs `player` and the named units as `missionNamespace` variables, runs the
  units' `init` fields unscheduled with `this` set, then `init.sqf` scheduled.

## Decisions

- **`mission.sqm` goes through `a3-config`.** The file is the editor's own config syntax
  (`version=12; class Mission { class Groups { items=26; class Item0 { ... } } }`); rapified
  missions are the same tree in the binary format `a3-config` already reads and writes. A parser
  per format would need the number semantics to match, or every position would be "close
  enough"; `a3-config` gives the file the same reading as `config.bin`.
- **The SQM value is not a `ConfigTree`.** `Mission` is a flat, typed value (`groups`, `objects`,
  `markers`, `triggers`, `intel`) with the SQM keys as fields, so the conversion to World types
  is a plain function and a test can assert on a `Unit` without a config lookup. `items=N` is
  ignored in favour of the `ItemK` classes actually present (the count and the entries disagree
  in shipped files more than once).
- **`position[]={x, y, z}` is `(x, z, y)` in world space** (ADR 0003): SQM's second component is
  height above sea level, and a two-element position means "on the surface".
- **`azimut` is `getDir` degrees clockwise from north** and is applied as an Entity heading;
  `setDir`-style negative values wrap.
- **The init order is the engine's**: unit `init` fields unscheduled (suspending in one is an
  error, as in the engine), then `init.sqf` scheduled against the World clock, so `sleep` and
  `waitUntil` work.
- **`this` is bound for init fields.** The VM's parameter variable is `_this`; the engine also
  binds `this` in an init field, which shipped missions rely on (`this allowDamage false;`), so
  the runner sets a mission-namespace `this` per unit and clears it before `init.sqf`.
- **What the VM cannot run is reported, not fatal**: `RunReport::missing_commands` lists the
  commands the mission's scripts use that have no implementation, with counts, so "what does
  the campaign still need?" is a test assertion.

## Considered options

- **The `a3-sqf` lexer for the SQM**: rejected. It lexes SQF expressions; the config grammar is
  the same token stream but the numbers are 32-bit-friendly in one and f32-backed in the other,
  and `a3-config` already round-trips config text and rap.
- **A dedicated SQM parser**: rejected for now. It is a fourth grammar for the same syntax, and
  every shipped SQM is a config file; if the config crate ever cannot express a construct the
  SQM uses, that construct is a config bug worth fixing there.
- **Spawning while loading**: rejected. Markers, triggers and waypoints have no World
  representation yet, and a tool that only prints a mission must not create Entities.
- **A `World::load_mission`**: rejected. The World owns Entities, not file formats; keeping the
  mission types outside it is what lets `spawn_mission` stay a pure function of `(World, types,
  Mission)`.

## Consequences

- The `Mission` value is a snapshot: triggers and waypoints are carried but not applied (the
  World has no AI or trigger system yet), and markers are returned by `Spawned::markers` rather
  than created.
- `crates/a3-mission` depends on `a3-config`, `a3-gamedata`, `a3-sqf`, `a3-vfs` and `a3-world`.
  `apps/arma3` can embed the whole pipeline in one call, and `a3-tools mission` prints what a
  mission places without opening a window.
- A command missing from the VM shows up as a failed script in the report rather than stopping
  the load, so the campaign can be run at any point of the command surface and what is left to
  implement falls out of the report.
