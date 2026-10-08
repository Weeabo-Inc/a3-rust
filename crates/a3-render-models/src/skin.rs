//! Skinned vertices: an ODOL LOD's per-vertex bone influences, resolved to Skeleton bones.
//!
//! ODOL stores up to four `(bone, weight)` pairs per vertex, the bone indices into the LOD's
//! sub-skeleton and the weights as bytes summing to 255 (the `BoneWeights` array of the LOD).
//! The two vertex-bone-ref paths the ODOL header distinguishes (`vertex_bone_ref_is_simple`)
//! carry the same `BoneWeights` array, so both decode identically.
//!
//! The shader blends a vertex's bones with palette matrices indexed by *Skeleton* bone, so
//! [`SkinData`] maps every LOD bone through the LOD's `sub_skeleton` table at prepare time.
//! Vertices with no usable influence are bound to a trailing identity slot of the palette,
//! which leaves them in the rest pose.

use a3_p3d::{Lod, Skeleton};
use bytemuck::{Pod, Zeroable};

/// One vertex's bone influences, as the model shaders read them.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SkinVertex {
    /// Palette slots, indexed by Skeleton bone; unused entries are the identity slot.
    pub bones: [u32; 4],
    /// Blending weights, summing to 1.
    pub weights: [f32; 4],
}

/// A LOD's skinning: one [`SkinVertex`] per vertex plus the palette layout it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinData {
    /// One per vertex of the LOD, in vertex order.
    pub vertices: Vec<SkinVertex>,
    /// Palette slots the LOD indexes: one per Skeleton bone, plus the trailing identity slot
    /// that vertices without influences are bound to. Palette slot 0 is bone 0.
    pub palette_len: u32,
}

impl SkinData {
    /// The skinning of `lod` against `skeleton`, or `None` when the LOD has no bone weights
    /// (an unskinned model part) or the skeleton has no bones.
    pub fn new(lod: &Lod, skeleton: &Skeleton) -> Option<SkinData> {
        let weights = &lod.vertices.bone_weights;
        let bone_count = skeleton.bones.len();
        if weights.is_empty() || bone_count == 0 {
            return None;
        }
        // The identity slot: one past the last Skeleton bone.
        let identity = bone_count as u32;
        let rest = || SkinVertex {
            bones: [identity; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        };
        let table = lod
            .odol
            .as_ref()
            .map_or(&[][..], |o| o.sub_skeleton.as_slice());
        let mut vertices: Vec<SkinVertex> = weights
            .iter()
            .take(lod.vertices.len())
            .map(|w| {
                let mut vertex = rest();
                let count = (w.count.min(4)) as usize;
                let mut sum = 0.0f32;
                for (k, &(bone, weight)) in w.pairs[..count].iter().enumerate() {
                    vertex.bones[k] = resolve(table, bone, bone_count, identity);
                    vertex.weights[k] = f32::from(weight);
                    sum += f32::from(weight);
                }
                if sum <= 0.0 {
                    return rest();
                }
                for w in &mut vertex.weights {
                    *w /= sum;
                }
                vertex
            })
            .collect();
        // Vertices past the end of the bone-weight array stay in the rest pose.
        vertices.resize(lod.vertices.len(), rest());
        Some(SkinData {
            vertices,
            palette_len: identity + 1,
        })
    }
}

/// The palette slot of a vertex's LOD bone: its Skeleton bone, or the identity slot when the
/// LOD's table does not map it (a corrupt or truncated sub-skeleton). An empty table (LODs
/// whose ODOL stores none) means LOD bones are Skeleton bones.
fn resolve(table: &[u32], lod_bone: u8, bone_count: usize, identity: u32) -> u32 {
    match table.get(usize::from(lod_bone)) {
        Some(&bone) if (bone as usize) < bone_count => bone,
        Some(_) => identity,
        None if table.is_empty() => {
            if usize::from(lod_bone) < bone_count {
                u32::from(lod_bone)
            } else {
                identity
            }
        }
        None => identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{Bone, BoneWeights, Lod, OdolLod, Vertices};
    use glam::Vec3;

    /// A skeleton of `bones` bones; its identity slot is palette slot `bones`.
    fn skeleton(bones: usize) -> Skeleton {
        Skeleton {
            name: "test".into(),
            bones: (0..bones)
                .map(|i| Bone {
                    name: format!("bone{i}"),
                    ..Bone::default()
                })
                .collect(),
            ..Skeleton::default()
        }
    }

    fn weights(count: u32, pairs: &[(u8, u8)]) -> BoneWeights {
        let mut out = [(0, 0); 4];
        out[..pairs.len()].copy_from_slice(pairs);
        BoneWeights { count, pairs: out }
    }

    /// A LOD of `vertices` vertices with bone weights for the first ones and the given LOD
    /// bone -> Skeleton bone table.
    fn test_lod(vertices: usize, weights: Vec<BoneWeights>, sub_skeleton: Vec<u32>) -> Lod {
        Lod {
            vertices: Vertices {
                positions: vec![Vec3::ZERO; vertices],
                bone_weights: weights,
                ..Vertices::default()
            },
            odol: Some(OdolLod {
                sub_skeleton,
                ..OdolLod::default()
            }),
            ..Lod::default()
        }
    }

    #[test]
    fn a_single_influence_takes_its_skeleton_bone_at_full_weight() {
        // LOD bone 2 is Skeleton bone 9.
        let lod = test_lod(1, vec![weights(1, &[(2, 255)])], vec![7, 3, 9]);
        let skin = SkinData::new(&lod, &skeleton(10)).unwrap();
        assert_eq!(skin.vertices[0].bones, [9, 10, 10, 10]);
        assert_eq!(skin.vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn lod_bones_map_through_the_sub_skeleton_table() {
        let lod = test_lod(1, vec![weights(1, &[(1, 255)])], vec![7, 3, 9]);
        let skin = SkinData::new(&lod, &skeleton(10)).unwrap();
        assert_eq!(
            skin.vertices[0].bones[0], 3,
            "LOD bone 1 is Skeleton bone 3"
        );
    }

    #[test]
    fn four_influences_blend_and_normalise_to_one() {
        let lod = test_lod(
            1,
            vec![weights(4, &[(0, 51), (1, 51), (2, 51), (3, 102)])],
            vec![0, 1, 2, 3],
        );
        let skin = SkinData::new(&lod, &skeleton(4)).unwrap();
        let v = skin.vertices[0];
        assert_eq!(v.bones, [0, 1, 2, 3]);
        assert!((v.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!((v.weights[3] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn weights_are_divided_by_their_sum_not_by_255() {
        // A vertex whose weights do not sum to 255 still blends to 1.
        let lod = test_lod(1, vec![weights(2, &[(0, 100), (1, 100)])], vec![0, 1]);
        let skin = SkinData::new(&lod, &skeleton(2)).unwrap();
        let v = skin.vertices[0];
        assert!((v.weights[0] - 0.5).abs() < 1e-6);
        assert!((v.weights[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn palettes_are_skeleton_sized_with_a_trailing_identity_slot() {
        let lod = test_lod(1, vec![weights(1, &[(0, 255)])], vec![0]);
        let skin = SkinData::new(&lod, &skeleton(5)).unwrap();
        assert_eq!(skin.palette_len, 6);
        assert_eq!(skin.vertices[0].bones[0], 0);
        // The identity slot is one past the last Skeleton bone.
        let empty = test_lod(1, vec![weights(0, &[])], vec![0]);
        let skin = SkinData::new(&empty, &skeleton(5)).unwrap();
        assert_eq!(skin.vertices[0].bones, [5; 4]);
        assert_eq!(skin.vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn vertices_without_influences_stay_in_the_rest_pose() {
        let lod = test_lod(2, vec![weights(0, &[]), weights(1, &[(0, 0)])], vec![0, 1]);
        let skin = SkinData::new(&lod, &skeleton(2)).unwrap();
        assert_eq!(
            skin.vertices[0],
            SkinVertex {
                bones: [2; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            }
        );
        assert_eq!(
            skin.vertices[1].bones, [2; 4],
            "a zero weight means no influence"
        );
    }

    #[test]
    fn vertices_past_the_bone_weight_array_stay_in_the_rest_pose() {
        let lod = test_lod(3, vec![weights(1, &[(0, 255)])], vec![0]);
        let skin = SkinData::new(&lod, &skeleton(1)).unwrap();
        assert_eq!(skin.vertices.len(), 3);
        assert_eq!(skin.vertices[0].bones, [0, 1, 1, 1]);
        assert_eq!(skin.vertices[1].bones, [1; 4]);
        assert_eq!(skin.vertices[2].bones, [1; 4]);
    }

    #[test]
    fn out_of_range_lod_bones_sink_to_the_identity_slot() {
        // LOD bone 1 maps to a Skeleton bone that does not exist; LOD bone 2 has no entry.
        let lod = test_lod(1, vec![weights(2, &[(1, 128), (2, 127)])], vec![0, 99]);
        let skin = SkinData::new(&lod, &skeleton(2)).unwrap();
        let v = skin.vertices[0];
        assert_eq!(v.bones[0], 2);
        assert_eq!(v.bones[1], 2);
    }

    #[test]
    fn an_empty_sub_skeleton_table_uses_lod_bones_as_skeleton_bones() {
        let lod = test_lod(1, vec![weights(1, &[(1, 255)])], Vec::new());
        let skin = SkinData::new(&lod, &skeleton(3)).unwrap();
        assert_eq!(skin.vertices[0].bones[0], 1);
        let lod = test_lod(1, vec![weights(1, &[(9, 255)])], Vec::new());
        let skin = SkinData::new(&lod, &skeleton(3)).unwrap();
        assert_eq!(skin.vertices[0].bones[0], 3);
    }

    #[test]
    fn more_than_four_influences_are_cut_to_four() {
        let mut w = weights(5, &[(0, 50), (1, 50), (2, 50), (3, 55)]);
        w.pairs[3] = (3, 55);
        let lod = test_lod(1, vec![w], vec![0, 1, 2, 3]);
        let skin = SkinData::new(&lod, &skeleton(4)).unwrap();
        assert_eq!(skin.vertices[0].bones, [0, 1, 2, 3]);
        assert!((skin.vertices[0].weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_unskinned_lod_or_skeleton_is_not_skinned() {
        let lod = test_lod(1, Vec::new(), Vec::new());
        assert_eq!(SkinData::new(&lod, &skeleton(2)), None);
        let lod = test_lod(1, vec![weights(1, &[(0, 255)])], vec![0]);
        assert_eq!(SkinData::new(&lod, &skeleton(0)), None);
    }
}
