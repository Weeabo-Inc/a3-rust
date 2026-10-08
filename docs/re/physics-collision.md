# Collision data: surface materials, collision LODs, placement

What `a3-physics` needs to know about the original's collision data, and what was checked on
the shipped game. Arma 3 2.22.0.154103, `arma3_x64.exe`; addresses are VAs. Confidence:
**high** = read from code or verified on all shipped data, **medium** = inferred or sampled,
**low** = hypothesis.

## Surface materials (`SurfaceInfo`)

| Function | VA | What |
|---|---|---|
| `SurfaceInfo` constructor | `0x1410dd800` | builds one surface from a name |
| `SurfaceInfo` load | `0x1410dfd80` | reads the keys below from a config class |
| surface bank lookup | `0x1410e26f0` | linear search by name (case-insensitive compare), else construct and append |
| `CfgSurfaces` preload | `0x1410df890` | one surface per `CfgSurfaces` class, named `#<class>`, with its `files` (or `pattern`) |
| material reader | `0x1415a4e10` | reads an rvmat; its `surfaceInfo` goes to the surface bank |

**Names.** A name starting with `#` is a `CfgSurfaces` class (`#GdtGrass` →
`CfgSurfaces >> GdtGrass`). Any other name is a file path parsed as a config file (`.bisurf`).
If the file cannot be read the engine logs `Cannot load surface info %s` and uses defaults:
`rough` 0, `dust` 0, `isWater` false, penetration 0, `surfaceFriction` 2, `tracksAlpha` 1,
`transparency` -1. **High.**

**Keys** read by `0x1410dfd80` (defaults when the key is absent): `rough`, `dust`, `isWater`,
`soundEnviron`, `impact` (required), `lucidity` (1), `grassCover` (0), `AIAvoidStance` (0),
`maxClutterColoringCoef`, `maxSpeedCoef` (1), `surfaceFriction` (2), `tracksAlpha` (1),
`transparency` (-1, clamped to at most 1), `terrainClutterFactor`, `character` (with
`CfgSurfaceCharacters` `probability`/`names`), `heightMap`,
`bulletPenetrabilityWithThickness` or else `bulletPenetrability` (stored as
`1e6 / value`), `thickness` (mm, stored × 0.001 as metres, default -1), `soundHit` (mapped to
a hit category: `soft_ground`, `hard_ground`, `building`, `glass`, `foliage`, `metal`,
`metal_plate`, `glass_armored`, `plastic`, `concrete`, `rubber`, `water`, `default`, and three
more not decoded), `deflection` (1). **High** for names and the penetration and thickness
transforms; **medium** for defaults read through vtable calls.

`Density`, `friction` and `restitution`, present in every `.bisurf`, are **not** read here
(probably by the PhysX material setup; not traced). `a3-physics` keeps them for contact
material and mass.

**Files.** 86 `.bisurf` files ship, 79 of them in `a3\data_f\penetration\` (plus
`a3\data_f\surfaces\clutter.bisurf`, `a3\data_f\wood.bisurf` and foliage files of the
vegetation addons). They are plain config text with unquoted words (`soundEnviron=Empty;`,
`isWater=false;`), not rapified. **High.**

## Which surface a face has

- **Faces with a material** (Geometry, Fire and View Geometry of buildings and vehicles): the
  ODOL embedded material's `surface` string, the rvmat's `surfaceInfo` (e.g.
  `a3\data_f\penetration\concrete.bisurf`). Fire Geometry sections carry the penetration
  materials (`building`, `building_plate`, `concrete`, `glass_plate`, `metal`, `wood` on
  `i_house_small_01_v1_f`). **High.**
- **Faces without a material** (Roadway LODs; the terrain): the texture file name, without
  extension, matched against `CfgSurfaces >> files` patterns (`*` wildcard; e.g. Roadway
  texture `a3\data_f\surfaces\betonout.paa` → class with `files = "betonout"`). Every Altis
  bridge deck resolves this way (`#SurfRoadConcrete`, `#concrete`). **Medium**: the matcher
  itself was not decompiled; first match in config order is assumed.

## Collision LODs

- Geometry, Fire Geometry and View Geometry LODs are sets of convex components: the binariser
  stores each as a named selection `componentNN` (`i_house_small_01_v1_f`: 62 Geometry
  components, plus door selections). `a3-physics` builds one convex hull per selection.
  **High** for the selections; the engine's own collision code using them is not traced.
- The ODOL special LOD indices give the fallbacks: no Fire Geometry → View Geometry → Geometry;
  no View Geometry → Geometry (`p3d-odol.md`).
- Roadway LODs are open surfaces (a house: 50 faces, 3 textures, no selections) and are used as
  triangle meshes.

## Placement

ODOL vertex positions are relative to the model origin, and the WRP object transform places
that origin (no extra bounding-centre offset). Checked on Stratis: ray casts from above at the
object position hit 94 of 95 houses of Agia Marina and 370 of 372 houses of Kavala (Altis) on
their own Geometry, more than 2 m above the terrain; the misses are a ruin and an annex whose
origin is over an opening. Walls stand on the ground (`stone_8m_f`: origin 0.58 m above the
terrain, top 1.69 m). **High.**

## Bridges

Altis has 33 road-net parts whose model is a bridge (`bridge_01_f`, `bridge_asphalt_f`,
`bridge_concrete_f`, `bridge_highway_f`). For each, the Roadway LOD under the midpoint of the
part's road ends is within 1 m of the ends' height and above the ground or water: asphalt and
concrete decks 0.29 m below the end points, `bridge_01_f` (an arched bridge near Kavala at
x 3494, z 13378) 0.7 m above them, its deck at 2.44 m over water at -1.02 m. **High** (all 33).

## Open questions

- The engine's surface query (`0x1416527b0`, terrain + Roadway, used by men and projectiles)
  and its step-up tolerance.
- How the Geometry components of animated parts (doors) follow the animation.
- The three undecoded `soundHit` categories.
