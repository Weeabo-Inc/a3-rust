//! The `airplanex` flight model (`AirplaneAutoEPE::Simulate`, `0x140690e40`): thrust, lift,
//! control surfaces, weathervane terms, stall, air friction, flaps, airbrake, gear, and the
//! landing-gear wheels. Formulas and constants: `docs/re/sim-air.md` §3.
//!
//! VTOL planes (`VTOL > 0`) fly here as conventional planes: their vectoring is not modelled.

use a3_config::ConfigRef;
use a3_vehicles::value;
use a3_vehicles::wheel::Wheel;
use glam::{DQuat, DVec3};

use crate::body::{Loads, RigidBody, integrate};
use crate::ground::{Environment, Touch, settle};
use crate::input::{Arbiter, FlightInput};
use crate::{Airframe, GRAVITY, KMH, altitude_factor, approach, sign, table};

/// What a `CfgVehicles` plane class says about how it flies (`AirplaneType`, loader
/// `0x140d1df50`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaneType {
    /// `maxSpeed`, m/s.
    pub max_speed: f64,
    /// `landingSpeed`, m/s (if the entry is below 1: `max(0.33·maxSpeed, 33.33)`).
    pub landing_speed: f64,
    /// The stall speed, m/s: `stallSpeedForced` or a share of the landing speed.
    pub stall_speed: f64,
    /// `envelope[]`: lift in g at 18° effective angle of attack, over `0..1.25·maxSpeed`.
    pub envelope: Vec<f64>,
    /// `thrustCoef[]` over `0..1.5·maxSpeed`.
    pub thrust_coef: Vec<f64>,
    pub elevator_coef: Vec<f64>,
    pub aileron_coef: Vec<f64>,
    pub rudder_coef: Vec<f64>,
    /// `elevatorControlsSensitivityCoef`: how fast the elevator moves, 1/s.
    pub elevator_rate: f64,
    /// `aileronControlsSensitivityCoef`.
    pub aileron_rate: f64,
    /// `rudderControlsSensitivityCoef`.
    pub rudder_rate: f64,
    pub aileron_sensitivity: f64,
    pub elevator_sensitivity: f64,
    pub wheel_steering_sensitivity: f64,
    pub flaps_friction_coef: f64,
    /// `gearsUpFrictionCoef` (0.5): the drag of the lowered gear.
    pub gears_up_friction_coef: f64,
    /// `airBrakeFrictionCoef` (3).
    pub air_brake_friction_coef: f64,
    /// `airFrictionCoefs0/1/2[]`, per axis.
    pub air_friction: [DVec3; 3],
    /// `flaps`: the plane has flaps.
    pub flaps: bool,
    /// `airBrake`: the plane has an airbrake (else it brakes by itself at low throttle).
    pub air_brake: bool,
    /// `rudderInfluence` (cos 5°).
    pub rudder_influence: f64,
    /// `angleOfIndicence`, rad (3°).
    pub angle_of_incidence: f64,
    /// `draconicForceX/Y/ZCoef` (7.5, 1, 1).
    pub draconic_force: DVec3,
    /// `draconicTorqueXCoef` (a number or a table; missing reads 1).
    pub draconic_torque_x: Vec<f64>,
    /// `draconicTorqueYCoef`.
    pub draconic_torque_y: Vec<f64>,
    /// `throttleToThrustLogFactor` (1).
    pub throttle_log_factor: f64,
    /// `altFullForce`, `altNoForce`, m ASL (5000, 13000).
    pub alt_full_force: f64,
    pub alt_no_force: f64,
    /// `gearRetracting`.
    pub gear_retracting: bool,
    /// `1/gearDownTime`, `1/gearUpTime`.
    pub gear_down_rate: f64,
    pub gear_up_rate: f64,
    /// `VTOL`: 0 for a conventional plane.
    pub vtol: i32,
    /// `class Wheels`.
    pub wheels: Vec<Wheel>,
}

/// A number, or a table (`draconicTorqueXCoef = 0.2` or `{...}`); missing is empty.
fn number_or_table(cfg: &ConfigRef<'_>) -> Vec<f64> {
    if cfg.is_array() {
        value::numbers(cfg)
    } else {
        value::number(cfg).into_iter().collect()
    }
}

/// A `{x, y, z}` friction triple, or its default.
fn triple(cfg: &ConfigRef<'_>, default: DVec3) -> DVec3 {
    match value::numbers(cfg)[..] {
        [x, y, z] => DVec3::new(x, y, z),
        _ => default,
    }
}

impl PlaneType {
    /// Reads a plane class (missing entries take the original's defaults).
    pub fn from_config(cfg: &ConfigRef<'_>) -> PlaneType {
        let n = |name: &str, default: f64| value::number_or(&cfg.get(name), default);
        let max_speed = n("maxSpeed", 0.0) * KMH;
        let landing = n("landingSpeed", 0.0);
        let landing_speed = if landing >= 1.0 {
            landing * KMH
        } else {
            (max_speed * 0.33).max(33.333_336)
        };
        let stall_speed = match value::number(&cfg.get("stallSpeedForced")) {
            Some(s) if s >= 0.0 => s,
            _ if landing_speed <= 21.0 => landing_speed * 0.87,
            _ => landing_speed * 0.65,
        };
        let rate = |name: &str| {
            let t = n(name, 0.0);
            if t > 0.0 { 1.0 / t } else { 0.0 }
        };
        PlaneType {
            max_speed,
            landing_speed,
            stall_speed,
            envelope: value::numbers(&cfg.get("envelope")),
            thrust_coef: value::numbers(&cfg.get("thrustCoef")),
            elevator_coef: value::numbers(&cfg.get("elevatorCoef")),
            aileron_coef: value::numbers(&cfg.get("aileronCoef")),
            rudder_coef: value::numbers(&cfg.get("rudderCoef")),
            elevator_rate: n("elevatorControlsSensitivityCoef", 0.0),
            aileron_rate: n("aileronControlsSensitivityCoef", 0.0),
            rudder_rate: n("rudderControlsSensitivityCoef", 0.0),
            aileron_sensitivity: n("aileronSensitivity", 0.0),
            elevator_sensitivity: n("elevatorSensitivity", 0.0),
            wheel_steering_sensitivity: n("wheelSteeringSensitivity", 0.0),
            flaps_friction_coef: n("flapsFrictionCoef", 0.0),
            gears_up_friction_coef: n("gearsUpFrictionCoef", 0.5),
            air_brake_friction_coef: n("airBrakeFrictionCoef", 3.0),
            air_friction: [
                triple(&cfg.get("airFrictionCoefs0"), DVec3::ZERO),
                triple(&cfg.get("airFrictionCoefs1"), DVec3::new(0.1, 0.05, 0.006)),
                triple(
                    &cfg.get("airFrictionCoefs2"),
                    DVec3::new(0.001, 0.0005, 6e-5),
                ),
            ],
            flaps: n("flaps", 0.0) != 0.0,
            air_brake: n("airBrake", 0.0) != 0.0,
            rudder_influence: n("rudderInfluence", 0.996_194_7),
            angle_of_incidence: n("angleOfIndicence", 0.052_359_88),
            draconic_force: DVec3::new(
                n("draconicForceXCoef", 7.5),
                n("draconicForceYCoef", 1.0),
                n("draconicForceZCoef", 1.0),
            ),
            draconic_torque_x: number_or_table(&cfg.get("draconicTorqueXCoef")),
            draconic_torque_y: number_or_table(&cfg.get("draconicTorqueYCoef")),
            throttle_log_factor: n("throttleToThrustLogFactor", 1.0),
            alt_full_force: n("altFullForce", 5000.0),
            alt_no_force: n("altNoForce", 13000.0),
            gear_retracting: n("gearRetracting", 0.0) != 0.0,
            gear_down_rate: rate("gearDownTime"),
            gear_up_rate: rate("gearUpTime"),
            vtol: n("VTOL", 0.0) as i32,
            wheels: Wheel::all_from_config(&cfg.get("Wheels")),
        }
    }

    /// Places the wheels from the model's memory points (`center`, `boundary`).
    pub fn place_wheels(&mut self, airframe: &Airframe) {
        for wheel in &mut self.wheels {
            if let (Some(center), Some(boundary)) = (
                airframe.memory_point(&wheel.center_point),
                airframe.memory_point(&wheel.boundary_point),
            ) {
                wheel.position = center;
                wheel.radius = (boundary - center).length().max(0.05);
            }
        }
    }

    /// A coefficient table at forward speed `speed`.
    fn coef(&self, values: &[f64], speed: f64) -> f64 {
        table::plane_coef(values, speed, self.max_speed, 1.0)
    }
}

/// The controls the pilot wants (the entity's `+0x13b4..+0x13c0`, `+0x1384`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlaneControls {
    /// Elevator, nose down positive (the keys ask for ±4.5; the surface clamps at ±1).
    pub elevator: f64,
    /// Aileron, roll left positive.
    pub aileron: f64,
    /// Rudder, yaw left positive.
    pub rudder: f64,
    /// Airbrake and wheel brake, 0..1.
    pub brake: f64,
}

/// The damage the plane model reads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlaneDamage {
    /// Mean damage of the engine hit points, 0..1.
    pub engine: f64,
    /// Destroyed.
    pub destroyed: bool,
}

/// A plane's flight state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaneState {
    /// The engine is switched on.
    pub engine_on: bool,
    /// Engine spool, 0..1 (±0.1/s).
    pub engine: f64,
    /// Thrust, 0..1, following the throttle.
    pub thrust: f64,
    /// Throttle wanted, 0..1.
    pub throttle: f64,
    /// Control surfaces, −1..1.
    pub elevator: f64,
    pub aileron: f64,
    pub rudder: f64,
    /// Elevator trim, 0..0.1 (set by the auto-trim).
    pub trim: f64,
    /// Flaps, 0..1, and the selected position (0, 1, 2).
    pub flaps: f64,
    pub flaps_position: u8,
    /// Gear, 0 down .. 1 up, and whether the pilot wants it down.
    pub gear: f64,
    pub gear_down: bool,
    /// Airbrake, 0..1.
    pub airbrake: f64,
    /// What the pilot asks for.
    pub controls: PlaneControls,
    /// The damage the model reads.
    pub damage: PlaneDamage,
    /// What the airframe touched in the last step.
    pub touch: Touch,
    /// The wheels' compression, m, by wheel (animation).
    pub wheel_compression: Vec<f64>,
    throttle_input: Arbiter,
}

impl PlaneState {
    /// A plane parked with its engine off and gear down.
    pub fn new() -> PlaneState {
        PlaneState {
            gear_down: true,
            ..PlaneState::default()
        }
    }

    /// A plane flying with its engine at `throttle` and gear up.
    pub fn flying(throttle: f64) -> PlaneState {
        PlaneState {
            engine_on: true,
            engine: 1.0,
            thrust: throttle,
            throttle,
            gear: 1.0,
            gear_down: false,
            ..PlaneState::default()
        }
    }

    /// The player's user actions to wanted controls (`0x140d1c5f0`, §3.2).
    pub fn pilot(&mut self, ty: &PlaneType, input: &FlightInput, dt: f64) {
        let side = input.heli_left - input.heli_right;
        self.controls.rudder = input.heli_rudder_left - input.heli_rudder_right + side;
        self.controls.aileron =
            (input.air_bank_left - input.air_bank_right).clamp(-1.0, 1.0) * 0.5 + side * 0.25;
        self.controls.elevator =
            (input.heli_fast_forward + input.heli_forward - input.heli_back).clamp(-1.0, 1.0) * 4.5;

        let digital = (input.heli_up - input.heli_down).clamp(-1.0, 1.0);
        let analog = input.heli_throttle_pos;
        self.throttle_input.update(digital, analog);
        let mut analog_on = 0.0;
        if !self.throttle_input.digital_in_charge {
            self.throttle = analog.clamp(0.0, 1.0);
            analog_on = analog;
        } else if digital.abs() > 0.0001 {
            let f = if ty.throttle_log_factor > 0.0 {
                ty.throttle_log_factor
            } else {
                1.0
            };
            let step = digital.clamp(-dt * 0.5, dt * 0.5);
            let lever = (self.throttle.max(0.0).powf(f) + step).max(0.0);
            self.throttle = lever.powf(1.0 / f).clamp(0.0, 1.0);
        }
        self.controls.brake = (input.air_plane_brake + input.heli_throttle_neg).clamp(0.0, 1.0);
        if digital > 0.0 || analog_on > 0.0 {
            self.engine_on = true;
        }
    }

    /// The throttle lever for animation: `throttle ^ throttleToThrustLogFactor`.
    pub fn lever(&self, ty: &PlaneType) -> f64 {
        self.throttle.max(0.0).powf(ty.throttle_log_factor)
    }

    /// Control smoothing and spool (§3.3).
    fn smooth(&mut self, ty: &PlaneType, dt: f64) {
        let on = if self.engine_on { 1.0 } else { 0.0 };
        self.engine = approach(self.engine, on, 0.1 * dt, 0.1 * dt).clamp(0.0, 1.0);
        self.thrust = approach(self.thrust, self.throttle, 0.7 * dt, 0.4 * dt).clamp(0.0, 1.0);
        let r = ty.rudder_rate * dt;
        self.rudder = approach(self.rudder, self.controls.rudder, r, r).clamp(-1.0, 1.0);
        let a = ty.aileron_rate * dt;
        self.aileron = approach(self.aileron, self.controls.aileron, a, a).clamp(-1.0, 1.0);
        let e = ty.elevator_rate * dt;
        self.elevator = approach(self.elevator, self.controls.elevator, e, e).clamp(-1.0, 1.0);
        if ty.flaps {
            let wanted = f64::from(self.flaps_position.min(2)) * 0.5;
            self.flaps = approach(self.flaps, wanted, 0.33 * dt, 0.33 * dt).clamp(0.0, 1.0);
        }
        if ty.gear_retracting {
            let wanted = if self.gear_down { 0.0 } else { 1.0 };
            self.gear = approach(
                self.gear,
                wanted,
                ty.gear_down_rate * dt,
                ty.gear_up_rate * dt,
            )
            .clamp(-1.0, 1.0);
        } else {
            self.gear = 0.0;
        }
        let brake = if self.damage.destroyed {
            0.0
        } else {
            self.controls.brake
        };
        self.airbrake = approach(self.airbrake, brake, dt, dt).clamp(0.0, 1.0);
    }
}

/// The angle of attack (`0x14068f510`): `angleOfIndicence − sy/|s|`.
pub fn angle_of_attack(ty: &PlaneType, s: DVec3) -> f64 {
    let l2 = s.length_squared();
    if l2 > 1e-6 {
        ty.angle_of_incidence - s.y / l2.sqrt()
    } else {
        0.0
    }
}

/// Lift in g (`0x140d2da40`), before the altitude factor; 0 below zero forward speed.
pub fn lift(ty: &PlaneType, speed: f64, aoa: f64, flaps: f64, ground_effect: f64) -> f64 {
    if speed <= 0.0 {
        return 0.0;
    }
    let x = 0.8 * speed / ty.max_speed;
    let flap_aoa = if x < 0.22 {
        4f64.to_radians()
    } else if x > 0.4 {
        0.0
    } else {
        4f64.to_radians() - (x - 0.22) * 0.387_850_94
    };
    let mut alpha = aoa + 2f64.to_radians() + (flap_aoa + 1f64.to_radians()) * flaps;
    if alpha > 18f64.to_radians() {
        alpha = (36f64.to_radians() - alpha).max(0.0);
    }
    let value = match table::plane_envelope(&ty.envelope, speed, ty.max_speed) {
        None => 0.0,
        Some(Err(last)) => last,
        Some(Ok(e)) => e * alpha.max(-10f64.to_radians()) / 18f64.to_radians(),
    };
    if value >= 0.0 {
        value * (1.0 + ground_effect)
    } else {
        value
    }
}

/// The stall factor (`0x140d164d0`), 0 on land.
pub fn stall_factor(ty: &PlaneType, s: DVec3, on_land: bool) -> f64 {
    if on_land || ty.stall_speed <= 0.1 {
        return 0.0;
    }
    let aoa = angle_of_attack(ty, s);
    let a = ((aoa.abs() - 10f64.to_radians()) * 3.819_718_6).clamp(0.0, 1.0);
    let b = ((1.5 - s.z / ty.stall_speed).max(0.0) + 0.3).min(1.0);
    a * b
}

/// The forces of one step in model space: force, torque and friction (§3.4).
pub fn forces(
    ty: &PlaneType,
    frame: &Airframe,
    state: &mut PlaneState,
    body: &RigidBody,
    env: &Environment<'_>,
    dt: f64,
) -> (DVec3, DVec3, DVec3) {
    let m = frame.mass;
    let r = frame.bounding_radius;
    let s = body.model_speed();
    let sz = s.z;
    let alt = altitude_factor(body.position.y, ty.alt_full_force, ty.alt_no_force);
    let ctrl = ((sz.abs() / (0.65 * ty.landing_speed)) - 0.5).clamp(0.0, 1.0) * alt;
    let on_land = state.touch.land;
    let mut force = DVec3::ZERO;
    let mut torque = DVec3::ZERO;

    // Thrust.
    let vref = ty.max_speed;
    let default_thrust = if sz <= vref * 0.66 {
        1.0
    } else if sz < vref * 1.15 {
        1.0 - (sz - vref * 0.66) / (vref * 1.15 - vref * 0.66)
    } else {
        0.0
    };
    let health = 1.0 - state.damage.engine;
    let thrust = ty.max_speed.sqrt()
        * state.engine
        * state.thrust.min(health)
        * 0.301_741_54
        * alt
        * table::plane_coef(&ty.thrust_coef, sz, ty.max_speed, default_thrust);
    force.z += m * thrust;

    // Lift.
    let h = env.height_above(body.position);
    let ground_effect = if r > 0.0 {
        (1.5 - h / (r * 1.5)).clamp(0.0, 1.0).powi(2) * 0.1
    } else {
        0.0
    };
    let aoa = angle_of_attack(ty, s);
    force.y += m * lift(ty, sz, aoa, state.flaps, ground_effect) * alt * GRAVITY;

    // Ground steering.
    let up = body.up();
    if on_land && sz.abs() < 25.0 && up.y > 0.0 {
        let k = if sz.abs() < 6.67 {
            sz.abs() * 0.149_925_04
        } else {
            (25.0 - sz.abs()) * 0.054_555_375
        }
        .min(40.0);
        torque.y += k * ty.wheel_steering_sensitivity * -30.0 * m * state.rudder;
    }

    // Ailerons.
    let half = state.aileron * 0.5;
    let aileron = r * ctrl * m * ty.aileron_sensitivity * ty.coef(&ty.aileron_coef, sz) * 6.537_733;
    torque.z += aileron * (half + half);

    // Elevator.
    let half = (state.elevator + state.trim) * 0.5;
    let elevator =
        m * ctrl * ty.coef(&ty.elevator_coef, sz) * 5.88396 * ty.elevator_sensitivity * 5.0;
    torque.x += elevator * (half + half);

    // Rudder.
    if s.length_squared() > 1.0 {
        let half = state.rudder * 0.5;
        let mut g = 1.0;
        if s.x * state.rudder > 0.0 {
            let along = sz.abs() / (sz * sz + s.x * s.x).sqrt();
            g = if along < ty.rudder_influence {
                0.0
            } else if along <= 1.0 {
                (along - ty.rudder_influence) / (1.0 - ty.rudder_influence)
            } else {
                1.0
            };
        }
        let d = m * ctrl * g * ty.coef(&ty.rudder_coef, sz);
        torque.y += -0.76 * r * d * (half + half);
        torque.z += 0.25 * d * (half + half);
    }

    // Draconic (weathervane) forces and torques.
    let sigma = stall_factor(ty, s, on_land);
    let w2 = s.length_squared();
    if w2 > 1.0 {
        let w = w2.sqrt();
        let unit = s / w;
        let fx = (w2 * 0.005 + w * 0.02) * unit.x.abs();
        let fy = (w2 * 0.02 + w * 0.2) * unit.y.abs();
        force.x -= alt * m * ty.draconic_force.x * fx * unit.x;
        force.y -= alt * m * ty.draconic_force.y * fy * unit.y;
        force.z += (fy * -0.05 - fx * 0.15) * m * alt * ty.draconic_force.z * sigma * unit.z;
        torque.y += (sigma * 0.3 + 0.04) * s.x * 4.0 * ty.coef(&ty.draconic_torque_x, sz) * m;
        torque.x += m * ty.coef(&ty.draconic_torque_y, sz) * (0.04 + sigma * 0.3) * s.y * -4.0;
    }

    // Stall: the nose and a wing drop.
    let stall = if state.damage.destroyed {
        1.0
    } else if sz.abs() < ty.landing_speed * 0.65 && !on_land {
        sigma
    } else {
        0.0
    };
    torque.x += m * stall * 2.5;
    torque.z += m * stall * 1.5;

    // Auto trim.
    if s.length_squared() > 1.0 {
        if state.elevator.abs() < 0.001 {
            state.trim -= torque.x * dt / m * 0.1;
        }
        let k = if sz > ty.landing_speed {
            ((sz - ty.landing_speed) / (0.5 * ty.landing_speed)).min(1.0)
        } else {
            0.0
        };
        state.trim = state.trim.max(0.0).min(k * 0.1);
    }

    // Air friction.
    let [c0, c1, c2] = ty.air_friction;
    let axis = |v: f64, c0: f64, c1: f64, c2: f64| c2 * v.abs() * v + c1 * v + c0 * sign(v);
    let brake = if ty.air_brake {
        state.airbrake
    } else if sz > 0.0 {
        (1.0 - state.engine * state.thrust) * (2.0 * sz / ty.max_speed).min(2.0)
    } else {
        0.0
    };
    let flaps = if ty.flaps { state.flaps } else { 0.0 };
    let fz = axis(s.z, c0.z, c1.z, c2.z)
        * ((1.0 - state.gear).abs() * ty.gears_up_friction_coef
            + flaps * ty.flaps_friction_coef
            + brake * ty.air_brake_friction_coef
            + 1.0);
    let friction = DVec3::new(
        axis(s.x, c0.x, c1.x, c2.x) * 0.1,
        axis(s.y, c0.y, c1.y, c2.y) * 0.1,
        fz,
    ) * (m * alt);

    (force, torque, friction)
}

/// The wheels: suspension, tire and brakes against the ground, added to `loads`. Returns
/// whether any wheel touched. _Ours, in place of the PhysX vehicle wheels_ (§3.5): the spring
/// and damper are the config's, the tire holds sideways slip and brakes up to the load.
fn wheels(
    ty: &PlaneType,
    frame: &Airframe,
    state: &mut PlaneState,
    body: &RigidBody,
    env: &Environment<'_>,
    loads: &mut Loads,
    dt: f64,
) -> bool {
    let com = body.center_of_mass(frame);
    let speed = body.velocity.length();
    let placed = ty.wheels.iter().filter(|w| w.is_placed()).count().max(1) as f64;
    let mut touched = false;
    state.wheel_compression.resize(ty.wheels.len(), 0.0);
    // The gear has to be down for the wheels to carry the plane.
    let down = state.gear < 0.5;
    for (i, wheel) in ty.wheels.iter().enumerate() {
        state.wheel_compression[i] = -wheel.suspension.max_droop;
        if !wheel.is_placed() || !down {
            continue;
        }
        let direction = body
            .to_world(wheel.suspension.travel_direction)
            .normalize_or(-body.up());
        let attach = body.point(wheel.position);
        let ground = env.ground.height(attach.x, attach.z);
        if direction.y >= -1e-6 {
            continue;
        }
        let distance = (attach.y - ground) / -direction.y;
        let reach = wheel.radius + wheel.suspension.max_droop;
        if distance > reach {
            continue;
        }
        touched = true;
        let compression = wheel.radius - distance;
        state.wheel_compression[i] = compression.min(wheel.suspension.max_compression);
        let contact = attach + direction * distance;
        let v = body.velocity_at(frame, contact);
        let sprung = if wheel.suspension.sprung_mass > 0.0 {
            wheel.suspension.sprung_mass
        } else {
            frame.mass / placed
        };
        let load = wheel
            .suspension
            .force_with(sprung, compression, v.dot(direction));
        let normal = env.ground.normal(contact.x, contact.z);
        loads.add_force_at(-direction * load, contact, com);

        // The tire: the wheel's heading flattened onto the ground; steering wheels turn with
        // the rudder, less with speed (0x14068e440).
        let steer = if wheel.steering {
            state.rudder * -0.942_477_9 / (speed * 0.4 + 1.0)
        } else {
            0.0
        };
        let heading = body.orientation * DQuat::from_rotation_y(steer) * DVec3::Z;
        let forward = heading.reject_from(normal).normalize_or(body.direction());
        let lateral = normal.cross(forward).normalize_or(body.aside());
        let grip = load * wheel.tire.friction_at(0.0);
        let hold = |v: f64, limit: f64| -sign(v) * limit.min(sprung * v.abs() / dt);
        let side = hold(v.dot(lateral), grip);
        let brake = state.controls.brake * wheel.max_brake_torque / wheel.radius;
        let along = hold(v.dot(forward), brake.min(grip));
        loads.add_force_at(lateral * side + forward * along, contact, com);
    }
    touched
}

/// The legacy contacts of the Geometry points (§3.5): belly, wingtips and anything below the
/// surface besides the wheels. Adds to `loads`, returns what was touched.
fn hull_contacts(
    ty: &PlaneType,
    frame: &Airframe,
    state: &PlaneState,
    body: &RigidBody,
    env: &Environment<'_>,
    loads: &mut Loads,
) -> Touch {
    let mut touch = Touch::default();
    let mut points = Vec::new();
    for &p in &frame.contact_points {
        let at = body.point(p);
        let land = env.ground.height(at.x, at.z);
        if at.y < land {
            points.push((at, land - at.y, env.ground.normal(at.x, at.z)));
        }
    }
    if points.is_empty() {
        return touch;
    }
    touch.land = true;
    let n = points.len() as f64;
    let share = 3.0 / n;
    let m = frame.mass;
    let com = body.center_of_mass(frame);
    let s = body.model_speed();
    let upright = body.up().y > 0.2 && !state.damage.destroyed;
    let momentum = body.angular_momentum(frame);
    for (at, depth, normal) in points {
        touch.depth = touch.depth.max(depth);
        let held = depth.min(0.1);
        if !state.damage.destroyed {
            loads.add_force_at(normal * (m * (90.0 / n) * held), at, com);
        }
        let w = (9.0 / n) * held;
        let n_model = body.to_model(normal);
        let vn = n_model * n_model.dot(s);
        let t = s - vn;
        let mut f = DVec3::new(
            sign(t.x) * 100_000.0 * w,
            (t.y * 300.0 + 500.0 * sign(t.y)) * w,
            (t.z * 300.0 + 500.0 * sign(t.z)) * w,
        );
        let roll = if !upright {
            if t.z.abs() <= 2.0 { 40_000.0 } else { 80_000.0 }
        } else if state.gear > 0.5 {
            20_000.0
        } else {
            0.0
        };
        let brake = if roll > 0.0 {
            sign(t.z) * roll * w
        } else {
            // The wheel brakes fade out between 0.1 and 0.2 of `maxSpeed`.
            let slow = (ty.max_speed * 0.1).max(22.222_223);
            let fast = (ty.max_speed * 0.2).max(36.111_11);
            let speed = t.z.abs();
            let fade = if speed < slow {
                1.0
            } else if speed <= fast {
                1.0 - (speed - slow) / (fast - slow)
            } else {
                0.0
            };
            sign(t.z) * 20_000.0 * w * state.controls.brake * fade
        };
        f.z += brake;
        f *= 0.0001;
        let clip = |f: f64, t: f64| {
            let cap = t * 10.0;
            if t > 0.0 {
                f.min(cap).max(0.0)
            } else if t < 0.0 {
                f.max(cap).min(0.0)
            } else {
                f
            }
        };
        let f = DVec3::new(clip(f.x, t.x), clip(f.y, t.y), clip(f.z, t.z)) * (m * share);
        let f_world = body.to_world(f);
        loads.friction += f_world;
        if !state.damage.destroyed {
            loads.torque -= (at - com).cross(f_world) * 0.5;
        }
        let stiction = DVec3::new(sign(vn.x), sign(vn.y), sign(vn.z)) * (1.5 * w);
        loads.friction += body.to_world(stiction) * (m * share);
        loads.angular_friction += momentum * (0.3 / n);
    }
    touch
}

/// One simulation step of a plane (`AirplaneAutoEPE::Simulate`).
pub fn step(
    ty: &PlaneType,
    frame: &Airframe,
    state: &mut PlaneState,
    body: &mut RigidBody,
    env: &Environment<'_>,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    state.smooth(ty, dt);
    let (force, torque, friction) = forces(ty, frame, state, body, env, dt);
    let mut loads = Loads {
        force: body.to_world(force),
        torque: body.to_world(torque),
        friction: body.to_world(friction),
        angular_friction: body.angular_momentum(frame) * 2.0,
    };
    let on_wheels = wheels(ty, frame, state, body, env, &mut loads, dt);
    let mut touch = hull_contacts(ty, frame, state, body, env, &mut loads);
    touch.land |= on_wheels;
    state.touch = touch;
    integrate(body, frame, &loads, env.gravity, dt);
    settle(body, frame, env.ground);
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::{ConfigTree, parse_text};

    /// The Caesar BTT's flight entries (`C_Plane_Civil_01_F`).
    const CAESAR: &str = r#"
        class C_Plane_Civil_01_F {
            maxSpeed = 450; landingSpeed = 130;
            envelope[] = {0, 0.01, 0.4, 1.6, 3.2, 3.4, 3.5, 3.6, 3.6, 3.7, 3.7, 3.6, 1.0};
            thrustCoef[] = {1.26, 1.25, 1.23, 1.21, 1.18, 1.14, 1.09, 1.03, 0.96, 0.87, 0.48, 0.12, 0.0, 0.0, 0.0, 0.0};
            elevatorCoef[] = {0.0, 0.1, 0.28, 0.35, 0.4, 0.45, 0.49, 0.53, 0.57, 0.58, 0.56};
            aileronCoef[] = {0.0, 0.4, 0.9, 1.1, 1.2, 1.3, 1.3};
            rudderCoef[] = {0.0, 0.89, 1.5, 2.1, 2.5, 3.0, 3.6, 3.9, 4.0, 3.6, 1.8};
            draconicForceXCoef = 12.0; draconicForceYCoef = 1.0; draconicForceZCoef = 1.0;
            draconicTorqueXCoef[] = {14.0, 12.0, 11.2, 10.6, 9.9, 9.6, 9.7, 10.5, 11.0, 11.5, 12.0};
            draconicTorqueYCoef[] = {4.5, 4.1, 3.7, 3.3, 3.0, 2.7, 2.5, 2.3, 2.1, 1.9, 1.8};
            airFrictionCoefs0[] = {0.0, 0.0, 0.0};
            airFrictionCoefs1[] = {0.1, 0.05, 0.006};
            airFrictionCoefs2[] = {0.001, 0.0005, 6e-5};
            aileronSensitivity = 0.7; elevatorSensitivity = 0.9; rudderInfluence = 0.6946;
            angleOfIndicence = "4*3.1415/180";
            flapsFrictionCoef = 0.4; gearsUpFrictionCoef = 0.5; airBrakeFrictionCoef = 3.0;
            elevatorControlsSensitivityCoef = 2.0; aileronControlsSensitivityCoef = 3.6;
            rudderControlsSensitivityCoef = 3.0; wheelSteeringSensitivity = 1.0;
            airBrake = 1; flaps = 1; gearRetracting = 0;
            altFullForce = 6000; altNoForce = 7500;
        };"#;

    fn caesar() -> PlaneType {
        let tree = ConfigTree::from_config(&parse_text(CAESAR).unwrap());
        PlaneType::from_config(&tree.root().get("C_Plane_Civil_01_F"))
    }

    #[test]
    fn the_config_reads_with_the_derived_speeds() {
        let ty = caesar();
        assert!((ty.max_speed - 125.0).abs() < 1e-9);
        assert!((ty.landing_speed - 130.0 / 3.6).abs() < 1e-9);
        assert!((ty.stall_speed - 130.0 / 3.6 * 0.65).abs() < 1e-9);
        // The config writes `"4*3.1415/180"`: the expression, not 4°.
        let written = 4.0 * 31415.0 / 10000.0 / 180.0;
        assert!((ty.angle_of_incidence - written).abs() < 1e-9);
        assert_eq!(ty.draconic_torque_x.len(), 11);
    }

    #[test]
    fn lift_grows_with_the_angle_of_attack_and_stalls_past_eighteen_degrees() {
        let ty = caesar();
        let v = 50.0;
        let low = lift(&ty, v, 0.0, 0.0, 0.0);
        let high = lift(&ty, v, 10f64.to_radians(), 0.0, 0.0);
        let stalled = lift(&ty, v, 25f64.to_radians(), 0.0, 0.0);
        assert!(high > low && low > 0.0);
        assert!(stalled < high);
        // At 16° effective (14° + 2°): 16/18 of the envelope.
        let e = table::plane_envelope(&ty.envelope, v, ty.max_speed)
            .unwrap()
            .unwrap();
        assert!((lift(&ty, v, 14f64.to_radians(), 0.0, 0.0) - e * 16.0 / 18.0).abs() < 1e-9);
    }

    #[test]
    fn flaps_add_lift_at_low_speed() {
        let ty = caesar();
        assert!(lift(&ty, 30.0, 0.0, 1.0, 0.0) > lift(&ty, 30.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn the_stall_factor_rises_past_ten_degrees() {
        let ty = caesar();
        // Descending steeply relative to the nose: a high angle of attack.
        let s = DVec3::new(0.0, -15.0, 20.0);
        assert!(stall_factor(&ty, s, false) > 0.5);
        assert_eq!(stall_factor(&ty, s, true), 0.0);
        assert_eq!(stall_factor(&ty, DVec3::new(0.0, 0.0, 40.0), false), 0.0);
    }

    #[test]
    fn keyboard_throttle_moves_half_a_lever_per_second() {
        let ty = caesar();
        let mut state = PlaneState::new();
        let input = FlightInput {
            heli_up: 1.0,
            ..FlightInput::default()
        };
        for _ in 0..15 {
            state.pilot(&ty, &input, 1.0 / 15.0);
        }
        assert!((state.throttle - 0.5).abs() < 1e-9, "{}", state.throttle);
        assert!(state.engine_on);
    }

    #[test]
    fn thrust_follows_the_throttle_slower_up_than_down() {
        let ty = caesar();
        let mut state = PlaneState::flying(0.0);
        state.throttle = 1.0;
        state.smooth(&ty, 1.0);
        assert!((state.thrust - 0.4).abs() < 1e-12);
        state.throttle = 0.0;
        state.smooth(&ty, 0.25);
        assert!((state.thrust - (0.4 - 0.175)).abs() < 1e-12);
    }
}
