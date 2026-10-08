# Terrain roads: `roads.shp`, `roads.dbf`, `RoadsLib.cfg`

Implemented in `crates/a3-landscape` (`RoadNetwork`, `shapefile`). The WRP road net holds only
bridges and runway roadways (see `wrp.md`); every ordinary road of an Arma 3 terrain comes from
these files. Confidence: **verified** (all six readable worlds), **high** (executable),
**medium**, **low**.

## Executable

| Address       | What |
|---------------|------|
| `0x14162b1d0` | Reads `CfgWorlds >> <world> >> newRoadsShape` (falls back to hard-coded paths for Stratis and Limnos), opens the shapefile and `RoadsLib.cfg` next to it, builds road segments |
| `0x14162c920` | Same pipeline for the "new roads" rebuild; "New roads: shapefile doesn't exist!" |
| `0x14164e7c0` | Reads `RoadTypesLibrary >> Road%04d` for one `ID` |
| `0x141ce8b60` | `double 200000.0`: the easting offset |

## Files

- `roads.shp`: ESRI shapefile, shape type 3 (PolyLine), one part per record in every shipped
  file, 2D points as `f64`. **Verified**.
- `roads.dbf`: dBase III table, one row per shape. Columns differ per terrain:
  Altis/Stratis/Malden/VR `__LAYER, ID, ORDER, __ID`; Tanoa adds `ROADMASK`; Livonia has
  `__LAYER, ID, WIDTH, __ID`. **Verified**.
- `RoadsLib.cfg`: text config (with `//` and `/* */` comments), `class RoadTypesLibrary` with
  `class Road0001`, `Road0002`, ... **Verified**.
- `roads.prj`: a WKT projection (UTM, false easting 500000). The engine ignores it _(high: no
  `.prj` string in the executable)_.

## Engine semantics (high)

- World coordinates: `x = X - 200000`, `z = Y` (shapefile X/Y in metres). **Verified**: every
  road point of every world lies within 10 m of its terrain square, 100 % above -1 m.
- Polylines with fewer than 2 points are skipped.
- Per row: `ID` (atoi, truncated to a byte) selects `RoadTypesLibrary >> Road%04d`;
  `ORDER` (optional, byte); `ROADMASK` (optional): for values below 112 the units, tens and
  hundreds digits being non-zero set bits 0, 1 and 2 _(meaning low)_. Other columns
  (`WIDTH`, `__LAYER`, `__ID`) are not read by the engine.
- Road class entries: `width`, `mainStrTex` (straight texture), `mainTerTex` (end texture),
  `mainMat` (rvmat), `map` (map symbol type name, matched against the map type names: `main
  road`, `road`, `track`, `trail`, ...), `AIpathOffset`, `pedestriansOnly`, `color[]` (AI cost
  map colour). Missing class: `road_ca.paa`, `road_end_ca.paa`, `road.rvmat` from
  `a3\roads_f\roads\data\`, map `main road`.
- `ROADMASK` and `ORDER` select a material variant: `0x14164e660` registers one variant per
  key `mask + (order + typeField * 10) * 10` (stored as a `short` with the order and mask
  bytes next to it) and returns its index, which each segment keeps. What the three mask bits
  change in the material is not known _(low)_; only Tanoa uses `ROADMASK`.

## Joining and curves (`0x14162c920`, high)

For every polyline A the loader scans every polyline B (A included) and compares ends:

| A end | B end | Direction test | Point used beyond A's end |
|---|---|---|---|
| start | start | `dot(intoA, intoB)` | `B[1]` |
| start | end | `dot(intoA, intoB)` | `B[n-2]` |
| end | start | `dot(intoA, intoB)` | `B[1]` |
| end | end | `dot(intoA, intoB)` | `B[n-2]` |

`into` is the unit direction from the end point into its polyline (2D). Ends match when their
squared distance is below `0.01` (0.1 m). The match with the smallest dot wins, and is used
only if that dot is below `0.5` (`0x141a790f4`); otherwise the end is **open** (the point
beyond is the end point itself).

Each polyline segment `p1 -> p2` (neighbours `p0`, `p3`, from the polyline or the matched
roads) becomes a cubic Bezier `p1, c1, c2, p2` with centripetal Catmull-Rom controls
(`d1 = |p1 - p0|`, `d2 = |p2 - p1|`, `d3 = |p3 - p2|`):

```
c1 = (d1*p2 - d2*p0 + (2*d1 + 3*sqrt(d1*d2) + d2)*p1) / (3*d1 + 3*sqrt(d1*d2))   (p1 when d1 = 0)
c2 = (d3*p1 - d2*p3 + (2*d3 + 3*sqrt(d2*d3) + d2)*p2) / (3*d3 + 3*sqrt(d2*d3))   (p2 when d3 = 0)
```

`0x141629350` builds a road segment from each Bezier: it samples the curve in steps, adds up
arc length divided by the material's texture length (UV `v`), and at open ends marks the first
/ last texture length of the curve for the end texture (`mainTerTex`).

## Rendering (`a3-render::roads`)

`RoadFeature` draws the Beziers as strips at the RoadsLib width, draped on the terrain with
`surface_height` and lifted 4 cm, alpha-blended in the alpha phase with a depth bias towards
the camera. Texture `u` runs across the road (0 = left), `v` along it in road widths (the
shipped road textures are square, so one texture length = one width _(assumption: the engine
reads the length from the material entry at `+0x38`)_). At open ends the first width uses
`mainTerTex` with `v = 0` at the end. Only stage 0 (albedo) is used so far; the `mainMat`
normal and `_smdi` stages and the Road pixel shader lighting are follow-ups.

## Road graph (`a3-landscape::RoadGraph`)

Shipped road networks mostly do not share end points: on Altis only 857 of 2,828 road ends
continue into another road by the engine's rule (9 nodes with 3+ ends). Most junctions are T
junctions where a road ends on another road's side: 936 Altis ends lie within 0.1 m of another
centre line (855 on one of its vertices), and about 1,450 more within 1-5 m (inside the other
road's half width). `RoadGraph` models both:

- nodes: ends within 0.1 m, continuations by the engine rule;
- attachments: an end within (half width + 0.1 m) of another road's centre line (Altis 1,963,
  Tanoa 925, Livonia 665, Malden 223, Stratis 79);
- queries: `connected_to` (`roadsConnectedTo`: node neighbours and attachments both ways),
  `nearest`, `road_at` / `is_on_road` (within half the RoadsLib width of the polyline),
  `roads_near` (`nearRoads`), `curve` (the engine's Beziers).

How the engine's own `roadsConnectedTo` (`0x14049e7e0`) and `isOnRoad` (`0x140546300`) treat
shapefile roads is not yet reverse engineered; they go through the landscape's road segment
objects _(open)_.

## Survey (build 2.22.0.154103)

| World | Roads | Points | Length | Types used / defined |
|---|---|---|---|---|
| Altis | 1,414 | 22,400 | 731 km | 5 / 6 |
| Stratis | 73 | 1,845 | 44 km | 4 / 5 |
| VR | 10 | 64 | 2.6 km | 4 / 6 |
| Tanoa | 719 | 13,852 | 403 km | 10 / 10 (incl. `trail`, width 1.6) |
| Malden | 212 | 4,856 | 120 km | 3 / 3 |
| Livonia | 407 | 11,422 | 458 km | 3 / 5 |

VR sets no `newRoadsShape` but ships `a3\map_vr\data\roads\roads.shp`.
