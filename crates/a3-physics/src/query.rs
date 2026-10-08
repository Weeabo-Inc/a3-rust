//! Queries on the collision world: rays per layer, penetrations, shape casts and overlaps,
//! surface heights and visibility.
//!
//! Queries see the Static objects of loaded land cells, every Entity body, and the terrain. Ray
//! queries test the terrain exactly over the whole map; shape queries see only the loaded
//! terrain chunks.

use glam::{DQuat, DVec3};
use rapier3d_f64::parry::query::ShapeCastOptions;
use rapier3d_f64::parry::shape::{FeatureId, Shape};
use rapier3d_f64::prelude::*;

use crate::conv;
use crate::world::decode;
use std::sync::Arc;

use crate::{CollisionWorld, Component, Layer, LayerMask, LayerShape, ObjectKey, SurfaceId};

/// A segment query from `from` to `to`.
#[derive(Debug, Clone, Copy)]
pub struct RayQuery<'a> {
    pub from: DVec3,
    pub to: DVec3,
    /// The layers of Objects to test.
    pub layers: LayerMask,
    /// Whether the terrain surface counts.
    pub terrain: bool,
    /// Objects to pass through (the shooter, the vehicle the eye is in).
    pub ignore: &'a [ObjectKey],
}

impl<'a> RayQuery<'a> {
    /// A segment against `layers` and the terrain.
    pub fn new(from: DVec3, to: DVec3, layers: impl Into<LayerMask>) -> Self {
        Self {
            from,
            to,
            layers: layers.into(),
            terrain: true,
            ignore: &[],
        }
    }

    pub fn without_terrain(mut self) -> Self {
        self.terrain = false;
        self
    }

    pub fn ignoring(mut self, objects: &'a [ObjectKey]) -> Self {
        self.ignore = objects;
        self
    }
}

/// Where a ray met an Object or the terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub object: ObjectKey,
    /// The layer that was hit; `None` for the terrain.
    pub layer: Option<Layer>,
    /// The convex component (index into the layer's components), or the triangle of a Roadway
    /// mesh; `None` for the terrain.
    pub component: Option<u32>,
    /// The surface material of what was hit, when known.
    pub surface: Option<SurfaceId>,
    /// The layer shape that was hit, for [`CollisionWorld::component`]; `None` for the
    /// terrain.
    pub shape: Option<ShapeRef>,
    pub position: DVec3,
    /// Unit normal of the surface, facing the ray's origin side.
    pub normal: DVec3,
    /// Metres from `from`.
    pub distance: f64,
}

/// A convex component a segment passes through, with where it enters and leaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Penetration {
    /// The entry (distance 0 when the segment starts inside).
    pub entry: RayHit,
    /// Metres from `from` where it leaves (the segment's length when it ends inside).
    pub exit_distance: f64,
}

impl Penetration {
    /// Metres of material along the segment.
    pub fn depth(&self) -> f64 {
        self.exit_distance - self.entry.distance
    }
}

/// A primitive shape for shape casts and overlap tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryShape {
    Sphere {
        radius: f64,
    },
    /// Upright (local Y) capsule: segment from `-half_height` to `+half_height`, then `radius`.
    Capsule {
        half_height: f64,
        radius: f64,
    },
    Box {
        half_extents: DVec3,
    },
    /// Capsule around the local segment `a`..`b` (a skeleton bone's collision capsule).
    Segment {
        a: DVec3,
        b: DVec3,
        radius: f64,
    },
}

impl QueryShape {
    fn to_shape(self) -> SharedShape {
        match self {
            QueryShape::Sphere { radius } => SharedShape::ball(radius),
            QueryShape::Capsule {
                half_height,
                radius,
            } => SharedShape::capsule_y(half_height, radius),
            QueryShape::Box { half_extents: h } => SharedShape::cuboid(h.x, h.y, h.z),
            QueryShape::Segment { a, b, radius } => {
                SharedShape::capsule(conv::vec(a), conv::vec(b), radius)
            }
        }
    }
}

/// Where a moving shape first touches something.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeHit {
    pub object: ObjectKey,
    pub layer: Option<Layer>,
    pub component: Option<u32>,
    pub surface: Option<SurfaceId>,
    pub shape: Option<ShapeRef>,
    /// Fraction of the motion travelled before contact (0 when touching at the start).
    pub fraction: f64,
    /// The contact point on the hit Object.
    pub position: DVec3,
    /// The hit Object's outward normal at the contact.
    pub normal: DVec3,
}

/// An Object (component) a shape or point overlaps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlap {
    pub object: ObjectKey,
    pub layer: Option<Layer>,
    pub component: Option<u32>,
    pub surface: Option<SurfaceId>,
    pub shape: Option<ShapeRef>,
}

/// A contact between a query shape and an Object: where they touch or overlap and how to
/// separate them (character collision response).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeContact {
    pub object: ObjectKey,
    pub layer: Option<Layer>,
    /// The convex component (or Roadway triangle) touched.
    pub component: Option<u32>,
    pub surface: Option<SurfaceId>,
    pub shape: Option<ShapeRef>,
    /// The deepest point on the Object's surface.
    pub point: DVec3,
    /// The Object's outward normal there: moving the query shape by `normal * -distance`
    /// separates them.
    pub normal: DVec3,
    /// Signed distance between the surfaces: negative when overlapping (penetration depth).
    pub distance: f64,
}

/// The walkable surface under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSample {
    /// Height of the surface (world `y`).
    pub y: f64,
    pub normal: DVec3,
    /// [`ObjectKey::Terrain`] or the Object whose Roadway it is.
    pub object: ObjectKey,
    pub surface: Option<SurfaceId>,
}

/// Refers to the layer shape of one collider, as long as its land cell or body stays loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeRef(ColliderHandle);

impl CollisionWorld {
    fn shape_ref(&self, h: ColliderHandle) -> Option<ShapeRef> {
        self.layer_shape(h).map(|_| ShapeRef(h))
    }

    /// The layer shape a hit refers to (`None` once its cell or body is unloaded).
    pub fn shape(&self, shape: ShapeRef) -> Option<&Arc<LayerShape>> {
        self.layer_shape(shape.0)
    }

    /// The convex component a hit refers to: its selection name (`component03`, for matching
    /// hit points' `convexComponent`) and surface.
    pub fn component(&self, shape: ShapeRef, component: u32) -> Option<&Component> {
        self.layer_shape(shape.0)?
            .components()
            .get(component as usize)
    }

    fn filter_fn<'a>(
        layers: LayerMask,
        terrain: bool,
        ignore: &'a [ObjectKey],
    ) -> impl Fn(ColliderHandle, &Collider) -> bool + 'a {
        move |_, co| {
            let (object, layer) = decode(co.user_data);
            if ignore.contains(&object) {
                return false;
            }
            match layer {
                Some(l) => layers.contains(l),
                None => terrain && object == ObjectKey::Terrain,
            }
        }
    }

    fn surface_of(&self, h: ColliderHandle, part: Option<u32>) -> Option<SurfaceId> {
        self.layer_shape(h)?.surface(part?)
    }

    /// Every crossing of the compound parts (or the first of a mesh) of one collider.
    fn collider_ray_hits(
        &self,
        h: ColliderHandle,
        co: &Collider,
        ray: &Ray,
        max: f64,
        out: &mut Vec<RayHit>,
    ) {
        let (object, layer) = decode(co.user_data);
        let pos = co.position();
        let mut push = |part: Option<u32>, hit: RayIntersection| {
            let mut normal = conv::dvec(hit.normal);
            let dir = conv::dvec(ray.dir);
            if normal.dot(dir) > 0.0 {
                normal = -normal;
            }
            out.push(RayHit {
                object,
                layer,
                component: part,
                surface: self.surface_of(h, part),
                shape: self.shape_ref(h),
                position: conv::dvec(ray.point_at(hit.time_of_impact)),
                normal: normal.normalize_or_zero(),
                distance: hit.time_of_impact,
            });
        };
        if let Some(compound) = co.shape().as_compound() {
            for (i, (p, part)) in compound.shapes().iter().enumerate() {
                if let Some(hit) = part.cast_ray_and_get_normal(&(*pos * *p), ray, max, false) {
                    push(Some(i as u32), hit);
                }
            }
        } else if let Some(hit) = co.shape().cast_ray_and_get_normal(pos, ray, max, false) {
            let part = match hit.feature {
                FeatureId::Face(f) => Some(f),
                _ => None,
            };
            push(part, hit);
        }
    }

    fn terrain_ray(&self, q: &RayQuery<'_>) -> Option<RayHit> {
        if !q.terrain || q.ignore.contains(&ObjectKey::Terrain) {
            return None;
        }
        let t = self.terrain.as_ref()?;
        let d = q.to - q.from;
        let len = d.length();
        if len <= 0.0 {
            return None;
        }
        let hit = t.cast_ray(q.from, d / len, len)?;
        Some(RayHit {
            object: ObjectKey::Terrain,
            layer: None,
            component: None,
            surface: None,
            shape: None,
            position: hit.position,
            normal: hit.normal,
            distance: hit.distance,
        })
    }

    /// Every surface the segment meets, nearest first: the first crossing of each convex
    /// component, of each Roadway mesh, and of the terrain.
    pub fn ray_cast_all(&self, q: &RayQuery<'_>) -> Vec<RayHit> {
        let mut hits = Vec::new();
        let d = q.to - q.from;
        let len = d.length();
        if len > 0.0 {
            let ray = Ray::new(conv::vec(q.from), conv::vec(d / len));
            let filter_fn = Self::filter_fn(q.layers, false, q.ignore);
            let filter = QueryFilter::default().predicate(&filter_fn);
            let pipeline = self.rapier.query_pipeline_with_filter(filter);
            for (h, co, _) in pipeline.intersect_ray(ray, len, false) {
                self.collider_ray_hits(h, co, &ray, len, &mut hits);
            }
        }
        hits.extend(self.terrain_ray(q));
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }

    /// The nearest surface crossing along the segment (`lineIntersectsSurfaces` with one
    /// result, `terrainIntersect` with only the terrain).
    pub fn ray_cast(&self, q: &RayQuery<'_>) -> Option<RayHit> {
        let d = q.to - q.from;
        let len = d.length();
        let mut best = self.terrain_ray(q);
        if len > 0.0 {
            let ray = Ray::new(conv::vec(q.from), conv::vec(d / len));
            let max = best.map_or(len, |b| b.distance);
            let filter_fn = Self::filter_fn(q.layers, false, q.ignore);
            let filter = QueryFilter::default().predicate(&filter_fn);
            let pipeline = self.rapier.query_pipeline_with_filter(filter);
            if let Some((h, first)) = pipeline.cast_ray(&ray, max, false) {
                let mut hits = Vec::new();
                self.collider_ray_hits(h, &self.rapier.colliders[h], &ray, first + 1e-6, &mut hits);
                if let Some(hit) = hits
                    .into_iter()
                    .min_by(|a, b| a.distance.total_cmp(&b.distance))
                {
                    if best.is_none_or(|b| hit.distance < b.distance) {
                        best = Some(hit);
                    }
                }
            }
        }
        best
    }

    /// Whether anything (in `q.layers` or the terrain) blocks the segment.
    pub fn ray_blocked(&self, q: &RayQuery<'_>) -> bool {
        self.ray_cast(q).is_some()
    }

    /// Every convex component the segment passes through, with entry and exit, nearest entry
    /// first (bullet penetration). Mesh layers (Roadway) report entry = exit.
    pub fn penetrations(&self, q: &RayQuery<'_>) -> Vec<Penetration> {
        let d = q.to - q.from;
        let len = d.length();
        let mut out = Vec::new();
        if len <= 0.0 {
            return out;
        }
        let dir = d / len;
        let ray = Ray::new(conv::vec(q.from), conv::vec(dir));
        let back = Ray::new(conv::vec(q.to), conv::vec(-dir));
        let filter_fn = Self::filter_fn(q.layers, false, q.ignore);
        let filter = QueryFilter::default().predicate(&filter_fn);
        let pipeline = self.rapier.query_pipeline_with_filter(filter);
        for (h, co, _) in pipeline.intersect_ray(ray, len, true) {
            let (object, layer) = decode(co.user_data);
            let pos = co.position();
            let mut push = |part: Option<u32>, shape: &dyn Shape, pose: Pose| {
                let Some(entry) = shape.cast_ray_and_get_normal(&pose, &ray, len, true) else {
                    return;
                };
                let exit = shape
                    .cast_ray(&pose, &back, len, true)
                    .map_or(entry.time_of_impact, |t| len - t);
                let mut normal = conv::dvec(entry.normal);
                if normal.dot(dir) > 0.0 {
                    normal = -normal;
                }
                let part = if shape.as_trimesh().is_some() {
                    match entry.feature {
                        FeatureId::Face(f) => Some(f),
                        _ => None,
                    }
                } else {
                    part
                };
                out.push(Penetration {
                    entry: RayHit {
                        object,
                        layer,
                        component: part,
                        surface: self.surface_of(h, part),
                        shape: self.shape_ref(h),
                        position: conv::dvec(ray.point_at(entry.time_of_impact)),
                        normal: normal.normalize_or_zero(),
                        distance: entry.time_of_impact,
                    },
                    exit_distance: exit.max(entry.time_of_impact),
                });
            };
            match co.shape().as_compound() {
                Some(compound) => {
                    for (i, (p, part)) in compound.shapes().iter().enumerate() {
                        push(Some(i as u32), part.as_ref(), *pos * *p);
                    }
                }
                None => push(None, co.shape(), *pos),
            }
        }
        out.sort_by(|a, b| a.entry.distance.total_cmp(&b.entry.distance));
        out
    }

    /// The first contact of `shape` (at `position`, `orientation`) moved by `motion`, against
    /// `layers` and, if `terrain`, the loaded terrain chunks.
    #[allow(clippy::too_many_arguments)]
    pub fn shape_cast(
        &self,
        shape: QueryShape,
        position: DVec3,
        orientation: DQuat,
        motion: DVec3,
        layers: LayerMask,
        terrain: bool,
        ignore: &[ObjectKey],
    ) -> Option<ShapeHit> {
        let shape = shape.to_shape();
        let pose = conv::pose(position, orientation);
        let filter_fn = Self::filter_fn(layers, terrain, ignore);
        let filter = QueryFilter::default().predicate(&filter_fn);
        let pipeline = self.rapier.query_pipeline_with_filter(filter);
        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };
        let (h, hit) = pipeline.cast_shape(&pose, conv::vec(motion), shape.as_ref(), options)?;
        let co = &self.rapier.colliders[h];
        let (object, layer) = decode(co.user_data);
        let witness = hit.witness1;
        let part = self.closest_part(co, witness);
        Some(ShapeHit {
            object,
            layer,
            component: part,
            surface: self.surface_of(h, part),
            shape: self.shape_ref(h),
            fraction: hit.time_of_impact,
            position: conv::dvec(witness),
            normal: conv::dvec(hit.normal1).normalize_or_zero(),
        })
    }

    fn closest_part(&self, co: &Collider, point: Vector) -> Option<u32> {
        let pos = co.position();
        if let Some(compound) = co.shape().as_compound() {
            compound
                .shapes()
                .iter()
                .enumerate()
                .map(|(i, (p, s))| (i, s.distance_to_point(&(*pos * *p), point, true)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i as u32)
        } else if co.shape().as_trimesh().is_some() {
            let (_, feature) = co.shape().project_point_and_get_feature(pos, point);
            match feature {
                FeatureId::Face(f) => Some(f),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Every Object component `shape` (at `position`, `orientation`) overlaps.
    pub fn overlaps(
        &self,
        shape: QueryShape,
        position: DVec3,
        orientation: DQuat,
        layers: LayerMask,
        terrain: bool,
        ignore: &[ObjectKey],
    ) -> Vec<Overlap> {
        let shape = shape.to_shape();
        let pose = conv::pose(position, orientation);
        let filter_fn = Self::filter_fn(layers, terrain, ignore);
        let filter = QueryFilter::default().predicate(&filter_fn);
        let pipeline = self.rapier.query_pipeline_with_filter(filter);
        let mut out = Vec::new();
        for (h, co) in pipeline.intersect_shape(pose, shape.as_ref()) {
            let (object, layer) = decode(co.user_data);
            let mut push = |part: Option<u32>| {
                out.push(Overlap {
                    object,
                    layer,
                    component: part,
                    surface: self.surface_of(h, part),
                    shape: self.shape_ref(h),
                });
            };
            match co.shape().as_compound() {
                Some(compound) => {
                    for (i, (p, part)) in compound.shapes().iter().enumerate() {
                        let hit = rapier3d_f64::parry::query::intersection_test(
                            &(*co.position() * *p),
                            part.as_ref(),
                            &pose,
                            shape.as_ref(),
                        );
                        if hit.is_ok_and(|h| h.intersecting) {
                            push(Some(i as u32));
                        }
                    }
                }
                None => push(None),
            }
        }
        out
    }

    /// The contacts of `shape` (at `position`, `orientation`) with every Object within
    /// `margin` of it: one per collider, at its deepest point (terrain chunks included when
    /// `terrain`).
    #[allow(clippy::too_many_arguments)]
    pub fn contacts(
        &self,
        shape: QueryShape,
        position: DVec3,
        orientation: DQuat,
        margin: f64,
        layers: LayerMask,
        terrain: bool,
        ignore: &[ObjectKey],
    ) -> Vec<ShapeContact> {
        let query = shape.to_shape();
        let pose = conv::pose(position, orientation);
        // The broad phase finds candidates by the shape grown by the margin.
        let grown = SharedShape::ball(query.compute_local_bounding_sphere().radius + margin);
        let filter_fn = Self::filter_fn(layers, terrain, ignore);
        let filter = QueryFilter::default().predicate(&filter_fn);
        let pipeline = self.rapier.query_pipeline_with_filter(filter);
        let mut out = Vec::new();
        let centre = pose * query.compute_local_bounding_sphere().center;
        for (h, co) in pipeline.intersect_shape(Pose::from_translation(centre), grown.as_ref()) {
            let Ok(Some(c)) = rapier3d_f64::parry::query::contact(
                co.position(),
                co.shape(),
                &pose,
                query.as_ref(),
                margin,
            ) else {
                continue;
            };
            let (object, layer) = decode(co.user_data);
            let part = (co.shape().as_compound().is_some() || co.shape().as_trimesh().is_some())
                .then_some(c.subshape1);
            out.push(ShapeContact {
                object,
                layer,
                component: part,
                surface: self.surface_of(h, part),
                shape: self.shape_ref(h),
                point: conv::dvec(c.point1),
                normal: conv::dvec(c.normal1),
                distance: c.dist,
            });
        }
        out
    }

    /// Every Object component in `layers` that contains `point` (`isInside`-style tests:
    /// is a position inside a building's Geometry).
    pub fn objects_at(&self, point: DVec3, layers: LayerMask) -> Vec<Overlap> {
        let filter_fn = Self::filter_fn(layers, false, &[]);
        let filter = QueryFilter::default().predicate(&filter_fn);
        let pipeline = self.rapier.query_pipeline_with_filter(filter);
        let p = conv::vec(point);
        let mut out = Vec::new();
        for (h, co) in pipeline.intersect_point(p) {
            let (object, layer) = decode(co.user_data);
            let parts: Vec<Option<u32>> = match co.shape().as_compound() {
                Some(compound) => compound
                    .shapes()
                    .iter()
                    .enumerate()
                    .filter(|(_, (pp, s))| s.contains_point(&(*co.position() * *pp), p))
                    .map(|(i, _)| Some(i as u32))
                    .collect(),
                None => vec![None],
            };
            for part in parts {
                out.push(Overlap {
                    object,
                    layer,
                    component: part,
                    surface: self.surface_of(h, part),
                    shape: self.shape_ref(h),
                });
            }
        }
        out
    }

    /// The highest walkable surface at `(point.x, point.z)` at most `max_drop` metres below
    /// `point.y`: a Roadway (bridge deck, floor, stairs) or the terrain.
    pub fn surface_below(&self, point: DVec3, max_drop: f64) -> Option<SurfaceSample> {
        let q = RayQuery::new(point, point - DVec3::Y * max_drop, Layer::Roadway);
        self.ray_cast(&q).map(|hit| SurfaceSample {
            y: hit.position.y,
            normal: if hit.normal.y < 0.0 {
                -hit.normal
            } else {
                hit.normal
            },
            object: hit.object,
            surface: hit.surface,
        })
    }

    /// How much of the line from `from` to `to` is unobstructed, from 0 (blocked by the
    /// terrain or an opaque View Geometry component) to 1 (`checkVisibility`). A component
    /// whose surface has a `transparency` of 0..1 lets that fraction through _(approximation
    /// of the engine's rule)_.
    pub fn visibility(&self, from: DVec3, to: DVec3, ignore: &[ObjectKey]) -> f64 {
        let q = RayQuery::new(from, to, Layer::ViewGeometry).ignoring(ignore);
        let mut seen = 1.0;
        let mut counted = std::collections::HashSet::new();
        for hit in self.ray_cast_all(&q) {
            if hit.object == ObjectKey::Terrain {
                return 0.0;
            }
            if !counted.insert((hit.object, hit.component)) {
                continue; // The exit of a component already entered.
            }
            let t = hit.surface.map_or(-1.0, |s| self.surface(s).transparency);
            if t <= 0.0 {
                return 0.0;
            }
            seen *= f64::from(t);
        }
        seen
    }
}
