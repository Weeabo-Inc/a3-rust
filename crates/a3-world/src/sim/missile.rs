//! Missiles: the guided flight of `Missile::Simulate` (`docs/re/sim-ballistics.md` §3.1).
//!
//! A missile is a thrust-vectoring body. The motor burns along the body's +z axis, the guidance
//! turns the body about its lateral axes, and the velocity follows the thrust; the body frame is
//! the Entity's orientation. One step of `dt`:
//!
//! 1. the model-space velocity is the world velocity in the body frame (`A·v`);
//! 2. the drag of §3.1 is a **deceleration magnitude** per body axis, subtracted below and never
//!    added; the `0x40` lock type (a rocket with no `thrustTime`) adds its own lateral terms to
//!    the acceleration and feeds the plain lateral drag into the body's turn, which is how a
//!    finned rocket stabilises itself;
//! 3. the motor walks its `initTime`/`thrustTime` phases: while it burns, the axial drag is
//!    replaced by `k·frac·thrust` and the lateral one cancels out of the step;
//! 4. the guidance (`FUN_140e59d50`) gives an angular acceleration about the body's x and y axes
//!    from the error between the flight direction and the direction to the target's led aim
//!    point, when the shot has a target and the axial speed is at least 30 m/s;
//! 5. the body's world angular velocity is `ω' = guide − 5·ω` and the body turns by `ω·dt`;
//! 6. the acceleration is taken to the world frame, gravity is added there, and the velocity
//!    steps; the drag is then subtracted per body axis, never reversing it.
//!
//! The position advance and the impact are the caller's: a missile runs the same segment test,
//! ricochet and explosion as every other shot (§3, §4), with the ShotShell velocity step switched
//! off because the drag above is its own.

use glam::{DQuat, DVec3};

use crate::weapons::AmmoType;
use crate::{Entity, ObjectRef, World};

use super::ProjectileState;

/// Gravity (`9.8066` in `0x140e75130`), m/s². A missile ignores `coefGravity`: the engine reads a
/// plain `k·9.8066` (§3.1).
const GRAVITY: f64 = 9.8066;

/// The body's angular velocity decays at this rate (`FUN_140e845f0` subtracts `5·ω·dt`), 1/s.
const ANGULAR_DAMPING: f64 = 5.0;

/// The guidance runs only at or above this axial speed (`30.0` in `Missile::Simulate`), m/s.
const MIN_GUIDED_SPEED: f64 = 30.0;

/// The seeker's longest lead time (`0.3` s in `FUN_140e55730`).
const MAX_LEAD_TIME: f64 = 0.3;

/// `k = FUN_140e85230(shot)`, the scale of the drag (`k·0.1`) and the gravity (`k·9.8066`). Only
/// `1` makes the gravity the physical `9.8066·dt` per step, which is what the shipped missiles
/// get (§3.1).
const K: f64 = 1.0;

/// A missile step longer than this is cut short, as an object that must not jump through a wall.
const MAX_STEP: f64 = 0.05;

/// The motor's phase (`shot + 0x74c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotorPhase {
    /// Waiting out `initTime` (`+0x740`) before the motor can light.
    #[default]
    Init,
    /// Burning: `thrustTime` (`+0x744`) counts down and the thrust ramps down over its last
    /// quarter.
    Burning,
    /// Burnt out, or never lit.
    BurntOut,
}

/// The guided flight of a missile (`ClassState::Projectile`'s `ProjectileState::missile`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissileState {
    /// The Object the guidance steers at (the engine's target at `shot + 0x758`); `None` flies
    /// the shot straight.
    pub target: Option<ObjectRef>,
    /// Seconds left of `initTime` before the motor can light (`shot + 0x740`).
    pub init_timer: f64,
    /// Seconds left of `thrustTime` (`shot + 0x744`).
    pub thrust_timer: f64,
    /// The motor's phase (`shot + 0x74c`).
    pub phase: MotorPhase,
    /// The body's angular velocity in **World** space, rad/s (`shot + 0x2cc`). The guidance
    /// drives it to `guide/5`.
    pub angular_velocity: DVec3,
    /// The World acceleration of the last step (`block + 0x6c`), which the seeker's velocity
    /// prediction reads.
    pub acceleration: DVec3,
    /// The last guidance command about the body's x and y axes, rad/s² (`shot + 0x79c`,
    /// `+0x7a0`), for the flight to be inspected and tested.
    pub guidance: [f64; 2],
}

impl MissileState {
    /// The state of a shot of `ammo`, with the motor waiting out its `initTime`.
    pub fn new(ammo: &AmmoType) -> Self {
        Self {
            target: None,
            init_timer: ammo.init_time,
            thrust_timer: ammo.thrust_time,
            phase: MotorPhase::Init,
            angular_velocity: DVec3::ZERO,
            acceleration: DVec3::ZERO,
            guidance: [0.0, 0.0],
        }
    }
}

/// One missile step (`Missile::Simulate`, `FUN_140e59ab0`, `FUN_140e59d50` and `FUN_140e75130`):
/// the motor, the drag, the guidance and the body's turn. The caller then moves the shot and
/// resolves what it meets.
pub(crate) fn step(
    entity: &mut Entity,
    state: &mut ProjectileState,
    ammo: &AmmoType,
    world: &World,
    dt: f64,
) {
    let Some(missile) = state.missile.as_mut() else {
        return;
    };
    let dt = dt.min(MAX_STEP);
    let orientation = entity.orientation;
    let to_model = orientation.conjugate();
    let model_velocity = to_model * entity.velocity;

    // 1. The drag, per body axis, in m/s² (§3.1). The motor gets a **copy** of it below, which it
    // rewrites; the deceleration actually subtracted at the end of the step is this one.
    let drag = DVec3::new(
        lateral_drag(model_velocity.x, ammo.side_air_friction),
        lateral_drag(model_velocity.y, ammo.side_air_friction),
        axial_drag(model_velocity.z, ammo.air_friction),
    );
    let mut motor = drag;
    let mut accel = DVec3::ZERO;
    // The step's angular command, in the body frame, rad/s² (`local_28d8` in the original).
    let mut guide = DVec3::ZERO;

    // 2. The unguided-rocket model (lock type 0x40).
    if ammo.uses_advanced_drag() {
        accel.x +=
            (model_velocity.x * -0.005 - model_velocity.x.abs() * model_velocity.x * 0.00033) * K;
        accel.y +=
            (model_velocity.y * -0.005 - model_velocity.y.abs() * model_velocity.y * 0.00033) * K;
        guide.x += drag.y * -0.03;
        guide.y += drag.x * 0.03;
    }

    // 3. The motor (§3.1).
    match missile.phase {
        MotorPhase::Init => {
            missile.init_timer -= dt;
            if missile.init_timer < 0.0 {
                missile.phase = if missile.thrust_timer <= 0.0 {
                    MotorPhase::BurntOut
                } else {
                    MotorPhase::Burning
                };
            }
        }
        MotorPhase::Burning => {
            missile.thrust_timer -= dt;
            if missile.thrust_timer >= 0.0 {
                let frac = if ammo.thrust_time > 0.0 {
                    (missile.thrust_timer * 4.0 / ammo.thrust_time).min(1.0)
                } else {
                    1.0
                };
                // The motor's copy: the lateral drag is dropped from the step and the axial one is
                // replaced by the thrust, both of which the acceleration then takes over.
                motor.x = 0.0;
                accel.y += motor.y;
                motor.z = K * frac * ammo.thrust;
                accel.z += motor.z;
            } else {
                missile.phase = MotorPhase::BurntOut;
            }
        }
        MotorPhase::BurntOut => {}
    }

    // 4. The guidance.
    missile.guidance = [0.0, 0.0];
    if model_velocity.z >= MIN_GUIDED_SPEED
        && let Some(command) = guidance(entity, missile, ammo, world)
    {
        missile.guidance = [command.angular.x, command.angular.y];
        guide += command.angular;
        // The unguided-rocket model also takes the command as a lateral acceleration (§3.1).
        accel += command.accel;
    }

    // 5. The body's angular velocity: `ω' = guide − 5·ω` (`FUN_140e845f0`), then its turn.
    let before = missile.angular_velocity;
    let mut omega = before + orientation * guide * dt;
    let damp = before * ANGULAR_DAMPING * dt;
    for i in 0..3 {
        if omega[i] * damp[i] > 0.0 {
            omega[i] = if omega[i].abs() <= damp[i].abs() {
                0.0
            } else {
                omega[i] - damp[i]
            };
        }
    }
    missile.angular_velocity = omega;

    // 6. The acceleration in the world frame, gravity, and the velocity step.
    let world_accel = orientation * accel - DVec3::Y * (GRAVITY * K);
    missile.acceleration = world_accel;
    let mut velocity = model_velocity + to_model * world_accel * dt;
    for i in 0..3 {
        if velocity[i] * drag[i] > 0.0 {
            let loss = drag[i] * dt;
            velocity[i] = if velocity[i].abs() <= loss.abs() {
                0.0
            } else {
                velocity[i] - loss
            };
        }
    }
    entity.velocity = orientation * velocity;

    // 7. The turn. The engine turns a PhysX body and the flight path follows it through the thrust
    // and the aerodynamic coupling; a Kinematic shot has no rigid body here, so the flight
    // direction turns with the body — a coordinated turn at the body's own rate.
    if omega != DVec3::ZERO {
        let turn = DQuat::from_scaled_axis(omega * dt);
        entity.orientation = (turn * orientation).normalize();
        entity.velocity = turn * entity.velocity;
    }
}

/// One guidance command, in the body frame: the angular acceleration about the body's x and y
/// axes (`FUN_140e59d50`'s effect on the shot's angular velocity) and, for the unguided-rocket
/// lock type, a lateral acceleration.
#[derive(Debug, Clone, Copy, Default)]
struct Guidance {
    /// rad/s².
    angular: DVec3,
    /// m/s², only for lock type `0x40`.
    accel: DVec3,
}

/// The guidance (`FUN_140e59d50`, §3.1). `None` without a target the World still knows.
fn guidance(
    entity: &Entity,
    missile: &MissileState,
    ammo: &AmmoType,
    world: &World,
) -> Option<Guidance> {
    let (target_position, target_velocity) = aim_point(world, missile.target?)?;
    let to_model = entity.orientation.conjugate();
    let model_velocity = to_model * entity.velocity;

    // The seeker (`FUN_140e55730`): the lead time and the target's aim point ahead of it.
    let offset = target_position - entity.position;
    let speed = model_velocity
        .z
        .max(ammo.max_speed * 0.3 + model_velocity.z * 0.7);
    let lead = if speed > 0.0 {
        (offset.length() / speed).min(MAX_LEAD_TIME)
    } else {
        MAX_LEAD_TIME
    };
    let aim = target_position + target_velocity * (lead * ammo.track_lead) - entity.position;

    // The flight direction, blended with the missile's own axis, and the direction to the target.
    let predicted = entity.velocity + missile.acceleration * lead;
    let u = (to_model * predicted).normalize_or_zero();
    let blend = (0.3 * ammo.maneuvrability).clamp(0.5, 0.95);
    let b = (blend * u + DVec3::Z * (1.0 - blend)).normalize_or_zero();
    let p = (to_model * aim).normalize_or_zero();

    let s = (0.02 * model_velocity.z).clamp(0.1, 3.0);
    let limit = (0.25 * ammo.maneuvrability).max(0.0);
    let oversteer = 20.0 * ammo.track_oversteer * 3.0 / s;
    // A target above the nose is reached by pitching up, a negative turn about x; a target to the
    // right by yawing right, a positive turn about y. The same errors drive a lateral
    // acceleration straight at the target for the lock type that uses one.
    let pitch = (-(p.y - b.y) * oversteer).clamp(-limit, limit);
    let yaw = ((p.x - b.x) * oversteer).clamp(-limit, limit);
    let scale = K * s * 0.04 * ammo.maneuvrability;
    Some(Guidance {
        angular: DVec3::new(pitch * scale, yaw * scale, 0.0),
        accel: if ammo.uses_advanced_drag() {
            DVec3::new(K * s * yaw, K * s * pitch, 0.0)
        } else {
            DVec3::ZERO
        },
    })
}

/// The aim point and velocity of a target Object (`vfunc +0x6c0` and `vfunc +0x488`): an
/// Entity's position and velocity, a Static object's position (a Static object does not move).
fn aim_point(world: &World, target: ObjectRef) -> Option<(DVec3, DVec3)> {
    match target {
        ObjectRef::Entity(id) => {
            let entity = world.entity(id).filter(|e| !e.is_deleted())?;
            Some((entity.position, entity.velocity))
        }
        ObjectRef::Static(key) => {
            let object = world.static_object(key)?;
            Some((object.position, DVec3::ZERO))
        }
    }
}

/// One lateral drag term (§3.1): `((|v|·v + v)·10 + v³·0.0005)·sideAirFriction·k·0.1`.
fn lateral_drag(v: f64, side_air_friction: f64) -> f64 {
    ((v.abs() * v + v) * 10.0 + v * v * v * 0.0005) * side_air_friction * 0.1 * K
}

/// The axial drag term (§3.1): `(|v|·v·0.01 + v³·1e-5 + 2v)·airFriction·k·0.1`.
fn axial_drag(v: f64, air_friction: f64) -> f64 {
    (v.abs() * v * 0.01 + v * v * v * 1e-5 + 2.0 * v) * air_friction * 0.1 * K
}
