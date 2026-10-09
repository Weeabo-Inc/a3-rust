//! The aircraft's rigid body and how both flight models integrate it
//! (`docs/re/sim-air.md` §4).
//!
//! The original accumulates a force, a torque, a friction force and an angular friction, runs
//! them through its friction integrator (`0x140e845f0`: friction never reverses a velocity) and
//! hands the resulting change of velocity to PhysX, which adds gravity and moves the body. Here
//! the body integrates itself with the same rule, semi-implicitly, as the PhysX step does.

use glam::{DMat3, DQuat, DVec3};

use crate::Airframe;

/// Where an aircraft is and how it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBody {
    /// The model origin, World space.
    pub position: DVec3,
    /// Model to World rotation.
    pub orientation: DQuat,
    /// Velocity of the centre of mass, World space, m/s.
    pub velocity: DVec3,
    /// Angular velocity, World space, rad/s.
    pub angular_velocity: DVec3,
}

impl Default for RigidBody {
    fn default() -> Self {
        RigidBody {
            position: DVec3::ZERO,
            orientation: DQuat::IDENTITY,
            velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
        }
    }
}

impl RigidBody {
    /// A body at rest at `position`, heading `yaw` radians clockwise from north (+Z).
    pub fn at(position: DVec3, yaw: f64) -> RigidBody {
        RigidBody {
            position,
            orientation: DQuat::from_rotation_y(yaw),
            ..RigidBody::default()
        }
    }

    /// The model's right axis (`aside`, the frame's first column), World space.
    pub fn aside(&self) -> DVec3 {
        self.orientation * DVec3::X
    }

    /// The model's up axis, World space.
    pub fn up(&self) -> DVec3 {
        self.orientation * DVec3::Y
    }

    /// The model's forward axis (`direction`), World space.
    pub fn direction(&self) -> DVec3 {
        self.orientation * DVec3::Z
    }

    /// A World vector in model space.
    pub fn to_model(&self, v: DVec3) -> DVec3 {
        self.orientation.inverse() * v
    }

    /// A model vector in World space.
    pub fn to_world(&self, v: DVec3) -> DVec3 {
        self.orientation * v
    }

    /// A model-space point in World space.
    pub fn point(&self, p: DVec3) -> DVec3 {
        self.position + self.orientation * p
    }

    /// The velocity in model space: the engine's "model speed" (the frame's `+0x60`); `.z` is
    /// forward speed.
    pub fn model_speed(&self) -> DVec3 {
        self.to_model(self.velocity)
    }

    /// The centre of mass, World space.
    pub fn center_of_mass(&self, airframe: &Airframe) -> DVec3 {
        self.point(airframe.center_of_mass)
    }

    /// The inertia tensor in World space, `R·I·Rᵀ`.
    pub fn world_inertia(&self, airframe: &Airframe) -> DMat3 {
        let r = DMat3::from_quat(self.orientation);
        r * airframe.inertia * r.transpose()
    }

    /// The angular momentum, World space: `R·I·Rᵀ·ω` (the entity's `+0x2cc`).
    pub fn angular_momentum(&self, airframe: &Airframe) -> DVec3 {
        self.world_inertia(airframe) * self.angular_velocity
    }

    /// The velocity of a World point attached to the body.
    pub fn velocity_at(&self, airframe: &Airframe, point: DVec3) -> DVec3 {
        self.velocity
            + self
                .angular_velocity
                .cross(point - self.center_of_mass(airframe))
    }
}

/// What a flight model asks of the body this step, World space: a force and a torque, and the
/// friction force and angular friction that slow it without reversing it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Loads {
    /// Force, N.
    pub force: DVec3,
    /// Torque about the centre of mass, N·m.
    pub torque: DVec3,
    /// Friction force, N, applied per model axis against the velocity (§4).
    pub friction: DVec3,
    /// Angular friction, N·m, applied per World axis against the angular momentum.
    pub angular_friction: DVec3,
}

impl Loads {
    /// Adds `force` acting at the World point `at` (a torque about the centre `com`).
    pub fn add_force_at(&mut self, force: DVec3, at: DVec3, com: DVec3) {
        self.force += force;
        self.torque += (at - com).cross(force);
    }
}

/// The friction integrator (`0x140e845f0`), per axis: `v += a·dt`, then the friction `μ` slows
/// `v` by `μ·dt` if it points the same way, stopping at 0 rather than reversing it.
pub fn apply_friction(v: DVec3, friction: DVec3, accel: DVec3, dt: f64) -> DVec3 {
    let axis = |v: f64, mu: f64, a: f64| {
        let v = v + a * dt;
        if v * mu > 0.0 {
            let step = mu * dt;
            if v.abs() <= step.abs() { 0.0 } else { v - step }
        } else {
            v
        }
    };
    DVec3::new(
        axis(v.x, friction.x, accel.x),
        axis(v.y, friction.y, accel.y),
        axis(v.z, friction.z, accel.z),
    )
}

/// One step of the body under `loads` and `gravity` (m/s², World space).
///
/// Linear: the force, gravity and friction act in model space through [`apply_friction`];
/// angular: the torque and angular friction act on the World angular momentum, and the angular
/// velocity is `I⁻¹·L`. Then the body moves with the new velocities (semi-implicit, as the PhysX
/// step that the original hands the velocity change to).
///
/// _Deviation_: PhysX adds gravity after the engine's friction step. We add it with the forces,
/// as the engine's legacy (non-PhysX) vehicles did, because our ground contact is that legacy
/// model (`docs/re/sim-air.md` §2.6) and only settles at rest this way; in flight the two orders
/// differ only when a friction would stop a velocity within one step.
pub fn integrate(
    body: &mut RigidBody,
    airframe: &Airframe,
    loads: &Loads,
    gravity: DVec3,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    let inv_mass = 1.0 / airframe.mass.max(1e-6);
    let v_model = body.to_model(body.velocity);
    let a_model = body.to_model(loads.force * inv_mass + gravity);
    let mu_model = body.to_model(loads.friction * inv_mass);
    let v_model = apply_friction(v_model, mu_model, a_model, dt);
    let velocity = body.to_world(v_model);

    let inertia = body.world_inertia(airframe);
    let momentum = inertia * body.angular_velocity;
    let momentum = apply_friction(momentum, loads.angular_friction, loads.torque, dt);
    let angular_velocity = if inertia.determinant().abs() > 1e-12 {
        inertia.inverse() * momentum
    } else {
        DVec3::ZERO
    };

    let com_local = airframe.center_of_mass;
    let com = body.point(com_local) + velocity * dt;
    let turn = angular_velocity * dt;
    let orientation = if turn.length_squared() > 0.0 {
        (DQuat::from_scaled_axis(turn) * body.orientation).normalize()
    } else {
        body.orientation
    };
    body.orientation = orientation;
    body.position = com - orientation * com_local;
    body.velocity = velocity;
    body.angular_velocity = angular_velocity;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friction_slows_but_never_reverses() {
        let v = apply_friction(
            DVec3::new(1.0, -1.0, 5.0),
            DVec3::new(100.0, -0.5, 30.0),
            DVec3::ZERO,
            0.1,
        );
        assert_eq!(v.x, 0.0, "1 m/s against 10 m/s of friction stops");
        assert!((v.y - -0.95).abs() < 1e-12);
        assert!((v.z - 2.0).abs() < 1e-12);
    }

    #[test]
    fn friction_against_the_motion_only() {
        // Friction pointing the other way (the engine's sign test) is ignored.
        let v = apply_friction(
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(-5.0, 0.0, 0.0),
            DVec3::ZERO,
            0.1,
        );
        assert_eq!(v.x, 1.0);
    }

    #[test]
    fn the_acceleration_is_applied_before_the_friction() {
        let v = apply_friction(
            DVec3::ZERO,
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(20.0, 0.0, 0.0),
            0.1,
        );
        assert!((v.x - 1.9).abs() < 1e-12);
    }

    #[test]
    fn a_free_body_falls_with_gravity() {
        let frame = Airframe::uniform_box(1000.0, DVec3::splat(2.0));
        let mut body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        for _ in 0..15 {
            integrate(
                &mut body,
                &frame,
                &Loads::default(),
                DVec3::new(0.0, -9.8066, 0.0),
                1.0 / 15.0,
            );
        }
        assert!((body.velocity.y - -9.8066).abs() < 1e-9);
        // Semi-implicit Euler: a little more than ½·g·t².
        assert!(
            (100.0 - body.position.y - 5.23).abs() < 0.01,
            "{}",
            body.position.y
        );
    }

    #[test]
    fn a_torque_spins_the_body_about_its_centre_of_mass() {
        let mut frame = Airframe::uniform_box(12.0, DVec3::new(1.0, 1.0, 1.0));
        frame.center_of_mass = DVec3::new(0.0, 0.0, 1.0);
        let mut body = RigidBody::default();
        let loads = Loads {
            torque: DVec3::new(0.0, 2.0, 0.0),
            ..Loads::default()
        };
        integrate(&mut body, &frame, &loads, DVec3::ZERO, 0.5);
        // I = 2 about Y: ω = τ·dt/I = 0.5 rad/s.
        assert!((body.angular_velocity.y - 0.5).abs() < 1e-12);
        // The centre of mass stays put while the origin swings around it.
        assert!(
            body.center_of_mass(&frame)
                .distance(DVec3::new(0.0, 0.0, 1.0))
                < 1e-12
        );
        assert!(body.position.length() > 0.0);
    }

    #[test]
    fn a_heading_turns_north_towards_east() {
        let body = RigidBody::at(DVec3::ZERO, std::f64::consts::FRAC_PI_2);
        assert!(body.direction().distance(DVec3::X) < 1e-12);
    }
}
