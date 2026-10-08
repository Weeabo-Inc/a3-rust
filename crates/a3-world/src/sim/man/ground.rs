//! The ground a Man stands on and the motion that follows it.
//!
//! The engine asks the world for the surface under a Man's feet — the terrain, or the Roadway
//! LOD face of an object (a bridge, a ramp, a house floor) — and walks him along it, letting him
//! fall when there is nothing within reach (`docs/re/sim-man-movement.md` §5).
//!
//! [`GroundQuery`] is the seam for that query: this module answers it from the terrain, and the
//! physics of #123 replaces it with the full surface query (roadways, step-up heights, dynamic
//! objects) without the Man family changing.

use glam::DVec3;

/// The highest surface a Man climbs in one step without jumping (metres).
///
/// _Placeholder_ until the original's step-up height is traced (issue #191 follow-up); it is
/// the value that keeps him off kerbs and low ledges. `docs/re/sim-man-locomotion.md`.
pub const MAX_STEP_UP: f64 = 0.5;

/// The deepest drop a Man walks down without leaving the ground (metres). A taller drop starts
/// a fall.
pub const MAX_STEP_DOWN: f64 = 0.5;

/// Gravity (m/s²). The original uses the same value for characters and physics objects.
pub const GRAVITY: f64 = 9.81;

/// A walkable surface under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundContact {
    /// Surface height at `(x, z)`, in metres above sea level.
    pub height: f64,
    /// Unit surface normal in world space (level ground is `+Y`). Its tilt is the slope a Man
    /// walks on.
    pub normal: DVec3,
}

/// The world surface a Man walks on.
pub trait GroundQuery {
    /// The surface to stand on at `(x, z)` for a Man whose feet are at `from_y`: the highest
    /// one not above `from_y + MAX_STEP_UP` in the original, so he walks under a bridge as
    /// readily as over it. `None` when there is nothing there (he falls).
    fn ground(&self, x: f64, z: f64, from_y: f64) -> Option<GroundContact>;
}

impl GroundQuery for a3_wrp::Terrain {
    fn ground(&self, x: f64, z: f64, _from_y: f64) -> Option<GroundContact> {
        let (x, z) = (x as f32, z as f32);
        Some(GroundContact {
            height: f64::from(self.surface_height(x, z)),
            normal: surface_normal(self, x, z),
        })
    }
}

/// The normal of the terrain triangle under `(x, z)`, from the same three corner heights
/// [`a3_wrp::Terrain::surface_height`] interpolates between.
fn surface_normal(terrain: &a3_wrp::Terrain, x: f32, z: f32) -> DVec3 {
    let cell = f64::from(terrain.terrain_cell_size());
    let inv = 1.0 / cell;
    let (gx, gz) = (f64::from(x) * inv, f64::from(z) * inv);
    let (fi, fj) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - fi, gz - fj);
    let (i, j) = (fi as i64, fj as i64);
    let h = |i: i64, j: i64| f64::from(terrain.grid_height(i, j));
    let (h00, h10, h01, h11) = (h(i, j), h(i + 1, j), h(i, j + 1), h(i + 1, j + 1));
    // Each cell is split along the diagonal (i + 1, j) -> (i, j + 1); the corners of the
    // triangle are (0, h00, 0), (cell, h10, 0), (0, h01, cell) in (x, y, z), or the three
    // lower ones in the other half of the cell.
    let (a, b, c) = if fx + fz <= 1.0 {
        (
            DVec3::new(0.0, h00, 0.0),
            DVec3::new(cell, h10, 0.0),
            DVec3::new(0.0, h01, cell),
        )
    } else {
        (
            DVec3::new(0.0, h01, cell),
            DVec3::new(cell, h10, 0.0),
            DVec3::new(cell, h11, cell),
        )
    };
    let normal = (b - a).cross(c - a).normalize();
    // The cross product of the corner order above points down; flip it to face the sky.
    if normal.y < 0.0 { -normal } else { normal }
}

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
    /// decides his height. Ground within a step is followed up or down, a taller step stops him,
    /// a deeper drop starts a fall, and with nothing under him he accelerates under gravity and
    /// lands on the first surface he reaches. The vertical part of `velocity` is ignored: the
    /// ground or gravity owns the height _(the original lets an animation's own rise play out;
    /// #191 follow-up)_.
    pub fn step(
        &mut self,
        feet: DVec3,
        velocity: DVec3,
        ground: &dyn GroundQuery,
        dt: f64,
    ) -> DVec3 {
        let moved = feet + DVec3::new(velocity.x, 0.0, velocity.z) * dt;
        if self.on_ground {
            match ground.ground(moved.x, moved.z, feet.y) {
                Some(contact)
                    if contact.height <= feet.y + MAX_STEP_UP
                        && contact.height >= feet.y - MAX_STEP_DOWN =>
                {
                    moved.with_y(contact.height)
                }
                // A step taller than a step: he is stopped by it.
                Some(contact) if contact.height > feet.y + MAX_STEP_UP => feet,
                // A drop: he walks off the edge and falls from where he was.
                _ => {
                    self.on_ground = false;
                    self.vertical_speed = 0.0;
                    moved.with_y(feet.y)
                }
            }
        } else {
            self.vertical_speed -= GRAVITY * dt;
            let next = moved.with_y(feet.y + self.vertical_speed * dt);
            match ground.ground(next.x, next.z, feet.y) {
                Some(contact) if next.y <= contact.height => {
                    self.on_ground = true;
                    self.vertical_speed = 0.0;
                    next.with_y(contact.height)
                }
                _ => next,
            }
        }
    }
}
