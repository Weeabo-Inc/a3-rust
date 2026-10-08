# Terrain rendering (satellite, mask, detail layers, LOD)

How Arma 3 2.22 shades the Landscape. Sources:
- the engine's compiled shaders from `Dta\bin.pbo` (`Shaders_5_0_{VS,PS}.shdc`). Extract and disassemble them with `tools/re/shdc.py` (D3DDisassemble from the Windows `d3dcompiler_47.dll`). Shader names, constant-buffer names and register layouts are preserved in the bytecode.
- the Altis layer rvmats in `map_altis_data_layers.pbo` and `CfgWorlds >> Altis`;
- the engine code that sets the terrain constants (`arma3_x64.exe`, RVAs).

**Confidence:** high where the text says "shader". Those parts are transcribed from the
disassembly of `PSTerrainSNX` / `VSTerrain`, the shaders that Altis tiles use. Medium where noted (CPU-side
values inferred from register names).

```sh
python tools/re/shdc.py list P:/a3-rust/.work/shaders/Shaders_5_0_PS.shdc Terrain
python tools/re/shdc.py dump P:/a3-rust/.work/shaders/Shaders_5_0_PS.shdc "^PSTerrainSNX$" out/
```
(`.work/shaders/` holds the caches extracted from `Dta\bin.pbo`; not committed.)

## 1. Data: one rvmat per terrain tile

`a3\map_altis\data\layers\p_XXX-YYY_<l0>_<l1>_<l2>_<l3>_<l4>.rvmat` (14,734 files for Altis).
The five suffixes name the global surface layers used in the tile. An `n` suffix marks an unused slot, for example
`p_020-020_n_l10_l14_l15_n`. A typical land tile:

| Stage | Texture | texGen | Shader slot |
|---|---|---|---|
| 0 | `layers\00_00\s_020_020_lco.paa` satellite colour | 3 (worldPos) | `t0` |
| 1 | `layers\00_00\m_020_020_lca.paa` layer mask | 4 (worldPos) | `t1` |
| 2 | `#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)` | 0 | `t2` |
| 3,5,7,9,11 | layer k `gdt_*_nopx.paa` normal+parallax | 1 (×5) | `t3,t5,t7,t9,t11` |
| 4,6,8,10,12 | layer k `gdt_*_co.paa` detail colour | 2 (×5) | `t4,t6,t8,t10,t12` |
| 14 | `layers\00_00\n_020_020_no.paa` satellite normal map | 3 | `t14` |

`PixelShaderID = "TerrainSNX"`, `VertexShaderID = "Terrain"`. Seabed tiles use the same
layout with a single layer (`gdt_seabed_*`). Placeholder tiles use `a3\map_data\tiled_{s,m}_co.paa`.

Satellite images on Altis (**verified** with `a3-landscape-render`'s real-data test):
- 4,096 tiles (64 x 64). 1,863 tiles name their own `s_XXX_YYY_lco.paa`, normally 512 x 512
  DXT1 with 8 mips (512 down to 4). 57 of them, along the coast, have no 64 px mip. The one
  checked (`00_01\s_003_038_lco.paa`) is a 4 x 4 single-mip placeholder.
- The remaining 2,233 open-sea tiles all name `a3\map_data\tiled_s_co.paa` (32 x 32 DXT1,
  average colour `#4c4946`) with the normal per-tile transform.
- Tiles exist only where land cells reference them; their `s_` files are spread over
  `map_altis_data_layers_{00,01}_{00,01}.pbo` by quadrant.

### Satellite/mask UV and tile overlap

`TexGen3/4`: `uvSource = "worldPos"`, `aside = {1/512, 0, 0}`, `up = {0, 0, 1/512}`,
`dir = {0, -1/512, 0}`, `pos = {pu, pv, 0}`. With world position (x, height, z):

```
u = x / 512 + pu          v = -z / 512 + pv
```

Example tile 020_020: `pu = -18.71875`, `pv = 41.28125`, so the texture spans x = 9584…10096 m
and z = 21136…20624 m. Altis is 30720 m with 64 × 64 tiles, so tiles are placed every **480 m**. Each 512 m
satellite/mask texture therefore overlaps its neighbours by **16 m on every side**
(`pu = -(480·i - 16)/512`, `pv = (30720 - 480·j + 16)/512`; the v axis points south). The mask
uses the same transform as the satellite, so the two always align.

### Detail UV

`TexGen1/2`: `uvSource = "tex"`, scale 5. The vertex shader's "tex" coordinate is the **integer
terrain-grid vertex index** (`TEXCOORD0` is an `int2`). Detail textures therefore repeat 5 times per
terrain grid cell (Altis grid cell 7.5 m → one detail repeat every 1.5 m). `TexGen0` (stage 2)
maps once per cell.

## 2. Vertex shader `VSTerrain` (shader, high)

Inputs: `POSITION0` (x, h, z, w), packed `NORMAL`/`TANGENT0`/`TANGENT1` (`ubyte*2/255-1`), `TEXCOORD0`
int2 grid index, `POSITION1.xy` and `POSITION2.xyzw` (heights for LOD levels 1–6), `TEXCOORD2.x`.

**Continuous LOD (geomorphing):** every vertex carries 7 heights, one per terrain LOD level:

```
d2   = |cameraPos.xz - pos.xz|²
lod  = clamp(0.5*log2(d2) + LODPars.x + LODPars.y, 0, LODPars.y)    // VSC_TerrainLODPars (cb8[2])
w_k  = max(0, 1 - |lod - k|)   for k = 0..6
h    = w0*pos.y + w1*P1.x + w2*P1.y + w3*P2.x + w4*P2.y + w5*P2.z + w6*P2.w
```
So the LOD level rises by one per doubling of horizontal distance. `LODPars.x` is a bias and
`LODPars.y` the coarsest level. The heights for each level, and the mesh/segment layout on the
CPU, are follow-up work (CPU side of `Landscape`).

Outputs used by the PS:
- `TEXCOORD4.xy` (`v3`): satellite/mask UV = TexTransform[4] · worldPos.
- `TEXCOORD0` (`v8`): stage-2 UV.
- `TEXCOORD1` (`v9`): detail UV = TexTransform · (gridIndex, 1).
- `TEXCOORD2` (`v10`): normal.
- `TEXCOORD3` (`v11`): `cameraPos - pos`.
- `COLOR0` (`v5`): light direction in tangent space.
- `COLOR1` (`v6`): world position.
- `TEXCOORD8` (`v7`): view-space position, used for fog.
- `TEXCOORD6.w` (`v1.w`): **detail weight** `saturate((AFogEnd - dist) * alpha * rcp(AFogEnd - AFogStart))`, with `VSC_Free_ExpFog_AFogEnd_RCPAFogEndMinusAFogStart`. AFogStart/End are taken from `fullDetailDist`/`noDetailDist` (Altis 10 m / 65 m); that link is medium confidence.

## 3. Pixel shader `PSTerrainSNX` (shader, high)

Constants (`PSCB_Terrain`, cb8, set by `0x1715e30`):

| Constant | Value |
|---|---|
| `PSC_Layers[0..5]` | `(f,f,f,f)` with `f = 1` if layer k is present (bit k of the tile's layer mask), else 0. Slot 5 is the satellite-normal flag (`enableSatNormalOnDetail` global) in the SN path. |
| `PSC_NoTexSizeLog2[0..1]` | log2 of each layer's `_nopx` texture size, for parallax mip selection. The CPU computes `ln(x)·0.7213475` = `0.5·log2(x)`, and x is most likely `width·height` (the decompiler loses the argument), which gives `log2(size)` for square textures. |
| `PSC_TerrainBlend` | `(10.0, terrainBlendMaxDarkenCoef, terrainBlendMaxBrightenCoef, 0)`. Altis: `(10, 0.85, 0.15, 0)`; without a landscape `(10, 0, 1, 0)`. |
| `PSC_TerrainDarkening` | shore darkening (`shoreDarkening*` config) |
| `PSC_TerrainSatNormDist` (cb0[0]) | `(satelliteNormalBlendStart, end - start, satelliteNormalOnDetail ? 1 : 0, 0)`. Altis: `(10, 90, 1, 0)` |
| `PSC_MaxColor` (cb6[1]) | per-material colour scale applied to the satellite |

### 3.1 Base colour (all distances)

```
S    = tex(t0, uvSat).rgb * MaxColor
base = 2 * S * tex(t2, uv2).rgb            // stage 2 is constant 0.5 grey → base = S
```

### 3.2 Layer weights from the mask (only if detail weight `v1.w > 0.01`)

`m = tex(t1, uvSat)` (RGBA), `L[k] = PSC_Layers[k].x`:

```
r  = min(1 - L0, 1)
w0 = saturate(3*L0)
t = max(r, m.r) * L1;  r = min(r, 1 - L1);  s1 = saturate(3t);  w0 *= 1 - s1;               w1 = s1
t = max(r, m.g) * L2;  r = min(r, 1 - L2);  s2 = saturate(3t);  (w0,w1) *= 1 - s2;          w2 = s2
t = max(r, m.b) * L3;  r = min(r, 1 - L3);  s3 = saturate(3t);  (w0,w1,w2) *= 1 - s3;       w3 = s3
t = max(r, m.b * saturate(2*(1 - m.a))) * L4;  s4 = saturate(3t);  (w0..w3) *= 1 - s4;      w4 = s4
```
Each layer "paints over" the earlier ones with coverage `saturate(3·channel)`. The first present
layer is the base. The weights sum to 1. The layer with the largest weight is the "dominant layer"
(used for parallax).

### 3.3 Parallax (dominant layer, within 15 m)

If `|v11| < 15`, the shader runs parallax occlusion mapping on the dominant layer's `_nopx`
height (alpha). Height scale 0.0325, fade `smoothstep` over 5…15 m, 1–24 steps chosen by view
angle, mip from `PSC_NoTexSizeLog2`. The result offsets the detail UV for all layers.
(The full step loop is in the disassembly; a simple POM with these constants matches it.)

### 3.4 Detail colour and the satellite blend

```
D = Σ w_k * tex(co_k, uvDetail)               // sampled with gradients
A = Σ w_k * texLod(co_k, uvDetail, 20)        // smallest mip = average colour of each layer
k   = clamp(1 / (S + A + 0.001), 1, TB.x)     // TB = PSC_TerrainBlend
kS  = k * S
lo  = max(kS, TB.y)                           // TB.y = maxDarkenCoef
hi  = min(kS, TB.z)                           // TB.z = maxBrightenCoef
res = lo * D * (1 - D) + D * (1 - (1 - D) * (1 - hi))
col = lerp(base, res, v1.w)                   // v1.w: detail weight (1 near, 0 beyond noDetailDist)
```
`kS` is the satellite relative to "satellite + average detail colour". Where the satellite is
darker than the layer average it darkens the detail, limited by `maxDarkenCoef`. Where it is brighter
it brightens, limited by `maxBrightenCoef`. Far away, only the satellite (`base`) is left.

### 3.5 Normals

- Satellite normal `t14`: `n.x = 2*(c.r - c.a + 1) - 1`, `n.y = 2*c.g - 1`,
  `n.z = sqrt(max(0, 1 - x² - y²))` (channels as sampled with the `.xywz` swizzle).
- Detail normal: `Σ w_k * nopx_k.ag`-style two-channel normal (`*2 - 1`, z reconstructed).
- The satellite normal is blended onto the detail normal over distance:
  `f = saturate((dist - satNormStart) / (satNormEnd - satNormStart)) * PSC_Layers[5].x`, and with
  `satelliteNormalOnDetail` the satellite normal also modulates the detail normal near the camera
  (`cb0[0].z * PSC_Layers[5].x`).
- The detail normal fades out with `v1.w` (toward `(0, 0, 1)`).

### 3.6 Lighting and fog

These are standard RV terrain lighting (sun diffuse with `PSC_Diffuse`, ambient with `PSC_AE`/
`PSC_AmbientMid`/`PSC_LDirectionGround_DiffuseBack`, point/spot lights from `PSC_LPSData`) and
the height/exponential fog of `PSCB_Object1`/`PSCB_NonFrequent`. They are documented with
lighting and atmosphere in `render-atmosphere.md` (separate topic).

## 4. Shader variants

| PixelShaderID | Use |
|---|---|
| `TerrainSNX` | Altis/Stratis tiles: 5 layers + satellite normal map (stage 14) |
| `TerrainX` | 6 layers (stage 13 = sixth `_co`), no satellite normal |
| `TerrainSimpleX`, `TerrainSimpleSNX` | without parallax |
| `TerrainNoDetailX`, `TerrainNoDetailSNX` | satellite (+normal) only, no detail layers |
| `TerrainGrassX`, `TerrainGrassAlphaX` | grass layer / grass alpha |
| `Terrain15`, `TerrainSimple15`, `TerrainGrass15` | legacy 15-texture terrains |
| `*Thermal` | thermal imaging versions |
| `DepthTerrain` | depth-only pass |

## 5. Outside the map

`CfgWorlds >> Altis >> OutsideTerrain`: `satellite = "A3\map_Altis\data\s_satout_co.paa"`, one
layer (`gdt_seabed_nopx/co`), `colorOutside[] = {0.227, 0.275, 0.384, 1}`,
`enableTerrainSynth = 0`. World-level `outsideHeight = -10` (and `outsideMaterial = ""`). The
engine reports `"OutsideTerrain config angleAltitudes wrong size, expected 512 got %d"`, so
procedural outside terrain uses a 512-entry altitude table when terrain synthesis is enabled.
The outside geometry, and how the satellite texture tiles beyond the map edge, are follow-up work.

## 6. Open points

- CPU construction of the terrain mesh: segment size, which grid points carry which LOD
  heights, and the `VSC_TerrainLODPars` values.
- Exact setter of the AFog (detail fade) constants (`fullDetailDist`/`noDetailDist`).
- Where `midDetailTexture` (`A3\Map_Data\middle_mco.paa`) is used. It is not referenced by the
  `TerrainSNX` path.
