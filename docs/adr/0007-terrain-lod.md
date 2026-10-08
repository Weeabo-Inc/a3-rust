---
status: accepted
---

# Terrain LOD: CDLOD patches with heights fetched in the vertex shader

The Landscape is drawn as a **CDLOD quadtree** (continuous distance-dependent LOD) over the
WRP heightmap. Every quadtree node is drawn with the same instanced grid patch of 32x32 quads;
a node at level `l` spaces its vertices `2^l` height cells apart. The heightmap lives on the GPU
as one `R32Float` texture and the vertex shader reads the heights. In the last 20 % of each
level's range the vertex shader morphs odd vertices onto the parent grid, so neighbouring levels
meet without cracks or skirts. Implemented in `crates/a3-landscape-render` (`lod.rs`,
`shaders/terrain.wgsl`).

## What RV does

`VSTerrain` (see `docs/re/render-terrain.md`) geomorphs too, but differently: every terrain
vertex carries seven heights, one per LOD level, and blends them with tent weights over
`lod = 0.5 * log2(d²) + bias`. The vertex buffers, segment layout and `VSC_TerrainLODPars`
values are built on the CPU and are not reverse engineered yet.

## Decision

- Quadtree over the height grid, leaf patch 32 cells, level `l` used up to
  `4 * 32 * cell * 2^l` metres from the camera (Altis: 960 m for 7.5 m cells), morph from 80 % of
  the range. The node bounds use a min/max height pyramid; nodes are frustum-culled.
- Areas drawn at a level never reach past the next level's morph start and never come closer
  than the previous level's range. Property tests check this on synthetic and Altis heights; it
  is what makes shared edges coincide.
- At full detail each quad splits along the engine's diagonal (from `(i + 1, j)` to
  `(i, j + 1)`), and the shader interpolates morphed vertices with the engine's triangle rule,
  so near the camera the drawn surface equals `getTerrainHeightASL` (objects stand on it
  exactly).
- Positions stay camera-relative (ADR 0003): each node's origin minus the camera is computed in
  `f64` on the CPU; world `x, z` for texturing come from integer grid indices.

## Considered options

- **RV's scheme (seven heights per vertex, CPU-built segments):** the most faithful, but its
  CPU side is not reverse engineered, it needs per-segment vertex buffers (Altis: 16.8 M height
  samples times 7), and it gives no rendering difference a player could notice. We can switch
  to it when the RE work is done.
- **Geometry clipmaps:** nested rings around the camera, very cheap to update. Rejected:
  vertices slide over the terrain as the camera moves (swimming), and culling is coarser.
- **Static chunks with precomputed LOD meshes and skirts:** simple, but skirts show at grazing
  angles and LOD changes pop.

## Consequences

- Detail beyond the heightmap (RV's terrain subdivision near the camera) needs a separate step.
- Far levels sample the full-resolution heights at sparse points (no prefiltering). A height mip
  pyramid can be added if distant ridges flicker.
- The GPU needs vertex-shader texture reads and storage buffers in the fragment stage. Every
  wgpu backend we target supports both.
