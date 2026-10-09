# FSM interpreters: native (CfgFSMs) and scripted (.fsm)

How Arma 3 2.22.0.154103 runs its two kinds of finite state machine, and how a unit's
`fsmFormation` / `fsmDanger` turn into running FSMs. Addresses are **RVA / VA** (image base
`0x140000000`) in `arma3_x64.exe`. Names in backticks such as `FSMEntity_Update` are Ghidra
names given in this pass (saved in the shared project; a few renames may still be pending).

Confidence: **H** = read directly from the code; **M** = read from the code but a callee or a
field meaning is inferred; **L** = inference. Inference is labelled as such.

Implementation: `crates/a3-fsm` (definitions, loader, the machine), `crates/a3-sqf/src/fsm.rs`
(mission FSMs in the scheduler) and `crates/a3-sqf/src/commands/fsm.rs` (the commands); what is
implemented and what is not is in section 7.

---

## 0. Class map (H)

| class (RTTI) | role |
|---|---|
| `FSM` (vt `0x1c59658`) | Base. +0x0c/+0x14 obfuscated `rpcorigin` (xor `0xd0867141`), +0x28 int `terminate`, +0x2c byte. Virtuals used below: +0x80 `Update(ctx) -> bool finished`, +0x88 thunk to +0x80, +0x90 `Init(ctx)`, +0xa8 `IsExclusive() -> bool`, +0xb8 variable table (scripted only), +0xd0 per-tick context hook. |
| `FSMEntity` (vt `0x1bd4fb0`) | Native FSM instance; its type comes from `CfgFSMs`. |
| `FSMEntityType` (vt `0x1bd4f90`) | Parsed `CfgFSMs` class, shared and cached by key. |
| `FSMEntityConditionFunc` / `FSMEntityActionFunc` | 0x18-byte function objects: vtable, member-function pointer (+8), this-adjust (+0x10). Concrete: `ConstMembFunc3<Entity/Man/SeaGullAuto, float, const FSMEntity*, const float*, int>` (conditions), `MembFunc3<..., void, FSMEntity*, const float*, int>` (actions). Slot +8 = call `(entity, fsm, params, nParams)`; slot +0x10 = "differs from member pointer X". |
| `FSMScripted` (vt `0x1acd650`) | Scripted FSM instance (`.fsm` file). |
| `FSMScriptedType` (vt `0x1acd618`) | Parsed `.fsm` file, shared through a bank at `DAT_1422211b0`. |
| `FSMScriptedMission` | `execFSM` instances (world list). |
| `AIBrainFSMScripted` (vt `0x1cb4390`) | Scripted FSM owned by an `AIUnit` (formation, danger, conversation). Overrides +0xd0: `_this` = the unit's person, set every tick. |

---

## 1. Native FSM (`FSMEntity`)

### 1.1 Data layout (H)

`FSMEntityType` (0x50 bytes; ctor `FSMEntityType_ctor` RVA `0xd8b580` / VA `0x140d8b580`):

| off | type | meaning |
|---|---|---|
| +0x10 | RString | CfgFSMs class name (cache key) |
| +0x18, +0x20, +0x28 | fn ptr x3 | key part: create-condition, create-action (enter), create-action (exit) lookup functions of the owning entity type |
| +0x30 / +0x38 / +0x40 | State* / int / int | states (0x70 bytes each), count, capacity |
| +0x48 | int | init state index |

State (0x70):

| off | meaning |
|---|---|
| +0x00 | RString class name (what `to` / `initState` / `finalStates` match, case-insensitive) |
| +0x08 | RString `name` value (display; FSMEntity vt +0x30 returns it) |
| +0x10 | Action `Init` (0x48 bytes, below) |
| +0x58 / +0x60 / +0x68 | Link* / count / capacity (links are 0x88 bytes) |

Action (0x48), loaded by `FSMEntityAction_Load` RVA `0xd8bd40`:

| off | meaning |
|---|---|
| +0x00 | ActionFunc* **enter** (from the create-action lookup) |
| +0x08 | ActionFunc* **exit** (from the create-action-exit lookup) |
| +0x10 | RString script text (only when `function` starts with `script:`; both function pointers are then null) |
| +0x18 / +0x20 | float* / int: `parameters[]` |
| +0x30 / +0x38 / +0x40 | `{int slot; float min; float max}`* (12 bytes each) / count / capacity: `thresholds[]` |

Condition (0x38), `FSMEntityCondition_Load` RVA `0xd8c110`:

| off | meaning |
|---|---|
| +0x00 | ConditionFunc* (null for `script:`) |
| +0x08 | bool **negated** (`1-` prefix) |
| +0x10 | RString script text |
| +0x18 / +0x20 | float* / int: `parameters[]` |
| +0x30 | int `threshold`: an **index into the instance's threshold slots** (config int; `< 0` gives error "Wrong threshold" and becomes 0) |

Link (0x88), `FSMEntityLink_Load` RVA `0xd8c9e0`: +0x00 float `priority`; +0x04 int target
state index (`to`; an unknown name gives `INT_MIN` = 0x80000000); +0x08 Condition; +0x40 Action.

`FSMEntity` instance (0x58 bytes; ctor `FSMEntity_ctor` RVA `0xd8b330`):

| off | meaning |
|---|---|
| +0x08 | int current state; ctor sets `type.initState`; -1 = finished |
| +0x30 | FSMEntityType* |
| +0x38 / +0x40 / +0x48 | float* / count / capacity: **threshold slots** |
| +0x50 | int time-out, in ms of the global ms clock `DAT_14225db08` |
| +0x54 | bool `initNeeded`; ctor sets 1 |
| +0x55 | byte, set by vt +0xc8 (not used on the paths below) |
| +0x56 | bool `isExclusive` (vt +0xd0 returns it; ctor 0). `AIUnit_Think` does not use it (see 3.4). |

Serialized keys (vt +0x10, RVA `0xd8d640`): `curState` (state class name), `timeOut`,
`initNeeded`, `isExclusive`, `thresholds`.

Timer virtuals (H): +0x68 `SetTimeOut(sec)`: `timeOut = now_ms + sec*1000`; +0x70
`GetTimeOut()`: `(timeOut - now_ms) * 0.001` (difference clamped to int32); +0x78
`TimedOut()`: `timeOut < now_ms`. The SeaGull functions `setTimer` / `timeElapsed` use them.

### 1.2 Loading a CfgFSMs class (H)

`FSMEntityType_Load` RVA `0xd8c390` reads `CfgFSMs >> name`:

1. `States`: each subclass is one state. Per state: `name` (string) and `Init` (an Action
   class: `function`, `parameters[]`, `thresholds[]`).
2. `initState` (class name) to index. Not found: error "Wrong init state", index 0.
3. `finalStates[]` (array of class names).
4. Second pass over the states. A state listed in `finalStates[]` gets its links **replaced**
   (`FSMEntityState_MakeFinal` RVA `0xd8dc30`) by one synthetic link: priority 1.0, `to = -1`,
   condition = Entity `true` (member fn `0x14018e9f0`, returns 1.0), threshold slot 0, not
   negated, action enter and exit = no-op. Its own `Links` are ignored. Other states load
   `Links` (each subclass: `priority` float, `Condition` class, `Action` class, `to` string)
   and then **sort** them (1.4).

Resolution of `function` (`FSMEntityAction_Load`, `FSMEntityCondition_Load`):

- Prefix `script:`, case-insensitive (`_strnicmp`, string at `PTR 0x1420b8920`): the whole
  string is stored as script text; the function pointers stay null.
- Condition, otherwise: `FSMEntity_ParseNegationPrefix` (RVA `0xd8de80`) splits the name at the
  **first alphabetic character**. The part before it, with spaces and `\t\n\v\f\r` removed, is
  compared with `"1-"`; equal sets `negated = true`. The name keeps only the part from the first
  letter on. Any other prefix (for example `"0-"`) is dropped silently with `negated = false`.
  Then `createCondition(name)`; null gives error "Wrong condition".
- Action, otherwise: `enter = createAction(name)`, `exit = createActionExit(name)`; if either is
  null: error "Wrong action".
- Unknown names do **not** return null. The Entity fallback lookups print
  `"%s: Unknown FSMEntity condition function: %s"` or `"... action function: %s"` and return
  `false` (0.0) or a no-op (section 2).

Number of threshold slots (ctor): `1 + max(index)` over every state's `Init.thresholds[].slot`,
every link's `Condition.threshold` and every link's `Action.thresholds[].slot` (zero slots if
nothing refers to one). **Every slot starts at 0.5** (`0x3f000000`).

### 1.3 Executing an action (H): `FSMEntityAction_Execute` RVA `0xd8d080`

```
for each {slot, min, max} in action.thresholds:
    fsm.thresholds[slot] = min + (max - min) * rng01()
if action.enter: action.enter(entity, fsm, params, nParams)
else:            entity.RunFSMScriptAction(scriptText)        // Entity vt +0xb90
```

`exit` is not called here. It is called (a) when a state is left (that state's `Init.exit`)
and (b) directly after a link's action (the link `Action.exit`); see 1.5.

Global RNG `DAT_142165668` (H): `seed = (seed * 0xC1C64E6D + 0x3039) & 0x7fffffff`;
`rng01() = seed * 2^-31` (`FUN_14030e310`); `rngAround(c, s) = c - s + s * seed * 2^-30`,
uniform in [c-s, c+s) (`FUN_14030e240`). All engine randomness shares this generator.

`script:` action (H; Entity vt +0xb90, RVA `0xe83dc0`): removes the prefix and executes the
rest as SQF in a new local scope with `_this = <the entity object>`, in the namespace at
`World+0x1840` (L: missionNamespace). `script:` condition (vt +0xb98, RVA `0xe83fe0`): same
setup; evaluates an expression and returns its **number** value (`GetNumber`). A string
without the prefix returns -FLT_MAX. **The `1-` negation is not applied to `script:`
conditions.**

### 1.4 Link order (H)

Comparator (code at VA `0x140d8bbc0`; not a Ghidra function):
`d = b.priority - a.priority; return d > 0 ? 1 : (d >= 0 ? 0 : -1)`, so **descending priority**.

`QSort_FSMEntityLinks` RVA `0xd8aa40` is the old MSVC CRT `qsort` algorithm (VC6 shape).
A range of 8 elements or fewer uses *shortsort*: a selection sort that scans `lo..hi`, keeps
the first element that compares strictly greater (the first **lowest-priority** link), swaps it
with `hi`, then `hi--`. A larger range swaps the middle element to `lo`, partitions with
`cmp(x, lo) <= 0` / `cmp(x, lo) >= 0`, and uses an explicit stack. The sort is **not stable**:
the order of equal-priority links is implementation-defined (two tied links in a short range
come out in reverse config order when nothing else moves them). To match tie order, port this
algorithm; with distinct priorities any sort gives the same result. The scripted loader uses the
same comparator and the same qsort template (`FUN_1413660a0`).

### 1.5 Per-tick update (H): `FSMEntity_Update` RVA `0xd8cd60` (vt +0x80)

```
Update(fsm, entity) -> finished:
  if cur == -1: return true
  if initNeeded: Execute(state[cur].Init); initNeeded = false     // enter of the init state
  for link in state[cur].links:                                  // sorted, descending priority
      if link.cond.func:
          v = link.cond.func(entity, fsm, params, n)
          if link.cond.negated: v = 1 - v
      else:
          v = entity.ScriptCondition(text)                       // Entity vt +0xb98
      if fsm.thresholds[link.cond.threshold] <= v:               // fires when slot <= value
          Execute(link.action)                                   // re-roll + enter
          link.action.exit?(entity, fsm, params, n)
          if link.to == INT_MIN: return false                    // bad target: action only
          if SetState(link.to): return true                      // reached -1
          goto chain
  return false
chain:   // follow-through in zero time
  first = cur; budget = number of states
  loop:
    s = state[cur]
    stop (return false) unless: s has exactly 1 link L, L's condition function IS Entity
         `true` (member pointer 0x14018e9f0), L is not negated, thresholds[L.threshold] <= 1.0
    Execute(L.action); L.action.exit?(...)
    if L.to == INT_MIN: return false
    if SetState(L.to): return true
    if L.to == first or --budget < 0: return false
```

`FSMEntity_SetState` RVA `0xd8cfe0`:
```
SetState(t): state[cur].Init.exit?(entity, fsm, Init.params)    // leave the old state
             cur = t
             if t == -1: return true
             Execute(state[t].Init)                              // re-roll + enter
             return false
```

Semantics (H):

- **Threshold rule.** A link fires when `thresholds[slot] <= conditionValue` (inclusive).
  With the default slot value 0.5, a 0.0/1.0 condition fires on 1.0. Slot 0 is just the first
  slot; it holds 0.5 until an action re-rolls it.
- **`thresholds[] = {{i, min, max}}`** in an `Init` or in a link `Action` sets slot `i` to a
  uniform value in [min, max) **each time that action executes** (state entry or link firing),
  before the enter function runs. Example, CfgFSMs `Formation`: state `Hide_or_Out_` sets slot 1
  in [0.2, 1.2); its link `Random` (`true`, threshold 1, priority 1) then fires with
  probability 0.8, otherwise link `Otherwise` (slot 0 = 0.5) fires. State `Init` sets slot 0 to
  exactly 0.5.
- **Action pair.** A function name gives (enter, exit). State `Init`: enter on entry, exit when
  the state is left. Link `Action`: enter, then exit immediately. Order on a transition:
  link enter, link exit, old state exit, new state threshold re-roll, new state enter. All Man
  formation functions have exit = no-op (`0x777060`); so do Entity `createFSM` / `deleteFSM`.
- **Transitions per tick.** One normal transition, then the zero-time chain through states
  whose *only* link is an un-negated `true` with slot <= 1.0. The chain is bounded by the number
  of states and stops when it comes back to the first state entered this tick. A state with
  any other single link (for example `coverReached`) waits for the next tick.
- **Final state.** Entering it runs its `Init` enter. Its synthetic `true` link then fires (in
  the chain of the same tick, because slot 0 <= 1.0 normally holds), calls the final state's
  exit, sets `cur = -1`, and `Update` returns `true`. After that `Update` returns `true` and does
  nothing.
- **Update rate.** No internal time step: it runs when its owner ticks it (3.4).

vt +0x90 `Init` (RVA `0xd8d350`): if `cur != -1`, `Execute(state[cur].Init)`; `initNeeded = false`.

---

## 2. Native function tables

Each entity type has three virtuals (+0xb0 / +0xb8 / +0xc0 on the *type* object; Man's are at
VA `0x141b39a68`) that return the three lookup functions. The lookups compare names
case-insensitively (`FUN_14128c730`, lower-case table) in the order listed; the last fallback is
the base class lookup.

### 2.1 Entity (base) (H)

Conditions, `Entity_CreateFSMConditionFunc` RVA `0xe7f340`:

| name | function | value |
|---|---|---|
| `true` | `0x14018e9f0` | 1.0 |
| `false` | `0x1401328d0` | 0.0 |
| `const` | `0x140e83be0` | `params[0]`; 0 when there are no params |
| `fsmFinished` | `0x140e83d80` | 1.0 if sub-FSM slot `int(params[0])` of the entity (`Entity+0x290` array, count `+0x298`) is missing (index >= count, or null), else 0.0; 0 when there are no params |
| other | `false` + warning | |

Actions, `Entity_CreateFSMActionFunc` RVA `0xe7efc0` (exit lookup RVA `0xe7f180`: all no-op):

| name | enter |
|---|---|
| `nothing` | no-op |
| `createFSM` `{slot, typeIdx}` (`0x140e83bf0`) | needs 2 params. If `typeIdx < entityType.fsmCount` (type +0x230): grow the `Entity+0x290` array to `slot+1`; if the slot is empty or holds an FSM of another type, put a new `FSMEntity` of `entityType.fsmTypes[typeIdx]` (type +0x228) there |
| `deleteFSM` (`0x140e83d10`) | removes a sub-FSM slot (body not decompiled; M) |
| other | no-op + warning |

### 2.2 SeaGullAuto (Dragonfly, Butterfly, HoneyBee) (H for names)

Conditions RVA `0xe3cfb0`: `moveCompleted` (`0x140e3db30`), `waitCompleted` (`0x140e3df70`),
`timeElapsed` (`0x140e3df50`), `moveCompletedVertical` (`0x140e3dba0`), then Entity.
Actions RVA `0xe3cb30`: `randomMove`, `randomMoveLand`, `stop`, `relativeMove`, `wait`,
`switchAction`, `setTimer` (`0x140e3de80`), `setNoBackwards`, `break`, `land`, then Entity.
The exit lookup RVA `0xe3cd70` gives no-op for all. Bodies not traced.

### 2.3 Man (formation FSM)

The table is H; the bodies are M/L. Every Man function is a thin wrapper:
`brain = Man+0xb38`, `unit = brain->vt+0x1e0()` (the AIUnit); without a unit a condition returns
0 and an action does nothing. They forward to `AIUnit::FSM*` methods. Notation:
`veh = unit.inVehicle (AIUnit+0xd0 link) if set, else unit.person (AIUnit+0xc8 link)`;
`behaviour = AIUnit::GetBehaviour` (`0x141347be0`): 1 CARELESS, 2 SAFE, 3 AWARE, 4 COMBAT,
5 STEALTH; CARELESS is never overridden by the group (M for names). Conditions return 1.0/0.0.

Conditions, `Man_CreateFSMConditionFunc` RVA `0x7760b0`, then Entity:

| name | wrapper -> AIUnit | meaning (confidence) |
|---|---|---|
| `behaviourCombat` | `0x776eb0` -> `0x1413432b0` | 1 if behaviour >= 4 (COMBAT/STEALTH) **and** the unit has a group (vt +0x1e8) with flag +0x324 clear **and** `FUN_14133a8b0` (unit's group slot permits; default true) **and** unit vt +0x1a0 is true (M) |
| `vehicleAir` | `0x777580` -> `0x141344c20` | 1 if `veh`'s type is-kind-of the world's air class (`World+0x1af8`) (M) |
| `vehicle` | `0x777510` -> `0x141344bd0` | 1 if `veh` is-kind-of type tag `0x1420ba620` (L: Transport, so "unit is in a vehicle") (M) |
| `formationIsLeader` | `0x777180` -> `0x141343cb0` | 1 if `AIUnit+0x234` is 3 or 5 (value 2 explicitly not); field meaning not traced (H code, L meaning) |
| `reloadNeeded` | `0x7773e0` -> `0x141344900` | 1 if `FUN_140fb40e0(veh, "")` >= 1.0 (a reload-need measure over the weapons) (M) |
| `coverReached` | `0x776f20` -> `0x141343320` | `veh->vt+0x14a0()`; Man impl `0x140713c20`: if cover use is **disabled** (disabled-AI bit 0x800, `COVER`) -> 1; else 1 only if `Man+0xf92 > 0`; sets `Man+0xf92 = -1` in both cases (read-once flag) (M) |
| `randomDelay` | `0x777310` -> `0x141344860` | 1 if `(now_ms - AIUnit+0xd618) * 0.001 >= AIUnit+0xd624` (H) |
| `formationCanLeaveCover` | `0x776f90` -> `0x141343370` | see below (M/L) |

Actions, `Man_CreateFSMActionFunc` RVA `0x775c30` (enter), then Entity. Exit lookup
`Man_CreateFSMActionExitFunc` RVA `0x775e70`: for every Man name exit = `0x140777060`, whose
AIUnit target is an empty function (identical-code-folded no-op).

| name | wrapper -> AIUnit | effect (confidence) |
|---|---|---|
| `formationInit` | `0x777060` -> no-op | nothing; only the state's `thresholds[]` re-roll matters (H) |
| `formationExcluded` | `0x7770c0` -> `0x141343bc0` | `person.SetUnitPosFSM(3 AUTO)` (Man vt +0x1c60, stores `Man+0x2294`); `veh+0xa14` = in vehicle: behaviour >= 4 ? 2 : 0, on foot: 0; `veh+0xaa0 = 1` (M) |
| `searchPath` `{a, b}` | `0x777450` -> `0x141344960` | see below (M) |
| `formationProvideCover` | `0x7772b0` -> `0x141344170` | see below (M) |
| `formationHideInCover` | `0x777120` -> `0x141343c70` | runs `formationNextTarget`, then `veh+0xa14 = 0` (H) |
| `formationLeader` | `0x7771f0` -> `0x141343ce0` | in vehicle: `veh+0xa14` = behaviour >= 4 ? 2 : 0; on foot: `person+0xa14 = 0` (H) |
| `formationNextTarget` | `0x777250` -> `0x141343d40` | see below (M) |
| `setUnitPosToDown` | `0x7774b0` -> `0x141344bb0` | `person.SetUnitPosFSM(2 DOWN)` (H; enum names L) |
| `reload` | `0x777380` -> `0x1413448c0` | `veh+0xa14 = 0`, then reload (`FUN_140fdfd10`) (M) |
| `formationCleanUp` | `0x777000` -> `0x141343a10` | see below (M) |

UnitPosFSM values used (names L): 1 MIDDLE, 2 DOWN, 3 AUTO (`unitPos` reports 3 for a
non-man). `EntityAI+0xa14` is an int state 0/1/2 that the FSM drives (0 on reload/hide, 2 while
covering/suppressing, 1 after clean-up); its consumer was not traced (L: a fire/cover stance).

AIUnit fields used by the formation functions (H for offsets and writers):

| off | type | written by | read by |
|---|---|---|---|
| +0xd4c8, +0xd4cc | float | searchPath (cover search distances) | path planner |
| +0xd5fc, +0xd608 | - | searchPath -> `FUN_14133d770` (cover query) | |
| +0xd614 | int ms | ProvideCover (= now) | CanLeaveCover (cover start) |
| +0xd618 | int ms | ProvideCover, NextTarget (= now) | randomDelay |
| +0xd61c | float s | ProvideCover: `3 + 5u` | CanLeaveCover (minimum cover time) |
| +0xd620 | float s | ProvideCover: `[+0xd61c] + 5 + 10u` | CanLeaveCover (maximum cover time) |
| +0xd624 | float s | ProvideCover: `(0.5 + 4u) * k + 0.5` | randomDelay |
| +0xd628 | ref | ProvideCover (suppression-target object) | NextTarget, CleanUp |
| +0xd630 .. +0xd66b | Vector3[5] | ProvideCover, NextTarget (aim points) | NextTarget |

`u = rng01()`. `k = FUN_141347a10(unit)`: 1.0 if the unit's group had an event in the last
30 s (`group+0x218 > now - 30 s`), else 0.5; then clamped to a per-unit range from unit
vt +0x198 (M).

**searchPath {a, b}** (M). No vehicle and no person: return. `a = params[0]`, `b = params[1]`
(0 when absent). Unit vt +0x168 gives the formation target (position, a mode int, a flag); if
mode > 2 and mode != 4: `a = b = 0`. `s = vehType->vt+0x1c8(behaviour)` (a per-behaviour
scale). `d = FUN_141347350(unit)` = signed distance of the unit from its formation position
along the subgroup direction. Scale factors:
`fa = d < 30 ? 1 : (d <= 300 ? 1 + (d - 30) * 0.04074074 : 12)` (1 to 12),
`fb = d < 30 ? 1 : (d <= 300 ? 1 + (d - 30) * 0.0037037036 : 2)` (1 to 2).
`unit+0xd4c8 = s*a*fa`, `unit+0xd4cc = s*b*fb`; runs the cover query (`FUN_14133d770`) and plans
the path (`FUN_14135ff90(unit, pos, mode, flag)`). Then: if `veh->vt+0x14c0()` is non-zero (Man
impl `0x140733840`: COVER enabled and a cover object at `Man+0xf88` gives 2, else `Man+0xf91`)
and behaviour >= 4: `veh+0xa14 = 2`; else if `veh+0xa14 == 2`: set it to 1.
So `{10, 5}` = look for cover (10 m / 5 m base radii, scaled); `{0, 0}` = path to the formation
slot without cover search.

**formationProvideCover** (M). `+0xd614 = +0xd618 = now`; minimum/maximum cover times as in the
table. On foot: if the person's current unit position (Man vt +0x1c48) is 2 (DOWN) or
`u <= 0.5`: SetUnitPosFSM(2), aim height 0.5, spread 4; else SetUnitPosFSM(1), height 1.5,
spread 6; aim distance 15 m. In a vehicle: distance 30, height 2.0, spread 10, no stance change.
Direction = the unit's own direction, or the direction from `FUN_141349750` of its leader
unit when that exists. Five aim points: `c = pos + dir*dist + up*h` and `c +/- perp*spread +/- up*h`
with `perp = (-dir.z, 0, dir.x)`. Creates a 0x58-byte suppression target object
(`FUN_1405d6b50(..., &c, ..., radius 4.0)`) at `c` and stores it in `+0xd628` (on foot also in
`unit+0x1b0`, and it becomes the unit's target if it has none; in a vehicle it is given to the
vehicle's commander unit). Sets the randomDelay time `+0xd624` and `veh+0xa14 = 2`. If the weapon
does not need a reload (measure < 1), the unit is on foot and the group has more than 1 unit:
`FUN_1412e0710` (group report; L: "covering" radio message).

**formationNextTarget** (M). `+0xd618 = now`. On foot: if the current unit position is 1 and
`u > 0.7`: SetUnitPosFSM(2) and recompute the five aim points with distance 15, height 0.5,
spread 4. If the unit has no target and `+0xd628` exists: target it. Pick
`i = clamp(round(5u), 0, 4)` and move the suppression target to aim point `i` (target
vt +0xd8, radius 4.0). `veh+0xa14 = 2`.

**formationCleanUp** (M). On foot: if `+0xd628` is the unit's current target, clear the target
(`FUN_140c25780(unit, null, 3)`); release `unit+0x1b0`. In a vehicle: clear the commander
unit's `+0x1b0` and target. Then `+0xd628 = null`, `person.SetUnitPosFSM(3 AUTO)`,
`veh+0xa14 = 1`, `unit+0x260 = 0`, `veh->vt+0x1498()`.

**formationCanLeaveCover** (M/L; RVA `0x1343370`; uses the local callback class
`AIUnit::FSMFormationCanLeaveCover::CheckBehind`, vt `0x1cb4ef8`). Returns 1 at once when the
unit has no subgroup. `F = vehType+0x155c` (a size/spacing constant of the unit's type);
`d` = signed distance ahead of the formation position (as above); `limit = veh->vt+0x1030()`.
In outline: when the unit is behind (`d < limit`) and is not its subgroup's leader, it first
checks a radius around its formation position (`2F`, or `10F` in a group mode `== 4`, when
`veh->vt+0x14c0() > 1`) and returns 0 inside it. Otherwise it walks the formation members with
the `CheckBehind` callback (nearest member behind/ahead; bounds start at +/-FLT_MAX), computes
`t = clamp(1 - d / max(2F + 2*behindDist, 8F), 0.1, 1)`, limited by `k` above, and returns 1
when the time in cover `(now - [+0xd614]) * 0.001 >= t * [+0xd61c]` under positional tests with
`4F` and `16F^2` radii, or after `3 * [+0xd620]` when the covering partner is not firing.
Until this is traced further, implement it as: "leave cover after a random 3-8 s scaled by `t`;
earlier when far behind the formation".

### 2.4 The CfgFSMs `Formation` graph (H; the merged game config, `CfgFSMs >> Formation`)

State: Init function {params} [thresholds] -> links as (priority, condition[, slot]) target.
Every condition has threshold slot 0 unless shown.

- `Init`: formationInit [{0,0.5,0.5}] -> (0, true) `Start`
- `Start`: nothing -> (2, vehicle) `Excluded`; (1, behaviourCombat) `Combat`; (0, true) `Excluded`
- `Combat`: nothing -> (1, formationIsLeader) `Leader`; (0, true) `Search_path__Covering`
- `Search_path__Covering`: searchPath {10,5} -> (4, coverReached) `Provide_cover__Out`; (1, true) `Start`
- `Provide_cover__Out`: formationProvideCover -> (3, vehicle) `Clean_up`; (3, 1-behaviourCombat) `Clean_up`; (2, reloadNeeded) `Drop_to_ground_1`; (1, formationCanLeaveCover) `Clean_up`; (0, randomDelay) `Hide_or_Out_`
- `Hide_or_Out_`: formationInit [{1,0.2,1.2}] -> (1, true, slot 1) `Next_target__Out`; (0, true) `Hide_in_cover__Hidden`
- `Next_target__Out`: formationNextTarget -> the same five links as `Provide_cover__Out`
- `Hide_in_cover__Hidden`: formationHideInCover -> the same five links
- `Clean_up`: formationCleanUp -> (0, true) `Start`
- `Leader`: formationLeader -> (0, true) `Search_path__No`
- `Search_path__No`: searchPath {0,0} -> (4, coverReached) `Provide_cover__Out`; (1, true) `Test_reload`
- `Test_reload`: nothing -> (3, reloadNeeded) `Drop_to_ground`; (1, true) `Start`
- `Drop_to_ground` / `Drop_to_ground_1`: setUnitPosToDown -> (0, true) `Reload` / `Reload__Hiden_`
- `Reload`: reload -> (1, true) `Start`; `Reload__Hiden_`: reload -> (0, true) `Provide_cover__Out`
- `Excluded`: formationExcluded -> (0, true) `Search_path__No_1`
- `Search_path__No_1`: searchPath {0,0} -> (4, coverReached) `Provide_cover__Out`; (1, true) `Start`
- `initState = "Init"`, `finalStates[] = {}`.

Consequences of the rules in 1.5 (inference, M): single-`true`-link states (`Init`, `Clean_up`,
`Leader`, `Excluded`, `Drop_to_ground*`, `Reload__Hiden_`) are passed through in the same tick.
A state with a `true` fallback (for example `Search_path__Covering`, two links) stays exactly one
tick: on the next tick either `coverReached` or the `true` fallback fires. So an unsheltered
unit in COMBAT cycles `Start -> Combat -> Search_path -> Start`, re-planning every few thinks,
until the read-once `coverReached` flag is seen.

---

## 3. Unit FSMs: `fsmFormation` / `fsmDanger`

### 3.1 Config keys (H)

The keys are read case-insensitively through two one-entry enum tables:
`{0, "FSMFormation"}` at `DAT_142220870` (static init RVA `0xe7820`) and `{0, "FSMDanger"}` at
`DAT_1422208a0` (static init RVA `0xea270`). The string `"-"` is interned once at
`DAT_142220968` (static init RVA `0xea140`). Config strings are interned, so the engine compares
the **pointer** (same result as exact string equality).

- `fsmDanger` is read when **ManType** loads (RVA `0x7d96d0`) into `ManType+0x1b68` (RString).
- `fsmFormation` is read on demand from the person's config class (`FUN_1405e0180(type)`) when
  an AIUnit is created (AIUnit ctor RVA `0x132f850`) and when it gets a new person
  (`AIUnit::SetPerson` RVA `0x135f040`).

### 3.2 Name to FSM (H): `AIUnit_CreateFSM` RVA `0x133e160`

```
CreateFSM(unit, entityType, name):
  if name == "": return null                                  // no FSM
  if CfgFSMs has class `name`:                                // native
      key = (name, type.createCondition, type.createAction, type.createActionExit)
      fsmType = FSMEntityType cache (DAT_1421d2600, FUN_140e8cfb0).findOrLoad(key)
      return new FSMEntity(fsmType)                           // 0x58 bytes
  fsmType = FSMScriptedType bank (DAT_1422211b0).get(name)    // load the .fsm file
  if fsmType: return new AIBrainFSMScripted(fsmType, namespace = World+0x1840)
  return null
```

The formation FSM is stored at **`AIUnit+0x160`**, the danger FSM at **`AIUnit+0x168`**, a
conversation FSM at `AIUnit+0x170` (3.4). A new person deletes and recreates the formation FSM.

`"-"` is special only for `fsmDanger` (3.3). As an `fsmFormation` value it would be looked up as
a file name and fail (no FSM). Values in the config dump: `fsmFormation`: `CAManBase = "Formation"`
(native), `Civilian_F = "A3\characters_f\scripts\formationC.fsm"`, other classes `""`.
`fsmDanger`: `CAManBase = "-"`, 49 classes `"A3\Modules_F_Tacops\Ambient\CivilianPresence\FSM\danger.fsm"`,
`Civilian_F = "A3\characters_f\scripts\formationCDanger.fsm"`, 57 classes `""` (Logic etc.).

### 3.3 Danger events to the danger FSM (H): `AIUnit_SetDanger` RVA `0x135dae0`

Signature: `(unit, int cause, const Vector3* pos, Target* causedBy, float until)`.

```
SetDanger(unit, cause, pos, by, until):
  if unit.person == World player person (World+0x2d88): return
  if until < 0: until = rngAround(5.0, 1.0)                 // uniform [4, 6) s
  if cause not in {5, 6}: unit+0x158 = now_ms + until*1000  // "in danger until"
  name = person.type.fsmDanger                               // ManType+0x1b68
  if unit.dangerFSM == null or (disabledAI & 0x40):          // 0x40 = "FSM"
      if name != "":
          if name == "-": return                             // danger handling fully off
          unit.dangerFSM = CreateFSM(unit, type, name)
          if the FSM has a variable table (scripted):
              _dangerCause    = cause                        // number
              _dangerPos      = [x, y, z]
              _dangerUntil    = now_ms*0.001 + until         // seconds
              _dangerCausedBy = by's object (objNull if none)
              _queue          = []
      // native fallback when there is no FSM (fsmDanger "" or the load failed)
      if name != "-" and dangerFSM == null and not (disabledAI & 0x40)
         and cause not in {5, 6} and behaviour in {SAFE 2, AWARE 3}:
          wasExpired = unit+0x154 < now_ms
          unit+0x154 = now_ms + until*1000
          if wasExpired: veh->vt+0x1810()                    // native danger reaction, not traced
  else:                                                       // the FSM is already running
      q = dangerFSM.vars["_queue"]
      if q is an array: q.push([cause, [x, y, z], now_ms*0.001 + until, by's object])
```

The variables are stored with lower-case names (`_dangercause` ...); SQF names are
case-insensitive. When the FSM already runs, the four `_danger*` variables are **not**
overwritten; the scripted FSM takes the best event out of `_queue` itself (`danger.fsm` state
`Reacting_on_danger`, priorities `[3,4,5,1,4,1,1,2,4,1]` indexed by cause).

**DangerCause** (values from engine call sites; names from `danger.fsm` comments). The value is
confirmed at a call site where marked "yes".

| value | name | confirmed | engine call site |
|---|---|---|---|
| 0 | DCEnemyDetected | yes | `FUN_1412dd340` (new enemy target; pos = target position; until -1) |
| 1 | DCFire | yes | `FUN_1405e2570`, `FUN_1405e2600` (fire seen; pos from the shooter) |
| 2 | DCHit | yes | `FUN_140fb6ca0` (3 calls) |
| 3 | DCEnemyNear | yes | `FUN_1405e4ca0` |
| 4 | DCExplosion | yes | `FUN_14121cd30` |
| 5 | DCDeadBodyGroup | yes | `FUN_1405e4770`: cause = `5 + (body's group != my group)` |
| 6 | DCDeadBody | yes | same |
| 7 | DCScream | no | event-struct path (`FUN_140900660` passes a stored cause) |
| 8 | DCCanFire | yes | `FUN_1407242b0` |
| 9 | DCBulletClose | no | event-struct path |
| 10 | (no name) | no | `AIUnit_OnDangerEvent` tests `cause - 9 < 2`, so 10 exists (L) |

`AIUnit_OnDangerEvent` RVA `0x1341fc0`; event struct `{int cause; Vector3 pos @+8;
Target* by @+0x18; bool report @+0x24}`. Skips the player, non-local units (vt +0x50), and
persons with flag `+0x5e4 & 1` or `FUN_141028db0() >= 1`. Adds suppression (`FUN_1405d45a0`).
A unit without a group: `SetDanger(cause, pos, by, rngAround(1.0, 0.2))` (0.8-1.2 s). With a
group: `SetDanger(..., -1)` unless group flag +0x324 is set. For causes 2, 3, 4, 9, 10 the
causer is told about the unit (`by->vt+0x210(unit, now)`). If the group is not in combat
(`FUN_1412d9cf0`) or cause == 3: a group report (`FUN_1412d59f0`) when `report` is set, and a
step of unit field +0x150.

### 3.4 Ticking (H): `AIUnit_Think` RVA `0x1361b60`

Called from the subgroup think `FUN_1413196a0`, which the group think `FUN_1412e6ca0` calls,
inside the AI phase of the world simulation: once per unit think.

```
exclusive = false
if unit is active (FUN_14134b200: person not destroyed (vt +0xb08) and
                   (local (vt +0x50) or person+0xc44 == 1)):
    if dangerFSM == null or (disabledAI & 0x40):
        if the conversation queue (+0x178 array, +0x180 count) is not empty and convFSM (+0x170) is null:
            start convFSM for the first queued sentence (FUN_141352f50; variables _from,
            _topic, _sentenceId, _<argName>...; .fsm from the speaker's config); pop the queue
        if convFSM:
            if convFSM.Update(unit): delete convFSM; --counter(+0xd554)
            else: exclusive = convFSM.IsExclusive()
    else:
        if dangerFSM.Update(unit): delete dangerFSM          // the next SetDanger recreates it
        else: exclusive = dangerFSM.IsExclusive()
... (other think work) ...
if not exclusive and formationFSM (+0x160) and not (disabledAI & 0x40):
    formationFSM.Update(unit)                                // result ignored
```

- **The danger FSM and the formation FSM both run in every think**; the danger FSM runs first.
  It pre-empts the formation FSM only when it reports *exclusive*. For a scripted FSM that is
  the FSM variable `_fsmExclusive` being `true` (`FSMScripted::IsExclusive` RVA `0x13671f0`).
  A native FSMEntity always reports false (vt +0xa8 = `return false`).
- While a danger FSM exists, the conversation FSM is neither started nor ticked.
- `disableAI "FSM"` = disabled-AI bit **0x40** at `AIUnit+0x138` (name table at `DAT_142220f80`;
  `"FSM"` -> 0x40) (H): no formation tick, no danger tick, no danger FSM creation. The
  conversation path still runs.
- A finished formation FSM is not deleted. Native: it stays finished (no-op). Scripted
  (inference, M): `FSMScripted_Update` sets `cur = -1` back to `initState` at the start of the
  next tick, so a scripted formation FSM that ends starts again from its init state, without
  running the init state's `init` again (`initNeeded` is already false).

---

## 4. Scripted FSM (`.fsm`)

### 4.1 File format the engine reads (H): `FSMScriptedType_Load` RVA `0x1368ea0`

The file is parsed as a config file (ParamFile). Root class **`FSM`**:

| key | required | read as |
|---|---|---|
| `FSM >> fsmName` | yes | string (type +0x40; debug messages) |
| `FSM >> States` | yes | class; each subclass is one state, in file order |
| `FSM >> initState` | yes | state class name; unknown: error "Wrong init state", index 0 |
| `FSM >> finalStates[]` | yes | array of state class names |
| state `name` | yes | string (state +0x08) |
| state `init` | yes | code string |
| state `precondition` | no | code string |
| state `itemno` | no | int, default -1 (editor id; debug only) |
| state `Links` | yes (non-final states) | class; each subclass is one link |
| link `priority` | yes | float |
| link `to` | yes | state class name; unknown: error "Incompatible game FSMs in between Save and later Load." |
| link `condition` | yes | code string; empty = always true |
| link `action` | yes | code string |
| link `precondition` | no | code string |
| link `itemno` | no | int, default -1 |

Comments, including the `/*%FSM<...>*/` editor markers, are config comments and disappear.
Every code string first goes through the engine's **StringPreprocessor** (`FUN_141367c90`:
macros and `#include` in the text are expanded), then it is compiled once at load
(`FUN_1402371d0`). A final state's links are replaced by one synthetic link (priority 1.0,
`to = -1`, empty condition, empty action; `FUN_141368ba0`). Links are sorted with the same
descending-priority qsort as in 1.4.

Layout: type +0x18 path, +0x20/+0x28 states (0x80 bytes each), +0x38 initState, +0x3c loaded
flag, +0x40 fsmName. State: +0x00 class name, +0x08 `name`, +0x10 init text, +0x18 precondition
text, +0x20 itemno, +0x28/+0x40 compiled init/precondition, +0x68/+0x70 links (0x88 bytes each).
Link: +0x00 priority, +0x04 target index, +0x08 condition, +0x10 action, +0x18 precondition,
+0x20 itemno, +0x28/+0x40/+0x58 compiled condition/action/precondition.

### 4.2 Instance (H): `FSMScripted_ctor` RVA `0x1366390`

+0x08 `cur` = -1 (not started), +0x30 type, +0x38 namespace, +0x48 **FSM variable table**,
+0x70 `initNeeded` = 1, +0x71 debug flag (`debugFSM`), +0x78 handle (mission FSMs only).

### 4.3 Step (H): `FSMScripted_Update` RVA `0x1367bf0` -> `FSMScripted_Step` RVA `0x1366c60`

```
Update(fsm, ctx) -> finished:
  if not type.loaded: return false
  if cur == -1: cur = type.initState
  push fsm.vars as the local-variable scope        // every code block below runs in it
  fsm.vt+0xd0(ctx)          // AIBrainFSMScripted: _this = the ctx unit's person, each tick
  r = Step(fsm)
  pop the scope; return r

Step(fsm):
  if cur == -1: return true
  if initNeeded: run state[cur].init; initNeeded = false
  run state[cur].precondition (if not empty)
  for link in state[cur].links:                    // descending priority
      run link.precondition (if not empty)
      if link.condition is empty or evaluates to true:
          run link.action
          next = link.to; break
  if no link fired: return false
  if next != cur: (debug log); cur = next
  if cur == -1: return true                        // the final state's synthetic link
  run state[cur].init                              // also when next == cur: a self-link re-runs init
  return false
```

- **Exactly one transition per update.** The conditions of the current state are evaluated in
  every update (no polling interval), in priority order; the first true condition wins.
  Scripted FSMs have no threshold slots.
- A state's `init` runs **on entry, in the same update as the transition**, directly after the
  link action. The FSM ends one update after it enters a final state: the final state's `init`
  runs on entry, its synthetic link fires in the next update, and `Update` returns true.
- Variables: all code runs with the FSM's own variable table pushed as the local scope, so
  local variables (`_x`) set in any init, action or condition persist across states and
  updates (H for the push; the persistence agrees with community documentation, M). `_this` is a
  variable in that table. `_fsmExclusive` (bool) is read by `IsExclusive`.
- Namespace of the code: `execFSM` uses the caller's current namespace (GameState+0x270); AI
  FSMs use `World+0x1840` (L: missionNamespace).
- `debugFSM` logging: `DebugFSM: %d,%d "%s" condition: %d "%s"` and `DebugFSM: %d,%d "%s" state: %d "%s"`.

### 4.4 Mission FSMs and SQF commands (H unless noted)

| command | handler RVA | behaviour |
|---|---|---|
| `execFSM file`, `arg execFSM file` | `0x48eda0`, `0x48ecf0` (array forms `0x48f020`, `0x48eea0`) | `SQF_execFSM_impl` RVA `0x48f1e0`: find or load the `FSMScriptedType` by path in the bank; create an `FSMScriptedMission` (0x80 bytes) in the caller's namespace; if `arg` is not nil: `_this = arg`; handle = `World+0x1aa8`, then the counter is incremented; `_thisFSM = handle`; copy rpcorigin; `terminate = 0`; append to the world's mission-FSM list (`World+0x1a90`, 16-byte entries `{FSM*, float waited}`, count `+0x1a98`). Returns the handle (number). Empty path: returns 0 (M). |
| `completedFSM h` | `0x48fa30` | `true` when no FSM with handle `round(h)` is in the world list (`FUN_141145e00`; handles <= 0 never match). Finished FSMs are removed, so this means "finished, or never existed". |
| `h getFSMVariable name` / `[name, default]` | `0x48f5d0` | reads the handle's variable table (not decompiled; M) |
| `h setFSMVariable [name, value]` | `0x48fb20` | writes into it (M) |
| `debugFSM` | `0x489a00` | sets the +0x71 debug flag (M) |
| `doFSM` / `commandFSM` | `0x569050` / `0x5682c0` | both call `FUN_14017c9e0`, with flag 1 (doFSM, silent) or 0 (commandFSM, radio): an AI Command of type FSM `[fsmName, position, target]` for the units (runs as `AISubgroupFSMScripted` / `AITeamMemberFSMScripted`; not traced, L) |

The first handle value is whatever `World+0x1aa8` holds at mission start (not traced; L:
positive, because 0 is the "invalid" handle).

**Scheduling** (`World_SimulateScheduledScripts` RVA `0x11789e0`; M). Mission FSMs share the
scheduled environment with `spawn`/`execVM` scripts and SQS scripts. Each frame: entries with
`terminate == 2` are removed; every other entry gets `waited += dt`; the entries are partially
sorted by `waited` (largest first, batches of up to 256) and run while the frame budget lasts.
The budget is `DAT_1420d072c` = **3 ms** in the normal frame (0.05 s on another call path);
the minimum slice is 0.5 ms. An FSM entry calls `Update` once (at most one transition), is
deleted when `Update` returns true, and its `waited` goes back to 0. Under load a mission FSM can
skip frames; it never steps twice in one frame.

---

## 5. Implementation checklist (inference from the above)

1. Native: per-instance `Vec<f32>` threshold slots, all 0.5; fire rule `slot <= value`; `1-`
   negation only for native functions; enter/exit pairs; the zero-time `true` chain; a final
   state = a synthetic `true` link to -1.
2. Scripted: one transition per tick; precondition hooks; `init` after the link action; a
   self-link re-runs `init`; a final state ends one tick later; an FSM-wide variable table;
   `_fsmExclusive`.
3. Unit: the formation FSM comes from `fsmFormation` when the unit is created (`""` = none,
   a CfgFSMs class = native, anything else = file). The danger FSM is created lazily from
   `fsmDanger` on the first danger event: `"-"` switches danger handling off completely, `""`
   falls back to a native SAFE/AWARE reaction. Tick the danger FSM first; skip the formation FSM
   when the danger FSM is exclusive; `disableAI "FSM"` stops both.
4. To match the order of equal-priority links, port the MSVC qsort shortsort/partition.

## 6. Open items

- Meaning of `AIUnit+0x234` (formationIsLeader), the consumer of `EntityAI+0xa14`, Man
  vt +0x1810 (native danger reaction), the exact formula of `FUN_141347350`, call sites of causes
  7/9/10, the `deleteFSM` body, `getFSMVariable`/`setFSMVariable`, and the command FSMs of
  `doFSM`/`commandFSM`.
- How often `AIUnit_Think` runs per unit (every frame or on a think interval).
- Whether `_dangerUntil` (`now_ms * 0.001 + until`) uses the same clock as SQF `time`.

## 7. What a3-rust implements

| item | where | status |
|---|---|---|
| `.fsm` file and `CfgFSMs` class loading (4.1, 1.2): states, links, `initState`, `finalStates[]`, `script:` functions, `1-` negation, thresholds, warnings for a bad init state / target / thresholds | `a3_fsm::Fsm::{parse_scripted, from_native_config}` | done; every shipped scripted FSM and every CfgFSMs class loads (real-data test) |
| `"a" \n "b"` string continuation the FSM Editor writes | `a3-config` text parser | done |
| link order: the MSVC qsort (1.4) | `a3_fsm::sort_links` | done, ported shortsort + partition |
| native step (1.5): slots start at 0.5, `slot <= value`, one transition, enter/exit order, zero-time `true` chain, final state = synthetic `true` link | `a3_fsm::Machine` | done |
| scripted step (4.3): precondition every step, `init` after the link action, self-link re-runs `init`, empty condition = true, a final state ends one step later | `a3_fsm::Machine` | done |
| mission FSMs (4.4): `execFSM` (4 forms), `completedFSM`, `getFSMVariable`, `setFSMVariable`, `diag_activeMissionFSMs`; one step per scheduler frame; FSM-wide local variables, `_this`, `_thisFSM` | `a3-sqf` | done. Not modelled: `allowTermination`/`terminate` on FSMs, `debugFSM`, the shared 3 ms budget (every FSM steps every frame), the caller's namespace (code runs in missionNamespace) |
| unit FSMs (3): `fsmFormation` / `fsmDanger`, `AIUnit_Think` order, danger events and `_queue`, `_fsmExclusive`, `disableAI "FSM"` | `a3-world` | follow-up (#242 second half) |
| native Man / Entity functions (2.1, 2.3) | `a3-world` | follow-up, with the formation movement |
| `doFSM` / `commandFSM` | | not done (the command FSMs are not traced) |
| the engine RNG (1.3) | | not used: the VM's own generator draws thresholds for mission FSMs |
