//! The ground under a vehicle: what the host (the world) answers when a wheel asks about the
//! surface below it, and the state a vehicle body carries while it moves.
//!
//! The vehicle model here is pure: it never touches a collision world of its own. `a3-world`
//! implements [`GroundSurface`] over its rapier world; the tests use [`FlatSurface`].

use glam::{DQuat, DVec3};

/// What a ray cast under a wheel found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceHit {
    /// The contact point, world space.
    pub point: DVec3,
    /// The surface normal there (unit).
    pub normal: DVec3,
    /// The material's `surfaceFriction` (1.0 on a road — the bisurf value the tire's friction is
    /// scaled by, `docs/re/sim-vehicles.md` §2).
    pub friction: f64,
}

/// The world as a vehicle needs it: a ray cast per wheel, and the water level for ships.
pub trait GroundSurface {
    /// The first surface a ray from `from` along `direction` hits, within `max_distance`.
    fn cast(&self, from: DVec3, direction: DVec3, max_distance: f64) -> Option<SurfaceHit>;

    /// The water surface height at `at`, or `f64::NEG_INFINITY` where there is no water.
    fn water_level(&self, at: DVec3) -> f64 {
        let _ = at;
        f64::NEG_INFINITY
    }
}

/// A level plane at `height`, with one friction and an optional water surface just above it —
/// what the tests drive on, and the stand-in when there is no collision world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatSurface {
    /// The height of the ground.
    pub height: f64,
    /// The friction the surface gives the tires.
    pub friction: f64,
    /// The water level, if any.
    pub water_level: Option<f64>,
}

impl FlatSurface {
    /// Dry ground at `height` with `friction`.
    pub fn new(height: f64, friction: f64) -> FlatSurface {
        FlatSurface {
            height,
            friction,
            water_level: None,
        }
    }

    /// The same ground, under water.
    pub fn flooded(mut self, water_level: f64) -> FlatSurface {
        self.water_level = Some(water_level);
        self
    }
}

impl GroundSurface for FlatSurface {
    fn cast(&self, from: DVec3, direction: DVec3, max_distance: f64) -> Option<SurfaceHit> {
        if direction.y >= 0.0 {
            return None;
        }
        let distance = (from.y - self.height) / -direction.y;
        if distance < 0.0 || distance > max_distance {
            return None;
        }
        Some(SurfaceHit {
            point: from + direction * distance,
            normal: DVec3::Y,
            friction: self.friction,
        })
    }

    fn water_level(&self, _at: DVec3) -> f64 {
        self.water_level.unwrap_or(f64::NEG_INFINITY)
    }
}

/// A rigid body: where it is, which way it faces, and how it is moving. A vehicle's chassis
/// integrates itself (ADR 0009), so this is the state it carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyState {
    /// The chassis origin, world space.
    pub position: DVec3,
    /// The chassis orientation.
    pub orientation: DQuat,
    /// Linear velocity, m/s.
    pub velocity: DVec3,
    /// Angular velocity, rad/s, world space.
    pub angular_velocity: DVec3,
}

impl Default for BodyState {
    fn default() -> Self {
        BodyState {
            position: DVec3::ZERO,
            orientation: DQuat::IDENTITY,
            velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
        }
    }
}

impl BodyState {
    /// A body at `position` facing `yaw` (rad, about Y, 0 = +Z).
    pub fn new(position: DVec3, yaw: f64) -> BodyState {
        BodyState {
            position,
            orientation: DQuat::from_rotation_y(yaw),
            velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
        }
    }

    /// The body's forward direction (its local +Z), world space.
    pub fn forward(&self) -> DVec3 {
        self.orientation * DVec3::Z
    }

    /// The body's up direction (its local +Y), world space.
    pub fn up(&self) -> DVec3 {
        self.orientation * DVec3::Y
    }

    /// The body's right direction (its local +X), world space.
    pub fn right(&self) -> DVec3 {
        self.orientation * DVec3::X
    }

    /// The yaw angle, rad, from the forward direction.
    pub fn yaw(&self) -> f64 {
        let forward = self.forward();
        forward.x.atan2(forward.z)
    }

    /// A local point in world space.
    pub fn local_to_world(&self, local: DVec3) -> DVec3 {
        self.position + self.orientation * local
    }

    /// A world vector in the body's frame.
    pub fn world_to_local(&self, world: DVec3) -> DVec3 {
        self.orientation.inverse() * world
    }

    /// The velocity of the body's point at world offset `at` from the origin.
    pub fn velocity_at(&self, at: DVec3) -> DVec3 {
        self.velocity + self.angular_velocity.cross(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_surface_is_hit_straight_down_and_only_from_above() {
        let ground = FlatSurface::new(3.0, 1.0);
        let hit = ground
            .cast(DVec3::new(0.0, 10.0, 0.0), -DVec3::Y, 20.0)
            .unwrap();
        assert_eq!(hit.point, DVec3::new(0.0, 3.0, 0.0));
        assert_eq!(hit.normal, DVec3::Y);
        assert_eq!(hit.friction, 1.0);
        // Out of reach, or looking up: nothing.
        assert!(
            ground
                .cast(DVec3::new(0.0, 10.0, 0.0), -DVec3::Y, 5.0)
                .is_none()
        );
        assert!(
            ground
                .cast(DVec3::new(0.0, 10.0, 0.0), DVec3::Y, 20.0)
                .is_none()
        );
        // A tilted ray: the hit walks sideways with the tilt.
        let direction = DVec3::new(0.0, -1.0, 1.0).normalize();
        let hit = ground
            .cast(DVec3::new(0.0, 6.0, 0.0), direction, 100.0)
            .unwrap();
        assert!((hit.point.y - 3.0).abs() < 1e-9, "{:?}", hit.point);
        assert!((hit.point.z - 3.0).abs() < 1e-9, "{:?}", hit.point);
        // No water unless asked for.
        assert_eq!(ground.water_level(DVec3::ZERO), f64::NEG_INFINITY);
        assert_eq!(ground.flooded(5.0).water_level(DVec3::ZERO), 5.0);
    }

    #[test]
    fn a_body_knows_where_it_points() {
        let body = BodyState::new(DVec3::new(1.0, 2.0, 3.0), std::f64::consts::FRAC_PI_2);
        assert!((body.forward() - DVec3::new(1.0, 0.0, 0.0)).length() < 1e-9);
        assert!((body.up() - DVec3::Y).length() < 1e-9);
        assert!((body.right() - DVec3::new(0.0, 0.0, -1.0)).length() < 1e-9);
        assert!((body.yaw() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!((body.local_to_world(DVec3::Z) - DVec3::new(2.0, 2.0, 3.0)).length() < 1e-9);
        assert!(
            (body.world_to_local(body.right()) - DVec3::X).length() < 1e-9,
            "right in the body's frame is +X"
        );
        // A point away from the origin moves with the body's rotation.
        let spinning = BodyState {
            angular_velocity: DVec3::Y,
            ..BodyState::default()
        };
        assert!((spinning.velocity_at(DVec3::Z) - DVec3::new(1.0, 0.0, 0.0)).length() < 1e-9);
    }
}
