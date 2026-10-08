# Moves: CfgMoves states, the move graph and the move path search

Implemented in `crates/a3-moves`. Addresses are VAs in `arma3_x64.exe` 2.22.0.154103 (Ghidra
project `a3`). The Man simulation that plays moves is in `a3-world` (`sim/man.rs`).
`sim-man-movement.md` is the overview of Man movement (config model, velocity blend, slopes,
collision); this file adds the loader's exact edge order, the class-level transition lists,
`connectAs`, the move queue and the details of the path search.

## Moves type (`0x140606300`, high)

One per CfgMoves class (`CfgMovesMaleSdr`, named by CfgVehicles `moves=`):

1. `skeletonName` → the shared skeleton object (cached by name).
2. `States`: every entry is registered by name, in config order; the index is the **move id**
   (16-bit in the graph, so at most 32767 moves).
3. Per move: the move parameters (below), then `ignoreMinPlayTime[]` per move (list of move ids;
   `0x140603410`).
4. The graph, edges added in this order (a later add of the same `from → to` replaces kind, cost
   and flag; `from == to` is ignored):
   - `Interpolations` class: each entry `{cost, m1, m2, ...}` adds an interpolation edge for every
     ordered pair of distinct listed moves.
   - `transitionsInterpolated[] = {from, to, cost, ...}` (interpolate) and
     `transitionsSimple[] = {from, to, cost, ...}` (connect). Unknown names log
     `Bad ipol transition from %s to %s`.
   - Per move `me`, in move order: `connectFrom[]` (`x → me`, connect), `connectTo[]`
     (`me → x`, connect), `interpolateWith[]` (both ways, interpolate), `interpolateTo[]`
     (`me → x`), `interpolateFrom[]` (`x → me`). Lists are `{name, cost, ...}` pairs; an unknown
     or empty name is reported and skipped.
   - `connectAs = "M"`: for every `t` with an edge `M → t` and no edge `me → t`, add `me → t`
     with M's kind and cost; for every `u` with `u → M` and no `u → me`, add `u → me`.
   - `transitionsDisabled[] = {from, to, ...}`: remove the edge (`Bad disabled transition`).

Edge record (6 bytes, `0x140603c60`): `i16 to`, `i16 cost = clamp(round(cost * 1000))`, `u8
kind` (1 = connect / `connectTo`, 2 = interpolate), `u8 ignoreMinPlayTime` (set for
interpolation edges whose target is in the source move's `ignoreMinPlayTime[]`; always 0 for
connect edges and `connectAs` copies). Lookup of a missing edge returns a static record with
kind 0.

## Move parameters (`0x1405fe3d0`, high for the fields listed)

`Actions` (the action map class, looked up in the moves type's `Actions`), `speed` (**a
negative value `s` is replaced by `-1 / s`**: a duration in seconds), `skillSpeedCoef`
(default 1), `relSpeedMin`, `relSpeedMax`, `equivalentTo` (move id), `interpolationSpeed`,
`interpolationRestart` (int), `walkcycles`, `terminal`, `minPlayTime` (clamped to 0..1),
`reverse` (move id), `affectedByFatigue` (default 1), `ragdoll`, `preload` (loads the RTM at
once). `a3-moves` reads many other display/weapon flags straight from config.

## Move queue and path search (high)

- `playMove` (`0x140538c30` → `Man` vtable `+0x1058` = `0x14073ed30`): looks the move up by name
  (unknown → nothing), refuses while a blocking state is active, then **appends** `{move,
  context}` to the Man's move queue (`Man + 0x1de0`, 16-byte items).
- `playMoveNow` (`+0x1060` = `0x14073efd0` → `0x14073eec0`): **clears the queue** and sets the
  move as the immediate target of the Man's move planner (`Man + 0x1348`, target at `+0x440`).
- Path search (`0x140604c40`, `MovesType::FindPath(path, from, target)`):
  - `from == target` → empty path, success.
  - A direct edge `from → target` of any kind → path `[target]`, whatever its cost.
  - Else Dijkstra from `from` over integer costs with a binary heap holding every move that
    has edges; **the loop runs only while `dist[target]` is unset**, so it stops at the first
    relaxation of the target — not necessarily the cheapest path. Unreachable → failure.
  - The path is built backwards through the predecessor table, target last; the request's
    context rides on the target item.

## Real data (build 2.22)

`CfgMovesMaleSdr`: 5,575 moves, 43,808 edges, 1,966 action maps; every name in the transition
lists resolves (some action entries name moves that do not exist, e.g. `medicUp =
"AinvPknlMstpSlayWrflDnon_medicUp"`). 4,041 distinct RTMs, all present. Stand → prone with a
rifle (`AmovPercMstpSrasWrflDnon` → `AmovPpneMstpSrasWrflDnon`) interpolates into
`AmovPercMstpSrasWrflDnon_AmovPpneMstpSrasWrflDnon`, which connects to the prone move. Rifle
walk `AmovPercMwlkSrasWrflDf`: RTM step `(-0.0025, 0, -1.624)`, `speed = 0.85` → 1.38 m/s if
speed is cycles per second times the step length (confirmed by `sim-man-movement.md` §3).

## Open

- How the Man consumes the path each step (when a connect edge fires relative to phase 1,
  blending weights, `interpolationRestart`, `minPlayTime` checks), how `relSpeedMin/Max` scale
  the speed with the requested movement speed, and variants/idles timing.
- `equivalentTo` and `reverse` use in the planner.
