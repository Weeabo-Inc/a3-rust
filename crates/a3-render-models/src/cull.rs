//! Visibility: the view frustum and a uniform grid over placed objects.
//!
//! Everything here works in camera-relative space (ADR 0003): the frustum comes from the
//! renderer's rotation-only view-projection, and grid cells are offset from the camera in
//! `f64` before testing.

use glam::{DVec3, Mat4, Vec3, Vec4};

/// The side and near planes of a camera-relative view-projection (reversed-Z infinite
/// projections have no far plane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// `xyz` unit normal pointing inside, `w` offset: inside when `n . p + w >= 0`.
    planes: [Vec4; 5],
}

impl Frustum {
    /// The frustum of `view_projection` (Direct3D clip space, `0 <= z <= w`).
    pub fn from_view_projection(m: Mat4) -> Frustum {
        let r = |i: usize| m.row(i);
        let planes = [
            r(3) + r(0),
            r(3) - r(0),
            r(3) + r(1),
            r(3) - r(1),
            r(3) - r(2),
        ]
        .map(|p| p / p.truncate().length().max(1e-12));
        Frustum { planes }
    }

    /// Whether a sphere (camera-relative centre) is at least partly inside.
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|p| p.truncate().dot(center) + p.w >= -radius)
    }

    /// Whether an axis-aligned box (camera-relative) is at least partly inside.
    pub fn intersects_box(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let n = p.truncate();
            let corner = Vec3::select(n.cmpge(Vec3::ZERO), max, min);
            n.dot(corner) + p.w >= 0.0
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct CellBounds {
    y_min: f64,
    y_max: f64,
    /// How far objects of the cell reach beyond its square in X and Z.
    pad: f64,
}

/// A uniform grid over the X/Z plane holding object indices, for distance and frustum queries.
#[derive(Debug, Clone)]
pub struct ObjectGrid {
    cell_size: f64,
    origin: (f64, f64),
    dims: (usize, usize),
    /// `items[cell_start[c]..cell_start[c + 1]]` are the objects of cell `c`.
    cell_start: Vec<u32>,
    items: Vec<u32>,
    bounds: Vec<CellBounds>,
}

impl ObjectGrid {
    /// A grid of `cell_size` metres over objects given as (index, position, bounding radius).
    pub fn build(cell_size: f64, objects: impl IntoIterator<Item = (u32, DVec3, f32)>) -> Self {
        let objects: Vec<_> = objects.into_iter().collect();
        let mut grid = ObjectGrid {
            cell_size,
            origin: (0.0, 0.0),
            dims: (0, 0),
            cell_start: vec![0],
            items: Vec::new(),
            bounds: Vec::new(),
        };
        if objects.is_empty() {
            return grid;
        }
        let (mut min_x, mut min_z) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_z) = (f64::MIN, f64::MIN);
        for (_, p, _) in &objects {
            (min_x, min_z) = (min_x.min(p.x), min_z.min(p.z));
            (max_x, max_z) = (max_x.max(p.x), max_z.max(p.z));
        }
        let nx = ((max_x - min_x) / cell_size).floor() as usize + 1;
        let nz = ((max_z - min_z) / cell_size).floor() as usize + 1;
        grid.origin = (min_x, min_z);
        grid.dims = (nx, nz);
        let cell_of = |p: DVec3| {
            let x = (((p.x - min_x) / cell_size) as usize).min(nx - 1);
            let z = (((p.z - min_z) / cell_size) as usize).min(nz - 1);
            z * nx + x
        };
        let mut counts = vec![0u32; nx * nz];
        let mut bounds = vec![
            CellBounds {
                y_min: f64::MAX,
                y_max: f64::MIN,
                pad: 0.0,
            };
            nx * nz
        ];
        for (_, p, r) in &objects {
            let c = cell_of(*p);
            counts[c] += 1;
            let r = f64::from(*r);
            let b = &mut bounds[c];
            b.y_min = b.y_min.min(p.y - r);
            b.y_max = b.y_max.max(p.y + r);
            b.pad = b.pad.max(r);
        }
        let mut start = Vec::with_capacity(nx * nz + 1);
        let mut total = 0u32;
        start.push(0);
        for c in &counts {
            total += c;
            start.push(total);
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; objects.len()];
        for (i, p, _) in &objects {
            let c = cell_of(*p);
            items[fill[c] as usize] = *i;
            fill[c] += 1;
        }
        grid.cell_start = start;
        grid.items = items;
        grid.bounds = bounds;
        grid
    }

    /// Number of objects in the grid.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` when the grid holds no objects.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Visit every object of the cells within `max_distance` (horizontally) of `eye` whose
    /// bounds intersect `frustum` (a frustum relative to `eye`). Objects are visited once each;
    /// callers do their own per-object test.
    pub fn query(
        &self,
        eye: DVec3,
        max_distance: f64,
        frustum: Option<&Frustum>,
        mut visit: impl FnMut(u32),
    ) {
        let (nx, nz) = self.dims;
        if nx == 0 {
            return;
        }
        let s = self.cell_size;
        let range = |centre: f64, origin: f64, n: usize| {
            let lo = ((centre - max_distance - origin) / s).floor().max(0.0) as usize;
            let hi = ((centre + max_distance - origin) / s).floor();
            if hi < 0.0 {
                return 0..0;
            }
            lo..(hi as usize).min(n - 1) + 1
        };
        let max2 = max_distance * max_distance;
        for z in range(eye.z, self.origin.1, nz) {
            for x in range(eye.x, self.origin.0, nx) {
                let c = z * nx + x;
                let (a, b) = (self.cell_start[c], self.cell_start[c + 1]);
                if a == b {
                    continue;
                }
                let x0 = self.origin.0 + x as f64 * s;
                let z0 = self.origin.1 + z as f64 * s;
                // Nearest point of the cell square to the eye.
                let dx = (eye.x - eye.x.clamp(x0, x0 + s)).abs();
                let dz = (eye.z - eye.z.clamp(z0, z0 + s)).abs();
                let pad = self.bounds[c].pad;
                let (dx, dz) = ((dx - pad).max(0.0), (dz - pad).max(0.0));
                if dx * dx + dz * dz > max2 {
                    continue;
                }
                if let Some(f) = frustum {
                    let bd = &self.bounds[c];
                    let min = DVec3::new(x0 - pad, bd.y_min, z0 - pad) - eye;
                    let max = DVec3::new(x0 + s + pad, bd.y_max, z0 + s + pad) - eye;
                    if !f.intersects_box(min.as_vec3(), max.as_vec3()) {
                        continue;
                    }
                }
                for &i in &self.items[a as usize..b as usize] {
                    visit(i);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_render::Camera;
    use glam::{DVec3, Vec3};

    /// Camera at the origin looking north (+Z), 16:9.
    fn frustum() -> Frustum {
        let camera = Camera::default();
        Frustum::from_view_projection(camera.view_projection(16.0 / 9.0))
    }

    #[test]
    fn spheres_in_front_are_visible_and_behind_are_not() {
        let f = frustum();
        assert!(f.intersects_sphere(Vec3::new(0.0, 0.0, 50.0), 1.0));
        assert!(!f.intersects_sphere(Vec3::new(0.0, 0.0, -50.0), 1.0));
        // Far out to the east, outside the ~107 degree horizontal field of view.
        assert!(!f.intersects_sphere(Vec3::new(500.0, 0.0, 50.0), 1.0));
        // ...unless big enough to reach into it.
        assert!(f.intersects_sphere(Vec3::new(500.0, 0.0, 50.0), 450.0));
        // Very far ahead: there is no far plane.
        assert!(f.intersects_sphere(Vec3::new(0.0, 0.0, 50_000.0), 1.0));
    }

    fn grid() -> ObjectGrid {
        // A row of objects every 100 m along the Z axis, from z = -1000 to 1000, at x = 15000.
        let items = (0..21).map(|i| {
            let z = -1000.0 + 100.0 * f64::from(i);
            (i, DVec3::new(15_000.0, 10.0, z), 5.0)
        });
        ObjectGrid::build(200.0, items)
    }

    #[test]
    fn grid_queries_return_objects_within_the_view_distance() {
        let g = grid();
        let mut seen = Vec::new();
        g.query(DVec3::new(15_000.0, 0.0, 0.0), 350.0, None, |i| {
            seen.push(i)
        });
        seen.sort_unstable();
        // Cells are 200 m: every object of the cells overlapping the 350 m disc, no others.
        assert!(seen.contains(&10)); // z = 0
        assert!(seen.contains(&13)); // z = 300
        assert!(seen.contains(&7)); // z = -300
        assert!(!seen.contains(&0)); // z = -1000
        assert!(!seen.contains(&20)); // z = 1000
        assert!(
            seen.windows(2).all(|w| w[0] < w[1]),
            "visited twice: {seen:?}"
        );
    }

    #[test]
    fn grid_queries_skip_cells_outside_the_frustum() {
        let g = grid();
        let camera = Camera {
            position: DVec3::new(15_000.0, 10.0, 0.0),
            ..Camera::default()
        };
        let f = Frustum::from_view_projection(camera.view_projection(16.0 / 9.0));
        let mut seen = Vec::new();
        g.query(camera.position, 2_000.0, Some(&f), |i| seen.push(i));
        assert!(seen.contains(&20)); // 1000 m ahead
        assert!(!seen.contains(&0)); // 1000 m behind
    }

    #[test]
    fn an_empty_grid_visits_nothing() {
        let g = ObjectGrid::build(100.0, std::iter::empty());
        let mut n = 0;
        g.query(DVec3::ZERO, 1e6, None, |_| n += 1);
        assert_eq!(n, 0);
    }
}
