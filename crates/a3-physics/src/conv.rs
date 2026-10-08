//! Conversions between the workspace's `glam` and rapier's (`glamx`, an older `glam`).

use glam::{DQuat, DVec3};
use rapier3d_f64::math::{Pose, Rotation, Vector};

pub(crate) fn vec(v: DVec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

pub(crate) fn dvec(v: Vector) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

pub(crate) fn rot(q: DQuat) -> Rotation {
    Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

pub(crate) fn dquat(q: Rotation) -> DQuat {
    DQuat::from_xyzw(q.x, q.y, q.z, q.w)
}

pub(crate) fn pose(position: DVec3, orientation: DQuat) -> Pose {
    Pose::from_parts(vec(position), rot(orientation))
}
