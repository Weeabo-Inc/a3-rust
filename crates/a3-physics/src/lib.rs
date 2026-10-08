//! Collision and rigid-body physics for the World: the terrain, the Static objects of the map
//! and the bodies of Entities in one rapier world (`rapier3d-f64`, ADR 0001 and ADR 0008), with
//! the query layers the engine uses. The fixed step that drives [`CollisionWorld::step`] is
//! ADR 0002. RE notes: `docs/re/physics-collision.md`.
//!
//! # Model
//!
//! - A model's collision ([`ModelCollision`]) is built once per model path by the
//!   [`ModelBank`] from its special LODs, one [`Layer`] each: **Geometry** (solid, mass),
//!   **Fire Geometry** (bullets), **View Geometry** (line of sight), each a compound of convex
//!   components (the LOD's `ComponentNN` selections), and **Roadway** (walkable surfaces), a
//!   triangle mesh. Fire and View Geometry fall back as the ODOL special LOD indices do. Every
//!   Object using the model shares the shapes; scaled map objects get a scaled copy, cached
//!   per millimetre of scale.
//! - Every face has a surface material ([`SurfaceInfo`], from the material's `.bisurf` or a
//!   `#Class` naming a `CfgSurfaces` class; faces without a material match `CfgSurfaces >> files`
//!   by texture name), interned in the [`SurfaceBank`].
//! - Static objects get colliders per land cell, only around [`Interest`]s:
//!   [`CollisionWorld::stream`] loads the cells and terrain heightfield chunks near them and
//!   unloads those unused for a while. [`StaticSource`] supplies a cell's objects (the World's
//!   Static object table; [`TerrainStatics`] reads a WRP directly).
//! - Entities get bodies ([`CollisionWorld::add_body`]): [`BodyKind::Dynamic`] when simulated
//!   here (a local physics Entity), [`BodyKind::Kinematic`] when moved from outside (remote
//!   Entities, men). Mass, centre of mass and inertia come from the model.
//!
//! # Queries
//!
//! All take `&self` and see loaded cells, every body and the terrain. Positions are World space
//! (ADR 0003, `f64`); hits name the Object ([`ObjectKey`]), layer, convex component and
//! surface, and carry a [`ShapeRef`] for [`CollisionWorld::component`] (its selection name).
//!
//! | Need | Call |
//! |---|---|
//! | first hit on a segment (`lineIntersectsSurfaces`, `intersect`, wheel rays, AI) | [`CollisionWorld::ray_cast`] with a [`RayQuery`] |
//! | every crossing (`lineIntersectsSurfaces` with many results, `lineIntersectsWith`) | [`CollisionWorld::ray_cast_all`] |
//! | entry, exit and depth per component (bullet penetration) | [`CollisionWorld::penetrations`] |
//! | terrain only (`terrainIntersect`, `getTerrainHeightASL`) | [`RayQuery`] with no layers, [`TerrainShape::height`] |
//! | ground under a man or wheel, bridges and floors included | [`CollisionWorld::surface_below`] |
//! | sweep a shape (movement) | [`CollisionWorld::shape_cast`] |
//! | push-out after moving (character collision) | [`CollisionWorld::contacts`] |
//! | what a shape or point is inside (`isInside`-style) | [`CollisionWorld::overlaps`], [`CollisionWorld::objects_at`] |
//! | line of sight through foliage (`checkVisibility`) | [`CollisionWorld::visibility`] |
//!
//! Ray queries test the terrain exactly over the whole map (the engine's triangle split);
//! shape queries see the terrain only where its chunks are loaded.
//!
//! # A frame
//!
//! ```text
//! world.stream(&statics, &[Interest { center: player, radius: 300.0 }, ...]);
//! world.set_body_pose(man, pos, rot);            // kinematic Entities
//! world.add_force_at(car, force, wheel_contact); // forces on local dynamic Entities
//! world.step(1.0 / 60.0);                        // per fixed step (ADR 0002)
//! let s = world.body(car).unwrap();              // write back to the Entity
//! ```

mod bank;
mod conv;
mod files;
mod layer;
mod model;
mod query;
mod statics;
mod surface;
mod terrain;
mod world;

pub use bank::ModelBank;
pub use files::{FileSource, MemoryFiles};
pub use layer::{Layer, LayerMask};
pub use model::{Component, LayerShape, MassProperties, ModelCollision};
pub use query::{
    Overlap, Penetration, QueryShape, RayHit, RayQuery, ShapeContact, ShapeHit, ShapeRef,
    SurfaceSample,
};
pub use statics::{TerrainStatics, wrp_transform};
pub use surface::{SurfaceBank, SurfaceId, SurfaceInfo};
pub use terrain::{TerrainHit, TerrainShape};
pub use world::{
    BodyKind, BodyState, CollisionWorld, GRAVITY, Interest, LandGrid, ObjectKey, StaticPlacement,
    StaticSource, StreamReport,
};
