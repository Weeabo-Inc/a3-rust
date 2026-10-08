//! A [`StaticSource`] straight from a WRP, for tools and tests without a World.

use std::sync::Arc;

use a3_wrp::Terrain;
use glam::DAffine3;

use crate::{LandGrid, StaticPlacement, StaticSource};

/// The placed objects of a terrain, bucketed by the land cell under their position, in file
/// order. Keys pack the cell and the index in it the way the World's `StaticKey` does (bit 31,
/// cell z in bits 21-30, cell x in bits 11-20, index in bits 0-10).
#[derive(Debug, Clone)]
pub struct TerrainStatics {
    terrain: Arc<Terrain>,
    grid: LandGrid,
    cell_start: Vec<u32>,
    /// Object indices into `terrain.objects`, grouped by cell.
    order: Vec<u32>,
}

impl TerrainStatics {
    pub fn new(terrain: Arc<Terrain>) -> Self {
        let grid = LandGrid {
            cell_size: f64::from(terrain.land_cell_size),
            width: terrain.land_grid.width,
            height: terrain.land_grid.height,
        };
        let cell_of = |i: usize| -> usize {
            let p = terrain.objects[i].transform.position().as_dvec3();
            let clamp = |v: f64, n: u32| (v / grid.cell_size).floor().clamp(0.0, f64::from(n - 1));
            (clamp(p.z, grid.height) as usize) * grid.width as usize
                + clamp(p.x, grid.width) as usize
        };
        let cells = grid.width as usize * grid.height as usize;
        let mut cell_start = vec![0u32; cells + 1];
        let of: Vec<usize> = (0..terrain.objects.len()).map(cell_of).collect();
        for &c in &of {
            cell_start[c + 1] += 1;
        }
        for c in 0..cells {
            cell_start[c + 1] += cell_start[c];
        }
        let mut fill = cell_start.clone();
        let mut order = vec![0u32; of.len()];
        for (i, &c) in of.iter().enumerate() {
            order[fill[c] as usize] = i as u32;
            fill[c] += 1;
        }
        Self {
            terrain,
            grid,
            cell_start,
            order,
        }
    }

    pub fn terrain(&self) -> &Arc<Terrain> {
        &self.terrain
    }

    /// The key of the `index`-th object of land cell `(x, z)`.
    pub fn key(x: u32, z: u32, index: u32) -> u32 {
        1 << 31 | z << 21 | x << 11 | index
    }

    /// The WRP object behind a key.
    pub fn object(&self, key: u32) -> Option<&a3_wrp::ObjectInstance> {
        let (x, z, index) = ((key >> 11) & 1023, (key >> 21) & 1023, key & 2047);
        if x >= self.grid.width || z >= self.grid.height {
            return None;
        }
        let c = (z * self.grid.width + x) as usize;
        let slot = self.cell_start[c] + index;
        (slot < self.cell_start[c + 1])
            .then(|| &self.terrain.objects[self.order[slot as usize] as usize])
    }
}

impl StaticSource for TerrainStatics {
    fn land_grid(&self) -> LandGrid {
        self.grid
    }

    fn for_each_in_cell(&self, x: u32, z: u32, f: &mut dyn FnMut(StaticPlacement<'_>)) {
        if x >= self.grid.width || z >= self.grid.height {
            return;
        }
        let c = (z * self.grid.width + x) as usize;
        let range = self.cell_start[c] as usize..self.cell_start[c + 1] as usize;
        for (index, &i) in self.order[range].iter().enumerate() {
            let o = &self.terrain.objects[i as usize];
            let Some(model) = self.terrain.model_of(o) else {
                continue;
            };
            f(StaticPlacement {
                key: Self::key(x, z, index as u32),
                model: model.as_str(),
                transform: wrp_transform(&o.transform),
            });
        }
    }
}

/// A WRP transform (three orientation columns, scale included, then the position) in `f64`.
pub fn wrp_transform(t: &a3_wrp::Transform) -> DAffine3 {
    let m = t.0.map(f64::from);
    DAffine3::from_cols_array(&m)
}
