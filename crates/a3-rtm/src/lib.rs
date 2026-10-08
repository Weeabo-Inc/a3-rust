//! Reader for RTM skeletal animations.
//!
//! An RTM holds, for a list of bones, one transform per bone at each of a series of keyframes.
//! Each keyframe sits at a phase in `0..=1` of the animation cycle. Two encodings ship:
//!
//! - Plain `RTM_0101` (optionally preceded by an `RTM_MDAT` keystone section): fixed 32-byte bone
//!   names and a 4x3 matrix per bone per frame.
//! - Binarized `BMTR` version 5: quaternion (i16) plus half-float translation per bone per
//!   frame, with large arrays LZO-compressed.
//!
//! Both decode to the same [`Animation`]. See `docs/re/rtm.md` for layouts and the survey.

mod bmtr;
mod cursor;
mod error;
mod half;
mod plain;

use glam::{Affine3A, Mat3, Quat, Vec3};

pub use error::{Error, Result};
pub use half::f16_to_f32;

/// How an [`Animation`] was stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Plain `RTM_0101` (with or without an `RTM_MDAT` section).
    Plain,
    /// Binarized `BMTR` of the given version.
    Binarized {
        /// Format version (5 in every shipped file).
        version: u32,
    },
}

/// The pose of one bone in one keyframe: a rotation, then a translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneTransform {
    /// Orientation, as stored. Unit length up to the precision of the file, except for a few
    /// binarized bones with a scale or a degenerate source matrix (see `docs/re/rtm.md`).
    pub rotation: Quat,
    /// Position.
    pub translation: Vec3,
}

impl BoneTransform {
    /// The transform that leaves every point in place.
    pub const IDENTITY: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// Builds a transform from a 3x3 orientation (columns: aside, up, direction) and a
    /// translation, as stored in plain RTMs. Scale and shear are dropped.
    pub fn from_matrix(orientation: Mat3, translation: Vec3) -> Self {
        Self {
            rotation: Quat::from_mat3(&orientation).normalize(),
            translation,
        }
    }

    /// The transform as an affine matrix.
    pub fn to_affine(self) -> Affine3A {
        Affine3A::from_rotation_translation(self.rotation, self.translation)
    }

    /// Interpolates between `self` (at `t = 0`) and `other` (at `t = 1`): spherical for the
    /// rotation, linear for the translation.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        // Take the short way round: q and -q are the same rotation.
        let other_rotation = if self.rotation.dot(other.rotation) < 0.0 {
            -other.rotation
        } else {
            other.rotation
        };
        Self {
            rotation: self.rotation.slerp(other_rotation, t).normalize(),
            translation: self.translation.lerp(other.translation, t),
        }
    }
}

/// One keyframe: a phase and a transform for every bone of the animation, in bone order.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Position of the keyframe in the animation cycle, normally `0..=1`.
    pub phase: f32,
    /// One transform per entry of [`Animation::bones`].
    pub transforms: Vec<BoneTransform>,
}

/// A named marker at a phase of the animation, such as a `StepSound` footstep (the engine's
/// "animation keystone").
#[derive(Debug, Clone, PartialEq)]
pub struct Keystone {
    /// The engine's keystone type number, or -1 for "look it up by name" (0 for every shipped
    /// `StepSound`; plain RTMs always give -1).
    pub kind: i32,
    /// Phase at which the keystone fires.
    pub phase: f32,
    /// Keystone name, such as `StepSound`.
    pub name: String,
    /// Keystone argument; empty in every shipped file.
    pub value: String,
}

/// What an RTM says before its keyframes ([`Animation::read_header`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Distance the animated model moves over one cycle (the "move vector").
    pub step: Vec3,
    /// Bone names, as stored.
    pub bones: Vec<String>,
    /// Phase keystones.
    pub keystones: Vec<Keystone>,
    /// See [`Animation::extra_names`].
    pub extra_names: Vec<String>,
}

/// A decoded RTM animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    /// How the file was stored.
    pub encoding: Encoding,
    /// Distance the animated model moves over one cycle (the "move vector").
    pub step: Vec3,
    /// Bone names, as stored. Compare them ignoring case.
    pub bones: Vec<String>,
    /// Keyframes in file order (ascending phase in every shipped file).
    pub frames: Vec<Frame>,
    /// Phase keystones.
    pub keystones: Vec<Keystone>,
    /// A list of names stored before the keystones in binarized files; empty in every shipped
    /// file _(meaning unknown)_.
    pub extra_names: Vec<String>,
}

impl Animation {
    /// Decodes an RTM file in any supported encoding.
    pub fn read(data: &[u8]) -> Result<Self> {
        match data.get(..8) {
            Some(b"RTM_0101") | Some(b"RTM_MDAT") => plain::read(data),
            Some(sig) if sig.starts_with(b"BMTR") => bmtr::read(data),
            _ => Err(Error::UnknownSignature(data[..data.len().min(8)].to_vec())),
        }
    }

    /// Decodes only what precedes the keyframes: the move vector, the bone names and the
    /// keystones. Much cheaper than [`Animation::read`] for binarized files, whose keyframes are
    /// most of the bytes (plain files are read whole).
    pub fn read_header(data: &[u8]) -> Result<Header> {
        match data.get(..4) {
            Some(b"BMTR") => bmtr::read_header(data),
            _ => Self::read(data).map(|a| Header {
                step: a.step,
                bones: a.bones,
                keystones: a.keystones,
                extra_names: a.extra_names,
            }),
        }
    }

    /// Index of the bone named `name`, ignoring ASCII case.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.eq_ignore_ascii_case(name))
    }

    /// The pose of every bone at `phase`, interpolated between the two surrounding keyframes.
    ///
    /// Phases before the first keyframe or after the last one clamp to that keyframe; a looping
    /// caller wraps the phase itself. Returns an empty list for an animation without frames.
    pub fn sample(&self, phase: f32) -> Vec<BoneTransform> {
        (0..self.bones.len())
            .map(|bone| self.sample_bone(bone, phase))
            .collect()
    }

    /// The pose of bone number `bone` at `phase` (see [`Animation::sample`]). Identity for an
    /// animation without frames or a bone index out of range.
    pub fn sample_bone(&self, bone: usize, phase: f32) -> BoneTransform {
        let pose = |frame: &Frame| {
            frame
                .transforms
                .get(bone)
                .copied()
                .unwrap_or(BoneTransform::IDENTITY)
        };
        let (Some(first), Some(last)) = (self.frames.first(), self.frames.last()) else {
            return BoneTransform::IDENTITY;
        };
        // First keyframe strictly after `phase`. Frame phases may be NaN in shipped files, so
        // the ends are found from this index rather than by comparing with the end phases.
        let next = self.frames.partition_point(|f| f.phase <= phase);
        if phase.is_nan() || next == 0 {
            return pose(first);
        }
        if next == self.frames.len() {
            return pose(last);
        }
        let (a, b) = (&self.frames[next - 1], &self.frames[next]);
        let span = b.phase - a.phase;
        let t = if span > 0.0 {
            (phase - a.phase) / span
        } else {
            0.0
        };
        pose(a).lerp(pose(b), t)
    }
}
