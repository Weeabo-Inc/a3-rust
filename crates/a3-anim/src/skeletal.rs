//! Skeletal poses from RTM animations.
//!
//! What follows the engine (`docs/re/model-animations.md`, "RTM"):
//!
//! - RTM bones bind to Skeleton bones by name.
//! - A stored rotation quaternion `q` is used as the matrix of `q`'s conjugate (the engine's
//!   decoder writes the rows of the usual matrix into its columns).
//! - On load the engine replaces each bone's translation `t` with `R * pivot + t`, the pivot
//!   being the bone's memory point in the skeleton's pivots model, so a bone's frame is
//!   `[R | R * pivot + t]` (`frames`). The weapon bone (`weaponBone`) keeps its raw translation.
//! - Keyframes and animations blend by slerp of the rotations and lerp of the posed pivots.
//!
//! - The pose builder turns a frame back into the local transform `[R | t]`, and bone chains
//!   compose parent first (local transforms, not model-space ones).
//! - RTM space is mirrored in X against model space.
//!
//! [`Pose::from_rtm_frames`] builds skinning matrices from that; see its docs.

use a3_p3d::{Lod, LodKind, Model, Skeleton};
use a3_rtm::Animation as Rtm;
use glam::{Affine3A, Vec3};

use crate::pose::Pose;

/// Which RTM bone drives each Skeleton bone (matched by name, ignoring case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtmBinding {
    /// Per skeleton bone: the index into the RTM's bone list, if the RTM animates it.
    pub rtm_bone: Vec<Option<usize>>,
}

/// Rest positions of the Skeleton's joints, from the skeleton's pivots model.
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonPivots {
    /// Per skeleton bone: its pivot in the pivots model's space.
    pub positions: Vec<Vec3>,
    /// Index of the `weaponBone` of CfgSkeletonParameters, which the engine leaves untouched.
    pub weapon_bone: Option<usize>,
    /// Per skeleton bone: its parent (RTM bone frames are composed down this hierarchy).
    pub parents: Vec<Option<usize>>,
}

impl SkeletonPivots {
    /// Pivots of `skeleton` from the Memory LOD of `pivots_model` (the CfgSkeletonParameters
    /// `pivotsModel`): the first point of the memory selection named like each bone; a bone
    /// without one takes its parent's pivot, and a root without one the origin (as the engine
    /// does).
    pub fn from_model(skeleton: &Skeleton, pivots_model: &Model, weapon_bone: &str) -> Self {
        let memory = pivots_model
            .lods
            .iter()
            .find(|l| l.resolution.kind() == LodKind::Memory);
        let point = |name: &str| -> Option<Vec3> {
            let lod: &Lod = memory?;
            let sel = lod
                .named_selections
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))?;
            let &v = sel.vertices.first()?;
            lod.vertices.positions.get(v as usize).copied()
        };
        let mut positions: Vec<Option<Vec3>> =
            skeleton.bones.iter().map(|b| point(&b.name)).collect();
        // Parents first where the list allows; repeat until stable for out-of-order lists.
        for _ in 0..skeleton.bones.len() {
            let mut changed = false;
            for (b, bone) in skeleton.bones.iter().enumerate() {
                if positions[b].is_none() {
                    if let Some(p) = bone.parent.and_then(|p| positions[p]) {
                        positions[b] = Some(p);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        Self {
            positions: positions
                .into_iter()
                .map(|p| p.unwrap_or(Vec3::ZERO))
                .collect(),
            weapon_bone: skeleton
                .bones
                .iter()
                .position(|b| !weapon_bone.is_empty() && b.name.eq_ignore_ascii_case(weapon_bone)),
            parents: skeleton.bones.iter().map(|b| b.parent).collect(),
        }
    }
}

impl RtmBinding {
    /// Matches the bones of `skeleton` to those of `rtm` by name.
    pub fn new(skeleton: &Skeleton, rtm: &Rtm) -> Self {
        Self {
            rtm_bone: skeleton
                .bones
                .iter()
                .map(|b| rtm.bone_index(&b.name))
                .collect(),
        }
    }

    /// Number of skeleton bones the RTM animates.
    pub fn bound(&self) -> usize {
        self.rtm_bone.iter().filter(|b| b.is_some()).count()
    }

    /// The engine's bone frames of keyframe `frame`: per skeleton bone `[R | R * pivot + t]`
    /// with `R` the conjugate of the stored quaternion (identity for unbound bones).
    pub fn keyframe(&self, rtm: &Rtm, frame: usize, pivots: &SkeletonPivots) -> Vec<Affine3A> {
        let Some(f) = rtm.frames.get(frame) else {
            return vec![Affine3A::IDENTITY; self.rtm_bone.len()];
        };
        self.rtm_bone
            .iter()
            .enumerate()
            .map(|(b, rb)| {
                let Some(t) = rb.and_then(|i| f.transforms.get(i)) else {
                    return Affine3A::IDENTITY;
                };
                let r = t.rotation.conjugate();
                let pivot = pivots.positions.get(b).copied().unwrap_or(Vec3::ZERO);
                let translation = if pivots.weapon_bone == Some(b) {
                    t.translation
                } else {
                    r * pivot + t.translation
                };
                Affine3A::from_rotation_translation(r, translation)
            })
            .collect()
    }

    /// The engine's bone frames at `phase`: the two surrounding keyframes blended linearly
    /// (whole matrices). Phases outside the keyframes clamp to the first or last one.
    pub fn frames(&self, rtm: &Rtm, phase: f32, pivots: &SkeletonPivots) -> Vec<Affine3A> {
        let n = rtm.frames.len();
        if n == 0 {
            return vec![Affine3A::IDENTITY; self.rtm_bone.len()];
        }
        let next = rtm.frames.partition_point(|f| f.phase <= phase);
        if phase.is_nan() || next == 0 {
            return self.keyframe(rtm, 0, pivots);
        }
        if next == n {
            return self.keyframe(rtm, n - 1, pivots);
        }
        let (a, b) = (&rtm.frames[next - 1], &rtm.frames[next]);
        let span = b.phase - a.phase;
        let w = if span > 0.0 {
            (phase - a.phase) / span
        } else {
            0.0
        };
        blend(
            &self.keyframe(rtm, next - 1, pivots),
            &self.keyframe(rtm, next, pivots),
            w,
        )
    }
}

/// Blends two lists of bone frames as the engine's pose builder does with a skeleton:
/// rotations by spherical interpolation, posed pivots (the frame translations) linearly.
/// `t = 0` gives `a`, `t = 1` gives `b`.
pub fn blend(a: &[Affine3A], b: &[Affine3A], t: f32) -> Vec<Affine3A> {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let qa = glam::Quat::from_mat3a(&x.matrix3).normalize();
            let mut qb = glam::Quat::from_mat3a(&y.matrix3).normalize();
            if qa.dot(qb) < 0.0 {
                qb = -qb;
            }
            Affine3A::from_rotation_translation(
                qa.slerp(qb, t).normalize(),
                Vec3::from(x.translation.lerp(y.translation, t)),
            )
        })
        .collect()
}

impl Pose {
    /// The skinning pose for RTM bone frames (see [`RtmBinding::frames`]).
    ///
    /// Each frame `[R | R * pivot + t]` gives the bone's local transform `[R | t]` (the engine's
    /// pose builder subtracts `R * pivot` again); local transforms compose down the skeleton,
    /// parent first, as the engine's bone-chain walk does. RTM space is mirrored in X against
    /// model space, so the composed transform is conjugated with that mirror. `model_offset`
    /// moves model space into pivot space (for an autocentred ODOL model and a pivots model
    /// that is not autocentred: the model's `bounding_center`).
    ///
    /// The root bone's translation is the pelvis height above the ground (about 1.0 m standing,
    /// 0.11 m prone), so the posed model sits that high above where the rest pose had its pelvis.
    pub fn from_rtm_frames(frames: &[Affine3A], pivots: &SkeletonPivots, model_offset: Vec3) -> Self {
        let n = frames.len();
        let local: Vec<Affine3A> = (0..n)
            .map(|b| {
                let pivot = pivots.positions.get(b).copied().unwrap_or(Vec3::ZERO);
                frames[b] * Affine3A::from_translation(-pivot)
            })
            .collect();
        let parents: Vec<Option<usize>> = (0..n)
            .map(|b| pivots.parents.get(b).copied().flatten().filter(|&p| p < n))
            .collect();
        let mut world: Vec<Option<Affine3A>> = vec![None; n];
        fn resolve(
            b: usize,
            local: &[Affine3A],
            parents: &[Option<usize>],
            world: &mut [Option<Affine3A>],
            depth: usize,
        ) -> Affine3A {
            if let Some(m) = world[b] {
                return m;
            }
            let m = match parents[b] {
                // A parent cycle (corrupt data) is cut after as many steps as there are bones.
                Some(p) if depth < local.len() => {
                    resolve(p, local, parents, world, depth + 1) * local[b]
                }
                _ => local[b],
            };
            world[b] = Some(m);
            m
        }
        let mirror = Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let to_pivot = Affine3A::from_translation(model_offset);
        let back = Affine3A::from_translation(-model_offset);
        Self {
            bones: (0..n)
                .map(|b| {
                    let w = resolve(b, &local, &parents, &mut world, 0);
                    back * mirror * w * mirror * to_pivot
                })
                .collect(),
            hidden: vec![false; n],
        }
    }
    /// Applies `config` (a pose from model.cfg animations, see [`crate::pose`]) under `self` (an
    /// RTM pose): each bone gets `self * config`, so the config animation acts in the rest pose
    /// and the RTM moves the result. Hidden bones come from `config`. Bones missing from either
    /// side count as identity.
    pub fn compose(&self, config: &Pose) -> Pose {
        let n = self.bones.len().max(config.bones.len());
        let get = |p: &Pose, i: usize| p.bones.get(i).copied().unwrap_or(Affine3A::IDENTITY);
        Pose {
            bones: (0..n).map(|i| get(self, i) * get(config, i)).collect(),
            hidden: (0..n)
                .map(|i| config.hidden.get(i).copied().unwrap_or(false))
                .collect(),
        }
    }
}
