//! The engine's packed quad tree of grid cells, read into and written from a dense [`Grid`].
//!
//! Layout (see `docs/re/wrp.md`): a flag byte, then the root. A node is a `u16` bit mask
//! followed by 16 children in row-major 4x4 order (`index = (cz << 2) | cx`); bit `i` set means
//! child `i` is a node, clear means a leaf. A leaf is 4 bytes holding a small tile of elements
//! (2x2 bytes, 2x1 `u16`s or one `u32`); a leaf above the bottom level repeats its tile over its
//! whole area. A root flag of 0 means the root is a single leaf.

use crate::cursor::{Reader, Writer};
use crate::grid::{Grid, GridSize};
use crate::{Error, Result};

/// An element type stored in a quad tree.
pub(crate) trait Element: Copy + PartialEq + Default {
    /// Size in bytes: 1, 2 or 4.
    const SIZE: usize;
    fn read(bytes: &[u8]) -> Self;
    fn write(self, out: &mut [u8]);
}

impl Element for u8 {
    const SIZE: usize = 1;
    fn read(bytes: &[u8]) -> Self {
        bytes[0]
    }
    fn write(self, out: &mut [u8]) {
        out[0] = self;
    }
}

impl Element for u16 {
    const SIZE: usize = 2;
    fn read(bytes: &[u8]) -> Self {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }
    fn write(self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.to_le_bytes());
    }
}

impl Element for u32 {
    const SIZE: usize = 4;
    fn read(bytes: &[u8]) -> Self {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
    fn write(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.to_le_bytes());
    }
}

/// Bits of x/z covered by one node level.
const LOG_NODE: u32 = 2;

/// The virtual (power-of-two) extent of a quad tree over a grid.
#[derive(Debug, Clone, Copy)]
struct Shape {
    /// log2 of the leaf tile width and height.
    leaf_log: (u32, u32),
    /// Number of node levels between the root and the leaves.
    levels: u32,
}

impl Shape {
    fn new<T: Element>(size: GridSize) -> Self {
        let leaf_log = match T::SIZE {
            1 => (1, 1),
            2 => (1, 0),
            _ => (0, 0),
        };
        let bits = |n: u32| u32::BITS - n.saturating_sub(1).leading_zeros();
        let levels_for = |total: u32, leaf: u32| total.saturating_sub(leaf).div_ceil(LOG_NODE);
        let levels = levels_for(bits(size.width), leaf_log.0)
            .max(levels_for(bits(size.height), leaf_log.1));
        Self { leaf_log, levels }
    }

    /// log2 of the area covered by a node at `depth` (0 = root).
    fn log_extent(self, depth: u32) -> (u32, u32) {
        let up = (self.levels - depth) * LOG_NODE;
        (self.leaf_log.0 + up, self.leaf_log.1 + up)
    }
}

/// Reads a quad tree over a grid of `size` cells.
pub(crate) fn read<T: Element>(r: &mut Reader<'_>, size: GridSize) -> Result<Grid<T>> {
    let shape = Shape::new::<T>(size);
    let mut grid = Grid::new(size);
    let is_node = r.u8("quad tree root flag")? != 0;
    read_child(r, &mut grid, shape, is_node, 0, 0, 0)?;
    Ok(grid)
}

fn read_child<T: Element>(
    r: &mut Reader<'_>,
    grid: &mut Grid<T>,
    shape: Shape,
    is_node: bool,
    depth: u32,
    x0: u32,
    z0: u32,
) -> Result<()> {
    if !is_node {
        let tile = r.array::<4>("quad tree leaf")?;
        fill_leaf(grid, shape, depth, x0, z0, &tile);
        return Ok(());
    }
    if depth >= shape.levels {
        return Err(Error::Invalid {
            offset: r.pos(),
            what: "quad tree",
            detail: "node below the leaf level".into(),
        });
    }
    let mask = r.u16("quad tree node")?;
    let (cx_log, cz_log) = shape.log_extent(depth + 1);
    for i in 0..16u32 {
        let x = x0 + ((i & 3) << cx_log);
        let z = z0 + ((i >> 2) << cz_log);
        read_child(r, grid, shape, mask & (1 << i) != 0, depth + 1, x, z)?;
    }
    Ok(())
}

fn fill_leaf<T: Element>(
    grid: &mut Grid<T>,
    shape: Shape,
    depth: u32,
    x0: u32,
    z0: u32,
    tile: &[u8; 4],
) {
    let (lx, lz) = shape.leaf_log;
    let (ex, ez) = shape.log_extent(depth);
    let size = grid.size();
    if x0 >= size.width || z0 >= size.height {
        return;
    }
    let x1 = (u64::from(x0) + (1u64 << ex)).min(u64::from(size.width)) as u32;
    let z1 = (u64::from(z0) + (1u64 << ez)).min(u64::from(size.height)) as u32;
    let mut values = [T::default(); 4];
    for (i, v) in values.iter_mut().enumerate().take(4 / T::SIZE) {
        *v = T::read(&tile[i * T::SIZE..]);
    }
    let width = size.width as usize;
    let cells = grid.as_mut_slice();
    for z in z0..z1 {
        let row = &mut cells[z as usize * width..];
        let tz = (z & ((1 << lz) - 1)) << lx;
        for x in x0..x1 {
            row[x as usize] = values[(tz | (x & ((1 << lx) - 1))) as usize];
        }
    }
}

/// Writes `grid` as a quad tree, collapsing every area whose cells repeat one leaf tile.
pub(crate) fn write<T: Element>(w: &mut Writer, grid: &Grid<T>) {
    let shape = Shape::new::<T>(grid.size());
    match uniform_tile(grid, shape, 0, 0, 0) {
        Some(tile) => {
            w.u8(0);
            w.bytes(&tile);
        }
        None => {
            w.u8(1);
            write_node(w, grid, shape, 0, 0, 0);
        }
    }
}

fn write_node<T: Element>(w: &mut Writer, grid: &Grid<T>, shape: Shape, depth: u32, x0: u32, z0: u32) {
    let (cx_log, cz_log) = shape.log_extent(depth + 1);
    let children: Vec<(u32, u32, Option<[u8; 4]>)> = (0..16u32)
        .map(|i| {
            let x = x0 + ((i & 3) << cx_log);
            let z = z0 + ((i >> 2) << cz_log);
            (x, z, uniform_tile(grid, shape, depth + 1, x, z))
        })
        .collect();
    let mask = children
        .iter()
        .enumerate()
        .filter(|(_, c)| c.2.is_none())
        .fold(0u16, |m, (i, _)| m | (1 << i));
    w.u16(mask);
    for (x, z, tile) in children {
        match tile {
            Some(tile) => w.bytes(&tile),
            None => write_node(w, grid, shape, depth + 1, x, z),
        }
    }
}

/// The leaf tile repeated over the area of the node at `depth`/`(x0, z0)`, if there is one.
/// Cells outside the grid match anything.
fn uniform_tile<T: Element>(
    grid: &Grid<T>,
    shape: Shape,
    depth: u32,
    x0: u32,
    z0: u32,
) -> Option<[u8; 4]> {
    let (lx, lz) = shape.leaf_log;
    let (ex, ez) = shape.log_extent(depth);
    let size = grid.size();
    let x1 = (u64::from(x0) + (1u64 << ex)).min(u64::from(size.width)) as u32;
    let z1 = (u64::from(z0) + (1u64 << ez)).min(u64::from(size.height)) as u32;
    let mut values: [Option<T>; 4] = [None; 4];
    for z in z0..z1 {
        for x in x0..x1 {
            let i = (((z & ((1 << lz) - 1)) << lx) | (x & ((1 << lx) - 1))) as usize;
            let v = *grid.get(x, z)?;
            match values[i] {
                None => values[i] = Some(v),
                Some(seen) if seen != v => return None,
                Some(_) => {}
            }
        }
    }
    let mut tile = [0u8; 4];
    for (i, v) in values.iter().enumerate().take(4 / T::SIZE) {
        v.unwrap_or_default().write(&mut tile[i * T::SIZE..]);
    }
    Some(tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_leaves_hold_a_2x2_tile_in_row_major_order() {
        // A 4x4 grid of bytes: one root node over an 8x8 virtual area, 16 leaves of 2x2 cells.
        let mut bytes = vec![1u8, 0, 0];
        for leaf in 0..16u8 {
            let tile = match leaf {
                0 => [1, 2, 3, 4],
                1 => [5, 6, 7, 8],
                4 => [9, 10, 11, 12],
                5 => [13, 14, 15, 16],
                _ => [0; 4],
            };
            bytes.extend(tile);
        }
        let grid = read::<u8>(&mut Reader::new(&bytes), GridSize::new(4, 4)).unwrap();
        let rows: Vec<&[u8]> = (0..4).map(|z| grid.row(z)).collect();
        assert_eq!(
            rows,
            [[1, 2, 5, 6], [3, 4, 7, 8], [9, 10, 13, 14], [11, 12, 15, 16]]
        );
    }

    #[test]
    fn a_leaf_root_repeats_its_tile_over_the_grid() {
        let bytes = [0u8, 0x34, 0x12, 0x78, 0x56];
        let grid = read::<u16>(&mut Reader::new(&bytes), GridSize::new(4, 4)).unwrap();
        for z in 0..4 {
            assert_eq!(grid.row(z), [0x1234, 0x5678, 0x1234, 0x5678]);
        }
    }

    #[test]
    fn writer_collapses_uniform_grids_to_one_leaf() {
        let mut w = Writer::default();
        write(&mut w, &Grid::filled(GridSize::new(256, 256), 7u32));
        assert_eq!(w.buf, [0, 7, 0, 0, 0]);
    }
}
