//! Perspective camera in RV world space and the free-fly debug camera.
//!
//! World space follows RV (see `docs/adr/0003-coordinates-and-precision.md`): left-handed,
//! X east, Y up, Z north, positions in `f64` metres. Rendering is camera-relative: the view
//! matrix holds only the camera's rotation and every position is turned into an `f32` offset
//! from the camera before it reaches the GPU.

use glam::{DVec3, Mat4, Vec3};

/// Field of view as RV stores it: tangents of the half angles (`fovTop`, `fovLeft` in the
/// profile). Arma's default is `fovTop = 0.75`, i.e. ~73.7 degrees vertical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fov {
    /// `tan(vertical_fov / 2)`.
    pub top: f32,
}

impl Fov {
    /// RV's default vertical field of view.
    pub const DEFAULT: Fov = Fov { top: 0.75 };

    /// Full vertical field of view in radians.
    pub fn vertical_radians(self) -> f32 {
        2.0 * self.top.atan()
    }

    /// `tan(horizontal_fov / 2)` for a viewport aspect ratio (width / height), RV's `fovLeft`.
    pub fn left(self, aspect: f32) -> f32 {
        self.top * aspect
    }
}

impl Default for Fov {
    fn default() -> Self {
        Fov::DEFAULT
    }
}

/// A perspective camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// World position in metres.
    pub position: DVec3,
    /// Heading in radians, clockwise from north (+Z) towards east (+X), like RV's `getDir`.
    pub yaw: f32,
    /// Elevation in radians, positive looking up.
    pub pitch: f32,
    pub fov: Fov,
    /// Near plane distance in metres. There is no far plane (infinite reversed-Z).
    pub near: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            position: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov: Fov::DEFAULT,
            near: 0.1,
        }
    }
}

impl Camera {
    /// Unit view direction.
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Unit vector to the camera's right, horizontal.
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, -sy)
    }

    /// Rotation-only view matrix (the camera sits at the origin of camera-relative space).
    pub fn view(&self) -> Mat4 {
        glam::camera::lh::view::look_to_mat4(Vec3::ZERO, self.forward(), Vec3::Y)
    }

    /// Infinite-far reversed-Z perspective projection: depth 1 at the near plane, approaching 0
    /// at infinity.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::lh::proj::directx::perspective_infinite_reverse(
            self.fov.vertical_radians(),
            aspect,
            self.near,
        )
    }

    /// `projection * view` for camera-relative positions.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Offset of a world position from the camera, computed in `f64` and then narrowed.
    pub fn relative(&self, world: DVec3) -> Vec3 {
        (world - self.position).as_vec3()
    }
}

/// Per-frame input to the [`FreeFlyController`], each component typically an action value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FreeFlyInput {
    /// Right (+) / left (-).
    pub strafe: f32,
    /// Up (+) / down (-), along world Y.
    pub lift: f32,
    /// Forward (+) / backward (-), along the view direction.
    pub forward: f32,
    /// Heading change, positive turns right; in look units (mouse counts or stick level).
    pub yaw: f32,
    /// Elevation change, positive looks up.
    pub pitch: f32,
    /// Speed multiplier from turbo actions (1 = normal).
    pub speed_multiplier: f32,
}

/// Free-flying debug/editor camera: WASD-style motion plus mouse look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeFlyController {
    /// Movement speed in metres per second at multiplier 1.
    pub speed: f64,
    /// Radians per look unit.
    pub look_sensitivity: f32,
}

impl Default for FreeFlyController {
    fn default() -> Self {
        FreeFlyController {
            speed: 20.0,
            look_sensitivity: 0.0025,
        }
    }
}

impl FreeFlyController {
    /// Largest pitch magnitude; keeps the view from flipping over the poles.
    pub const MAX_PITCH: f32 = 1.55;

    /// Apply one frame of input over `dt` seconds.
    pub fn update(&self, camera: &mut Camera, input: &FreeFlyInput, dt: f64) {
        camera.yaw =
            (camera.yaw + input.yaw * self.look_sensitivity).rem_euclid(std::f32::consts::TAU);
        camera.pitch = (camera.pitch + input.pitch * self.look_sensitivity)
            .clamp(-Self::MAX_PITCH, Self::MAX_PITCH);

        let direction = camera.forward().as_dvec3() * f64::from(input.forward)
            + camera.right().as_dvec3() * f64::from(input.strafe)
            + DVec3::Y * f64::from(input.lift);
        let multiplier = if input.speed_multiplier > 0.0 {
            f64::from(input.speed_multiplier)
        } else {
            1.0
        };
        camera.position += direction * self.speed * multiplier * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn ndc(camera: &Camera, world: DVec3) -> Vec3 {
        let clip = camera.view_projection(1.0) * camera.relative(world).extend(1.0);
        clip.truncate() / clip.w
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() < 1e-4
    }

    #[test]
    fn default_camera_looks_north() {
        let c = Camera::default();
        assert!(close(c.forward(), Vec3::Z));
        assert!(
            close(c.right(), Vec3::X),
            "east is to the right when facing north"
        );
    }

    #[test]
    fn yaw_is_clockwise_from_north() {
        let c = Camera {
            yaw: FRAC_PI_2,
            ..Camera::default()
        };
        assert!(close(c.forward(), Vec3::X), "yaw 90 degrees faces east");
        assert!(close(c.right(), -Vec3::Z), "and south is to the right");
    }

    #[test]
    fn point_ahead_projects_to_centre_with_reversed_depth() {
        let c = Camera::default();
        let p = ndc(&c, DVec3::new(0.0, 0.0, 100.0));
        assert!(close(p, Vec3::new(0.0, 0.0, 0.1 / 100.0)), "{p}");
        let near = ndc(&c, DVec3::new(0.0, 0.0, 0.1));
        assert!((near.z - 1.0).abs() < 1e-5, "near plane maps to depth 1");
    }

    #[test]
    fn depth_keeps_resolving_at_terrain_distances() {
        let c = Camera::default();
        let a = ndc(&c, DVec3::new(0.0, 0.0, 12_000.0)).z;
        let b = ndc(&c, DVec3::new(0.0, 0.0, 12_001.0)).z;
        assert!(a > b && b > 0.0, "{a} vs {b}");
    }

    #[test]
    fn east_and_up_appear_right_and_up_on_screen() {
        let c = Camera::default();
        let p = ndc(&c, DVec3::new(10.0, 5.0, 100.0));
        assert!(p.x > 0.0 && p.y > 0.0, "{p}");
    }

    #[test]
    fn fov_top_is_tangent_of_half_vertical_angle() {
        // A point on the top edge of the view: y / z = fovTop.
        let c = Camera::default();
        let p = ndc(&c, DVec3::new(0.0, 75.0, 100.0));
        assert!((p.y - 1.0).abs() < 1e-5, "{p}");
        assert!((Fov::DEFAULT.vertical_radians().to_degrees() - 73.74).abs() < 0.01);
    }

    #[test]
    fn camera_relative_offsets_keep_millimetres_far_from_origin() {
        let c = Camera {
            position: DVec3::new(25_000.123_4, 50.0, 25_000.567_8),
            ..Camera::default()
        };
        let r = c.relative(DVec3::new(25_000.124_4, 50.0, 25_000.567_8));
        assert!((r.x - 0.001).abs() < 1e-6, "{r}");
        // Narrowing the absolute position first would lose the millimetre.
        let lossy = 25_000.124_4f64 as f32 - 25_000.123_4f64 as f32;
        assert!((lossy - 0.001).abs() > 1e-4);
    }

    #[test]
    fn free_fly_moves_along_view_and_turbo_scales_speed() {
        let ctl = FreeFlyController {
            speed: 10.0,
            look_sensitivity: 0.01,
        };
        let mut c = Camera::default();
        let input = FreeFlyInput {
            forward: 1.0,
            speed_multiplier: 2.0,
            ..FreeFlyInput::default()
        };
        ctl.update(&mut c, &input, 0.5);
        assert!((c.position - DVec3::new(0.0, 0.0, 10.0)).length() < 1e-6);

        let strafe = FreeFlyInput {
            strafe: 1.0,
            lift: 1.0,
            ..FreeFlyInput::default()
        };
        ctl.update(&mut c, &strafe, 1.0);
        assert!((c.position - DVec3::new(10.0, 10.0, 10.0)).length() < 1e-6);
    }

    #[test]
    fn free_fly_look_wraps_yaw_and_clamps_pitch() {
        let ctl = FreeFlyController {
            speed: 1.0,
            look_sensitivity: 1.0,
        };
        let mut c = Camera::default();
        ctl.update(
            &mut c,
            &FreeFlyInput {
                yaw: -1.0,
                pitch: 10.0,
                ..FreeFlyInput::default()
            },
            0.0,
        );
        assert!((c.yaw - (std::f32::consts::TAU - 1.0)).abs() < 1e-5);
        assert_eq!(c.pitch, FreeFlyController::MAX_PITCH);
    }
}
