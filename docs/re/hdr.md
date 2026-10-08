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
| `minAperture` / `maxAperture` | 1e-5 / 256 | clamp of the adapted "aperture" | medium |
| `apertureRatioMin` / `apertureRatioMax` | 10 / 4 | unknown, probably limits of adaptation around a standard | low |
| `eyeAdaptFactorLight` | 3.3 | adaptation speed towards brighter scenes (1/s) | medium |
| `eyeAdaptFactorDark` | 0.75 | adaptation speed towards darker scenes (1/s) | medium |
| `bloomScale` etc. | 0.09, ... | bloom (not implemented yet) | — |

`DefaultWorld` uses `tonemapMethod = 2`, `tonemapExposureBias = 2.0`, B = 0.3, E = 0.01,
F = 0.3 (Hable's published constants), adapt 0.6 / 0.2. `VR` uses a very different curve
(A 6.8, B 0.8, C 0.8, D 0.6, E 2.35, F 8.0, W 4.0). `Enoch` adapts to dark at 0.25.

Hable's curve: `f(x) = (x(Ax + CB) + DE) / (x(Ax + B) + DF) - E/F`, output
`f(bias * x) / f(W)`.

## Aperture

Weapons and optics configure `apertureMin`, `apertureStandard`, `apertureMax` (and the script
commands `setAperture`, `setApertureNew [min, std, max, stdLum]` exist). Larger aperture values
darken the image, i.e. the aperture behaves like the scene luminance the eye is adapted to
_(medium confidence; from `setAperture` behaviour as documented for scripting)_. We model the
adapted luminance as the aperture, clamped to `[minAperture, maxAperture]`, and expose with
`key / aperture`. The `key` (target mid-tone) is our own choice until RE shows RV's mapping.

## Anti-aliasing

The game ships `readme_fxaa.txt` and `readme_smaa.txt` (licence texts) and the exe contains
`FXAA`/`SMAA` strings: the PC version offers both as post-process AA. We implement an FXAA-style
filter first.

## Open questions

- ~~Exact enum order of `tonemapMethod`~~ resolved in `render-atmosphere.md` §3.1 (0 none, 1 filmic, 2 Reinhard; `DefaultWorld` therefore uses Reinhard with `tonemapLinearWhiteReinhard` as W).
- How average luminance is measured (histogram vs. downsampled mean, which percentiles).
- Meaning of `apertureRatioMin/Max`; interaction with `apertureStandard` of optics.
- Units of scene luminance (RV's sun and sky intensities in config `CfgWorlds >> LightingNew`).
