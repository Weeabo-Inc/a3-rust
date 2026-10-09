# Render oracle

The render oracle measures our renderer against the real Arma 3 2.22 client. It renders the same
shots in both. The camera, date, time, weather and view distance are the same. It then compares
the two images. The client runs only on a developer machine with the game installed. Game
screenshots and comparison images stay in `.work/oracle/` and are never committed.

Tool: [`tools/oracle/render/render_oracle.py`](../../tools/oracle/render/render_oracle.py)
(Python standard library only). Shot list:
[`tools/oracle/render/shots.json`](../../tools/oracle/render/shots.json). Metrics:
`a3-tools image-diff`.

## How to run

```sh
cargo build --release -p arma3 -p a3-tools
python -I tools/oracle/render/render_oracle.py all --game-dir P:\a3-rust\oirignal
# or one step / some shots:
python -I tools/oracle/render/render_oracle.py client --only altis_kavala_noon
python -I tools/oracle/render/render_oracle.py ours
python -I tools/oracle/render/render_oracle.py compare
```

- Output goes to `<main checkout>/.work/oracle/render/<shot>/`: `arma.png`, `ours.png`,
  `side.png` (Arma on the left, ours on the right, half size), `diff.png` (half size, red where
  ours is brighter, blue where ours is darker, ×4), `metrics.json` and `ours.log`.
  `report.md` holds the metric table.
- Steam must be running and logged in. A game window opens. Do not render anything else on the
  GPU during a client run: with GPU contention, `forceWeatherChange` stalls the client for
  minutes (see below).
- A client run takes about 1–2 min per World plus about 20 s per shot. Our side takes about
  40 s per shot, most of it game data and terrain loading.

## Client setup

The client is `arma3_x64.exe`. It never uses the user's `Documents\Arma 3` settings.

```
arma3_x64.exe -cfg=<work>\client\Arma3.cfg -profiles=<work>\client\profiles -name=a3rust_oracle
  -skipIntro -noSplash -noLauncher -window -nosound -world=empty
  "-init=playScriptedMission ['Altis', {<scenario>}, missionConfigFile, true]"
```

- **Starting a scenario:** `-init` runs its code once in the main menu. `playScriptedMission`
  loads the World and runs the inline code in an empty mission, so no mission file or PBO is
  needed. The third argument must be a Config: `nil` fails with "expected Config entry", so the
  tool passes `missionConfigFile`. The code uses single quotes only, because it travels inside
  one command-line argument. One client run per World.
- **Screenshots:** `screenshot "<name>.png"` writes to
  `<profiles>\Users\a3rust_oracle\Screenshots\`. With `-profiles` it works. A wiki community note
  says that `screenshot` fails silently with `-profiles`; with 2.22 this is not true.
  `HDRPrecision=16` is set, as `screenshot` requires.
- **Video settings (`Arma3.cfg`):** 1600×900 windowed and `HDRPrecision=16`. AA is off
  (`PPAA=0`, `multiSampleCount=1`, `AToC=0`). SSAO, DOF, rotation and radial blur, sharpen,
  caustics and `ppHaze` are off. Bloom is on (`ppBloom=1`). `cloudsQuality=3`. Screen-space
  water reflections and PiP are off.
- **Video settings (profile):** `sceneComplexity=1800000` (Ultra; ours `--objects-quality
  Ultra`), `terrainGrid=3.125`, `viewDistance=3000`, `preferredObjectViewDistance=2000`,
  `shadowZDistance=100`, `shadowQuality=3`, `textureQuality=3`, `fovTop=0.75`,
  `fovLeft=0.75·aspect`, `gamma=brightness=1`.
- **Window size:** 1600×900 keeps the window inside a 1080p desktop. The client renders at its
  window's client size. At 1920×1080, Windows sometimes shrank the window to fit the work area,
  and the game then wrote 1920×1040 screenshots. The tool marks each shot whose size differs
  from the requested size.
- **Per shot**, the scenario does the steps below:
  - It sets `setViewDistance`, `setObjectViewDistance`, rain 0, lightning 0 and the overcast.
    It runs `forceWeatherChange` only when the overcast changes, because each call stalls the
    game, and the shot with a different overcast comes last.
  - It sets `setFog [value, decay, base]` and `setDate`.
  - It places a `camera` with `cameraEffect ['internal','back']` at
    `max(getTerrainHeightASL, 0) + altitude`, with `setVectorDirAndUp` from heading and
    pitch, and `camSetFov`.
  - It waits for `camPreloaded` (60 s maximum) and 10 s more, sets the date and fog again,
    writes a `diag_log` line and calls `screenshot`.
  - Wind, gusts and waves are 0. `enableEnvironment [false, false]` is set.
- **Stopping:** the tool stops the process when the last screenshot is complete. It also
  deletes the freeze minidumps (`*.mdmp`, about 40 MB each) that the watchdog writes during
  weather changes.

## Our side

```
arma3 --world altis --camera east,north,alt,heading,pitch --fov <fovTop> --date y-m-d --time hh:mm
  --overcast o --fog v,d,b --fog-distance <viewDistance> --view-distance <objectViewDistance>
  --objects-quality Ultra --width 1600 --height 900 --no-overlay --frames 120 --screenshot ours.png
```

`--fov` (RV `fovTop`) was added for the oracle. `--no-overlay` leaves out the debug text. Our camera
altitude is already measured from `max(terrain, sea level)`, the same as the scenario.

The SQF server oracle (`tools/oracle/oracle.py`) needs a mission folder for the dedicated server.
The client oracle needs none, because `playScriptedMission` starts the scenario.

### Camera FOV: `camSetFov` against `fovTop` (measured)

A `camSetFov f` camera renders with **`fovTop = f · 0.75`**, and `fovLeft` follows the aspect.
The 0.75 is the profile's `fovTop`, so the factor is probably that value. The profile was not
changed to confirm this. Measured on `altis_kavala_noon`:
- At 16:9, with `--fov 0.75` our image is the client's image scaled down by 4/3 about the
  centre. Two features each gave a ratio of 1.31–1.37.
- At 4:3 (1200×900) the factor is the same, so the vertical tangent is not tied to a 4:3
  horizontal.
- With `fovTop = 0.5625` both aspects line up to within a few pixels.

The tool converts `shots.json`'s `fov` (the `camSetFov` value) with this rule.

## Metrics

`a3-tools image-diff <arma.png> <ours.png> --out <dir> --json`, on 8-bit sRGB images of the same
size:

| metric | meaning |
|---|---|
| MAE lin | mean absolute error in linear light, averaged over R, G, B (0 = same, 1 = black against white) |
| SSIM | mean SSIM of sRGB Rec.709 luma over 8×8 windows, stride 4 (1 = same structure) |
| lum Arma / ours | mean linear Rec.709 luminance of each image |
| lum ratio | ours / Arma (exposure: > 1 ours brighter) |
| hist dist | earth mover's distance between the 64-bin sRGB luma histograms (0..1) |
| bands | mean linear RGB of the top, middle and bottom third (in `metrics.json`; sky against ground) |

## Results

Baseline: run on 2026-10-09 against `main` at `2418a9c` (Arma 3 2.22.0.154103, 1600×900, AMD RX 6600).

| shot | MAE lin | SSIM | lum Arma | lum ours | lum ratio | hist dist | MAE R/G/B (linear) |
|---|---|---|---|---|---|---|---|
| altis_kavala_noon | 0.131 | 0.770 | 0.593 | 0.673 | 1.136 | 0.050 | 0.111/0.124/0.158 |
| altis_kavala_sunset | 0.141 | 0.642 | 0.384 | 0.283 | 0.736 | 0.131 | 0.160/0.140/0.124 |
| altis_hills_ground | 0.122 | 0.547 | 0.535 | 0.616 | 1.150 | 0.053 | 0.131/0.113/0.123 |
| stratis_coast | 0.223 | 0.683 | 0.492 | 0.715 | 1.453 | 0.151 | 0.175/0.245/0.249 |
| altis_building_close | 0.118 | 0.708 | 0.591 | 0.651 | 1.101 | 0.054 | 0.129/0.125/0.098 |
| altis_vegetation_close | 0.143 | 0.661 | 0.472 | 0.536 | 1.135 | 0.109 | 0.155/0.165/0.109 |
| altis_kavala_night_moon | 0.048 | 0.308 | 0.051 | 0.009 | 0.180 | 0.119 | 0.038/0.046/0.060 |
| altis_kavala_overcast_fog | 0.084 | 0.952 | 0.708 | 0.615 | 0.869 | 0.057 | 0.117/0.091/0.045 |

After #290 (the lighting table's hemisphere ambient in the model, terrain and road shaders),
with the same client images:

| shot | MAE lin | SSIM | lum Arma | lum ours | lum ratio | hist dist |
|---|---|---|---|---|---|---|
| altis_kavala_noon | 0.126 | 0.773 | 0.593 | 0.690 | 1.165 | 0.061 |
| altis_kavala_sunset | 0.135 | 0.716 | 0.384 | 0.274 | 0.713 | 0.119 |
| altis_hills_ground | 0.118 | 0.549 | 0.535 | 0.627 | 1.170 | 0.058 |
| stratis_coast | 0.222 | 0.683 | 0.492 | 0.719 | 1.460 | 0.154 |
| altis_building_close | 0.108 | 0.719 | 0.591 | 0.670 | 1.134 | 0.056 |
| altis_vegetation_close | 0.139 | 0.677 | 0.472 | 0.546 | 1.155 | 0.096 |
| altis_kavala_night_moon | 0.047 | 0.322 | 0.051 | 0.010 | 0.192 | 0.113 |
| altis_kavala_overcast_fog | 0.067 | 0.957 | 0.708 | 0.639 | 0.903 | 0.041 |

The MAE goes down on every shot. The largest changes: overcast fog 0.084 → 0.067, building
0.118 → 0.108, sunset SSIM 0.642 → 0.716. The blue tree trunks and the blue shaded walls are
gone, and roads keep their ambient at low sun. The terrain hue is closer to Arma's: the red/blue
ratio of the bottom third in `altis_hills_ground` is 1.59 for Arma, 1.29 before and 1.48 after.
Daylight exposure goes up by about 2 % (#296).
Most informative side-by-sides: `stratis_coast` (water), `altis_vegetation_close` (blue tree
trunks, missing grass) and `altis_kavala_night_moon` (night exposure, no lamps).

After #295 (the sky dome's elevation ramp from the world's `skyTexture`), with the same client
images:

| shot | MAE lin | SSIM | lum Arma | lum ours | lum ratio | hist dist |
|---|---|---|---|---|---|---|
| altis_kavala_noon | 0.126 | 0.773 | 0.593 | 0.690 | 1.165 | 0.061 |
| altis_kavala_sunset | 0.135 | 0.716 | 0.384 | 0.274 | 0.714 | 0.119 |
| altis_hills_ground | 0.117 | 0.549 | 0.535 | 0.626 | 1.169 | 0.058 |
| stratis_coast | 0.221 | 0.683 | 0.492 | 0.718 | 1.458 | 0.153 |
| altis_building_close | 0.093 | 0.720 | 0.591 | 0.651 | 1.102 | 0.045 |
| altis_vegetation_close | 0.120 | 0.678 | 0.472 | 0.498 | 1.054 | 0.073 |
| altis_kavala_night_moon | 0.047 | 0.322 | 0.051 | 0.010 | 0.192 | 0.113 |
| altis_kavala_overcast_fog | 0.067 | 0.957 | 0.708 | 0.639 | 0.903 | 0.041 |

The ramp only touches the sky above 19.6° of elevation (the dome's first UV ring), so the two
shots that matter are the ones looking up. `altis_building_close` 0.108 → 0.093 and
`altis_vegetation_close` 0.139 → 0.120; its top third goes from (0.574, 0.741, 0.896) to
(0.503, 0.692, 0.881) against Arma's (0.413, 0.559, 0.798), and its luminance ratio from 1.155
to 1.054. Every other shot is unchanged to within a thousandth (`altis_kavala_noon` and
`altis_kavala_sunset` look below 5° of elevation and keep their old images exactly), so this
takes 14 % off the two worst sky shots without moving anything else. What is left is the sky's
*level*: the shipped ramp carries the gradient and its hue, but the tint under it is still our
own (`docs/re/render-atmosphere.md` §4.3, §4.5).

After #293 (the engine's own sun and moon model instead of a real ephemeris,
`docs/re/environment.md` §Sun and moon), with the same client images:

| shot | MAE lin | SSIM | lum Arma | lum ours | lum ratio | hist dist |
|---|---|---|---|---|---|---|
| altis_kavala_noon | 0.120 | 0.825 | 0.593 | 0.688 | 1.160 | 0.059 |
| altis_kavala_sunset | 0.133 | 0.751 | 0.384 | 0.279 | 0.727 | 0.117 |
| altis_hills_ground | 0.117 | 0.556 | 0.535 | 0.626 | 1.170 | 0.058 |
| stratis_coast | 0.221 | 0.686 | 0.492 | 0.717 | 1.457 | 0.153 |
| altis_building_close | 0.095 | 0.722 | 0.591 | 0.648 | 1.097 | 0.045 |
| altis_vegetation_close | 0.120 | 0.677 | 0.472 | 0.498 | 1.055 | 0.072 |
| altis_kavala_night_moon | 0.041 | 0.561 | 0.051 | 0.041 | 0.807 | 0.063 |
| altis_kavala_overcast_fog | 0.073 | 0.973 | 0.708 | 0.634 | 0.895 | 0.044 |

The night shot is the change: its luminance ratio goes from 0.192 (5.2× too dark) to 0.807 and
its SSIM from 0.322 to 0.561, and its bottom third from (0.0042, 0.0070, 0.0122) to (0.0257,
0.0596, 0.1099) against Arma's (0.0555, 0.0601, 0.0747) — the green channel now matches. The
cause was documented and wrong: the engine builds its own sun and moon (a 23° tilt, a yearly and
a daily rotation and an 81.90581 rad/year lunar angle from 1985.0082) rather than asking trueSKY
for ephemerides, and for `altis_kavala_night_moon` that model's moon is 43° up and 70 % lit
while the real Moon is new and below the horizon. The daytime shots do not suffer: the noon shot
*improves* (0.126 → 0.120, SSIM 0.773 → 0.825) and the rest move by at most 0.006, because the
model's sun stays within a couple of degrees of the real one.

What is left at night is the lamps (#294, split into its own PR) and our blue cast: our bottom
third is still short of red (0.026 against 0.056) and long on blue (0.110 against 0.075).

After #314 (the lighting entry's aperture stage is the exposure, `docs/re/hdr.md` §Aperture),
measured the same way against the same client images:

| shot | MAE lin | SSIM | lum Arma | lum ours | lum ratio | hist dist |
|---|---|---|---|---|---|---|
| altis_kavala_noon | 0.141 | 0.821 | 0.593 | 0.569 | 0.960 | 0.027 |
| altis_kavala_sunset | 0.139 | 0.741 | 0.384 | 0.267 | 0.694 | 0.131 |
| altis_hills_ground | 0.098 | 0.551 | 0.535 | 0.493 | 0.921 | 0.039 |
| stratis_coast | 0.151 | 0.696 | 0.492 | 0.594 | 1.206 | 0.094 |
| altis_building_close | 0.109 | 0.707 | 0.591 | 0.524 | 0.887 | 0.051 |
| altis_vegetation_close | 0.110 | 0.668 | 0.472 | 0.429 | 0.908 | 0.089 |
| altis_kavala_night_moon | 0.043 | 0.542 | 0.051 | 0.031 | 0.609 | 0.070 |
| altis_kavala_overcast_fog | 0.107 | 0.968 | 0.708 | 0.598 | 0.845 | 0.065 |

Against the table above: MAE sum 0.920 → 0.898, Σ|1 − ratio| 1.510 → 1.382, and the *daylight*
exposure lands on RV's `1/apertureStandard²` (noon 1.160 → 0.960, hills 1.170 → 0.921,
vegetation 1.055 → 0.908, stratis 1.457 → 1.206, with hills/stratis/vegetation also improving
their MAE). Five shots move away because the old `key/minAperture` clamp was compensating for
scene errors that are now unmasked — the same ones #337 names:

- `altis_kavala_noon` 0.120 → 0.141 MAE *although* its ratio improves 1.160 → 0.960: the sky's
  red channel, already short from #295, is darkened further.
- `altis_building_close` 0.095 → 0.109 (ratio 1.097 → 0.887) and
  `altis_kavala_overcast_fog` 0.073 → 0.107 (0.895 → 0.845): both were riding the clamp.
- `altis_kavala_night_moon` 0.041 → 0.043 (ratio 0.807 → 0.609) and
  `altis_kavala_sunset` 0.133 → 0.139 (0.727 → 0.694): the match was the clamp over-exposing
  past what the engine's aperture allows (client-verified); relative to the session's first
  capture the night ratio is still 0.192 → 0.807 (#329) → 0.609.

## Discrepancies

Ranked by visible effect. Each discrepancy has a `fidelity` issue that gives its metric, the RE
reference and a hypothesis.

1. **Ambient is sky-coloured** (#290, fixed). Shaded walls, rocks and tree trunks are blue, and terrain is
   lavender. Ours lights ambient with the sky colours, not the lighting table's
   `ambient`/`ambientMid`/`groundReflection` (`render-materials.md` §3.1). Roads get almost no
   ambient and go black at low sun.
2. **Sea** (#291). Ours is a flat, bright cyan placeholder with no waves and no reflection. Arma shows
   dark blue-grey water with wave normals, sky reflection and transparent shallows.
3. **No ground clutter** (#292). Arma shows dense grass and weeds near the camera. Ours shows bare
   terrain.
4. **Night exposure** (#293, fixed). Ours was 5.2× too dark under what the shot called a full moon.
   The engine's own sun and moon model puts a 70 %-lit moon 43° up at that date, so the ground is
   moonlit; we were using a real ephemeris, which has a new moon below the horizon. Luminance
   ratio 0.192 → 0.807. What is left is the blue cast and the missing lamps.
   With #314 the exposure is the engine's own (`1/apertureMin² = 1/16`, client-verified) instead
   of a clamp that over-exposed, so the ratio reads 0.609 while the exposure is right: the rest is
   scene, tracked in #337.
5. **No point lights** (#294, split). Street lamps light Kavala at dusk and at night in Arma. Ours
   has no lamps; the light list and the lamp discovery are a separate lighting path.
6. **Sky colour** (#295, the gradient fixed). Our zenith was paler, more cyan and brighter.
   The sky now carries the shipped dome's own elevation ramp (`render-atmosphere.md` §4), which
   took `altis_building_close` 0.108 → 0.093 and `altis_vegetation_close` 0.139 → 0.120. What is
   left is the sky's overall level: the tint the engine multiplies the ramp by (`PSHorizon`'s
   `v1.xyz`) is not traced, so our sky is still brighter than Arma's and its zenith a little
   short on red.
7. **Daylight exposure** (#296, fixed). Ours was 10–15 % brighter at noon, because our exposure
   was `key/minAperture = 1.102e-4` instead of the engine's `1/apertureStandard² = 6.94e-5`
   (#314, `docs/re/hdr.md` §Aperture): noon now reads 0.960, hills 0.921, vegetation 0.908,
   stratis 1.206. The noon MAE rises (0.120 → 0.141) because the sky's red channel, short from
   #295, is darkened with it.
8. **Fog colour and density, overcast** (#297). Our fog is darker and bluer. Arma's fog is neutral
   white-grey.
9. **Sunset light** (#298). Ours is too dark and blue at low sun. Arma is still neutral at 19:15.
   The lighting entry our lookup picks matches the client's default capture (0.3 %, `hdr.md`), so
   this is the shading under it; the exposure half is #337.
