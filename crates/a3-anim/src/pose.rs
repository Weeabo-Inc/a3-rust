//! Bone poses, skinning matrices and skinned vertices.

use a3_p3d::{Lod, Model};
use glam::{Affine3A, Vec3};

use crate::eval::{AnimTransform, animation_transform};
use crate::sources::Sources;

/// Model-space transforms of every bone of a Skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    /// Per skeleton bone: the transform from the rest pose to the posed model.
    pub bones: Vec<Affine3A>,
    /// Per skeleton bone: hidden by a `hide` animation on it or on a parent.
    pub hidden: Vec<bool>,
}

impl Pose {
    /// The rest pose of `bone_count` bones.
    pub fn identity(bone_count: usize) -> Self {
        Self {
            bones: vec![Affine3A::IDENTITY; bone_count],
            hidden: vec![false; bone_count],
        }
    }

    /// Per LOD bone (the index used by vertex bone weights): its skinning matrix. Hidden bones
    /// get the zero matrix, which collapses their vertices. Without the ODOL sub-skeleton
    /// table, LOD bones are taken to be skeleton bones.
    pub fn skinning(&self, lod: &Lod) -> Vec<Affine3A> {
        let matrix = |bone: usize| match self.bones.get(bone) {
            Some(_) if self.hidden[bone] => Affine3A::ZERO,
            Some(m) => *m,
            None => Affine3A::IDENTITY,
        };
        match lod.odol.as_ref() {
            Some(odol) if !odol.sub_skeleton.is_empty() => odol
                .sub_skeleton
                .iter()
                .map(|&b| matrix(b as usize))
                .collect(),
            _ => (0..self.bones.len()).map(matrix).collect(),
        }
    }
}

/// Poses the Skeleton of `model` for LOD `lod` with the given source values.
///
/// Each bone's own transform is the product of the animations bound to it in that LOD, the
/// first listed applied first. A bone's model-space transform is its parent's transform times
/// its own, so children follow their parents. Models without a skeleton give an empty pose.
pub fn pose(model: &Model, lod: usize, sources: &Sources) -> Pose {
    let Some(skeleton) = &model.skeleton else {
        return Pose::identity(0);
    };
    let n = skeleton.bones.len();
    let mut local = vec![Affine3A::IDENTITY; n];
    let mut own_hidden = vec![false; n];
    // The per-LOD bone -> animations table, or (when the file stores none) the bone each
    // animation is bound to in this LOD, in animation order.
    let mut table: Vec<Vec<u32>> = model
        .lods
        .get(lod)
        .map(|l| l.bone_animations.clone())
        .unwrap_or_default();
    if table.is_empty() {
        table = vec![Vec::new(); n];
        for (i, anim) in model.animations.iter().enumerate() {
            let bone = anim.bindings.get(lod).copied().flatten().map(|b| b.bone);
            if let Some(list) = bone.and_then(|b| table.get_mut(b as usize)) {
                list.push(i as u32);
            }
        }
    }
    {
        for (bone, anims) in table.iter().enumerate().take(n) {
            for &a in anims {
                let Some(anim) = model.animations.get(a as usize) else {
                    continue;
                };
                match animation_transform(anim, lod, sources.get(&anim.source)) {
                    AnimTransform::Matrix(m) => local[bone] = m * local[bone],
                    AnimTransform::Hidden => own_hidden[bone] = true,
                }
            }
        }
    }

    // Compose through the parents. Parents may come after children in the bone list, so
    // resolve recursively with memoisation; a parent cycle (corrupt data) is cut at the bone
    // that closes it.
    let mut pose = Pose::identity(n);
    let mut state = vec![0u8; n]; // 0 = todo, 1 = in progress, 2 = done
    fn resolve(
        b: usize,
        parents: &[Option<usize>],
        local: &[Affine3A],
        own_hidden: &[bool],
        state: &mut [u8],
        pose: &mut Pose,
    ) {
        if state[b] != 0 {
            return;
        }
        state[b] = 1;
        let (mut world, mut hidden) = (local[b], own_hidden[b]);
        if let Some(p) = parents[b].filter(|&p| p < parents.len()) {
            resolve(p, parents, local, own_hidden, state, pose);
            if state[p] == 2 {
                world = pose.bones[p] * world;
                hidden |= pose.hidden[p];
            }
        }
        pose.bones[b] = world;
        pose.hidden[b] = hidden;
        state[b] = 2;
    }
    let parents: Vec<Option<usize>> = skeleton.bones.iter().map(|b| b.parent).collect();
    for b in 0..n {
        resolve(b, &parents, &local, &own_hidden, &mut state, &mut pose);
    }
    pose
}

/// Vertex positions and normals after skinning.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinnedVertices {
    /// One per vertex of the LOD.
    pub positions: Vec<Vec3>,
    /// One per vertex when the LOD has normals, else empty.
    pub normals: Vec<Vec3>,
}

/// The bone matrix blend of one vertex: weights are bytes summing to 255. Vertices without
/// bone weights stay in place.
fn blend(lod: &Lod, vertex: usize, skinning: &[Affine3A]) -> Affine3A {
    let Some(w) = lod.vertices.bone_weights.get(vertex) else {
        return Affine3A::IDENTITY;
    };
    if w.count == 0 {
        return Affine3A::IDENTITY;
    }
    let mut sum = Affine3A::ZERO;
    let mut total = 0.0;
    for &(bone, weight) in &w.pairs[..w.count.min(4) as usize] {
        let m = skinning
            .get(usize::from(bone))
            .copied()
            .unwrap_or(Affine3A::IDENTITY);
        let k = f32::from(weight) / 255.0;
        sum.matrix3 += m.matrix3 * k;
        sum.translation += m.translation * k;
        total += k;
    }
    if total <= 0.0 {
        return Affine3A::IDENTITY;
    }
    sum
}

/// Skins the vertices of `lod` with per-LOD-bone `skinning` matrices (from
/// [`Pose::skinning`]). Normals are transformed by the blended matrix and renormalised.
pub fn skin(lod: &Lod, skinning: &[Affine3A]) -> SkinnedVertices {
    let v = &lod.vertices;
    let mut out = SkinnedVertices {
        positions: Vec::with_capacity(v.len()),
        normals: Vec::with_capacity(v.normals.len()),
    };
    for (i, p) in v.positions.iter().enumerate() {
        let m = blend(lod, i, skinning);
        out.positions.push(m.transform_point3(*p));
        if let Some(n) = v.normals.get(i) {
            out.normals
                .push(m.transform_vector3(*n).normalize_or_zero());
        }
    }
    out
}

/// Per section of `lod`: `true` when every vertex of its faces is bound only to hidden bones
/// (all its bone matrices are zero), so the pose hides the whole section.
pub fn hidden_sections(lod: &Lod, skinning: &[Affine3A]) -> Vec<bool> {
    let vertex_hidden = |i: u32| {
        let Some(w) = lod.vertices.bone_weights.get(i as usize) else {
            return false;
        };
        w.count > 0
            && w.pairs[..w.count.min(4) as usize]
                .iter()
                .all(|&(bone, weight)| {
                    weight == 0 || skinning.get(usize::from(bone)) == Some(&Affine3A::ZERO)
                })
    };
    lod.sections
        .iter()
        .map(|s| {
            let faces = &lod.faces[s.faces.start as usize..s.faces.end as usize];
            !faces.is_empty()
                && faces
                    .iter()
                    .all(|f| f.indices().iter().all(|&i| vertex_hidden(i)))
        })
        .collect()
}
