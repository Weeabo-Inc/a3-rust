# Sea surface (ocean water)

How Arma 3 2.22 draws the sea: the patch mesh the Landscape builds, the wave function shared by
the CPU and the vertex shader, and the `Water` pixel shader (normal maps, sky reflection,
screen-space reflection, refraction, foam, sun glint). Underwater fog of other objects is in
`render-atmosphere.md` §2.

**Sources:**
- Compiled shaders `VSWater` and `PSWater` from `Dta\bin.pbo` (`Shaders_5_0_VS.shdc`,
  `Shaders_5_0_PS.shdc`), disassembled with `tools/re/shdc.py`. Formulas marked **(shader)** are
  transcribed from the disassembly (high confidence).
- Engine code in `arma3_x64.exe` (RVAs below). Constant names come from the shader bytecode, so the
  CPU-to-shader mapping is high confidence where a setter writes the named descriptor.
- The resolved config of build 2.22 (`CfgWorlds >> Altis`, `CfgMaterials >> Water`).

```sh
python tools/re/shdc.py dump .work/shaders/Shaders_5_0_PS.shdc '^PSWater$' out/
python tools/re/shdc.py dump .work/shaders/Shaders_5_0_VS.shdc '^VSWater$' out/
```

## 1. Configuration

`CfgWorlds >> <world> >> Sea` (loader `0x141648a40`, fields of `Landscape`):

| entry | Landscape | default | Altis | meaning |
|---|---|---|---|---|
| `WaterMapScale` | `+0x4c` | 20 | 20 | scale of the per-cell minimum-height bytes (§2) |
| `WaterGrid` | `+0x50` | 50 | 50 | water cell edge in metres |
| `MaxTide` | `+0x58` | 1.5 | 0 | tide amplitude |
| `MaxWave` | `+0x5c` | 0.25 | 0.25 | only used by the patch cull (§2) |
| `SeaWaveXScale` | `+0x60` | 0.04 | 2/50 | radial wave frequency (cycles per metre) |
| `SeaWaveZScale` | `+0x64` | 0.02 | 1/50 | angular wave frequency (cycles per water cell of arc) |
| `SeaWaveHScale` | `+0x68` | 1 | 1 | wave height scale |
| `SeaWaveXDuration` | `+0x6c` | 5000 | 5000 | radial wave period, ms |
| `SeaWaveZDuration` | `+0x70` | 10000 | 10000 | angular wave period, ms |

`seaMaterial = "#water"` is `CfgMaterials >> Water`: `PixelShaderID = VertexShaderID = "Water"`,
stage 1 `A3\data_f_Exp\water_nofhq.paa`, stage 2 `A3\data_f_Exp\sea_foam_lco.paa`, stage 3
`A3\data_f_Exp\water2_nohq.paa`, stages 4/5 grey placeholders the engine replaces (§4.3),
`specular[] = {0.12,0.12,0.12}`.

`CfgWorlds >> <world> >> WaterExPars` (loader `0x14161c9e0`): every entry is a 0x20-byte record
(value, then a "set" flag at `+0x14` that must equal 2). The engine copies the struct to
`EngineDD11 + 0xc38a60`. Entries used by the sea: `fogDensity`, `fogColor`,
`fogColorExtinctionSpeed`, `ligtExtinctionSpeed`, `diffuseLigtExtinctionSpeed`, `fogGradientCoefs`,
`fogColorLightInfluence`, `ssReflection{Strength,MaxJitter,RippleInfluence,EdgeFadingCoef,DistFadingCoef}`,
`specularMaxIntensity`, `specularPowerOvercast{0,1}`, `specularNormalModifyCoef`,
`refraction{MinCoef,MaxCoef,MaxDist}`, `surfaceOpacity`, `shadowIntensity`,
`foamAroundObjects{Intensity,FadeCoef}`, `foamColorCoef`, `foamDeformationCoef`, `foamTextureCoef`,
`foamTimeMove{Speed,Amount}`. An entry the world does not set counts as 0 in the shader
constants below (Altis sets all except `surfaceOpacity`).

## 2. Patches and mesh (CPU, high)

`Landscape` draws the sea in square **patches** of `N = 8` water cells (400 m on Altis)
(`0x141634510`):
- It walks the visible rectangle of water cells in steps of `N`. The rectangle extends beyond the
  map edge, so the sea continues past the terrain.
- A patch is drawn only if its lowest terrain is at or below `seaLevel + MaxWave`. `seaLevel` is
  `Landscape+0x103c`, including the tide. The lowest terrain comes from a byte per water cell
  (`Landscape+0xd50`, value / `WaterMapScale`). Outside the map it comes from the outside terrain,
  or from the clamped edge cell.
- It also skips patches outside the view frustum, or beyond the view distance plus the patch radius.
- Each patch is a micro-job (`Landscape::WaterDrawTask`, `0x141620830` → `0x141659080`) that
  draws one of 6 LOD meshes, translated to the patch centre at height `seaLevel`.

**LOD.** `d` = distance from the camera to the patch centre minus the patch's bounding radius
`sqrt(2·(N·WaterGrid/2)² + 0.25)`.
- If `d / sqrt(P00·P11) · 0.01 < 1` (P = projection matrix; the zoom scales it), LOD 0.
- Otherwise `lod = round(max(d, near) / (N·WaterGrid))`. LOD 4 and 5 (≥ 6 counts as 5) are used
  only when all four patch corners are at least 10 m deep (terrain ≤ −10). Otherwise the LOD is 3.

**LOD mesh** (`0x14163b120`):
- Subdivisions per water cell `s = round(sqrt(B / (r²·N²)))`, clamped to 1…16, then rounded down
  to a multiple of `32/N` (= 4).
  - `B = clamp(round(sceneComplexity · 0.1), 10000, 300000)`. `sceneComplexity` is `scene+0x8a4`,
    see `render-lod.md` §1.
  - `r = round(390 / (N·WaterGrid) + 0.5)` (= 1).
  - So `s = 16` from Standard objects quality up, 12 at VeryLow.
- LOD `l` has `n = (s·N) >> l` quads per side.
- Vertices are at `(i − n/2)·(2^l/s)·WaterGrid` around the patch centre, `y = 0`. Border vertices
  are pushed 0.01 m outwards, so neighbouring patches overlap.
- The two triangles of each quad alternate their diagonal in a checkerboard.

**Per-patch depth grid** (`WaterSegment`, `0x14163ad00`): `(N+1)²` = 81 float4. `.x` is the
terrain height at each water-grid vertex of the patch: the height sample when `WaterGrid` equals
the terrain cell, otherwise the interpolated height. `.w` = 1. `.yz` are not used by the sea
shader. Uploaded as `VSC_WaterDepth[81]`.

## 3. Waves

### 3.1 CPU wave height (`0x14163d300`, high)

Physics and the camera's underwater test use the same function as the vertex shader:

```
t      = weather time in ms (int, Weather+0xf0)
phaseX = (t mod XDuration) / XDuration
phaseZ = (t mod (8·ZDuration)) / ZDuration                      // 0..8
g      = (x, z) / WaterGrid − c,   c = (mapSize/2) / WaterGrid   // water cells from the map centre
radius = |g|,   angle = atan2(g.x, g.z) · 79.577472               // 500 per turn
u      = WaterGrid · radius · XScale + phaseX
v      = WaterGrid · angle  · ZScale + phaseZ
u'     = u + 0.2·sin(2π·0.25·v − π) + 0.12·sin(2π·0.375·v − π)
A      = waves · 0.5 · HScale                                     // waves = Weather+0x4c (setWaves)
H      = bilinear terrain height at (x, z) over the WaterGrid lattice
k      = saturate((−1 − (H − seaLevel)) / 7)²                     // no waves shallower than 1 m
height = seaLevel + A · (sin(2π·u' − π) + sin(2π·v − π)) · k
```
`Landscape+0x54` is `1/WaterGrid`. The waves are rings around the map centre. `u` grows with
time at a fixed radius, so a crest moves towards the centre (the island's shores) by one radial
wavelength (`1/XScale` = 25 m) every `XDuration`. A second, angular wave and
the two small `v` terms wobble the rings. `0.375·500` is not an integer, so the 0.12 term has a
seam along the line due south of the map centre (also in the shader).

### 3.2 Vertex shader `VSWater` (shader)

Constants (`VSCB_Water`, set by `0x1417284c0`):

| constant | value |
|---|---|
| `VSC_WaveHeight` | `(A, A, 2π·XScale·A, 2π·ZScale·A)`. Amplitude and frequencies come from a virtual call (`engine+0x120`, slot 2) that is not traced. They are assumed to be the §3.1 values (medium) |
| `VSC_WaveGrid` | `(1/WaterGrid, N+1, N, N/2)` |
| `VSC_WaterPeakWhite` | `.zw = (1/(top − bottom), −bottom/(top − bottom))` with `top = −1`, `bottom = −8` |
| `VSC_WaterSeaLevel` | `(seaLevel, 1/32, (40 − seaLevel)/32, 0.9)` |
| `VSC_WaveUVPars` | `(c, c, 65534/N, −32767)`: polar centre in water cells, then the UV packing scale and offset that the stage transform undoes |
| `VSC_WaterDepth[81]` | the patch depth grid (§2) |
| `VSC_WaterPars[0]` | `(cameraWorld.xyz, globalTime_s)` |
| `VSC_TexTransform[0]` | maps (radius, angle·79.58) to (u, v) of §3.1, scale `WaterGrid·{X,Z}Scale`, offset the phases (medium: the matrix is built from the material's `texWaterAnim` stage; it matches the CPU function term for term) |

```
W    = world position of the vertex (patch translation + local)
(u,v)= §3.1 from W.xz
h    = sin(2πu' − π)·WH.x + sin(2πv − π)·WH.y
d    = |viewPos|;  fade = 1 − saturate((d − 50) · sqrt(1/(P00·P11)) · 0.01)   // waves only near the camera
G    = bilinear VSC_WaterDepth[.].x at W.xz·WaveGrid.x + WaveGrid.w (clamped to 0..N) − SeaLevel.x
peak = saturate(PeakWhite.z·G + PeakWhite.w)          // 0 deeper than 8 m, 1 shallower than 1 m
y    = local.y + h · fade · (1 − peak)²
shallow (o6.x) = saturate(SeaLevel.y·G + SeaLevel.z)  // 0 deeper than 40 m, 1 shallower than 8 m
N    = normalize(lerp((−cos(2πu' − π)·WH.z, 10, −cos(2πv − π)·WH.w), (0,1,0),
                      min(1 + peak − saturate(1.25 − 0.00125·|cam − W|), 1)))
```
The vertex normal is the wave normal within 200 m (unless shallow) and fades to straight up by
1000 m.

**Normal-map UVs** (`t` = `WaterPars[0].w`, seconds; `W.xz` world):
- `o4.xy = W.xz · 0.002 + t·0.004`
- `o4.zw = W.xz · 0.02 + t·0.003`
- `o5.xy = W.xz · 0.0666667 + t·0.002`

**Octave weights by distance** (`h` = |camera height|, `d` as above):
- `o5.z = 1.6 · saturate(1.4286 − d / (700·(1 + 0.02h)))`
- `o5.w = 1.2 · saturate(1.75 − d / (20·(1 + h/30)))`

`o6.y = VSC_LDirectionD.w` blends the two sky reflection textures (§4.3).

## 4. Pixel shader `PSWater` (shader)

### 4.1 Constants (`PSCB_Water`, setter `0x141717090`)

`o` is the weather's cloudiness (`Weather+0x3c`; medium: assumed to be the overcast), `x = 1 − o`.
`Ly` is the y of the light direction `L` (the direction the light travels, pointing down by day).

| constant | value |
|---|---|
| `PSC_WaveColor` (foam colour) | `saturate(0.25·(ambient + diffuse)·foamColorCoef)`, `.w = −0.4` |
| `PSC_CalmWaterPars1[0]` | `0.6·(sin(a + kπ/2) + 1)`, k = 0..3, `a = (2x + 3)·t` |
| `PSC_CalmWaterPars1[1]` | `0.5·(sin(b + kπ/2) + 1)`, `b = (2.01x + 3.011)·t` |
| `PSC_CalmWaterPars1[2]` | `(1/width, 1/height, 0, 0)`: pixel → screen UV |
| `PSC_CalmWaterPars1[3]` | `(0.5 − 0.5x, 4x + 2.5, sunUp · smoothstep(x) · specularMaxIntensity, specularPowerOvercast0 − o·(Overcast0 − Overcast1))`. `sunUp = saturate((−Ly + 0.03489)·3.368251)`. Without WaterExPars: intensity 25, power `200 − 150o` |
| `PSC_CalmWaterPars1[4]` | `(x, saturate((−Ly + 0.1736)·1.6772895), 1 − that, t)` |
| `PSC_CalmWaterPars1[5..7]` | inverse projection scale, camera-above-water flag, `SeaWaterShaderPars` (CalmWater only) |
| `PSC_WaterAdditionalPars[0]` | `(refractionMinCoef, refractionMaxCoef, 1/refractionMaxDist, shadowIntensity)` |
| `PSC_WaterAdditionalPars[1]` | `(foamAroundObjectsIntensity, foamDeformationCoef, foamTextureCoef, foamAroundObjectsFadeCoef)` |
| `PSC_WaterAdditionalPars[2]` | `(foamTimeMoveAmount·sin(foamTimeMoveSpeed·t), foamTimeMoveAmount·cos(…), specularNormalModifyCoef, surfaceOpacity)` |
| `PSC_WaterSSReflectionPars[0]` | `(ssReflectionStrength, maxDistance, ssReflectionMaxJitter, ssReflectionRippleInfluence)`. `maxDistance` is the view distance (×1.1 in one mode). Strength is 0 when screen-space reflections are off in the video options |
| `PSC_WaterSSReflectionPars[1]` | `(ssReflectionEdgeFadingCoef, ssReflectionDistFadingCoef, 1/P00, 1/P11)` |
| `PSC_GlassEnvColor` | `ambient + 0.05·diffuse` (`0x1410880e0`), times the engine's exposure terms |
| `PSC_Specular` | sun colour × material `specular` |
| `PSC_LDirectionGround` | `(L.z, −L.x, −L.y)` (the shader swizzles it back to `L`) |

`t` is the global time in seconds. Lighting `+0x6c` and `+0x7c` are read as diffuse and ambient
(medium: they are the two lighting colours summed for the foam).

Textures:

| slot | content |
|---|---|
| t1 | `water_nofhq` (stage 1). Normal in `.g`/`.a` |
| t2 | `sea_foam_lco` (stage 2) |
| t3 | `water2_nohq` (stage 3). Normal in `.g`/`.a` |
| t4, t5 | sky reflection textures (§4.3) |
| t10 | scene colour before the water |
| t11 | scene depth (linear view depth) |
| t12, t13 | colour and depth for screen-space reflections (the same buffers, or last frame's) |

### 4.2 Normal

```
s  = 0.8·x + 0.4
w6 = s·CWP1[0]·o5.z,   w7 = s·CWP1[1]·o5.w
a  = nm(t1, o4.xy) + nm(t1, o4.zw)                              // nm = tex.ga·2 − 1
b  = Σk w6[k]·nm(t1, o4.zw + 0.25k) + Σk w7[k]·nm(t3, o5.xy + 0.25k)
t  = normalize(1.8·a + b, CWP1[3].y)                            // tangent-space (xy, z)
steep = 1/|(2a, 1)|                                             // for crest foam
T  = normalize(cross((1,0,0), N)),  B = cross(N, T)              // N = vertex normal
n  = −T·t.x + B·t.y + N·t.z                                     // flat: n = (t.y, t.z, −t.x)
```

### 4.3 Reflection and refraction

`V` is the unit vector from the pixel to the camera. The surface is seen from below when its
front face is towards the camera (the mesh is wound so that its front faces down).
- **Fresnel.**
  - From above: `F = 0.02 + 0.98·(1 − max(n·V, 0))⁵`.
  - From below: `F = min((1.512146·(1 − max(−n·V, 0)))⁴, 1)` (total internal reflection).
- **Sky reflection.**
  - `R = reflect(−V, n)`, `uv = R.xz·|R.xz|²·0.45 + 0.5`.
  - From above: `refl = lerp(t5(uv), t4(uv), o6.y) · GlassEnvColor`.
  - From below: `refl = WaterFogColor`.
  - t4/t5 replace the material's grey stages 4/5. They are the weather's sky reflection
    textures (`CfgWorlds >> … >> Weather >> WeatherN >> skyR`, e.g.
    `A3\Map_Stratis\Data\sky_clear_lco.paa`) of the current and next overcast level, blended by
    `VSC_LDirectionD.w` (medium: the engine's binding of t4/t5 is not traced; the blend factor
    matches the overcast sample's).
- **Screen-space reflection** (if `SSRPars0.x > 0` and the reflected ray points up, or the
  surface is seen from below):
  1. Blend the normal toward the vertex normal: `n' = normalize(lerp(N, n, SSRPars0.w))`.
  2. Reflect the view ray in view space and clip it at `maxDistance` depth.
  3. Weight by `saturate((Rview.z − 0.2)·5)`: rays toward the camera get none.
  4. Project the start and end points and clip the ray to the screen.
  5. Take 8 steps (fractions 0.0125 + k·0.125). A step hits where `|1/z_ray − 1/z_scene|` is
     below the step's `1/z` span.
  6. A hit takes `t12` with `edge = 1 − min((2·max|uv − 0.5|)^EdgeFadingCoef, 1)` and
     `dist = (1 − saturate((z − z0)/(maxDistance − z0)))^DistFadingCoef`, where
     `z0 = min(0.75·maxDistance, 100)`.
  7. A second, jittered pass averages four rays (`ssReflectionMaxJitter`).
  8. `refl = lerp(refl, avg, weight · SSRPars0.x)`.
- **Refraction.**
  1. `uvS = pixel·CWP1[2].xy`, `thick = t11(uvS) − viewZ`.
  2. `k = lerp(AP0.x, AP0.y, saturate(thick·AP0.z)) · clamp(10·P00/viewZ, 0.1, 1)`.
  3. `k` is zero where `thick < −0.01`, and fades within 0.05 of the screen edge.
  4. `off = saturate(5·thick)·k`.
  5. If `off ≥ 0.001`, sample at `uvS + n.xz·off`.
     - Each lookup takes the minimum depth of a 5-tap cross (±1 texel), and `off` shrinks to stay
       inside the water.
     - If any of the 5 colour taps is the clear colour (sky, `|c − ClearColor|² < 0.005`), the
       undistorted `uvS` is used.
  6. `refr = lerp(t10(uv), WaterFogColor, (1 − exp(−0.1·|z_scene − viewZ|))·max(1 − 4|V.y|, 0))`.
  7. `refr = lerp(refr, WaterFogColor, surfaceOpacity)`.

  The scene behind the water was already fogged with the underwater fog by its own shaders
  (`render-atmosphere.md` §2), so this extra tint only matters at grazing angles.

### 4.4 Foam, glint and output

```
edge  = saturate((rel − 0.05)·10) if AP1.x > 0.01, else 1      // shore alpha
rel   = (z_scene − viewZ) · max(0.2, |camera.y|) / viewZ
shoreFoam = saturate((1 − saturate((rel − 0.1)·AP1.w))²
                     · saturate(t2(n.xz·AP1.y + P.xz·AP1.z + AP2.xy).r
                                − (saturate(0.5·(rel − 0.1)·AP1.w) + 0.2))
                     · edge · AP1.x)
crest = (1 − c)(1.5 − c),  c = saturate((steep − 0.7)·5)
      · (shallow⁴ + 0.1) · saturate(0.5 − x)
      · saturate(t2(o4.xy·50.1).r + t2(o4.zw·10.2).r + t2(o5.xy·0.11).r − 2.4)
foam  = max(shoreFoam, crest)
col   = lerp(refr, refl, F)
ns    = normalize(lerp(n, normalize(n.x, 0.6, n.z), AP2.z))
spec  = saturate(F · saturate(reflect(L, ns)·V)^CWP1[3].w · PSC_Specular · CWP1[3].z)
col   = lerp(col, WaveColor, foam);   spec *= (1 − foam)·edge
```
Point and spot lights add like `render-materials.md` §3.4, with `0.5×` on the diffuse part.

**Output** (premultiplied, alpha = `max(edge, DiscretizeAlpha.w)`):
- `col` is split as `col·r` and `col·(1 − r)`, with `r = 0.6x + 0.2`.
  - Underwater, the first part is multiplied by `exp(−depth·WaterDiffuseLightExtinctionCoefs)`
    and the second by `exp(−depth·WaterLightExtinctionCoefs)`.
  - Above water both factors are 1 (`depth = max(0, cb5[12].z)`), so the split cancels.
- The fog switch follows `render-atmosphere.md` §2.
  - From below: water fog over the whole distance.
  - From above: air fog only.

## 5. Ponds

Ponds and rivers are p3d objects with a material on the `Water` shader (`WaterRiver` in
`CfgMaterials`, used by Livonia). They go through the same `VSWater`/`PSWater` pair. Altis and
Stratis have none. Not traced further.

## 6. Open points

- The `engine+0x120` slot-2 call that supplies `VSC_WaveHeight` (assumed to equal §3.1), and the
  `texWaterAnim` stage matrix.
- Whether `Weather+0x3c` is the overcast or a derived value.
- The CPU source of `PSC_WaterFogColor`. The engine keeps a colour at `EngineDD11+0x38` and uploads
  half of it (`0x141713150`). It likely comes from `WaterExPars >> fogColor` and the lighting
  (`fogColorLightInfluence`).
- The binding of t4/t5. `PSSSSMWater` (shadow-receiving variant), `PSShore*` (shore waves) and
  `PSCalmWater` are not decoded.

## 7. Our implementation (`a3-landscape-render::sea`)

What the sea renderer does with the above. These are engine-side decisions, not RE.

- **Patches and LOD meshes** follow §2. The renderer draws patches within the view distance,
  culled against the frustum. The per-patch depth grids are one texture of the terrain height
  at every water-grid vertex of the map (the same values; bilinear between them). Outside the
  map, depths come from the clamped edge.
- **Waves.** The vertex shader is §3.2, with `VSC_WaveHeight` from §3.1.
- **Pixel shader** follows §4, with these differences:
  - The sky reflection textures are the overcast sample's `skyR` (current and next). They are
    read as linear data, not sRGB-decoded: with the sRGB decode the reflection is several times
    darker than the real client's. Measured on the render oracle's `stratis_coast` shot: the
    middle band (distant sea) matches the client within 5 % with the linear read.
  - The screen-space reflection uses one ray with the §4.3 steps and fades (no jittered second
    pass).
  - No point lights.
  - Light colours carry no exposure, so the foam colour and the glint are capped in exposed units
    using the eye adaptation readout of the previous frame, instead of the engine's
    pre-multiplied `saturate`.
- **Scene copy.** `a3-render`'s water phase gives the colour and depth after the opaque phase
  (t10–t13).
- **Underwater fog.**
  - Seen from above, the water shader fogs the refracted scene itself, with §2 of
    `render-atmosphere.md` over the underwater part of the ray. Our post pass, not the object
    shaders, applies fog.
  - Seen from below, the post pass splits each ray at the water height.
- **Underwater light.** The terrain and model shaders dim the light under the sea by depth, as
  in `render-materials.md` §3.1: ambient by `ligtExtinctionSpeed` and sun by
  `diffuseLigtExtinctionSpeed`. Shallow water over sand turns turquoise from this.
- `PSC_WaterFogColor` = `fogColor · (0.8·ambient + 0.2·diffuse) · 1.0`, using
  `fogColorLightInfluence = (0.8, 0.2, 1.0)` as the weights. This is an approximation (see §6).
  On `stratis_coast` the near sea comes out about 15 % darker than the client.
