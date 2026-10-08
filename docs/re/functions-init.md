# Function library initialisation (CfgFunctions `init`)

How the engine starts the SQF function library (`BIS_fnc_*` and every addon's CfgFunctions).
Implemented in `crates/a3-gamedata/src/boot.rs` (`init_functions`).

## Engine side: `RunInitFunctionsScript` — `FUN_14063c8f0`

Found from the string `"RunInitFunctionsScript file %s preprocessor error!"` (`0x141b22c28`),
referenced only from `FUN_14063c8f0` (and a second copy in `FUN_1413ae9d0`).

1. Reads `configFile >> "CfgFunctions" >> "init"` (key string `"init"` at `0x141aa3c44`) as text.
   In 2.22 it is `A3\functions_f\initFunctions.sqf`.
2. Preprocesses the file through the global preprocessor object (`PTR_14208b610`, vtable `+0x10`).
   On failure it logs `RunInitFunctionsScript file %s preprocessor error!` and stops.
3. Compiles the text (`FUN_1402fac30`) and runs it (`FUN_1402fbb30`) on the global script VM
   (`DAT_14208b070`), unscheduled, with an empty argument list: **`_this` is undefined**.
4. The namespace argument is `*(DAT_14220dc60 + 0x1840)`. The `missionNamespace` nular command
   (handler `0x1408b0fb0`) returns exactly that pointer, so **the script runs in
   `missionNamespace`**. (`uiNamespace`, handler `0x1408b1080`, returns the separate global
   `DAT_142222e30`.)

`FUN_14063c8f0` has four callers (`FUN_14061e6f0`, `FUN_140627d30`, `FUN_140634550`,
`FUN_140636b90`). Together with the script's own mode detection this matches the wiki: the
function runs at game start and again at every mission start. Which caller is game start was not
traced _(uncertain)_.

## Script side: `initFunctions.sqf` modes

With `_this` undefined, the script picks its mode itself:

- `uiNamespace getVariable "bis_fnc_init"` unset (game start): mode 0. It compiles every
  `configFile`, `campaignConfigFile` and `missionConfigFile` CfgFunctions entry. Addon functions
  go to uiNamespace with `compileScript [path, compileFinal, header]` plus a `<var>_meta` entry.
  It creates missionNamespace shortcuts to the uiNamespace code, then sets
  `uiNamespace bis_fnc_init = true`.
- Set (mission start): mode 3. It compiles campaign/mission functions, runs `preInit` functions,
  and spawns the `postInit` / `initServer.sqf` / `initPlayerLocal.sqf` sequence.

It reads `cheatsEnabled`, `is3DEN`, `"Preferences" get3DENMissionAttribute "RecompileFunctions"`
(always evaluated, `&&` is not short-circuited there) and `findDisplay 26`. The headless VM
answers these as a retail main menu: no cheats, no editor, no displays.

`preStart` functions run only in mode 2 (`[2] call ...`), which the engine's game-start call
does not use. Where the engine runs preStart functions is not traced _(uncertain)_.

## Observed with a3-rust (2.22 install, all optional DLC mounted)

- **Result:** all 2137 distinct `<tag>_fnc_<name>` functions compiled into uiNamespace in about
  1.2 s (release build), and `bis_fnc_init` was set. Without optional DLC: 1765 functions.
- **Missing files:** two declared functions point to files that do not exist
  (`missionTasks.sqf`, `missionConversations.sqf`). `compileScript` gives empty code for these, as
  `compile preprocessFileLineNumbers` would.
- **`0 = expr` statements:** three shipped functions (`fn_fire.sqf`, Contact's
  `fn_updateIDWMapDrawData.sqf` and `fn_preInitTXScan.sqf`) contain them, so the engine's compiler
  accepts a number as assignment target.
