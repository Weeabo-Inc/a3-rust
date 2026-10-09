# Object LOD selection and drop-out

How Arma 3 2.22 decides whether to draw an object, how it fades objects in and out, which
Resolution LOD it draws, and which LOD casts its shadow. Sources: engine code in `arma3_x64.exe`
(addresses below), `CfgVideoOptions`, and the shipped ODOL models.

The pipeline per frame:

1. **Draw test** per object (§4): drop it, fade it, or draw it, from its screen size against a
   distance threshold.
2. **Visible area** per drawn object (§5): its bounding square on screen, in pixels.
3. **LOD budget** for the frame (§6): a face budget of `sceneComplexity` faces is shared
   between all drawn objects in proportion to their visible area, and each object gets the
   finest LOD that fits its share.
4. **Shadow LOD** (§7): a per-model table maps the chosen visual LOD to the LOD that casts the
   shadow (normally one of the model's Shadow Buffer LODs).

Everything here is **high** confidence unless a line says otherwise.

## 1. Scene complexity → LOD coefficients

`sceneComplexity` is the profile value. The video menu sets it from `CfgVideoOptions >> ObjectsQuality`:

| level | value |
|---|---|
| VeryLow | 100000 |
| Standard | 600000 |
| High | 900000 |
| VeryHigh | 1300000 |
| Ultra | 1800000 |
| Extreme | 2600000 |

The setter (`0x1412842b0`) clamps it to [1e4, 1e7] and stores it at `scene+0x8a4`. The global scene
pointer is `0x1422177e0`. `0x14127ba30` then derives four coefficients from `SC = sceneComplexity`:

```
c1 = clamp(sqrt(1e6 / SC),        1, 2)        // scene+0x8a8
c2 = clamp(pow(1.2e6 / SC, 0.55), 1, 4)        // scene+0x8ac
c3 = max(pow(3e6 / SC, 0.77), 1)               // scene+0x8b0
c4 = max(c3, 6)                                 // scene+0x8b4
```
A higher SC gives smaller coefficients, so smaller objects stay visible. `sceneComplexity` is
also the frame's face budget (§6).

## 2. Coefficient over distance

`0x14127a1d0(scene, d²)` returns `coef²` for a squared camera distance `d²`. It interpolates
**linearly in d²** between these knots:

| distance (m) | coef |
|---|---|
| ≤ 20 | c1 |
| 200 | c2 |
| 350 | c3 |
| ≥ 1500 | c4 |

For example, between 20 and 200 m: `coef² = c1² + (d² − 20²)/(200² − 20²) · (c2² − c1²)`.

## 3. Object size, view scale and the model coefficients

- **Object size** `r` (`Object` vfunc +0x550 → `0x14123b0e0`, which calls `0x141272e70`):
  `r = ((maxX−minX) + (maxY−minY) + (maxZ−minZ)) / 6 · scale`, the mean half-extent of the
  LODShape bounding box (`shape+0x230 .. +0x244`) times the object scale. The vfunc returns `r²`
  times a per-class factor (vfunc +0x568, 1.0 for every class checked).
- **View scale** `K = cam+0x1d8 · cam+0x1dc · scene+0x8c4` (`0x140d57240`, `0x14127bca0`):
  - `cam+0x1dc = 1 / tan(fovX/2)` (`1 / cam+0x1c4`) and `cam+0x1d8 = 1 / tan(fovY/2)`
    (`1 / cam+0x1c8`);
  - `scene+0x8bc` / `scene+0x8c0` are the render target's width and height in pixels, and
    `scene+0x8c4 = width · height`.

  So `K · r² / d²` is the area in pixels of the square of side `2r` seen at distance `d`
  (the bounding disc covers `π/4` of it).
- **Model coefficients** (ModelInfo, also the named properties of MLOD models, parsed in
  `0x1415bf6e0`):
  - `drawImportance` → `shape+0x264`, default 1, clamped to [0.001, 10000]. Used by the draw
    test (§4).
  - `lodDensityCoef` → `shape+0x260`, default 1, clamped to [0.001, 10000]. Used by the LOD
    budget (§5).
  - `viewDensityCoef` → `shape+0x500`. Not used by this pipeline.

## 4. Draw and fade test (`0x14127ad50`)

Per object, with `d²` the squared distance from the camera to the object position (for
animated objects and proxies closer than 200 m, the animated position):

```
if d > R + objectViewDistance and (d > viewDistance or cls == 0): not drawn
if cls == 0:                                    // the size test
    T  = coef²(d²) · d²                          // vfunc +0x558 may adjust coef²; default keeps it
    A  = r² · drawImportance · K
    A  = min(A, (R + objectViewDistance)² · coef²(d²) / 1.1025)
    if A < 0.9025·T                → not drawn  (0.95²)
    if A ≥ 1.1025·T                → fully drawn (1.05²)
    else                           → dithered fade, f = (A − 0.9025T)/(1.1025T − 0.9025T)
```
- `R` is the object's bounding radius (`0x14123d7e0`). `objectViewDistance` and `viewDistance`
  are the globals `0x14225db6c` and `0x14225db68` (defaults before the profile loads: 600 m and
  900 m, `0x14117d770`).
- The `min` is what fades objects out at the object view distance: an object reaches full
  opacity only inside `(R + objectViewDistance) / 1.1025` and disappears beyond
  `≈ (R + objectViewDistance) · 1.0025`.
- `cls` is a per-Entity class (`Entity+0x556`, low nibble, signed; vfunc +0x4a0; plain
  `Object`s return 0). A non-zero class skips the size test and is limited by the view distance
  only. Its config source is not traced (medium).
- The fade level is `table[round(f·6.999 − 0.5)]`, 7 steps (`0x141ca7e28`). With the
  alternative dither mode (`0x1420bb16b`) a 9-step variant is used.
- When the object's flag `Object+0xa8` is non-negative the test has no fade band: drawn when
  `A ≥ T`.

## 5. Visible area for LOD selection (`0x14127a520`)

For each drawn object the engine stores a "visible area" in the draw entry (`entry+0xd8`). It
uses a distance `d'` measured to the bounding sphere's surface rather than its centre:

```
R  = bounding sphere radius · scale
if d < 10 R:
    t = max(d − R, 0)
    if d > 4 R:  s = (d/R − 4)/6;  t = d · s + (1 − s) · t     // blend back to d at 10 R
    d' = t
else: d' = d
d' = max(d', near plane distance)                              // cam+0x1bc

A_lod = (r² ≥ 2 d'²) ? 2K : K · r² / d'²                        // 0x141267bc0
A_lod = A_lod · vfunc+0x560(object) · lodDensityCoef            // vfunc+0x560: 1 (Man: global 0x14209dae4, 1)
```
`A_lod` is the object's bounding square on screen, in pixels, capped at twice the screen.

The same `d'` drives the **shadow test**: the object casts a sun shadow only when
`K · r² ≥ 4 · coef²(d'²) · d'²` (no `drawImportance` here), otherwise its shadow LODs are set to
"none" (`0x7f7f`).

**Area classes** (`0x141284b40`, table set up by `0x141258c30` with ratio 1.5, base 1, 40
classes): draw entries are grouped by model, requested levels and area class. The class is the
area's base-1.5 logarithm rounded with a ±0.25 hysteresis against last frame's class
(`entry+0xb9`):

```
x = ln(A_lod) / ln(1.5)            // A_lod ≤ 1e-20 → class 0
lo = floor(x), hi = lo + 1
class = prev < hi ? (x > hi − 0.25 ? hi : lo)      // coming from below
                  : (x < lo + 0.25 ? lo : hi)      // coming from above
class = clamp(class, 0, 39)
A_class = 1.5^class                // what the budget uses
```

## 6. The LOD budget (`0x141260900`, LOD pick `0x141273010`)

`F[i]` is the face count of Resolution LOD `i` (ShapeRef vfunc +0x30: the ODOL LOD summary's
`face_count` for loadable LODs, the loaded face array's count otherwise). LODs are ordered
finest first; `n` is the number of Resolution LODs (`shape+0x31e`).

**Picking a LOD for a face budget `b`** (`0x141273010`, with `prev` the group's LOD last frame,
`entry+0xb8`):

```
if n == 0: none
for i = n−1 down to 1:
    if b < (int)(0.7·F[i] + 0.3·F[i−1]):                 return i
    if prev ≥ i and b < (int)(0.7·F[i−1] + 0.3·F[i]):    return i     // hysteresis
return 0
```
So an object switches to the finer LOD `i−1` once its budget reaches 70 % of the way from
`F[i]` towards `F[i−1]`, and switches back only below 30 %.

**Sharing the budget** over all draw groups of the frame (each group: `m` instances of one model
at one area class):

```
fixed  = (int)round((viewDistance · 0.02)²)  + Σ faces of groups whose LOD is already fixed
total  = Σ over undecided groups of  m · A_class · (1 + shadowShare)
ratio  = clamp((sceneComplexity − fixed) / max(total, 1e-20), 0.03, 0.3)

pass 1: for each undecided group: if pick(F, A_class·ratio, prev) == 0:
            LOD 0;  fixed += m·F[0];  total −= m·A_class
if total > 1e-4 · (total before pass 1):
    ratio = clamp((sceneComplexity − fixed) / total, 0.03, 0.3)
pass 2: for each undecided group: LOD = pick(F, A_class·ratio, prev)
```
- The lower clamp is `max(0.3 · 0.1, 0.1 / c4)`, which is 0.03 for every legal `c4`; the upper
  clamp is 0.3 (`0x1420c2174`). So a drawn object never gets more than 0.3 faces per pixel of
  its bounding square, and never less than 0.03 if a LOD that coarse exists.
- `shadowShare` adds the shadow passes' cost: 1/32 when the object casts a shadow-buffer shadow,
  1/16 for a shadow-volume shadow, 1/64 for a third pass enabled by `scene+0xac8`. Shadows are
  skipped entirely when the sun factor (`|light+0xfc − 0.5| · 4`) is below 12/255.
- A group's LOD is "already fixed" (`entry+0xbe ≥ 0`) when: the caller forces a level, or the
  model's coarsest LOD does not fit even at the maximum ratio, `A_lod · 0.3 < F[n−1]`, which
  forces `n−1` (`0x14127a520`).
- After the passes, the final ratio is stored at `scene+0x89c`/`scene+0x898` (also raised to
  `0.25 / (W·H) · (sceneComplexity − used)` when that is larger).
- `0x141275500` then caps the frame: when the zoom is ≥ 10× (`max(cam+0x1d8, cam+0x1dc) ≥ 10`)
  and the summed screen area of the drawn objects exceeds `4 · W · H`, the remaining objects
  (in distance order) drop to their coarsest LOD.

## 7. Shadow LOD

The model's `sbsource` (ModelInfo `shadow_source`, `shape+0x508`): `visual` 0, `shadowvolume`
1, `explicit` 2, `none` 3, `visualex` 4 (strings at `0x141a857c4`, `0x141ce1998`,
`0x141ce1988`, `0x141b39300`, `0x141ce19b0`). Shipped models: `ShadowVolume` for houses and
characters, `Explicit` for trees (checked with `a3-tools p3d info`).

**Per-LOD shadow tables, built at load** (`0x1415d09e0`): for every Resolution LOD `i`,
`shape+0x4c0[i]` = the LOD that casts its shadow-map shadow and `shape+0x4a0[i]` = its
shadow-volume LOD.

1. When any Resolution LOD has a `shadowBufferLOD` / `shadowBufferLODVis` property (ModelInfo
   `preferred_shadow_buffer_lod` / `_visible`, raw property values): the value is resolved by
   resolution. `shadowBufferLODVis` names a Resolution LOD; `shadowBufferLOD` = v names the
   Shadow Buffer LOD `11000 + v` (v < 1000) or `10000 + v` (1000 ≤ v < 2000). LODs without a
   value inherit the previous LOD's entry; the ones before the first value get the first Shadow
   Buffer LOD. The shadow-volume table works the same way (`shadowVolumeLOD` = v → LOD
   `10000 + v`).
2. Otherwise (`0x1415ba4d0`), for LOD `i` with `F[i]` faces:
   - `visualex`: the LOD itself.
   - `v` = among the Resolution LODs from `min_shadow` to `n−1`, the one with the most faces
     not above `F[i]` (closest by face count if none is below);
   - `s` = the same pick among the Shadow Buffer LODs (`shape+0x31b`, count `+0x31c`; the only
     one if there is one);
   - `visual` with a `v`: `v`. Otherwise `s`, unless `s` has more faces than both `F[i]` and
     `F[v]`, then `v`. Only one of them exists: that one.
   - For the View Pilot LOD (resolution 1100) `v`/`s` are the candidates with the most faces.

**Per frame** (`0x141260900`, with `0x1420c2164` = 1 in the shipped exe): the object's shadow
LOD is `shape+0x4c0[visual LOD]`, accepted if it is a Shadow Buffer LOD or a Resolution LOD
(`0x1415bbea0`). `0x141285570` then draws the nearest loaded LOD to it.

Example, `i_house_small_01_v1_f` (Resolution LODs 7078/3523/1701/738/52 faces, `min_shadow` 4,
Shadow Buffer LODs 964 and 44 faces, `sbsource` ShadowVolume): LODs 0–3 cast their shadow from
Shadow Buffer 0 (964 faces), LOD 4 from Shadow Buffer 10 (44 faces).

(With `0x1420c2164` = 0, the shadow LOD would instead be picked per frame by face count against
`A_class · ratio / 32`, `0x1412733e0`/`0x141273200`. That branch is dead in 2.22.)

## 8. The landscape-cell pre-cull (`0x141273150`)

`0x14115cff0` walks the objects of each landscape grid cell. A cell keeps its objects sorted
by `object+0xac` (size), descending, in ranges. Given the cell's distance, `0x141273150`
binary-searches each range for the first object with `K · object+0xac < coef²·d²·0.9025` and
skips it and all smaller ones; the rest go through §4. This is only a fast path for §4. (An
earlier version of this page read `object+0xac` as a per-LOD value; it is not.)

## 9. Open points

- Super-LODs (FOREST_LOD1/2, TOWN_LOD1) handling, from the landscape side.
- The config source of the Entity class `Entity+0x556` (§4) and of vfunc +0x560 for Man.
- How proxies pick their LOD (they are drawn by their host's draw call, `Object` vfunc +0x5b8).
