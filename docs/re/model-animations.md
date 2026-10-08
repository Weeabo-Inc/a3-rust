# Model animations (model.cfg, binarised in ODOL)

Implemented in `crates/a3-anim`. The data layout is in `p3d-odol.md` (section "Animations");
this file covers how the engine turns an animation source value into a bone transform.
Addresses are RVAs in `arma3_x64.exe` 2.22.0.154103 (add `0x140000000` for the VA).

## Classes (RTTI)

`AnimationType` subclasses, one per model.cfg `type`, each embedding the transform object at
`+0x40`:

| model.cfg type | class | vtable | `GetMatrix` (slot 9) |
|---|---|---|---|
| rotation | `AnimationRotationType` | `0x1ca20e8` | `0x1203060` → `0x1203980` |
| rotationX | `AnimationRotationXType` | `0x1ca2170` | `0x12024e0` → `0x1203fe0` |
| rotationY | `AnimationRotationYType` | `0x1ca21f8` | `0x1202540` → `0x12043c0` |
| rotationZ | `AnimationRotationZType` | `0x1ca2280` | `0x12025a0` → `0x12047a0` |
| translation | `AnimationTranslationType` | `0x1ca2308` | `0x12033c0` |
| translationX (Y, Z) | `AnimationTranslationXType` ... | `0x1ca2390` ... | `0x1202600` ... |
| direct | `AnimationGenericType` | `0x1ca27b8` | `0x1202810` |
| hide | `AnimationHideType` | `0x1ca2888` | `0x1202be0` |

`AnimationHolder` (vtable `0x1ca1f30`) owns the list of types plus the per-LOD tables. Per-LOD
data is stored for up to 32 LODs (`0x20` loops in `0x1203630`, `0x12035e0`): bone index at
`+0x48 + 4*lod`, axis position at `+0xd0 + 12*lod`, axis vector at `+0x250 + 12*lod`.

## Base fields (serialiser `0x120cc00`, high confidence)

`type` (virtual), `name` `+0x10`, `source` `+0x18`, then in file order `+0x20 minValue`,
`+0x24 maxValue`, `+0x28 minPhase`, `+0x2c maxPhase`; for stream version >= 56 `+0x34
animPeriod`, `+0x38 initPhase` (else 0); last `+0x30 sourceAddress`. Rotation/translation
types store `angle0/offset0`, `angle1/offset1` at `+0x3d8`, `+0x3dc`.

## Source value to parameter (`0x12037d0`, high confidence)

`interpolate(value, a, b)` (with `a, b` = angle0/angle1, offset0/offset1, or 0/1 for direct and
hide):

- `sourceAddress = loop (1)`: `t = (value - minPhase) / (maxPhase - minPhase)`;
  `value = (t - round(t - 0.5)) * range + minPhase`, then as clamp.
- `mirror (2)`: the same wrap over a period of `2 * range`, then values above `maxPhase` are
  reflected (`maxPhase - (v - maxPhase)`), then clamped to `minValue..maxValue`, then
  interpolated as below.
- `clamp (0)` and after loop: `v = clamp(value, minValue, maxValue)` (`v <= min → min`, then
  `v >= max → max`); result `a` if `v <= minPhase`, `b` if `v >= maxPhase`, else
  `a + (v - minPhase) / (maxPhase - minPhase) * (b - a)`.

So `minValue/maxValue` clamp and `minPhase/maxPhase` give the interpolation range. In shipped
models they are almost always equal pairs. `animPeriod` and `initPhase` are not used here;
they belong to the source evaluation _(not traced)_.

## Matrices

`Matrix4` is 12 floats: three columns `aside`, `up`, `dir`, then the position; it maps column
vectors (`M * v = aside*x + up*y + dir*z + pos`). Checked in the parent-times-child product at
`0x123d080` and the inverse at `0x035bc80`. `0x18760a0` returns `(sin, cos)` (small-angle path
`(x, 1 - k*x^2)`). Helpers: `SetRotationX` `0x035c480`, `SetRotationY` `0x035c5d0`,
`SetRotationZ` `0x035c720`, `SetTranslation` `0x035cb00`, `SetDirectionAndUp` `0x035b2c0`.

Signs, in plain (right-handed formula) rotation terms `R(axis, angle)` as `glam` builds them:

| type | transform | confidence |
|---|---|---|
| rotation | `R(axisDir, +angle)` about the line through `axisPos` (built as basis * RotZ(angle) * basis^-1 with `dir` as the basis z axis) | high |
| rotationX/Y/Z | `R(axis, -angle)` about the line through `axisPos` (X and Z pass `-angle` to `SetRotationX/Z`; `SetRotationY` is itself mirrored and gets `+angle`) | high |
| translation | translation by `offset * axisVector`; the axis vector (memory points `begin` to `end`) is **not** normalised, so offsets are in axis lengths | high |
| translationX/Y/Z | translation by `offset` metres along the model axis | high |
| direct | `p = interpolate(value, 0, 1)`; `R(axisDir, -angle * p)` about `axisPos` plus `axisOffset * p` along the unit axis | medium |
| hide | `p = interpolate(value, 0, 1)`; hidden when `p >= hideValue` and not (`unhideValue >= 0` and `p >= unhideValue`); hidden returns a constant matrix (`0x2165948`, taken to be all zero), so the bone's vertices collapse | high (rule), medium (zero matrix) |

A bone with no entry for the LOD (index < 0) gets the identity.

Check against data: opening the Hunter's four doors (`door_lf` ... = 1, `angle1` = ±80°)
swings each door outwards about its front hinge, and half a turn of `wheel` (`rotationX`,
`angle1 = -2π`) mirrors every wheel vertex through its hub axis (`crates/a3-anim/tests/
real_data.rs`, plus rendered exports).

## Composition _(medium confidence; the call site was not found)_

`a3-anim` composes as follows; the engine code that walks the bones was not located (no
`call [reg+0x48]` site near the animation classes), so this is the documented model.cfg
behaviour, not a traced one:

- A bone's own transform is the product of the animations listed for it in the LOD's bone →
  animations table, the first listed applied first (`A_n * ... * A_1`).
- A bone's model-space transform is `parent * own`, so children follow their parents; hiding
  a bone hides its children.
- Skinning: vertex bone indices are LOD (sub-skeleton) bones, mapped to skeleton bones through
  the LOD's `sub_skeleton` table; weights are bytes summing to 255.

Note that all sources at 0 is not the rest pose: for example a damper `translation` with
`offset0 = 0.5` already shifts the wheels.

## Open questions

- The bone walk and the order of several animations on one bone (see above).
- Hunter `wheel_*_destruct_unhide`: `hide` with `minValue..maxValue = -1..0`, so with
  `hitlfwheel` in `0..1` the bone stays hidden; the hit sources may feed negative values.
- `animPeriod` / `initPhase` use by time-driven sources.
