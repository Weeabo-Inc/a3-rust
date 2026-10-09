# Lighting tables, fog/haze and HDR (atmosphere)

How Arma 3 2.22 turns `CfgWorlds >> <world> >> Weather >> LightingNew` into light colours, how the object
shaders apply fog, haze and underwater fog, and the HDR chain (tonemap, bloom, luminance measurement,
eye adaptation). This page corrects and extends `hdr.md`.

**Sources:**
- Engine code (`arma3_x64.exe`, RVAs given).
- Compiled shaders from `Dta\bin.pbo` (`Shaders_5_0_PS.shdc`, `Shaders_5_0_PP.shdc`), disassembled
  with `tools/re/shdc.py`.
- The resolved Altis config.

Shader formulas are transcribed from the disassembly (high confidence). Where the text maps
a constant back to config or script values, it gives a confidence level.

## 1. LightingNew table (CPU, high)

### 1.1 Colour notation

Parser `0x1411fc520`, used for every colour entry:

| config form | value |
|---|---|
| `{{r,g,b}, ev}` | `rgb · 2^ev / (0.299r + 0.587g + 0.114b)`. The colour is normalised to luma 1, then scaled by 2^ev (EV notation). |
| `{r,g,b}` | as given, alpha 1 |
| `{r,g,b,a}` | as given |

Example (Altis `Lighting0`): `diffuse[] = {{0.14,0.19,0.3}, 4}` is (0.14,0.19,0.3) / 0.1878 · 16
= (11.93, 16.19, 25.56).

### 1.2 Entry fields

Loader `0x141616b50` / `0x141616e20`. Each `LightingN` class holds:

| field | notes |
|---|---|
| `height`, `overcast`, `sunAngle` (degrees, stored as `sin(sunAngle)`), `sunOrMoon` | lookup keys and sun/moon blend |
| `diffuse`, `diffuseCloud` | sun colour, clear and overcast |
| `ambient`, `ambientCloud` | sky ambient (→ `AE`) |
| `ambientMid`, `ambientMidCloud` | horizon ambient. **If missing: (ambient + groundReflection)/2** |
| `groundReflection`, `groundReflectionCloud` | ground bounce (→ `GE`) |
| `bidirect`, `bidirectCloud` | back light (likely → `LDirectionGround_DiffuseBack`) |
| `sky`, `skyAroundSun`, `fogColor` (optional) | sky and fog colours |
| `desiredLuminanceCoef[Cloud]`, `luminanceRectCoef[Cloud]` | exposure control |
| `apertureMin`, `apertureStandard`, `apertureMax`, `standardAvgLum` | eye adaptation, see §3.3 |
| `rayleigh[]`, `mie[]`, `cloudsColor`, `swBrightness` | sky and cloud scattering (Simul weather) |

### 1.3 Lookup

`0x141615910` → `0x1416153d0` → `0x141615640`. The table is grouped by `height`, then `overcast`,
then `sin(sunAngle)`, and the lookup is **trilinear**:
- For each key, find the first entry with a key greater than the query.
- Interpolate linearly between it and the previous entry (t clamped to [0,1]).
- If `t < 0.0001` or `t > 0.9999`, take the entry directly.

All colour fields are interpolated componentwise (`0x141616190`).

Then `0x141615b80` blends each `X`/`XCloud` pair with a cloud factor `c`: `X·(1-c) + XCloud·c`. This
covers diffuse, ambient, ambientMid, groundReflection, bidirect, desiredLuminanceCoef and
luminanceRectCoef. Sky, skyAroundSun, fogColor, apertures, rayleigh, mie and cloudsColor are copied
as they are. It also outputs `sunColourForSky = max(diffuse_clear, skyAroundSun)`
(componentwise). Where `c` comes from, and how sunOrMoon mixes in the moon, is not traced yet; `c`
is likely overcast-derived.

The mapping of these outputs to the shader constants (`PSC_AE`, `PSC_GE`, `PSC_AmbientMid`,
`PSC_Diffuse`, `PSC_LDirectionGround_DiffuseBack`) follows from the names but has not been traced
through the CPU. See `render-materials.md` §3 for how the shaders use them.

## 2. Fog, haze and water fog in object shaders (shader)

All object pixel shaders end with the same block. They switch on `PSC_FogMode` (cb5[11]):
- 0: no fog.
- 1 or 3: full fog. The output colour becomes a blend of scene, air fog and water fog.
- 2: transmittance only. The output is multiplied by it (for additive passes).

Constants:

| constant | contents |
|---|---|
| `PSC_FogEnd_RCPFogEndMinusFogStart_WaterHeight_ExpCoef` (cb5[12]) | `x` fogEnd, `y` 1/(fogEnd−fogStart), `z` water height, `w` water exp coefficient |
| `PSC_CamUnderwater` (cb5[13]) | `.y` offset added to the water height |
| `PSC_PhysicalFog` (cb5[10]) | `x` height falloff, `y` density |
| `PSC_HazePars` (cb0[14]) | `x` base height, `y` density, `z` height falloff |
| `PSC_CameraWorldPosition` (cb6[9]) | camera position |
| `PSC_FogColor` (cb0[1]) | air fog colour |
| `PSC_WaterFogColor` (cb0[7]) | water fog colour |
| `PSC_WaterFogGradientCoefs` (cb0[17]) | water fog gradient |

`P` = pixel world position (`COLOR1.xyz`), `C` = camera position.

```
hw   = waterHeight + CamUnderwater.y
d    = |C - P|
// split the view ray into an underwater part du and an air part da
if C.y > hw:  s = saturate((hw - C.y) / (max(P.y - C.y, 0) + 1e-5))
              du = d*s;  da = d - du
else:         s = saturate((C.y - hw) / (max(C.y - P.y, 0) + 1e-5))
              da = d*s;  du = d - da;  (the height terms below use max(hw, P.y) and C.y)
// exponential height fog integrated along the air part of the ray
expInt(a, L) = (a == 0) ? L : (1 - exp(-a*L)) / a
if da > 0:
    dy = P.y - hw
    Tphys = min(exp(-PhysicalFog.y * exp(-PhysicalFog.x * min(hw, P.y))
                    * expInt(|dy|/(da+1e-5) * PhysicalFog.x, da')), 1)
    Thaze = min(exp(-HazePars.y * exp(-HazePars.z * (hw - HazePars.x))
                    * expInt(dy/(da+1e-5) * HazePars.z, da')), 1)
    Tair  = Tphys * Thaze            // da' = (1-s)*d, the in-air length
else Tair = 1
Twater = min(exp(-du * ExpCoef), 1)
Lin    = saturate((fogEnd - da) * RCPFogEndMinusFogStart)      // classic linear fog on top
Ta     = Tair * Lin
// water fog colour depends on view direction (vy = normalize(P - C).y)
g = vy < 0 ? WFG.x + (1+vy)² * (WFG.y - WFG.x) : WFG.y + vy * (WFG.z - WFG.y)
airFog   = FogColor.rgb * alpha;   waterFog = g * WaterFogColor.rgb * alpha
camera above water: out = scene*Twater*Ta + airFog*Twater*(1-Ta) + waterFog*(1-Twater)
camera below water: out = scene*Ta*Twater + airFog*(1-Ta)     + waterFog*Ta*(1-Twater)
FogMode 2:          out = scene * Ta * Twater
```
The fog colour terms are multiplied by `alpha` because the output is premultiplied.

**CPU side (medium):**
- `PSC_HazePars` is set in `0x1416f5ff0` from the weather object (`Landscape+0xe80`, fields
  +0x60/+0x64/+0x68). With fog disabled it is (0, 1e-5, 1e-5); a second preset uses 2e-5. This is
  the `setFog [value, decay, base]` fog: `x` = base, `y` = density (beta), `z` = decay. The config
  ranges are `fogBeta0Min/Max` (Altis 0…0.05) and `startFogDecay` (0.014).
- `PSC_PhysicalFog` is set per view in `0x141715a80` from per-camera state; its source is not traced.
- Altis water fog config: `fogDensity 0.07`, `fogColor`, `fogGradientCoefs {0.35,1,1.7}`,
  `fogColorExtinctionSpeed`.

TreeAdv and TreeAdvTrunk compute the same fog inline.

## 3. HDR chain (post-process shaders + CPU)

### 3.1 `tonemapMethod` (high; corrects hdr.md)

Config loader `0x141031ec0` stores `HDRNewPars` in globals `0x1420bb240…`. Shader table
`0x1417643e0` has `+0x568 None`, `+0x570 Filmic`, `+0x578 Reinhard`, indexed by method:

| value | shader | used by |
|---|---|---|
| 0 | `PSPostProcessGlowNewFinalNone` | |
| 1 | `PSPostProcessGlowNewFinalFilmic` (Hable) | Altis, Stratis, Tanoa, Malden |
| 2 | `PSPostProcessGlowNewFinalReinhard` | `DefaultWorld` |

There is **no ACES shader**. Night variants (`…FinalNight*`) and `…FinalNVG` exist for NVG/night.

### 3.2 Final pass (shader)

Constants:
- `PSC_BloomPars` = (`bloomImageScale`, `bloomScale`, 1, `bloomExponent`). Set in `0x14175ac10`.
- `PSC_TonemapPars[0]` = (A, B, C, D) = (shoulderStrength, linearStrength, linearAngle, toeStrength).
- `PSC_TonemapPars[1]` = (E, F, W, bias) = (toeNumerator, toeDenominator, W, exposureBias). W is
  `tonemapLinearWhiteReinhard` when method = 2, otherwise `tonemapLinearWhite`.

```
scene = tex(t0).rgb * BloomPars.z
bloom = pow(tex(t1).rgb, BloomPars.w)
k     = (1 - saturate(2 * dot(scene, (0.299, 0.587, 0.114)))) * BloomPars.y
c     = scene * BloomPars.x + bloom * k                 // bloom fades out on bright pixels
// Filmic:
c    *= bias
h(x)  = (x(Ax + BC) + DE) / (x(Ax + B) + DF)
out   = saturate((h(c) - E/F) / (h(W) - E/F))
// Reinhard (on Rec.709 luminance, preserving hue):
L     = dot(c, (0.2126, 0.7152, 0.0722))
out   = saturate(c * (L * (1 + L/W²) / (1 + L)) / (L + 0.0001))
// None: out = c
o.rgb = pow(out, PSC_RgbEyeCoef.w);  o.a = 1
```
Note that `bias` applies only to the Filmic curve. Filmic and Reinhard apply no other exposure:
the HDR buffer is already exposed (§3.3). `PSC_RgbEyeCoef.w` is a final gamma; its CPU value is not
traced.

### 3.3 Luminance measurement and adaptation

**Measurement (shader):** `PSPostProcessGlowNewLuminanceInit` takes 4 taps of the scene. For each it
computes `ln(dot(rgb·s, (0.299,0.587,0.114)) + 0.001)` and averages them, with the result clamped
to ≤ 1637.6. `…LuminanceAvg2x/4x` then box-average down to 1×1. The result is the **log-average
(geometric mean) luminance**. `PSPostProcessDownSampleMaxAvgMinLuminance` also exists.

There is a separate compute path: `CSCalculateHistogramLum` builds a 512-bin histogram of
saturated Rec.709 luminance, and `CSComputeCDFFromHistogram` turns it into a CDF, capping bins at
40000 and smoothing over time by `CSC_HistogramPars.x`. This is a post-tonemap equalisation (likely
thermal/NVG), not the exposure meter.

**GPU adaptation (shader, `PSPostProcessAssumedLuminance`):** it keeps a 1×1 "assumed luminance" value
stored as `log2(v)·6.79556 + 0.50196` (8-bit friendly).

```
m      = max(0.9*min(t0.x,1000) + 0.2*min(t0.y,1000), 1e-4)  // measured (two channels of the downsample)
target = Pars1.z / m
if Pars1.x > 0:                                               // previous value available
    prev  = exp2((t1.y - 0.50196) * 0.147155) * Pars1.x
    r     = target / prev;  l = log2(r)
    r     = min(r, exp2(|l| < 1 ? l² : |l|))                  // soft step near the target
    r     = clamp(r, Pars2.z, Pars2.w)                        // per-frame change limits (adapt speed)
    target= r * prev
v = clamp(target, Pars2.x, Pars2.y)
```
`PSC_AssumedLuminancePars1/2` are set in `0x14173fda0`. Their exact CPU values are not traced.
They likely come from `minAperture`/`maxAperture` and from `eyeAdaptFactorLight/Dark` × frame time.

**CPU aperture (`0x14175ac10`, medium):**
- With `lum` = the measured luminance, the aperture is interpolated from the current lighting
  entry's (`apertureStandard` `Pstd`, `apertureMin` `Pmin`, `apertureMax` `Pmax`, `standardAvgLum`
  `Lstd`). These come from §1.3 or from `setApertureNew`; NVG uses the `nvgAperture*` values.
- `ratioMax` = `apertureRatioMax` and `ratioMin` = `apertureRatioMin`.

```
if Pstd <= 0 or Lstd <= 0: ap = 1
elif lum < Lstd:  x = Lstd/lum  →  ap = x <= 1 ? Pstd : x >= ratioMax ? Pmax
                                    : Pstd + (x-1)/(ratioMax-1) * (Pmax - Pstd)
else:             x = -Lstd/lum →  ap = x < -ratioMin ? Pmin : x > 1 ? Pstd
                                    : Pmin + (x+ratioMin)/(1+ratioMin) * (Pstd - Pmin)
ap = clamp(ap, minAperture, maxAperture)
exposure = 1/ap²   then smoothed toward the previous value with an exp(-k·|log2 ratio|)-style factor
```
The branch structure above is literal. Some of the branch semantics (sign of `x`, which bound is
used) look odd and should be checked before relying on them. `hdr.md`'s statement that aperture acts
like adapted luminance and darkens the image is consistent with `exposure = 1/ap²`.

## 4. Sky

### 4.1 The dome and its textures (high)

The sky is a dome model, `CfgWorlds >> <world> >> skyObject`. Altis and Stratis share
`A3\Map_Stratis\data\obloha.p3d`: ODOL, one resolution LOD, 335 vertices, 490 faces, one section.
Its bbox is ±10.03 m wide and ±7.22 m tall, so the mesh is a squashed hemisphere. Its UVs depend
on elevation only (constant in `u`, and no two elevations share a `v`):

| elevation (deg) | `v` | elevation (deg) | `v` |
|---|---|---|---|
| −35.8 (skirt) | −0.052 | 41.01 | 0.707 |
| 19.64 | 0.000 | 47.97 | 0.809 |
| 22.74 | 0.156 | 56.35 | 0.891 |
| 26.28 | 0.309 | 66.30 | 0.951 |
| 30.39 | 0.454 | 77.69 | 0.988 |
| 35.23 | 0.588 | 89.95 | 1.000 |

Every elevation from the horizon up to 19.6° therefore has `v = 0` (the skirt), and `v = 1` is
the zenith. `u` ranges over `[-0.01, 1.10]` around the dome.

The dome's textures come from the world or, overriding it, from the overcast level
(`Weather >> Overcast >> WeatherN >> sky` / `horizon` / `skyR`):

| config | Altis/Stratis value | format | kind | size |
|---|---|---|---|---|
| `skyTexture` / overcast `sky` (clear) | `A3\Map_Stratis\Data\sky_semicloudy_sky.paa` / `sky_clear_gs.paa` | DXT5 / AI88 | `Sky` / `GreyScale` | 8×8 |
| `skyTextureR` / overcast `skyR` | `sky_semicloudy_lco.paa` / `sky_clear_lco.paa` | DXT1 | `LayerColor` | 512×512 |

A `Sky` texture carries its colour in R, B and (inverted) A, and its alpha in the inverted green
channel: its PAA swizzle is `A<-InvertedGreen R<-Red G<-InvertedAlpha B<-Blue`. The shipped 8×8
ramps are constant along `u`; the semicloudy one runs from `(13,22,60)` at the zenith to
`(57,81,132)` at the horizon (8-bit, read as linear light), and the grey clear-weather one from
127 to 255. So the horizon end of the ramp is both **brighter and less saturated** than the
zenith end.

### 4.2 `PSHorizon` (shader, high)

`PSHorizon` (`61F433D2`, `Dta\bin.pbo` → `Shaders_5_0_PS.shdc`) shades the dome. With `t0` the
`skyTexture`, `t1` the `skyTextureR`, `v1` TEXCOORD6 and `D` the view direction:

```
c     = lerp(t1, t0, v1.w)
a     = c.g                                  // the swizzled alpha, see §4.1
up    = saturate(D.y)
cosl  = dot(LDirectionTransformed, D)
glow  = (0.75 * (1 + cosl))³ * (1 - up)⁴ * a * (1 - a²)
sky   = v1.xyz * (c.r, 1 - c.a, c.b) + PSC_Diffuse * glow
```

and then the fog/haze/water block of §2. So the dome's colour is **the sky texture times a
per-vertex tint**, plus a glow that follows the light direction and fades out towards the zenith
(`(1-up)⁴`). The glow is scaled by `PSC_Diffuse`, the sun's own light colour and level.

`v1.xyz` (the tint) and `v1.w` (the `sky`/`skyR` blend) are produced by the dome's vertex shader;
what the CPU sets them to is **not traced**. Everything above the tint is.

### 4.3 What the render oracle measures (high)

On the clear shots (sun 61–78° up, `Lighting11/12`), the mean linear RGB of Arma's sky, by
elevation, against ours (filmic inverted to scene light, `docs/fidelity/render-oracle.md`):

| elevation | Arma | ours (before #295) |
|---|---|---|
| ~4° (horizon) | matches ours | — |
| 27–35° | (0.46, 0.76, 2.08) | (2.02, 4.30, 8.70) |
| 30–43° | (0.51, 0.86, 2.71) | (2.20, 4.74, 9.95) |

Arma's sky is 4× dimmer than ours above 25° and falls to **0.27 of the horizon's luminance**
between 4° and 35°, while ours falls only to 0.67. The shipped ramp over the dome's UV (§4.1)
has a zenith of **0.28** of the horizon's luminance for the same range — the dome's own texture
already carries the gradient we were missing, with the hue shift from its own rows (its
blue-over-red runs 1.0 at the horizon to 2.0 at the zenith).

Two further checks of the same data:
- The horizon's hue in Arma is `(0.3, 0.44, 0.74)` of luma 1 — exactly the lighting table's
  `fogColor`. The zenith's is bluer, `(1 : 1.75 : 4.75)`, which the ramp's own ratio reproduces.
- The table's `sky` value is *not* the rendered zenith. At `Lighting11/12` it is
  `{{0.02, 0.12, 0.8}, 13.8}` = (1702, 10211, 68074), a hue of `1 : 6 : 40` and a luma of 0.44 of
  `fogColor`'s — no exponent can bring that down to the measured 0.27, and no sky is that
  saturated. It is the sky's colour for the Simul model and for reflections, not the dome's
  radiance.

### 4.4 The Simul keyframes (medium)

`CfgWorlds >> <world> >> SimulWeather` holds the trueSKY atmosphere: `DefaultKeyframe` and the
per-overcast `Overcast >> WeatherN` keyframes give `rayleigh[]`, `mie[]`, `haze`, `hazeBaseKm`,
`hazeScaleKm`, `hazeEccentricity`, `brightnessAdjustment`, `cloudiness`, `cloudBaseKm`,
`cloudHeightKm`, `directLight`, `indirectLight`, `ambientLight`, `extinction`, `diffusivity` and
the noise parameters; `fadeNumAltitudes/Elevations/Distances` size its lookup grid, and
`CfgWorlds >> swBrightness` scales it. `PSSimulWeatherClouds` (disassembled) shows the keyframe
reaching the shaders as `PSC_SimulWeatherPars[8]` in `PSCB_NonFrequent` cb0[18..25]: `cb0[18].x`
is the Henyey-Greenstein asymmetry (`mieAsymmetry = 0.5087`), `cb0[25]` the light direction.

The lighting table's own `rayleigh[]` and `mie[]` vary per entry (Altis: `rayleigh` R is always
0.007, G/B run 0.0139/0.035 at high sun, 0.038/0.0675 around 0–2°, 0.018/0.04 at 12°;
`mie[] = {0.005}` throughout). The product `rayleigh ⊙ diffuse` reproduces the *hue* of the
measured clear-sky zenith and horizon to within a few per cent, which is what single scattering
predicts; its absolute scale is **not traced**, so our sky does not use it yet.

### 4.5 What we do now

The sky stays a gradient from the lighting table — `mix(fogColor, sky, saturate(dir.y)^0.45)`,
plus the sun glow, the cloud layer, the moon and the stars — and the World's `skyTexture` is
applied on top as the dome's ramp: the texture's `v` axis at the elevation the dome's UV table
(§4.1) gives, divided by its value at the horizon, so the horizon is unchanged and the zenith
keeps the shipped ramp's ratio and hue. That part is engine data and engine structure; the
gradient under it, the `0.45` exponent, the glow, the clouds, the moon and the stars are ours.
`sky[]` is still the gradient's zenith colour, which §4.3 shows is wrong — replacing the tint
needs `v1.xyz`, which is the next thing to trace.


## 5. Open points

- Where the cloud factor `c` comes from, sun/moon blending, and the CPU mapping of the lighting outputs
  to `PSC_*` constants (including any division by the aperture).
- `PSC_RgbEyeCoef.w` (gamma), `PSC_AssumedLuminancePars1/2` values, and `PSC_PhysicalFog` source.
- The sky dome's per-vertex tint `v1.xyz` and its `sky`/`skyR` blend `v1.w` (§4.2): which lighting
  table colour drives them, and whether the tint varies over the dome. This is what stands between
  our gradient and the engine's.
- The absolute scale of the Simul atmosphere (§4.4): the sky's radiance in the engine's light units,
  and how `swBrightness` and `brightnessAdjustment` enter it.
- The Simul sky/cloud model at the horizon: a plane-parallel single-scattering integral over the
  traced `rayleigh`/`mie` reproduces the *shape* of `PSHorizon`'s glow but is far too dark and too
  saturated at the horizon, so the engine is doing something more (multiple scattering, the
  `fadeNum*` lookup grid, or the horizon band's own texture).

