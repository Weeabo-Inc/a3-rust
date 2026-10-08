//! The collision world: terrain chunks, Static objects streamed per land cell, and the rigid
//! bodies of Entities, in one rapier world.

use std::collections::HashMap;
use std::sync::Arc;

use a3_wrp::Terrain;
use glam::{DAffine3, DMat3, DQuat, DVec2, DVec3};
use rapier3d_f64::prelude::*;

use crate::conv;
use crate::{
    Layer, LayerShape, ModelBank, ModelCollision, SurfaceBank, SurfaceId, SurfaceInfo, TerrainShape,
};

/// The Object a collider belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectKey {
    /// The terrain surface.
    Terrain,
    /// A Static object, by the packed key its [`StaticSource`] gave (the World's `StaticKey`).
    Static(u32),
    /// An Entity with a body, by the key it was added with (the World's encoded Entity ID).
    Entity(u64),
}

/// Collision groups: what collides with what in the rigid-body simulation. Queries filter by
/// layer on the collider's user data instead.
pub(crate) mod groups {
    pub const TERRAIN: u32 = 1;
    pub const GEOMETRY: u32 = 1 << 1;
    pub const FIRE: u32 = 1 << 2;
    pub const VIEW: u32 = 1 << 3;
    pub const ROADWAY: u32 = 1 << 4;
    pub const BODY: u32 = 1 << 5;
}

fn interaction(memberships: u32, filter: u32) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(memberships),
        Group::from_bits_truncate(filter),
        InteractionTestMode::And,
    )
}

fn layer_group(layer: Layer) -> u32 {
    match layer {
        Layer::Geometry => groups::GEOMETRY,
        Layer::FireGeometry => groups::FIRE,
        Layer::ViewGeometry => groups::VIEW,
        Layer::Roadway => groups::ROADWAY,
    }
}

const NO_LAYER: u128 = 7;

/// Packs what a collider stands for into its `user_data`.
pub(crate) fn encode(object: ObjectKey, layer: Option<Layer>) -> u128 {
    let (kind, value): (u128, u128) = match object {
        ObjectKey::Terrain => (0, 0),
        ObjectKey::Static(k) => (1, u128::from(k)),
        ObjectKey::Entity(e) => (2, u128::from(e)),
    };
    value | kind << 64 | layer.map_or(NO_LAYER, |l| l as u128) << 66
}

pub(crate) fn decode(data: u128) -> (ObjectKey, Option<Layer>) {
    let value = data as u64;
    let object = match (data >> 64) & 3 {
        1 => ObjectKey::Static(value as u32),
        2 => ObjectKey::Entity(value),
        _ => ObjectKey::Terrain,
    };
    (object, Layer::from_index(((data >> 66) & 7) as u32))
}

/// The land grid Static objects are streamed by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LandGrid {
    /// Edge length of a land cell in metres.
    pub cell_size: f64,
    pub width: u32,
    pub height: u32,
}

impl LandGrid {
    /// The land cell under world `(x, z)`, if inside the grid.
    pub fn cell_at(&self, x: f64, z: f64) -> Option<(u32, u32)> {
        let (cx, cz) = ((x / self.cell_size).floor(), (z / self.cell_size).floor());
        (cx >= 0.0 && cz >= 0.0 && cx < f64::from(self.width) && cz < f64::from(self.height))
            .then_some((cx as u32, cz as u32))
    }
}

/// One Static object to give colliders to.
#[derive(Debug, Clone, Copy)]
pub struct StaticPlacement<'a> {
    pub key: u32,
    /// The model path.
    pub model: &'a str,
    /// Model space to world space, scale included (the WRP transform).
    pub transform: DAffine3,
}

/// Where the Static objects of each land cell come from (the World's Static object table, or
/// [`TerrainStatics`](crate::TerrainStatics) straight from a WRP).
pub trait StaticSource {
    fn land_grid(&self) -> LandGrid;
    /// Calls `f` for every Static object of land cell `(x, z)` that should collide.
    fn for_each_in_cell(&self, x: u32, z: u32, f: &mut dyn FnMut(StaticPlacement<'_>));
}

/// A position around which colliders must be loaded, with the radius in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interest {
    pub center: DVec3,
    pub radius: f64,
}

/// What a [`CollisionWorld::stream`] call changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamReport {
    pub cells_loaded: usize,
    pub cells_unloaded: usize,
    pub chunks_loaded: usize,
    pub chunks_unloaded: usize,
    /// Colliders in the world afterwards.
    pub colliders: usize,
}

/// Whether a body's motion comes from the simulation or is set from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyKind {
    /// Simulated: forces, gravity, contacts (a local physics Entity).
    Dynamic,
    /// Moved by the owner of the Entity (a remote Entity, a man, a non-physics vehicle);
    /// pushes dynamic bodies but is not pushed.
    Kinematic,
}

/// The state of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyState {
    pub position: DVec3,
    pub orientation: DQuat,
    pub linear_velocity: DVec3,
    pub angular_velocity: DVec3,
    pub sleeping: bool,
}

struct Loaded {
    colliders: Vec<ColliderHandle>,
    last_used: u64,
}

struct Body {
    handle: RigidBodyHandle,
    model: Arc<ModelCollision>,
    kind: BodyKind,
}

/// Standard gravity, m/s² _(uncertain: the engine's exact constant)_.
pub const GRAVITY: f64 = 9.8066;

/// The collision world. See the crate docs.
pub struct CollisionWorld {
    pub(crate) rapier: PhysicsWorld,
    pub(crate) terrain: Option<TerrainShape>,
    pub(crate) models: ModelBank,
    cells: HashMap<(u32, u32), Loaded>,
    chunks: HashMap<(u32, u32), Loaded>,
    bodies: HashMap<u64, Body>,
    shapes: HashMap<ColliderHandle, Arc<LayerShape>>,
    generation: u64,
    keep_generations: u64,
    placements: Vec<(u32, String, DAffine3)>,
}

impl std::fmt::Debug for CollisionWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionWorld")
            .field("cells", &self.cells.len())
            .field("chunks", &self.chunks.len())
            .field("bodies", &self.bodies.len())
            .field("colliders", &self.rapier.colliders.len())
            .finish()
    }
}

impl CollisionWorld {
    /// An empty world loading models through `models`.
    pub fn new(models: ModelBank) -> Self {
        let mut rapier = PhysicsWorld::new();
        rapier.gravity = Vector::new(0.0, -GRAVITY, 0.0);
        Self {
            rapier,
            terrain: None,
            models,
            cells: HashMap::new(),
            chunks: HashMap::new(),
            bodies: HashMap::new(),
            shapes: HashMap::new(),
            generation: 0,
            keep_generations: 60,
            placements: Vec::new(),
        }
    }

    /// Replaces the terrain (and drops every streamed cell and chunk).
    pub fn set_terrain(&mut self, terrain: Option<Arc<Terrain>>) {
        let cells: Vec<_> = self.cells.keys().copied().collect();
        for c in cells {
            self.unload(c, false);
        }
        let chunks: Vec<_> = self.chunks.keys().copied().collect();
        for c in chunks {
            self.unload(c, true);
        }
        self.terrain = terrain.map(TerrainShape::new);
    }

    pub fn terrain(&self) -> Option<&TerrainShape> {
        self.terrain.as_ref()
    }

    pub fn models(&mut self) -> &mut ModelBank {
        &mut self.models
    }

    pub fn surfaces(&self) -> &SurfaceBank {
        self.models.surfaces()
    }

    /// The surface behind a [`SurfaceId`] returned by a query.
    pub fn surface(&self, id: SurfaceId) -> &SurfaceInfo {
        self.models.surfaces().get(id)
    }

    /// How many [`stream`](Self::stream) calls a cell survives without being of interest
    /// (default 60).
    pub fn set_keep_generations(&mut self, generations: u64) {
        self.keep_generations = generations;
    }

    /// Number of land cells whose Static objects have colliders.
    pub fn loaded_cells(&self) -> usize {
        self.cells.len()
    }

    /// Whether land cell `(x, z)` has its colliders.
    pub fn is_cell_loaded(&self, x: u32, z: u32) -> bool {
        self.cells.contains_key(&(x, z))
    }

    /// Number of terrain heightfield chunks loaded.
    pub fn loaded_chunks(&self) -> usize {
        self.chunks.len()
    }

    /// Number of colliders in the world.
    pub fn collider_count(&self) -> usize {
        self.rapier.colliders.len()
    }

    // ---------------------------------------------------------------- streaming

    /// Loads the land cells and terrain chunks within each interest (on x/z), marks them used,
    /// and unloads those not used during the last `keep_generations` calls.
    pub fn stream(&mut self, statics: &dyn StaticSource, interests: &[Interest]) -> StreamReport {
        self.generation += 1;
        let mut report = StreamReport::default();
        for i in interests {
            let r = DVec2::splat(i.radius);
            let c = DVec2::new(i.center.x, i.center.z);
            self.touch_area(statics, c - r, c + r, &mut report);
        }
        let (g, keep) = (self.generation, self.keep_generations);
        let stale = |m: &HashMap<(u32, u32), Loaded>| -> Vec<(u32, u32)> {
            m.iter()
                .filter(|(_, l)| l.last_used + keep < g)
                .map(|(&k, _)| k)
                .collect()
        };
        for c in stale(&self.cells) {
            self.unload(c, false);
            report.cells_unloaded += 1;
        }
        for c in stale(&self.chunks) {
            self.unload(c, true);
            report.chunks_unloaded += 1;
        }
        report.colliders = self.rapier.colliders.len();
        report
    }

    /// Loads (and marks used) everything in the x/z rectangle `min..max`, for one-off queries
    /// far from any interest (scripts).
    pub fn load_area(
        &mut self,
        statics: &dyn StaticSource,
        min: DVec2,
        max: DVec2,
    ) -> StreamReport {
        let mut report = StreamReport::default();
        self.touch_area(statics, min, max, &mut report);
        report.colliders = self.rapier.colliders.len();
        report
    }

    fn touch_area(
        &mut self,
        statics: &dyn StaticSource,
        min: DVec2,
        max: DVec2,
        report: &mut StreamReport,
    ) {
        let range = |lo: f64, hi: f64, size: f64, n: u32| {
            let a = (lo / size).floor().max(0.0) as u32;
            let b = ((hi / size).floor().max(0.0) as u32).min(n.saturating_sub(1));
            a..=b
        };
        let grid = statics.land_grid();
        if grid.width > 0 && grid.height > 0 && grid.cell_size > 0.0 {
            for z in range(min.y, max.y, grid.cell_size, grid.height) {
                for x in range(min.x, max.x, grid.cell_size, grid.width) {
                    if let Some(l) = self.cells.get_mut(&(x, z)) {
                        l.last_used = self.generation;
                    } else {
                        self.load_cell(statics, x, z);
                        report.cells_loaded += 1;
                    }
                }
            }
        }
        if let Some(t) = &self.terrain {
            let (size, n) = (t.chunk_size(), t.chunks_per_side());
            let xs = range(min.x, max.x, size, n);
            for cz in range(min.y, max.y, size, n) {
                for cx in xs.clone() {
                    if let Some(l) = self.chunks.get_mut(&(cx, cz)) {
                        l.last_used = self.generation;
                    } else {
                        self.load_chunk(cx, cz);
                        report.chunks_loaded += 1;
                    }
                }
            }
        }
    }

    fn load_chunk(&mut self, cx: u32, cz: u32) {
        let Some(t) = &self.terrain else { return };
        let (shape, centre) = t.chunk(cx, cz);
        let collider = ColliderBuilder::new(shape)
            .translation(conv::vec(centre))
            .collision_groups(interaction(groups::TERRAIN, groups::BODY))
            .user_data(encode(ObjectKey::Terrain, None))
            .build();
        let h = self.rapier.insert_collider(collider, None);
        self.set_aabb(h);
        self.chunks.insert(
            (cx, cz),
            Loaded {
                colliders: vec![h],
                last_used: self.generation,
            },
        );
    }

    fn load_cell(&mut self, statics: &dyn StaticSource, x: u32, z: u32) {
        let mut placements = std::mem::take(&mut self.placements);
        placements.clear();
        statics.for_each_in_cell(x, z, &mut |p| {
            placements.push((p.key, p.model.to_owned(), p.transform));
        });
        let mut colliders = Vec::new();
        for (key, model, transform) in placements.drain(..) {
            let (pose, scale) = decompose(&transform);
            let Some(collision) = self.models.get_scaled(&model, scale) else {
                continue;
            };
            for layer in Layer::ALL {
                let Some(shape) = collision.layer(layer) else {
                    continue;
                };
                let filter = if layer == Layer::Geometry {
                    groups::BODY
                } else {
                    0
                };
                let collider = ColliderBuilder::new(shape.shape().clone())
                    .position(pose)
                    .collision_groups(interaction(layer_group(layer), filter))
                    .user_data(encode(ObjectKey::Static(key), Some(layer)))
                    .build();
                let h = self.rapier.insert_collider(collider, None);
                self.set_aabb(h);
                self.shapes.insert(h, shape.clone());
                colliders.push(h);
            }
        }
        self.placements = placements;
        self.cells.insert(
            (x, z),
            Loaded {
                colliders,
                last_used: self.generation,
            },
        );
    }

    fn unload(&mut self, key: (u32, u32), chunk: bool) {
        let loaded = if chunk {
            self.chunks.remove(&key)
        } else {
            self.cells.remove(&key)
        };
        for h in loaded.into_iter().flat_map(|l| l.colliders) {
            self.rapier.remove_collider(h);
            self.shapes.remove(&h);
        }
    }

    /// Drops the colliders of land cell `(x, z)` so that the next [`stream`](Self::stream)
    /// rebuilds them (after a Static object of the cell was removed or changed).
    pub fn invalidate_cell(&mut self, x: u32, z: u32) {
        self.unload((x, z), false);
    }

    /// The layer shape behind a collider (none for terrain chunks and PhysX contact shapes).
    pub(crate) fn layer_shape(&self, h: ColliderHandle) -> Option<&Arc<LayerShape>> {
        self.shapes.get(&h)
    }

    fn set_aabb(&mut self, h: ColliderHandle) {
        let aabb = self.rapier.colliders[h].compute_aabb();
        self.rapier
            .broad_phase
            .set_aabb(&self.rapier.integration_parameters, h, aabb);
    }

    // ---------------------------------------------------------------- bodies

    /// Gives an Entity a body with the colliders of `model` (every layer, for queries; the body
    /// shape, PhysX geometry or Geometry, takes part in contacts). The body's origin is the
    /// model origin; mass, centre of mass and inertia come from the model. Replaces any body
    /// the key had.
    pub fn add_body(
        &mut self,
        key: u64,
        model: Arc<ModelCollision>,
        position: DVec3,
        orientation: DQuat,
        kind: BodyKind,
    ) {
        self.remove_body(key);
        let mass = *model.mass();
        let builder = match kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };
        let mut builder = builder
            .pose(conv::pose(position, orientation))
            .user_data(encode(ObjectKey::Entity(key), None));
        let have_mass = mass.mass > 0.0;
        if have_mass {
            let com = conv::vec(mass.center_of_mass);
            let props = match mass.inertia {
                Some(i) => MassProperties::with_inertia_matrix(com, mass.mass, mat3(i)),
                None => MassProperties::new(com, mass.mass, Vector::splat(mass.mass)),
            };
            builder = builder.additional_mass_properties(props);
        }
        let handle = self.rapier.insert_body(builder);
        let entity = ObjectKey::Entity(key);
        let contact_groups = interaction(
            groups::GEOMETRY | groups::BODY,
            groups::TERRAIN | groups::GEOMETRY | groups::BODY,
        );
        let density = if have_mass { 0.0 } else { 1000.0 };
        let body_shape = model.body_shape().cloned();
        let mut handles = Vec::new();
        for layer in Layer::ALL {
            let Some(shape) = model.layer(layer) else {
                continue;
            };
            let contact = layer == Layer::Geometry
                && body_shape.as_ref().is_some_and(|b| Arc::ptr_eq(b, shape));
            let collider = ColliderBuilder::new(shape.shape().clone())
                .collision_groups(if contact {
                    contact_groups
                } else {
                    interaction(layer_group(layer), 0)
                })
                .density(if contact { density } else { 0.0 })
                .user_data(encode(entity, Some(layer)))
                .build();
            let h = self.rapier.insert_collider(collider, Some(handle));
            self.shapes.insert(h, shape.clone());
            handles.push(h);
        }
        if let Some(b) = &body_shape {
            if !model
                .layer(Layer::Geometry)
                .is_some_and(|g| Arc::ptr_eq(g, b))
            {
                // A separate PhysX shape: contacts only; queries use the Geometry layer.
                let collider = ColliderBuilder::new(b.shape().clone())
                    .collision_groups(interaction(
                        groups::BODY,
                        groups::TERRAIN | groups::GEOMETRY | groups::BODY,
                    ))
                    .density(density)
                    .user_data(encode(entity, None))
                    .build();
                handles.push(self.rapier.insert_collider(collider, Some(handle)));
            }
        }
        for h in handles {
            self.set_aabb(h);
        }
        // Now, not at the next step: forces and impulses may come before it.
        self.rapier.bodies[handle].recompute_mass_properties_from_colliders(&self.rapier.colliders);
        self.bodies.insert(
            key,
            Body {
                handle,
                model,
                kind,
            },
        );
    }

    /// Removes an Entity's body and colliders. Returns `false` if it had none.
    pub fn remove_body(&mut self, key: u64) -> bool {
        let Some(b) = self.bodies.remove(&key) else {
            return false;
        };
        for h in self.rapier.bodies[b.handle].colliders() {
            self.shapes.remove(h);
        }
        self.rapier.remove_body(b.handle);
        true
    }

    pub fn has_body(&self, key: u64) -> bool {
        self.bodies.contains_key(&key)
    }

    /// The model a body was made from.
    pub fn body_model(&self, key: u64) -> Option<&Arc<ModelCollision>> {
        self.bodies.get(&key).map(|b| &b.model)
    }

    pub fn body_kind(&self, key: u64) -> Option<BodyKind> {
        self.bodies.get(&key).map(|b| b.kind)
    }

    /// Switches a body between simulated and externally moved (when the Entity's Locality
    /// changes: only local Entities are simulated).
    pub fn set_body_kind(&mut self, key: u64, kind: BodyKind) {
        let Some(b) = self.bodies.get_mut(&key) else {
            return;
        };
        b.kind = kind;
        let ty = match kind {
            BodyKind::Dynamic => RigidBodyType::Dynamic,
            BodyKind::Kinematic => RigidBodyType::KinematicPositionBased,
        };
        self.rapier.bodies[b.handle].set_body_type(ty, true);
    }

    /// Places a body. A kinematic body moves there during the next step (pushing what is in
    /// the way); a dynamic body is teleported. Its colliders follow at once for queries.
    pub fn set_body_pose(&mut self, key: u64, position: DVec3, orientation: DQuat) {
        let Some(b) = self.bodies.get(&key) else {
            return;
        };
        let pose = conv::pose(position, orientation);
        let body = &mut self.rapier.bodies[b.handle];
        match b.kind {
            BodyKind::Kinematic => body.set_next_kinematic_position(pose),
            BodyKind::Dynamic => body.set_position(pose, true),
        }
        let colliders = body.colliders().to_vec();
        for h in colliders {
            let co = &mut self.rapier.colliders[h];
            let local = co.position_wrt_parent().copied().unwrap_or(Pose::IDENTITY);
            co.set_position(pose * local);
            self.set_aabb(h);
        }
    }

    pub fn set_body_velocity(&mut self, key: u64, linear: DVec3, angular: DVec3) {
        if let Some(b) = self.bodies.get(&key) {
            let body = &mut self.rapier.bodies[b.handle];
            body.set_linvel(conv::vec(linear), true);
            body.set_angvel(conv::vec(angular), true);
        }
    }

    /// Adds a force (N) at a world point for the next step.
    pub fn add_force_at(&mut self, key: u64, force: DVec3, point: DVec3) {
        if let Some(b) = self.bodies.get(&key) {
            self.rapier.bodies[b.handle].add_force_at_point(
                conv::vec(force),
                conv::vec(point),
                true,
            );
        }
    }

    /// Applies an impulse (N·s) at a world point now.
    pub fn apply_impulse_at(&mut self, key: u64, impulse: DVec3, point: DVec3) {
        if let Some(b) = self.bodies.get(&key) {
            self.rapier.bodies[b.handle].apply_impulse_at_point(
                conv::vec(impulse),
                conv::vec(point),
                true,
            );
        }
    }

    pub fn body(&self, key: u64) -> Option<BodyState> {
        let b = self.bodies.get(&key)?;
        let body = &self.rapier.bodies[b.handle];
        let pose = body.position();
        Some(BodyState {
            position: conv::dvec(pose.translation),
            orientation: conv::dquat(pose.rotation),
            linear_velocity: conv::dvec(body.linvel()),
            angular_velocity: conv::dvec(body.angvel()),
            sleeping: body.is_sleeping(),
        })
    }

    /// The keys of every body.
    pub fn body_keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.bodies.keys().copied()
    }

    /// One rigid-body step of `dt` seconds (call it with the fixed step, ADR 0002).
    pub fn step(&mut self, dt: f64) {
        self.rapier.integration_parameters.dt = dt;
        self.rapier.step();
    }
}

fn mat3(m: DMat3) -> rapier3d_f64::math::Matrix {
    rapier3d_f64::math::Matrix::from_cols_array(&m.to_cols_array())
}

/// Splits a WRP transform into a rigid pose and a uniform scale (the mean column length).
pub(crate) fn decompose(t: &DAffine3) -> (Pose, f64) {
    let m = t.matrix3;
    let lengths = [m.x_axis.length(), m.y_axis.length(), m.z_axis.length()];
    let scale = (lengths[0] + lengths[1] + lengths[2]) / 3.0;
    let rot = if lengths.iter().all(|&l| l > 1e-9) {
        DMat3::from_cols(
            m.x_axis / lengths[0],
            m.y_axis / lengths[1],
            m.z_axis / lengths[2],
        )
    } else {
        DMat3::IDENTITY
    };
    let q = DQuat::from_mat3(&rot).normalize();
    (conv::pose(t.translation, q), scale)
}
