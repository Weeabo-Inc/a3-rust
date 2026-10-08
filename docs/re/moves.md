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
   `0x140603410`); every move's `ignoreMinPlayTime` is read before any edge is added.
4. The graph, edges added in this order (a later add of the same `from → to` replaces kind, cost
   and flag; `from == to` is ignored):
   - `Interpolations` class: each entry `{cost, m1, m2, ...}` adds an interpolation edge for every
     ordered pair of the listed moves (`m_j → m_i` for all `i != j`, so both ways, cost from
     element 0), with the flag taken from the source move's `ignoreMinPlayTime`. Names that do
     not resolve are dropped without a log line.
   - `transitionsInterpolated[] = {from, to, cost, ...}` (interpolate, flag from `from`'s
     `ignoreMinPlayTime`) and `transitionsSimple[] = {from, to, cost, ...}` (connect, flag 0).
     Unknown names log `Bad ipol transition from %s to %s` / `Bad simple transition from %s to
     %s`.
   - Per move `me`, in move order: `connectFrom[]` (`x → me`, connect), `connectTo[]`
     (`me → x`, connect), `interpolateWith[]` (both ways, interpolate; each direction's flag
     looked up in its own source move's list), `interpolateTo[]` (`me → x`), `interpolateFrom[]`
     (`x → me`). Lists are `{name, cost, ...}` pairs; an unknown or empty name logs
     `"  <State>: Bad move <name>"` (`0x1406045e0`) and is skipped. A list is read while
     `i + 1 < len`, so a trailing odd element is ignored.
   - `connectAs = "M"`: for every `t` with an edge `M → t` and no edge `me → t`, add `me → t`
     with M's kind and cost; for every `u` with `u → M` and no `u → me`, add `u → me`. The copies
     never carry the `ignoreMinPlayTime` flag, and a name that does not resolve is skipped
     silently. M's cost round-trips through `(f32)stored * 0.001` and back through the ×1000
     rounding, which is exact.
   - `transitionsDisabled[] = {from, to, ...}`: remove the edge — only the two names are read,
     the entries carry no cost. A name that does not resolve logs `Bad disabled transition from
     %s to %s`; removing an edge that does not exist is silent.
5. Each move's edge array is then compacted to its exact length (`0x140609390`, 6-byte records,
   element count then capacity). Nothing sorts or merges them: the lists stay in insertion order,
   which is the order `Moves::edges` returns.

Edge record (6 bytes, `0x140603c60`): `i16 to`, `i16 cost = clamp(round(cost * 1000))`, `u8
kind` (1 = connect / `connectTo`, 2 = interpolate), `u8 ignoreMinPlayTime` (set for
interpolation edges whose target is in the source move's `ignoreMinPlayTime[]`; always 0 for
connect edges and `connectAs` copies). Lookup of a missing edge returns a static record with
kind 0. The class-level lists are read in triples with an `i += 3` loop bounded by the entry
count, so an unterminated triple reads past the end (we ignore the remainder instead).

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
  - Else Dijkstra from `from` over integer costs; the heap is seeded with every move (the dist
    array is the key, `0x7fffffff` = unset) and **the loop runs only while `dist[target]` is
    unset**, so it stops at the first relaxation of the target — not necessarily the cheapest
    path. A relaxation with `d > 0x7fffffe` breaks out. Unreachable → failure.
  - The path is built backwards through the predecessor table, target last; the request's
    context rides on the target item.
  - Ties: the engine's binary heap holds every move, so among equal distances the one expanded
    first is decided by the heap's own layout; two equal-cost paths may therefore differ from
    ours, which breaks ties by move id.

## Real data (build 2.22)

`CfgMovesMaleSdr`: 5,575 moves, 43,808 edges, 1,966 action maps (1,234,872 action entries);
every name in the transition lists resolves (some action entries name moves that do not exist,
e.g. `medicUp = "AinvPknlMstpSlayWrflDnon_medicUp"`). 4,041 distinct RTMs, all present. Stand
→ prone with a rifle (`AmovPercMstpSrasWrflDnon` → `AmovPpneMstpSrasWrflDnon`) interpolates
into `AmovPercMstpSrasWrflDnon_AmovPpneMstpSrasWrflDnon`, which connects to the prone move.
Rifle walk `AmovPercMwlkSrasWrflDf`: RTM step `(-0.0025, 0, -1.624)`, `speed = 0.85` → 1.38 m/s
if speed is cycles per second times the step length (confirmed by `sim-man-movement.md` §3).

Shapes checked in the shipped config (a `config dump` of `CfgMovesMaleSdr`, 2.9 MB):

- The class-level `Interpolations`, `transitionsInterpolated`, `transitionsSimple` and
  `transitionsDisabled` are **empty**: every real edge comes from the per-state lists, so those
  engine paths are covered by synthetic tests only.
- 2,601 `ConnectTo` and 4,049 `InterpolateTo` / 1 `InterpolateFrom` lists are flat, even-length
  `{name, cost}` arrays (quoted even items, numeric odd ones; no odd-length list). 8
  `ignoreMinPlayTime` lists, each a single name. `connectAs` does not appear. `variantsAI`
  appears 577 times as name/probability pairs.
- 16 `primaryActionMaps`; `skeletonName = OFP2_ManSkeleton`; `gestures = CfgGesturesMale`.
- The loader's 1,966 action maps are exactly the classes of the resolved
  `CfgMovesMaleSdr/Actions`. Its 65 warnings are all action entries: `ManActions` is a base
  class under `CfgMovesBasic` (not under the moves type), and maps deriving from it name moves
  that do not exist, e.g. `GestureReloadRPG7` — a quirk of the shipped config, not of the
  loader.

## Open

- How the Man consumes the path each step (when a connect edge fires relative to phase 1, how the
  blend weight advances and what `interpolationRestart` restarts, `minPlayTime` checks), how
  `relSpeedMin/Max` scale the speed with the requested movement speed, and variants/idles timing.
- `equivalentTo` and `reverse` use in the planner.
- The blending of two moves in progress is known from the animation side (`crates/a3-pose`, and
  `model-animations.md` "RTM skeletal poses"), and it pins what a transition may mix: the moves
  are blended in **joint space**, not record space — every bone's loaded record is
  `[R | R * pivot + t]`, the blend slerps the rotations and lerps those posed joints (`0x12102c0`)
  and emits `T - M * Q` — and each move is sampled at **its own phase**, so two moves with
  different cycles and different `step` vectors mix per bone with no phase alignment. The
  animation side takes the blend weight as given; how the sim produces it from
  `interpolationSpeed` is still open (first bullet).
