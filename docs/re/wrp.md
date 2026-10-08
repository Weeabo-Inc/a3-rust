# WRP terrain format (OPRW)

Implemented in `crates/a3-wrp`. Every shipped terrain of build 2.22.0.154103 is version 25 and
parses to its last byte. Confidence per item: **verified** (checked against all six shipped
terrains), **high** (read from the executable), **medium**, **low**.

Sources: the executable `arma3_x64.exe` (Ghidra, addresses below are virtual addresses with image
base `0x140000000`), the community C# reader `BIS.WRP` in `Braini01/bis-file-formats` (read for
orientation only), and the shipped data.

## Executable functions

| Address       | What                                                                       |
|---------------|----------------------------------------------------------------------------|
| `0x141078fd0` | `Landscape` serialise (load and save share one function); "Bad version %d in landscape %s" |
| `0x14162f590` | Landscape grid setup: checks "Landscape/Terrain dimensions %dx%d not rectangular / not power of 2" |
| `0x141063930` | Quad tree read (root flag byte, then node or leaf)                         |
| `0x1410d0270` | Road net serialise (per land cell)                                        |
| `0x1410cfb10` | One road part serialise                                                   |
| `0x140b5fc30` | Map object factory: `switch` from map type to `MapObject*` class          |
| `0x140b60390` and the `MapObject*` vtable slot 13 | Map object record serialise   |
| `0x141655d30` | `SurfaceY(x, z)`: terrain height with triangle interpolation (called by `getTerrainHeightASL`, handler `0x1404b41d0`) |
| `0x14121f740` | Fetch the four corner heights of a height cell                             |
| `0x141051460` | Returns `Landscape+0xdac`, the sound map coefficient (`soundMapSizeCoef`)  |

Version acceptance: the loader accepts 3 and 15 to 25 (`version - 15 <= 10`). Comments in the C#
reader give the version history: 21 drops the random array, 22 adds the primary texture array,
23 switches compressed arrays to LZO, 24 adds road connection types, 25 adds the app id.

## Conventions

- Little-endian. `asciiz` = NUL-terminated string. `vec3` = 3 x `f32` (x east, y up, z north).
- **Compressed array** of N bytes: raw when N < 1024, else compressed: LZO1X (version >= 23,
  inline, no flag byte, ends with the LZO end marker) or LZSS (< 23) _(LZSS checksum kind:
  low, assumed unsigned; no such file ships)_. **Verified** for LZO.
- **Grid order**: arrays over a grid are row-major, index `z * width + x`, `z = 0` at the south
  edge. Height sample `(i, j)` sits at world `(i * cell, j * cell)`. **Verified** (objects stand
  on the surface: median gap 0.9 m; transposed gives 50-90 m).
- Land grid ("LandRange", the layer/cell grid) and terrain grid ("TerrainRange", the heightmap)
  are square powers of two. `terrain cell = land cell * land / terrain`.

## Layout (version 25)

```
char[4]  "OPRW"
u32      version                    25
u32      app id                     v >= 25: Steam app id (107410 Arma 3, 395180 Apex/Tanoa,
                                    1021790 Contact/Livonia; 0 for Malden)
u32      land range x, land range y
u32      terrain range x, terrain range y
f32      land cell size (m)
QuadTree<u16>  geography            land grid
QuadTree<u8>   sound map            (land * soundMapSizeCoef) grid, see below
u32 n; vec3[n] mountains            peak positions
QuadTree<u16>  material index       land grid, index into the material list
Compressed u16[land^2]  random      v < 21 only
Compressed u8[terrain^2] grass approximation        v >= 18
Compressed u8[terrain^2] primary texture index     v >= 22
Compressed f32[terrain^2] elevation (metres)
u32 n; { asciiz rvmat; i8 major } [n]              material list; major = -1 forced for v < 17
u32 n; asciiz[n] models                            p3d paths; object model index is 0-based
u32 n; { asciiz class; asciiz shape; vec3 pos; u32 object id } [n]   static entities, v >= 15
QuadTree<u32>  object offsets       land grid: byte offset of the cell's objects in the object block
u32      object block size          bytes, multiple of 60
QuadTree<u32>  map object offsets   land grid: byte offset into the map block
u32      map block size             bytes
Compressed u8[land^2]    persistent flags
Compressed u8[terrain^2] subdivision hints          v >= 13 (all zero in shipped files)
u32      max object id
u32      road net size              bytes of the road net that follows (v >= 7)
RoadNet  (road net size bytes)
Object[object block size / 60]
MapObject records (map block size bytes) to end of file
```

All of the above is **verified** (exact end of file on all six terrains; section sizes match
their declared byte counts).

### Quad tree

`u8 root flag`; flag 1: a node follows, flag 0: one leaf. Node: `u16 mask`, then 16 children in
row-major 4x4 order (child `i` covers `cx = i & 3`, `cz = i >> 2`); bit `i` set = node, clear =
leaf. Leaf: 4 bytes holding a tile of 2x2 `u8`, 2x1 `u16` or 1 `u32`; element `(x, z)` of the
tile is at byte `((z & mz) << lx | (x & mx)) * size`. A leaf above the bottom level repeats its
tile over its whole area. **High** (decompiled getters) and **verified** (all trees end exactly
where the next field starts).

Virtual size: `log_total = ceil(log2(n))`; leaf tile log size `(lx, lz)` = (1,1) for u8, (1,0)
for u16, (0,0) for u32; `levels = max over x,z of ceil((log_total - leaf_log) / 2)`; the root
covers `2^(leaf_log + 2 * levels)` cells per axis. Cells beyond the grid are ignored.

### Sound map size

The sound map grid is `land * soundMapSizeCoef`, a CfgWorlds value (`soundMapSizeCoef = 4` in
every shipped world; the base class has 1) read into `Landscape+0xdac`. It is not in the file.
`a3-wrp` takes 4 when the tree depth fits it, else the smallest of 1, 2, 8, 16 that fits.
**Verified** for all six terrains (tree depth matches coefficient 4).

### Geography (u16 per land cell)

Bits from least significant: `minWaterDepth:2, full:1, forest:1, road:1, maxWaterDepth:2,
howManyObjects:2, howManyHardObjects:2, gradient:3, someRoadway:1, someObjects:1`. **Medium**:
from the C# reader; consistent with data (every cell holding a tree symbol has `someObjects`;
99 % of cells with `maxWaterDepth > 0` lie below 0 m).

### Materials

Entry 0 is the empty path. Other entries are layer rvmats named
`p_XXX-YYY_lNN[_lNN...].rvmat`: `XXX` is the tile column (west to east), `YYY` the tile row
counted from the **north** edge (cell `(0, 0)` on Altis maps to `p_000-063`). The loader parses
these names (`p_`, `-` at 5, `_` at 9) to build a tile table. **Verified** naming, **high**
parsing. `major` is 0 in all shipped files; meaning unknown.

### Object (60 bytes)

`u32 id; u32 model index; f32[12] transform; u32 shape param`. Transform: 3 orientation
columns (aside, up, direction, scale included) then the position. Ids are `0..=max object id`
in shipped files. Shape param is 2 in every shipped object (meaning unknown). **Verified**.

### Static entities

Static entities (lamps, houses with config classes, ...) duplicate a placed object at the same
x/z (y differs by centimetres) with the same model. Their `object id` is encoded:
`bit 31 = 1`, bits 21-30 = land cell z, bits 11-20 = land cell x, bits 0-10 = low 11 bits of the
object id. **Medium** (x/z checked on samples; matches the C# `ObjectId`).

### Road net

For each land cell, `x` outer, `z` inner (**verified** from part positions):
`u32 n` then `n` road parts:

```
u16 n_ends; vec3[n_ends] end positions
u8[n_ends] connection types         v >= 24 (shipped parts have one end of type 0 and one of 1)
u32 object id                       encoded like static entity ids
asciiz model                        v >= 16
f32[12] transform                   v >= 16
```

In Arma 3 the road net only holds bridges and invisible runway roadways (932 parts on Altis).
Ordinary roads come from the terrain's roads shapefile (`roads.shp`, `RoadsLib.cfg`), not the
WRP.

### Map objects

`u32 type; u32 object id` then a payload chosen by the factory at `0x140b5fc30`:

| Class                       | Types                                                                              | Payload |
|-----------------------------|------------------------------------------------------------------------------------|---------|
| `MapObjectWithPos`          | 0-2, 10-17, 22, 23, 26, 27, 30                                                     | `f32 x, z` |
| `MapObjectWithRectColored`  | 3, 4, 8, 9, 18-21, 28, 29, 36-39                                                   | `f32[8]` four corners (x, z), `u32` colour |
| `MapObjectPlain`            | 5, 7                                                                               | none |
| `MapObjectWithAngle`        | 6                                                                                  | `f32` angle |
| `MapObjectWithRect`         | 24, 31, 32, 40                                                                     | `f32[8]` four corners |
| `MapObjectForest`           | 25, 33, 41, 42, 43                                                                 | `u8[4]`, `f32[4]` |
| `MapObjectWithLine`         | 34                                                                                 | `f32 x1, z1, x2, z2` |
| `MapObjectRailWay`          | 35                                                                                 | `f32[6]` (from x/z, to x/z, centre x/z), `u8` |
| `MapObjectRiver`            | 44                                                                                 | `u32 n`, compressed `f32[2 * n]` polyline |

Type names (engine debug table): 0 TREE, 1 SMALL TREE, 2 BUSH, 3 BUILDING, 4 HOUSE, 5 FOREST
BORDER, 6 FOREST TRIANGLE, 7 FOREST SQUARE, 8 CHURCH, 9 CHAPEL, 10 CROSS, 11 ROCK, 12 BUNKER,
13 FORTRESS, 14 FOUNTAIN, 15 VIEW-TOWER, 16 LIGHTHOUSE, 17 QUAY, 18 FUELSTATION, 19 HOSPITAL,
20 FENCE, 21 WALL, 22 HIDE, 23 BUSSTOP, 24 ROAD, 25 FOREST, 26 TRANSMITTER, 27 STACK, 28 RUIN,
29 TOURISM, 30 WATERTOWER, 31 TRACK, 32 MAIN ROAD, 33 ROCKS, 34 POWER LINES, 35 RAILWAY,
36 POWERSOLAR, 37 POWERWAVE, 38 POWERWIND, 39 SHIPWRECK, 40 TRAIL, 41 FOREST_LOD1,
42 FOREST_LOD2, 43 TOWN_LOD1, 44 RIVER. Any other type: "Unknown map type loaded".

Factory and payloads: **high**. Payloads parse to the exact end on every terrain: **verified**.
Forest payload: the flags are `0,0,1,1` everywhere and the values are multiples of 1/3; meaning
**low**. River (type 44): not present in shipped terrains; whether its array is compressed under
the usual threshold is **low**. Type 5/6/7 records do not occur in shipped files.

## Surface height

`SurfaceY(x, z)` (`0x141655d30`): `gx = x / cell`, `gz = z / cell`, `i = floor(gx)`,
`j = floor(gz)` (computed as `round(g - 0.5)`), `fx = gx - i`, `fz = gz - j`. With corner heights
`h00 = H(i, j)`, `h10 = H(i+1, j)`, `h01 = H(i, j+1)`, `h11 = H(i+1, j+1)`:

- `fx + fz <= 1`: `h00 + (h10 - h00) * fx + (h01 - h00) * fz`
- otherwise: `(h01 + h10 - h11) + (h11 - h01) * fx + (h11 - h10) * fz`

So each cell splits along the diagonal from `(i+1, j)` to `(i, j+1)`. **High**. Outside the grid
the engine calls `0x14163c2b0` (outside-terrain synthesis, not reverse engineered); `a3-wrp`
clamps to the edge instead.

## Survey (build 2.22.0.154103)

Only six `.wrp` files are readable: the CDLC terrains ship in EBOs, which cannot be opened yet.

| Terrain | App id | Size | Land grid | Heights | Objects | Map objects | Road parts | File | Parse (release) |
|---|---|---|---|---|---|---|---|---|---|
| `a3\map_altis\altis.wrp` | 107410 | 30720 m | 1024 @ 30 m | 4096 @ 7.5 m | 1,779,908 | 1,197,359 | 932 | 175 MiB | 1.5 s |
| `a3\map_enoch\enoch.wrp` (Livonia) | 1021790 | 12800 m | 256 @ 50 m | 2048 @ 6.25 m | 2,899,064 | 474,933 | 21 | 198 MiB | 1.9 s |
| `a3\map_malden\malden.wrp` | 0 | 12800 m | 512 @ 25 m | 1024 @ 12.5 m | 620,038 | 448,505 | 234 | 53 MiB | 0.4 s |
| `a3\map_stratis\stratis.wrp` | 107410 | 8192 m | 256 @ 32 m | 2048 @ 4 m | 163,953 | 80,420 | 432 | 25 MiB | 0.2 s |
| `a3\map_tanoabuka\tanoa.wrp` | 395180 | 15360 m | 512 @ 30 m | 4096 @ 3.75 m | 1,675,073 | 636,911 | 928 | 163 MiB | 1.3 s |
| `a3\map_vr\vr.wrp` | 107410 | 8192 m | 256 @ 32 m | 2048 @ 4 m | 0 | 0 | 0 | 0.4 MiB | 0.07 s |

Memory of the parsed `Terrain` (big arrays): Altis ~300 MiB (heights 64 MiB, 1.78 M objects at
60 bytes, 1.2 M map objects), Tanoa ~245 MiB, Livonia ~215 MiB. Parsing allocates one `Vec` per
array and none per object (rivers excepted).

Malden ships in the `Argo` folder (`Argo\Addons\map_malden.pbo`, free with the base game) with
app id 0; Livonia ships in `Enoch\Addons\map_enoch.pbo`.

## Open questions

- Meaning of `major`, object shape param, persistent flag bits (values 0-23 seen), forest map
  payload, road connection types.
- Outside-terrain height synthesis (`0x14163c2b0`).
- LZSS checksum kind for versions before 23.
