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
the result becomes the Pose that skinning uses. Implemented in `crates/a3-pose`. A moves type
(`a3-moves`) names each move's RTM (`Move::file`) and plays it to a phase; `crates/a3-pose`'s
`MoveClips` holds those animations and `MoveBlend` is a blend state in the moves type's terms
(previous and current `MoveId` with their phases, and the blend between them).

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
  below skips; it is **empty** for `OFP2_ManSkeleton` in the shipped config (`ragdoll =
  "Soldier"` is the class's other entry), so every bone is converted.
- **Reversed records** (high for the data, medium for where the engine does it): RTM records are
  stored a half turn about Y away from the decoded ODOL model space. Converting a record to model
  space conjugates it with `S = diag(-1, 1, -1)`: the quaternion's x and z and the translation's
  x and z change sign (`a3_anim::reversed`). The engine's plain-RTM loader `0x1250490` does
  exactly this to every 12-float matrix when its "reversed" flag is set (it negates elements 1, 3,
  5, 7, 9, 11 of the column-major `Matrix4`, i.e. `S * M * S`). The flag comes from the animation
  key (`FUN_14124a0a0` passes `key+0x10`) and is stored at `anim+0x10c`; for BMTR files the
  serialiser `0x12557a0` overwrites it with the header byte, which is 1 in every shipped file. No
  sign flip was found on the BMTR record path (`0x1255e50` reads the 14-byte records raw), yet the
  shipped binarized records only fit the shipped soldier after the conversion: unconverted, the
  rotation centres of the arm and finger records land on the mirror image (`x -> -x`) of their
  joints and no composition keeps the arms together (issue #247's eight readings); converted,
  every joint of the idle and the walk keeps its rest distance to 2..5 mm (below). Where the
  engine reconciles the BMTR records with the model (a reversed Shape on load is the likely
  place) is open; the rule itself is settled by the data.
- **Load conversion** (high): after reading a keyframe, every bound bone except the weapon bone
  gets its translation replaced by `R * pivot + t` (written back as half floats). A loaded record
  is therefore `[R | R * pivot + t]`: it maps the bone's rest pivot onto the **posed position of
  the joint in its parent's frame** (see *Skeleton walk*).
- **Blending** (high): a layer's two record buffers — two keyframes of one RTM, or an animation
  layer's two sources — are blended in `0x12102c0` by **slerping the quaternions** (`0x35d140`)
  and **lerping the converted translations** by `phase`. With one buffer present the record is
  copied through unchanged. Further layers are folded in the same way, pairwise, by
  `0x12105b0` / `0x1211710` / `0x1211d90`: each layer's contribution is weighted by the layer
  weight at `+0x1c` times the animation's per-bone weight (`*(anim+0x18) + 4 + 8*i`), clamped to
  0..1, and weights at or below 0.001 are skipped. The record's index `i` is
  `FUN_14069c8c0(anim, bone)` — not a weight itself but a lookup in the animation's bone table
  (int array at `anim+0x60`, count `anim+0x68`) returning `-1` for a bone the animation does not
  carry. The Man's layer list at `+0x3a0` (count `+0x3a8`, stride `0x70`, layer weight `+0x1c`)
  treats that as weight 0 — the layer leaves the bone alone — while the list at `+0x740` (count
  `+0x748`) leaves such a bone at weight 1. The earlier "whole matrices linearly, no slerp"
  note described the **skeleton-less** path (`0x35bf70` / `0x35a020`, taken when the pose builder
  is called without a skeleton); the skeletal path does slerp.
- **Pose output** (high): `0x12105b0` builds `M` from the accumulated quaternion (after the
  decoder's row/column swap, so it is the matrix of the conjugate) and emits
  `translation = T - M * Q`, with `T` the blended posed joint and `Q` the bone's rest pivot
  (`*(float3*)(*(skeleton+0x78) + 0xc*bone)`). The emitted matrix is the bone's frame **relative
  to its parent**: it maps the rest pivot onto the posed joint in the parent's frame. Where
  nothing is blended (`T = R * Q + t`) its translation is exactly the file's `t` — the pivot term
  only shows up while blending.
- **Attach bone** (high, narrow): inside `0x12105b0`, the one bone whose parent index equals the
  animation's attach bone (`*(int*)(anim+0x90)`) gets the parent's accumulated quaternion composed
  into its own and slerped by the animation's weight at `+0x94`. Which animations set `+0x90` is
  not traced.
- **Skeleton walk** (high): the palette is built by a depth-first walk over the skeleton tree
  (`0x12547f0` -> `0x12499f0` (recursive, child counts per node at `skeleton+0xc0`) ->
  `0x124b550`): each node's matrix is `parent * own`, with `own` the `0x12105b0` output and the
  root's parent the identity (`0x208be18`); the result is copied to every LOD bone that maps to
  the skeleton bone (`lod+0xa8`, `lod+0xd8`). `0x1211190` (a bone's model matrix: walk up through
  `*(skeleton+0x40)+0x78`, multiplying on the left) and `0x1211390` (a point on a bone, used for
  memory points) compose the same way. So a child follows every parent above it; the earlier
  reading of a flat palette was wrong.
- **Pose coherence** (measured, `crates/a3-pose/tests/real_data.rs`, `crates/a3-anim/tests/
  real_data.rs`): reversed, converted and composed down the skeleton, every parent/child joint
  distance of the shipped soldier holds to **2 mm in the idle and 5 mm in the walk** over every
  keyframe (f16 precision), the toes of the idle stand at `y = 0.010` and the walk's lowest toe
  stays within `0.005..0.056` m of `y = 0`, the hands of the rifle idle are in front of the chest,
  and the posed drawn mesh is a `0.62 x 1.49 x 1.11` m box (no head: it is a proxy). Three bones
  are not joints of the body: `weapon` (`(-1, 0, 0)`), `launcher` (`(1, 0, 0)`) and `camera`
  (`(0, -0.61, 0.007)`) are attachment points.
- **Face rig** (measured): in the move RTMs, `face_hub`'s record is the inverse of the head's
  composed frame (its children land on their rest positions in model space), so the face rig does
  not follow the head through the base move. No vertex of the body model binds to a face bone;
  the face belongs to the head proxy model, posed by its own path (not traced).
- **Pose space and placement** (high for the data): the composed pose is in the pivots model's
  space, where the root record (the pelvis's `t.y = 0.912`) puts the feet on `y = 0`. An ODOL
  model's vertices are stored relative to its `bounding_center` (`autocenter`), so a vertex goes
  to pivot space by adding it: for `B_Soldier_01` (`bounding_center = (0.305, 0.836, -0.401)`)
  the rest mesh plus the centre lands on the pivots (hands at `x = +-0.62`, toes at `y = -0.97`).
  The skinning matrix in the model's own space is `translate(-c) * composed * translate(c)`, and
  the ground under the posed Man is the model-space point `-c`, which goes on the entity's
  position. Like any offset it belongs to the placement: folding a lift into each bone
  (`bone * up`) multiplies it by the bone's rotation (issue #247).
- **Facing**: the posed Man faces `-Z` in model space, like the rest mesh (`p3d-odol.md`: a raw
  model's front is `-Z`).
- **Skinning** (high): `a3-anim`'s `frame * translate(-pivot)` is exactly this emitted
  `[M | T - M*Q]`; `a3_anim::Pose::from_rtm_frames` composes it down the skeleton
  (`a3_anim::compose_hierarchy`), and `a3_pose::ManRig::skinning_pose` does the same for a move
  state.

## Open questions

- The order in which several animations act on one bone in the model.cfg (non-skeletal) path.
- Where the engine applies the half-turn conversion to binarized (BMTR) records, or reverses the
  model instead (see *Reversed records*).
- Which layers a Man's animation holder carries on top of the base move (`+0x3a0`, `+0x740`):
  gestures, aiming, the weapon's `handAnim`, and how hand IK and the head/look direction act on
  the composed pose.
- How the head proxy's face rig is posed (see *Face rig*).
- Hunter `wheel_*_destruct_unhide`: `hide` with `minValue..maxValue = -1..0`, so with
  `hitlfwheel` in `0..1` the bone stays hidden; the hit sources may feed negative values.
- `animPeriod` / `initPhase` use by time-driven sources.
