//! The surface below an aircraft, as the host answers it, and the contact forces of the
//! airframe's Geometry points against it (`docs/re/sim-air.md` §2.6).
//!
//! The original's PhysX vehicles collide in PhysX; its legacy (non-PhysX) helicopter applies the
//! engine's own per-point contact forces, which are what this module implements. After the
//! step, [`settle`] stands in for PhysX's guarantee that a body never sinks into the ground.

use glam::DVec3;

use crate::{Airframe, Loads, RigidBody, sign};

/// What the World says about the surface below a point.
pub trait Ground {
    /// The surface height (terrain, roads, roadways) under World `x`, `z`.
    fn height(&self, x: f64, z: f64) -> f64;

    /// The surface normal there (unit, pointing up).
    fn normal(&self, x: f64, z: f64) -> DVec3 {
        let e = 0.5;
        let dx = (self.height(x + e, z) - self.height(x - e, z)) / (2.0 * e);
        let dz = (self.height(x, z + e) - self.height(x, z - e)) / (2.0 * e);
        DVec3::new(-dx, 1.0, -dz).normalize()
    }

    /// The water surface height under `x`, `z`; `f64::NEG_INFINITY` where there is none.
    fn water_level(&self, x: f64, z: f64) -> f64 {
        let _ = (x, z);
        f64::NEG_INFINITY
    }
}

/// A level plane at `height`, optionally under water: what the tests fly over, and the stand-in
/// when the World has no terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatGround {
    pub height: f64,
    pub water: Option<f64>,
}

impl FlatGround {
    pub fn new(height: f64) -> FlatGround {
        FlatGround {
            height,
            water: None,
        }
    }
}

impl Ground for FlatGround {
    fn height(&self, _x: f64, _z: f64) -> f64 {
        self.height
    }

    fn normal(&self, _x: f64, _z: f64) -> DVec3 {
        DVec3::Y
    }

    fn water_level(&self, _x: f64, _z: f64) -> f64 {
        self.water.unwrap_or(f64::NEG_INFINITY)
    }
}

/// Everything outside the aircraft a step reads.
pub struct Environment<'a> {
    /// The surface below.
    pub ground: &'a dyn Ground,
    /// The wind, World space, m/s (the landscape's wind plus any gust).
    pub wind: DVec3,
    /// Gravity, World space, m/s².
    pub gravity: DVec3,
}

impl<'a> Environment<'a> {
    /// Still air and standard gravity over `ground`.
    pub fn calm(ground: &'a dyn Ground) -> Environment<'a> {
        Environment {
            ground,
            wind: DVec3::ZERO,
            gravity: DVec3::new(0.0, -crate::GRAVITY, 0.0),
        }
    }

    /// The height of the surface below a World point, and the point's height above it.
    pub fn height_above(&self, at: DVec3) -> f64 {
        at.y - self.ground.height(at.x, at.z)
    }
}

/// Which surfaces the airframe touched in a step (the original's `+0x452` bits 1 and 2).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Touch {
    /// Touching land.
    pub land: bool,
    /// Touching water.
    pub water: bool,
    /// The deepest point below the surface, m.
    pub depth: f64,
}

/// The original's contact count cap: at most this many points share the support.
const MAX_CONTACTS: usize = 5;

/// The legacy contact forces (`0x140daba00`, its "apply" branch) of every Geometry point below
/// the surface, added to `loads`.
///
/// _Deviation_: the original feeds the World velocity into the friction formula and then treats
/// the result as model-space; we feed the model-space velocity, which is what the formula's
/// per-axis coefficients (sideways, vertical, along the fuselage) are written for.
pub fn contacts(
    body: &RigidBody,
    airframe: &Airframe,
    env: &Environment<'_>,
    loads: &mut Loads,
) -> Touch {
    let mut touch = Touch::default();
    let mut points = Vec::new();
    for &p in &airframe.contact_points {
        let at = body.point(p);
        let land = env.ground.height(at.x, at.z);
        let water = env.ground.water_level(at.x, at.z);
        if water > land && at.y < water {
            points.push((at, water - at.y, DVec3::Y, true));
        } else if at.y < land {
            points.push((at, land - at.y, env.ground.normal(at.x, at.z), false));
        }
    }
    if points.is_empty() {
        return touch;
    }
    let n = points.len().min(MAX_CONTACTS) as f64;
    let share = 3.0 / n;
    let m = airframe.mass;
    let com = body.center_of_mass(airframe);
    let momentum = body.angular_momentum(airframe);
    let v = body.model_speed();
    for (at, depth, normal, wet) in points {
        touch.depth = touch.depth.max(depth);
        let held = if wet {
            touch.water = true;
            depth.min(3.0) * 0.001
        } else {
            touch.land = true;
            depth.min(0.1)
        };
        loads.add_force_at(normal * (m * 40.0 * share * held), at, com);

        let f = DVec3::new(
            v.x * 5000.0 + 10000.0 * sign(v.x),
            v.y * v.y.abs() * 1000.0 + v.y * 8000.0 + 10000.0 * sign(v.y),
            v.z * v.z.abs() * 150.0 + v.z * 250.0 + 5000.0 * sign(v.z),
        );
        let f = body.to_world(f * (m * share * 0.0001));
        loads.friction += f;
        let moving = DVec3::new(
            if v.x.abs() >= 1.0 { 1.0 } else { 0.0 },
            if v.y.abs() >= 1.0 { 1.0 } else { 0.0 },
            if v.z.abs() >= 1.0 { 1.0 } else { 0.0 },
        );
        let f_torque = body.to_world(body.to_model(f) * moving);
        loads.torque -= (at - com).cross(f_torque);
        loads.angular_friction += momentum * 15.0 * depth;
    }
    touch
}

/// Keeps the body out of the ground after a step: lifts it so no Geometry point is deeper than
/// the depth the contact forces saturate at (0.1 m), and removes the velocity into the surface.
/// _Ours_: PhysX's contact solver does this in the original.
pub fn settle(body: &mut RigidBody, airframe: &Airframe, ground: &dyn Ground) {
    let mut deepest = 0.0_f64;
    let mut normal = DVec3::Y;
    for &p in &airframe.contact_points {
        let at = body.point(p);
        let depth = ground.height(at.x, at.z) - at.y;
        if depth > deepest {
            deepest = depth;
            normal = ground.normal(at.x, at.z);
        }
    }
    let excess = deepest - 0.1;
    if excess > 0.0 {
        body.position.y += excess;
        let into = body.velocity.dot(normal);
        if into < 0.0 {
            body.velocity -= normal * into;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::integrate;

    #[test]
    fn a_box_settles_on_its_contacts_at_the_spring_depth() {
        // Four bottom corners touch: each holds m·40·(3/4)·d, so all four hold the weight at
        // d = g/120.
        let frame = crate::Airframe::uniform_box(1000.0, DVec3::new(2.0, 1.0, 4.0));
        let ground = FlatGround::new(0.0);
        let env = Environment::calm(&ground);
        let mut body = RigidBody::at(DVec3::new(0.0, 0.5, 0.0), 0.0);
        for _ in 0..600 {
            let mut loads = Loads::default();
            contacts(&body, &frame, &env, &mut loads);
            integrate(&mut body, &frame, &loads, env.gravity, 1.0 / 15.0);
            settle(&mut body, &frame, &ground);
        }
        let depth = -(body.position.y - 0.5);
        assert!((depth - crate::GRAVITY / 120.0).abs() < 0.005, "{depth}");
        assert!(body.velocity.length() < 0.05, "{:?}", body.velocity);
    }

    #[test]
    fn the_normal_of_a_slope_leans_downhill() {
        struct Slope;
        impl Ground for Slope {
            fn height(&self, x: f64, _z: f64) -> f64 {
                x
            }
        }
        let n = Slope.normal(0.0, 0.0);
        assert!(n.x < 0.0 && n.y > 0.0);
    }
}
