//! The ground a Man stands on and the motion that follows it.
//!
//! The engine asks the world for the surface under a Man's feet — the terrain, or the Roadway
//! LOD face of an object (a bridge, a ramp, a house floor) — and walks him along it, letting him
//! fall when there is nothing within reach (`docs/re/sim-man-movement.md` §5).
//!
//! That question is [`CollisionWorld::surface_below`], the engine's one surface query
//! (`docs/adr/0008-collision-world.md`): a Man is one of the things that drives the collision
//! world, and the terrain is only one of the surfaces in it.

use a3_physics::CollisionWorld;
use glam::DVec3;

/// Gravity (m/s²): the physics world's constant, which the original uses for characters and
/// physics objects alike.
pub use a3_physics::GRAVITY;

/// The highest surface a Man climbs in one step without jumping (metres).
///
/// _Placeholder_ until the original's step-up height is traced (issue #191 follow-up); it is
/// the value that keeps him off kerbs and low ledges. `docs/re/sim-man-locomotion.md`.
pub const MAX_STEP_UP: f64 = 0.5;

/// The deepest drop a Man walks down without leaving the ground (metres). A taller drop starts
/// a fall.
pub const MAX_STEP_DOWN: f64 = 0.5;

/// How a Man is standing on, or falling to, the ground. Kept per Man and advanced by
/// [`Motion::step`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// His feet are on a surface.
    pub on_ground: bool,
    /// Vertical speed in m/s, negative downwards. Always 0 while [`Motion::on_ground`].
    pub vertical_speed: f64,
}

impl Default for Motion {
    /// A Man starts standing on the ground.
    fn default() -> Self {
        Self {
            on_ground: true,
            vertical_speed: 0.0,
        }
    }
}

impl Motion {
    /// Moves the feet from `feet` over `dt` seconds.
    ///
    /// The horizontal part of `velocity` (world m/s, from the animation) carries him; the ground
    /// decides his height. He steps up or down to a surface within [`MAX_STEP_UP`] /
    /// [`MAX_STEP_DOWN`] of his feet, walks off a taller edge and falls, accelerates under
    /// gravity with nothing under him, and lands on the first surface this step reaches. The
    /// vertical part of `velocity` is ignored: the ground or gravity owns the height _(the
    /// original lets an animation's own rise play out; #191 follow-up)_.
    ///
    /// Only surfaces a Man can walk on answer: a Roadway (a deck, a floor, stairs) or the
    /// terrain. Geometry — walls, the sides of Objects — is not asked; a Man is stopped by those
    /// in the character controller (issue #123), not here.
    pub fn step(
        &mut self,
        feet: DVec3,
        velocity: DVec3,
        ground: &CollisionWorld,
        dt: f64,
    ) -> DVec3 {
        let moved = feet + DVec3::new(velocity.x, 0.0, velocity.z) * dt;
        if self.on_ground {
            // The band a step takes him through: from MAX_STEP_UP over his feet down to
            // MAX_STEP_DOWN under them. The highest surface in it is the one he stands on; a
            // deck above the band is walked under rather than climbed.
            let probe = moved.with_y(feet.y + MAX_STEP_UP);
            if let Some(surface) = ground.surface_below(probe, MAX_STEP_UP + MAX_STEP_DOWN) {
                return moved.with_y(surface.y);
            }
            // Nothing within reach: he walks off the edge and falls from where he was.
            self.on_ground = false;
            self.vertical_speed = 0.0;
            moved.with_y(feet.y)
        } else {
            self.vertical_speed -= GRAVITY * dt;
            let next = moved.with_y(feet.y + self.vertical_speed * dt);
            // From just over his feet down to where this step took him: the first surface in
            // that band catches him, so even a fall that crosses the surface in one step lands.
            let probe = next.with_y(feet.y + MAX_STEP_UP);
            match ground.surface_below(probe, probe.y - next.y) {
                Some(surface) => {
                    self.on_ground = true;
                    self.vertical_speed = 0.0;
                    next.with_y(surface.y)
                }
                None => next,
            }
        }
    }
}
