---
status: accepted
---

# One shared texture residency manager: ref-counted handles, per-frame mip wants, LRU under a budget

Streamed textures (PAA from the VFS) go through one `a3_render::TextureResidency` shared by all
renderers (terrain materials, ODOL models, later UI and effects). Users hold **ref-counted
`TextureHandle`s** (clone = acquire, drop = release) and **re-state each frame** which mip they
need (`want_mip`, or automatically from the screen size of `MeshDraw`s). The manager loads the
**low-detail mip tail first** on loader threads, refines to the wanted mip by loading only the
missing finer mips and **copying the resident ones on the GPU**, and keeps all streamed textures
under a **byte budget**: unreferenced textures are evicted least recently used first, then
referenced textures holding finer mips than they currently want are coarsened. Uploads are capped
per frame. Uploaded meshes and textures (`upload_mesh` / `upload_texture`) get **generational
ids** and can be removed; wgpu's reference counting defers the actual destruction until no
submitted frame uses them.

## Why

- **Altis scale.** Thousands of distinct textures and models are referenced across a 30 km
  terrain; only those near the camera need detail. One budget across all renderers prevents each
  from sizing its own pool for the worst case.
- **Wants expire.** Need changes every frame with the camera. Making wants last one frame means a
  renderer that stops drawing something automatically stops paying for its detail, without
  bookkeeping on its side.
- **Tail first.** A 64 px tail of every texture costs little (about 1/256 of the full chain for a
  1024 px texture) and gives something to draw immediately; detail streams in by need. RV itself
  streams mips by distance (texture `Mip` bias settings); the exact policy is not reverse
  engineered.
- **GPU copies for refinement.** Re-reading and re-uploading coarse mips on every refinement would
  double the IO and upload cost of streaming.
- **RAII handles.** Explicit `release` calls are easy to forget across the many owners a texture
  has (a model shared by thousands of placed objects); `Arc` counts them for free, and the manager
  reads the count when planning.

## Considered options

- **Per-renderer pools** (as the terrain's satellite tile array has today): simple, but budgets
  do not compose and each renderer reimplements LRU and loading. Kept only where a renderer needs
  a fixed-size texture array (terrain satellite tiles), which this manager does not provide.
- **Sparse/virtual textures (tiled resources):** not portable across wgpu backends today.
- **Explicit acquire/release ids:** cheaper per handle, but leak-prone; rejected.
- **Wants that persist until changed:** fewer calls, but stale wants keep detail resident;
  rejected.

## Consequences

- Renderers must call `want_mip` (or draw through `MeshDraw`) every frame they use a texture,
  otherwise it decays to its tail under memory pressure.
- Views can change when a texture is refined or coarsened: resolve them in `prepare` each frame
  instead of caching bind groups across frames.
- The planner (`residency/plan.rs`) is pure and unit-tested; RV's own streaming heuristics
  (distance bias, `textureQuality` options) can replace the policy there.
- Mesh (ODOL) streaming is not part of this manager yet: models use `upload_mesh`/`remove_mesh`
  with their own LRU, or a mesh residency can follow the same pattern.
