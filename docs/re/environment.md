# Environment: date, sun and moon, lighting tables, overcast, fog

How the engine turns a World's config and the current date, weather and fog into light.
Implemented in `crates/a3-environment`. Addresses are RVAs in `arma3_x64.exe` (build 2.22).
Lighting/tonemap/atmosphere shader maths: see `render-materials.md` and the render-lighting topic.

## Sun and moon position

The engine has its **own** sun and moon model, and it is the one that lights the scene. An
earlier revision of this page said the directions came from trueSKY and that we therefore used
real ephemerides; the oracle's own client disproves it (below), and the consequence was a night
with no moonlight at all (#293).

- The world config is read at `0x1155750` into the Landscape object: `latitude` (default -40)
  and `longitude` (default 15) in radians at +0x2e34 / +0x2e38, `elevationOffset`. **high**
- **`0x10813c0` builds the sun and moon directions** from rotation matrices, and it is called
  unconditionally by `0x10851a0`, the function that produces the light the renderer shades with.
  `0x10851a0` also has a trueSKY branch, but it only fills the directions itself when the Simul
  object and its data are present, and it passes the builder *null* direction outputs in that
  case; which branch runs in the shipped client is settled by the oracle, not by the code. **high
  for the call chain, medium for which branch wins**
  - helpers, each decompiled: `0x35c410(m, a)` is `RotX(−a)`, `0x35c560(m, a)` is `RotY(a)`,
    `0x35c6b0(m, a)` is `RotZ(a)`, `0x35bc40` transposes, `0x35acb0` normalises. **high**
  - the chain: `A = RotY(2π·t)` daily (`t` the day fraction), `B = RotY(2π·(f − 0.030136986))`
    yearly with `f = (day_of_year_0based + t)/365`, `G = RotX(−0.40142)` (23° tilt),
    `L = RotX(−latitude)`; `D = B·A·G`, `F = L·D`, `E = Fᵀ·RotX(−π/2)`. **high**
  - the sun is `E` applied to `B`'s third row; the moon is `E` applied to `M`'s third row carried
    through `RotZ(5°)`, where `M = RotY((year + f − 1985.0082)·81.90581 + π)`. The x and z
    components are negated in both, and the engine's `(east, north, up)` frame is the model's
    negated as a whole. **high**
  - the config's `latitude = -35.152` (Altis) means 35.152° N and the clock is the world's local
    time; `longitude` and the time zone do not enter this builder. **high**
- The **moon phase** (`0x1080fc0`, SQF `moonPhase` at `0x4a6d70`): `x = acos(−dot(Ls, Lm))/(2π) +
  0.5` with `L` the same builder's two directions, `moonPhase = 2 − 2x` = sun-moon elongation / π
  (0 new, 1 full). The SQF command hands the builder a time of day of zero, so `moonPhase` is the
  same all day; the renderer passes the real hour. **high**
- The **oracle says the legacy model is what the client shows.** Its scenario logs
  `moonPhase date`, `sunOrMoon` and `date` for every shot:
  - `2035-06-06 23:30` → `moonPhase=0.704879 sunOrMoon=0`, and the night capture is moonlit: its
    ground is about five times our moonless ground. The real Moon that night is **new**
    (elongation 11°, 34° below the horizon), which is what our ephemeris gave us.
  - `2035-06-24` (all shots) → `moonPhase=0.107916`, identical at 12:00 and 19:50: the hour does
    not enter the command, as above.
  - A port of the chain above reproduces both to 0.0003. Its sun is within 2° of the real
    ephemeris at noon over Altis (76.5° against 78.2°) and 6-9° in azimuth, so the day shots keep
    their shadows. **high**
- `a3-environment` implements this model (`celestial.rs`, `legacy_directions`). The NOAA and
  Meeus ephemerides it used before are gone; `0x1670830`/`0x10851a0`'s trueSKY plumbing is not
  implemented. **high**
- The **light direction** used for shading is the sun when the table's `sunOrMoon` ≥ 0.5, else
  the moon. A copy used for shadows has its height forced to at least 0.4 before
  renormalising (`0x10851a0`, `if (-0.4 < y) y = -0.4` on the light-travel vector). **high**
- Calendar: days per month with Gregorian leap years (`0x1088ab0`); index 0 is January and only
  index 1 takes the leap day. **high**


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
