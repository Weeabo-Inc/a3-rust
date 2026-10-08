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

The sky and clouds use the Simul weather shaders (`VSSimulWeatherClouds`, `PSSimulWeatherClouds*`,
`PSPostProcessSimulWeather*`), driven by the `rayleigh`, `mie`, `cloudsColor`, `sky`, `skyAroundSun`
and `swBrightness` lighting fields. `PSHorizon`, `PSCloud` and `VSStar`/`PSPoint` handle horizon,
legacy clouds and stars. These shaders have not been decoded. A first renderer can shade a sky
gradient from `sky` (zenith) to `skyAroundSun` (around the sun), with `fogColor` at the horizon.

## 5. Open points

- Where the cloud factor `c` comes from, sun/moon blending, and the CPU mapping of the lighting outputs
  to `PSC_*` constants (including any division by the aperture).
- `PSC_RgbEyeCoef.w` (gamma), `PSC_AssumedLuminancePars1/2` values, and `PSC_PhysicalFog` source.
- Simul sky model.
