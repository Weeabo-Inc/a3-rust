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
- Segment ends closer than 0.1 m (`0.01` squared) are joined with neighbouring polylines when
  their directions agree _(medium; not implemented in `a3-landscape`)_.

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
