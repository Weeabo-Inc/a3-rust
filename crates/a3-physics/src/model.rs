//! Collision shapes of a model, built once per model from its special LODs and shared by every
//! Object that uses it.
//!
//! Geometry, Fire Geometry and View Geometry LODs are sets of convex components (the binariser
//! stores each as a named selection `ComponentNN`); each becomes one convex hull. A Roadway LOD
//! is an open surface and becomes a triangle mesh.

use std::collections::HashMap;
use std::sync::Arc;

use a3_p3d::{Lod, LodKind, Model};
use glam::{DMat3, DVec3};
use rapier3d_f64::math::{Pose, Vector};
use rapier3d_f64::prelude::SharedShape;

use crate::conv;
use crate::{Layer, SurfaceBank, SurfaceId};

/// One convex component of a layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// The named selection it came from (`component01`), or the LOD name when the LOD has no
    /// component selections and is taken as one piece.
    pub name: String,
    /// The surface of its faces (the material of its first face).
    pub surface: Option<SurfaceId>,
}

/// The collision shape of one layer of a model.
#[derive(Clone)]
pub struct LayerShape {
    shape: SharedShape,
    components: Vec<Component>,
    /// Mesh layers: the surface of each triangle.
    triangle_surfaces: Vec<Option<SurfaceId>>,
}

impl std::fmt::Debug for LayerShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerShape")
            .field("components", &self.components.len())
            .field("mesh", &self.is_mesh())
            .finish()
    }
}

impl LayerShape {
    /// A compound of convex hulls, part `i` being component `i`, or a triangle mesh.
    pub fn shape(&self) -> &SharedShape {
        &self.shape
    }

    /// The convex components (empty for mesh layers).
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    pub fn is_mesh(&self) -> bool {
        self.shape.as_trimesh().is_some()
    }

    /// Number of triangles of a mesh layer.
    pub fn triangle_count(&self) -> usize {
        self.triangle_surfaces.len()
    }

    /// The surface of a component, or of a triangle of a mesh layer.
    pub fn surface(&self, part: u32) -> Option<SurfaceId> {
        if self.is_mesh() {
            let n = self.triangle_surfaces.len().max(1);
            self.triangle_surfaces
                .get(part as usize % n)
                .copied()
                .flatten()
        } else {
            self.components.get(part as usize).and_then(|c| c.surface)
        }
    }

    fn convex(parts: Vec<(Component, Vec<DVec3>)>) -> Option<Self> {
        let mut components = Vec::new();
        let mut shapes = Vec::new();
        for (component, p) in parts {
            let rp: Vec<_> = p.iter().map(|&v| conv::vec(v)).collect();
            let Some(hull) = SharedShape::convex_hull(&rp) else {
                continue;
            };
            shapes.push((Pose::IDENTITY, hull));
            components.push(component);
        }
        if shapes.is_empty() {
            return None;
        }
        Some(Self {
            shape: SharedShape::compound(shapes),
            components,
            triangle_surfaces: Vec::new(),
        })
    }

    fn mesh(
        vertices: Vec<DVec3>,
        triangles: Vec<[u32; 3]>,
        triangle_surfaces: Vec<Option<SurfaceId>>,
    ) -> Option<Self> {
        if triangles.is_empty() {
            return None;
        }
        let shape =
            SharedShape::trimesh(vertices.iter().map(|&v| conv::vec(v)).collect(), triangles)
                .ok()?;
        Some(Self {
            shape,
            components: Vec::new(),
            triangle_surfaces,
        })
    }

    /// The same layer uniformly scaled by `s` about the model origin. Scaling keeps hulls
    /// convex, so the existing hulls are scaled rather than rebuilt.
    pub(crate) fn scaled(&self, s: f64) -> Option<Self> {
        let scale = Vector::splat(s);
        let shape = if let Some(mesh) = self.shape.as_trimesh() {
            SharedShape::new(mesh.clone().scaled(scale))
        } else {
            let parts = self
                .shape
                .as_compound()?
                .shapes()
                .iter()
                .map(|(p, part)| {
                    let hull = part.as_convex_polyhedron()?.clone().scaled(scale)?;
                    Some((
                        Pose::from_parts(p.translation * s, p.rotation),
                        SharedShape::new(hull),
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            SharedShape::compound(parts)
        };
        Some(Self {
            shape,
            components: self.components.clone(),
            triangle_surfaces: self.triangle_surfaces.clone(),
        })
    }
}

/// Mass properties from the model (ODOL `ModelInfo`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    /// Total mass in kg (0 for massless models).
    pub mass: f64,
    /// Centre of mass in model space.
    pub center_of_mass: DVec3,
    /// Inertia tensor about the centre of mass, from the stored inverse; `None` when the model
    /// stores none (all zero).
    pub inertia: Option<DMat3>,
}

/// Every collision layer of one model.
#[derive(Debug, Clone)]
pub struct ModelCollision {
    layers: [Option<Arc<LayerShape>>; 4],
    physx: Option<Arc<LayerShape>>,
    mass: MassProperties,
}

impl ModelCollision {
    /// Builds the layers of `model`, resolving face materials to surfaces through `surfaces`.
    pub fn from_model(model: &Model, surfaces: &mut SurfaceBank) -> Self {
        let special = &model.info.special_lods;
        let by_kind = |kinds: &[LodKind]| {
            kinds
                .iter()
                .find_map(|k| model.lods.iter().position(|l| l.resolution.kind() == *k))
        };
        let index = |i: Option<u8>| i.map(usize::from).filter(|&i| i < model.lods.len());
        let geometry = index(special.geometry).or_else(|| by_kind(&[LodKind::Geometry]));
        let fire = index(special.fire_geometry).or_else(|| {
            by_kind(&[
                LodKind::FireGeometry,
                LodKind::ViewGeometry,
                LodKind::Geometry,
            ])
        });
        let view = index(special.view_geometry)
            .or_else(|| by_kind(&[LodKind::ViewGeometry, LodKind::Geometry]));
        let roadway = index(special.roadway).or_else(|| by_kind(&[LodKind::Roadway]));
        let physx = index(special.geometry_physx).or_else(|| by_kind(&[LodKind::GeometryPhysx]));

        let mut built: HashMap<usize, Option<Arc<LayerShape>>> = HashMap::new();
        let mut convex = |i: Option<usize>, surfaces: &mut SurfaceBank| {
            let i = i?;
            built
                .entry(i)
                .or_insert_with(|| convex_layer(&model.lods[i], surfaces).map(Arc::new))
                .clone()
        };
        let layers = [
            convex(geometry, surfaces),
            convex(fire, surfaces),
            convex(view, surfaces),
            roadway.and_then(|i| mesh_layer(&model.lods[i], surfaces).map(Arc::new)),
        ];
        let physx = convex(physx, surfaces);

        let info = &model.info;
        let inv = DMat3::from_cols_array(&info.inv_inertia.to_cols_array().map(f64::from));
        let inertia = (inv.determinant().abs() > 1e-20).then(|| inv.inverse());
        Self {
            layers,
            physx,
            mass: MassProperties {
                mass: f64::from(info.mass),
                center_of_mass: info.center_of_mass.as_dvec3(),
                inertia,
            },
        }
    }

    /// The shape of `layer`, if the model has that LOD (or its fallback).
    pub fn layer(&self, layer: Layer) -> Option<&Arc<LayerShape>> {
        self.layers[layer as usize].as_ref()
    }

    /// The shape a rigid body collides with: the PhysX geometry when the model has one, else
    /// the Geometry LOD.
    pub fn body_shape(&self) -> Option<&Arc<LayerShape>> {
        self.physx.as_ref().or(self.layer(Layer::Geometry))
    }

    pub fn mass(&self) -> &MassProperties {
        &self.mass
    }

    /// `true` if no layer has any shape.
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(Option::is_none)
    }

    /// The model uniformly scaled by `s` (map objects placed with a scale).
    pub fn scaled(&self, s: f64) -> Self {
        let scale =
            |l: &Option<Arc<LayerShape>>| l.as_ref().and_then(|l| l.scaled(s)).map(Arc::new);
        Self {
            layers: [
                scale(&self.layers[0]),
                scale(&self.layers[1]),
                scale(&self.layers[2]),
                scale(&self.layers[3]),
            ],
            physx: scale(&self.physx),
            mass: MassProperties {
                mass: self.mass.mass * s * s * s,
                center_of_mass: self.mass.center_of_mass * s,
                inertia: self.mass.inertia.map(|i| i * s.powi(5)),
            },
        }
    }
}

/// Surfaces of a LOD's sections, resolved once each.
struct SectionSurfaces {
    ranges: Vec<(std::ops::Range<u32>, Option<SurfaceId>, bool)>,
}

impl SectionSurfaces {
    fn new(lod: &Lod, surfaces: &mut SurfaceBank) -> Self {
        let ranges = lod
            .sections
            .iter()
            .map(|s| {
                let material = s
                    .material
                    .and_then(|m| lod.materials.get(m as usize))
                    .map(|m| m.surface.as_str())
                    .filter(|m| !m.is_empty());
                let surface = match material {
                    Some(m) => Some(surfaces.surface(m)),
                    None => s
                        .texture
                        .and_then(|t| lod.textures.get(t as usize))
                        .and_then(|t| surfaces.for_texture(t)),
                };
                (s.faces.clone(), surface, s.is_proxy())
            })
            .collect();
        Self { ranges }
    }

    fn of(&self, face: u32) -> (Option<SurfaceId>, bool) {
        self.ranges
            .iter()
            .find(|(r, _, _)| r.contains(&face))
            .map_or((None, false), |&(_, s, proxy)| (s, proxy))
    }
}

fn convex_layer(lod: &Lod, surfaces: &mut SurfaceBank) -> Option<LayerShape> {
    let sections = SectionSurfaces::new(lod, surfaces);
    let positions = &lod.vertices.positions;
    let point = |v: u32| positions.get(v as usize).map(|p| p.as_dvec3());
    let selections: Vec<_> = lod
        .named_selections
        .iter()
        .filter(|s| s.name.to_ascii_lowercase().starts_with("component"))
        .collect();
    let parts: Vec<(Component, Vec<DVec3>)> = if selections.is_empty() {
        let mut used = vec![false; positions.len()];
        let mut surface = None;
        for (f, face) in lod.faces.iter().enumerate() {
            let (s, proxy) = sections.of(f as u32);
            if proxy {
                continue;
            }
            surface = surface.or(s);
            for &v in face.indices() {
                if let Some(u) = used.get_mut(v as usize) {
                    *u = true;
                }
            }
        }
        let points: Vec<_> = (0..positions.len() as u32)
            .filter(|&v| used[v as usize])
            .filter_map(point)
            .collect();
        vec![(
            Component {
                name: format!("{}", lod.resolution),
                surface,
            },
            points,
        )]
    } else {
        selections
            .iter()
            .map(|sel| {
                let mut vertices = sel.vertices.clone();
                if vertices.is_empty() {
                    for &f in &sel.faces {
                        if let Some(face) = lod.faces.get(f as usize) {
                            vertices.extend_from_slice(face.indices());
                        }
                    }
                }
                let surface = sel.faces.first().and_then(|&f| sections.of(f).0);
                let points = vertices.iter().filter_map(|&v| point(v)).collect();
                (
                    Component {
                        name: sel.name.clone(),
                        surface,
                    },
                    points,
                )
            })
            .collect()
    };
    LayerShape::convex(parts)
}

fn mesh_layer(lod: &Lod, surfaces: &mut SurfaceBank) -> Option<LayerShape> {
    let sections = SectionSurfaces::new(lod, surfaces);
    let vertices: Vec<DVec3> = lod
        .vertices
        .positions
        .iter()
        .map(|p| p.as_dvec3())
        .collect();
    let mut triangles = Vec::new();
    let mut triangle_surfaces = Vec::new();
    for (f, face) in lod.faces.iter().enumerate() {
        let (surface, proxy) = sections.of(f as u32);
        if proxy {
            continue;
        }
        for t in face.triangles() {
            if t.iter().all(|&v| (v as usize) < vertices.len()) {
                triangles.push(t);
                triangle_surfaces.push(surface);
            }
        }
    }
    LayerShape::mesh(vertices, triangles, triangle_surfaces)
}
