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
//! - The records are stored a half turn about Y away from the decoded model data: the engine's
//!   plain-RTM loader converts them for a reversed model by conjugating with `diag(-1, 1, -1)`,
//!   and the shipped binarized records need the same conversion to fit the shipped soldier
//!   ([`reversed`]). [`RtmBinding::keyframe`] applies it.
//! - A bone's frame `[R | R * pivot + t]` maps its rest pivot onto its posed joint **relative to
//!   its parent**: the skinning matrix is the parent's composed matrix times the bone's own frame
//!   over its pivot, down from the root ([`Pose::from_rtm_frames`], [`compose_hierarchy`]), as
//!   the engine's skeleton walk (`0x12499f0` / `0x124b550`) does.
//! - Keyframes and animations are blended here by weighting whole matrices linearly (the
//!   engine's skeleton-less path; `a3-pose` blends as the skeletal path does).

use a3_p3d::{Lod, LodKind, Model, Skeleton};
use a3_rtm::{Animation as Rtm, BoneTransform};
use glam::{Affine3A, Quat, Vec3};

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
        }
    }
}

/// An RTM record in the decoded model space: the record conjugated by a half turn about Y
/// (`diag(-1, 1, -1)`), which negates the rotation's x and z components and the translation's x
/// and z. The engine's plain-RTM loader (`0x1250490`) applies exactly this to every matrix when
/// the animation is loaded for a reversed model (flag at `+0x10c` of the animation); the shipped
/// binarized records need it too (`docs/re/model-animations.md`, "Reversed records").
pub fn reversed(record: BoneTransform) -> BoneTransform {
    let q = record.rotation;
    let t = record.translation;
    BoneTransform {
        rotation: Quat::from_xyzw(-q.x, q.y, -q.z, q.w),
        translation: Vec3::new(-t.x, t.y, -t.z),
    }
}

/// Composes per-bone frames relative to their parents into model-space frames: each bone gets
/// its parent's composed frame times its own, down from the roots (the engine's skeleton walk).
/// `parents[b]` is bone `b`'s parent; a missing or out-of-range parent makes the bone a root,
/// and a parent cycle (not in shipped data) is cut where it closes.
pub fn compose_hierarchy(local: &[Affine3A], parents: &[Option<usize>]) -> Vec<Affine3A> {
    let n = local.len();
    let parent = |b: usize| parents.get(b).copied().flatten().filter(|&p| p < n);
    let mut world: Vec<Option<Affine3A>> = vec![None; n];
    for bone in 0..n {
        // The chain up to the first composed ancestor (or a root), then composed back down.
        let mut chain = vec![bone];
        let mut at = bone;
        while world[at].is_none() {
            match parent(at) {
                Some(p) if world[p].is_none() && !chain.contains(&p) => {
                    chain.push(p);
                    at = p;
                }
                _ => break,
            }
        }
        while let Some(b) = chain.pop() {
            if world[b].is_some() {
                continue;
            }
            let above = parent(b).and_then(|p| world[p]);
            world[b] = Some(above.map_or(local[b], |p| p * local[b]));
        }
    }
    world
        .into_iter()
        .map(|w| w.unwrap_or(Affine3A::IDENTITY))
        .collect()
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
    /// with `R` the conjugate of the [`reversed`] record's quaternion and `t` its translation
    /// (identity for unbound bones).
    pub fn keyframe(&self, rtm: &Rtm, frame: usize, pivots: &SkeletonPivots) -> Vec<Affine3A> {
        let Some(f) = rtm.frames.get(frame) else {
            return vec![Affine3A::IDENTITY; self.rtm_bone.len()];
        };
        self.rtm_bone
            .iter()
            .enumerate()
            .map(|(b, rb)| {
                let Some(&t) = rb.and_then(|i| f.transforms.get(i)) else {
                    return Affine3A::IDENTITY;
                };
                let t = reversed(t);
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

/// Weights two lists of bone frames linearly, matrix by matrix, as the engine blends keyframes
/// and animations: `t = 0` gives `a`, `t = 1` gives `b`.
pub fn blend(a: &[Affine3A], b: &[Affine3A], t: f32) -> Vec<Affine3A> {
    a.iter()
        .zip(b)
        .map(|(x, y)| Affine3A {
            matrix3: x.matrix3 * (1.0 - t) + y.matrix3 * t,
            translation: x.translation * (1.0 - t) + y.translation * t,
        })
        .collect()
}

impl Pose {
    /// A pose from RTM bone frames (see [`RtmBinding::frames`]): each bone's own frame over its
    /// rest pivot, `frame * translate(-pivot)`, composed with its parents' down from the root
    /// ([`compose_hierarchy`] over `skeleton`'s parents). `model_offset` moves model space into
    /// pivot space: for an ODOL model, its `bounding_center` (the pivots model is not
    /// autocentred).
    pub fn from_rtm_frames(
        frames: &[Affine3A],
        pivots: &SkeletonPivots,
        skeleton: &Skeleton,
        model_offset: Vec3,
    ) -> Self {
        let to_pivot = Affine3A::from_translation(model_offset);
        let back = Affine3A::from_translation(-model_offset);
        let local: Vec<Affine3A> = frames
            .iter()
            .enumerate()
            .map(|(b, f)| {
                let pivot = pivots.positions.get(b).copied().unwrap_or(Vec3::ZERO);
                *f * Affine3A::from_translation(-pivot)
            })
            .collect();
        let parents: Vec<Option<usize>> = (0..frames.len())
            .map(|b| skeleton.bones.get(b).and_then(|bone| bone.parent))
            .collect();
        Self {
            bones: compose_hierarchy(&local, &parents)
                .into_iter()
                .map(|w| back * w * to_pivot)
                .collect(),
            hidden: vec![false; frames.len()],
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
