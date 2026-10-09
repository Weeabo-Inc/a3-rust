//! Poses of a Man from its move state.
//!
//! A move state is the move a Man is leaving and the one it is entering, each with a position
//! (phase) in its own cycle, plus the blend phase between them. A [`ManRig`] — a Skeleton with
//! its joint pivots — turns that into a [`ManPose`]: one frame per bone of the Man skeleton,
//! relative to its parent bone. [`ManRig::skinning_pose`] composes it down the Skeleton into the
//! model-space pose that skins the Man's model ([`a3_anim::Pose::skinning`]).
//!
//! The conventions follow the engine (`docs/re/model-animations.md`, "RTM skeletal poses"):
//!
//! - A record is first [reversed](a3_anim::reversed) — turned a half turn about Y into the
//!   decoded model space — and its quaternion `q` is then used as the matrix of `q`'s conjugate
//!   (the engine's decoder writes the rows of the usual quaternion matrix into its columns).
//! - A bone's joint is `R * pivot + t` over the bone's rest pivot `Q` (the raw `t` for the
//!   `weaponBone`, which is empty for the shipped Man skeleton); that is the quantity the
//!   engine's blend averages and its frame is emitted from.
//! - Poses blend by slerping rotations and lerping joints, both between the keyframes of one
//!   move and between the previous and the current move.
//! - The emitted bone frame is `[M | T - M * Q]` with `M` the blend's matrix, `T` the blended
//!   joint and `Q` the rest pivot: it maps the bone's rest pivot onto its posed joint in its
//!   parent's frame. Unblended it reduces to `[R | t]`, the file's own record. The stored
//!   quaternions are f16-quantized and are not renormalized, as in the engine, so a frame is
//!   orthonormal to about 1e-4.
//! - A bone's model-space matrix is its parent's times its own frame, down from the root (the
//!   engine's skeleton walk), so a child follows every parent above it.
//!
//! The composed pose is in the pivots model's space, where the root record lifts the pelvis to
//! its hip height so the feet stand on `y = 0`: the entity's position is that ground point.
//! The move's own advance is [`MoveSample::step`], not a bone translation.
//!
//! A moves type (`a3-moves`) names each move's RTM by path and plays it to a phase: [`MoveClips`]
//! holds the animations themselves, keyed by that path, and [`MoveBlend`] is a blend state in the
//! moves type's terms (move ids and phases), which resolves to a [`MoveState`] over the clips.

mod clips;

pub use clips::{ClipLoadReport, MoveBlend, MoveClips};

use a3_anim::SkeletonPivots;
use a3_p3d::Skeleton;
use a3_rtm::Animation;
use glam::{Affine3A, Mat3, Quat, Vec3};

/// One move at one position in its cycle.
#[derive(Debug, Clone, Copy)]
pub struct MoveSample<'a> {
    /// The move's animation.
    pub animation: &'a Animation,
    /// Position in the move's cycle, `0` (start) to `1` (end); keyframes sit at phases.
    pub phase: f32,
}

impl<'a> MoveSample<'a> {
    /// The move on `animation`, played to `phase` of its cycle.
    pub fn new(animation: &'a Animation, phase: f32) -> Self {
        Self { animation, phase }
    }

    /// Distance the move carries the Man over one cycle (the animation's move vector): forward
    /// is `-Z`, `+X` to the Man's left. The caller advances the entity by this scaled by how
    /// fast the cycle runs; no bone translation carries it.
    pub fn step(&self) -> Vec3 {
        self.animation.step
    }
}

/// A move state: the move being left, the move being entered, and where between them the Man is.
#[derive(Debug, Clone, Copy)]
pub struct MoveState<'a> {
    /// The move being left (all of the blend at phase 0).
    pub previous: MoveSample<'a>,
    /// The move being entered (all of the blend at phase 1).
    pub current: MoveSample<'a>,
    /// Blend position between the two moves, `0..=1` (clamped).
    pub phase: f32,
}

impl<'a> MoveState<'a> {
    /// A Man blending from `previous` to `current`, `phase` of the way across.
    pub fn new(previous: MoveSample<'a>, current: MoveSample<'a>, phase: f32) -> Self {
        Self {
            previous,
            current,
            phase,
        }
    }

    /// A Man in one move: the blend between it and itself has no effect.
    pub fn single(move_: MoveSample<'a>) -> Self {
        Self {
            previous: move_,
            current: move_,
            phase: 0.0,
        }
    }
}

/// One bone of a pose: its frame relative to its parent bone, in the model's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BonePose {
    /// The bone's orientation.
    pub rotation: Quat,
    /// The bone's frame translation, as the engine emits it: `T - R * Q`, the posed joint `T`
    /// taken back over the rest pivot `Q`.
    pub translation: Vec3,
}

impl BonePose {
    /// A bone at its rest pivot: no rotation, no offset.
    pub const REST: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// The bone's rotation as a matrix.
    pub fn matrix(self) -> Mat3 {
        Mat3::from_quat(self.rotation)
    }

    /// The bone's frame as an affine matrix.
    pub fn to_affine(self) -> Affine3A {
        Affine3A::from_rotation_translation(self.rotation, self.translation)
    }

    /// Where `rest` lands under this frame, `R * rest + t`: the bone's joint in its parent's
    /// frame, as the engine's load conversion defines it — the quantity the blend lerps.
    pub fn posed_pivot(self, rest: Vec3) -> Vec3 {
        self.translation + self.matrix() * rest
    }
}

/// The pose of every bone of a Man, in the skeleton's bone order.
#[derive(Debug, Clone, PartialEq)]
pub struct ManPose {
    /// One frame per skeleton bone.
    pub bones: Vec<BonePose>,
}

impl ManPose {
    /// The rest pose of a Man of `bone_count` bones: every bone at its rest pivot.
    pub fn identity(bone_count: usize) -> Self {
        Self {
            bones: vec![BonePose::REST; bone_count],
        }
    }

    /// The bone frames as affine matrices, each relative to its parent bone.
    pub fn to_affines(&self) -> Vec<Affine3A> {
        self.bones.iter().map(|b| b.to_affine()).collect()
    }
}

/// A Man's Skeleton with the rest pivots its moves are posed over.
#[derive(Debug, Clone, PartialEq)]
pub struct ManRig {
    names: Vec<String>,
    parents: Vec<Option<usize>>,
    pivots: SkeletonPivots,
    root: Option<usize>,
}

impl ManRig {
    /// A rig for `skeleton`, with the joints of the CfgSkeletonParameters `pivotsModel`
    /// (`SkeletonPivots::from_model`). `weapon_bone` is the skeleton's `weaponBone`, whose
    /// translation the engine does not convert (empty for none); it is matched by name.
    pub fn new(skeleton: &Skeleton, pivots_model: &a3_p3d::Model, weapon_bone: &str) -> Self {
        Self::with_pivots(
            skeleton,
            SkeletonPivots::from_model(skeleton, pivots_model, weapon_bone),
        )
    }

    /// A rig from a Skeleton and pivots already built.
    pub fn with_pivots(skeleton: &Skeleton, pivots: SkeletonPivots) -> Self {
        Self {
            names: skeleton.bones.iter().map(|b| b.name.clone()).collect(),
            parents: skeleton.bones.iter().map(|b| b.parent).collect(),
            pivots,
            root: skeleton.bones.iter().position(|b| b.parent.is_none()),
        }
    }

    /// Number of bones of the Man.
    pub fn bone_count(&self) -> usize {
        self.names.len()
    }

    /// Index of the bone named `name`, ignoring ASCII case.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|b| b.eq_ignore_ascii_case(name))
    }

    /// Index of the Man's root bone, the pelvis of a shipped soldier skeleton.
    pub fn root_bone(&self) -> Option<usize> {
        self.root
    }

    /// The rest pivots this rig poses over.
    pub fn pivots(&self) -> &SkeletonPivots {
        &self.pivots
    }

    /// The Man's pose in `state`: every bone's frame of the previous move blended towards the
    /// current one by [`MoveState::phase`].
    pub fn pose(&self, state: &MoveState<'_>) -> ManPose {
        // The engine clamps a blend weight to 0..1.
        let phase = if state.phase.is_nan() {
            0.0
        } else {
            state.phase.clamp(0.0, 1.0)
        };
        let bones = self
            .names
            .iter()
            .enumerate()
            .map(|(bone, name)| {
                let rest = self.rest_pivot(bone);
                let previous = sample_bone(&self.pivots, bone, name, state.previous);
                let current = sample_bone(&self.pivots, bone, name, state.current);
                // The engine's record blend (`0x12102c0`): slerp the rotations, lerp the posed
                // joints.
                let rotation = previous.rotation.slerp(current.rotation, phase);
                let joint = previous.joint.lerp(current.joint, phase);
                BonePose {
                    // The engine's emitted frame (`0x12105b0`): `T - M * Q`.
                    translation: joint - Mat3::from_quat(rotation) * rest,
                    rotation,
                }
            })
            .collect();
        ManPose { bones }
    }

    /// The parent of every bone, in skeleton order (`None` for the root).
    pub fn parents(&self) -> &[Option<usize>] {
        &self.parents
    }

    /// The model-space pose of `pose`: every bone's frame composed with its parents' down from
    /// the root, in the pivots model's space.
    pub fn compose(&self, pose: &ManPose) -> Vec<Affine3A> {
        a3_anim::compose_hierarchy(&pose.to_affines(), &self.parents)
    }

    /// `pose` as the [`a3_anim::Pose`] that skins the Man's model (nothing hidden), for
    /// [`a3_anim::Pose::skinning`] and a renderer's bone palette. `model_offset` is where the
    /// model's origin sits in the pivots model's space: an ODOL model's vertices are stored
    /// relative to its `bounding_center`, so pass that. Each matrix is
    /// `translate(-offset) * composed * translate(offset)`: it takes a rest vertex of the model
    /// into pivot space, poses it there and brings it back, so the posed mesh stays in the
    /// model's space and the ground under the posed Man is at `y = -offset.y`
    /// ([`ManRig::ground_offset`]).
    pub fn skinning_pose(&self, pose: &ManPose, model_offset: Vec3) -> a3_anim::Pose {
        let bones = self.palette(&self.compose(pose), model_offset);
        a3_anim::Pose {
            hidden: vec![false; bones.len()],
            bones,
        }
    }

    /// The bone palette, in this rig's bone order, of a model at `model_offset` (see
    /// [`ManRig::skinning_pose`]) from an already [composed](ManRig::compose) pose.
    pub fn palette(&self, composed: &[Affine3A], model_offset: Vec3) -> Vec<Affine3A> {
        let to_pivot = Affine3A::from_translation(model_offset);
        let back = Affine3A::from_translation(-model_offset);
        composed.iter().map(|m| back * *m * to_pivot).collect()
    }

    /// The bone palette of another model the Man wears (a head, a vest, a helmet), in that
    /// model's own `skeleton` order: each bone takes the Man's bone of the same name (ignoring
    /// case), and a bone the Man does not have stays at rest. `model_offset` is that model's
    /// own offset from the pivots model's origin (its `bounding_center` when autocentred).
    pub fn palette_for(
        &self,
        composed: &[Affine3A],
        skeleton: &Skeleton,
        model_offset: Vec3,
    ) -> Vec<Affine3A> {
        let mine = self.palette(composed, model_offset);
        skeleton
            .bones
            .iter()
            .map(|b| {
                self.bone_index(&b.name)
                    .and_then(|i| mine.get(i).copied())
                    .unwrap_or(Affine3A::IDENTITY)
            })
            .collect()
    }

    /// A per-bone mask, in this rig's order: 1 for the bone named `root` and every bone below
    /// it, 0 elsewhere (all 0 when there is no such bone).
    pub fn subtree(&self, root: &str) -> Vec<f32> {
        let Some(root) = self.bone_index(root) else {
            return vec![0.0; self.names.len()];
        };
        (0..self.names.len())
            .map(|mut b| {
                // Walk up to the root of the skeleton; the bone count bounds a broken cycle.
                for _ in 0..=self.names.len() {
                    if b == root {
                        return 1.0;
                    }
                    match self.parents[b] {
                        Some(p) => b = p,
                        None => return 0.0,
                    }
                }
                0.0
            })
            .collect()
    }

    /// `pose` with another animation laid over it by per-bone `weights` (in this rig's order;
    /// missing entries count as 0): each bone's rotation slerps and its joint lerps towards the
    /// layer's by its weight, clamped to 0..1, as the engine folds a layer in (`0x12105b0`).
    pub fn overlay(&self, pose: &ManPose, layer: MoveSample<'_>, weights: &[f32]) -> ManPose {
        let bones = pose
            .bones
            .iter()
            .enumerate()
            .map(|(bone, base)| {
                let w = weights.get(bone).copied().unwrap_or(0.0);
                let w = if w.is_nan() { 0.0 } else { w.clamp(0.0, 1.0) };
                // The engine skips a layer weight at or below 0.001.
                if w <= 0.001 {
                    return *base;
                }
                let rest = self.rest_pivot(bone);
                let top = sample_bone(&self.pivots, bone, &self.names[bone], layer);
                let rotation = base.rotation.slerp(top.rotation, w);
                let joint = base.posed_pivot(rest).lerp(top.joint, w);
                BonePose {
                    translation: joint - Mat3::from_quat(rotation) * rest,
                    rotation,
                }
            })
            .collect();
        ManPose { bones }
    }

    /// Where the ground under a posed Man is in the space of a model whose vertices sit at
    /// `model_offset` from the pivots model's origin (see [`ManRig::skinning_pose`]): the
    /// point to put on the entity's position.
    pub fn ground_offset(model_offset: Vec3) -> Vec3 {
        -model_offset
    }

    fn rest_pivot(&self, bone: usize) -> Vec3 {
        self.pivots
            .positions
            .get(bone)
            .copied()
            .unwrap_or(Vec3::ZERO)
    }
}

/// One move's contribution to one bone: its rotation and the position of its posed joint.
#[derive(Debug, Clone, Copy)]
struct BoneSample {
    rotation: Quat,
    joint: Vec3,
}

/// Samples `bone` (`name`) of one move at its phase: the two surrounding keyframes slerped and
/// lerped, on the load-converted records.
fn sample_bone(
    pivots: &SkeletonPivots,
    bone: usize,
    name: &str,
    move_: MoveSample<'_>,
) -> BoneSample {
    let rest = pivots.positions.get(bone).copied().unwrap_or(Vec3::ZERO);
    let rest_sample = BoneSample {
        rotation: Quat::IDENTITY,
        joint: rest,
    };
    // A bone the move does not animate keeps its rest pivot. (In the engine its record comes
    // from a constant one; a shipped skeleton has a name for every RTM bone.)
    let Some(rtm_bone) = move_.animation.bone_index(name) else {
        return rest_sample;
    };
    if move_.animation.frames.is_empty() {
        return rest_sample;
    }
    let at = |frame: usize| {
        let Some(t) = move_
            .animation
            .frames
            .get(frame)
            .and_then(|f| f.transforms.get(rtm_bone))
        else {
            return rest_sample;
        };
        let t = a3_anim::reversed(*t);
        let rotation = t.rotation.conjugate();
        let joint = if pivots.weapon_bone == Some(bone) {
            // The load conversion skips the weapon bone: `t` stays an offset.
            t.translation
        } else {
            Mat3::from_quat(rotation) * rest + t.translation
        };
        BoneSample { rotation, joint }
    };
    let (a, b, weight) = frame_pair(move_.animation, move_.phase);
    let (a, b) = (at(a), at(b));
    BoneSample {
        rotation: a.rotation.slerp(b.rotation, weight),
        joint: a.joint.lerp(b.joint, weight),
    }
}

/// The two keyframes around `phase` and the blend weight between them (the rule
/// `a3_anim::RtmBinding::frames` uses): a phase outside the keyframes clamps to the first or
/// last one, and a zero-length span snaps to its first frame.
fn frame_pair(animation: &Animation, phase: f32) -> (usize, usize, f32) {
    let frames = &animation.frames;
    let next = frames.partition_point(|f| f.phase <= phase);
    if phase.is_nan() || next == 0 {
        return (0, 0, 0.0);
    }
    if next >= frames.len() {
        let last = frames.len() - 1;
        return (last, last, 0.0);
    }
    let (a, b) = (next - 1, next);
    let span = frames[b].phase - frames[a].phase;
    let weight = if span > 0.0 {
        ((phase - frames[a].phase) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a, b, weight)
}
