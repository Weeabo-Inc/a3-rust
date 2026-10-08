---
status: accepted
---

# Own engine on winit + wgpu, with an RV-shaped object model and a from-scratch SQF VM

We build our own engine from focused crates — **winit** (windowing/input), **wgpu** (rendering),
**glam** (math), **rapier3d** (physics, replacing the original's PhysX for vehicles and
collisions), **cpal** (audio output) with **lewton** (OGG Vorbis decoding) — rather than adopting
a general-purpose engine such as Bevy. The object model mirrors Real Virtuality's own concepts
(Landscape, World, Entity/Object, Simulation), the SQF virtual machine is written from scratch,
and all game data is read directly from the user's original PBOs with no conversion step.

## Reasoning

- **Fidelity to RV semantics.** The goal is to run unmodified Arma 3 content. Configs, SQF and
  missions assume RV's object model (entity classes chosen by `simulation`, config-driven
  animations, hit points, locality). Mirroring those concepts directly keeps behaviour matchable
  against the original; mapping them onto an ECS would add a translation layer at every boundary.
- **SQF needs synchronous world access.** SQF commands read and mutate world state immediately
  and in order (`setPos` then `getPos` must agree within one script step), from both scheduled
  and unscheduled code and from event handlers fired mid-simulation. An engine whose world is
  only reachable through deferred commands or system scheduling fights this model.
- **Terrain scale.** Arma terrains are 20 km+ per side (Altis is ~30 km) with millions of placed
  objects. That needs custom heightmap LOD, object streaming and floating-origin handling, which
  general engines do not supply and would constrain.
- **Load the original data.** Users keep their own install; we ship no game data. Reading PBOs,
  ODOL, PAA, WRP and rapified configs at runtime avoids an asset pipeline, keeps mods working, and
  makes the format crates (Phase 1) directly useful as tools.

## Considered options

- **Bevy**: rejected. Its ECS and scheduler are a poor fit for synchronous SQF world access and
  RV's class-based entities; we would also inherit its asset pipeline and churn.
- **Converting game data to modern formats ahead of time**: rejected. An extra step for users,
  breaks with mods, and still requires all the format readers.
- **PhysX bindings**: rejected. Poor Rust bindings and a heavy C++ dependency; rapier3d is native
  Rust and covers rigid bodies, vehicles via raycast/joint models, and character collision.
  Exact vehicle handling will differ and must be tuned against the original.

## Consequences

- We own the renderer, streaming, audio mixer and simulation loop: more code, full control.
- Physics behaviour is an approximation of the original; vehicle feel is tuned, not bit-exact.
- Every format reader must handle the real shipped data, including its quirks; `docs/re/` records
  them.
- The SQF VM is a large, central component; its command registry is the main integration surface
  between scripting and every other subsystem.
