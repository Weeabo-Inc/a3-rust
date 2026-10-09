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
4. **Night exposure** (#293). Ours is 5.5× too dark under a full moon. Arma adapts until the moonlit
   terrain shows its colours.
5. **No point lights** (#294). Street lamps light Kavala at dusk and at night in Arma. Ours has no lamps.
6. **Sky colour** (#295). Our zenith is paler and more cyan, and brighter. Arma's sky is a deeper blue
   with a stronger gradient towards the horizon.
7. **Daylight exposure** (#296). Ours is 10–15 % brighter at noon.
8. **Fog colour and density, overcast** (#297). Our fog is darker and bluer. Arma's fog is neutral
   white-grey.
9. **Sunset light** (#298). Ours is too dark and blue at low sun. Arma is still neutral at 19:15.
