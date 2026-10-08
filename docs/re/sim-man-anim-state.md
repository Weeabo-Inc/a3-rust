# Man animation: move requests, the move queue and transition timing

How a Man's **Move state machine** is driven: what `playMove`, `playMoveNow`, `switchMove`,
`playAction`, `playActionNow` and the AI put where, and how the next move is started when the
current one reaches the end of its **Animation phase**. This closes the animation-request part
of §7 of `sim-man-movement.md`; that file's §1 (config model), §2 (transition graph, path
search) and §3 (weight blend) are assumed and not repeated here.

Source: `arma3_x64.exe` 2.22.0.154103. All addresses are RVAs from the image base
`0x140000000` (Ghidra `FUN_...` names where a function has no symbol; SQF handler RVAs come
from `docs/re/sqf-commands.tsv`). Terms follow `CONTEXT.md`: a **Move** is a config state, a
**Man** plays one Move at a time through his **Move state machine**, an **Animation phase**
runs 0..1.

**Status.**

- Two animation layers, the move record, the move queue and the request slots: **high**.
- The command table (handler → Man vtable slot → core) and what each command does to the
  state: **high**, except `playAction`'s name resolution (medium) and the FNV-1a special case
  in `playMove` (high that it exists, low what it is for).
- The step's transition rules (no edge / connect / interpolate), the phase wrap and the
  `minPlayTime` gate: **high** for the branches, **medium** for a few corner conditions.
- Stance and the action map: **medium**. How `upDegree` participates in picking a target move:
  **not traced**.

## 1. Two animation layers

A Man runs **two** instances of the same machinery, side by side, driven by the same step
function:

| layer | animation-state block | move record | what it plays |
|---|---|---|---|
| move | `Man+0x1348` | `*(Man+0xD0)+0xC8` | the movement moves (`AmovPercM...`) |
| action / gesture | `Man+0x1830` | `*(Man+0xD0)+0x118` | gestures and one-shot actions |

The moves object (the loaded `CfgMoves*` set) is shared: `*(Man+0x170)+0x1F88`. `Man+0x178` is
an override used in its place when non-null (`*(Man+0x178) ?: *(Man+0x170)`). The per-frame Man
animation update `0x140759190` calls the step `0x1406005d0` once per layer (move record, then
action record). Everything below applies to either layer unless an offset says "move".

Every layer's state block has the same shape (offsets relative to the block; add `0x1348` or
`0x1830` for the Man-relative address):

| offset | type | meaning |
|---|---|---|
| `+0x00` | `int32` | last resolved move id (fallback value read by `0x140601560`) |
| `+0x04` | `uint32` | stamped with the global time stamp `0x14225db08` when that id changes |
| `+0x08` | array | the **plan** (the route through the move graph, §5): same container as the queue, see §3 |
| `+0x10` | `int32` | plan length (element count) |
| `+0x428` / `+0x430` | `int32` / ptr | deferred **action** request: move id and refcounted descriptor |
| `+0x438` | `u8` | flag belonging to the action request |
| `+0x440` / `+0x448` | `int32` / ptr | outstanding **move** request: move id and refcounted descriptor |
| `+0x450` | list | the active-move weight list of `sim-man-movement.md` §3 |
| `+0x4E0` | `u8` | "move request satisfied" flag |
| `+0x4E1` | `u8` | "action request satisfied" flag |

A request pair is always `{int32 id; int32 pad; ptr desc}` (16 bytes); `desc` is a refcounted
descriptor (`+8` is the reference count, released through its own vtable slot `+8` at zero).

## 2. The Man, his visual state and the move record

`Man+0xD0` points at the Man's visual-state object. The two move records live inside it, 0x50
bytes apart: the move record at `+0xC8`, the action record at `+0x118`. One record holds two
blended slots, A (current) and B (previous):

| offset | type | meaning |
|---|---|---|
| `+0x00` | `int32` | slot A move id (`visual state +0xC8`; this is what `animationState` returns, §7) |
| `+0x08` | ptr | slot A descriptor (`visual state +0xD0`) |
| `+0x10` | `float` | slot A animation phase (`visual state +0xD8`) |
| `+0x14` | `float` | slot A phase delta of the last step (`visual state +0xDC`) |
| `+0x18` | ptr | slot A helper object, reset when the slot's id changes (medium) |
| `+0x20` | `int32` | slot B move id (`visual state +0xE8`) |
| `+0x28` | ptr | slot B descriptor |
| `+0x30` / `+0x34` | `float` | slot B phase / delta |
| `+0x38` | ptr | slot B helper object |
| `+0x40` | `u8` | cycle-ended flag (set when slot A's phase reaches 1) |
| `+0x44` | `float` | blend weight — slot A's share (`visual state +0x10C`) |
| `+0x48` | `float` | blend ramp accumulator (`visual state +0x110`) |

A record that is not blending holds weight = accumulator = 1.0 (`0x140609df0`, 15 bytes).
`0x14076bb80` (first call of the per-frame update) picks the dominant slot by the weight
(slot A's id at `+0xC8` while weight > 0.5, else slot B at `+0xE8`) and writes the animation
speed scale to `Man+0x244C`.

The Man also mirrors the current move: `Man+0x1338` = the move id last set, `Man+0x133C` = a
time stamp (the global stamp, or now + 1 s in the `switchMove` array form). `Man+0x1D18` is
written to −1 by every action/gesture install. The meanings of these three are **not traced**
beyond what is written to them.

The fields a request needs from the config (§1 of `sim-man-movement.md`) hang off the
per-move object (`moves+0x148 + id*8`, null when the id does not exist):

| offset | type | meaning |
|---|---|---|
| `+0x18` | ptr | the move's moves-type object (stance at `+0x40`, category id at `+0x48`) |
| `+0x38` | `float` | `minPlayTime` (used as a phase gate, §5) |
| `+0x3C` | `int32` | variant / equivalence id (compared against a target move id in §5) |
| `+0x40` | `u8` | 0 means the move may not be interrupted (checked before re-applying a hop) |
| `+0x44` | `int32` | start-phase mode for an interpolate hop: 0 = carry the blend, 1 = 0, 2 = 1 − phase |
| `+0x48` | `int32` | normalised id (`equivalentTo`; `−1` means "use the id itself"), `0x1406056f0` |
| `+0x50` | obj | the move's displacement / velocity info (used in the blend, §3 of `sim-man-movement.md`) |

The move object's own vtable slot `+0x20` is a **veto**: a non-zero answer refuses the request
(§6). What it tests is **not traced**.

## 3. The move queue (`Man+0x1DE0`)

One queue per Man, for the move layer only. It is a small-array container with 64 entries of
inline storage before it goes to the heap:

| offset (Man-relative) | type | meaning |
|---|---|---|
| `+0x1DE0` | ptr | data pointer (initially points at its own `+0x14`, i.e. the inline buffer) |
| `+0x1DE8` | `int32` | element count |
| `+0x1DF4` | array | inline storage: 0x400 bytes = 64 entries of 16 bytes `{u32 id; u32 pad; ptr desc at +8}` |
| `+0x21F4` | `u8` | "inline storage in use" flag |
| `+0x21F8` | `int32` | capacity in elements (initially 64) |

Operations: append (grow when `count >= capacity` through `0x140601850`, which resizes the
container to a target capacity — inline while it fits in 0x400 bytes, heap above; the size
argument is not rendered at the call site, so the exact growth factor is **medium
confidence**), erase front `0x1406013c0(container, 0, 1)`, clear `0x1406014c0` (releases every
entry's descriptor and drops the heap buffer).

An entry holds a **reference** to the move descriptor (taken on append, released on consume or
clear). Because the queue stores the descriptor, a queued move's config data cannot be freed
while it waits.

**Consumer** `0x140707d10(man)`, called from the per-frame controller `0x1407871c0` (and a few
other places; which of them is the per-frame path is medium — `0x1407871c0` is the obvious
one). It pops **at most one entry per call**, and only when the previous move request is done:

- the "satisfied" flag `Man+0x1828` (move state block `+0x4E0`) is set, **or** the request id
  `Man+0x1788` ((`+0x440`)) is −1, and
- the queue is not empty.

It takes the front entry `{id, desc}`, takes one more reference on `desc`, arms the move
request slot (§4) and erases the front entry. The controller then sees `Man+0x1788 != -1` and
hands the request to `0x140753b30` (what that does with it is **not traced**). So a move queued
behind another starts within one frame of the pending request being satisfied, and the queue
never holds more than one request in the slot.

## 4. Requests: arming and planning

**Arming the move request slot** — `0x140751100(stateBlock, {id, desc})`: writes `id` to
`+0x440`, reference-swaps `desc` into `+0x448`, clears the satisfied flag `+0x4E0`, then
consumes the caller's reference (the caller passes its reference in and gets `desc` zeroed).
Used by `playMoveNow` (§6), by the queue consumer, by the `switchMove` array form and by the AI
paths.

**The planner** — `0x140600f30(stateBlock, record, moves, outFlag, request, mode)` is "ask this
layer to play this move". It decides:

- The request is already the current move (slot A): re-set the pair and drop one element from
  the plan (report success).
- `mode == 1`: start it **now**, hard — slot B takes over slot A with its phase, slot A becomes
  the request, phases and blend reset. This is what every action/gesture install uses.
- The request is slot B with the blend not finished and no edge problem: reverse the blend
  (swap the slots, weight = easing of 1 − accumulator).
- Otherwise, if the plan's **last** element is not already the request, run the path search
  `0x140604c40` (`sim-man-movement.md` §2) from the current move to the request and write the
  route into the plan list. Success sets `outFlag`.
- The move object cannot be loaded, or no route at all: the function returns 0 with the flag
  clear — **the request fails and nothing is planned**. That is the "move with no path" case.

So the plan list holds a whole route of moves; the step consumes it one hop at a time (§5),
each hop being an edge of the transition graph with its own kind.

**The action layer's install** — `0x140712bc0(man, {id, desc}, mode)` calls the planner with
the action state block `Man+0x1830` and the action record `*(Man+0xD0)+0x118`. On success it
looks up the new move's config name (`0x140601590`) and fires engine event `0x49` with it, then
clears `Man+0x1D88`. The action commands call it with `mode 1` (immediate).

## 5. The step: how a planned move is started

`0x1406005d0(stateBlock, record, moves, dt, scale, outVelocity, outCycleEnded, flagA, flagB)`
is the per-frame step for one layer. What it does, in order:

1. **Apply the plan** — only in a frame after a cycle end has already been reported (the
   previous call's out flag is clear), when the plan is not empty, the plan's head is not
   slot B, and the blend into B is essentially finished (weight ≥ ~1). The hop's kind is the
   **direct edge** from slot A to the head (`0x140604be0` over the graph's per-state edge
   lists; the static "no edge" record has kind 0):
   - **no edge (kind 0)** — snap: slot A := head, slot B cleared to −1, phases zeroed, blend
     reset. This is the fallback when the plan head has no direct edge from the current move
     (e.g. the record was reset underneath a pending plan). Reason **medium**.
   - **connect (kind 1)** — wait for the cycle end of the current move (the `record+0x40` byte),
     then slot A := head, slot B cleared, phase and delta zeroed, blend reset.
   - **interpolate (kind 2)** — blend into the head: the old slot A moves to B keeping its
     phase, the head becomes A with its start phase from the move object's `+0x44` (0 = carry
     the blend over by converting the old rate·phase into the new move's phase; 1 = start at 0;
     2 = start at 1 − old phase), blend weight and accumulator reset to 0.
     This hop is **deferred** while the current move's phase is below the move object's
     `minPlayTime` (`+0x38`), unless one of three exceptions holds: the current move is looped
     (its RTM info object `+0x108 == 1`), the edge's flag byte is set, or the current move
     object's `+0x3C` (variant id) equals the head's id.
   - Every applied hop is consumed: the head entry's descriptor reference is released and the
     remaining entries shift down.
   - Head == slot A: the pair is re-set and one entry is consumed. Head == slot B: only
     re-applied when the current move's object is missing or its `+0x40` is 0 (the corner case
     from §2; purpose **medium/low**).
2. **Advance the phase** — slot A's phase grows by `rate · dt · scale` (`rate` from
   `sim-man-movement.md` §1/§3, `scale` from `Man+0x244C`). At ≥ 1 the cycle-ended byte
   (`record+0x40`) and the caller's out flag are raised. A looped move wraps the phase
   (subtracting 1, not below 0.5); a non-looped one is clamped at 1. An absurd rate
   (> 100000) zeroes the phase instead. The delta is stored in `record+0x14`. Slot B keeps
   advancing while the blend is not finished.
3. **Velocity / blend** — this is `sim-man-movement.md` §3: the two slots' contributions are
   weighted by the blend weight and 1 − weight, plus the move object's displacement object
   (`+0x50`, `0x1405ff2d0`), into the caller's accumulator; the returned local velocity is
   sign-flipped. The weight ramps: accumulator += `0x14060b170(...) · dt`, weight = eased
   accumulator (`0x140e11880`); at accumulator 1 the record is reset to weight = accumulator =
   1 (`0x140609df0`) and, for a "now" install, slot B is dropped. In the weight-list mode the
   active-move weight list (`+0x450`) is updated through `0x140602d60` with the plan tail.
4. **Request bookkeeping** — the action request (`+0x428`) is retired when the current move's
   normalised id (`0x1406056f0`) matches it and the flag `+0x438` is clear (clearing `+0x4E1`);
   on a cycle end, if the outstanding move request (`+0x440`) matches the current move by
   normalised id, the satisfied flag `+0x4E0` is raised and the request slot is cleared once
   satisfied. The finished move's descriptor is handed back to the caller and slot A's
   descriptor pointer cleared.

`minPlayTime` therefore gates the **interpolate** hop: it is the fraction of the current move's
cycle that must have played before a blend to the next move may start. `sim-man-movement.md` §2
says "connect waits for the current cycle, respecting minPlayTime"; in the step, connect waits
for the cycle-end flag while the explicit `minPlayTime` comparison guards the interpolate hop —
the cycle-end flag is the stricter condition of the two (medium).

## 6. The script commands

Common front end of all six commands: the object comes from the argument, the class is checked
against a class descriptor, a string is taken from the argument GameValue (empty string
fallback `0x14216ac08`), and one Man virtual slot is called. The four `play*` cores share a
prelude: a controller notification (`Man+0x2548` → controller object; with no descriptor the
Man's virtual slot `0x1E20` is called, otherwise `0x140793030` for descriptors whose kind is not
2/3), then the veto check on the move object's vtable slot `+0x20`. An unknown name interns to
id −1 and the command returns 0 (failing; the descriptor is released).

| SQF command | handler | Man vtable slot → core | effect on the state |
|---|---|---|---|
| `playMove` | `0x140538c30` | `0x1058` → `0x14073ed30` | **append** `{id, desc}` to the move queue (`Man+0x1DE0`), return 1 |
| `playMoveNow` | `0x140538dc0` | `0x1060` → `0x14073efd0` | intern the name, then `0x14073eec0` (below) |
| `playAction` | `0x1405389c0` | `0x1078` → `0x14073e940` | resolve the name (`0x140729bb0`), then the shared dispatcher `0x14073eb40` |
| `playActionNow` | `0x140538a90` | `0x1080` → `0x14073e9e0` | resolve the name, then `0x14073eec0` |
| `switchMove` (string) | `0x5420b0` | — (direct) | `0x140765040`: immediate record reset, §6.3 |
| `switchMove` (array) | `0x5423e0` | — (direct) | `0x140765380`: reset + explicit phase/blend, §6.3 |

The Man primary vtable is at `0x141b36168`; slots `0x1058`–`0x1080` were read from the slot
array at `0x141b371c0`.

### 6.1 The dispatchers

- `0x14073eb40(man, {id, kind}, desc, flag)` — the shared dispatcher:
  - **kind 0 (a move)**: the same controller notification and veto as `playMove`, then the same
    queue append as `playMove`. This is what `playAction` does when its name resolves to a move.
  - **kind ≠ 0 (an action/gesture)**: with no moves object it prints `No gestures available` and
    fails; otherwise install on the action layer through `0x140712bc0` with `mode 1`
    (immediate) and set `Man+0x1D18 = −1`.
  - id −1 with nothing to do: releases the descriptor and returns 0.
- `0x14073eec0(man, {id, kind}, desc, flag)` — the "now" dispatcher:
  - **kind 0 with a valid id**: controller notification, veto, **clear the whole move queue**
    (`0x1406014c0`), then arm the move request slot (`0x140751100`). This is `playMoveNow`.
  - otherwise: the shared dispatcher above (so a gesture goes to the same immediate install).
- `0x140712bc0` — the action-layer install (§4). Its `mode 1` means "start now, no path search".

`playAction`'s name resolution `0x140729bb0(man, name)` works from the **current move's**
moves-type object (through the action map at `moves+0x18`, falling back to the move-name table
at `moves+0x20`) and returns the request the dispatcher should act on. The decompiler renders
its argument passing poorly, so the internals are **medium confidence**; the split of the two
commands (shared dispatcher vs "now" dispatcher) is high.

Consequence: `playAction`/`playActionNow` differ in exactly the same way as
`playMove`/`playMoveNow` when the name resolves to a **move** (queue vs clear-and-start-now);
when it resolves to a **gesture**, both reach the same immediate install (mode 1). Whether the
engine queues gestures anywhere else is **not traced** (medium that it does not).

### 6.2 `playMove`'s FNV special case

The `playMove` handler (`0x140538c30`, not the core) hashes the name with FNV-1a-64. When the
hash equals `0x23a65aeaa9132142` it does not touch the Move state machine at all: it looks up the
singleton object at `0x1421a0380`, asks its virtual slot, and returns the integer 1 to the
script when that value is > 3. The name that hashes to that value is **not identified** (no
xrefs to the singleton; a brute force over 67 candidate state/property names found nothing), so
the purpose is **low confidence** — it looks like a special-cased "instant" move name handled by
a different subsystem.

### 6.3 `switchMove`

**String form** (`0x140765040`): release the current move's handle (`0x140743030(man, handle,
1.0)` + `0x140742d50`); an id of −1 (any name that does not resolve, including `""`) is
replaced by the **default state for the current action context** — `0x140725240(man)` gives the
context key and `*(u32*)(moves+0x244)` the default index, looked up in the action/state map at
`moves+0x18` (`0x140605720`). Then the whole record is reset (`0x140602ab0`: both slots to the
new move, phases and deltas zeroed, the plan cleared, the blend reset, the cycle-ended byte
cleared), `Man+0x1338`/`Man+0x133C` are set, the **move queue is cleared** and the move request
slot emptied (id −1, descriptor released, satisfied flag cleared), a large aim/weapon block is
zeroed, and the change is pushed through `0x1407455d0`, `0x1407473f0`, two Man virtual calls and
`0x140713cc0`. A request whose descriptor kind is not 0 prints `Not implemented`.

So `switchMove` never queues and never blends through the graph: it replaces the record's
contents on the spot (which is why `switchMove` is the "snap" command).

**Array form** (`0x140765380`), `[move, time, blendFactor, resetAim]`: the same release and
default-state substitution, the record reset through the sibling `0x140602c30` (keeps no old
descriptor), and then the three numbers are written straight into the move record — `time` into
slot A's phase (`record+0x10`, visual state `+0xD8`) with the delta zeroed, and `blendFactor`
into **both** the blend weight (`record+0x44`, visual state `+0x10C`) and the accumulator
(`record+0x48`, `+0x110`). `Man+0x1338`/`Man+0x133C` are set (the stamp is now + 1 s when the
move object exists), the queue is cleared, the freshly reset request is armed into the slot
(`0x140751100`), and when `resetAim` is non-zero a large aim/turn/weapon block (`Man+0xDB0`..
`0xF20` and visual state `+0x374`..`+0x3A0` among others) is zeroed plus `0x14074c550(man)`.
Then the same finishing calls, a Man virtual (`+0x50`) and bit 2 of `Man+0x1CD` set. Field
mapping: **high** (the written offsets are the known record offsets); "time is the Animation
phase, not seconds" is **medium** (it lands in the phase field).

## 7. `animationState`

Handler `0x14056a520` (unary): after the Man class check it calls `0x14072ad70(man)` and then
looks up the move id in the moves name table (`0x1406057e0`) — the id is slot A of the move
record, `*(int*)(*(Man+0xD0)+0xC8)`. The result is the **config state class name of the current
move** (`CfgMoves*` `States` class, e.g. `AmovPercMstpSrasWrflDnon`), in the spelling the
config uses — the engine returns the interned name unchanged. Whether the intern lookup itself
is case-insensitive is **not traced** (medium: the engine's other name tables are
case-insensitive). With id −1 the helper returns the static empty string GameValue
`0x14216db00`. A different class (descriptor `0x1420b7a98`) goes to `0x140d39cc0` (the moves
object of `*(Man+0x178) ?: *(Man+0x170)` — used by non-Man entities); anything else returns an
empty value.

`switchMove ""` does **not** empty it: `""` resolves to id −1, which the command replaces by
the action map's default state (§6.3), so `animationState` afterwards returns that default
state's name. High confidence, from the handler + command paths.

## 8. Stance and the action map

- **Stance**: `0x14060a3c0(record, moves)` returns the moves-type object's `+0x40` for the
  record's current move. The `stance` command (handler `0x140532d40`) maps that value through a
  runtime string table at `0x142220e70` (24-byte entries, string pointer at `+8`) to a string.
  The table is zero-filled in the file (built at runtime), so the strings themselves were not
  read (medium). `unitPos` / `setUnitPos` (handlers `0x533390` / `0x53fc20`) were not traced.
- **Action map**: the moves object holds the action/state map at `+0x18`, the action-name table
  at `+0x20`, the move-name table at `+0x30` (id array at `+0x48`), and the default-action index
  at `+0x244`. `0x140725240(man)` computes the current **action context key** from the
  inventory/weapon state (a small enum; values 8, 9, 10 and 17 were observed in the code paths);
  `switchMove`'s default-state substitution and `0x140601560`'s fallback use it. What the keys
  mean is **not traced**.
- **`upDegree`**: the strings `upDegree` (`0x141b20b20`), `DefaultUpDegree` (`0x141b20de0`) and
  `UpDegreeChangeTime` (`0x141b20df0`) exist and the moves-type level is loaded
  (`sim-man-movement.md` §1), but **no reader was followed to a target-move decision** — how the
  value picks the target move is not traced.
- What is known about the move selection keys: moves-type `+0x48` is a category/selector id
  (`0x140601560`, falling back to the state block's `+0x00`) and `0x1406056f0` gives the
  normalised id (move object `+0x48` unless −1) that the request bookkeeping compares.

## 9. Not traced / low confidence

- The FNV-1a-64 name and the singleton `0x1421a0380` behind `playMove`'s special case (§6.2).
- What the move object's veto vtable slot `+0x20` tests.
- The class descriptors `0x14209dadc`, `0x1420ba9a0`, `0x1420b7a98` used by the command
  handlers (only their role as class checks is known).
- The meaning of the global time stamp `0x14225db08` and of `Man+0x1338` / `Man+0x133C` /
  `Man+0x1D18` / `Man+0x1D88`.
- `0x140753b30` — what the controller does with the armed move request.
- The record pair's `+0x18` / `+0x38` helper objects.
- The queue container's growth factor (its resize function `0x140601850` is understood; the
  size argument is not rendered at the call sites) and the exact purpose of the two satisfaction
  flags `+0x4E0` / `+0x4E1`.
- The head == B re-apply corner in the step (§5).
- Whether a *queued* (as opposed to "now") gesture exists: `playAction`'s action branch installs
  immediately, like `playActionNow`.
- `0x14060ab90` (an alternative blend seed) and the name helpers' case sensitivity.

## 10. Answers per question

| question | where | confidence |
|---|---|---|
| how the five `play*` commands are registered and what each does to the state | handler RVAs + Man slots `0x1058`/`0x1060`/`0x1078`/`0x1080`, §6 | high (medium for `playAction`'s name resolution) |
| where the queue, current move, phase, previous move and blend live; where the action request lives | §2, §3, §4 | high |
| what differs between `playMove` / `playMoveNow` / `switchMove`; the array form | §6 | high (medium that gestures are never queued) |
| transition timing: how the next move starts, `minPlayTime`, connect vs interpolate, no path | §5, §4 ("the planner") | high for the branches, medium for the corner conditions |
| what `animationState` returns; `switchMove ""` | §7 | high |
| stance storage and the action map's `stance` / `upDegree` use | §8 | medium; `upDegree` not traced |
