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

## RTM skeletal poses

The path a character move takes: each record is decoded to a bone matrix, records are blended, and
the result becomes the Pose that skinning uses. Implemented in `crates/a3-pose`.

Engine functions (RVAs): BMTR serialiser `0x12557a0`; transform decode `0x1212520`; load-time
conversion `0x124d370`; skeleton pivots `0x1252020` (from `0x1251c20`); the two-buffer record
blend `0x12102c0` (quaternion slerp `0x35d140`); the pose builder `0x12105b0`, which folds in
further layers (`0x1211710`, `0x1211d90`) and emits each bone matrix; half-float decode
`0x1211fe0`; the skeleton-less scaled matrix add `0x35bf70` / `0x35a020`.

The skeleton object as the pose builder reads it: parent index `int32` per bone at `+0x40`
(stride 4), pivot `float3` per bone at `+0x78` (stride `0xc`), bone count at `+0x80`. The
animation object carries its attach bone at `+0x90` (`int`) and that bone's blend weight at
`+0x94`.

- **Bone binding** (high): RTM bone names map to Skeleton bones by name (`0x12531f0`).
- **Rotation** (high): the decoder `0x1212520` turns the `i16` quaternion (x, y, z, w) / 16384
  into a `Matrix4` whose columns are the **rows** of the usual quaternion matrix, i.e. the
  matrix of the conjugate quaternion. `a3-rtm` returns the quaternion as stored; the engine (and
  `a3-anim`) uses its conjugate.
- **Pivots** (high): CfgSkeletonParameters `>> skeleton >> pivotsModel` (for
  `OFP2_ManSkeleton`: `A3\anims_f\data\skeleton\SkeletonPivots.p3d`, not autocentred, pelvis at
  the origin). Each bone's pivot is the memory point named like the bone; without one it takes
  the parent's pivot, else the origin. `weaponBone` (same class) names one bone the conversion
  below skips.
- **Load conversion** (high): after reading a keyframe, every bound bone except the weapon bone
  gets its translation replaced by `R * pivot + t` (written back as half floats). A loaded record
  is therefore `[R | R * pivot + t]`: it maps the bone's rest pivot onto the **posed position of
  the joint**. How far that reading holds on real data is measured under *Pose coherence* below.
- **Pose coherence** (measured, `crates/a3-pose/tests/real_data.rs`): reading the emitted frame as
  `[M | T - M*Q]` over the rest pivots, the shipped `OFP2_ManSkeleton` keeps 31 of the 102
  parent/child rest distances within 5 cm (`idle`; 34 for `walk`). It holds along the trunk above
  the spine (`spine1`, `spine3`, `neck`, `head` within 1 cm of rest) and down the legs (knee,
  foot, toe within 2..5 cm) — the idle stands nearly at its rest pose — and it fails in three
  groups:
  - **Records that carry a position, not a correction.** Almost every bone's `t` is a small
    correction to its pivot (a few cm; the whole trunk), but the root's `y` is the hip height
    (`0.9116` at frame 0 of both moves, while `pivotsModel` has the pelvis at the origin with the
    feet at `-0.889`), which alone puts the pelvis 0.912 m out of the trunk's space and both
    uplegs 0.85..0.88 m out of it; `face_hub` carries `t = (-0.29, -0.82, 0.17)` in the idle and
    drags every `face_*` and eye bone with it (0.64..0.88 m).
  - **Pivots that are not joints.** `weapon` and `launcher` are `(-1, 0, 0)` and `(1, 0, 0)`, and
    `camera` is `(0, -0.61, 0.007)` — attachment points, not bone positions (the `weaponBone` is
    also the one the load conversion skips, so its `t` stays a raw offset).
  - **The arm chain**, where the error accumulates outward: shoulder 0.06 m, arm roll 0.33, forearm
    0.43..0.89, hand 1.20, finger tips 1.77. The pivots model's arms are a T-pose (hand at
    `x = 0.59`), the idle's arms hang at the hip (hand `y = -0.88`), and the file's arm rotations
    are large there — the signature of a rest pose other than the pivots model's. No single-rule
    reading fixes them: raw `t`, `t + Q`, `Q + t - t_root`, `M*Q + t - t_root` and `R*Q` alone
    were all measured, and none keeps the chain rigid.
  None of this contradicts the emitted matrices — an unblended frame's translation is exactly the
  file's `t` (the `M*Q` term only appears while blending), and `a3-pose` reproduces `a3-anim`'s
  frames to 1e-4 — but the arm records are evidently not a complete Man pose on their own.
- **Blending** (high): a layer's two record buffers — two keyframes of one RTM, or an animation
  layer's two sources — are blended in `0x12102c0` by **slerping the quaternions** (`0x35d140`)
  and **lerping the converted translations** by `phase`. With one buffer present the record is
  copied through unchanged. Further layers are folded in the same way, pairwise, by
  `0x12105b0` / `0x1211710` / `0x1211d90`: each layer's contribution is weighted by the layer
  weight at `+0x1c` times the animation's per-bone weight (`*(anim+0x18) + 4 + 8*i`), clamped to
  0..1, and weights at or below 0.001 are skipped. The earlier "whole matrices linearly, no
  slerp" note described the **skeleton-less** path (`0x35bf70` / `0x35a020`, taken when the pose
  builder is called without a skeleton); the skeletal path does slerp.
- **Pose output** (high): `0x12105b0` builds `M` from the accumulated quaternion (after the
  decoder's row/column swap, so it is the matrix of the conjugate) and emits
  `translation = T - M * Q`, with `T` the blended posed joint and `Q` the bone's rest pivot
  (`*(float3*)(*(skeleton+0x78) + 0xc*bone)`). The emitted matrix maps the rest pivot onto the
  posed joint. Where nothing is blended (`T = R * Q + t`) its translation is exactly the file's
  `t` — the pivot term only shows up while blending.
- **Parent composition** (high, narrow): every bone is emitted flat except the one whose parent
  index equals the animation's attach bone (`*(int*)(anim+0x90)`, the CfgSkeletonParameters
  `weaponBone` for a Man); for that bone the parent's pose is composed in (a quaternion product)
  and blended by the animation's weight at `+0x94`. There is no general parent walk in this
  path — a child's rest offset is carried by the pivots, not by composing bone matrices.
- **Skinning** (high): `a3-anim`'s `frame * translate(-pivot)` composes to exactly this emitted
  `[M | T - M*Q]`, so `a3_anim::Pose::from_rtm_frames` plus `a3_anim::skin` is the engine's
  transform for these poses.

## Open questions

- The order in which several animations act on one bone in the model.cfg (non-skeletal) path.
- The origin of the pose space: the pivots model has the pelvis at the origin and the feet at
  `y = -0.889`, while the root bone's own record carries `y = 0.912` on every frame, so the
  emitted pose mixes two origins (see *Pose coherence*). How the pose is placed on the entity
  (the exact root offset) was not traced. `a3-pose` returns the pose in the engine's animation
  space and leaves the offset to the caller.
- What completes the arm chains (see *Pose coherence*): the base move's arm rotations are large
  where the trunk's are near identity, and the error grows along the chain, so the leading
  suspect is a rest pose other than `skeletonpivots.p3d`'s T-pose arms — either another pivots
  set (`CfgSkeletonParameters` per skeleton) or a further pose-builder layer (`0x1211710` /
  `0x1211d90`, e.g. a weapon hold) that re-poses them. Worth checking next: what the per-bone
  weight `FUN_14069c8c0(anim, bone)` returns for the arm and face bones.
- Hunter `wheel_*_destruct_unhide`: `hide` with `minValue..maxValue = -1..0`, so with
  `hitlfwheel` in `0..1` the bone stays hidden; the hit sources may feed negative values.
- `animPeriod` / `initPhase` use by time-driven sources.
