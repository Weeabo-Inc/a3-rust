# Object LOD selection and drop-out

How Arma 3 2.22 decides whether to draw an object, how it fades objects in and out, and how the
scene-complexity setting feeds the LOD choice. Sources: engine code in `arma3_x64.exe` (RVAs below)
and `CfgVideoOptions`.

**Status.** These parts are pinned down:
- the scene-complexity coefficients;
- their distance curve;
- the object size measure;
- the draw/fade test.

The final step, mapping the threshold onto a LOD index, is identified, but the per-LOD value it
compares against is not decoded yet (§5). §6 gives a working rule for the renderer in the
meantime.

## 1. Scene complexity → LOD coefficients (high)

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
A higher SC gives smaller coefficients, so objects get finer LODs and smaller objects stay
visible.

## 2. Coefficient over distance (high)

`0x14127a1d0(scene, d²)` returns `coef²` for a squared camera distance `d²`. It interpolates
**linearly in d²** between these knots:

| distance (m) | coef |
|---|---|
| ≤ 20 | c1 |
| 200 | c2 |
| 350 | c3 |
| ≥ 1500 | c4 |

For example, between 20 and 200 m: `coef² = c1² + (d² − 20²)/(200² − 20²) · (c2² − c1²)`.

## 3. Object size and view scale (high on the formulas, medium on meaning)

- **Object size** (`Object` vfunc +0x550 → `0x141272e70`):
  `r = ((maxX−minX) + (maxY−minY) + (maxZ−minZ)) / 6 · scale`, i.e. the mean half-extent of the
  LODShape bounding box (`shape+0x230/+0x23c`) times the object scale. The function returns `r²`,
  multiplied by a per-class factor (vfunc +0x568, 1 for plain objects).
- **View scale** `K = cam+0x1d8 · cam+0x1dc · scene+0x8c4`. The camera fields are most likely the
  projection scales (1/tan of the half FOV, x and y), so `K·r²/d²` is roughly the projected area.
  `scene+0x8c4` is a global multiplier whose source is not traced.
- The shape has a density factor at `shape+0x264`, likely the `lodDensityCoef` / `viewDensityCoef`
  named properties. The engine knows both names.

## 4. Draw and fade test (high)

From `0x14127ad50`, the per-object visibility pass:

```
A  = r² · shapeDensity · K                 // "visible area"
T  = coef²(d²) · d²
if A < 0.9025·T        → not drawn         (0.95²)
if A ≥ 1.1025·T        → fully drawn       (1.05²)
else                   → dithered fade, f = (A − 0.9025T)/(1.1025T − 0.9025T)
                          level = table[round(f·6.999 − 0.5)] (7 steps, 0x141ca7e28)
```
Objects also need `d ≤` the object view distance (`0x14225db68`, read from the profile at load) plus
their bounding radius. Objects closer than 200 m (`d² < 40000`) with an animated or proxy flag use
the animated-position distance.

A second test in `0x14127a520` sets the shadow flags (`0xffff` vs `0x7f7f`). An object casts
shadows when `4·coef²(d'²)·d'² ≤ A`, so it must be about twice as large on screen as for drawing.
`d'` is the distance pushed out by a size-dependent term. Medium confidence.

## 5. LOD index (algorithm high, threshold value not decoded)

`0x141276e90` packs `(coef², K, coef²·d²·0.9025)`. `0x141273150(params, shape,
lo, hi)` binary-searches the shape's resolution LOD list (`shape+0x40`, count `+0x48`) for the
first LOD `i` in `[lo, hi)` with `K · level[i]+0xac < coef²·d²·0.9025`, and draws that LOD. So
`level+0xac` is a per-LOD area-like value that decreases from fine to coarse LODs. Its meaning
(face area or a resolution-derived screen area) is not decoded yet. The caller (`0x14115cff0`)
iterates LOD ranges and handles the forest/town super-LODs.

The engine also reads the model named properties `shadowLOD`, `shadowVolumeLOD`,
`preferShadowVolume`, `lodDensityCoef` and `viewDensityCoef`. How the shadow LOD is chosen (shadow
volume 10000/10010 vs a visual LOD for shadow maps) is not traced.

## 6. Working rule for the renderer (until §5 is decoded)

Use §1, §2 and §4 as they are. For the LOD index, follow the same criterion: draw the coarsest
resolution LOD whose detail still exceeds the threshold. A reasonable stand-in for `level+0xac` is
`(shape r / LOD resolution)²`: the resolution LODs in ODOL are numbered 1, 2, 3… roughly by screen
size. Test it against the real game visually before relying on it.

## 7. Open points

- Meaning of `level+0xac` and of `scene+0x8c4`.
- Shadow LOD selection.
- Super-LOD (FOREST_LOD1/2, TOWN_LOD1) handling.
