//! The turn of a Man: how his front swings towards where he is asked to turn
//! (`docs/re/sim-man-locomotion.md` §2).

use std::f64::consts::FRAC_PI_2;

/// How fast the turn he applies follows what he asks for, per second: the engine clamps the
/// change to `6 · dt` per step (`docs/re/sim-man-locomotion.md` §2). _The unit of the engine's
/// turn value is not established_ (`6 rad/s` if it is radians), so this is the same number read
/// as "of full deflection per second".
const TURN_RAMP: f64 = 6.0;

/// A full deflection at `turnSpeed` 1 turns him a quarter turn per second. The moves type's
/// `turnSpeed` is the per-move turn limit, but where the engine applies it and in which unit is
/// **not traced** (`docs/re/sim-man-locomotion.md` §2); a man's own turn rate divided this way
/// is the plainest reading of it.
const FULL_TURN: f64 = FRAC_PI_2;

/// The turn of one Man: he follows the turn he is asked for at a limited rate, and the move he
/// plays says how fast a full deflection turns him.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Turning {
    /// What he is turning at now, `-1.0..=1.0`: what he has caught up with of the turn asked
    /// for.
    applied: f64,
}

impl Turning {
    /// One step of the turn, returning the yaw to turn him by this step in radians.
    ///
    /// `wanted` is [`super::ManInput::turn`], `turn_speed` the current move's `turnSpeed`.
    /// Positive is to his right, which is the World's `orientation = yaw(angle) · orientation` —
    /// north towards east.
    pub fn step(&mut self, wanted: f32, turn_speed: f32, dt: f64) -> f64 {
        let limit = TURN_RAMP * dt;
        self.applied += (f64::from(wanted) - self.applied).clamp(-limit, limit);
        self.applied * f64::from(turn_speed) * FULL_TURN * dt
    }
}
