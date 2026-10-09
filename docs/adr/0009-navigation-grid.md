---
status: accepted
---

# Navigation is a land-cell cost grid searched by A*, in its own `a3-nav` crate, with building paths as separate small meshes

`crates/a3-nav` owns pathfinding: a **cost grid** at the terrain's land-cell resolution, an
**A\*** over it with 8 neighbours and a reusable per-planner scratch, a **string-pull** that
turns the cell path back into positions, **road** cells baked cheap so long paths follow the
road network, and **building paths** as their own triangle meshes from a model's Paths/Roadway
LOD. The query is `Navigator::find_path(from, to, radius) -> Option<Vec<Vec3>>`.

## Decisions

- **Grid A\*, not a triangle navmesh.** The engine's own search is a template `AStar` over a
  cell field (`OperMap`/`OperField`, `AIPathPlanner::ProcessSearching`;
  `docs/re/navigation.md`): a cell grid that is baked from terrain data, patched per AI type and
  combat mode, and traversed with pluggable costs. A navmesh would have to be generated from
  the same data plus the placed objects, which the engine never does — and it would have to be
  rebuilt whenever an object moves. A grid is bake-once, cheap to query for many agents, and
  its cells carry much of what the WRP already gives us (geography flags, heightmap). The bake
  deliberately ignores the surface material: `a3-landscape` reads `CfgSurfaces` for physics and
  rendering, but no shipped navigation behaviour was found that depends on it, so a cell's cost
  is geography, slope and roads only.
- **Land-cell resolution.** The grid is `Terrain::land_grid` sized, one cell per land cell
  (30 m on Altis, 25 m on Malden, 50 m on Livonia). That is the resolution the geography flags
  are stored at, so the bake is a straight copy instead of an interpolation, and the whole of
  Altis is 1024 x 1024 cells — about 1 MiB of cost. A finer grid (7.5 m, the Altis heightmap)
  would be 16.7 M cells per array and would not add anything the string pull does not already
  recover for terrain; a coarser one would put two land cell's worth of information into one
  cost.
- **Costs are `u8`, `0` = impassable.** Open ground is 100 (percent), forest 130, built-up 140,
  shallow water 200, road 70, and 0 for deep water, slopes above `CfgSlopeLimits::maxRun`
  (0.6) and cells patched by an obstacle. The A* multiplies the moving cost by
  `(cost_of(a) + cost_of(b)) / 200`, so the value is a multiplier, not a distance.
- **Roads are a cost, not a second graph search.** The engine puts the road network into the
  field (roads cheap for AI, `RoadsLib >> AIpathOffset` for the lateral offset), so the same
  search follows roads. `a3-nav` bakes the `RoadGraph`'s curve segments into the cells they
  cross at cost 70, and a road cell wins over water and built-up: a road over water is a bridge
  or runway deck, and the deck is what a man walks on. No separate road routing.
- **Cell path becomes positions by string pull.** The engine's paths are oper positions, not
  cell centres. After A*, a greedy furthest-visible pull on the cell centres walks the segment
  at heightmap resolution and rejects a shortcut that leaves walkable ground (blocked cell,
  slope over the limit, deep water), which is what removes the staircase from an 8-neighbour
  search.
- **Buildings are separate meshes.** Indoor movement is not a terrain problem: the engine has
  `IPaths` with `PathAction`s (ladders are `PathActionLadderTop`/`PathActionLadderBottom`) and
  keeps a terrain-wide house path index, so an outdoor path ends at a house position and the
  house's own graph continues. `PathMesh` is that graph: triangles from the model's Paths LOD
  (falling back to the Roadway LOD it already has to stand on), adjacency by shared edge, an
  A* over triangles and a path crossing each shared edge at its midpoint. It is built per model,
  not per Object, and shared. The 60 `households` models Altis's object list references that the
  real-data test checks all have one.
- **Obstacles come from physics where they are loaded.** The baked grid sees geography and
  terrain; the objects are not in it, because a WRP object has no footprint until its model is
  read and physics has already read those models. `Navigator::block_from_colliders` patches
  cells around a query from the `CollisionWorld`'s loaded colliders (Geometry layer, tall
  enough to be walked around), and `NavGrid::set_cost` is the general patch for dynamic
  obstacles.
- **Its own crate.** `a3-world` deliberately has no `a3-landscape` dependency (roads, surfaces,
  world config) and no `a3-p3d`; navigation needs all three plus `a3-physics`. A crate also
  keeps the whole thing testable on a synthetic `TerrainBuilder` terrain and a hand-built
  `Lod`, without a World — the same argument ADR 0008 makes for `a3-physics`.

## Considered options

- **A triangle navmesh** (recast-style, then funnel): rejected. The bake would have to see
  every object's collision geometry at load time, the result is invalidated by a moved object,
  and the engine's behaviour we are matching is a cell field. A mesh also gives no obvious place
  for the per-AI-type/per-combat-mode cost the engine's `OperMap::CreateFields` carries.
- **Navigation inside `a3-world`**: rejected. It would drag `a3-landscape` and `a3-p3d` into every
  `a3-world` user (the server, the tools, the UI) for a feature only the AI and the editor need.
- **Heightmap-resolution cells**: rejected — 16.7 M cells on Altis and 8x the bake time, for
  detail the smoothing pass recovers.
- **A separate road graph search stitched to the grid path** (as a routing problem): rejected
  for now; the engine does not do it this way, and a cheap road cell gets the same long-range
  behaviour with no stitch seam. A road-graph route is still available in `a3-landscape` for
  vehicles later.
- **`f32` or `f64` positions**: the grid, the terrain and the road graph are all `f32` metres,
  so navigation works in `Vec3` and converts at the `a3-world` boundary (`as_dvec3()`); see
  ADR 0003.

## Consequences

- The grid is baked once per terrain and is read-only afterwards; live obstacles are patched per
  query area or cleared with `set_cost`. On an idle machine, baking Altis (1,048,576 land cells)
  takes about 45 ms — geography and slope — or 60 ms including its 1,414 roads, and planning 60
  paths of 300 m – 2 km across the island takes 38 ms (0.6 ms per path, a `Planner` of the
  caller's own keeping the search allocation-free); with every core busy both scale to about
  2–3 times that, still well inside a load.
- Path quality is bounded by the cell size before smoothing and by the smoothing's own checks
  after it: a path never leaves walkable ground, but it can pass within a cell of an object
  the bake did not know about (an unstreamed collider).
- Vehicles need their own cost set (`CfgVehicles`-driven) before `calculatePath` can be
  answered; the crate's costs are the man's (`CfgSlopeLimits`).
- Building paths are per model; a door position links the two graphs, and the link is by
  geometry (the nearest mesh vertex to the door), not by a baked house index yet.
