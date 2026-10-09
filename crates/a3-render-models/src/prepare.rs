//! Turning a decoded P3D into render-ready CPU data: one vertex/index buffer per Resolution
//! LOD, index ranges per material, and the LOD's proxies.

use std::ops::Range;

use a3_p3d::{Lod, Model, Skeleton};
use bytemuck::{Pod, Zeroable};
use glam::{Affine3A, Vec2, Vec3};

use crate::lod::LodMetrics;
use crate::material::{MaterialDesc, Slot};
use crate::shader::ShaderFamily;
use crate::skin::SkinData;

/// One model vertex as the model shaders read it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    /// ODOL's tangent S (which points along -dP/du) in xyz; w = +-1 so that
    /// `cross(normal, S) * w` is ODOL's T (along -dP/dv).
    pub tangent: [f32; 4],
}

/// A run of indices drawn with one material.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSection {
    pub indices: Range<u32>,
    pub material: MaterialDesc,
}

/// Another model placed in a LOD.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyRef {
    /// VFS path of the model, with `.p3d`.
    pub model: String,
    /// Proxy space to model space.
    pub transform: Affine3A,
}

/// One Resolution LOD, ready for upload.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedLod {
    pub resolution: f32,
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
    pub sections: Vec<PreparedSection>,
    pub proxies: Vec<ProxyRef>,
    /// The LOD's vertices in Skeleton bones, when the model is skinned.
    pub skin: Option<SkinData>,
    /// Distance of the farthest vertex from the model origin.
    pub radius: f32,
    /// Triangle count.
    pub faces: u32,
    /// Face count as the engine counts it for LOD selection: polygons (a quad is one face),
    /// proxy faces included.
    pub polygons: u32,
}

/// A model's Resolution LODs, most detailed first.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedModel {
    pub lods: Vec<PreparedLod>,
    /// Config `lodDensityCoef` from the model info (1 when unset).
    pub lod_density_coef: f32,
    /// Config `drawImportance` from the model info (1 when unset).
    pub draw_importance: f32,
    /// Bounding radius around the model origin over all Resolution LODs.
    pub radius: f32,
    /// Bounding box (min, max) of the drawn geometry of all Resolution LODs.
    pub bbox: (Vec3, Vec3),
}

impl PreparedModel {
    pub fn new(model: &Model) -> PreparedModel {
        let mut lods: Vec<PreparedLod> = model
            .lods
            .iter()
            .filter(|l| l.resolution.is_visual())
            .map(|l| prepare_lod(l, model.skeleton.as_ref()))
            .collect();
        lods.sort_by(|a, b| a.resolution.total_cmp(&b.resolution));
        let radius = lods.iter().map(|l| l.radius).fold(0.0, f32::max);
        let mut bbox = (Vec3::MAX, Vec3::MIN);
        for lod in &lods {
            for &i in &lod.indices {
                let p = Vec3::from(lod.vertices[i as usize].position);
                bbox = (bbox.0.min(p), bbox.1.max(p));
            }
        }
        if bbox.0.x > bbox.1.x {
            bbox = (Vec3::ZERO, Vec3::ZERO);
        }
        // The engine clamps both to 0.001..=10000; 0 (never written) means unset.
        let coef = |v: f32| {
            if v > 0.0 {
                v.clamp(0.001, 10_000.0)
            } else {
                1.0
            }
        };
        PreparedModel {
            lods,
            lod_density_coef: coef(model.info.lod_density_coef),
            draw_importance: coef(model.info.draw_importance),
            radius,
            bbox,
        }
    }

    /// What LOD selection needs, per LOD.
    pub fn lod_metrics(&self) -> Vec<LodMetrics> {
        self.lods
            .iter()
            .map(|l| LodMetrics {
                resolution: l.resolution,
                faces: l.polygons,
            })
            .collect()
    }
}

fn prepare_lod(lod: &Lod, skeleton: Option<&Skeleton>) -> PreparedLod {
    let v = &lod.vertices;
    let n = v.len();
    let uv = |set: usize, i: usize| {
        v.uv_sets
            .get(set)
            .and_then(|s| s.get(i))
            .copied()
            .unwrap_or(Vec2::ZERO)
    };
    let vertices: Vec<ModelVertex> = (0..n)
        .map(|i| {
            let normal = v.normals.get(i).copied().unwrap_or(Vec3::Y);
            let (s, t) = v
                .tangents
                .get(i)
                .map_or((Vec3::X, Vec3::Z), |[s, t]| (*s, *t));
            let w = if normal.cross(s).dot(t) < 0.0 {
                -1.0
            } else {
                1.0
            };
            ModelVertex {
                position: v.positions[i].to_array(),
                normal: normal.to_array(),
                uv0: uv(0, i).to_array(),
                uv1: uv(1, i).to_array(),
                tangent: s.extend(w).to_array(),
            }
        })
        .collect();

    // Group the sections' triangles by material, keeping first-appearance order.
    let mut groups: Vec<(MaterialDesc, Vec<u32>)> = Vec::new();
    for section in &lod.sections {
        // Proxy triangles are placeholders for the proxies of the LOD, not geometry.
        if section.is_proxy() {
            continue;
        }
        let texture = section
            .texture
            .and_then(|t| lod.textures.get(t as usize))
            .map(String::as_str);
        let material = section.material.and_then(|m| lod.materials.get(m as usize));
        let desc = MaterialDesc::new(material, texture);
        if desc.family == ShaderFamily::Unsupported || desc.texture(Slot::Diffuse).is_none() {
            continue;
        }
        let triangles = lod.section_triangles(section);
        let triangles = triangles.into_iter().filter(|&i| (i as usize) < n);
        match groups.iter_mut().find(|(d, _)| *d == desc) {
            Some((_, indices)) => indices.extend(triangles),
            None => groups.push((desc, triangles.collect())),
        }
    }
    let mut indices = Vec::new();
    let mut sections = Vec::new();
    for (material, group) in groups {
        let start = indices.len() as u32;
        indices.extend(group);
        // Drop incomplete triangles left by out-of-range indices.
        indices.truncate(start as usize + (indices.len() - start as usize) / 3 * 3);
        let end = indices.len() as u32;
        if end > start {
            sections.push(PreparedSection {
                indices: start..end,
                material,
            });
        }
    }

    let proxies = lod
        .proxies
        .iter()
        .map(|p| ProxyRef {
            model: proxy_model_path(&p.model),
            transform: Affine3A::from_mat3_translation(p.orientation, p.position),
        })
        .collect();
    // Only drawn vertices count: LODs keep stray points (memory-like helpers) that are not
    // part of any face.
    let radius = indices
        .iter()
        .map(|&i| v.positions[i as usize].length())
        .fold(0.0, f32::max);
    PreparedLod {
        resolution: lod.resolution.0,
        faces: indices.len() as u32 / 3,
        polygons: lod.faces.len() as u32,
        vertices,
        indices,
        sections,
        proxies,
        skin: skeleton.and_then(|s| SkinData::new(lod, s)),
        radius,
    }
}

/// `\a3\data_f\proxy` -> `a3\data_f\proxy.p3d`.
fn proxy_model_path(model: &str) -> String {
    let trimmed = model.trim_start_matches(['\\', '/']);
    if trimmed.to_ascii_lowercase().ends_with(".p3d") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}.p3d")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{BoneWeights, OdolLod};
    use a3_p3d::{Face, Lod, LodResolution, Material, Model, Proxy, Section, Vertices};
    use glam::{Mat3, Vec2, Vec3};

    /// A unit quad in the XY plane facing -Z, split into two sections with different textures,
    /// plus a third face in the first texture again.
    fn quad_lod(resolution: f32) -> Lod {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let n = positions.len();
        Lod {
            resolution: LodResolution(resolution),
            vertices: Vertices {
                positions,
                normals: vec![Vec3::NEG_Z; n],
                uv_sets: vec![vec![Vec2::ZERO; n], vec![Vec2::ONE; n]],
                tangents: vec![[Vec3::X, Vec3::Y]; n],
                ..Vertices::default()
            },
            faces: vec![
                Face::triangle(0, 1, 2),
                Face::triangle(0, 2, 3),
                Face::quad(0, 1, 2, 4),
            ],
            sections: vec![
                Section {
                    faces: 0..1,
                    texture: Some(0),
                    material: Some(0),
                    ..Section::default()
                },
                Section {
                    faces: 1..2,
                    texture: Some(1),
                    material: Some(0),
                    ..Section::default()
                },
                Section {
                    faces: 2..3,
                    texture: Some(0),
                    material: Some(0),
                    ..Section::default()
                },
            ],
            textures: vec!["a_co.paa".into(), "b_co.paa".into()],
            materials: vec![Material {
                name: "m.rvmat".into(),
                pixel_shader: 102,
                ..Material::default()
            }],
            proxies: vec![Proxy {
                model: r"\a3\structures_f\data\window".into(),
                orientation: Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2),
                position: Vec3::new(1.0, 2.0, 3.0),
                sequence_id: 1,
                named_selection: 0,
                bone: -1,
                section: -1,
            }],
            ..Lod::default()
        }
    }

    fn model() -> Model {
        let mut model = empty_model();
        model.lods = vec![
            quad_lod(2.0),
            Lod {
                resolution: LodResolution(1e15),
                ..Lod::default()
            },
            quad_lod(1.0),
        ];
        model.info.lod_density_coef = 1.5;
        model.info.draw_importance = 0.5;
        model
    }

    fn empty_model() -> Model {
        Model {
            encoding: a3_p3d::Encoding::Odol,
            version: 73,
            info: Default::default(),
            skeleton: None,
            animations: Vec::new(),
            lods: Vec::new(),
        }
    }

    #[test]
    fn only_resolution_lods_are_kept_most_detailed_first() {
        let p = PreparedModel::new(&model());
        let resolutions: Vec<f32> = p.lods.iter().map(|l| l.resolution).collect();
        assert_eq!(resolutions, vec![1.0, 2.0]);
        assert_eq!(p.lod_density_coef, 1.5);
        assert_eq!(p.draw_importance, 0.5);
        // LOD selection counts faces as the engine does: two triangles and a quad are three.
        assert_eq!(p.lods[0].faces, 4);
        assert_eq!(p.lod_metrics()[0].faces, 3);
    }

    #[test]
    fn sections_with_the_same_material_share_one_index_range() {
        let p = PreparedModel::new(&model());
        let lod = &p.lods[0];
        assert_eq!(lod.sections.len(), 2);
        let first = &lod.sections[0];
        assert_eq!(
            first.material.texture(Slot::Diffuse).unwrap().path,
            "a_co.paa"
        );
        // Triangle (0,1,2) and the quad split into (0,1,2), (0,2,4): 9 indices.
        assert_eq!(first.indices.len(), 9);
        let r = first.indices.start as usize..first.indices.end as usize;
        assert_eq!(&lod.indices[r], &[0, 1, 2, 0, 1, 2, 0, 2, 4]);
        assert_eq!(lod.sections[1].indices.len(), 3);
    }

    #[test]
    fn vertices_carry_both_uv_sets_and_tangent_handedness() {
        let p = PreparedModel::new(&model());
        let v = &p.lods[0].vertices[0];
        assert_eq!(v.uv0, [0.0, 0.0]);
        assert_eq!(v.uv1, [1.0, 1.0]);
        // cross(N, S) = cross(-Z, X) = -Y, opposite to T = +Y: handedness -1.
        assert_eq!(v.tangent, [1.0, 0.0, 0.0, -1.0]);
        assert!((p.lods[0].radius - Vec3::new(1.0, 1.0, 0.0).length()).abs() < 1e-5);
    }

    #[test]
    fn proxies_keep_their_model_and_placement() {
        let p = PreparedModel::new(&model());
        let proxy = &p.lods[0].proxies[0];
        assert_eq!(proxy.model, r"a3\structures_f\data\window.p3d");
        assert_eq!(proxy.transform.translation, Vec3::new(1.0, 2.0, 3.0).into());
    }

    #[test]
    fn proxy_sections_are_not_drawn() {
        let mut lod = quad_lod(1.0);
        // The third section's faces are proxy triangles, placeholders for proxies.
        lod.sections[2].flags = Section::PROXY_FLAG;
        let mut model = empty_model();
        model.lods = vec![lod];
        let p = PreparedModel::new(&model);
        let lod = &p.lods[0];
        assert_eq!(lod.sections.len(), 2);
        // The triangle (0,1,2) and the triangle (0,2,3); the proxy quad's six indices are gone.
        assert_eq!(lod.indices.len(), 6);
        assert_eq!(lod.faces, 2);
    }

    /// A skinned LOD: two vertices, bound to LOD bones 0 and 1, of a two-bone skeleton that
    /// swaps them (LOD bone 0 is Skeleton bone 1).
    fn skinned_lod() -> Lod {
        let mut lod = quad_lod(1.0);
        lod.vertices.bone_weights = vec![
            BoneWeights {
                count: 1,
                pairs: [(0, 255), (0, 0), (0, 0), (0, 0)],
            },
            BoneWeights {
                count: 2,
                pairs: [(1, 128), (0, 127), (0, 0), (0, 0)],
            },
            BoneWeights {
                count: 0,
                pairs: [(0, 0); 4],
            },
        ];
        lod.odol = Some(OdolLod {
            sub_skeleton: vec![1, 0],
            ..OdolLod::default()
        });
        lod
    }

    fn skeleton() -> a3_p3d::Skeleton {
        a3_p3d::Skeleton {
            name: "test".into(),
            bones: vec![
                a3_p3d::Bone {
                    name: "root".into(),
                    ..a3_p3d::Bone::default()
                },
                a3_p3d::Bone {
                    name: "head".into(),
                    ..a3_p3d::Bone::default()
                },
            ],
            ..a3_p3d::Skeleton::default()
        }
    }

    #[test]
    fn a_lod_of_a_skinned_model_carries_its_bone_influences() {
        let mut model = empty_model();
        model.skeleton = Some(skeleton());
        model.lods = vec![skinned_lod()];
        let p = PreparedModel::new(&model);
        let skin = p.lods[0].skin.as_ref().expect("the LOD is skinned");
        // Two Skeleton bones plus the trailing identity slot.
        assert_eq!(skin.palette_len, 3);
        assert_eq!(
            skin.vertices[0].bones[0], 1,
            "LOD bone 0 is Skeleton bone 1"
        );
        assert_eq!(skin.vertices[0].weights[0], 1.0);
        assert_eq!(skin.vertices[1].bones[0], 0);
        assert_eq!(skin.vertices[1].bones[1], 1);
        assert!((skin.vertices[1].weights[0] + skin.vertices[1].weights[1] - 1.0).abs() < 1e-6);
        // No influence: the rest pose through the identity slot.
        assert_eq!(skin.vertices[2].bones, [2; 4]);
    }

    #[test]
    fn a_lod_of_an_unskinned_model_has_no_bone_influences() {
        let mut model = empty_model();
        model.skeleton = Some(skeleton());
        model.lods = vec![quad_lod(1.0)];
        let p = PreparedModel::new(&model);
        assert_eq!(p.lods[0].skin, None);
        let mut model = empty_model();
        model.lods = vec![skinned_lod()];
        let p = PreparedModel::new(&model);
        assert_eq!(p.lods[0].skin, None, "no skeleton, no skinning");
    }
}
