# HDR, eye adaptation and tonemapping

What the shipped data says about RV's HDR pipeline, as used by `a3-render`'s post chain.

Sources: the merged config of build 2.22 (`a3-tools config dump`), strings in
`arma3_x64.exe`, and the `readme_fxaa.txt` / `readme_smaa.txt` licence files shipped in the game
folder. No decompilation was used.

## Config: `CfgWorlds >> <world> >> HDRNewPars`

Per-world HDR parameters. Values for `Altis`, `Stratis`, `Tanoa`, `Malden` and `CAWorld`
(identical):

| entry | value | meaning (our reading) | confidence |
| ----- | ----- | --------------------- | ---------- |
| `tonemapMethod` | 1 | curve selector: 0 = none, 1 = filmic (Hable), 2 = Reinhard (see `render-atmosphere.md` §3) | high: shader table in the exe; there is no ACES shader |
| `tonemapShoulderStrength` | 0.22 | Hable A | high (names match Hable's parameters one to one) |
| `tonemapLinearStrength` | 0.12 | Hable B | high |
| `tonemapLinearAngle` | 0.1 | Hable C | high |
| `tonemapToeStrength` | 0.2 | Hable D | high |
| `tonemapToeNumerator` | 0.022 | Hable E | high |
| `tonemapToeDenominator` | 0.2 | Hable F | high |
| `tonemapLinearWhite` | 11.2 | Hable W (white point) | high |
| `tonemapExposureBias` | 1.0 | multiplier before the curve | high |
| `tonemapLinearWhiteReinhard` | 2.5 | white point of extended Reinhard | medium |
| `minAperture` / `maxAperture` | 1e-5 / 256 | clamp of the CPU aperture (`render-atmosphere.md` §3.3) | high |
| `apertureRatioMin` / `apertureRatioMax` | 10 / 4 | how far below / above `standardAvgLum` the measured luminance must be for the aperture to reach `apertureMin` / `apertureMax` | high |
| `eyeAdaptFactorLight` | 3.3 | speed of the CPU aperture stage towards brighter scenes (scaled by `max(|log2 ratio|, 1)` and most likely the frame time) | high (frame time: medium) |
| `eyeAdaptFactorDark` | 0.75 | the same towards darker scenes | high (frame time: medium) |
| `bloomScale` etc. | 0.09, ... | bloom (not implemented yet) | — |

`DefaultWorld` uses `tonemapMethod = 2`, `tonemapExposureBias = 2.0`, B = 0.3, E = 0.01,
F = 0.3 (Hable's published constants), adapt 0.6 / 0.2. `VR` uses a very different curve
(A 6.8, B 0.8, C 0.8, D 0.6, E 2.35, F 8.0, W 4.0). `Enoch` adapts to dark at 0.25.

Hable's curve: `f(x) = (x(Ax + CB) + DE) / (x(Ax + B) + DF) - E/F`, output
`f(bias * x) / f(W)`.

## Aperture

`CfgWorlds >> <world> >> Weather >> LightingNew >> LightingN` gives each lighting condition an
aperture triple and the luminance it is calibrated for: `apertureMin` / `apertureStandard` /
`apertureMax` and `standardAvgLum` (Altis: 4/4/8 at `standardAvgLum` 4 for the night entry, up to
70/120/120 at 8000 at noon). Weapons and optics override them, and the script commands
`setAperture` / `setApertureNew [min, std, max, stdLum]` set them from scripts.

**The exposure the engine renders with is `1 / aperture²`** (_high_, see below). The aperture
itself is the CPU stage of `render-atmosphere.md` §3.3: it moves from `apertureMin` to
`apertureMax` around `apertureStandard` as the measured scene luminance moves from
`standardAvgLum / apertureRatioMin` to `standardAvgLum * apertureRatioMax`, and is clamped to
`[minAperture, maxAperture]` (1e-5 / 256). Larger apertures therefore darken the image, matching
the `setAperture` documentation.

### Measured against the real client (2026-10-09)

Oracle captures with `tools/oracle/render/render_oracle.py`'s scenario, one aperture override per
screenshot (script: `.work/hdr313/aperture_probe.py`; scratch, not committed):

| scene, `setApertureNew` | mean linear luminance | vs default |
| --- | --- | --- |
| Altis 2035-06-06 23:30, default | 0.05123 | 1.000 |
| the same, `[4, 4, 8, 4]` (= the Lighting0 entry) | 0.05125 | **1.000** |
| the same, `[2, 2, 4, 4]` (half the aperture) | 0.21231 | 4.144 |
| the same, `[8, 8, 16, 4]` (double) | 0.00601 | 0.117 |
| Altis 2035-06-24 19:15, default | 0.38413 | 1.000 |
| the same, `[6.8627186, 8.862719, 21.725437, 41.567963]` (the entry our engine samples there) | 0.38277 | **0.997** |
| the same, `[20, 25, 35, 250]` (Lighting9) | 0.11245 | 0.293 |
| the same, `[8, 16, 26, 100]` (Lighting8) | 0.33293 | 0.867 |

Three conclusions (_high_):

1. The default night aperture **is** the table's night entry (ratio 1.000), and the default
   sunset aperture **is** the twilight entry our own lighting lookup samples (0.997) — the config
   path and our lighting-table lookup agree with the client.
2. Halving every aperture makes the image 4.14x brighter, doubling makes it 8.5x darker — the
   exposure is `1 / aperture²`, and the eye adaptation does not cancel it in steady state (the
   probe lets each variant settle for 12 s).
3. Pushing the default capture through the inverse of the Altis filmic curve and scaling the
   pre-curve values by 4 (the exposure `[2,2,4,4]` asks for) predicts the capture to a mean
   luminance ratio of 0.993 and an MAE of 6.8/255; the 1/4 case predicts 0.900 and 29/255. So the
   total exposure is a pure scale in front of the curve, and `a3-render`'s curve matches RV's.

`night_stdlum_40` (`[4,4,8,4]` with `standardAvgLum` raised 10x) reproduced the default capture
(0.05126 vs 0.05123): with `apertureStandard == apertureMin` the dark branch saturates at
`apertureMin` and `standardAvgLum` only moves the anchor, as the formula above says.

## Anti-aliasing

The game ships `readme_fxaa.txt` and `readme_smaa.txt` (licence texts) and the exe contains
`FXAA`/`SMAA` strings: the PC version offers both as post-process AA. We implement an FXAA-style
filter first.

## Open questions

- ~~Exact enum order of `tonemapMethod`~~ resolved in `render-atmosphere.md` §3.1 (0 none, 1 filmic, 2 Reinhard; `DefaultWorld` therefore uses Reinhard with `tonemapLinearWhiteReinhard` as W).
- How average luminance is measured (histogram vs. downsampled mean, which percentiles). The
  engine's `PSPostProcessGlowNewLuminanceInit` averages `ln(luma)` (a geometric mean, which we
  implement), but `PSPostProcessAssumedLuminance` combines two channels of the downsampled
  luminance as `0.9·min(t0.x, 1000) + 0.2·min(t0.y, 1000)`, and
  `PSPostProcessDownSampleMaxAvgMinLuminance` suggests one of them is a max. On high-contrast
  scenes that mix reads higher than a pure geometric mean — the candidate explanation for the
  1.4–4x gap between our meter and the client's effective value at dusk and night (see
  `docs/fidelity/render-oracle.md`). Not traced (_medium_).
- ~~Meaning of `apertureRatioMin/Max`~~ resolved in `render-atmosphere.md` §3.3 (CPU aperture
  stage).
- Units of scene luminance (RV's sun and sky intensities in config `CfgWorlds >> LightingNew`).
  Our meter and the client's agree at noon (both ~1.2–1.3e4 for the Kavala view) but our scene is
  1.8x darker at 19:15 and 2.5x darker at 23:30, which no exposure inside the table's aperture
  range can recover (see the PR / `docs/fidelity/render-oracle.md`).
- The value of `engine+0x368` in `PSC_AssumedLuminancePars1.z` (`engine+0x368 · brightness ·
  0.5`), and whether the GPU's assumed-luminance stage adds anything in steady state. The client
  probe above says the *total* exposure is `1/ap²`, so that stage is at most a transient.

## Our implementation (`a3-render::post`)

Follows `render-atmosphere.md` §3:

- **Meter**: log-average luminance, `ln(Rec.601 luma + 0.001)` over 2x2-tap samples, averaged in
  two compute passes (no histogram). It reads the unexposed HDR buffer, so its value is absolute
  scene luminance, the quantity the engine's CPU aperture stage works on (the engine meters the
  exposed buffer and divides the read-back by the exposure; the `min(t0.x, 1000)` bound of
  `PSPostProcessAssumedLuminance` therefore does **not** apply to our meter — with it, every
  daylit frame would meter at 1000).
- **Aperture stage**: `HdrSettings::aperture_exposure` / `aperture` are RV's curve
  (`0x14175ac10`), fed per frame from the lighting entry's `apertureMin/Standard/Max`,
  `standardAvgLum` and the global `apertureRatioMin/Max`, `minAperture/maxAperture`. The exposure
  pass (`shaders/exposure.wgsl`) evaluates the same curve at the measured luminance and steps
  towards it with the soft step and the per-frame ratio limits (`PSC_AssumedLuminancePars2.zw`:
  halve every 0.5–1 s, double every 1–20 s depending on `HdrSettings::cpu_exposure`). The Rust and
  WGSL implementations are pinned together by `the_exposure_pass_applies_the_aperture_stage`
  (the settled exposure must equal the Rust curve at the read-back luminance).
  Lighting entries with a degenerate aperture stage fall back to the old
  `key / measured` step (`key = standardAvgLum / apertureStandard²`).
  The CPU side's own interpolation (`eyeAdaptFactorLight/Dark`, `k = max(|log2(e/prev)|, 1) ·
  rate · dt`) is not modelled separately: our single GPU stage carries both the curve and the
  smoothing.
- **Bloom**: mixed before the curve as decoded, `k = (1 - saturate(2·luma601)) · bloomScale`. The
  bloom image itself (quarter-resolution exposed scene, Gaussian blur) is our approximation; RV's
  bloom generation passes (`bloomLuminance*` entries) are not decoded yet.
- **Curves**: filmic with `bias` and the E/F-offset normalisation, Reinhard on Rec.709 luminance
  with `tonemapLinearWhiteReinhard`, method 0 passes through; then `pow(·, RgbEyeCoef.w)` with
  `final_gamma = 1` (RV sets `PSC_RgbEyeCoef.w = 1` always), then our sRGB encode for display.
