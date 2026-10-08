//! View frustum culling in camera-relative space.

use glam::{Mat4, Vec3, Vec4};

/// The side planes and the near plane of a camera's view volume, in camera-relative space
/// (the space [`PrepareContext::view_projection`](crate::PrepareContext) maps from). There is no
/// far plane: the projection is infinite reversed-Z; view distance is a separate decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// Plane `(n, d)`: a point `p` is inside when `n.dot(p) + d >= 0`.
    planes: [Vec4; 5],
}

impl Frustum {
    /// Extract the planes from a camera-relative view-projection matrix.
    pub fn from_view_projection(m: Mat4) -> Frustum {
        let row = |i: usize| m.row(i);
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        let normalize = |p: Vec4| p / p.truncate().length().max(f32::MIN_POSITIVE);
        Frustum {
            planes: [
                normalize(w + x),
                normalize(w - x),
                normalize(w + y),
                normalize(w - y),
                // Reversed-Z: depth 1 at the near plane, so inside means z <= w.
                normalize(w - z),
            ],
        }
    }

    /// Whether an axis-aligned box (camera-relative) may be visible. Conservative: boxes near a
    /// frustum corner can pass although they are outside.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|plane| {
            let n = plane.truncate();
            // The box corner furthest along the plane normal.
            let p = Vec3::select(n.cmpge(Vec3::ZERO), max, min);
            n.dot(p) + plane.w >= 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;

    fn frustum(camera: &Camera) -> Frustum {
        Frustum::from_view_projection(camera.view_projection(16.0 / 9.0))
    }

    fn cube(centre: Vec3, half: f32) -> (Vec3, Vec3) {
        (centre - Vec3::splat(half), centre + Vec3::splat(half))
    }

    #[test]
    fn box_ahead_is_visible_and_box_behind_is_not() {
        let f = frustum(&Camera::default());
        let (min, max) = cube(Vec3::new(0.0, 0.0, 100.0), 1.0);
        assert!(f.intersects_aabb(min, max));
        let (min, max) = cube(Vec3::new(0.0, 0.0, -100.0), 1.0);
        assert!(!f.intersects_aabb(min, max));
    }

    #[test]
    fn boxes_beside_the_view_are_culled_but_very_far_ones_ahead_are_kept() {
        let f = frustum(&Camera::default());
        let (min, max) = cube(Vec3::new(-500.0, 0.0, 100.0), 1.0);
        assert!(!f.intersects_aabb(min, max), "far to the left");
        let (min, max) = cube(Vec3::new(0.0, 400.0, 100.0), 1.0);
        assert!(!f.intersects_aabb(min, max), "far above");
        let (min, max) = cube(Vec3::new(0.0, 0.0, 50_000.0), 10.0);
        assert!(f.intersects_aabb(min, max), "no far plane");
    }

    #[test]
    fn a_box_around_the_camera_is_visible() {
        let f = frustum(&Camera::default());
        let (min, max) = cube(Vec3::ZERO, 5.0);
        assert!(f.intersects_aabb(min, max));
    }

    #[test]
    fn culling_follows_the_heading() {
        // Facing east: a box to the east is visible, one to the north is not.
        let camera = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Camera::default()
        };
        let f = frustum(&camera);
        let (min, max) = cube(Vec3::new(100.0, 0.0, 0.0), 1.0);
        assert!(f.intersects_aabb(min, max));
        let (min, max) = cube(Vec3::new(0.0, 0.0, 100.0), 1.0);
        assert!(!f.intersects_aabb(min, max));
    }
}
