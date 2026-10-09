# Fidelity tools

How close the engine is to Arma 3 2.22.0.154103, measured by tools that any agent can rerun.
Each tool writes its raw output to `.work/` (never committed) and a summary table to this folder.
Tracking epic: #261.

| Tool | Question it answers | Summary | Run it with |
|---|---|---|---|
| [Scenario sweep](#scenario-sweep) | Which shipped scenarios load, start and run, and what breaks most? | [`scenario-sweep.md`](scenario-sweep.md) | `a3-sweep` (`apps/a3-sweep`) |
| [SQF coverage ledger](#sqf-coverage-ledger) | Which script commands are implemented, ranked by how much shipped code uses them? | [`sqf-coverage.md`](sqf-coverage.md), [`sqf-verified.tsv`](sqf-verified.tsv) | `a3-tools sqf coverage` |
| [SQF server oracle](#sqf-server-oracle) | Do probes give the same answers on `arma3server_x64.exe` and on our VM? | [`sqf-oracle.md`](sqf-oracle.md) | `python tools/oracle/oracle.py run` |
| [Render oracle](#render-oracle) | Does a frame render like the real client's? | [`render-oracle.md`](render-oracle.md) | `python tools/oracle/render/render_oracle.py` |
| [RE gap ledger](#re-gap-ledger) | Where does the implementation still guess? | [`re-gaps.md`](re-gaps.md) | `python tools/re/re_gaps.py` |

The sweep is the whole-game number: it runs shipped content end to end and ranks every problem by
how many scenarios it affects. The other tools explain *why* a particular behaviour is wrong and
check it against the original.

## Scenario sweep

Loads every shipped scenario headlessly with the engine and reports pass/fail per scenario, the
most frequent error signatures and the unimplemented commands ranked by how many scenarios they
block. Use it to pick the next thing to fix: the top of its tables is what breaks the most of the
game.

```sh
# From the repository root; the game install comes from --game-dir or A3_ROOT.
cargo run --release -p a3-sweep -- --game-dir P:\a3-rust\oirignal --doc docs/fidelity/scenario-sweep.md

# A subset: one world, one kind, a few seconds each.
cargo run --release -p a3-sweep -- --filter altis --kind campaign --seconds 10

# Inventory only: every mission folder of the install, classified.
cargo run --release -p a3-sweep -- --list

# Regenerate the summary from an earlier run (does not load the game).
cargo run --release -p a3-sweep -- --summarize .work/sweep/<timestamp>.json --doc docs/fidelity/scenario-sweep.md

# Debug one scenario in this process (no crash isolation).
cargo run --release -p a3-sweep -- --filter boot_m01 --in-process
```

The install holds 396 mission folders; 238 of them are scenarios (campaign, scenario, showcase,
challenge, tutorial, multiplayer, cutscene, unlisted) and 158 are **Mission fragments** (Contact
sites, Old Man layers, a folder nested inside another mission) that are skipped unless `--all`.
A full run (238 scenarios, 60 simulated seconds each, 3 workers) takes about two minutes; each
worker loads the game once (about 2 GB of memory). Run it with `--release`: the recorded
`ms/frame` is only comparable within one build profile, and the summary says which one produced
it.

**What it runs.** The inventory is every folder with a `mission.sqm` in the VFS (base game, every
DLC, the loose `Missions`/`MPMissions` PBOs; encrypted EBOs are skipped), classified by where
`CfgMissions` lists it. Each scenario then goes through the engine's start-up, as `a3-mission`
implements it:

1. the world's terrain (WRP) into a fresh `World`;
2. `mission.sqm` and `description.ext` (`load_mission`), the campaign's `description.ext` as
   `campaignConfigFile` for campaign missions;
3. the units, groups and objects (`spawn_mission`);
4. `start_mission`: the function library's mission start (`initFunctions.sqf`: campaign and
   mission functions, preInit functions, then the spawned postInit sequence with
   `initServer.sqf` and `initPlayerLocal.sqf`), each unit's `init` field, the 3D editor's entity
   attribute expressions, then `init.sqf` spawned;
5. `--seconds` of simulation at `--fps` (World step plus `--frame-budget-ms` of scheduled
   scripts per frame), stopping early after `--budget` wall-clock seconds.

The function library is compiled once per worker at "game start" and copied into each scenario's
VM. Scenarios that ship with an optional DLC (Contact, creator DLC) run with the optional DLC
loaded; all others run on the base game and its default DLC, as a player would start them.

**What it records** (per scenario, in `.work/sweep/<timestamp>.json`): status, the last stage
reached, timings (ms per simulated frame), script errors grouped by Error signature,
"Unimplemented command" hits at runtime, unimplemented commands the mission's own scripts use
(every `.sqf` of the folder, init fields, trigger and 3D-editor attribute expressions, compiled
statically), stubbed commands those scripts use, files that do not compile, units that could not
be created, units the World placed more than a metre from their SQM position (this is the check a
misread coordinate lands in; a unit merely high above the terrain does not), model files missing
from the VFS, and sanity checks (player present and alive, no non-finite positions, nothing below
the terrain).

**Two pass numbers.** A `pass` is a scenario with no finding at all. A command whose record in
the verification ledger (`docs/fidelity/sqf-verified.tsv`, `stub` rows, `--stubs` to point
elsewhere) says *stub* runs without error while its effect is a stand-in, so a scenario that
calls one passes on the stand-in. The summary therefore reports **pass** and **pass with no
stubbed command** side by side, with the stub set's record count and sha1: the second number is
the one to quote, and the gap between them is what the stubs are hiding.

**Status.** `pass` (no finding at all), `errors` (ran to the end with findings), `load_failed`
(`mission.sqm`, `description.ext` or the terrain did not load), `panic` (caught, the worker goes
on), `crash` (the worker process died: stack overflow, abort), `timeout` (killed after
`--hard-timeout`). Scenarios run in worker processes (`--jobs`), so a crash or hang costs one
scenario, never the sweep; a dead worker is replaced.

**Not covered yet.** Triggers are compiled (for the static command scan) but not evaluated, as
the World has no trigger system; waypoints are not given to the AI; the player is a unit like any
other (no input); physics only moves what the World simulates, so most scenarios see little
change in 60 seconds. The sweep is headless: no UI, no rendering, no sound.

## SQF coverage ledger

`a3-tools sqf --all-mods coverage` lists every command overload of the engine's command table
with its implementation status and how often shipped code uses it; [`sqf-coverage.md`](sqf-coverage.md)
is the generated backlog, and [`sqf-verified.tsv`](sqf-verified.tsv) records how each implemented
overload was verified. The scenario sweep's command table ranks the same commands by how many
scenarios they block, which weighs what a player meets first.

## SQF server oracle

`python tools/oracle/oracle.py run` runs every probe in `tools/oracle/probes/` twice: on the
original `arma3server_x64.exe` and on `a3-tools sqf exec`, in the same world, and diffs the
answers. [`sqf-oracle.md`](sqf-oracle.md) is the generated report. Use it to settle semantics the
decompiled handlers leave open; the sweep says which commands are worth settling first.

## Render oracle

`python tools/oracle/render/render_oracle.py` renders the same shots in the real 2.22 client and
in our renderer (same camera, date, time, weather, view distance) and compares the images with
`a3-tools image-diff`. [`render-oracle.md`](render-oracle.md) describes the shots and the
metrics; screenshots stay in `.work/oracle/`.

## RE gap ledger

`python tools/re/re_gaps.py` finds every confidence marker and open question in `docs/re/`;
[`re-gaps.md`](re-gaps.md) is the curated list of what the implementation still guesses and how
visible each guess is.
