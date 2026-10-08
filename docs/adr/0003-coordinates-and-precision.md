---
status: accepted
---

# RV world axes, f64 world positions with camera-relative f32 rendering, reversed-Z depth

The engine uses Real Virtuality's own world space everywhere: **left-handed, X east, Y up,
Z north, metres**. World positions are **`f64`** (`glam::DVec3` / `DAffine3`). The renderer is
**camera-relative**: the view matrix holds only the camera rotation, and every position is
turned into an `f32` offset `world - camera` (subtracted in `f64`) before it reaches the GPU.
Depth is **reversed-Z with an infinite far plane** into a `Depth32Float` buffer (clear 0,
compare `Greater`). Front faces are **clockwise** as seen from outside (Direct3D convention).

## RV conventions

- **Axes and handedness.** RV is a Direct3D engine. P3D model space is X right, Y up, Z forward
  in a left-handed system; the World uses the same axes with X = east, Z = north, Y = up. A WRP
  heightmap is a grid over X (east) and Z (north) with heights in Y _(high confidence for the
  axis assignment from model/terrain tooling conventions; to be re-confirmed on real data when
  the ODOL and WRP readers land)_.
- **Script positions swap Y and Z.** SQF positions are `[x, y, z]` = `[east, north, height]`
  (`getPosASL`, `setPos`, `modelToWorld`), so the script boundary converts
  `script [x, y, z]` ↔ `engine (x, z, y)`. Directions follow the same swap (`vectorDir`).
- **Heading.** `getDir`/`setDir` measure degrees clockwise from north. Our `Camera::yaw` uses the
  same convention in radians: yaw 0 looks along +Z (north), yaw 90° along +X (east).
- **Field of view.** RV stores FOV as tangents of half angles (`fovTop`, `fovLeft` in the
  profile, default `fovTop = 0.75`, about 73.7° vertical). `Fov::top` keeps that
  representation; horizontal follows from the aspect ratio.
- **Scale.** Terrains reach 20–40 km per side (Altis about 30 km); view distance goes to 12 km
  and beyond; object detail is centimetres.

## Why camera-relative f64

`f32` has a 24-bit mantissa: at 30 km from the origin its step is about 2 mm, and a
world→clip transform built in `f32` loses far more in the matrix multiply, producing visible
vertex jitter. Keeping the World in `f64` (sub-micrometre at 40 km) and subtracting the camera
position in `f64` gives small `f32` offsets near the camera where precision matters, and
graceful loss only far away where it does not. The same `f64` positions serve simulation and
SQF (`getPosASL` returns 32-bit floats in the original, so we lose nothing by being more
precise).

## Why reversed-Z infinite

With a floating-point depth buffer, mapping near to 1 and infinity to 0 spreads precision
almost logarithmically over distance; a 0.1 m near plane still resolves metres at 12 km (see
`camera::tests::depth_keeps_resolving_at_terrain_distances`). An infinite far plane removes a
tuning knob that RV exposes as view distance; view distance stays a culling and LOD decision,
not a projection parameter.

## Considered options

- **Right-handed internal space (convert at load)**: rejected. Every format reader, config
  value (memory points, `selectionPosition`) and RE note would need conversion; mistakes would
  be silent mirror images. Keeping RV axes makes RE findings directly usable.
- **Floating origin (rebase world when the camera moves far)**: rejected for now. It mutates
  every stored position and interacts badly with SQF holding positions. Camera-relative
  rendering gives the precision benefit without moving the World.
- **Standard Z (near 0, far 1) with a far plane**: rejected; z-fighting at kilometre ranges
  unless the near plane is pushed out, which clips nearby geometry such as the player's weapon.
- **`f32` world positions**: rejected for the precision reasons above.

## Consequences

- Every renderer feature computes `world - camera.position` in `f64` per object or per tile
  and uploads `f32` offsets; it never uploads absolute world positions.
- Depth tests use `Greater`/`GreaterEqual`; depth clears to 0; shaders that reconstruct
  distance use `view_z = near / depth`.
- Mesh data from ODOL keeps its winding; pipelines use `FrontFace::Cw` _(to confirm against
  real ODOL faces)_.
- The SQF boundary owns the Y/Z swap; engine code never stores script-order vectors.
