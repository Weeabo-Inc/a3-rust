//! The heightmap as the terrain renderer uses it: sampling (mirrored by the vertex shader),
//! per-sample normals and min/max bounds of every LOD node.

use a3_wrp::Terrain;
use glam::Vec3;

/// A square grid of height samples. Sample `(i, j)` sits at world `(i * cell, j * cell)`.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightField {
    /// Samples per axis.
    pub size: u32,
    /// Sample spacing in metres.
    pub cell: f32,
    /// Row-major heights, `j * size + i`, south row first.
    pub heights: Vec<f32>,
}

impl HeightField {
    pub fn from_terrain(terrain: &Terrain) -> HeightField {
        HeightField {
            size: terrain.heightmap.width(),
            cell: terrain.terrain_cell_size(),
            heights: terrain.heightmap.as_slice().to_vec(),
        }
    }

    /// Height sample `(i, j)`, clamped to the grid.
    pub fn at(&self, i: i64, j: i64) -> f32 {
        let max = i64::from(self.size) - 1;
        let (i, j) = (i.clamp(0, max), j.clamp(0, max));
        self.heights[(j * i64::from(self.size) + i) as usize]
    }

    /// Surface height at world `(x, z)` in the engine's triangulation: each cell splits along
    /// the diagonal from `(i + 1, j)` to `(i, j + 1)`. The terrain vertex shader (`height_at`
    /// in `terrain.wgsl`) computes exactly this.
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let (gx, gz) = (x / self.cell, z / self.cell);
        let (fi, fj) = (gx.floor(), gz.floor());
        let (fx, fz) = (gx - fi, gz - fj);
        let (i, j) = (fi as i64, fj as i64);
        let h00 = self.at(i, j);
        let h10 = self.at(i + 1, j);
        let h01 = self.at(i, j + 1);
        let h11 = self.at(i + 1, j + 1);
        if fx + fz <= 1.0 {
            h00 + (h10 - h00) * fx + (h01 - h00) * fz
        } else {
            (h01 + h10 - h11) + (h11 - h01) * fx + (h11 - h10) * fz
        }
    }

    /// Unit surface normal at sample `(i, j)` from central differences.
    pub fn normal(&self, i: i64, j: i64) -> Vec3 {
        let dx = self.at(i + 1, j) - self.at(i - 1, j);
        let dz = self.at(i, j + 1) - self.at(i, j - 1);
        Vec3::new(-dx, 2.0 * self.cell, -dz).normalize()
    }

    /// Normals of every sample packed as RGBA8 snorm (x, y, z, 0), row-major like `heights`.
    pub fn normals_rgba8_snorm(&self) -> Vec<u8> {
        let n = i64::from(self.size);
        let mut out = Vec::with_capacity((n * n * 4) as usize);
        let pack = |v: f32| ((v.clamp(-1.0, 1.0) * 127.0).round() as i8) as u8;
        for j in 0..n {
            for i in 0..n {
                let v = self.normal(i, j);
                out.extend_from_slice(&[pack(v.x), pack(v.y), pack(v.z), 0]);
            }
        }
        out
    }
}

/// Minimum and maximum height of square blocks of the grid, for every quadtree level.
#[derive(Debug, Clone, PartialEq)]
pub struct MinMaxPyramid {
    /// Cells per block edge at level 0.
    pub leaf: u32,
    /// `levels[l]`: blocks of `leaf << l` cells, row-major, `(min, max)`. A block includes the
    /// samples on its far edges (the vertices it shares with its neighbours).
    pub levels: Vec<Vec<(f32, f32)>>,
}

impl MinMaxPyramid {
    /// Build for blocks of `leaf` cells up to one block covering the whole grid.
    /// `heights.size` must be `leaf` times a power of two.
    pub fn build(heights: &HeightField, leaf: u32) -> MinMaxPyramid {
        let blocks = (heights.size / leaf).max(1);
        let mut level0 = Vec::with_capacity((blocks * blocks) as usize);
        for bz in 0..blocks {
            for bx in 0..blocks {
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for j in bz * leaf..=(bz + 1) * leaf {
                    for i in bx * leaf..=(bx + 1) * leaf {
                        let h = heights.at(i64::from(i), i64::from(j));
                        lo = lo.min(h);
                        hi = hi.max(h);
                    }
                }
                level0.push((lo, hi));
            }
        }
        let mut levels = vec![level0];
        let mut n = blocks;
        while n > 1 {
            let half = n / 2;
            let prev = levels.last().expect("level 0 exists");
            let mut next = Vec::with_capacity((half * half) as usize);
            for z in 0..half {
                for x in 0..half {
                    let mut b = (f32::INFINITY, f32::NEG_INFINITY);
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let c = prev[((2 * z + dz) * n + 2 * x + dx) as usize];
                        b = (b.0.min(c.0), b.1.max(c.1));
                    }
                    next.push(b);
                }
            }
            levels.push(next);
            n = half;
        }
        MinMaxPyramid { leaf, levels }
    }

    /// `(min, max)` of block `(x, z)` (in blocks) at `level`.
    pub fn get(&self, level: u32, x: u32, z: u32) -> (f32, f32) {
        let blocks = 1usize << (self.levels.len() - 1 - level as usize);
        self.levels[level as usize][z as usize * blocks + x as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_wrp::TerrainBuilder;
    use proptest::prelude::*;

    fn hills(i: u32, j: u32) -> f32 {
        let (x, z) = (i as f32, j as f32);
        20.0 * (x * 0.37).sin() + 13.0 * (z * 0.21).cos() + 0.5 * x - 0.25 * z
    }

    fn terrain() -> Terrain {
        // 8 land cells of 40 m: 32 height cells of 10 m.
        TerrainBuilder::new(8, 32, 40.0).heights(hills).build()
    }

    proptest! {
        #[test]
        fn sampling_matches_the_engine_surface_height(x in -20.0f32..340.0, z in -20.0f32..340.0) {
            let t = terrain();
            let field = HeightField::from_terrain(&t);
            let (ours, engine) = (field.sample(x, z), t.surface_height(x, z));
            prop_assert!((ours - engine).abs() < 1e-3, "{ours} vs {engine} at {x} {z}");
        }
    }

    #[test]
    fn samples_sit_on_grid_points() {
        let t = terrain();
        let field = HeightField::from_terrain(&t);
        assert_eq!(field.cell, 10.0);
        assert_eq!(field.sample(50.0, 70.0), hills(5, 7));
    }

    #[test]
    fn normals_tilt_away_from_the_uphill_side() {
        // Height rises 1 m per metre to the east.
        let field = HeightField {
            size: 4,
            cell: 2.0,
            heights: (0..16).map(|k| (k % 4) as f32 * 2.0).collect(),
        };
        let n = field.normal(1, 1);
        let expected = Vec3::new(-1.0, 1.0, 0.0).normalize();
        assert!((n - expected).length() < 1e-5, "{n}");
        let packed = field.normals_rgba8_snorm();
        assert_eq!(packed.len(), 64);
        assert_eq!(packed[(4 + 1) * 4] as i8, -90);
    }

    #[test]
    fn min_max_blocks_bound_every_sample_and_merge_upwards() {
        let field = HeightField::from_terrain(&terrain());
        let pyramid = MinMaxPyramid::build(&field, 8);
        assert_eq!(pyramid.levels.len(), 3, "32 cells: 4x4, 2x2, 1x1 blocks");
        let (lo, hi) = pyramid.get(0, 1, 2);
        for j in 16..=24 {
            for i in 8..=16 {
                let h = hills(i, j);
                assert!(lo <= h && h <= hi);
            }
        }
        let all = field
            .heights
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |b, &h| {
                (b.0.min(h), b.1.max(h))
            });
        assert_eq!(pyramid.get(2, 0, 0), all);
    }
}
