# Terrain surfaces: world config, surface types and layer materials

Implemented in `crates/a3-landscape`. Complements `docs/re/wrp.md`. Confidence: **verified**
(checked on all six readable worlds of build 2.22.0.154103), **high**, **medium**, **low**.

## World config (`CfgWorlds >> <world>`)

A world class is a `CfgWorlds` class with a non-empty `worldName` (the `.wrp`). The six readable
worlds are `Altis`, `Stratis`, `VR`, `Tanoa`, `Malden`, `Enoch`. **Verified**.

Entries `a3-landscape` reads (inheritance resolved): `worldName`, `description`, `author`,
`mapSize` (equals the WRP's land size x cell size on every world that sets it; VR does not),
`centerPosition`, `longitude`, `latitude`, `elevationOffset`, `startTime`, `startDate`,
`soundMapSizeCoef` (4 on every world, 1 in `DefaultWorld`; the WRP sound map has
`land * coef` cells per side, **verified**), `newRoadsShape` (roads shapefile), `pictureMap`,
`class Grid` (map grid references: `offsetX/Y`, `ZoomN` with `stepX/Y`, `format*`),
`class OutsideTerrain` (`satellite`, `enableTerrainSynth`, `Layers >> X >> nopx/texture`,
`colorOutside`), `outsideHeight`, `minHeight`, `satelliteNormalBlendStart/End`,
`midDetailTexture`, `clutterGrid`, `clutterDist`, `noDetailDist`, `fullDetailDist`,
`class clutter` (models; missing values come from `class DefaultClutter`), sky
(`skyObject`, `horizontObject`, `skyTexture`, `skyTextureR`, `starsObject`, `sunObject`,
`moonObject`, `haloObject`, `rainbowObject`, `pointObject`, `clouds[]`, `class EnvMaps`),
the names of the lighting classes (`Lighting`, `DayLighting*`), water (`class Sea`,
`class Underwater`, `waterTexture`, `seaBedUnderwaterDepth`, `shoreTop`, `peakWave*`),
`class AmbientA3` (spawn rings and species) and `class Names` (locations).

## Surface types

`CfgSurfaces >> X`: `files` is a file-name pattern (`*` wildcard, case-insensitive) matched
against the **file name** of a layer's detail colour texture (`gdt_seabed_co.paa` matches
`gdt_seabed_*`). Every layer texture of all six worlds matches a surface pattern
(**verified**: 19 on Altis, 10 Stratis, 1 VR, 10 Tanoa, 9 Malden, 8 Livonia). `character`
names a `CfgSurfaceCharacters` class: parallel arrays `names[]` (clutter classes of the
world's `class clutter`) and `probability[]`. 145 surfaces and 36 characters in the merged
config.

## Layer materials (`p_XXX-YYY_*.rvmat`)

The WRP material list names one rvmat per satellite tile (see `wrp.md`). Each is a rapified
config. Layout (**verified** on all 30,173 materials of the six worlds; all parse):

| Stage | Texture | TexGen | Meaning |
|---|---|---|---|
| 0 | `s_XXX_YYY_lco.paa` | 3 | satellite colour tile |
| 1 | `m_XXX_YYY_lca.paa` | 4 | layer mask tile (same transform as stage 0) |
| 2 | `#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)` | 0 | constant detail colour |
| 3 + 2k | `<surface>_nopx.paa` or `""` | 1 | layer slot k normal/parallax |
| 4 + 2k | `<surface>_co.paa` or `""` | 2 | layer slot k detail colour |
| 14 | `n_XXX_YYY_nohq.paa` | 3 | satellite normal tile (TerrainSNX only) |

- Slots: `k` in 0..4 (stages 3-12) in every shipped material; the slot names follow
  `p_XXX-YYY_` in the file name (`p_012-028_n_n_l05_n_n`: only slot 2 is filled, `n` = empty).
  Empty slots have `texture = ""`.
- `PixelShaderID`: `TerrainSNX` (Altis, Stratis, VR, Tanoa; with stage 14) or `TerrainX`
  (Malden, Livonia; no stage 14). `VertexShaderID = "Terrain"`.
- `XXX` is the tile column from the west, `YYY` the row from the **north**.

### UV transforms

`class TexGenN { uvSource; class uvTransform { aside[]; up[]; dir[]; pos[]; } }`.
`a3-landscape` applies `uv = aside * p.x + up * p.y + dir * p.z + pos` and takes `(uv.x, uv.y)`.
For `uvSource = "worldPos"` `p` is the world position (x east, y up, z north). Example (Altis
`p_006-063`): `aside = (1/512, 0, 0)`, `dir = (0, -1/512, 0)`, `pos = (-5.59375, 0.96875, 0)`,
so `u = x / 512 - 5.59375`, `v = 0.96875 - z / 512`: the tile covers 512 m starting 16 m before
its 480 m stride (overlapping neighbours). **Verified**: the centre of every sampled land cell
(64 x 64 per world) maps into `[0, 1]` of both its satellite and mask tile.

`uvSource = "tex"` (stages 2-12): `aside = (5,0,0), up = (0,5,0)` for the layer stages, identity
for stage 2. What the terrain's own texture coordinates are (per land cell? per metre?) is
**low**: needs the `Terrain` vertex shader or the landscape mesh builder.

## Open questions

- How the `_lca` mask colours select layer slots (TerrainSNX / TerrainX pixel shaders).
- The terrain mesh's `tex` coordinates (scale of the detail layers).
- Which surface wins when several `files` patterns match (first in config order assumed; no
  overlap occurs in shipped data).
