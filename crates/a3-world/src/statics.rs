//! Static objects: the terrain's placed objects, kept compactly per land cell.

use a3_wrp::Terrain;
use glam::DVec3;

use crate::{Error, NetworkId};

const INDEX_BITS: u32 = 11;
const CELL_BITS: u32 = 10;
const STATIC_FLAG: u32 = 1 << 31;

/// Identifies a Static object within the loaded terrain: its land cell and its index among the
/// cell's objects, packed like the original's landscape ObjectId (bit 31 set, cell z in bits
/// 21-30, cell x in bits 11-20, index in bits 0-10; `docs/re/wrp.md`). The Static object's
/// Network object ID is `{1, key}`.
///
/// _Uncertain_: the original assigns objects to cells and orders them by the WRP's grouping;
/// we use the position's cell and file order. Matching the original exactly is issue #117.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StaticKey(u32);

impl StaticKey {
    /// `None` if a coordinate or the index does not fit the packing.
    pub fn new(cell_x: u32, cell_z: u32, index: u32) -> Option<Self> {
        let max_cell = (1 << CELL_BITS) - 1;
        if cell_x > max_cell || cell_z > max_cell || index >= 1 << INDEX_BITS {
            return None;
        }
        Some(Self(
            STATIC_FLAG | cell_z << (INDEX_BITS + CELL_BITS) | cell_x << INDEX_BITS | index,
        ))
    }

    /// The packed value.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// From a packed value; `None` if bit 31 is clear.
    pub fn from_raw(raw: u32) -> Option<Self> {
        (raw & STATIC_FLAG != 0).then_some(Self(raw))
    }

    pub fn cell(self) -> (u32, u32) {
        let mask = (1 << CELL_BITS) - 1;
        (
            (self.0 >> INDEX_BITS) & mask,
            (self.0 >> (INDEX_BITS + CELL_BITS)) & mask,
        )
    }

    pub fn index(self) -> u32 {
        self.0 & ((1 << INDEX_BITS) - 1)
    }

    /// `{1, key}`.
    pub fn network_id(self) -> NetworkId {
        NetworkId::new(NetworkId::STATIC_CREATOR, self.0)
    }

    /// The key of a creator-1 Network object ID.
    pub fn from_network_id(id: NetworkId) -> Option<Self> {
        if id.is_static() {
            Self::from_raw(id.id)
        } else {
            None
        }
    }
}

/// One placed object of the terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticObject {
    pub key: StaticKey,
    /// The Object ID from the WRP (`getObjectID`).
    pub object_id: u32,
    /// Index into the terrain's model list.
    pub model_index: u32,
    /// World-space position.
    pub position: DVec3,
}

/// The Static objects, bucketed by land cell (compressed rows: `cell_start[c]..cell_start[c+1]`).
#[derive(Debug, Default)]
pub(crate) struct StaticObjects {
    cell_size: f64,
    width: u32,
    height: u32,
    cell_start: Vec<u32>,
    objects: Vec<StaticObject>,
    removed: Vec<bool>,
}

impl StaticObjects {
    pub(crate) fn from_terrain(terrain: &Terrain) -> Result<Self, Error> {
        let (width, height) = (terrain.land_grid.width, terrain.land_grid.height);
        let cell_size = f64::from(terrain.land_cell_size);
        let cells = (width as usize) * (height as usize);
        let cell_of = |p: DVec3| -> (u32, u32) {
            let clamp =
                |v: f64, n: u32| (v / cell_size).floor().clamp(0.0, f64::from(n - 1)) as u32;
            (clamp(p.x, width), clamp(p.z, height))
        };

        let mut counts = vec![0u32; cells + 1];
        let positions: Vec<DVec3> = terrain
            .objects
            .iter()
            .map(|o| o.transform.position().as_dvec3())
            .collect();
        for &p in &positions {
            let (x, z) = cell_of(p);
            counts[(z * width + x) as usize + 1] += 1;
        }
        for c in 0..cells {
            counts[c + 1] += counts[c];
        }
        let cell_start = counts.clone();
        let mut fill = counts;
        let mut objects = vec![
            StaticObject {
                key: StaticKey(STATIC_FLAG),
                object_id: 0,
                model_index: 0,
                position: DVec3::ZERO,
            };
            positions.len()
        ];
        for (o, &p) in terrain.objects.iter().zip(&positions) {
            let (x, z) = cell_of(p);
            let c = (z * width + x) as usize;
            let slot = fill[c];
            fill[c] += 1;
            let index = slot - cell_start[c];
            let key = StaticKey::new(x, z, index).ok_or(Error::StaticKeyOverflow {
                cell_x: x,
                cell_z: z,
            })?;
            objects[slot as usize] = StaticObject {
                key,
                object_id: o.id,
                model_index: o.model_index,
                position: p,
            };
        }
        let removed = vec![false; objects.len()];
        Ok(Self {
            cell_size,
            width,
            height,
            cell_start,
            objects,
            removed,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.objects.len()
    }

    fn slot(&self, key: StaticKey) -> Option<usize> {
        let (x, z) = key.cell();
        if x >= self.width || z >= self.height {
            return None;
        }
        let c = (z * self.width + x) as usize;
        let slot = self.cell_start[c] + key.index();
        (slot < self.cell_start[c + 1]).then_some(slot as usize)
    }

    pub(crate) fn get(&self, key: StaticKey) -> Option<&StaticObject> {
        self.slot(key)
            .filter(|&s| !self.removed[s])
            .map(|s| &self.objects[s])
    }

    pub(crate) fn contains(&self, key: StaticKey) -> bool {
        self.get(key).is_some()
    }

    pub(crate) fn remove(&mut self, key: StaticKey) {
        if let Some(s) = self.slot(key) {
            self.removed[s] = true;
        }
    }

    fn cell(&self, x: u32, z: u32) -> &[StaticObject] {
        let c = (z * self.width + x) as usize;
        &self.objects[self.cell_start[c] as usize..self.cell_start[c + 1] as usize]
    }

    /// The object with `object_id`, searching the cell of `near` first and then rings of cells
    /// around it (as `nearestObject [position, id]` does).
    pub(crate) fn find(&self, near: DVec3, object_id: u32) -> Option<StaticKey> {
        if self.objects.is_empty() {
            return None;
        }
        let cx = ((near.x / self.cell_size).floor() as i64).clamp(0, i64::from(self.width) - 1);
        let cz = ((near.z / self.cell_size).floor() as i64).clamp(0, i64::from(self.height) - 1);
        let max_ring = i64::from(self.width.max(self.height));
        for ring in 0..=max_ring {
            for z in cz - ring..=cz + ring {
                for x in cx - ring..=cx + ring {
                    let on_ring = (z - cz).abs() == ring || (x - cx).abs() == ring;
                    let inside = (0..i64::from(self.width)).contains(&x)
                        && (0..i64::from(self.height)).contains(&z);
                    if !on_ring || !inside {
                        continue;
                    }
                    if let Some(o) = self
                        .cell(x as u32, z as u32)
                        .iter()
                        .find(|o| o.object_id == object_id)
                    {
                        if self.contains(o.key) {
                            return Some(o.key);
                        }
                    }
                }
            }
        }
        None
    }

    /// Static objects (not removed) within `radius` of `center` (3D distance).
    pub(crate) fn near(&self, center: DVec3, radius: f64) -> Vec<&StaticObject> {
        if self.objects.is_empty() || radius < 0.0 {
            return Vec::new();
        }
        let cell =
            |v: f64, n: u32| ((v / self.cell_size).floor() as i64).clamp(0, i64::from(n) - 1);
        let (x0, x1) = (
            cell(center.x - radius, self.width),
            cell(center.x + radius, self.width),
        );
        let (z0, z1) = (
            cell(center.z - radius, self.height),
            cell(center.z + radius, self.height),
        );
        let mut out = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.extend(
                    self.cell(x as u32, z as u32)
                        .iter()
                        .filter(|o| o.position.distance(center) <= radius && self.contains(o.key)),
                );
            }
        }
        out
    }
}
