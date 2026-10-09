# Environment: date, sun and moon, lighting tables, overcast, fog

How the engine turns a World's config and the current date, weather and fog into light.
Implemented in `crates/a3-environment`. Addresses are RVAs in `arma3_x64.exe` (build 2.22).
Lighting/tonemap/atmosphere shader maths: see `render-materials.md` and the render-lighting topic.

## Sun and moon position

- The world config is read at `0x1155750` into the Landscape object: `latitude` (default -40)
  and `longitude` (default 15) in radians at +0x2e34 / +0x2e38, `elevationOffset`. **high**
- **With trueSKY (all A3 worlds with `class SimulWeather`)** the sun and moon directions come
  from trueSKY. `0x1670830` hands it the latitude **negated** (`^0x80000000`), the longitude, and
  a time zone of `longitude · 12/π` hours (= longitude° / 15), plus date and time of day.
  `0x10851a0` reads the directions back and maps trueSKY's (east, north, up) to RV axes. So the
  config's `latitude = -35.152` (Altis) means 35.152° N, Tanoa's `17.698` means 17.7° S, and
  the clock runs on local mean solar time. **high** for the plumbing; trueSKY's own ephemeris is
  not in the exe, so `a3-environment` uses standard ones (NOAA solar position; Meeus' truncated
  lunar theory with topocentric parallax), good to a fraction of a degree.
- **Without trueSKY** `0x10813c0` builds the directions from rotation matrices: tilt
  `RotX(0.40142)` (23.0°), daily `RotY(2π·t)`, yearly `RotY(2π·(d/365 − 0.030137))` (offset
  11 days: the December solstice), `RotX(latitude)`, `RotX(−π/2)`; the moon adds an orbit tilt
  `RotZ(5°)` and an angle `((year + d/365) − 1985.0082)·81.90581 + π`. Not used by A3 worlds;
  not implemented. **high**
- The **moon phase** (`0x1080fc0`, SQF `moonPhase`): `x = acos(−dot(Ls, Lm))/(2π) + 0.5` with
  `L` the light directions; `moonPhase = 2 − 2x` = sun-moon elongation / π (0 new, 1 full).
  **high**
- The **light direction** used for shading is the sun when the table's `sunOrMoon` ≥ 0.5, else
  the moon. A copy used for shadows has its height forced to at least 0.4 before
  renormalising (`0x10851a0`, `if (-0.4 < y) y = -0.4` on the light-travel vector). **high**
- Calendar: days per month with Gregorian leap years (`0x1088ab0`). **high**

## Lighting table: `CfgWorlds >> W >> Weather >> LightingNew`

- Each class is one entry keyed by `height`, `overcast` and `sunAngle` (`0x1616b50`). The key
  stored is **`sin(sunAngle)`**, not the angle. **high**
- Entries are grouped by height (tolerance 1e-4), then by overcast, and sorted by
  sin(sun angle) (`0x1614f20`). Lookup is trilinear with linear interpolation of every field
  (`0x1615910` height → `0x16153d0` overcast → `0x1615640` sun, lerp `0x1616190`): find the
  first key above the value, take it and its predecessor, `t = clamp((x − lo)/(hi − lo))`;
  `t < 0.0001` takes the lower entry, `t > 0.9999` the upper, otherwise lerp. Outside the table
  the end entry is used. **high**
- Colours (`0x11fc520`): `{r, g, b}` or `{r, g, b, a}` as written (linear), or
  `{{r, g, b}, ev}` = `rgb · 2^ev / luma(rgb)` with `luma = 0.299 r + 0.587 g + 0.114 b`. So
  the EV is the base-2 log of the luminance in the engine's absolute units. **high**
- Fields (`0x1616e20`): `diffuse`, `diffuseCloud`, `ambient`, `ambientCloud`, `ambientMid`
  (default `(ambient + groundReflection)/2`), `ambientMidCloud` (likewise),
  `groundReflection(Cloud)`, `bidirect(Cloud)`, `sky`, `skyAroundSun`, `fogColor`,
  `desiredLuminanceCoef(Cloud)`, `luminanceRectCoef(Cloud)`, `apertureStandard/Min/Max`,
  `standardAvgLum`, `rayleigh`, `mie`, `cloudsColor`, `swBrightness`; `sunOrMoon`.
- Each `X` / `XCloud` pair is blended by how much cloud covers the sun (trueSKY). We use
  `1 − through` of the current overcast level. **medium** (the coverage source is trueSKY).
- The **height** key is the camera's height relative to the water: Altis has groups at 0 and
  −0.1 (under water). We pick −0.1 below sea level. **medium**
- The lookup **overcast** is the weather's `lightingOvercast` (below), not the overcast itself.
  **medium** (from the config layout; Altis' overcast 0.3 gives lighting overcast ≈ 0.23).
- Shipped Altis table: overcast groups 0.25 / 0.6 / 0.85 × sun angles −24…90°, plus an
  underwater group. Noon direct light ≈ 2^17.2 (150 000), ambient ≈ 2^14.8, night ≈ 1–15.

## Overcast levels: `Weather >> Overcast`

Classes with `overcast`, sky textures (`sky` 8×8 gradient AI88, `horizon` band, `skyR` fisheye
sky-and-cloud photo used for reflections), cloud layer `alpha`, `size`, `height`, `bright`,
`speed`, `through`, `diffuse`, `cloudDiffuse`, `waves`, `lightingOvercast`. We sort by overcast
(Altis lists `Weather1` and `Weather7` both at 0; the first is kept) and interpolate numbers
linearly, textures by blend factor. **medium** (interpolation not traced).

## Fog

- Config (`0x114b690`): `startFog`, `forecastFog`, `startFogBase`, `forecastFogBase`,
  `startFogDecay`, `forecastFogDecay`, `fogBeta0Min`, `fogBeta0Max`. **high**
- Extinction at the fog base (`0x1265600`): `beta0 = min + (max − min) · (e^(4v) − 1)/(e^4 − 1)`
  for fog value `v` (clamped to 0..1). The fog parameter block (`0x12677c0`) stores
  `beta0 · e^(decay · base)` and the decay, i.e. height fog
  `beta(h) = beta0 · e^(−decay · (h − base))`. **high**
- In the shaders (`render-atmosphere.md` §2) this fog is `PSC_HazePars` (base, density, decay),
  integrated along the in-air part of the view ray; `PSC_PhysicalFog` (source not traced) and a
  linear `fogStart`/`fogEnd` fog come on top. `a3-render` implements the height fog analytically
  (`RenderSettings::fog_density`/`fog_decay`), the linear fog (`fog_start`/`fog_end`) and a
  colour haze (`haze`) in the post atmosphere pass.
- Distance haze is trueSKY's (Rayleigh/Mie in `SimulWeather` keyframes and the lighting
  table's `rayleigh`/`mie`, per km). We use the table's `(rayleigh + mie) / 1000` per metre. **low**

## Stars and moon disc

`Lighting >> starEmissivity` (25 on Altis), `moonObjectColorFull` (460, 440, 400),
`moonHaloObjectColorFull`. Star visibility by sun depression (−4° to −12°) and cloud alpha is
our choice. **low**

## Exposure hints

Each lighting entry carries `apertureMin/Standard/Max` and `standardAvgLum` (noon: 70/120/120,
8000; night: 4/4/8, 4). How the eye adaptation uses them is in the render-lighting topic.
