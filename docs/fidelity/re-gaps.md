# RE gap ledger

Where our implementation still guesses because the reverse engineering is incomplete. One row per
gap: the RE doc and item, the code that stands in for the engine today, how visible the guess is,
and its status. Part of the fidelity epic (#261).

`python tools/re/re_gaps.py` lists every confidence marker and open-question bullet in
`docs/re/` (`--summary` for counts per doc). It finds candidates; this file is the curated list.
When a gap closes, the RE doc loses its marker and the row here records the PR.

**Impact** is on visible or gameplay fidelity:
- **high**: seen every frame or in core gameplay (LOD choice, colour, exposure, how a man moves,
  how a bullet flies);
- **med**: seen in specific situations or once a feature lands;
- **low**: edge cases, formats no shipped file uses, tooling.

**Status**: `open`; `closed #PR` (doc at high confidence, code fixed or nothing to fix);
`doc #PR → #issue` (doc closed, implementation change filed for the owning crate).

Network items are out of scope for this track (deferred with Phase 6).

## Tier 1: implemented today with a stand-in

| id | doc § | gap | guessing code | impact | status |
|---|---|---|---|---|---|
| R1 | render-lod §6 | how the Resolution LOD is picked (was read as a per-LOD value `level+0xac`; it is a frame-wide face budget) | `a3-render-models::lod::LodSelector::assign` | high | closed #312 (frame-to-frame hysteresis not yet kept) |
| R2 | render-lod §3 | global area multiplier `scene+0x8c4` (= render target width · height) | `a3-render-models::lod::ViewScale` | high | closed #312 |
| R3 | render-lod §7 | shadow LOD: per-model table from the visual LOD, normally a Shadow Buffer LOD | `a3-render-models` shadow pass (draws the visual LOD) | med | doc closed → #306 |
| R4 | render-lod §5 | shadow test distance `d'` (to the bounding sphere surface) and `K·r²` without drawImportance | `a3-render-models::lod::LodSelector::casts_shadow` | low | closed #312 |
| R5 | render-lod §7 | super-LODs (FOREST_LOD1/2, TOWN_LOD1) | not implemented | med | open |
| C1 | render-materials §6 | which stages bind the sRGB vs the linear SRV (`0x1416d7e10`) | `a3-render-models::material` (colour types sRGB by suffix), `a3-landscape-render` satellite / detail layers | high | open (#173) |
| C2 | render-materials §2, render-atmosphere §5 | CfgWorlds lighting and rvmat colours → `PSC_AE/GE/AmbientMid/Diffuse/DForced/Specular` | `a3-render-models` hemisphere ambient from a3-render sky colours | high | open (#173) |
| H1 | render-atmosphere §3.3, hdr | `PSC_AssumedLuminancePars1/2`: per-frame limits (halve in 0.5–1 s, double in 1–20 s by CPU exposure) decoded; the key `engine+0x368 · brightness · 0.5` has one unidentified factor | `a3-render::post` `key = 0.3` | high | limits closed #313-PR; key factor open |
| H2 | render-atmosphere §3.2 | `PSC_RgbEyeCoef.w` = 1 always (and its rgb / night shift) | `a3-render::post` `final_gamma = 1` (correct) | high | closed #313-PR |
| H3 | hdr, render-atmosphere §3.3 | CPU aperture stage: `apertureMin/Std/Max`, `standardAvgLum`, `apertureRatioMin/Max`, `eyeAdaptFactorLight/Dark` | `a3-render::post` (not implemented) | med | doc closed #313-PR → #314 |
| H4 | hdr, render-atmosphere §3.3 | luminance meter (histogram vs mean, percentiles) | `a3-render::post` log-average | med | open |
| H5 | hdr | bloom generation passes (`bloomLuminance*`) | `a3-render::post` quarter-res Gaussian | med | open |
| A1 | render-atmosphere §1.3 | cloud factor `c` and sun/moon blending in the lighting lookup | `a3-render::sky`, `apps/arma3/src/environment.rs` | med | open |
| A2 | render-atmosphere §2 | `PSC_PhysicalFog` source | `a3-render::post` fog | med | open |
| T1 | render-terrain §2/§6 | setter of the AFog detail-fade constants (`fullDetailDist`/`noDetailDist` link, medium) | `a3-landscape-render` detail fade | med | open |
| T2 | render-terrain §6 | CPU terrain mesh: segment size, LOD heights, `VSC_TerrainLODPars` | `a3-landscape-render::lod` | med | open |
| S1 | shadows | cascade split scheme, `shaderQuality` taps, `shadowZDistance` default/range | `a3-render::shadow` (λ = 0.75 blend, 3×3 PCF, 150 m) | med | open |
| M1 | sim-man-locomotion §3 | step-up / step-down height | `a3-world::sim::man::ground::MAX_STEP_UP`/`MAX_STEP_DOWN` = 0.5 (placeholders) | high | open |
| M2 | sim-man-locomotion §4 | gravity on a Man, fall trigger, landing | `a3-world::sim::man::ground::Motion` (`a3_physics::GRAVITY`) | high | open |
| M3 | sim-man-locomotion §2 | turn units, where `turnSpeed` applies, the turn clamp | `a3-world::sim::man::turn` (`TURN_RAMP = 6`, `FULL_TURN = π/2`) | high | open |
| M4 | sim-man-locomotion §5, sim-man-movement §4 | how `CfgSlopeLimits` downgrades or blocks a move | not implemented | med | open |
| M5 | sim-man-movement §1 | `skillSpeedCoef`'s skill input `s` (`0x1405ff5a0`) | `a3-world::sim::man::moves` | low | open |
| B1 | sim-ballistics §8 | missile thrust, guidance, submunitions | `a3-world::sim::projectile` (PR #250) | high | open |
| B2 | sim-ballistics §8 | Man fatal hit points, `depends`, total damage from hit points | hit points (PR #254) | high | open |
| B3 | sim-ballistics §7.2/§8 | tunable `g`, `shotCoef`, vfunc `+0x640` component | hit distribution (PR #254) | med | open |
| B4 | sim-ballistics §4 | surface field for the ricochet coefficient; penetration `R = 0.01` | ricochet / penetration (PR #250) | med | open |
| I1 | ai §4 | formation slot offsets | `a3-world::ai` formation offsets ("ours") | med | open |
| I2 | ai §5 | target knowledge constants | `a3-world::ai::target` | med | open |
| I3 | ai §2.2 | waypoint insert at the current index | `a3-world::ai::waypoint` | low | open |

## Tier 2: flight models (blocking #126)

| id | doc § | gap | guessing code | impact | status |
|---|---|---|---|---|---|
| F1 | sim-vehicles §3 | basic helicopter force formulas (`helicopterrtd` without RTD) | `a3-world::sim::air` (empty) | high | open |
| F2 | sim-vehicles §4 | `airplanex` aero formulas, envelope bin spacing | `a3-world::sim::air` (empty) | high | open |
| F3 | sim-vehicles §2 | PhysX drive mapping, wheel mass and MOI | `a3-vehicles` (PR #258) | med | open |
| F4 | sim-vehicles §5 | ship, submarine, hovercraft forces | not implemented | low | open |

## Tier 3: long tail

| id | doc § | gap | guessing code | impact | status |
|---|---|---|---|---|---|
| L1 | p3d-odol | quad split `(a,b,c)(a,c,d)` (medium), tangent sign in the vertex shader | `a3-p3d`, `a3-render-models::prepare` | med | open (#150) |
| L2 | p3d-odol | face flags, selection vertex weights, material render flags | `a3-p3d` | low | open (#150) |
| L3 | rtm | keyframe sampling (linear vs slerp, looping), bind pose vs model space | `a3-rtm`, `a3-anim` | med | open (#152, #178) |
| L4 | model-animations | order of several animations on one bone; `animPeriod`/`initPhase` | `a3-anim` | med | open |
| L5 | landscape | `_lca` mask colour → layer slot, detail texcoord scale, surface pattern priority | `a3-landscape-render`, `a3-physics::surface` | med | open |
| L6 | physics-collision | engine surface query `0x1416527b0`, animated Geometry, `soundHit` categories | `a3-physics::query` | med | open |
| L7 | navigation §8 | oper-field cell size and costs, planner budget, Clearance/cover cost | `a3-nav` | med | open |
| L8 | config | load order, inheritance resolution time, `getNumber` on strings, `getText` on numbers | `a3-config::order`, `a3-config::tree` | med | open (#25, #72, #24) |
| L9 | preprocessor | unverified preprocessor behaviour | `a3-preproc` | med | open (#35) |
| L10 | sqf-semantics | scheduler budget (3 ms), NaN print, `for` scoping, nil-argument commands, `pixelGrid = 16` | `a3-sqf` | med | open (#73) |
| L11 | input-keys | double-tap window, hold threshold, mouse-axis scaling, combo suppression | `a3-input` | med | open |
| L12 | fxy, ui | page metrics, offsets, `CfgFontFamilies` size choice, control loader | `a3-fonts`, `a3-ui` | med | open (#155) |
| L13 | environment | overcast interpolation between levels | `a3-environment` | med | open |
| L14 | missions | two-element `position[]`, `special` CARGO, `randomSeed`, intro/outro loading | `a3-mission` | low | open |
| L15 | audio | `night` factor source, occlusion/obstruction/doppler consumers, 3D processor blend, legacy `distance` | `a3-audio` | low | open |
| L16 | paa | how MAXC/average colour is applied; mip size rule | `a3-paa`, `a3-render` | low | open (#137) |
| L17 | vfs | bank override priority | `a3-vfs` | med | open (#18) |
| L18 | wrp | outside-terrain height synthesis, `major`, shape param, pre-23 LZSS | `a3-wrp`, `a3-landscape-render` | low | open (#63) |
| L19 | stringtable | the `@` table, CSV tables, preprocessing | `a3-stringtable` | low | open (#153) |
| L20 | world-object-model | rest of the `simulation` table, World sub-lists | `a3-world::class` | low | open (#117) |
| L21 | compression, signing, pbo | LZSS checksum kinds, unsorted-header hash order, EBO | `a3-compress`, `a3-signing`, `a3-pbo` | low | open (#59, #154, #19) |
| L22 | functions-init, wss | preStart call site; WSS streaming | `a3-sqf` init, `a3-audio-formats` | low | open |
