//! Cars and tanks: the PhysX 4W drive rebuilt on the config's own numbers
//! (`docs/re/sim-vehicles.md` §2). Cars and tanks share the model — the tank's track
//! differential is the same wheels with a steering trade between the tracks and the
//! `tankTurnForce` yaw moment instead of steering wheels.
//!
//! Each fixed step, per wheel: cast a ray along the suspension's travel direction, turn the hit
//! into a spring and damper force, take the tire's slip from the wheel's spin and the contact's
//! velocity, and turn the slip into a force inside the friction circle. The chassis integrates
//! the forces itself (ADR 0009): `a3-world` owns the entity and the collision world, this owns
//! the motion, and the surface is only asked questions.

use crate::engine::EngineState;
use crate::gearbox::{DriveDirection, Transmission};
use crate::ground::{BodyState, GroundSurface};
use crate::vehicle::{VehicleData, VehicleInput, VehicleKind};
use crate::wheel::{GRAVITY, Wheel, WheelSide};
use glam::{DQuat, DVec3};

/// How much wheel spin counts as full slip: the speed the slip is normalised by, m/s _(ours;
/// PhysX normalises by the wheel's own speed with a floor — `docs/re/sim-vehicles.md` §2)_.
const SLIP_SPEED_FLOOR: f64 = 1.0;

/// The brakes are smoothed over this much wheel spin, so a wheel coming to a stop does not
/// chatter around zero, rad/s. At a standstill they hold nothing — a vehicle parked on a slope
/// creeps slowly until the brake bites _(ours)_.
const BRAKE_SMOOTHING: f64 = 0.5;

/// How much of a tank's drive torque the steering trades from one track to the other at full
/// lock _(ours)_.
const TANK_TRACK_BIAS: f64 = 0.5;

/// One wheel's dynamic state, as the step leaves it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelState {
    /// The wheel's spin, rad/s.
    pub omega: f64,
    /// The wheel's steering angle this step, rad.
    pub steer: f64,
    /// The suspension's compression from the design position, m: positive when the wheel is
    /// pushed up into the body past the modelled ride height.
    pub compression: f64,
    /// Whether the wheel is touching the ground.
    pub in_contact: bool,
    /// The suspension force, N.
    pub load: f64,
    /// Longitudinal slip: 0 rolling, ±1 at the slip speed, more when the wheel spins.
    pub slip_long: f64,
    /// Lateral slip.
    pub slip_lat: f64,
    /// The contact point, world space (zero without contact).
    pub contact: DVec3,
    /// The surface normal at the contact (unit; +Y without contact).
    pub normal: DVec3,
    /// The surface friction at the contact (`surfaceFriction`).
    pub friction: f64,
    /// The whole force this wheel puts on the chassis — the suspension along its travel
    /// direction plus the tire's longitudinal and lateral forces. Zero without contact.
    pub force: DVec3,
}

impl Default for WheelState {
    fn default() -> Self {
        WheelState {
            omega: 0.0,
            steer: 0.0,
            compression: 0.0,
            in_contact: false,
            load: 0.0,
            slip_long: 0.0,
            slip_lat: 0.0,
            contact: DVec3::ZERO,
            normal: DVec3::Y,
            friction: 1.0,
            force: DVec3::ZERO,
        }
    }
}

/// A driving car or tank: its chassis, its wheels, its engine and its gearbox.
///
/// The wheel geometry must be resolved ([`VehicleData::resolve_geometry`]) before the first
/// step, and the chassis placed at [`VehicleData::rest_height`], where its springs hold it.
#[derive(Debug, Clone, PartialEq)]
pub struct WheeledVehicle {
    /// The chassis.
    pub body: BodyState,
    /// The wheels, in [`VehicleData::wheels`] order.
    pub wheels: Vec<WheelState>,
    /// The engine.
    pub engine: EngineState,
    /// The gearbox.
    pub transmission: Transmission,
    /// The direction the driver has selected.
    pub direction: DriveDirection,
    /// How far the drive demand has ramped up, 0..=1 (`thrustDelay`).
    pub thrust: f64,
    /// The chassis's inertia about its own axes, kg·m².
    pub inertia: DVec3,
}

impl WheeledVehicle {
    /// A vehicle resting at `position` on level ground, facing `yaw`, in gear. `size` is the
    /// model's bounding-box size, which the inertia is taken from.
    pub fn new(data: &VehicleData, position: DVec3, yaw: f64, size: DVec3) -> WheeledVehicle {
        WheeledVehicle {
            body: BodyState::new(position, yaw),
            wheels: vec![WheelState::default(); data.wheels.len()],
            engine: EngineState {
                omega: data.engine.min_omega,
            },
            transmission: Transmission::new(&data.gearbox, DriveDirection::Forward),
            direction: DriveDirection::Forward,
            thrust: 0.0,
            inertia: data.box_inertia(size).max(DVec3::splat(1.0)),
        }
    }

    /// The chassis's speed, m/s.
    pub fn speed(&self) -> f64 {
        self.body.velocity.length()
    }

    /// The speed along the chassis's forward axis, m/s (negative when reversing).
    pub fn forward_speed(&self) -> f64 {
        self.body.velocity.dot(self.body.forward())
    }

    /// The yaw angle, rad.
    pub fn yaw(&self) -> f64 {
        self.body.yaw()
    }

    /// The gear number the box is in (1 = the first forward gear, −1 the first reverse).
    pub fn gear_number(&self, data: &VehicleData) -> i32 {
        self.transmission.gear_number(&data.gearbox)
    }

    /// A fixed step of the whole vehicle: `dt` seconds of driver input against `surface`.
    pub fn step(
        &mut self,
        data: &VehicleData,
        input: &VehicleInput,
        surface: &dyn GroundSurface,
        dt: f64,
    ) {
        if dt <= 0.0 {
            return;
        }
        let input = input.clamped();
        self.steer_wheels(data, &input);
        self.select_direction(data, &input);
        self.ramp_thrust(data, &input, dt);
        let throttle = self.throttle(data, &input);
        self.cast_wheels(data, surface);
        let drive = self.drivetrain(data, throttle, input.steer, dt);
        self.step_wheels(data, &input, &drive, dt);

        // The chassis: gravity, and the force every wheel holds it up and drives it with.
        let mut force = -DVec3::Y * (data.mass * GRAVITY);
        let mut torque = DVec3::ZERO;
        for wheel in &self.wheels {
            force += wheel.force;
            if wheel.in_contact {
                torque += (wheel.contact - self.body.position).cross(wheel.force);
            }
        }
        let (bar_force, bar_torque) = self.anti_rollbar(data);
        force += bar_force;
        torque += bar_torque;
        torque += self.tank_turn(data, &input);
        self.integrate(data, force, torque, dt);
    }

    /// The driver's direction demand, engaged once the vehicle is slow enough to turn around
    /// (`brakeIdleSpeed`); the gearbox itself takes the direction change through a gear change.
    fn select_direction(&mut self, data: &VehicleData, input: &VehicleInput) {
        let wanted = if input.reverse {
            DriveDirection::Reverse
        } else {
            DriveDirection::Forward
        };
        if wanted == self.direction {
            return;
        }
        let idle = data.handling.brake_idle_speed.max(1.0);
        if self.forward_speed().abs() < idle {
            self.direction = wanted;
        }
    }

    /// `thrustDelay`: the drive comes up over that many seconds instead of jumping, and falls
    /// away the moment the driver lifts off.
    fn ramp_thrust(&mut self, data: &VehicleData, input: &VehicleInput, dt: f64) {
        let delay = data.handling.thrust_delay;
        if input.throttle <= 0.0 || delay <= 0.0 {
            self.thrust = 0.0;
        } else {
            self.thrust = (self.thrust + dt / delay).min(1.0);
        }
    }

    /// The throttle the engine sees: the driver's, ramped, and cut once the vehicle is at
    /// `maxSpeed` — the top-speed governor.
    fn throttle(&self, data: &VehicleData, input: &VehicleInput) -> f64 {
        let mut throttle = input.throttle * self.thrust;
        let max_speed = data.handling.max_speed();
        if max_speed > 0.0 && self.forward_speed() >= max_speed {
            throttle = 0.0;
        }
        throttle
    }

    /// The steering wheels turn to the driver's lock. Every other wheel points straight ahead.
    ///
    /// The lock itself is [`VehicleData::max_steer_angle`] — the configs mark *which* wheels
    /// steer and leave the angle to the physics _(ours, `docs/re/sim-vehicles.md` §5)_.
    fn steer_wheels(&mut self, data: &VehicleData, input: &VehicleInput) {
        let angle = input.steer * data.max_steer_angle;
        for index in 0..self.wheels.len() {
            self.wheels[index].steer = if data.wheels[index].steering {
                angle
            } else {
                0.0
            };
        }
    }

    /// The suspension: one ray per wheel along the travel direction, kept as the wheel's contact
    /// for the rest of the step.
    ///
    /// The design position is the modelled one — the memory point is the hub with the wheel at
    /// `radius` off the ground — so a wheel at the design position carries exactly the sprung
    /// mass, and a vehicle placed at [`VehicleData::rest_height`] is in equilibrium.
    fn cast_wheels(&mut self, data: &VehicleData, surface: &dyn GroundSurface) {
        let body = self.body;
        for index in 0..data.wheels.len() {
            let wheel = &data.wheels[index];
            let reach = wheel.radius + wheel.suspension.max_droop;
            // The cast runs along `suspTravelDirection`, from the hub the model places.
            let down = body.orientation * wheel.suspension.travel_direction;
            let attach = body.local_to_world(wheel.position);
            let hit = if wheel.radius > 0.0 && reach > 0.0 && down.length_squared() > 0.0 {
                surface.cast(attach, down.normalize(), reach)
            } else {
                None
            };
            let state = &mut self.wheels[index];
            match hit {
                Some(hit) => {
                    let distance = (hit.point - attach).length();
                    state.compression = (wheel.radius - distance).clamp(
                        -wheel.suspension.max_droop,
                        wheel.suspension.max_compression,
                    );
                    state.in_contact = true;
                    state.contact = hit.point;
                    state.normal = hit.normal.normalize_or(DVec3::Y);
                    state.friction = hit.friction;
                }
                None => {
                    state.in_contact = false;
                    state.compression = -wheel.suspension.max_droop;
                    state.contact = DVec3::ZERO;
                }
            }
        }
    }

    /// The engine, clutch and gearbox: the engine's torque crosses the clutch into the driven
    /// wheels' spin. Returns the torque each wheel is driven with.
    ///
    /// The clutch couples the engine to the gearbox with a torque that grows with their
    /// difference in speed, capped at twice the engine's peak torque — as much as a clutch
    /// transmits before it slips _(ours)_. At or below `minOmega` the clutch is idle: a car
    /// stopped in gear neither stalls nor creeps.
    fn drivetrain(&mut self, data: &VehicleData, throttle: f64, steer: f64, dt: f64) -> Vec<f64> {
        let driven = self.driven_wheels(data);
        let wheel_speed = mean_omega(&self.wheels, &driven);
        self.transmission.step(
            &data.gearbox,
            &data.engine,
            wheel_speed,
            self.engine.omega,
            self.direction,
            dt,
        );
        let ratio = self.transmission.ratio(&data.gearbox);
        if ratio == 0.0 {
            // A gear change holds the box in neutral: the engine drives nothing.
            self.engine.step(&data.engine, throttle, 0.0, dt);
            return vec![0.0; data.wheels.len()];
        }
        let gearbox_omega = wheel_speed * ratio;
        let capacity = 2.0 * data.engine.peak_torque;
        let clutch = if self.engine.omega <= data.engine.min_omega {
            0.0
        } else {
            (data.gearbox.clutch_strength * (self.engine.omega - gearbox_omega))
                .clamp(-capacity, capacity)
        };
        self.engine.step(&data.engine, throttle, clutch, dt);
        self.drive_torques(data, clutch * ratio, steer)
    }

    /// How much torque each wheel gets: the differential's split between the axles, the
    /// limited-slip split between the sides, and — for a tank — the steering's trade between the
    /// tracks.
    fn drive_torques(&self, data: &VehicleData, torque: f64, steer: f64) -> Vec<f64> {
        let differential = &data.handling.differential;
        let front = data.front_wheels();
        let rear = data.rear_wheels();
        let mut out = vec![0.0; data.wheels.len()];
        match (
            differential.kind.drives_front(),
            differential.kind.drives_rear(),
        ) {
            (true, true) => {
                let share = self.centre_share(data, &front, &rear);
                split_axle(
                    &mut out,
                    &front,
                    torque * share,
                    differential.front_bias,
                    |index| self.wheels[index].omega,
                );
                split_axle(
                    &mut out,
                    &rear,
                    torque * (1.0 - share),
                    differential.rear_bias,
                    |index| self.wheels[index].omega,
                );
            }
            // Nothing says which axle: the front takes it, so the vehicle still drives.
            (drives_front, _) => {
                let axle = if drives_front { &front } else { &rear };
                let bias = if drives_front {
                    differential.front_bias
                } else {
                    differential.rear_bias
                };
                split_axle(&mut out, axle, torque, bias, |index| {
                    self.wheels[index].omega
                });
            }
        }
        if data.kind == VehicleKind::Tank {
            split_tracks(&mut out, &data.wheels, steer);
        }
        out
    }

    /// The share of the torque the front axle takes: `frontRearSplit`, moved toward the slower
    /// axle by `centreBias` when the centre differential may limit slip.
    fn centre_share(&self, data: &VehicleData, front: &[usize], rear: &[usize]) -> f64 {
        let differential = &data.handling.differential;
        let base = differential.front_rear_split.clamp(0.0, 1.0);
        if !differential.kind.limited_slip() {
            return base;
        }
        let bias = differential.centre_bias.max(1.0);
        let front_speed = mean_omega(&self.wheels, front);
        let rear_speed = mean_omega(&self.wheels, rear);
        if front_speed < rear_speed {
            base.max(bias / (1.0 + bias))
        } else if rear_speed < front_speed {
            base.min(1.0 / (1.0 + bias))
        } else {
            base
        }
    }

    /// The wheels the differential drives.
    fn driven_wheels(&self, data: &VehicleData) -> Vec<usize> {
        let kind = data.handling.differential.kind;
        let mut driven = Vec::new();
        if kind.drives_front() {
            driven.extend(data.front_wheels());
        }
        if kind.drives_rear() {
            driven.extend(data.rear_wheels());
        }
        driven
    }

    /// Per wheel: the suspension's spring and damper, the tire's slip and force, and the wheel's
    /// own spin.
    ///
    /// The spin is solved together with the tire's longitudinal force, so a stiff tire cannot
    /// make the step unstable: the force that brings the wheel's surface speed to the contact's
    /// is found first, the friction circle caps it, and the spin follows from the capped force.
    /// Whatever the tire cannot hold, the wheel spins away as slip.
    fn step_wheels(&mut self, data: &VehicleData, input: &VehicleInput, drive: &[f64], dt: f64) {
        let body = self.body;
        let mass = data.mass.max(1.0);
        // `brakeIdleSpeed`: with the driver off the throttle the brakes come on below that
        // speed, so the vehicle comes to a stop instead of creeping on the clutch.
        let idle = if input.throttle <= 0.0
            && self.forward_speed().abs() < data.handling.brake_idle_speed.max(0.0)
        {
            1.0
        } else {
            0.0
        };
        let brake = input.brake.max(idle);
        for (index, wheel) in data.wheels.iter().enumerate() {
            let mut state = self.wheels[index];
            let moi = wheel.moi.max(0.01);
            if !state.in_contact {
                // In the air the wheel just coasts down against its own damping.
                state.omega -= state.omega * wheel.damping_rate_in_air * dt / moi;
                state.load = 0.0;
                state.slip_long = 0.0;
                state.slip_lat = 0.0;
                state.force = DVec3::ZERO;
                self.wheels[index] = state;
                continue;
            }

            let up = (body.orientation * -wheel.suspension.travel_direction).normalize_or(DVec3::Y);
            let offset = state.contact - body.position;
            let v = body.velocity_at(offset);
            let load = wheel.suspension.force_with(
                sprung_mass(data, index),
                state.compression,
                v.dot(-up),
            );
            state.load = load;

            // The contact frame: the wheel's own direction, flattened onto the surface.
            let forward = (body.orientation * DQuat::from_rotation_y(state.steer) * DVec3::Z)
                .reject_from(state.normal)
                .normalize_or(body.forward());
            let lateral = state.normal.cross(forward).normalize_or(body.right());
            let v_forward = v.dot(forward);
            let v_lateral = v.dot(lateral);
            let reference = v_forward.abs().max(SLIP_SPEED_FLOOR);
            let r = wheel.radius;

            // The brakes hold the wheel against its spin, smoothed over the zero crossing so a
            // stopped wheel is not held by a chattering torque.
            let brake_torque = (brake * wheel.max_brake_torque
                + input.handbrake * wheel.max_hand_brake_torque)
                * (state.omega / BRAKE_SMOOTHING).tanh();

            // The longitudinal force the tire settles at: the wheel and the contact brought to
            // one speed, capped by the friction circle afterwards.
            let k_long = wheel.tire.longitudinal_stiffness(load);
            let denominator = reference / k_long.max(1e-9) - dt / mass - r * r * dt / moi;
            let raw_long = if k_long > 0.0 && denominator.abs() > 1e-12 {
                (v_forward - r * state.omega - r * (drive[index] - brake_torque) * dt / moi)
                    / denominator
            } else {
                0.0
            };
            let k_lat = wheel.tire.lateral_stiffness(load);
            let raw_lat = -k_lat * v_lateral / reference;
            // `frictionVsSlipGraph` at the combined slip, over the surface's `surfaceFriction`
            // and the tire's own coefficient, times the load: the most the tire may hold.
            let slip = ((state.omega * r - v_forward) / reference)
                .hypot(v_lateral / reference)
                .min(1.0);
            let friction = state.friction / 2.0 * wheel.tire.friction_at(slip);
            let (longitudinal, lateral_force) = limit_circle(raw_long, raw_lat, friction * load);

            // The spin follows from the force the tire ended up holding, from the drive and
            // brake torques, and from the rolling wheel's own `dampingRate`.
            state.omega += (drive[index] - brake_torque - longitudinal * r) * dt / moi;
            state.omega -= state.omega * wheel.damping_rate * dt / moi;
            state.slip_long = (state.omega * r - v_forward) / reference;
            state.slip_lat = v_lateral / reference;
            state.force = up * load + forward * longitudinal + lateral * lateral_force;
            self.wheels[index] = state;
        }
    }

    /// The anti-rollbar between the two sides of each axle: a force that opposes their
    /// difference in compression (`antiRollbarForceCoef`, faded in and out with speed).
    fn anti_rollbar(&self, data: &VehicleData) -> (DVec3, DVec3) {
        let mut force = DVec3::ZERO;
        let mut torque = DVec3::ZERO;
        if data.handling.anti_rollbar_force_coef <= 0.0 {
            return (force, torque);
        }
        let speed = self.speed();
        for axle in [data.front_wheels(), data.rear_wheels()] {
            let of_side = |side: WheelSide| -> Vec<usize> {
                axle.iter()
                    .copied()
                    .filter(|index| data.wheels[*index].side == Some(side))
                    .collect()
            };
            for (left, right) in of_side(WheelSide::Left)
                .into_iter()
                .zip(of_side(WheelSide::Right))
            {
                if !self.wheels[left].in_contact || !self.wheels[right].in_contact {
                    continue;
                }
                let bar = data.handling.anti_rollbar_force(
                    speed,
                    self.wheels[left].compression - self.wheels[right].compression,
                );
                if bar == 0.0 {
                    continue;
                }
                // The compressed side is pushed back up, the other held down.
                for (index, sign) in [(left, 1.0), (right, -1.0)] {
                    let at = DVec3::Y * (bar * sign);
                    force += at;
                    torque += (self.wheels[index].contact - self.body.position).cross(at);
                }
            }
        }
        (force, torque)
    }

    /// The yaw moment a tank's steering applies (`tankTurnForce`), full up to
    /// `tankTurnForceAngMinSpd` of turning and fading to nothing by `tankTurnForceAngSpd` — a
    /// track differential loses its purchase as the turn rate comes up _(our reading; the pairs
    /// are the only entries the shipped tanks carry — `docs/re/sim-vehicles.md` §2)_.
    fn tank_turn(&self, data: &VehicleData, input: &VehicleInput) -> DVec3 {
        let handling = &data.handling;
        if input.steer == 0.0 || handling.tank_turn_force <= 0.0 {
            return DVec3::ZERO;
        }
        let rate = self.body.angular_velocity.dot(self.body.up()).abs();
        let min = handling.tank_turn_force_ang_min_speed;
        let max = handling.tank_turn_force_ang_speed;
        let fade = if rate <= min {
            1.0
        } else if max > min {
            (1.0 - (rate - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.body.up() * (input.steer * handling.tank_turn_force * fade)
    }

    /// The chassis's motion: semi-implicit Euler, with the inertia rotated into world space.
    fn integrate(&mut self, data: &VehicleData, force: DVec3, torque: DVec3, dt: f64) {
        let mass = data.mass.max(1.0);
        self.body.velocity += force / mass * dt;
        let local = self.body.world_to_local(torque);
        let angular = DVec3::new(
            local.x / self.inertia.x,
            local.y / self.inertia.y,
            local.z / self.inertia.z,
        );
        self.body.angular_velocity += (self.body.orientation * angular) * dt;
        self.body.position += self.body.velocity * dt;
        let spin = self.body.angular_velocity * dt;
        let angle = spin.length();
        if angle > 1e-9 {
            let rotation = DQuat::from_axis_angle(spin / angle, angle);
            self.body.orientation = (rotation * self.body.orientation).normalize();
        }
    }
}

/// The mass one wheel carries: its own `sprungMass`, or — when the config leaves it to the
/// model, as the tanks do with `-1` — an even share of the vehicle's mass. The share sets the
/// spring's preload, so an even split puts every wheel at the same ride height.
pub fn sprung_mass(data: &VehicleData, index: usize) -> f64 {
    let wheel = &data.wheels[index];
    if wheel.suspension.sprung_mass > 0.0 {
        wheel.suspension.sprung_mass
    } else {
        data.mass / data.wheels.len().max(1) as f64
    }
}

/// The mean spin of a set of wheels.
fn mean_omega(states: &[WheelState], wheels: &[usize]) -> f64 {
    if wheels.is_empty() {
        return 0.0;
    }
    wheels.iter().map(|index| states[*index].omega).sum::<f64>() / wheels.len() as f64
}

/// Gives one axle's wheels their share of its torque, through the differential's `bias`.
fn split_axle(
    out: &mut [f64],
    axle: &[usize],
    torque: f64,
    bias: f64,
    speed: impl Fn(usize) -> f64,
) {
    if axle.is_empty() {
        return;
    }
    let speeds: Vec<f64> = axle.iter().map(|index| speed(*index)).collect();
    for (index, share) in axle.iter().zip(axle_shares(&speeds, bias)) {
        out[*index] += torque * share;
    }
}

/// The share of its axle's torque each wheel takes: even, unless a limited-slip differential
/// may put `bias` times as much through the slower side.
fn axle_shares(speeds: &[f64], bias: f64) -> Vec<f64> {
    let count = speeds.len();
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![1.0];
    }
    if count == 2 && bias > 1.0 {
        let slow = bias / (1.0 + bias);
        return match speeds[0].total_cmp(&speeds[1]) {
            std::cmp::Ordering::Less => vec![slow, 1.0 - slow],
            std::cmp::Ordering::Greater => vec![1.0 - slow, slow],
            std::cmp::Ordering::Equal => vec![0.5, 0.5],
        };
    }
    vec![1.0 / count as f64; count]
}

/// A tank's steering trades drive torque between the tracks: the left gets
/// `(1 + steer·bias)/2` of it and the right the rest, so a held stick pivots the tank.
fn split_tracks(torques: &mut [f64], wheels: &[Wheel], steer: f64) {
    if steer == 0.0 {
        return;
    }
    let mut left = 0.0;
    let mut right = 0.0;
    for (index, wheel) in wheels.iter().enumerate() {
        match wheel.side {
            Some(WheelSide::Left) => left += torques[index],
            Some(WheelSide::Right) => right += torques[index],
            None => {}
        }
    }
    let total = left + right;
    if total <= 0.0 {
        return;
    }
    let left_target = total * (1.0 + steer * TANK_TRACK_BIAS) * 0.5;
    let right_target = total - left_target;
    for (index, wheel) in wheels.iter().enumerate() {
        match wheel.side {
            Some(WheelSide::Left) if left > 0.0 => torques[index] *= left_target / left,
            Some(WheelSide::Right) if right > 0.0 => torques[index] *= right_target / right,
            _ => {}
        }
    }
}

/// Caps the two tire forces at the friction circle, keeping their direction.
fn limit_circle(f_long: f64, f_lat: f64, limit: f64) -> (f64, f64) {
    if limit <= 0.0 {
        return (0.0, 0.0);
    }
    let combined = f_long.hypot(f_lat);
    if !combined.is_finite() {
        return (0.0, 0.0);
    }
    if combined <= limit {
        return (f_long, f_lat);
    }
    let scale = limit / combined;
    (f_long * scale, f_lat * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::FlatSurface;
    use crate::vehicle::{MemoryPoints, VehicleData};
    use a3_config::parse_text;

    /// A car the size of the offroad, with its shipped numbers where the step reads them.
    const OFFROAD: &str = r#"
        class C_Offroad_01_F {
            simulation = "carx";
            maxSpeed = 200;
            wheelCircumference = 2.805;
            enginePower = 150;
            maxOmega = 450;
            minOmega = 100;
            brakeIdleSpeed = 1.78;
            thrustDelay = 0.5;
            differentialType = "all_limited";
            frontRearSplit = 0.5;
            frontBias = 1.5;
            rearBias = 1.5;
            centreBias = 1.3;
            changeGearMinEffectivity[] = {1, 0.15, 1, 1, 1, 1, 1, 1};
            switchTime = 0.31;
            latency = 1.5;
            clutchStrength = 20.0;
            class complexGearbox {
                GearboxRatios[] = {"R1", -4, "N", 0, "D1", 4.5, "D2", 2.61, "D3", 1.51, "D4", 0.88};
                TransmissionRatios[] = {"High", 7};
            };
            class Wheels {
                class LF { side = "left"; steering = 1; center = "wheel_1_1_axis";
                           boundary = "wheel_1_1_bound"; width = 0.3; mass = 30; MOI = 6;
                           dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                           maxCompression = 0.05; maxDroop = 0.1; sprungMass = 400;
                           springStrength = 14400; springDamperRate = 3840;
                           longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                           latStiffY = 18;
                           frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
                class LR { side = "left"; steering = 0; center = "wheel_1_2_axis";
                           boundary = "wheel_1_2_bound"; width = 0.3; mass = 30; MOI = 6;
                           dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                           maxHandBrakeTorque = 3000; maxCompression = 0.05; maxDroop = 0.1;
                           sprungMass = 400; springStrength = 14400; springDamperRate = 3840;
                           longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                           latStiffY = 18;
                           frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
                class RF { side = "right"; steering = 1; center = "wheel_2_1_axis";
                           boundary = "wheel_2_1_bound"; width = 0.3; mass = 30; MOI = 6;
                           dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                           maxCompression = 0.05; maxDroop = 0.1; sprungMass = 400;
                           springStrength = 14400; springDamperRate = 3840;
                           longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                           latStiffY = 18;
                           frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
                class RR { side = "right"; steering = 0; center = "wheel_2_2_axis";
                           boundary = "wheel_2_2_bound"; width = 0.3; mass = 30; MOI = 6;
                           dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                           maxHandBrakeTorque = 3000; maxCompression = 0.05; maxDroop = 0.1;
                           sprungMass = 400; springStrength = 14400; springDamperRate = 3840;
                           longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                           latStiffY = 18;
                           frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
            };
        };"#;

    /// The car's geometry: a 2.2 x 1.9 x 4.3 box on a 1.4 m half-wheelbase, wheels at 0.8 m
    /// either side of the centre line.
    fn car() -> (VehicleData, DVec3) {
        let tree = a3_config::ConfigTree::from_config(&parse_text(OFFROAD).unwrap());
        let mut data = VehicleData::from_class(&tree.root().get("C_Offroad_01_F")).unwrap();
        let mut points = MemoryPoints::new();
        for (name, x, z) in [
            ("wheel_1_1", 0.8, 1.4),
            ("wheel_1_2", 0.8, -1.4),
            ("wheel_2_1", -0.8, 1.4),
            ("wheel_2_2", -0.8, -1.4),
        ] {
            points.insert(format!("{name}_axis"), DVec3::new(x, 0.3, z));
            points.insert(format!("{name}_bound"), DVec3::new(x, 0.3, z + 0.45));
        }
        let size = DVec3::new(2.2, 1.9, 4.3);
        data.resolve_geometry(&points, size);
        (data, size)
    }

    fn placed(data: &VehicleData, size: DVec3) -> WheeledVehicle {
        WheeledVehicle::new(data, DVec3::new(0.0, data.rest_height(), 0.0), 0.0, size)
    }

    #[test]
    fn a_vehicle_at_rest_height_stands_on_its_springs() {
        let (data, size) = car();
        let ground = FlatSurface::new(0.0, 1.0);
        let mut vehicle = placed(&data, size);
        for _ in 0..50 {
            vehicle.step(&data, &VehicleInput::idle(), &ground, 0.01);
        }
        // Every wheel is on the ground carrying its own sprung mass, and the chassis neither
        // falls nor rises.
        for index in 0..vehicle.wheels.len() {
            let state = vehicle.wheels[index];
            assert!(state.in_contact, "wheel {index} in the air");
            let weight = sprung_mass(&data, index) * GRAVITY;
            assert!(
                (state.load - weight).abs() < 1.0,
                "wheel {index} carries {} N, not {weight}",
                state.load
            );
        }
        assert!(
            (vehicle.body.position.y - data.rest_height()).abs() < 0.01,
            "settled at {}",
            vehicle.body.position.y
        );
        assert!(vehicle.speed() < 0.05, "{}", vehicle.speed());
    }

    #[test]
    fn the_limits_split_the_torque_by_the_bias_to_the_slower_side() {
        // Open (bias 1): even.
        assert_eq!(axle_shares(&[10.0, 20.0], 1.0), vec![0.5, 0.5]);
        // Limited at 1.5: the slower wheel takes 1.5/2.5 of it.
        let shares = axle_shares(&[10.0, 20.0], 1.5);
        assert!((shares[0] - 0.6).abs() < 1e-12, "{shares:?}");
        assert!((shares[1] - 0.4).abs() < 1e-12, "{shares:?}");
        // Turning alike, even again; and a lone wheel takes everything.
        assert_eq!(axle_shares(&[10.0, 10.0], 1.5), vec![0.5, 0.5]);
        assert_eq!(axle_shares(&[10.0], 1.5), vec![1.0]);
    }

    #[test]
    fn the_friction_circle_holds_the_combined_force() {
        assert_eq!(limit_circle(3.0, 4.0, 10.0), (3.0, 4.0));
        let (long, lat) = limit_circle(6.0, 8.0, 5.0);
        assert!((long.hypot(lat) - 5.0).abs() < 1e-12);
        assert!((long - 3.0).abs() < 1e-12, "the direction is kept");
        assert_eq!(limit_circle(6.0, 8.0, 0.0), (0.0, 0.0));
        assert_eq!(limit_circle(f64::INFINITY, 0.0, 5.0), (0.0, 0.0));
    }
}
