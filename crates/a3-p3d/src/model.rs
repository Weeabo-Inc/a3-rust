//! The decoded model: one shape for both encodings.

use std::ops::Range;

use glam::{Mat3, Vec2, Vec3, Vec4};

use crate::resolution::LodResolution;

/// The encoding a [`Model`] was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Editable Object Builder format.
    Mlod,
    /// Binarised, engine-optimised format that ships in game PBOs.
    Odol,
}

/// A P3D model: model-wide info, skeleton, Model animations and its LODs.
#[derive(Debug, Clone)]
pub struct Model {
    /// Which encoding the file used.
    pub encoding: Encoding,
    /// Format version (ODOL `73` in every shipped 2.22 model; MLOD `257`).
    pub version: u32,
    /// Model-wide info. ODOL stores it; for MLOD only the bounding box is filled.
    pub info: ModelInfo,
    /// The Skeleton, when the model has one (ODOL only).
    pub skeleton: Option<Skeleton>,
    /// Binarised model.cfg animations (ODOL only).
    pub animations: Vec<Animation>,
    /// The LODs in file order.
    pub lods: Vec<Lod>,
}

impl Model {
    /// The first LOD with the given resolution.
    pub fn lod(&self, resolution: LodResolution) -> Option<&Lod> {
        self.lods.iter().find(|lod| lod.resolution == resolution)
    }
}

/// Model-wide data from the ODOL header. Field names follow the community format notes; see
/// `docs/re/p3d-odol.md` for offsets and confidence.
#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    /// Steam app ID of the product that ships the model (`107410` for the base game).
    pub app_id: u32,
    /// Path of the default muzzle-flash proxy model (or a selection name); usually empty.
    pub muzzle_flash: String,
    /// Model-wide special flags.
    pub special_flags: u32,
    /// Radius of the sphere around all visual LODs.
    pub bounding_sphere: f32,
    /// Radius of the sphere around the Geometry LOD.
    pub geometry_sphere: f32,
    /// Unknown; 0 in shipped models.
    pub remarks: u32,
    /// Clip-flag hints common to every LOD.
    pub and_hints: u32,
    /// Clip-flag hints of any LOD.
    pub or_hints: u32,
    /// Aiming center.
    pub aiming_center: Vec3,
    /// Packed ARGB colour.
    pub color: u32,
    /// Packed ARGB colour type.
    pub color_type: u32,
    /// View density (negative for see-through objects).
    pub view_density: f32,
    /// Bounding box over all LODs.
    pub bbox_min: Vec3,
    /// Bounding box over all LODs.
    pub bbox_max: Vec3,
    /// LOD density coefficient (config `lodDensityCoef`).
    pub lod_density_coef: f32,
    /// Draw importance.
    pub draw_importance: f32,
    /// Bounding box over the visual LODs.
    pub bbox_visual_min: Vec3,
    /// Bounding box over the visual LODs.
    pub bbox_visual_max: Vec3,
    /// Center of the bounding sphere.
    pub bounding_center: Vec3,
    /// Center of the Geometry LOD.
    pub geometry_center: Vec3,
    /// Center of mass.
    pub center_of_mass: Vec3,
    /// Inverse inertia tensor.
    pub inv_inertia: Mat3,
    /// Model property `autocenter`.
    pub auto_center: bool,
    /// Unknown lock of `auto_center`.
    pub lock_auto_center: bool,
    /// Model property `canocclude`.
    pub can_occlude: bool,
    /// Model property `canbeoccluded`.
    pub can_be_occluded: bool,
    /// Model property `aicovers`.
    pub ai_covers: bool,
    /// Thermal imaging parameters.
    pub thermal: ThermalParams,
    /// Model property `forcenotalpha`.
    pub force_not_alpha: bool,
    /// Shadow source selection _(uncertain meaning)_.
    pub shadow_source: i32,
    /// Model property `prefershadowvolume`.
    pub prefer_shadow_volume: bool,
    /// Shadow offset (`f32::MAX` when unset).
    pub shadow_offset: f32,
    /// The model is animated.
    pub animated: bool,
    /// Map icon type (`map` property).
    pub map_type: u8,
    /// Per-point mass of the Geometry LOD, when stored (always empty in shipped models).
    pub mass_array: Vec<f32>,
    /// Total mass.
    pub mass: f32,
    /// `1 / mass` (`1e10` for massless models).
    pub inv_mass: f32,
    /// Armor (`armor` property).
    pub armor: f32,
    /// `1 / armor`.
    pub inv_armor: f32,
    /// Explosion shielding (`explosionShielding` property).
    pub explosion_shielding: f32,
    /// Indices of the special LODs.
    pub special_lods: SpecialLods,
    /// Unknown; equals the LOD count in most models _(uncertain)_.
    pub min_shadow: u32,
    /// Model property `canblend`.
    pub can_blend: bool,
    /// Model property `class`.
    pub class: String,
    /// Model property `damage`.
    pub damage: String,
    /// Model property `frequent`.
    pub frequent: bool,
    /// Unknown; 0 in shipped models.
    pub unknown: u32,
    /// Per LOD: index of the preferred shadow-volume LOD, or -1.
    pub preferred_shadow_volume_lod: Vec<i32>,
    /// Per LOD: index of the preferred shadow-buffer LOD, or -1.
    pub preferred_shadow_buffer_lod: Vec<i32>,
    /// Per LOD: index of the preferred visible shadow-buffer LOD, or -1.
    pub preferred_shadow_buffer_lod_visible: Vec<i32>,
}

/// Thermal imaging parameters of a model _(names from community notes; meaning uncertain)_.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThermalParams {
    /// Minimum half-cooling time.
    pub ht_min: f32,
    /// Maximum half-cooling time.
    pub ht_max: f32,
    /// Maximum alive factor.
    pub af_max: f32,
    /// Maximum movement factor.
    pub mf_max: f32,
    /// Metabolism factor.
    pub m_fact: f32,
    /// Body temperature.
    pub t_body: f32,
}

/// Indices into [`Model::lods`] of the special LODs; `None` when the model has none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpecialLods {
    /// Memory LOD.
    pub memory: Option<u8>,
    /// Geometry LOD.
    pub geometry: Option<u8>,
    /// Simplified geometry _(uncertain)_.
    pub geometry_simple: Option<u8>,
    /// PhysX geometry LOD.
    pub geometry_physx: Option<u8>,
    /// Fire Geometry LOD (falls back to View Geometry).
    pub fire_geometry: Option<u8>,
    /// View Geometry LOD.
    pub view_geometry: Option<u8>,
    /// Pilot view geometry.
    pub view_pilot_geometry: Option<u8>,
    /// Gunner view geometry.
    pub view_gunner_geometry: Option<u8>,
    /// Commander view geometry (always `None` in shipped models).
    pub view_commander_geometry: Option<u8>,
    /// Cargo view geometry.
    pub view_cargo_geometry: Option<u8>,
    /// Land Contact LOD.
    pub land_contact: Option<u8>,
    /// Roadway LOD.
    pub roadway: Option<u8>,
    /// Paths LOD.
    pub paths: Option<u8>,
    /// Hit-points LOD.
    pub hitpoints: Option<u8>,
}

/// The Skeleton: ordered bones with parents.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skeleton {
    /// CfgSkeletons class name.
    pub name: String,
    /// `true` when the skeleton inherits from another CfgSkeletons class _(uncertain)_.
    pub inherited: bool,
    /// Bones in skeleton order.
    pub bones: Vec<Bone>,
    /// Name of the pivots model (`pivotsModel`), usually empty.
    pub pivots_model: String,
}

/// One bone of a [`Skeleton`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bone {
    /// Bone name (a named selection in each LOD).
    pub name: String,
    /// Index of the parent bone in [`Skeleton::bones`]; `None` for a root bone or when
    /// `parent_name` names no bone of the skeleton (some shipped models do that).
    pub parent: Option<usize>,
    /// The parent name as stored (empty for a root bone).
    pub parent_name: String,
}

/// One binarised Model animation (a model.cfg `class Animations` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    /// Class name.
    pub name: String,
    /// Animation source.
    pub source: String,
    /// The transform it applies.
    pub transform: AnimationTransform,
    /// `minValue`.
    pub min_value: f32,
    /// `maxValue`.
    pub max_value: f32,
    /// `minPhase`.
    pub min_phase: f32,
    /// `maxPhase`.
    pub max_phase: f32,
    /// `animPeriod`.
    pub anim_period: f32,
    /// `initPhase`.
    pub init_phase: f32,
    /// `sourceAddress`.
    pub source_address: SourceAddress,
    /// Per LOD (indexed like [`Model::lods`]): the bone it moves there, if any.
    pub bindings: Vec<Option<AnimationBinding>>,
}

/// How an animation source value outside `min..max` is mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAddress {
    /// Clamp to the range.
    Clamp,
    /// Mirror back and forth.
    Mirror,
    /// Wrap around.
    Loop,
    /// A value this crate does not know.
    Other(u32),
}

impl From<u32> for SourceAddress {
    fn from(v: u32) -> Self {
        match v {
            0 => Self::Clamp,
            1 => Self::Mirror,
            2 => Self::Loop,
            v => Self::Other(v),
        }
    }
}

/// The axis an animation turns about or moves along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationAxis {
    /// Taken from the model's memory points (`axis`, `begin`/`end`).
    Custom,
    /// The model X axis.
    X,
    /// The model Y axis.
    Y,
    /// The model Z axis.
    Z,
}

/// The transform of a Model animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationTransform {
    /// `rotation`, `rotationX/Y/Z`: angles in radians.
    Rotation {
        /// Axis.
        axis: AnimationAxis,
        /// Angle at `minValue`.
        angle0: f32,
        /// Angle at `maxValue`.
        angle1: f32,
    },
    /// `translation`, `translationX/Y/Z`.
    Translation {
        /// Axis.
        axis: AnimationAxis,
        /// Offset at `minValue`.
        offset0: f32,
        /// Offset at `maxValue`.
        offset1: f32,
    },
    /// `direct`: rotation about an explicit axis plus an offset along it.
    Direct {
        /// A point on the axis.
        axis_pos: Vec3,
        /// Axis direction.
        axis_dir: Vec3,
        /// Angle in radians.
        angle: f32,
        /// Offset along the axis.
        axis_offset: f32,
    },
    /// `hide`.
    Hide {
        /// `hideValue`.
        hide_value: f32,
        /// `unHideValue`.
        unhide_value: f32,
    },
}

/// Where an [`Animation`] acts in one LOD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationBinding {
    /// Index into [`Skeleton::bones`].
    pub bone: u32,
    /// Axis point and direction resolved from the memory LOD; `None` for `direct` and `hide`.
    pub axis: Option<(Vec3, Vec3)>,
}

/// One LOD of a model.
#[derive(Debug, Clone, Default)]
pub struct Lod {
    /// The LOD's resolution value.
    pub resolution: LodResolution,
    /// Vertex attributes, one entry per render vertex.
    pub vertices: Vertices,
    /// Polygons (triangles or quads) indexing [`Lod::vertices`].
    pub faces: Vec<Face>,
    /// Runs of faces sharing texture, material and flags, in face order.
    pub sections: Vec<Section>,
    /// Texture paths referenced by sections.
    pub textures: Vec<String>,
    /// Materials referenced by sections.
    pub materials: Vec<Material>,
    /// Named selections.
    pub named_selections: Vec<NamedSelection>,
    /// Named properties (`key`, `value`).
    pub properties: Vec<(String, String)>,
    /// Proxies.
    pub proxies: Vec<Proxy>,
    /// Animation frames (vertex positions per time).
    pub frames: Vec<Frame>,
    /// Per bone of the Skeleton: indices into [`Model::animations`] acting on it in this LOD.
    pub bone_animations: Vec<Vec<u32>>,
    /// MLOD only: the source point of each vertex (empty when vertices are points).
    pub vertex_to_point: Vec<u32>,
    /// MLOD only: per-point mass (`#Mass#` tag).
    pub point_masses: Vec<f32>,
    /// MLOD only: sharp edges as point index pairs (`#SharpEdges#` tag).
    pub sharp_edges: Vec<[u32; 2]>,
    /// ODOL only: LOD-level fields without a render meaning.
    pub odol: Option<OdolLod>,
}

impl Lod {
    /// Triangle-list indices of every face, in face order (quads split as in [`Face::triangles`]).
    pub fn triangles(&self) -> Vec<u32> {
        self.faces
            .iter()
            .flat_map(Face::triangles)
            .flatten()
            .collect()
    }

    /// Triangle-list indices of the faces of one section.
    pub fn section_triangles(&self, section: &Section) -> Vec<u32> {
        let range = section.faces.start as usize..section.faces.end as usize;
        self.faces[range]
            .iter()
            .flat_map(Face::triangles)
            .flatten()
            .collect()
    }
}

/// Per-vertex attributes. Every non-empty array has one entry per vertex.
#[derive(Debug, Clone, Default)]
pub struct Vertices {
    /// Positions in model space.
    pub positions: Vec<Vec3>,
    /// Normals (may be empty in non-visual LODs).
    pub normals: Vec<Vec3>,
    /// UV sets; set 0 is the main texture mapping.
    pub uv_sets: Vec<Vec<Vec2>>,
    /// Tangent-space S and T vectors (ODOL; empty when the LOD has none).
    pub tangents: Vec<[Vec3; 2]>,
    /// Point flags (ODOL clip flags; MLOD point flags).
    pub flags: Vec<u32>,
    /// Bone weights (ODOL; empty for unskinned LODs).
    pub bone_weights: Vec<BoneWeights>,
}

impl Vertices {
    /// Number of vertices.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// `true` when there are no vertices.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// Up to four bone influences of one vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoneWeights {
    /// Number of used entries in `pairs`.
    pub count: u32,
    /// `(bone, weight)`: bone index in the LOD's sub-skeleton, weight `0..=255`.
    pub pairs: [(u8, u8); 4],
}

/// A triangle or quad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    indices: [u32; 4],
    len: u8,
}

impl Face {
    /// A triangle.
    pub fn triangle(a: u32, b: u32, c: u32) -> Self {
        Self {
            indices: [a, b, c, 0],
            len: 3,
        }
    }

    /// A quad.
    pub fn quad(a: u32, b: u32, c: u32, d: u32) -> Self {
        Self {
            indices: [a, b, c, d],
            len: 4,
        }
    }

    /// Vertex indices in file order (3 or 4).
    pub fn indices(&self) -> &[u32] {
        &self.indices[..usize::from(self.len)]
    }

    /// The face split into triangles: `(0,1,2)` and, for a quad, `(0,2,3)`.
    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        let i = self.indices;
        let quad = (self.len == 4).then_some([i[0], i[2], i[3]]);
        std::iter::once([i[0], i[1], i[2]]).chain(quad)
    }
}

/// A run of faces drawn with one texture and material.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    /// Face index range into [`Lod::faces`].
    pub faces: Range<u32>,
    /// Index into [`Lod::textures`].
    pub texture: Option<u32>,
    /// Index into [`Lod::materials`].
    pub material: Option<u32>,
    /// Face flags common to the section.
    pub flags: u32,
    /// ODOL only: fields without a render meaning.
    pub odol: Option<OdolSection>,
}

impl Section {
    /// Face flag bit set on the sections holding proxy triangles.
    pub const PROXY_FLAG: u32 = 0x1000_0000;

    /// `true` for a section of proxy triangles: placeholders the renderer does not draw. In the
    /// shipped models this bit is set exactly on the sections that proxies reference.
    pub fn is_proxy(&self) -> bool {
        self.flags & Self::PROXY_FLAG != 0
    }
}

/// A material: an embedded copy of an rvmat (ODOL) or just its path (MLOD).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Material {
    /// The rvmat path.
    pub name: String,
    /// Embedded material version (ODOL; `11` in shipped models).
    pub version: u32,
    /// Emissive colour (RGBA).
    pub emissive: Vec4,
    /// Ambient colour.
    pub ambient: Vec4,
    /// Diffuse colour.
    pub diffuse: Vec4,
    /// Forced diffuse colour.
    pub forced_diffuse: Vec4,
    /// Specular colour.
    pub specular: Vec4,
    /// Second specular colour _(uncertain)_.
    pub specular2: Vec4,
    /// Specular power.
    pub specular_power: f32,
    /// Pixel shader ID.
    pub pixel_shader: u32,
    /// Vertex shader ID.
    pub vertex_shader: u32,
    /// Main light mode.
    pub main_light: u32,
    /// Fog mode.
    pub fog_mode: u32,
    /// Surface (`.bisurf`) path.
    pub surface: String,
    /// Render flags.
    pub render_flags: u32,
    /// Texture stages.
    pub stages: Vec<MaterialStage>,
    /// UV generators referenced by the stages.
    pub tex_gens: Vec<TexGen>,
    /// The extra "TI" (thermal imaging) stage.
    pub ti_stage: Option<MaterialStage>,
}

/// A texture stage of a [`Material`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialStage {
    /// Texture filter mode.
    pub filter: u32,
    /// Texture path or procedural texture.
    pub texture: String,
    /// Index into [`Material::tex_gens`] _(uncertain)_.
    pub tex_gen: u32,
    /// Use the world environment map.
    pub use_world_env_map: bool,
}

/// A UV generator: source UV set and a 4x3 transform.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TexGen {
    /// UV source (0 none, 1 tex, 2 tex1, ...; see rvmat `uvSource`).
    pub uv_source: u32,
    /// Transform rows: `aside`, `up`, `dir`, `pos`.
    pub transform: [[f32; 3]; 4],
}

/// A named set of faces and vertices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NamedSelection {
    /// Name (bone names, `damage`, `zbytek`, proxies, hit points, ...).
    pub name: String,
    /// Selected face indices.
    pub faces: Vec<u32>,
    /// Selected vertex indices.
    pub vertices: Vec<u32>,
    /// Per selected vertex weight byte (empty when all are fully selected).
    pub weights: Vec<u8>,
    /// ODOL: the selection is drawn as whole sections.
    pub sectional: bool,
    /// ODOL: indices of the sections the selection covers.
    pub sections: Vec<u32>,
}

/// A reference to another model placed in the LOD.
#[derive(Debug, Clone, PartialEq)]
pub struct Proxy {
    /// Path of the referenced model.
    pub model: String,
    /// Orientation.
    pub orientation: Mat3,
    /// Position.
    pub position: Vec3,
    /// Proxy number (the `.NNN` suffix of the selection name).
    pub sequence_id: i32,
    /// Index of the named selection holding the proxy triangle.
    pub named_selection: i32,
    /// Index of the bone the proxy follows, or -1.
    pub bone: i32,
    /// Index of the section of the proxy triangle, or -1.
    pub section: i32,
}

/// One animation frame: vertex positions at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// Time.
    pub time: f32,
    /// Positions, one per vertex.
    pub positions: Vec<Vec3>,
}

/// ODOL LOD fields without a render meaning.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OdolLod {
    /// The engine may unload the LOD when `false`.
    pub permanent: bool,
    /// Counts the engine reads before loading a non-permanent LOD.
    pub summary: Option<LodSummary>,
    /// Per LOD bone (the index used by [`BoneWeights`]): its index in [`Skeleton::bones`].
    pub sub_skeleton: Vec<u32>,
    /// Per skeleton bone: the LOD bones it maps to.
    pub skeleton_to_sub_skeleton: Vec<Vec<u32>>,
    /// Total face area.
    pub face_area: f32,
    /// Clip-flag hints of any vertex.
    pub or_hints: u32,
    /// Clip-flag hints common to all vertices.
    pub and_hints: u32,
    /// Bounding box minimum.
    pub bbox_min: Vec3,
    /// Bounding box maximum.
    pub bbox_max: Vec3,
    /// Bounding sphere center.
    pub bbox_center: Vec3,
    /// Bounding sphere radius.
    pub bbox_radius: f32,
    /// Point index to vertex index map (empty in shipped models).
    pub point_to_vertex: Vec<u32>,
    /// Packed ARGB icon colour.
    pub icon_color: u32,
    /// Packed ARGB selection colour.
    pub selected_color: u32,
    /// LOD special flags.
    pub special: u32,
    /// Every vertex has a single bone with full weight _(uncertain)_.
    pub vertex_bone_ref_is_simple: bool,
    /// Per vertex: the two neighbour vertices and their bone weights (empty for most LODs).
    pub neighbor_bones: Vec<NeighborBones>,
    /// Unknown `u32` at the end of the vertex block; 0 in shipped models.
    pub unknown_u32: u32,
    /// Unknown trailing byte; 1 in nearly all shipped LODs.
    pub unknown_u8: u8,
}

/// The neighbour-vertex record of a skinned vertex _(meaning uncertain)_.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeighborBones {
    /// First neighbour vertex.
    pub pos_a: u16,
    /// Its bone weights.
    pub weights_a: BoneWeights,
    /// Second neighbour vertex.
    pub pos_b: u16,
    /// Its bone weights.
    pub weights_b: BoneWeights,
}

/// The header-side summary of a non-permanent ODOL LOD.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LodSummary {
    /// Face count.
    pub faces: u32,
    /// Packed ARGB colour.
    pub color: u32,
    /// Special flags.
    pub special: u32,
    /// Clip-flag hints.
    pub or_hints: u32,
    /// The LOD is skinned.
    pub has_skeleton: bool,
    /// Vertex count.
    pub vertices: u32,
    /// Total face area.
    pub face_area: f32,
}

/// ODOL section fields without a render meaning.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OdolSection {
    /// First sub-skeleton bone used.
    pub min_bone: u32,
    /// Number of sub-skeleton bones used.
    pub bone_count: u32,
    /// Material path stored inline when the material index is -1 (empty in shipped models).
    pub material_name: String,
    /// Area over texture, per stage _(uncertain)_.
    pub area_over_tex: Vec<f32>,
    /// Eleven unknown floats present in a few sections.
    pub unknown: Option<[f32; 11]>,
}
