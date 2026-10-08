---
status: accepted
---

# One rapier world in f64 for collision and bodies, streamed per land cell, with per-layer compound colliders

`crates/a3-physics` owns collision and rigid bodies. It is a separate crate, not a module of
`a3-world`: it depends only on the format crates (`a3-p3d`, `a3-wrp`, `a3-config`, `a3-vfs`),
so its queries can be tested and used by tools without a World, and `a3-world` stays the single
owner of Entities that drives it. It uses **`rapier3d-f64`** (ADR 0001 picked rapier) and puts
the terrain, the Static objects and the Entity bodies in **one** rapier world.

## Decisions

- **f64.** World positions are `f64` (ADR 0003). With `f32` rapier, a body 30 km from the
  origin moves in 2 mm steps; a car at 1 m/s and 60 Hz moves 17 mm per step, so its velocity
  would be quantised by about 12 %. The f64 build costs some SIMD width and nothing else.
- **Layers as separate colliders.** Each Object gets one collider per special LOD it has
  (Geometry, Fire Geometry, View Geometry, Roadway). Queries filter by layer on the collider's
  user data; collision groups make only Geometry (and the terrain) take part in contacts, and
  never Static against Static.
- **Compound per layer, not collider per component.** A house has 60 Geometry, 76 Fire and 42
  View components; one collider each would be about 180 colliders per house. A compound per
  layer keeps it at 4, and the component is recovered by testing the compound's parts of the
  one collider a query hit. Per-component data (selection name, surface) lives next to the
  shared shape.
- **Shared shapes.** A model's shapes are built once (`ModelBank`) and shared through
  `SharedShape` by every Object using it. Scaled map objects get scaled copies of the hulls,
  cached per millimetre of scale.
- **Streaming per land cell.** Static object colliders exist only for land cells near an
  interest (Entities, the camera, a script query's area) and are dropped after a number of
  streaming generations without interest. Altis has 1.78 M objects; a 600 m square around
  Kavala is about 440 cells and 23 k colliders.
- **Terrain twice.** Ray queries intersect the heightmap analytically, with the engine's
  triangle split, over the whole map, so long rays (AI visibility, `terrainIntersect`) need no
  loaded colliders. Contacts and shape queries use parry heightfield chunks (32 x 32 height
  cells), whose triangle split is the same.
- **Bodies.** Local physics Entities are `Dynamic`; remote Entities and those moved by their
  own simulation (men) are `Kinematic` (position based). Mass, centre of mass and inertia come
  from the ODOL `ModelInfo`; the contact shape is the PhysX geometry LOD when the model has one,
  else the Geometry LOD.

## Considered options

- **A module in `a3-world`**: rejected. Collision building and queries are large and need no
  Entity state; a crate keeps them testable on synthetic terrains and usable by tools.
- **`rapier3d` (f32) with a moving origin**: rejected for now; rebasing every collider and body
  when the origin moves interacts badly with streaming and with SQF holding positions (the
  reasons ADR 0003 gives against a floating origin).
- **One rapier world per layer**: rejected. Four broad phases to keep in sync for every moving
  body, for no gain over filtering.
- **Our own collision code throughout** (as the original does for non-PhysX objects): rejected
  for now; rapier gives the broad phase, convex and mesh queries and the solver. Engine-specific
  rules (ricochet, penetration, man capsules) are written on top of its queries.

## Consequences

- Queries see only loaded cells: callers stream interests every frame (the World does it for
  its Entities) and call `load_area` before a one-off query far away.
- Broad-phase removals are applied at the next physics step. Queries skip removed colliders, so
  this is invisible except for memory between steps.
- Animated parts (doors, hatches) do not move their components yet.
