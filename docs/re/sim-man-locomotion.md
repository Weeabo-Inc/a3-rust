# Man locomotion runtime: the per-step movement of a walking Man

How `arma3_x64.exe` 2.22.0.154103 turns the moves state machine of
[`sim-man-movement.md`](sim-man-movement.md) into world displacement: the call chain of one
simulation step, the wanted speed, the ground snap, the slope block, the air flag, and which
frame the displacement is expressed in.

**Address convention.** All addresses are **VAs** with image base `0x140000000` (RVA = address −
0x140000000). Function names are Ghidra's `FUN_…` unless marked otherwise. Field offsets are into
the `Man` object unless written `vis+…`, which means the `ManVisualState` at **Man+0xd0**
(`Man+0x1a` as a pointer index); its layout and vtable (`0x141b35d28`) are in
[`sim-man-movement.md`](sim-man-movement.md) and §6 below.

**Status / confidence.**

| Part | Confidence |
|---|---|
| Per-step call chain and its once-per-frame gates (§1) | high (caller/callee lists + asm) |
| The world displacement formula and its basis (§6) | high (asm) |
| Ground snap, step-down, the 0.2 m clearance, the blocked case (§3) | high for the code, medium for the wording |
| Air flag `Man+0x12a2` and its two thresholds (§4) | medium |
| Turn command accumulator and its 6·dt clamp (§2) | high for the code, medium for units/semantics |
| Gravity constant, `CfgSlopeLimits` (`settings+0x778`) during a step, leaning, evasive moves | **not traced** (§2, §4, §5) |

## 1. What runs each simulation step, in order (high)

```
Soldier vtable slot 374                   per-step "move" entry
└── FUN_14078c930                          trampoline, single caller of FUN_140756310
    └── FUN_140756310                      the Man mover, ~11.9 KB               (single caller: the trampoline)
        ├── FUN_140717e40   @0x140757604   on-ground step (single caller: the mover)
        │   ├── FUN_140759190              wanted move / speed range
        │   │   └── FUN_1406005d0          state & phase advance
        │   │       └── FUN_140602d60      the velocity blend of sim-man-movement.md §3
        │   ├── FUN_14070c6d0              post-step resolve (medium: collision internals)
        │   ├── FUN_140e78050              pose/event step, returns a bool
        │   └── FUN_140754900              replay of a searched move
        └── FUN_14075bc10                  alternate (not-on-ground) step, ~29 KB
```

- The trampoline `FUN_14078c930` really receives the `Man` in RCX and `dt` in XMM1; Ghidra prints
  the inner call with no arguments (`FUN_140756310()`), which is a decompiler artefact.
- `FUN_140759190` is reached both from the mover and from inside the on-ground step
  (`FUN_140717e40` calls it once); so state selection and the phase advance happen **after** this
  step's displacement. The model-space velocity produced by the blend therefore feeds the state /
  pose and the following step, not the displacement the same step already wrote.
- The displacement itself uses a **control** value, not the animation step: see §6.

### The per-step driver (once-per-frame gates)

`FUN_14078c9d0` (no direct callers → invoked through a vtable slot) is the per-step dispatch:

| branch | call | note |
|---|---|---|
| ragdoll | – | separate ragdoll path |
| local player | `FUN_1407871c0` | gated on `Man+0x7c0 != DAT_142208a10` (a frame counter): runs **once per frame**, not once per simulation step |
| AI | `FUN_140783010` | reads `Man+0x12b8` |
| dead / clearing | – | |
| view | `FUN_14078d950` | calls virtual `+0x1440` → `FUN_140f8c3b0` → `FUN_140e3a8a0(Man+0xddc, dt, …)` and `FUN_140e3a490(Man+0xe58, dt)` — head/view tracker, **not** the body turn |
| tail | `FUN_140e95680`, `FUN_140ff4690` | |

The player path `FUN_1407871c0` (≈970 lines) is the input handler; it writes the wanted forward
speed (below). Keys/axes are converted by `FUN_140780e20(ctrl, &mode, &fwd, &lat)`, which builds
two analog axes out of input indices `0xc/6/0xd/7/9/8` (lateral = `f6 − f7`;
forward = `2·a + b + 0.5·c − d`), clamps to ±0.5 in walk mode, and shifts a gear
(walk → run → sprint) with a 1.5 threshold and a snap to −1/0/2 — i.e. **walk/run/sprint is
selected by the input layer**, before any slope question (§5).

### Flags on `Man` used by the step

| offset | meaning (confidence) |
|---|---|
| `+0x12b8` | wanted forward speed, m/s-ish; written once per frame by the player input handler, read by the AI handler; the step multiplies it by `dt` for the forward displacement (high for the use, medium for the unit) |
| `+0x12a5` | "this Man's motion is simulated here" flag; read six times in the on-ground step: on the no-work path a 0 jumps straight to the tail, skipping the delta, the position write *and* the post-step resolve `FUN_14070c6d0`; on the displacement path a 0 discards the **vertical** component and disables the ground rules and the state-11…16 block (§3, §5); in the tail it forces the searched-move replay `FUN_140754900` and gates a vis-side virtual call (`vis` vtable `+0x68`) (high for the reads; whether the horizontal delta is still written when it is 0 was **not established** — the caller may keep non-simulated men out of the mover entirely) |
| `+0x12a0` | move-blocked flag; when 0 and `+0x12a5 != 0`, the forward step length is forced to 0 (high for the code) |
| `+0x12a2` | airborne flag, written by the on-ground step, read by the AI handler (high) |
| `+0x1268`, `+0x1340` | choose the step variant: `FUN_140717e40` when `+0x1268 == 0 \|\| +0x1340 != 0`, else `FUN_14075bc10` (high for the condition; the names are unknown) |
| `+0x129c`, `+0x12a3`, `+0x12bc`, `+0x12bd` | slope-block state: last ground height ahead, its "initialised" flag, and two mode flags (§5) |
| `+0xf5c`, `+0x1eb`, `+0x1ec` | wanted turn, turn command accumulator, previous command (§2) |
| `+0x184`, `+0x1b0` | written by `FUN_140e94cf0(Man, x)` with x = 0.2 (`FUN_140e94b30` writes 0.2 or 0.06 nearby); medium: capsule / clearance radii |

## 2. Turning (code high, units medium)

- **Wanted turn.** The input layer produces the raw turn from mouse/keys; `FUN_14078b410`
  scales the axis rate by the globals `DAT_142208ba0` / `DAT_142208ba4` (cubed, ×0.1 + 0.2) —
  the mouse-to-turn curve (medium). `FUN_140788c50` post-processes the input and drives a spring/
  damper integrator (`FUN_140795ac0`, `FUN_140795cd0`) in the **control object at `Man+0x1138`**,
  whose `+0x21c` field is advanced by `FUN_1407818e0` as `clamp(f + (2·flag − 1)·5·dt, 0, 1)` —
  a stance/lean-style ramp, not the yaw itself (medium).
- **The mover's turn block (high for the code).** In `FUN_140756310`, when the input predicate
  `cVar13` (computed earlier in the function, not identified) is set:

  ```
  limit = 6.0 · dt
  Δ     = clamp((Man+0xf5c − Man+0x1eb), −limit, +limit)
  Man+0x1eb += Δ
  ```

  The result is written to `Man+0x1eb`, applied by `FUN_1407473f0` (which reads `Man+0x1eb` and
  also re-applies it in a short ±1/0 sequence before the real value), and the change is announced
  by `FUN_140fc9e30(Man, 0x4d, new, old)` with `Man+0x1ec = new`. So **the body turn is a
  rate-limited follower of the wanted turn, limited to 6·dt per step**. Whether `Man+0x1eb` is
  radians (→ 6 rad/s ≈ 344°/s) or a normalised command is **not established**; treat the unit as
  open (medium).
- **`turnSpeed`** (moves-type field at type `+0x4c`, sim-man-movement.md §1) is the per-move
  limit that the engine is documented to use. I did **not** find a read of the moves-type
  `turnSpeed` inside the step chain (`FUN_140756310`, `FUN_140717e40`, `FUN_140759190`,
  `FUN_1406005d0`, `FUN_140602d60`); where it clamps the turn — **not traced**.
- **Standing-still instant turn**: whether the body snaps to the wish direction at zero speed is
  **not traced**. The turn block above runs only when the input predicate `cVar13` is set, and it
  is rate limited even then, which argues against an instant snap (low confidence).
- **`EvasiveLeft` / `EvasiveRight`**: these are `ActionMap` action names (config), i.e. ordinary
  moves states; nothing in the locomotion step treats them specially — they enter as a target
  state and are played by the same blend. The turn-rate limiting of the reachable states is their
  moves-type `turnSpeed` (reasoning from the config model, **not traced in code**).
- **Leaning**: the moves-type carries `leanLRot` / `leanRRot` / `leanLShift` / `leanRShift`
  (sim-man-movement.md §1) and the mover tests `Man+0x1340` when choosing the step variant, but
  the per-step code traced here has no lean branch and the lean angle is **not traced**.

## 3. Ground following (high for the code)

Three world queries are used by the on-ground step:

| function | arguments | returns |
|---|---|---|
| `FUN_1416527b0(world, &vis+0x2c, 0…)` | world = `DAT_142237f20`, the Man's position | terrain/roadway height under the Man |
| `FUN_141655d30(world, x, z)` | two floats in XMM1/XMM2 (a horizontal position) | surface height at that horizontal position |
| `FUN_141659820(world, &point)` | a world point | a height used for the slope ramp, the step-down and the air test |

`FUN_140717e40` builds a **probe point** ahead of the Man, computes the world delta, then corrects
only its vertical component `dy`:

1. `terrain = FUN_1416527b0(world, &pos)`; if `pos.y + dy < terrain` then `dy += terrain − (pos.y + dy)` — never leave the Man below the ground.
2. `surf = FUN_141655d30(world, probe.x, probe.z)`; if the probe's clearance above `surf` is less
   than **0.2 m** (including negative — the probe below the surface), raise the Man so the probe
   point sits exactly 0.2 m above the surface (constant `0x141a9bb30` = 0.2).
3. **Step-down**: if the height `g = FUN_141659820(world, &probe)` is more than 0.1 m below
   `probe.y + dy` (constant `0x141a792a4` = 0.1), lower the Man so that `probe.y + dy = g + 0.1`.
   The correction is clamped to ≤ 0, so this rule only ever moves the Man **down** — within one
   step he settles at 0.1 m above the surface ahead.
4. **Blocked**: if rule 2 fired *and* the Man was not below the terrain, the **horizontal delta is
   zeroed for this step**. A rise ahead that the body would have to climb more than the clearance
   does not push the Man up and does not slide him sideways — the step is simply refused. What
   turns that into a state (a step-up, a fall or a stop) is the mover's obstacle/ledge search
   (§5) and the moves graph, not this function.

**Two further facts about this block.** First, the rules above and the air flag of §4 live inside
the same gate as the slope block of §5: they run only when the state index is 11…16 **and**
`Man+0x12a5 != 0` (simulated on this machine) — the state gate covers the ground snap, not only
the slope damping. Second, before the rules run, the animation's own vertical component is
discarded: the vertical delta is forced to **0** when the motion is not simulated here, or when
`Man+0x12bc` is clear and the forward step length is 0, or when the ground at the probe is more
than 0.1 m below the probe while the forward step length is ≥ 0 (i.e. stepping onto lower ground).
In those cases the walk cycle contributes no height at all — the rules below (or, off a ledge, the
fall path) set it. Code high; the "(stepping onto lower ground)" reading of the third condition is
mine (medium).

**Where the probe point comes from (medium).** It is not a fixed offset: the on-ground step asks
the Man's model for a point in its own frame, `virtual +0x970(Man, buf, vis, 0x309-of-something,
count)` where `count` is a per-object field at `+0x1f64` of the model object (the pointer at index
`0x2f`, falling back to the one at index `0x2e` — `Man+0x178` / `Man+0x170`),
and the three returned floats are the coefficients of the up / forward / side basis groups added
to the position. Read with rules 2 and 3, this is an **animated attachment point** (a foot/contact
point, re-posed by the walk cycle every step) — the two rules then form a ground servo with a
0.1–0.2 m dead band: the body's height follows the animation, corrected so the probe never sinks
into the surface ahead and never floats more than 0.1 m above it. That reading explains both
constants coherently, but the probe's identity as a specific bone is my inference, not something
I confirmed (medium; the formulas themselves are high).

- The step-up itself is decided in `FUN_140756310` (obstacle/ledge search with the `FUN_1407135d0`
  scoring and `FUN_140754900` replays). The one `0.6` constant in the mover is **not** a step
  height: it is a ground-attach snap — the mover queries the terrain 0.5 m above the Man's position
  and, if the Man's y is **less than 0.6 m above** the terrain it returns, sets his y exactly to it
  (which also pulls him up when he is below the ground). It is gated on `Man+0x12a7 != 0`, bits 2
  and 4 of `Man+0x452`, and a timer comparison against `Man+0x126c`, so it is an occasional
  landing/attach correction, not the every-step follow of rules 1–4 (code direct, flag names
  unknown). The immediate step-up height inside the on-ground step is the 0.2 m clearance of rule 2
  — where a larger step-up is allowed, it comes out of the obstacle search, **not traced**.
- Ledge / above-a-slope: the ground ahead is followed down by rule 3 and the Man keeps walking off
  the edge; the air flag of §4 is what tells the rest of the engine that the support is gone.

## 4. Falling (mostly not traced)

- `Man+0x12a2` is the airborne flag. It is cleared and set in the on-ground step:

  ```
  Man+0x12a2 = 0
  if ((y_adj − terrain) < 2.0  &&  probe.y_raw < FUN_141659820(world,&probe) − 0.5)
      Man+0x12a2 = 1
  ```

  with `0x141a790fc` = 2.0 and `0x141a790f4` = 0.5. `y_adj` is the Man's height after rules 1–2 of
  §3 have adjusted `dy` — the test sits between rule 2 and the step-down rule 3, so the step-down
  correction is *not* included; `probe.y_raw` is the probe height as the animation placed it,
  before any of the corrections (the corrections only move `dy`). Read literally the Man must be
  **within 2 m of the ground** and the animated probe point must be **more than 0.5 m under the
  surface ahead** — a contact/near-contact test rather than a "far from the ground" test; the exact
  intent is medium confidence, the formula is high.
- **Gravity: not traced.** There is no 9.81/9.806 float constant anywhere in the image (checked by
  scanning the PE), so the falling acceleration is not a literal; it is a config value, a runtime
  vector, or lives in the physics world. Two inconclusive candidates: an unreferenced `.rdata`
  table holding 9.8 floats at VA `0x141d3d4c4` / `0x141d3d5d0` (no code reference found), and a
  literal `20.0` inside the call to `FUN_140fc5cc0` made by the alternate step `FUN_14075bc10`
  (that function also reads the tunables `settings+0x5a8` via `FUN_1406ecb70` and
  `settings+0x5ac` via `FUN_1406ecae0`). Whether the Man shares the physics objects' gravity is
  **not traced**; the PhysX/`*Physx3*` path was not examined.
- `FUN_14075bc10` is the step used when the on-ground case does not apply (mover branch, §1). It
  is the vertical/off-ground path: it uses `FUN_141659820`, basis helpers (`FUN_14035ad90` on
  `vis+0x54`, `FUN_14035acb0`), `FUN_140718a90`, `FUN_1407138c0`, `FUN_14076e890`,
  `FUN_140719ed0` — its internals were **not mapped** in this pass.
- **Fall animation trigger and landing**: not traced. The air flag and the ground queries are the
  inputs a landing/fall state selection would use; the state selection itself lives in the
  want-move/state-advance path (`FUN_140759190` → `FUN_1406005d0`).

## 5. Slope handling inside a step (code high, semantics medium)

- The block runs only when the current moves-graph state index is one of **11…16**
  (`FUN_140601560(Man+0x1348, vis+0xc8, statesTable)` → `FUN_1406056d0` → field `+0x48` of the
  state record) **and** the Man is locally controlled. Which moves those six states are is
  **not traced** (they are consecutive entries of the states table; they are plausibly the
  on-ground standing/walking group).
- The vertical component is damped-followed: let `g` be `FUN_141659820` at the probe point and
  `g_prev` the value kept in `Man+0x129c`; then

  ```
  dy += (g − g_prev) · t
  ```

  with `t` computed on the first path and, on repeat paths, from the global tunable `s` read by
  `FUN_1406ecc60` (settings block `DAT_14225db68 + 0x578`, default 5.0):

  ```
  s   = FUN_1406ecc60(&DAT_14225db68)          // settings +0x578, default 5.0
  t   = clamp((probe.y + s) / (g + s), 0, 1)   // s used negated as the floor
  if (Man+0x12bc == 0) t = 1.0                 // damping only when the flag is set
  ```

  So the Man's body follows the ground height **ahead** rather than the one under him, weighted by
  `t`; when the ground falls away the effect is a shallower descent path, which is what keeps him
  attached to a downhill slope instead of stepping off it in staircases.
- `Man+0x12a3` = "the previous ground height has been stored" (medium). On that **first** path the
  vertical delta is also snapped so that `probe.y + dy = g + 0.1` — the same 0.1 m target as §3
  rule 3, but here applied in either direction — gated on the corrected probe height being above
  world zero; and `Man+0x12bd` is set under that same comparison. The compared value is the
  world-space probe height (the Man's own y appears in the expression and cancels) against the
  literal `0.0`, so the comparator is certain even though the flag's purpose is not; **what reads
  `Man+0x12bd` was not traced**.
- `Man+0x12bc` is read twice: as the damping gate in the block above, and in the vertical-delta
  suppression test of §3.
- **`CfgSlopeLimits` (`settings+0x778`, loader `FUN_1406f2cc0`, values in sim-man-movement.md §4)
  is NOT read by any traced step function.** An instruction-pattern search over the whole image
  for float accesses at `+0x778` finds only 19 sites, none of them in `FUN_14078c930`,
  `FUN_14078c9d0`, `FUN_140756310`, `FUN_140717e40`, `FUN_140759190`, `FUN_1406005d0` or
  `FUN_140602d60`. The locomotion step uses `settings+0x578`, not the slope-limits block.
  (Caveat: an access through a computed index into a copied record would not match the search.)
- Consequence for "what happens when the slope exceeds `maxRun`/`maxSprint`": inside the traced
  step there is **no sprint→run→walk downgrade and no slope test at all**; the only slope-related
  action is the block of §3 rule 4 (the horizontal delta is zeroed when the ground ahead rises
  into the body) and the damped vertical follow above. Any downgrade must therefore happen in the
  state/move-range selection (`FUN_140759190`, which picks the wanted move and speed range) or in
  the input gear shifter (`FUN_140780e20`, §1) — **not traced**.

## 6. Model-space velocity → world displacement (high)

`FUN_140717e40` computes a **world-space delta** by projecting a body-frame delta through the vis
basis and then adds it to the position:

```
b  = dt · Man+0x12b8                               // the wanted forward speed, see below
Δx = b · vis[+0x14].x + a · vis[+0x08].x + v · vis[+0x20].x
Δy = b · vis[+0x14].y + a · vis[+0x08].y + v · vis[+0x20].y
Δz = b · vis[+0x14].z + a · vis[+0x08].z + v · vis[+0x20].z
... vertical corrections of §3/§5 applied to Δy ...
vis[+0x54/+0x58/+0x5c] = Δ / dt                    // world velocity
vis[+0x2c/+0x30/+0x34] = Δ + position              // via vis vtable slot 2 (vtable+0x10)
```

- The basis groups are the `ManVisualState` fields at `+0x08`, `+0x14`, `+0x20` (3 floats each).
  `FUN_14035b2c0(a, b)` builds them as: `+0x20` = normalise(a) (the up axis, tilted by the surface
  normal — the "hat"), `+0x14` = normalise(b − (a·b)a) (the horizontal forward axis), `+0x08` =
  their cross (the third, side axis). Whether that side axis points left or right in the
  left-handed world space (X east, Y up, Z north) was **not established**.
- The forward coefficient is `dt · Man+0x12b8` (asm: `MULSS XMM11, [RBX+0x12b8]` at `0x140718392`),
  where `Man+0x12b8` is written once per frame by `FUN_1407871c0` at `0x140787438` as the global
  tunable `settings+0x54c` (`FUN_1406ecc20`, default 0.05) times the difference of the two thrust
  axes `FUN_141093600(input, 0/1, …)`. **Caveat:** 0.05 as a direct m/s scale is inconsistent with
  observed walk speeds, so either the axis value is not the unit-range stick value or
  `Man+0x12b8` is not m/s; the multiply and its consumers are certain, the unit is **medium**.
- A second coefficient multiplies the `+0x08` group (a lateral component) and a third multiplies
  the `+0x20` (up) group. Both begin as the two floats the wanted-move call `FUN_140759190` writes
  through its two output pointers at the top of `FUN_140717e40` (asm `0x1407180fe`, `0x140718104`
  load them); a later block on the same path reduces each of them by a term proportional to `dt`,
  built from the three floats at `Man+0x1148` / `+0x114c` / `+0x1150` (asm `0x1407182c6`,
  `0x1407182cb` subtract them) — that block was **not traced**, and how `FUN_140759190` derives the
  pair is also **not traced** (it looks like a wanted-move velocity vector, but that is a guess).
  Note the `Man+0xfd4` value is *not* one of these coefficients: the mover ramps `Man+0xfd4` toward
  `Man+0x1fb` by at most `0.1·dt` per step (asm `0x1407188b7`–`0x1407189a4`), but that ramp runs on
  the other branch of the function — the one taken when `FUN_141028db0(Man) >= 1.0` or `Man+0x5e4`
  bit 0 is set (`0x14071833f`-`0x140718356`) — where no displacement delta is composed at all. What
  those two fields mean was **not traced**.
- **Facing vs velocity**: the displacement is *expressed in the body frame* — the forward
  component is along the body's own forward axis (`vis+0x14`), so for normal walking the
  displacement direction and the facing are the same by construction; there is no separate
  velocity vector to align. A lateral component exists (the `+0x08` coefficient), which is the
  hook for side-steps / aim-relative motion, but nothing in the traced code makes the body yaw
  follow a velocity vector (strafing is not implemented by turning the displacement frame; the
  turns of §2 move the frame itself). The entity's own orientation matrix (`Object`) and how it
  tracks the vis basis is **not traced**.

## 7. Not traced / open

- The gravity constant, the fall integrator and the landing (Q4). No `9.8…` literal exists in the
  image; the `FUN_14075bc10` path was only skimmed.
- Any read of `CfgSlopeLimits` (`settings+0x778`) during a step: not found in the traced chain, so
  the sprint→run→walk downgrade on a slope is unimplemented in this document (Q5).
- The moves-type `turnSpeed` clamp of the body yaw rate, the unit of `Man+0x1eb`, an instant turn
  at zero speed, leaning, and where `EvasiveLeft`/`EvasiveRight` states limit their rate (Q2).
- Where a step-up of more than 0.2 m (the §3 rule 2 clearance) is allowed: the obstacle search
  inside `FUN_140756310` was not traced; the mover's only `0.6` is the ground-attach snap of §3.
- The name of `settings+0x578`: the tunables loader (`FUN_1406eec80`, called only by
  `FUN_1406f0240`) writes `WaveEffectDepth` there and gives it the default 5.0; the step uses it as
  a slope floor. The block is a mixed "Man tunables" record (defaults plus `cfgDiving` overrides),
  so the config key names do not necessarily describe the runtime use of every slot (medium).
- `FUN_14075bc10` (the off-ground step), `FUN_14070c6d0` (post-step resolve, i.e. the collision
  response summarised in sim-man-movement.md §5) and `FUN_140e78050` were not analysed.
