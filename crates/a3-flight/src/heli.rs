//! The engine's basic helicopter flight model (`HelicopterAutoEPE::Simulate`, `0x140db4a30`):
//! what `helicopterrtd` helicopters fly with when the advanced flight model is off, the
//! default. Formulas and constants: `docs/re/sim-air.md` §2.

use std::f64::consts::PI;

use a3_config::ConfigRef;
use a3_vehicles::value;
use glam::DVec3;

use crate::body::{Loads, RigidBody, integrate};
use crate::ground::{Environment, Touch, contacts, settle};
use crate::input::{Arbiter, FlightInput};
use crate::{Airframe, KMH, altitude_factor, approach, sign, table};

/// What a `CfgVehicles` helicopter class says about how it flies (`HelicopterType`, loader
/// `0x140da39c0`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeliType {
    /// `maxSpeed`, m/s.
    pub max_speed: f64,
    /// `liftForceCoef` (default 1).
    pub lift_force_coef: f64,
    /// `cyclicAsideForceCoef` (1).
    pub cyclic_aside_force_coef: f64,
    /// `cyclicForwardForceCoef` (1).
    pub cyclic_forward_force_coef: f64,
    /// `backRotorForceCoef` (1).
    pub back_rotor_force_coef: f64,
    /// `bodyFrictionCoef` (1).
    pub body_friction_coef: f64,
    /// `altFullForce`, m ASL (1000).
    pub alt_full_force: f64,
    /// `altNoForce`, m ASL (3000).
    pub alt_no_force: f64,
    /// `startDuration`, s (20): the engine spins the rotor up in this time.
    pub start_duration: f64,
    /// `envelope[]`: lift over horizontal speed, spanning `0..1.4·maxSpeed`.
    pub envelope: Vec<f64>,
    /// `mainRotorSpeed`, `backRotorSpeed`: animation speed of the rotors.
    pub main_rotor_speed: f64,
    pub back_rotor_speed: f64,
    /// `maxMainRotorDive`, radians: a helicopter with rotor dive uses the other friction set.
    pub max_main_rotor_dive: f64,
}

impl HeliType {
    /// Reads a helicopter class (missing entries take the original's defaults).
    pub fn from_config(cfg: &ConfigRef<'_>) -> HeliType {
        let n = |name: &str, default: f64| value::number_or(&cfg.get(name), default);
        let start = n("startDuration", 20.0);
        HeliType {
            max_speed: n("maxSpeed", 0.0) * KMH,
            lift_force_coef: n("liftForceCoef", 1.0),
            cyclic_aside_force_coef: n("cyclicAsideForceCoef", 1.0),
            cyclic_forward_force_coef: n("cyclicForwardForceCoef", 1.0),
            back_rotor_force_coef: n("backRotorForceCoef", 1.0),
            body_friction_coef: n("bodyFrictionCoef", 1.0),
            alt_full_force: n("altFullForce", 1000.0),
            alt_no_force: n("altNoForce", 3000.0),
            start_duration: if start > 0.0 { start } else { 20.0 },
            envelope: value::numbers(&cfg.get("envelope")),
            main_rotor_speed: n("mainRotorSpeed", 0.0),
            back_rotor_speed: n("backRotorSpeed", 0.0),
            max_main_rotor_dive: n("maxMainRotorDive", 0.0).to_radians(),
        }
    }

    /// Whether the main rotor tilts (the other body-friction set).
    pub fn has_rotor_dive(&self) -> bool {
        self.max_main_rotor_dive > 0.01
    }
}

/// The controls the pilot wants (the entity's `+0x1350..+0x1360`); [`HeliState`] moves the
/// actual controls towards them at the original's rates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeliControls {
    /// Collective, −1..1 (used while [`Self::climb`] is `None`).
    pub collective: f64,
    /// A vertical speed to hold, m/s: the digital collective keys ask for ±10 m/s, and 0 when
    /// released. The step turns it into [`Self::collective`].
    pub climb: Option<f64>,
    /// Cyclic forward (nose down), unclamped.
    pub cyclic_forward: f64,
    /// Cyclic aside (left), unclamped.
    pub cyclic_aside: f64,
    /// Pedal, left positive, −1..1.
    pub rudder: f64,
    /// Main rotor dive, −1..1.
    pub rotor_dive: f64,
}

/// The hit point damage the model reads, 0..1 each, and whether the helicopter is destroyed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeliDamage {
    /// `HitHRotor`.
    pub main_rotor: f64,
    /// `HitVRotor`.
    pub tail_rotor: f64,
    /// `HitEngine`.
    pub engine: f64,
    /// Destroyed (the entity's `+0x5e4` bit 0).
    pub destroyed: bool,
}

impl HeliDamage {
    /// The tail-rotor loss factor `min(1, 1.3·d)⁴` (`0x140d9f770`).
    pub fn tail_loss(&self) -> f64 {
        (self.tail_rotor * 1.3).min(1.0).powi(4)
    }
}

/// A helicopter's flight state (the frame's `+0x1f0..+0x214` and the entity's controls).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeliState {
    /// The engine is running (throttle wanted 1, `EngineOn`/`EngineOff`).
    pub engine_on: bool,
    /// Rotor speed, 0..1.
    pub rotor: f64,
    /// Collective, −0.2..rotor.
    pub collective: f64,
    /// Cyclic forward, −1..1.
    pub cyclic_forward: f64,
    /// Cyclic aside, −1..1.
    pub cyclic_aside: f64,
    /// Pedal, −1..1.
    pub pedal: f64,
    /// Main rotor dive, radians, −1..1.
    pub rotor_dive: f64,
    /// Main rotor angle, rad (wraps at 60π).
    pub main_rotor_angle: f64,
    /// Tail rotor angle, rad.
    pub tail_rotor_angle: f64,
    /// Collective lever position, 0..1 (animation).
    pub collective_lever: f64,
    /// What the pilot asks for.
    pub controls: HeliControls,
    /// The damage the model reads.
    pub damage: HeliDamage,
    /// What the airframe touched in the last step.
    pub touch: Touch,
    /// World acceleration over the last step.
    pub acceleration: DVec3,
    previous_velocity: Option<DVec3>,
    collective_input: Arbiter,
}

impl HeliState {
    /// A helicopter standing with its engine off.
    pub fn new() -> HeliState {
        HeliState::default()
    }

    /// A helicopter flying: engine on, rotor at full speed, holding its altitude.
    pub fn flying() -> HeliState {
        HeliState {
            engine_on: true,
            rotor: 1.0,
            controls: HeliControls {
                climb: Some(0.0),
                ..HeliControls::default()
            },
            ..HeliState::default()
        }
    }

    /// The player's user actions to wanted controls (`0x140da2c50`, `docs/re/sim-air.md` §2.3).
    pub fn pilot(&mut self, input: &FlightInput, body: &RigidBody) {
        let digital = input.heli_collective_raise - input.heli_collective_lower;
        let analog = input.heli_collective_raise_cont - input.heli_collective_lower_cont;
        let change = self.collective_input.update(digital, analog);
        if (digital > 0.0 || change > 0.0) && !self.engine_on {
            self.engine_on = true;
        }
        if self.collective_input.digital_in_charge {
            self.controls.climb = Some(10.0 * digital);
        } else {
            self.controls.climb = None;
            self.controls.collective = analog;
            self.collective_lever = analog;
        }

        let side = 3.0 * (input.heli_left - input.heli_right);
        let speed = body.model_speed().z.abs();
        let fast = ((speed - 4.0).max(0.0) * 0.04).clamp(0.0, 1.0);
        let mut bank = (body.aside().y * 6.0).min(1.0);
        if bank * side > 0.0 {
            bank = 0.0;
        }
        let k = bank.abs().max(fast);
        self.controls.rudder =
            ((1.0 - k) * side + input.heli_rudder_left - input.heli_rudder_right).clamp(-1.0, 1.0);
        self.controls.cyclic_aside =
            3.0 * (input.heli_cyclic_left - input.heli_cyclic_right) + k * side;
        self.controls.cyclic_forward = 3.0 * (input.heli_cyclic_forward - input.heli_cyclic_back);
    }

    /// The controls with nobody at them (`0x140db5810`): settled to the ground's rest position
    /// in the air, centred on land.
    pub fn no_pilot(&mut self) {
        if self.touch.land {
            self.controls = HeliControls::default();
        } else {
            self.controls = HeliControls {
                collective: 0.1,
                climb: None,
                cyclic_forward: 0.1,
                cyclic_aside: -0.1,
                rudder: -0.1,
                rotor_dive: 0.0,
            };
        }
    }

    /// The digital collective's vertical-speed hold (`0x140db5810`): collective wanted from
    /// the wanted climb rate, the current vertical speed and acceleration.
    fn hold_climb(&mut self, body: &RigidBody) {
        let Some(climb) = self.controls.climb else {
            return;
        };
        let wanted =
            (climb - (self.acceleration.y * 0.5 + body.velocity.y)) * 0.1 + self.collective;
        let up = body.up().y;
        let upright = if up < -0.2 {
            -1.0
        } else if up <= 0.0 {
            (up + 0.2) * 10.0 - 1.0
        } else {
            1.0
        };
        self.controls.collective = (upright * wanted).clamp(-1.0, 1.0);
    }

    /// Control surfaces' smoothing (`0x140db7990`).
    fn smooth_cyclic(&mut self, dt: f64) {
        let aside = self.controls.cyclic_aside.clamp(-2.0, 2.0);
        self.cyclic_aside = approach(self.cyclic_aside, aside, 4.0 * dt, 4.0 * dt).clamp(-1.0, 1.0);
        self.cyclic_forward = approach(
            self.cyclic_forward,
            self.controls.cyclic_forward,
            10.0 * dt,
            10.0 * dt,
        )
        .clamp(-1.0, 1.0);
        let dive = self.controls.rotor_dive.clamp(-1.0, 1.0);
        self.rotor_dive = approach(self.rotor_dive, dive, 0.15 * dt, 0.15 * dt).clamp(-1.0, 1.0);
    }

    /// Rotor speed, collective and pedal (`0x140db75c0`).
    fn spool(&mut self, ty: &HeliType, model_speed: DVec3, dt: f64) {
        let d = &self.damage;
        if d.main_rotor > 0.95 {
            self.engine_on = false;
        }
        let throttle_cap = 1.0 - d.main_rotor.max(d.engine);
        let on: f64 = if self.engine_on { 1.0 } else { 0.0 };
        let throttle = on.min(throttle_cap);
        if d.destroyed {
            self.rotor -= 0.2 * dt;
        } else {
            let mut step = throttle - self.rotor;
            if d.main_rotor > 0.95 {
                self.rotor = 0.0;
                step = 0.0;
            }
            let collective = (self.collective * 0.2).max(0.0);
            let autorotation = (model_speed.y * -0.125 - 0.25).clamp(0.0, 1.0);
            if autorotation > 0.0 && !self.touch.land && throttle <= 0.1 {
                step = (autorotation - self.rotor) * 0.04 * dt;
            }
            let down = (0.025 + collective * 0.37) * dt;
            let up = dt / ty.start_duration;
            self.rotor += step.clamp(-down, up);
        }
        self.rotor = self.rotor.clamp(0.0, 1.0);
        let wanted = self.controls.collective.min(self.rotor);
        self.collective = approach(self.collective, wanted, 0.25 * dt, 0.25 * dt)
            .max(-0.2)
            .min(self.rotor);
        self.pedal =
            approach(self.pedal, self.controls.rudder, 10.0 * dt, 10.0 * dt).clamp(-1.0, 1.0);
    }

    /// The rotor angles for animation (in `Simulate`, before the spool).
    fn turn_rotors(&mut self, dt: f64) {
        let step = dt * self.rotor * 20.0;
        self.tail_rotor_angle += (1.0 - self.damage.tail_loss()) * self.rotor * dt * 20.0;
        self.main_rotor_angle = (self.main_rotor_angle + step).rem_euclid(60.0 * PI);
    }
}

/// The forces and torques of one step, model space, before the friction (§2.5).
pub fn forces(
    ty: &HeliType,
    frame: &Airframe,
    state: &HeliState,
    body: &RigidBody,
    env: &Environment<'_>,
    dt: f64,
) -> (DVec3, DVec3) {
    let m = frame.mass;
    let r = frame.bounding_radius;
    let rho = state.rotor;
    let c = state.collective;
    let s = body.model_speed();
    let origin = body.position;
    let alt = altitude_factor(origin.y, ty.alt_full_force, ty.alt_no_force);
    let mut force = DVec3::ZERO;
    let mut torque = DVec3::ZERO;

    // Main rotor lift (0x140db29c0, 0x140da6b00).
    let rho_lift = if state.damage.destroyed {
        rho * 0.1
    } else {
        rho
    };
    if rho_lift > 0.01 {
        let ground_effect = if r > 0.0 {
            let h = env.height_above(origin);
            (1.2 - h / (1.5 * r)).clamp(0.0, 1.0).powi(2) * 0.25
        } else {
            0.0
        };
        let a = (s.y + 3.0 - rho_lift * c * 18.0 * (ground_effect + 1.0)).max(-5.0);
        let horizontal = s.x.hypot(s.z);
        let e = table::heli_envelope(&ty.envelope, horizontal, ty.max_speed);
        let lift = (e * 4000.0 - (a.abs() * a * 400.0 + a * 6000.0)).max(-5000.0) * alt;
        let scale = ty.lift_force_coef * rho * rho * m / 3000.0;
        let lift = if state.rotor_dive.abs() > 0.001 {
            DVec3::new(
                0.0,
                lift * state.rotor_dive.cos(),
                lift * state.rotor_dive.sin(),
            )
        } else {
            DVec3::new(0.0, lift, 0.0)
        };
        force += lift * scale;
    }

    // Cyclic: the lift acts off the centre of mass.
    let lc = alt * m * rho * rho * 2.11;
    let arm_x = r * ty.cyclic_aside_force_coef * state.cyclic_aside * 0.6;
    let arm_z = r * ty.cyclic_forward_force_coef * state.cyclic_forward * -1.6;
    torque.x += -(lc * arm_z);
    torque.z += arm_x * lc;

    // No rotor in the air.
    if !state.touch.land && rho < 0.1 {
        torque.x += m * -1.3;
        torque.z += m * 0.5;
    }

    // Tail rotor (0x140db3c50).
    let loss = state.damage.tail_loss();
    let dir = body.direction();
    let flat = dir.x.hypot(dir.z);
    let bank = if flat > 0.0 {
        body.aside().y / flat
    } else {
        sign(body.aside().y)
    };
    let v = s.z.abs();
    let v2 = s.z * s.z;
    let pedal = ((1.0 - loss) * ty.back_rotor_force_coef * state.pedal * 8.0
        - (c + 0.1).max(0.05) * rho * loss * 20.0)
        * (1.0 - (v * 0.0125).min(1.0));
    let turn = bank
        * (bank
            * bank
            * (bank * (bank * v * 0.00147 + bank * v2 * 2.52e-5)
                + v * 0.003_266_666_6
                + v2 * 5.6e-5)
            + v2 * 0.000336
            + v * 0.0196);
    let tail = (pedal + turn) * rho * rho * m * r * 0.079_166_666;
    torque.y += -0.95 * r * tail;
    torque.z -= 0.0076 * r * tail;

    // Weathervane and pitch damping: a side force at the tail.
    let k = ((1.0 / 30.0) / dt).min(1.0);
    let fx =
        m * k * (-s.x * (v2 * 4.8e-6 + v * 2.8e-4) - s.x.abs() * s.x * (v2 * 4.8e-6 + v * 2.8e-4));
    let fy =
        m * k * (-s.y * (v2 * 2.4e-6 + v * 1.4e-4) - s.y.abs() * s.y * (v * 8.4e-6 + v2 * 1.44e-7));
    torque.x += 0.95 * r * fy;
    torque.y += -0.95 * r * fx;

    (force, torque)
}

/// The body friction, World space (`0x140db2c40`, `0x140d9aa60`).
pub fn body_friction(ty: &HeliType, state: &HeliState, body: &RigidBody, wind: DVec3) -> DVec3 {
    let u = body.model_speed() - body.to_model(wind);
    let q = state.rotor * state.rotor * 0.6 + 0.4;
    let fx = (u.x * u.x.abs() * 3.0 + u.x * 50.0 + 2.0 * sign(u.x)) * q;
    let (fy, fz) = if ty.has_rotor_dive() {
        (
            (u.y * u.y.abs() * 4.0 + u.y * 500.0 + 15.0 * sign(u.y)) * q,
            u.z * u.z * u.z * 0.05 + (u.z.abs() + 100.0) * u.z + 6.0 * sign(u.z),
        )
    } else {
        (
            (u.y * u.y.abs() * 3.0 + u.y * 500.0 + 2.0 * sign(u.y)) * q,
            u.z * u.z.abs() * 2.05 + u.z * 50.0 + 2.0 * sign(u.z),
        )
    };
    body.to_world(DVec3::new(fx, fy, fz) * ty.body_friction_coef)
}

/// One simulation step of a helicopter (`HelicopterAutoEPE::Simulate`).
pub fn step(
    ty: &HeliType,
    frame: &Airframe,
    state: &mut HeliState,
    body: &mut RigidBody,
    env: &Environment<'_>,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    state.acceleration = match state.previous_velocity {
        Some(previous) => (body.velocity - previous) / dt,
        None => DVec3::ZERO,
    };
    state.hold_climb(body);
    if body.angular_velocity.length_squared() >= 100.0 {
        // Spinning faster than 10 rad/s: the controls let go.
        state.controls = HeliControls {
            collective: 0.3,
            ..HeliControls::default()
        };
    }
    state.turn_rotors(dt);
    state.smooth_cyclic(dt);
    state.spool(ty, body.model_speed(), dt);

    let (force, torque) = forces(ty, frame, state, body, env, dt);
    let mut loads = Loads {
        force: body.to_world(force),
        torque: body.to_world(torque),
        friction: body_friction(ty, state, body, env.wind),
        angular_friction: DVec3::ZERO,
    };
    let s = body.model_speed();
    let damping = (s.z.abs() * 0.014 + s.z * s.z * 0.00024 + 2.5) * (state.rotor + 0.2);
    loads.angular_friction = body.angular_momentum(frame) * damping;
    state.touch = contacts(body, frame, env, &mut loads);

    integrate(body, frame, &loads, env.gravity, dt);
    settle(body, frame, env.ground);
    state.previous_velocity = Some(body.velocity);

    if state.controls.climb.is_some() {
        // The original's limiter, in its order (the rate may be negative).
        let mut wanted = (state.controls.collective + 1.0) * 0.5 - state.collective_lever;
        let rate = dt * 0.5 * wanted + dt * 0.2;
        if wanted <= -rate {
            wanted = -rate;
        }
        if rate <= wanted {
            wanted = rate;
        }
        state.collective_lever += wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::FlatGround;

    /// The Hummingbird's flight entries (`B_Heli_Light_01_F`).
    fn hummingbird() -> HeliType {
        HeliType {
            max_speed: 245.0 * KMH,
            lift_force_coef: 1.5,
            cyclic_aside_force_coef: 1.3,
            cyclic_forward_force_coef: 1.0,
            back_rotor_force_coef: 1.0,
            body_friction_coef: 0.3,
            alt_full_force: 2000.0,
            alt_no_force: 6000.0,
            start_duration: 20.0,
            envelope: vec![
                0.0, 0.2, 0.9, 2.1, 2.5, 3.3, 3.5, 3.6, 3.7, 3.8, 3.8, 3.0, 0.9, 0.7, 0.5,
            ],
            main_rotor_speed: 1.0,
            back_rotor_speed: 1.5,
            max_main_rotor_dive: 0.0,
        }
    }

    fn airframe() -> Airframe {
        let mut frame = Airframe::uniform_box(1820.0, DVec3::new(2.0, 2.0, 8.0));
        frame.bounding_radius = 6.386;
        frame
    }

    #[test]
    fn hover_lift_equals_weight_at_the_documented_collective() {
        // 400·b² + 6000·b = 9.8066·3000/1.5 with b = 18c − 3.
        let ty = hummingbird();
        let frame = airframe();
        let ground = FlatGround::new(-1000.0);
        let env = Environment::calm(&ground);
        let body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        let target = 9.8066 * 3000.0 / 1.5;
        let b = (-6000.0 + (6000.0f64.powi(2) + 1600.0 * target).sqrt()) / 800.0;
        let mut state = HeliState::flying();
        state.collective = (b + 3.0) / 18.0;
        let (force, _) = forces(&ty, &frame, &state, &body, &env, 1.0 / 15.0);
        assert!((force.y - frame.mass * 9.8066).abs() < 1e-6, "{force:?}");
        assert!(
            (state.collective - 0.3196).abs() < 1e-3,
            "{}",
            state.collective
        );
    }

    #[test]
    fn the_rotor_spins_up_in_start_duration() {
        let ty = hummingbird();
        let mut state = HeliState::new();
        state.engine_on = true;
        let dt = 1.0 / 15.0;
        for _ in 0..(15 * 10) {
            state.spool(&ty, DVec3::ZERO, dt);
        }
        assert!((state.rotor - 0.5).abs() < 1e-9, "{}", state.rotor);
        for _ in 0..(15 * 11) {
            state.spool(&ty, DVec3::ZERO, dt);
        }
        assert_eq!(state.rotor, 1.0);
    }

    #[test]
    fn a_descent_keeps_the_rotor_turning_with_the_engine_off() {
        let ty = hummingbird();
        let mut state = HeliState::new();
        state.rotor = 0.2;
        // Sinking at 10 m/s: the airflow drives the rotor towards 1.
        for _ in 0..150 {
            state.spool(&ty, DVec3::new(0.0, -10.0, 0.0), 1.0 / 15.0);
        }
        assert!(state.rotor > 0.2, "{}", state.rotor);
        let mut idle = HeliState::new();
        idle.rotor = 0.2;
        for _ in 0..150 {
            idle.spool(&ty, DVec3::ZERO, 1.0 / 15.0);
        }
        assert!(idle.rotor < 0.2);
    }

    #[test]
    fn the_collective_follows_at_a_quarter_per_second_and_never_passes_the_rotor() {
        let ty = hummingbird();
        let mut state = HeliState::flying();
        state.controls.collective = 1.0;
        state.spool(&ty, DVec3::ZERO, 1.0);
        assert!((state.collective - 0.25).abs() < 1e-12);
        state.rotor = 0.1;
        state.engine_on = false;
        state.spool(&ty, DVec3::ZERO, 1.0);
        assert!(state.collective <= state.rotor);
    }

    #[test]
    fn forward_cyclic_pitches_the_nose_down_and_left_cyclic_rolls_left() {
        let ty = hummingbird();
        let frame = airframe();
        let ground = FlatGround::new(-1000.0);
        let env = Environment::calm(&ground);
        let body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        let mut state = HeliState::flying();
        state.cyclic_forward = 1.0;
        let (_, t) = forces(&ty, &frame, &state, &body, &env, 1.0 / 15.0);
        // Nose down is +X in the engine's frame: the forward axis turns towards −Y.
        assert!(t.x > 0.0);
        state.cyclic_forward = 0.0;
        state.cyclic_aside = 1.0;
        let (_, t) = forces(&ty, &frame, &state, &body, &env, 1.0 / 15.0);
        assert!(t.z > 0.0);
    }

    #[test]
    fn left_pedal_yaws_left() {
        let ty = hummingbird();
        let frame = airframe();
        let ground = FlatGround::new(-1000.0);
        let env = Environment::calm(&ground);
        let body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        let mut state = HeliState::flying();
        state.pedal = 1.0;
        let (_, t) = forces(&ty, &frame, &state, &body, &env, 1.0 / 15.0);
        // Yaw left: the forward axis turns towards −X, a negative rotation about Y.
        assert!(t.y < 0.0);
    }

    #[test]
    fn a_shot_off_tail_rotor_spins_the_helicopter_against_the_main_rotor() {
        let ty = hummingbird();
        let frame = airframe();
        let ground = FlatGround::new(-1000.0);
        let env = Environment::calm(&ground);
        let body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        let mut state = HeliState::flying();
        state.collective = 0.3;
        state.damage.tail_rotor = 1.0;
        let (_, t) = forces(&ty, &frame, &state, &body, &env, 1.0 / 15.0);
        assert!(t.y > 0.0, "{t:?}");
    }

    #[test]
    fn body_friction_matches_the_formula() {
        let ty = hummingbird();
        let mut state = HeliState::flying();
        state.rotor = 1.0;
        let body = RigidBody {
            velocity: DVec3::new(0.0, 0.0, 10.0),
            ..RigidBody::default()
        };
        let f = body_friction(&ty, &state, &body, DVec3::ZERO);
        assert!((f.z - 0.3 * (100.0 * 2.05 + 500.0 + 2.0)).abs() < 1e-9);
    }

    #[test]
    fn released_collective_keys_hold_altitude() {
        let ty = hummingbird();
        let frame = airframe();
        let ground = FlatGround::new(0.0);
        let env = Environment::calm(&ground);
        let mut body = RigidBody::at(DVec3::new(0.0, 100.0, 0.0), 0.0);
        let mut state = HeliState::flying();
        state.collective = 0.3;
        let input = FlightInput::default();
        let dt = 1.0 / 15.0;
        for _ in 0..(15 * 30) {
            state.pilot(&input, &body);
            step(&ty, &frame, &mut state, &mut body, &env, dt);
        }
        assert!(body.velocity.y.abs() < 0.2, "{:?}", body.velocity);
        assert!(
            (body.position.y - 100.0).abs() < 15.0,
            "{:?}",
            body.position
        );
    }
}
