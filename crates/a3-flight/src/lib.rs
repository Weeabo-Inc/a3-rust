//! Flight models: how a helicopter or a plane turns its pilot's controls into forces, and the
//! forces into motion.
//!
//! - [`heli`]: the engine's **basic** helicopter model (`helicopterrtd` with the advanced flight
//!   model off, the default): rotor spool and autorotation, collective lift over the
//!   `envelope`, cyclic and tail-rotor torques, body friction, damage of `HitHRotor` /
//!   `HitVRotor` / `HitEngine`.
//! - [`plane`]: the `airplanex` aero model: thrust, lift over `envelope` and angle of attack,
//!   control surfaces, the "draconic" weathervane terms, stall, air friction, flaps, airbrake,
//!   gear and the landing-gear wheels.
//! - [`input`]: the pilot's user actions (`heliCollectiveRaise`, `heliCyclicForward`, ...) as
//!   the original reads them.
//! - [`body`]: the rigid body both integrate, with the original's friction integrator.
//! - [`airframe`]: what the model (ODOL) gives a flight model: mass, inertia, bounding radius,
//!   geometry points and memory points.
//! - [`ground`]: what the host answers about the surface below, and the contact forces.
//!
//! Every formula comes from `arma3_x64.exe` and is written down in `docs/re/sim-air.md`, which
//! the code follows section by section. The crate is pure: no world, no collision world, no
//! clock. `a3-world`'s air family steps it once per simulation step (1/15 s, the original's
//! precision for PhysX vehicles).
//!
//! # A step
//!
//! ```text
//! state.pilot(&input, &body);                       // player: user actions → wanted controls
//! heli::step(&ty, &airframe, &mut state, &mut body, &env, dt);
//! ```

pub mod airframe;
pub mod body;
pub mod ground;
pub mod heli;
pub mod input;
pub mod plane;
pub mod table;

pub use airframe::Airframe;
pub use body::{Loads, RigidBody};
pub use ground::{Environment, FlatGround, Ground};
pub use input::FlightInput;

/// Standard gravity, m/s² (`G_CONST`; plane lift uses it explicitly).
pub const GRAVITY: f64 = 9.8066;

/// km/h to m/s, as the engine converts `maxSpeed` (`0.2777778`).
pub const KMH: f64 = 1.0 / 3.6;

/// Moves `value` towards `target` by at most `down` (a positive step) downwards and `up`
/// upwards: the engine's rate limiter for every control.
pub(crate) fn approach(value: f64, target: f64, down: f64, up: f64) -> f64 {
    value + (target - value).clamp(-down, up)
}

/// `-1`, `0` or `1`, as the engine's friction formulas use the sign (0 at exactly 0).
pub(crate) fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The altitude factor: 1 up to `full`, linear to 0 at `none` (heights ASL).
pub(crate) fn altitude_factor(height: f64, full: f64, none: f64) -> f64 {
    if height <= full {
        1.0
    } else if height <= none {
        1.0 - (height - full) / (none - full)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_is_limited_by_the_rate_in_each_direction() {
        assert_eq!(approach(0.0, 1.0, 0.7, 0.4), 0.4);
        assert_eq!(approach(1.0, 0.0, 0.7, 0.4), 0.30000000000000004);
        assert_eq!(approach(0.5, 0.6, 0.7, 0.4), 0.6);
    }

    #[test]
    fn sign_is_zero_at_zero() {
        assert_eq!(sign(0.0), 0.0);
        assert_eq!(sign(-3.0), -1.0);
        assert_eq!(sign(2.0), 1.0);
    }

    #[test]
    fn the_altitude_factor_fades_between_full_and_no_force() {
        assert_eq!(altitude_factor(500.0, 1000.0, 3000.0), 1.0);
        assert_eq!(altitude_factor(2000.0, 1000.0, 3000.0), 0.5);
        assert_eq!(altitude_factor(3500.0, 1000.0, 3000.0), 0.0);
    }
}
