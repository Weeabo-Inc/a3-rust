//! Model animation: evaluates the binarised model.cfg animations of an ODOL model into bone
//! matrices, and turns bone matrices into skinning matrices and skinned vertices.
//!
//! - [`Sources`]: animation source values by name (`door_lf`, `wheel`, `reload`, ...).
//! - [`animation_transform`]: one animation's transform in one LOD, for a source value.
//! - [`pose`]: every bone of the Skeleton in model space for one LOD, composed through the
//!   bone parents, plus which bones are hidden.
//! - [`Pose::skinning`], [`skin`], [`hidden_sections`]: per-LOD-bone matrices, skinned vertex
//!   positions and normals, and the sections a pose hides.
//! - [`RtmBinding`], [`SkeletonPivots`], [`blend`], [`Pose::from_rtm_frames`], [`Pose::compose`]:
//!   RTM bone frames as the engine builds them, blending, composing them down the Skeleton
//!   ([`compose_hierarchy`]) and combining with model.cfg animations; [`reversed`] converts an
//!   RTM record into the decoded model space.
//!
//! Conventions (matrix layout, rotation signs, value mapping) follow `arma3_x64.exe`; see
//! `docs/re/model-animations.md`. All matrices act on column vectors in the engine's model
//! space, like `glam`.

mod eval;
mod pose;
mod skeletal;
mod sources;

pub use eval::{AnimTransform, animation_transform, interpolate};
pub use pose::{Pose, SkinnedVertices, hidden_sections, pose, skin};
pub use skeletal::{RtmBinding, SkeletonPivots, blend, compose_hierarchy, reversed};
pub use sources::Sources;
