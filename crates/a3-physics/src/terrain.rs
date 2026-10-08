//! The terrain surface: exact heights and ray casts over the whole heightmap, and heightfield
//! chunks for rapier contacts.
//!
//! Each height cell is split into two triangles along the diagonal from `(i + 1, j)` to
//! `(i, j + 1)`, as the engine's `SurfaceY` does (`docs/re/wrp.md`). parry's heightfield splits
//! its cells along the same diagonal, so rapier contacts and our ray casts agree.

use std::sync::Arc;

use a3_wrp::Terrain;
use glam::DVec3;
use rapier3d_f64::parry::utils::Array2;
use rapier3d_f64::prelude::SharedShape;

/// Height cells per side of one heightfield chunk.
pub(crate) const CHUNK_CELLS: u32 = 32;

/// Where a ray met the terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainHit {
    /// Distance along the ray (the direction's length is the unit).
    pub distance: f64,
    pub position: DVec3,
    /// The upward normal of the triangle that was hit.
    pub normal: DVec3,
}

/// The terrain heightmap as a collision surface.
#[derive(Debug, Clone)]
pub struct TerrainShape {
    terrain: Arc<Terrain>,
    cell: f64,
    /// Height samples per side.
    samples: u32,
}

impl TerrainShape {
    pub fn new(terrain: Arc<Terrain>) -> Self {
        let cell = f64::from(terrain.terrain_cell_size());
        let samples = terrain.heightmap.width();
        Self {
            terrain,
            cell,
            samples,
        }
    }

    pub fn terrain(&self) -> &Arc<Terrain> {
        &self.terrain
    }

    /// Size of one height cell in metres.
    pub fn cell_size(&self) -> f64 {
        self.cell
    }

    /// Edge length of the terrain square in metres.
    pub fn size(&self) -> f64 {
        self.cell * f64::from(self.samples)
    }

    fn h(&self, i: i64, j: i64) -> f64 {
        f64::from(self.terrain.grid_height(i, j))
    }

    /// The cell `(i, j)` and the position inside it, `0..1` on each axis.
    fn locate(&self, x: f64, z: f64) -> (i64, i64, f64, f64) {
        let (gx, gz) = (x / self.cell, z / self.cell);
        let (fi, fj) = (gx.floor(), gz.floor());
        (fi as i64, fj as i64, gx - fi, gz - fj)
    }

    /// The surface height at world `(x, z)`, in `f64` (the engine computes the same in `f32`).
    /// Outside the grid the edge heights extend outwards.
    pub fn height(&self, x: f64, z: f64) -> f64 {
        let (i, j, fx, fz) = self.locate(x, z);
        let [h00, h10, h01, h11] = self.corners(i, j);
        if fx + fz <= 1.0 {
            h00 + (h10 - h00) * fx + (h01 - h00) * fz
        } else {
            (h01 + h10 - h11) + (h11 - h01) * fx + (h11 - h10) * fz
        }
    }

    /// The upward unit normal of the surface triangle under `(x, z)`.
    pub fn normal(&self, x: f64, z: f64) -> DVec3 {
        let (i, j, fx, fz) = self.locate(x, z);
        let [h00, h10, h01, h11] = self.corners(i, j);
        let (dx, dz) = if fx + fz <= 1.0 {
            (h10 - h00, h01 - h00)
        } else {
            (h11 - h01, h11 - h10)
        };
        DVec3::new(-dx, self.cell, -dz).normalize()
    }

    fn corners(&self, i: i64, j: i64) -> [f64; 4] {
        [
            self.h(i, j),
            self.h(i + 1, j),
            self.h(i, j + 1),
            self.h(i + 1, j + 1),
        ]
    }

    /// The first crossing of the surface (from either side) along `origin + dir * t` for
    /// `0 <= t <= max`. `dir` need not be normalised; distances are in units of its length.
    /// Only the terrain square `0..size` is searched (the engine synthesises terrain outside it;
    /// we do not).
    pub fn cast_ray(&self, origin: DVec3, dir: DVec3, max: f64) -> Option<TerrainHit> {
        let size = self.size();
        // Clip to the terrain square in x/z.
        let (mut t0, mut t1) = (0.0f64, max);
        for (o, d) in [(origin.x, dir.x), (origin.z, dir.z)] {
            if d.abs() < 1e-12 {
                if o < 0.0 || o > size {
                    return None;
                }
            } else {
                let (a, b) = ((0.0 - o) / d, (size - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if t0 > t1 {
            return None;
        }
        // Walk the cells the x/z projection crosses (Amanatides & Woo).
        let inv = 1.0 / self.cell;
        let start = origin + dir * t0;
        let last = i64::from(self.samples) - 1;
        let mut i = ((start.x * inv).floor() as i64).clamp(0, last);
        let mut j = ((start.z * inv).floor() as i64).clamp(0, last);
        let step = |d: f64| if d > 0.0 { 1 } else { -1 };
        let next = |cell: i64, o: f64, d: f64| {
            if d.abs() < 1e-12 {
                f64::INFINITY
            } else {
                let edge = if d > 0.0 { cell + 1 } else { cell } as f64 * self.cell;
                (edge - o) / d
            }
        };
        let delta = |d: f64| {
            if d.abs() < 1e-12 {
                f64::INFINITY
            } else {
                self.cell / d.abs()
            }
        };
        let (si, sj) = (step(dir.x), step(dir.z));
        let (mut tx, mut tz) = (next(i, origin.x, dir.x), next(j, origin.z, dir.z));
        let (dtx, dtz) = (delta(dir.x), delta(dir.z));
        let mut t_enter = t0;
        loop {
            let t_exit = tx.min(tz).min(t1);
            if let Some(hit) = self.cast_cell(i, j, origin, dir, t_enter, t_exit) {
                return Some(hit);
            }
            if t_exit >= t1 {
                return None;
            }
            if tx < tz {
                i += si;
                tx += dtx;
            } else {
                j += sj;
                tz += dtz;
            }
            if i < 0 || j < 0 || i > last || j > last {
                return None;
            }
            t_enter = t_exit;
        }
    }

    fn cast_cell(
        &self,
        i: i64,
        j: i64,
        origin: DVec3,
        dir: DVec3,
        t_min: f64,
        t_max: f64,
    ) -> Option<TerrainHit> {
        let [h00, h10, h01, h11] = self.corners(i, j);
        let (x0, z0) = (i as f64 * self.cell, j as f64 * self.cell);
        let (x1, z1) = (x0 + self.cell, z0 + self.cell);
        let p00 = DVec3::new(x0, h00, z0);
        let p10 = DVec3::new(x1, h10, z0);
        let p01 = DVec3::new(x0, h01, z1);
        let p11 = DVec3::new(x1, h11, z1);
        let eps = 1e-9 * (1.0 + t_max.abs());
        [(p00, p10, p01), (p11, p01, p10)]
            .into_iter()
            .filter_map(|(a, b, c)| ray_triangle(origin, dir, a, b, c))
            .filter(|&(t, _)| t >= t_min - eps && t <= t_max + eps)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(t, mut n)| {
                if n.y < 0.0 {
                    n = -n;
                }
                TerrainHit {
                    distance: t,
                    position: origin + dir * t,
                    normal: n,
                }
            })
    }

    /// Number of heightfield chunks per side.
    pub(crate) fn chunks_per_side(&self) -> u32 {
        self.samples.div_ceil(CHUNK_CELLS)
    }

    /// Edge length of a chunk in metres.
    pub(crate) fn chunk_size(&self) -> f64 {
        self.cell * f64::from(CHUNK_CELLS)
    }

    /// The heightfield of chunk `(cx, cz)` and the world position of its centre.
    pub(crate) fn chunk(&self, cx: u32, cz: u32) -> (SharedShape, DVec3) {
        let n = CHUNK_CELLS as usize + 1;
        let (i0, j0) = (i64::from(cx * CHUNK_CELLS), i64::from(cz * CHUNK_CELLS));
        // Column-major, rows along z and columns along x.
        let mut data = Vec::with_capacity(n * n);
        for col in 0..n as i64 {
            for row in 0..n as i64 {
                data.push(self.h(i0 + col, j0 + row));
            }
        }
        let size = self.chunk_size();
        let shape = SharedShape::heightfield(
            Array2::new(n, n, data),
            crate::conv::vec(DVec3::new(size, 1.0, size)),
        );
        let centre = DVec3::new(
            (i0 as f64) * self.cell + size / 2.0,
            0.0,
            (j0 as f64) * self.cell + size / 2.0,
        );
        (shape, centre)
    }
}

/// Intersection of a ray with a triangle from either side: `(t, unit normal)`.
fn ray_triangle(o: DVec3, d: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<(f64, DVec3)> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    const TOL: f64 = 1e-9;
    if !(-TOL..=1.0 + TOL).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -TOL || u + v > 1.0 + TOL {
        return None;
    }
    let t = e2.dot(q) * inv;
    Some((t, e1.cross(e2).normalize()))
}
