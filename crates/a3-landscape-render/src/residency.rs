//! Which full-resolution satellite tiles live in the GPU texture array.
//!
//! Each frame the renderer asks for the tiles nearest the camera ([`nearest_tiles`]); the
//! [`TileResidency`] answers which of them must be loaded and, when a loaded tile arrives,
//! which array slot it goes to (evicting a tile that is no longer wanted).

use std::collections::{HashMap, HashSet};

use crate::satellite::{SatelliteGrid, Tile};

/// Where an arrived tile goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub slot: u32,
    /// The tile that held the slot before, now no longer resident.
    pub evicted: Option<u16>,
}

/// Slot bookkeeping for a texture array of `capacity` layers.
#[derive(Debug, Clone)]
pub struct TileResidency {
    slots: Vec<Option<u16>>,
    slot_of: HashMap<u16, u32>,
    pending: HashSet<u16>,
    /// Tiles that failed to load; not requested again.
    failed: HashSet<u16>,
    /// Wanted tiles, nearest first, at most `capacity`.
    desired: Vec<u16>,
}

impl TileResidency {
    pub fn new(capacity: u32) -> TileResidency {
        TileResidency {
            slots: vec![None; capacity as usize],
            slot_of: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            desired: Vec::new(),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }

    /// Set the wanted tiles, nearest first (truncated to the capacity). Returns the ones to
    /// load now: wanted, not resident and not already requested, nearest first.
    pub fn want(&mut self, mut desired: Vec<u16>) -> Vec<u16> {
        desired.truncate(self.slots.len());
        let requests: Vec<u16> = desired
            .iter()
            .copied()
            .filter(|t| {
                !self.slot_of.contains_key(t)
                    && !self.pending.contains(t)
                    && !self.failed.contains(t)
            })
            .collect();
        self.pending.extend(&requests);
        self.desired = desired;
        requests
    }

    /// A requested tile finished loading. Returns its slot, or `None` when it is no longer
    /// wanted (it is dropped).
    pub fn arrived(&mut self, tile: u16) -> Option<Placement> {
        self.pending.remove(&tile);
        if self.slot_of.contains_key(&tile) || !self.desired.contains(&tile) {
            return None;
        }
        let slot = match self.slots.iter().position(Option::is_none) {
            Some(free) => free as u32,
            None => {
                // The resident tile that is not wanted, else the least wanted one.
                let rank = |t: u16| self.desired.iter().position(|&d| d == t);
                let (slot, _) = self
                    .slots
                    .iter()
                    .enumerate()
                    .filter_map(|(s, t)| t.map(|t| (s, rank(t))))
                    .max_by_key(|&(_, r)| r.unwrap_or(usize::MAX))?;
                slot as u32
            }
        };
        let evicted = self.slots[slot as usize].replace(tile);
        if let Some(old) = evicted {
            self.slot_of.remove(&old);
        }
        self.slot_of.insert(tile, slot);
        Some(Placement { slot, evicted })
    }

    /// A requested tile could not be loaded: it is no longer pending and is not requested
    /// again.
    pub fn failed(&mut self, tile: u16) {
        self.pending.remove(&tile);
        self.failed.insert(tile);
    }

    /// The slot of a resident tile.
    pub fn slot(&self, tile: u16) -> Option<u32> {
        self.slot_of.get(&tile).copied()
    }

    pub fn resident(&self) -> usize {
        self.slot_of.len()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// Indices of the tiles with a satellite texture whose core lies within `radius` metres of
/// `(x, z)`, nearest first.
pub fn nearest_tiles(
    grid: &SatelliteGrid,
    tiles: &[Tile],
    world: f32,
    x: f32,
    z: f32,
    radius: f32,
) -> Vec<u16> {
    let mut found: Vec<(f32, u16)> = tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.satellite.is_some())
        .filter_map(|(i, t)| {
            let x0 = t.coord.col as f32 * grid.step;
            let z1 = world - t.coord.row as f32 * grid.step;
            let dx = (x0 - x).max(x - (x0 + grid.step)).max(0.0);
            let dz = ((z1 - grid.step) - z).max(z - z1).max(0.0);
            let d = (dx * dx + dz * dz).sqrt();
            (d <= radius).then_some((d, i as u16))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satellite::{TileCoord, TileUv};
    use a3_core::VfsPath;

    #[test]
    fn wanted_tiles_are_requested_once() {
        let mut r = TileResidency::new(4);
        assert_eq!(r.want(vec![5, 6, 7]), vec![5, 6, 7]);
        assert_eq!(
            r.want(vec![6, 5, 8]),
            vec![8],
            "5 and 6 are already on their way"
        );
        assert_eq!(r.pending(), 4);
    }

    #[test]
    fn arrivals_fill_free_slots_then_evict_unwanted_tiles() {
        let mut r = TileResidency::new(2);
        r.want(vec![1, 2]);
        assert_eq!(
            r.arrived(1),
            Some(Placement {
                slot: 0,
                evicted: None
            })
        );
        assert_eq!(r.arrived(2).unwrap().slot, 1);
        // The camera moved: tile 3 is now nearest, 1 is no longer wanted.
        assert_eq!(r.want(vec![3, 2]), vec![3]);
        assert_eq!(
            r.arrived(3),
            Some(Placement {
                slot: 0,
                evicted: Some(1)
            })
        );
        assert_eq!((r.slot(3), r.slot(2), r.slot(1)), (Some(0), Some(1), None));
        assert_eq!(r.resident(), 2);
    }

    #[test]
    fn tiles_no_longer_wanted_on_arrival_are_dropped() {
        let mut r = TileResidency::new(2);
        r.want(vec![1]);
        r.want(vec![2]);
        assert_eq!(r.arrived(1), None);
        assert_eq!(r.pending(), 1, "only tile 2 is still on its way");
        assert_eq!(r.slot(1), None);
    }

    #[test]
    fn failed_tiles_are_not_requested_again() {
        let mut r = TileResidency::new(2);
        assert_eq!(r.want(vec![1, 2]), vec![1, 2]);
        r.failed(1);
        assert_eq!(r.pending(), 1);
        assert_eq!(r.want(vec![1, 2]), Vec::<u16>::new());
    }

    #[test]
    fn requests_are_capped_at_capacity() {
        let mut r = TileResidency::new(2);
        assert_eq!(r.want(vec![9, 8, 7, 6]), vec![9, 8]);
    }

    #[test]
    fn nearest_tiles_are_sorted_by_distance_and_skip_tiles_without_texture() {
        // A 4x4 grid of 100 m tiles over 400 m; rows count from the north.
        let grid = SatelliteGrid {
            tiles: 4,
            step: 100.0,
            size: 110.0,
        };
        let uv = TileUv {
            u: [0.0; 3],
            v: [0.0; 3],
        };
        let tiles: Vec<Tile> = (0..16)
            .map(|i| Tile {
                coord: TileCoord {
                    col: i % 4,
                    row: i / 4,
                },
                satellite: (i != 13).then(|| VfsPath::new("s.paa")),
                shared_satellite: None,
                mask: None,
                uv,
            })
            .collect();
        // Camera in the south-west tile (col 0, row 3 = index 12).
        let near = nearest_tiles(&grid, &tiles, 400.0, 50.0, 50.0, 60.0);
        assert_eq!(near, vec![12, 8], "13 (east) has no texture; 8 is north");
        let all = nearest_tiles(&grid, &tiles, 400.0, 50.0, 50.0, 1e4);
        assert_eq!(all.len(), 15);
        assert_eq!(all[0], 12);
    }
}
