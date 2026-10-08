# Man movement: moves graph, speed from animations, slopes, collision

How Arma 3 2.22 drives a soldier from `CfgMovesBasic` / `CfgMovesMaleSdr`. Source:
`arma3_x64.exe` (RVAs below) and the shipped config. Read `rtm.md` for the animation format
(the per-cycle `step` vector).

**Status.** These parts are pinned down from code (high):
- the config model;
- the transition graph and how a path through it is found;
- the phase rate;
- the velocity blend.

Slope limits, collision and turning are only summarised (medium/low). They are listed as
follow-ups in §7.

## 1. Config model (loaders `0x1405fba60`, `0x1405fe3d0`, `0x140606300`)

`CfgMovesBasic` / `CfgMovesMaleSdr` contain:

- **`Actions`**: `ActionMap` classes. Each maps action names (`WalkF`, `Stop`, `Crouch`,
  `FireNotPossible`, …) to state names. A state selects its map with `actions = "..."`.
- **`States`**: one class per animation state. Fields used by the engine:

| entry | meaning |
|---|---|
| `file` | RTM; its `step` is the displacement per cycle (forward = −Z) |
| `speed` | phase rate in cycles/s. **If negative, the rate is `1/|speed|`**: −T means one cycle lasts T seconds |
| `skillSpeedCoef` | default 1. Effective rate = `speed · (1 + (skillSpeedCoef − 1)·s)`, with `s` from a global tunable block (`0x1405ff5a0`); probably unit skill (medium) |
| `relSpeedMin`, `relSpeedMax` | allowed speed scaling |
| `interpolationSpeed` | weight ramp rate when blending into this state (§3) |
| `interpolationRestart` | |
| `minPlayTime` | clamped to [0, 1]; fraction of the cycle before leaving is allowed |
| `terminal`, `walkcycles`, `reverse`, `equivalentTo`, `affectedByFatigue`, `ragdoll`, `looped`, `variantsAI`, `limitGunMovement`, `enableOptics`, `headBobStrength`, `soundOverride` | loaded (semantics as the names suggest; not traced) |
| `connectTo[]`, `connectFrom[]`, `interpolateTo[]`, `interpolateFrom[]`, `interpolateWith[]`, `connectAs` | transition edges (§2) |

- **Moves-type level** (`0x1405fba60`): `turnSpeed`, `stance`, `upDegree`, `limitFast`,
  `useFastMove`, `rifle`, `leanLRot`/`leanRRot`, `leanLShift`/`leanRShift`.

## 2. Transition graph and path search (high)

Edges are added by `0x140603c60(graph, from, to, kind, cost, flag)` into per-state lists
(`graph+0xe8`, 0x18 bytes per state). Each edge is 6 bytes:
`{int16 to, int16 round(cost·1000) clamped to ±32767, u8 kind, u8 flag}`.

| config | edges added |
|---|---|
| `connectTo[] = {state, cost, ...}` | from → to, kind 1 (**connect**) |
| `connectFrom[]` | reverse edges, kind 1 |
| `interpolateTo[]` | from → to, kind 2 (**interpolate**) |
| `interpolateFrom[]` | reverse edges, kind 2 |
| `interpolateWith[]` | both directions, kind 2 |
| `connectAs` | copies another state's edges |

The engine moves from the current state to a target state (the state that the wanted action maps
to) along the cheapest path, found by `0x140604c40`:
- If a direct edge exists, the path is just the target.
- Otherwise it runs **Dijkstra** over the integer costs (priority queue, `INT_MAX` = unreached)
  and returns the state sequence.
- With no path, the request fails.

The kind decides how each hop starts. **Interpolate** starts blending at once. **Connect** waits
for the current cycle, respecting `minPlayTime`. The kind is certain from the loader; the timing
semantics are medium confidence, from the names and the usual RV behaviour.

## 3. Speed from animations (high, `0x140602d60`)

The man keeps a small list of active states with weights `w_i`. It is updated each step `dt`:

```
for each active state i:
    if i is the current or the target state: w_i = min(w_i + dt·interpolationSpeed_i, 1)
    else:                                     w_i = w_i − 3·dt·interpolationSpeed_i   (removed at ≤ 0)
v_target = Σ_i w_i · rate_i · step_i · k / Σ_i w_i       // states with a zero step add weight only
Δ = clamp(v_target − v_prev, −20·dt, +20·dt) per axis    // acceleration limit 20 m/s²
v = v_prev + Δ;  displacement = v·dt                     // model space; forward is −Z
```
- `rate_i` is the effective phase rate from §1.
- `step_i` is the RTM step per cycle.
- `k` is a caller-supplied speed factor (e.g. the `relSpeed` scaling).

The **animation is the source of truth for movement speed**: ground speed = `|step| · rate`. For
example, `AmovPercMwlkSrasWrflDf` (rifle walk) has `speed = 0.85` and RTM `step = (0,0,−1.62)`,
which gives 0.85 · 1.62 ≈ 1.38 m/s before `relSpeed` scaling (0.8–1).

## 4. Slopes and fatigue (config high, use medium)

`CfgSlopeLimits` (loaded in `0x1406f2cc0` into the global settings block at `+0x778`):

| entry | value |
|---|---|
| `maxRun` / `minRun` | 0.6 / −0.8 |
| `maxSprint` / `minSprint` | 0.3 / −0.5 |
| AI variants | same |
| `Duty` | `maxSlope 0.839`, `minSlope −1`, `optimalSlope −0.268`, `maxDuty 15`, `minDuty 0.15` |

The values are terrain gradients (rise/run) along the movement direction:
- above `maxRun` the man cannot run (falls back to walking);
- above `maxSprint` he cannot sprint;
- `Duty` scales fatigue cost by slope.

The thresholds are certain. Exactly how the engine downgrades the move is not traced.

## 5. Ground and collision (medium)

- **Ground height** comes from the world surface query (`0x1416527b0`, also used by projectiles).
  It returns the terrain height or a **Roadway LOD** face of an object, plus the object. Walking
  on buildings, bridges and ramps works this way: the man stands on the highest roadway/terrain
  surface under him within step range.
- **Collision** against objects uses `CollisionCapsule`s built from the skeleton's convex
  components (`collisionGeomCompPattern` in the skeleton config; error text at `0x141ca7600`).
  The capsules are tested with `Object::Intersect(CollisionBuffer, …)` against the other object's
  **Geometry LOD**. This is the engine's own code, not a PhysX character controller.

## 6. Turning

`turnSpeed` (moves type) limits the body yaw rate. Aiming and head turning use the
`limitGunMovement` / `aimPrecision` family. Not traced further.

## 7. Open points

- How actions are requested (`playMove`/`switchMove`/AI) and how connect vs interpolate timing
  works exactly (`minPlayTime`, cycle end).
- How slope limits are applied, and how ground snapping and step-up height work.
- Capsule sizes, collision response, and falling (gravity) when there is no ground.
- Turning model details.
