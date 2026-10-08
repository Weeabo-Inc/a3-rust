# Material shaders (rvmat PixelShaderID / VertexShaderID)

This page covers what each material shader does with its rvmat stages, and the lighting model they share. It is
the source of truth for the model renderer (#52). `rvmat-shaders.md` links here. Terrain shaders are in
`render-terrain.md`. Fog, haze and sky are in the lighting/atmosphere topic.

**Sources and confidence**
- Shader code comes from the engine's compiled DX11 shaders (`Dta\bin.pbo` → `Shaders_5_0_PS.shdc`),
  disassembled with `tools/re/shdc.py`. The bytecode keeps constant-buffer and constant names. Formulas
  marked **(shader)** are read directly from the disassembly, so confidence is high.
- Enum values come from the registration code in `arma3_x64.exe` (`0x1400f80f0` for PS, `0x1400fd200` for VS).
  Confidence is high.
- Usage counts come from a byte scan of the PixelShaderID / VertexShaderID strings in every rvmat of the base
  game and DLC PBOs, terrain layer PBOs excluded.

```sh
python tools/re/shdc.py list .work/shaders/Shaders_5_0_PS.shdc '^PSSuper'
python tools/re/shdc.py dump .work/shaders/Shaders_5_0_PS.shdc '^PSSuper$' out/
```

Notation: DXBC `exp`/`log` are base 2. The formulas below write `pow`, `exp2` or `exp` (natural) as
appropriate.

## 1. Enums

ODOL stores a material's shaders as these integers. Rvmat files store the names. The data compares names
case-insensitively: rvmats contain `super`, `TreeADV` and `Basicfade`.

### PixelShaderID (value = name; the HLSL entry is `PS<name>` unless given in brackets)

```
0 Normal [PSSpecularAlpha]          1 NormalDXTA                       2 NormalMap
3 NormalMapThrough [PSSpecularNormalMapThrough]   4 NormalMapGrass [PSSpecularNormalMapGrass]
5 NormalMapDiffuse [PSSpecularNormalMapDiffuse]   6 Detail [PSDetailSpecularAlpha]
7 Interpolation   8 Water   9 WaterSimple   10 White   11 WhiteAlpha
12 AlphaShadow [PSReflect]   13 AlphaNoShadow [PSReflectNoShadow]   14 Dummy0 (no shader)
15 DetailMacroAS [PSDetailSpecularAlphaMacroAS]   16 NormalMapMacroAS
17 NormalMapDiffuseMacroAS [PSSpecularNormalMapDiffuseMacroAS]
18 NormalMapSpecularMap   19 NormalMapDetailSpecularMap   20 NormalMapMacroASSpecularMap
21 NormalMapDetailMacroASSpecularMap   22 NormalMapSpecularDIMap   23 NormalMapDetailSpecularDIMap
24 NormalMapMacroASSpecularDIMap   25 NormalMapDetailMacroASSpecularDIMap
26..40 Terrain1..Terrain15 [all use PSTerrain15]
41..55 TerrainSimple1..TerrainSimple15 [PSTerrainSimple15]
56 Glass   57 NonTL   58 NormalMapSpecularThrough [PSSpecularNormalMapSpecularThrough]   59 Grass
60 NormalMapThroughSimple [PSSpecularNormalMapThroughSimple]
61 NormalMapSpecularThroughSimple [PSSpecularNormalMapSpecularThroughSimple]
62 Road   63 Shore   64 ShoreWet   65 Road2Pass   66 ShoreFoam   67 NonTLFlare
68 NormalMapThroughLowEnd (no shader)   69..83 TerrainGrass1..15 [PSTerrainGrass15]
84..97 Crater1..Crater14   98 Sprite   99 SpriteSimple   100 Cloud   101 Horizon
102 Super   103 Multi   104 TerrainX   105 TerrainSimpleX   106 TerrainGrassX
107 Tree   108 TreePRT   109 TreeSimple   110 Skin   111 CalmWater   112 TreeAToC   113 GrassAToC
114 TreeAdv   115 TreeAdvSimple   116 TreeAdvTrunk   117 TreeAdvTrunkSimple   118 TreeAdvAToC
119 TreeAdvSimpleAToC   120 TreeSN   121 SpriteExtTi   122 TerrainSNX   123 InterpolationAlpha
124 VolCloud   125 VolCloudSimple   126 UnderwaterOcclusion   127 SimulWeatherClouds
128 SimulWeatherCloudsWithLightning   129 SimulWeatherCloudsCPU   130 SimulWeatherCloudsWithLightningCPU
131 SuperExt   132 SuperHair   133 SuperHairAtoC   134 Caustics   135 Refract   136 SpriteRefract
137 SpriteRefractSimple   138 SuperAToC   139 NonTLFlareNew   140 NonTLFlareLight
141 TerrainNoDetailX   142 TerrainNoDetailSNX   143 TerrainSimpleSNX   144 NormalPiP
145 NonTLFlareNewNoOcclusion   146 Empty   147 Point   148 TreeAdvTrans   149 TreeAdvTransAToC
150 Collimator   151 LODDiag   152 DepthOnly
```
The bracketed entries come from the shader cache. Its order matches the enum one-to-one, so the
mapping is certain except for the slots that have no shader (14, 68).

### VertexShaderID

```
0 Basic  1 NormalMap  2 NormalMapDiffuse  3 Grass  4 Dummy1  5 Dummy2  6 ShadowVolume  7 Water
8 WaterSimple  9 Sprite  10 Point  11 NormalMapThrough  12 Dummy3  13 Terrain  14 BasicAS
15 NormalMapAS  16 NormalMapDiffuseAS  17 Glass  18 NormalMapSpecularThrough
19 NormalMapThroughNoFade  20 NormalMapSpecularThroughNoFade  21 Shore  22 TerrainGrass  23 Super
24 Multi  25 Tree  26 TreeNoFade  27 TreePRT  28 TreePRTNoFade  29 Skin  30 CalmWater  31 TreeAdv
32 TreeAdvTrunk  33 VolCloud  34 Road  35 UnderwaterOcclusion  36 SimulWeatherClouds
37 SimulWeatherCloudsCPU  38 SpriteOnSurface  39 TreeAdvModNormals  40 Refract
41 SimulWeatherCloudsGS  42 BasicFade  43 Star  44 TreeAdvNoFade
```
Object vertex shaders are not compiled per ID. They are 3600 variants of one `VSShaderPool` entry, keyed
by a hash that covers ID × skinning × instancing × other flags. Terrain, TerrainGrass, Road, Water, Shore,
Sprite, Point, Star and the cloud shaders have their own entries.
`NormalMapDiffuseAlpha` appears in 28 rvmats but is not in the enum. How the engine resolves it is
unknown.

### What the data uses (rvmat count, excluding terrain layers)

| PixelShaderID | rvmats | usual VertexShaderID |
|---|---|---|
| Super | 4632 | Super (some Basic) |
| Multi | 1628 | Multi |
| TreeAdv | 380 | TreeAdv / TreeAdvModNormals / TreeAdvNoFade |
| Normal | 377 | Basic / BasicFade |
| Skin | 347 | Skin |
| TreeAdvTrunk | 174 | TreeAdvTrunk |
| NormalMapSpecularDIMap | 111 | NormalMap |
| NormalMapDetailSpecularMap, Grass, NormalMapSpecularMap, NormalMapDiffuse | 30–42 each | NormalMap / Grass |
| all others | < 15 each | |

Materials embedded in ODOL without an rvmat default to Normal/Basic. A renderer that implements
**Super, Multi, TreeAdv(+Trunk), Normal, NormalMap*, Grass and Skin** covers almost all content.

## 2. Binding conventions (all object pixel shaders)

- **Stage k → texture slot `t<k>`, sampler `s<k>`.** Stage 0 is the face texture from the model, not
  from the rvmat. Stages 1…n come from `class StageN` in the rvmat. `t16` is the engine's SSAO +
  caustics buffer, and `t15` is the shadow map in the SSSM/SR variants (§5).
- **Stage UVs are packed two per interpolator.** Stage 2j uses `TEXCOORD{j}.xy` and stage 2j+1 uses
  `TEXCOORD{j}.wz`. So stage 0 uses `TEXCOORD0.xy` and stage 1 uses `TEXCOORD0.wz` …; stage 7 uses
  `TEXCOORD3.wz`. Each stage's `uvSource`/`uvTransform` (`TexGen`) is applied in the VS.
- **Constant buffers** (names from the bytecode):
  - `PSCB_NonFrequent` cb0: per frame. `PSC_FogColor`, `PSC_WLight` (sun direction toward the light,
    world axes), `PSC_InvW_InvH_X_X`, `PSC_WaterFogColor`, `PSC_CausticsPars[3]`,
    `PSC_SSAOCausticsScale`, `PSC_HazePars`, `PSC_WaterLightExtinctionCoefs`,
    `PSC_WaterDiffuseLightExtinctionCoefs`, `PSC_WaterFogGradientCoefs`.
  - `PSCB_Special1` cb1: `PSC_GlassEnvColor` (env-map tint).
  - `PSCB_Object1` cb5: per material/light. `PSC_DiscretizeAlpha`, `PSC_AE`, `PSC_GE`, `PSC_DForced`,
    `PSC_Diffuse`, `PSC_LDirectionGround_DiffuseBack`, `PSC_Specular` (.w = specular power),
    `PSC_MatSpecular`, `PSC_Emissive`, `PSC_AmbientMid`, `PSC_PhysicalFog`, `PSC_FogMode`,
    `PSC_FogEnd_RCPFogEndMinusFogStart_WaterHeight_ExpCoef`, `PSC_CamUnderwater`.
  - `PSCB_Object2` cb6: `PSC_GlassMatSpecular`, `PSC_MaxColor` (texture colour scale from the PAA
    `MAXC` tag), `PSC_Shadow_Factor_ZHalf`, `PSC_AlphaTest`, `PSC_NightEmissiveCoef`,
    `PSC_CameraWorldPosition`.
  - `PSCB_Tree` cb3: `PSC_TreeCrownAlphaCoef`, `PSC_TreeAdvPars[5]`.
  - `PSCB_Lights1` cb10 and `PSCB_Lights2` cb11: point/spot lights (§3.4).
- **Output is premultiplied by alpha:** `rgb = albedo·light·α + specular`, `a = α`.
- **Register names below (`v1`, `v5`, …) are those of `PSSuper`.** Match other shaders by
  semantic:

  | Register | Semantic | Contents |
  |---|---|---|
  | `v1` | TEXCOORD6 | rgb = VS ambient add, a = alpha multiplier |
  | `v2` | TEXCOORD7 | rgb = VS specular, w = sun visibility |
  | `v3` | TEXCOORD4 | |
  | `v4` | TEXCOORD5 | shadow coordinates |
  | `v5` | COLOR0 | |
  | `v6` | COLOR1 | xyz = world position, w = world height |
  | `v7` | TEXCOORD10 | VS point lights |
  | `v8`–`v11` | TEXCOORD0–3 | stage UVs |

The CPU-side mapping from rvmat `ambient/diffuse/forcedDiffuse/emmisive/specular/specularPower` and the
CfgWorlds light colours to `PSC_AE/GE/AmbientMid/Diffuse/DForced/Emissive/Specular` is not traced yet
(lighting topic). The names suggest:
- `AE` = sky ambient × material ambient;
- `GE` = ground ambient;
- `Diffuse` = sun × material diffuse;
- `DForced` = forcedDiffuse;
- `Specular` = sun × material specular, with `.w` = specularPower.

## 3. Lighting model (shader)

### 3.1 Per-pixel family: Super, SuperExt, Multi, Skin, Road

The interpolators `v5`, `v11`, `v3` are the rows of the tangent→world rotation. `.w` is the pixel position
relative to the camera, so the camera is at the origin:

```
n   = normalize(float3(dot(v5.xyz, nt), dot(v11.xyz, nt), dot(v3.xyz, nt)))   // world axes, y up
P   = float3(v5.w, v11.w, v3.w);  V = normalize(-P);  L = PSC_WLight.xyz;  H = normalize(V + L)
```
Tangent normal from `_nohq` (`c` = texel): `nt = float3(2*(c.r - c.a) + 1, 2*c.g - 1, 2*c.b - 1)`. This
handles both plain RGB (`a = 1` → `2r - 1`) and alpha-swizzled storage.

```
NdotL = dot(n, L)
spec  = (NdotL >= 0 && NdotH >= 0) ? min(pow(NdotH, power), 1) : 0      // power = Specular.w * smdi.b
// hemisphere ambient on n.y:
amb   = n.y > 0 ? lerp(AmbientMid, AE, saturate(n.y))
                : lerp(GE, AmbientMid, saturate(1 + n.y))
amb  += Emissive.rgb + LDirectionGround_DiffuseBack.rgb * max(-NdotL, 0)  // back/ground bounce
amb   = amb * aoAmbient + v1.rgb                                           // v1.rgb: VS ambient add
sunD  = Diffuse.rgb * max(NdotL, 0) * (1 - reflectivity) + DForced.rgb
sunS  = Specular.rgb * spec * reflectivity
```
- **Super:** `reflectivity = smdi.g * fresnel(N·V)`. It also adds an environment reflection
  `env.rgb * exp2(4*(1 - env.a)) * GlassEnvColor.rgb * GlassMatSpecular.rgb * reflectivity * 2`.
  - The env map is sampled at `R = reflect(-V, n)`, `uv = (R.x*0.5 + 0.5, -R.y*0.5 + 0.5)`, with an
    explicit mip `8 * pow(1 - saturate(power*0.001), 10)`.
  - The fresnel value is the **alpha** of stage 6 at `u = dot(n, R)` (= N·V), `v = 0.5`.
- **NormalMap*SpecularDIMap / SpecularMap:** see §4. These are vertex-lit for point lights (below), but
  take the sun per pixel in tangent space: `v5` = L, `v11` = H and `v3` = world-up, all in tangent space.

Combine (all object shaders, after point lights `ptD`/`ptA`/`ptS`):

```
alpha  = max(saturate(D.a * MaxColor.a * v1.a), DiscretizeAlpha.w)
depthW = max(waterHeight - v6.w, 0)                       // v6.w: world height; waterHeight = cb5[12].z
amb   *= exp(-depthW * WaterLightExtinctionCoefs)         // underwater only (depthW > 0); env likewise
sunD  *= exp(-depthW * WaterDiffuseLightExtinctionCoefs); sunS likewise
(ssao, caustics) = t16.Load(pixel * SSAOCausticsScale.xy)
sunD  += caustics * sunD * CausticsPars[1].w * alpha
amb   *= lerp(1, ssao, alpha)
sunVis = saturate(aoSun * v2.w)                           // v2.w: VS light visibility; SSSM: §5
rgb    = albedo * (ptD + ptA + amb + sunD*sunVis) * alpha + sunS*sunVis + env + ptS
```
After this the fog switch runs on `PSC_FogMode`: 0 = none, 1/3 = full height+haze fog blended toward
`FogColor`/`WaterFogColor`, 2 = fog transmittance only (multiplies the output, for additive blending).
The formulas belong to the atmosphere topic.

### 3.2 Vertex-lit family: Normal, NormalDXTA, Detail, Grass, Glass

The VS computes the lighting terms: `v11.w` = N·L, `v11.z` = n.y, `v2.rgb` = specular colour,
`v7.rgb` = point lights. The PS uses the same ambient/sun formulas with those values and no normal map.
`Normal` (`PSSpecularAlpha`) also adds `2·|v2.rgb|²` to alpha, so speculars stay visible on transparent
surfaces.

### 3.3 Albedo

```
albedo = D.rgb * MaxColor.rgb               // D = stage 0 (face texture)
```
Then each shader applies its own detail/macro steps (§4).

### 3.4 Point and spot lights (Super, Multi, Road, Skin)

`PSC_PointLoopCount.x` point lights come first, then `PSC_SpotLoopCount.x` spot lights. Each light is
6 float4 in `PSC_LPSData`:

| idx | content |
|---|---|
| 0 | `.xyz` position (camera-relative) |
| 1 | spot: `.xyz` direction, `.w` cos-cutoff |
| 2 | `.xyz` colour; spot `.w` = 1/(1 − cos-cutoff) scale |
| 3 | `.xyz` ambient colour; spot `.w` = falloff exponent |
| 4 | `.x` start distance, `.yzw` attenuation (constant, linear, quadratic) |
| 5 | `.x` fade start, `.y` 1/fade length |

```
d   = |pos - P| * LightDistScale.x;   dd = max(d - L4.x, 0)
att = saturate(1 / dot(L4.yzw, (1, dd, dd²))) * (1 - saturate((d - L5.x) * L5.y))
spot= pow(saturate((dot(-dir, Ldir) - L1.w) * L2.w), L3.w)            // spot lights only
ptD += MatDiffuse.rgb * L2.rgb * max(N·Ldir, 0) * att;  ptA += L3.rgb * att
ptS += MatSpecular.rgb * L2.rgb * spec(N·Hl) * reflectivity * att
```

## 4. Per-shader stage meanings (shader; texture suffixes as used in the data)

UV notation: `s0` means stage 0's UV, and so on.

**Super** (`PSSuper`):

| Stage | texture | use |
|---|---|---|
| 0 | `_co`/`_ca` | albedo, alpha |
| 1 | `_nohq` | normal |
| 2 | `_dt` | `albedo ×= 2·dt.rgb` (after macro) |
| 3 | `_mc` | `albedo = lerp(albedo, mc.rgb, mc.a)` |
| 4 | `_as` | `aoAmbient = lerp(pow(as.a, 2.2), as.g, as.r)`, `aoSun = lerp(as.g, as.b, as.r)` |
| 5 | `_smdi` | `.g` specular intensity, `.b` gloss (× `specularPower`) |
| 6 | `#(ai,64,64,1)fresnel(n,k)` | fresnel lookup (alpha), see §6 |
| 7 | `_sky`/`_env` | environment map, alpha = HDR exponent |

**SuperExt**: same as Super, plus stage 8 (sampled with stage 0's UV) as an emissive map. Ambient gains
`t8.rgb * NightEmissiveCoef.w * Emissive.w`. **SuperAToC** and **SuperHair***: Super with
alpha-to-coverage / hair. Not dumped in detail.

**Multi** (`PSMulti`): four layers blended by a mask.

| Stage | texture | use |
|---|---|---|
| 0–3 | layer `_co` ×4 (own UVs) | `C = lerp(lerp(lerp(C0·MaxColor, C1, m.r), C2, m.g), C3, m.b)` |
| 4 | `_mask` (`m`) | layer weights r, g, b |
| 5–8 | layer `_dtsmdi` ×4 (stage 8 uses stage 3's texGen) | blended like the colours. `.r` = detail (`albedo = 2·dtsmdi.r·C`), `.g` = spec intensity, `.b` = gloss |
| 9 | `_mc` (mask UV) | `C = lerp(C, C·clamp(mc.rgb / Cavg, 0, 2), mc.a)`. `Cavg` = the same layer blend sampled at mip 20 (the average colour of each layer) |
| 10 | `_ads`/`_as` (mask UV) | AO as in Super |
| 11–14 | layer `_nohq` ×4 | normals, blended with the mask |

Multi has no fresnel or env. Its sun specular uses `reflectivity = dtsmdi.g`, and its alpha is `v1.a`
only. Multi and Road light in **tangent space**:
- The VS passes the tangent-frame rows (`v5`, `v2`, `v3`, normalized in the PS).
- The PS projects `-PSC_LDirectionTransformedDir`, `PSC_UDirectionTransformedDir` (up) and
  `PSC_CameraPosition - pos` (model-space position in `TEXCOORD`) onto those rows.

**NormalMap family** (sun per pixel in tangent space; point lights per vertex). Stage 0 = albedo,
stage 1 = `_nohq`:

| ID | stage 2 | stage 3 | stage 4 | stage 5 |
|---|---|---|---|---|
| NormalMap | – | engine lookup (§6) | | |
| NormalMapSpecularMap | `_sm`: `sunD ×= sm.r`, spec int `sm.g`, gloss `sm.b` | | | |
| NormalMapSpecularDIMap | `_smdi`: int `.g`, gloss `.b`, `sunD ×= 1 - .g` | | | |
| NormalMapDetailSpecularDIMap | `_dt` (×2) | `_smdi` | | |
| NormalMapMacroASSpecularDIMap | `_mc` (lerp by `.a`) | `_as` | `_smdi` | |
| NormalMapDetailMacroASSpecularDIMap | `_dt` | `_mc` | `_as` | `_smdi` |
| NormalMapDiffuse | `_dt` (×2), spec from VS | | | |

`NormalMap` (no specular map) samples stage 3 at `(N·L, N·H)` → `rgb` diffuse factor, `a` specular
factor. Specular is masked by the normal map's alpha. Stage 3 is not in the rvmat, so it is likely an
engine-made lighting lookup (unverified).

**Normal** (`PSSpecularAlpha`): stage 0 only, vertex-lit. **Detail**: stage 1 = `_dt` (×2).
**NormalDXTA**: alpha is snapped to 0/1, and the colour under transparent texels is filled from `cb4[0]`.

**TreeAdv / TreeAdvTrunk** (`PSCB_Tree`):

| Stage | texture | use |
|---|---|---|
| 0 | `_ca`/`_co` | albedo; crown alpha test `D.a·v1.a·TreeCrownAlphaCoef.x > 0.5` (else discard), output alpha 1 |
| 1 | `_nohq` | normal (same decode as Super) |
| 2 | `_mca` / `#(argb,8,8,3)color(…,MCA)` | `albedo ×= mca.rgb · 4.5947` (= 2^2.2: neutral sRGB 0.5 → ×1). `mca.a` = AO: `ao = saturate(a·TreeAdvPars[1].z + TreeAdvPars[1].w)` |

- **Wrapped diffuse:** `saturate(N·L·P[0].z + P[0].w)`.
- **Translucency:** `saturate(N·L·P[1].x + P[1].y)·ao·v9.z`. It is weighted by `P[2]`/`P[3]`
  (`TreeAdvPars` = cb3[2..6]).
- **Rim/reflection term:** `pow(saturate(1 - 0.8·N·V), 4)·P[4].rgb`.
- **Ambient:** uses the hemisphere evaluated at `n·up·0.5 + 0.5`.

Fog is computed inline, not through the FogMode switch. TreeAdvTrunk is the same without the alpha test
or translucency.

**NormalMapThrough / NormalMapSpecularThrough** (bushes; `PSSpecularNormalMap[Specular]Through`):
- Stage 0 = albedo. Stage 1 = normal (`2c - 1`).
- Stage 2 `.a` scales the sun term, which includes the back-lit term `max(-N·L, 0)·LDirectionGround_DiffuseBack.w`.
- Stage 3 `.rgb` multiplies the VS ambient `v1.rgb`.
- Alpha test against `TreeCrownAlphaCoef.x`, offset by the normal-map alpha.

**Grass**: stage 0 × `v9.rgb` (VS colour), vertex-lit, alpha test at 0.5.

**Glass**:
- Stage 0 = colour/alpha. Pixels with `alpha·MaxColor.a < 1/255` are discarded.
- Stage 1 = fresnel (alpha at N·V).
- Stage 2 = environment map, same reflect UV as Super.
- `spec = (env·GlassEnvColor·GlassMatSpecular·2 + v2.rgb) · fresnel`.

**Skin**:
- Stage 0 = albedo, 1 = `_nohq`, 2 = macro (`lerp` by `.a`), 3 = a second colour map sampled with stage 0's UV.
- 4 = `_as`, 5 = `_smdi`, 6 = fresnel.
- Subsurface-style wrap lighting uses cb2 parameters. It is not decoded further.

**Road**: stage 0 albedo × 2·stage 2 (detail). Its interpolator registers are shifted, so match them by
semantic. Stage 1 = normal, stage 3 = `_smdi` with
`sunD ×= .r`. It has per-pixel point lights like Super.

## 5. Variants of each shader

Every object PS ID has up to 10 entries:

| Suffix/prefix | meaning |
|---|---|
| `PS<X>` | base. Variant hash `61F433D2` |
| `PS<X>_AlphaTest` | hash `4C03596E`. If `PSC_AlphaTest.x > 0` and `alpha < PSC_AlphaTest.y`, discard |
| `PSSSSM<X>` | screen-space shadow mask: `sh = t15.SampleLevel(pixel·InvW_InvH, 1)`. `f = saturate((v4.w - Shadow_Factor_ZHalf.w) / Shadow_Factor_ZHalf.w)`. `vis = lerp(sh, aoSun, f)`, then `sunVis = 1 + Shadow_Factor_ZHalf.x·(vis·v2.w - 1)` |
| `PSSR<X>_Default` | the same with a real shadow map: `t15.SampleCmpLevelZero(v4.xy, v4.z)` (VS shadow coordinates) |
| `PSThermal<X>` / `<X>Thermal` | thermal imaging (`PSCB_TI` cb7, t14/t15) |
| `PSDEBUGSHD<X>` | shader debug |
| `PSAlphaOnly*`, `PSDepthOnly`, `PSDepthTerrain`, `PSShadowBufferAlpha` | depth/shadow passes |

## 6. Engine-made textures

**`fresnel(n,k)`**: `TextureSourceFresnel`, generator `0x1410b88e0`. A 1D table. For texel
`i` of `w`, `x = i/(w-1) = cosθ`. It holds the unpolarised Fresnel reflectance of a conductor with
complex index `n + ik`:

```
t  = n² - k² - sin²θ;   s = sqrt(t² + 4n²k²);   a = sqrt((s + t)/2)
Rs = (s - 2a·cosθ + cos²θ) / (s + 2a·cosθ + cos²θ)
Rp = Rs · (s - 2a·sinθ·tanθ + sin²θ·tan²θ) / (s + 2a·sinθ·tanθ + sin²θ·tan²θ)   // Rp = Rs at i = 0
texel = clamp(round((Rs + Rp) · 127.5), 0, 255)
```
`n ≤ 0` or `k ≤ 0` is an error ("Fresnel n must be >0"). Format `ai` stores the value in alpha (and
intensity), and shaders read `.a`. `FresnelGlass` is a separate source and has not been decoded.

**Texture type from the file suffix** (`0x1410bb490`; used by the loader and by procedural
`color(…,TAG)`):

| Type | Suffixes |
|---|---|
| 0 | `_co`, `_ca`, unknown suffixes, and the terrain satellite `s_*_lco` |
| 1 | `_sky`, `_lco` (other than satellite) |
| 2 | `_detail`, `_cdt`, `_dt`, `_mco` |
| 3 | `_no`, `_non`, `_nopx`, `_noex`, `_nohq`, `_novhq`, `_nofhq`, `_nof`, `_nofex`, `_ns`, `_nsex`, `_nshq`, `_normalmap` |
| 7 | `_mc` |
| 8 | `_as` |
| 9 | `_sm`, `_smdi` |
| 11 | `_dtsmdi` |
| 12 | `_mask` |
| 13 | `_ti_ca` |

The D3D11 texture creator (`0x1416d7e10`) makes both a linear SRV and an `_SRGB` SRV for every
texture. Which one each stage binds has not been traced. The shaders give two clues:
- TreeAdv scales MCA by 2^2.2, so that stage is read sRGB-decoded.
- Super/Multi treat `_dt` with a plain ×2 and decode `_as` alpha with an explicit `pow(·, 2.2)`, so
  those stages are read linear.

A renderer should decode the colour types (0, 1, and MCA) as sRGB and read the data types (2, 3, 8, 9,
11, 12) as linear. This is medium confidence, pending the CPU trace.

## 7. Open points

- CPU mapping of rvmat colours and CfgWorlds lighting to the `PSC_*` constants (lighting topic).
- Per-stage sRGB view selection (see §6).
- `VSShaderPool` variant key layout, and what each VertexShaderID changes: skinning, instancing,
  `*NoFade`, `TreeAdvModNormals`.
- Skin subsurface constants (cb2) and the `FresnelGlass` generator.

## 8. Our implementation (`a3-render-models`)

How the model renderer maps the above onto wgpu. It is engine-side, not RE: where the renderer
departs from the shaders above, this section says so.

**Families.** `PixelShader::family` (`crates/a3-render-models/src/shader.rs`) groups the IDs:

| family | IDs | WGSL path |
|---|---|---|
| Basic | Normal, NormalDXTA, Detail, Interpolation(Alpha), White(Alpha), AlphaShadow, AlphaNoShadow, DetailMacroAS | colour map only, lit per pixel normal (the engine lights these per vertex) |
| Super | Super, SuperExt, SuperHair, Skin, NormalPiP, NormalMap* | §3.1 / §4 Super; NormalMap* stages recognised by suffix |
| SuperAlphaTest | SuperAToC, SuperHairAtoC | Super, alpha test instead of alpha-to-coverage |
| Multi | Multi | §4 Multi: layer colours, mask, per-layer `_dtsmdi`, macro over the mip-20 layer average, AS, per-layer normals |
| Tree | Tree*, Grass, GrassAToC, NormalMapGrass | §4 TreeAdv: `albedo × mca.rgb × 4.5947`, `mca.a` as ambient AO, alpha test 0.5, two-sided |
| Glass | Glass, Refract | §4 Glass: fresnel + env, premultiplied blend |
| Unsupported | water, terrain, sky, clouds, sprites | sections skipped |

The real-data test (`crates/a3-render-models/tests/real_data.rs`) checks the 963 models placed on
Altis: 8,703 sections (Super 6,427, Multi 1,445, Basic 503, Tree 263, SuperAlphaTest 65), none
unsupported, no missing texture.

**Stages to bindings.** The engine binds stage k to `t<k>`. Multi needs 15 stages, but wgpu's
default limit is 16 sampled textures per stage. So a material binds 15 textures, and slots that no
family uses together share a binding (`Slot::binding`): t0 colour/layer 0, t1 normal/layer 0
normal, t2 smdi/layer 0 dtsmdi, t3 AS, t4 macro (`_mca` for trees), t5 detail/mask, t6
fresnel/layer 1, t7 env/layer 2, t8 layer 3, t9–t11 layer normals 1–3, t12–t14 layer dtsmdi 1–3.
- Super, SuperExt, SuperAToC, SuperHair*, Skin and Glass stages are taken by position (§4).
  NormalMap* share the Super code path but have their own layouts, so they go by suffix.
- Multi stage 8 uses stage 3's tex gen.
- The other shaders' stages are taken by Texture suffix (`_nohq`, `_smdi`/`_sm`, `_as`,
  `_mc`/`_mca`, `_dt`/`_cdt`).
- Empty slots get 1×1 neutral textures per family: flat normal, no specular, transparent macro,
  0.5 detail, neutral `_mca`.

**Colour spaces** follow §6: colour, layer, macro and env maps are sRGB; the rest are linear.

**Approximations, pending the open points of §7:**
- Hemisphere ambient: a3-render's sky colours stand in for `AE`/`AmbientMid`/`GE`.
- The env-map tint `GlassEnvColor × GlassMatSpecular` is replaced by sky level × material specular.
- `DForced`, point/spot lights, SSAO/caustics, underwater extinction and the FogMode switch are
  not implemented. Fog and haze come from a3-render's post pass.
- TreeAdv `TreeAdvPars` (wrap, translucency, rim) are unknown. We use a fixed wrap
  (`N·L·0.5 + 0.5`), no translucency and no rim term.
- Alpha: Glass blends; Tree and SuperAlphaTest test; for other families the colour map's PAA
  `FLAG` decides (interpolated alpha blends, binary alpha tests). Blended sections are not yet
  sorted back to front within a frame.
- Sun shadows: opaque and alpha-tested sections cast within the shadow distance. Receivers use
  a3-render's cascades (`sun_visibility`), multiplied by the AS `aoSun` term.

**LOD selection** (`lod.rs`) follows `render-lod.md`. These parts are the engine's:
- the objects-quality coefficients and their distance curve;
- the object size;
- the draw/fade test (a dithered fade per instance);
- the shadow test.

The global area multiplier and the per-LOD index are named stand-ins (`LodSelector::area_scale`,
`stand_in_lod_index`) until §5 of that page is decoded.
