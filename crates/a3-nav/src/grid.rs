//! The navigation grid: one cost per land cell (ADR 0009).
//!
//! The grid is baked once from the terrain — the land grid's [`Geography`] flags, the heightmap
//! slope across each cell and the road network — and is read-only afterwards except for
//! patches ([`NavGrid::set_cost`], [`NavGrid::patch`]).

use a3_landscape::{CurveSegment, RoadGraph};
use a3_wrp::{Geography, Grid, GridSize, Terrain};
use glam::Vec3;

pub use cost::*;

/// What a cell costs to cross, in percent of open ground. `0` is impassable.
///
/// The A* multiplies the distance it moves by `(cost_of(a) + cost_of(b)) / 200`, so these are
/// multipliers, not distances: a road is cheaper than open ground, a forest costs more.
pub mod cost {
    /// Impassable: deep water, too steep, or blocked by an obstacle.
    pub const IMPASSABLE: u8 = 0;
    /// A road cell: the AI prefers it, which is what makes long paths follow the network.
    pub const ROAD: u8 = 70;
    /// Open ground.
    pub const OPEN: u8 = 100;
    /// Forest.
    pub const FOREST: u8 = 130;
    /// Built-up ground: a cell mostly covered by objects.
    pub const BUILT_UP: u8 = 140;
    /// Water a man wades through.
    pub const SHALLOW_WATER: u8 = 200;
}

/// Water depth class at which a cell is too deep to wade (of the 0-3 classes the geography
/// flags carry).
const DEEP_WATER_CLASS: u8 = 2;

/// How many samples a road curve is baked with per cell of its length.
const ROAD_SAMPLES_PER_CELL: f32 = 4.0;

/// The most samples one curve segment is baked with.
const MAX_ROAD_SAMPLES: usize = 64;

/// The cost grid of one terrain, at the terrain's land-cell resolution.
#[derive(Clone, PartialEq)]
pub struct NavGrid {
    cell_size: f32,
    size: GridSize,
    costs: Grid<u8>,
}

impl NavGrid {
    /// Bakes the grid from `terrain` alone: geography flags and heightmap slope. Road cells
    /// come from the WRP's own road flag; use [`NavGrid::bake_with_roads`] to bake the
    /// shapefile network in as well.
    pub fn bake(terrain: &Terrain) -> Self {
        let size = terrain.land_grid;
        let mut costs = Grid::filled(size, cost::OPEN);
        for z in 0..size.height {
            for x in 0..size.width {
                let geography = terrain.geography.get(x, z).copied().unwrap_or_default();
                let c = terrain_cost(geography, cell_slope(terrain, x, z));
                *costs.get_mut(x, z).expect("inside the grid") = c;
            }
        }
        Self {
            cell_size: terrain.land_cell_size,
            size,
            costs,
        }
    }

    /// Bakes the grid from `terrain` and the road network.
    pub fn bake_with_roads(terrain: &Terrain, roads: &RoadGraph) -> Self {
        let mut grid = Self::bake(terrain);
        for road in 0..roads.road_nodes.len() {
            for curve in roads.curve(road) {
                grid.bake_curve(&curve);
            }
        }
        grid
    }

    /// Bakes one road curve segment into the cells it crosses, at [`cost::ROAD`].
    ///
    /// A cell is only made cheaper, never walkable: a curve sampled over deep water (a bridge
    /// whose deck is a Roadway LOD, not terrain) does not open the water up.
    pub fn bake_curve(&mut self, curve: &CurveSegment) {
        let step = (self.cell_size / ROAD_SAMPLES_PER_CELL).max(0.5);
        let approx = (curve.p1 - curve.p0).length()
            + (curve.c1 - curve.p0).length()
            + (curve.c2 - curve.c1).length()
            + (curve.p1 - curve.c2).length();
        let n = ((approx / step).ceil() as usize).clamp(1, MAX_ROAD_SAMPLES);
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let p = curve.point(t);
            let Some((x, z)) = self.cell_of(Vec3::new(p.x, 0.0, p.y)) else {
                continue;
            };
            if self.cost(x, z) != IMPASSABLE {
                self.set_cost(x, z, ROAD);
            }
        }
    }

    /// A grid from an existing cost array (row-major, `size` cells) — a saved bake, a
    /// hand-built test grid. `None` when the length does not match the size.
    pub fn from_parts(cell_size: f32, size: GridSize, costs: Vec<u8>) -> Option<Self> {
        Some(Self {
            cell_size,
            size,
            costs: Grid::from_vec(size, costs)?,
        })
    }

    /// The grid dimensions, in cells.
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Edge length of one cell in metres.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Cells along x.
    pub fn width(&self) -> u32 {
        self.size.width
    }

    /// Cells along z.
    pub fn height(&self) -> u32 {
        self.size.height
    }

    /// The cost of cell `(x, z)`; [`IMPASSABLE`] outside the grid.
    pub fn cost(&self, x: u32, z: u32) -> u8 {
        self.costs.get(x, z).copied().unwrap_or(IMPASSABLE)
    }

    /// The cost of the cell containing world `p`, [`IMPASSABLE`] outside the grid.
    pub fn cost_at(&self, p: Vec3) -> u8 {
        self.cell_of(p).map_or(IMPASSABLE, |(x, z)| self.cost(x, z))
    }

    /// Whether the cell can be crossed at all.
    pub fn is_walkable(&self, x: u32, z: u32) -> bool {
        self.cost(x, z) != IMPASSABLE
    }

    /// Sets one cell's cost — the general patch for a dynamic obstacle.
    pub fn set_cost(&mut self, x: u32, z: u32, cost: u8) {
        if let Some(cell) = self.costs.get_mut(x, z) {
            *cell = cost;
        }
    }

    /// Sets every cell whose centre is within `radius` metres of `center`, returning how many
    /// cells changed.
    pub fn patch(&mut self, center: Vec3, radius: f32, cost: u8) -> usize {
        let mut patched = 0;
        for (x, z) in self.cells_within(center, radius) {
            if self.cost(x, z) != cost {
                patched += 1;
            }
            self.set_cost(x, z, cost);
        }
        patched
    }

    /// The cell containing world `p`, or `None` outside the grid.
    pub fn cell_of(&self, p: Vec3) -> Option<(u32, u32)> {
        let (cx, cz) = (p.x / self.cell_size, p.z / self.cell_size);
        if cx < 0.0 || cz < 0.0 {
            return None;
        }
        let (x, z) = (cx as u32, cz as u32);
        (x < self.size.width && z < self.size.height).then_some((x, z))
    }

    /// The centre of cell `(x, z)` in world x/z, at y = 0.
    pub fn cell_center(&self, x: u32, z: u32) -> Vec3 {
        Vec3::new(
            (x as f32 + 0.5) * self.cell_size,
            0.0,
            (z as f32 + 0.5) * self.cell_size,
        )
    }

    /// Every cell whose centre is within `radius` metres of `center`, in row-major order.
    pub fn cells_within(&self, center: Vec3, radius: f32) -> Vec<(u32, u32)> {
        let r = radius.max(0.0);
        if self.size.is_empty() {
            return Vec::new();
        }
        let min_x = ((center.x - r) / self.cell_size).floor().max(0.0) as u32;
        let max_x = ((center.x + r) / self.cell_size)
            .floor()
            .min(self.size.width as f32 - 1.0) as u32;
        let min_z = ((center.z - r) / self.cell_size).floor().max(0.0) as u32;
        let max_z = ((center.z + r) / self.cell_size)
            .floor()
            .min(self.size.height as f32 - 1.0) as u32;
        let mut cells = Vec::new();
        for z in min_z..=max_z {
            for x in min_x..=max_x {
                let d = self.cell_center(x, z) - center;
                if d.x.hypot(d.z) <= r {
                    cells.push((x, z));
                }
            }
        }
        cells
    }

    /// The walkable cell nearest to `p` with a centre within `radius` metres of it, searched
    /// ring by ring so the result is the closest one.
    pub fn nearest_walkable(&self, p: Vec3, radius: f32) -> Option<(u32, u32)> {
        let (cx, cz) = (
            (p.x / self.cell_size).floor(),
            (p.z / self.cell_size).floor(),
        );
        if cx < 0.0 || cz < 0.0 {
            return None;
        }
        let (cx, cz) = (cx as i64, cz as i64);
        let max_ring = (radius / self.cell_size).ceil().max(0.0) as i64;
        for ring in 0..=max_ring {
            let mut best: Option<(u32, u32)> = None;
            let mut best_d = f32::INFINITY;
            let mut consider = |x: i64, z: i64| {
                if x < 0
                    || z < 0
                    || x >= i64::from(self.size.width)
                    || z >= i64::from(self.size.height)
                {
                    return;
                }
                let (x, z) = (x as u32, z as u32);
                if !self.is_walkable(x, z) {
                    return;
                }
                let d = (self.cell_center(x, z) - p).length();
                if d <= radius && d < best_d {
                    best_d = d;
                    best = Some((x, z));
                }
            };
            for i in -ring..=ring {
                consider(cx + i, cz - ring);
                consider(cx + i, cz + ring);
                consider(cx - ring, cz + i);
                consider(cx + ring, cz + i);
            }
            if let Some(cell) = best {
                return Some(cell);
            }
        }
        None
    }

    /// All costs, row-major (cell `(x, z)` at `z * width + x`).
    pub fn as_slice(&self) -> &[u8] {
        self.costs.as_slice()
    }
}

impl std::fmt::Debug for NavGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NavGrid({}x{})", self.size.width, self.size.height)
    }
}

/// The cost of one land cell from its geography flags and its terrain slope.
///
/// Road first: the WRP road flag is how the engine carries bridge, runway and pier decks, so a
/// road cell is walkable even over deep water (the deck is a Roadway LOD, not terrain). Then
/// water, then slope, then cover: forest costs more than open ground, ground mostly covered by
/// objects more still (the engine's `AIPathPlanner` costs, `docs/re/navigation.md` §2).
pub fn terrain_cost(geography: Geography, slope: f32) -> u8 {
    if geography.road() {
        return cost::ROAD;
    }
    if geography.min_water_depth() >= DEEP_WATER_CLASS {
        return cost::IMPASSABLE;
    }
    if geography.max_water_depth() > 0 {
        return cost::SHALLOW_WATER;
    }
    if slope > crate::MAX_SLOPE {
        return cost::IMPASSABLE;
    }
    if geography.forest() {
        return cost::FOREST;
    }
    if geography.how_many_hard_objects() >= 2 || geography.how_many_objects() >= 2 {
        return cost::BUILT_UP;
    }
    cost::OPEN
}

/// The steepest terrain slope (rise over run) inside land cell `(x, z)`.
///
/// The cell's height samples are the quad `[i0, i1] x [j0, j1]`, split along the same diagonal
/// as [`Terrain::surface_height`]; the slope of each triangle is the length of its gradient.
pub fn cell_slope(terrain: &Terrain, x: u32, z: u32) -> f32 {
    let (hw, hh) = (terrain.heightmap.width(), terrain.heightmap.height());
    let step = (hw / terrain.land_grid.width.max(1)).max(1);
    let (i0, j0) = (x * step, z * step);
    let (i1, j1) = (
        (i0 + step).min(hw.saturating_sub(1)),
        (j0 + step).min(hh.saturating_sub(1)),
    );
    let sample = terrain.terrain_cell_size();
    let run_x = (i1 - i0) as f32 * sample;
    let run_z = (j1 - j0) as f32 * sample;
    if run_x <= 0.0 || run_z <= 0.0 {
        return 0.0;
    }
    let (i0, i1, j0, j1) = (i64::from(i0), i64::from(i1), i64::from(j0), i64::from(j1));
    let h00 = terrain.grid_height(i0, j0);
    let h10 = terrain.grid_height(i1, j0);
    let h01 = terrain.grid_height(i0, j1);
    let h11 = terrain.grid_height(i1, j1);
    let a = ((h10 - h00) / run_x).hypot((h01 - h00) / run_z);
    let b = ((h11 - h01) / run_x).hypot((h11 - h10) / run_z);
    a.max(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_wrp::TerrainBuilder;
    use glam::Vec2;

    /// A flat `land` x `land` terrain of `cell` metres, heights sampled `step` per land cell.
    fn flat_terrain(land: u32, step: u32, cell: f32) -> Terrain {
        TerrainBuilder::new(land, land * step, cell)
            .heights(|_, _| 0.0)
            .build()
    }

    /// A geography record with the fields the costs read.
    fn geography(min_water: u8, max_water: u8, forest: bool, road: bool, objects: u8) -> Geography {
        let mut bits = u16::from(min_water & 3) | (u16::from(max_water & 3) << 5);
        bits |= u16::from(objects & 3) << 7;
        bits |= u16::from(forest) << 3;
        bits |= u16::from(road) << 4;
        Geography(bits)
    }

    /// A straight curve from `a` to `b` (control points on the line).
    fn straight_curve(a: Vec2, b: Vec2) -> CurveSegment {
        CurveSegment {
            road: 0,
            index: 0,
            p0: a,
            c1: a + (b - a) / 3.0,
            c2: a + (b - a) * (2.0 / 3.0),
            p1: b,
            open_start: true,
            open_end: true,
        }
    }

    #[test]
    fn flat_open_ground_is_open() {
        let terrain = flat_terrain(4, 4, 10.0);
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.size(), GridSize::new(4, 4));
        assert_eq!(grid.cell_size(), 10.0);
        for z in 0..4 {
            for x in 0..4 {
                assert_eq!(grid.cost(x, z), cost::OPEN, "cell {x},{z}");
                assert!(grid.is_walkable(x, z));
            }
        }
    }

    #[test]
    fn water_is_waded_or_impassable() {
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|_, _| 0.0)
            .edit(|t| {
                *t.geography.get_mut(0, 0).unwrap() = geography(1, 1, false, false, 0);
                *t.geography.get_mut(1, 0).unwrap() = geography(2, 2, false, false, 0);
                *t.geography.get_mut(2, 0).unwrap() = geography(0, 1, false, false, 0);
            })
            .build();
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(0, 0), cost::SHALLOW_WATER);
        assert_eq!(grid.cost(1, 0), cost::IMPASSABLE);
        assert_eq!(grid.cost(2, 0), cost::SHALLOW_WATER);
        assert_eq!(grid.cost(3, 0), cost::OPEN);
    }

    #[test]
    fn forest_and_built_up_ground_cost_more() {
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|_, _| 0.0)
            .edit(|t| {
                *t.geography.get_mut(0, 0).unwrap() = geography(0, 0, true, false, 0);
                *t.geography.get_mut(1, 0).unwrap() = geography(0, 0, false, false, 2);
            })
            .build();
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(0, 0), cost::FOREST);
        assert_eq!(grid.cost(1, 0), cost::BUILT_UP);
    }

    #[test]
    fn the_wrp_road_flag_makes_a_cell_cheap() {
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|_, _| 0.0)
            .edit(|t| {
                *t.geography.get_mut(0, 0).unwrap() = geography(0, 0, false, true, 0);
            })
            .build();
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(0, 0), cost::ROAD);
    }

    #[test]
    fn a_cell_road_on_water_stays_a_road() {
        // The WRP road flag is how the engine carries bridge and runway decks: the cell is
        // walkable even when the water under it is deep.
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|_, _| 0.0)
            .edit(|t| {
                *t.geography.get_mut(0, 0).unwrap() = geography(3, 3, false, true, 0);
            })
            .build();
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(0, 0), cost::ROAD);
    }

    #[test]
    fn steep_cells_are_impassable() {
        // Height sample every 2.5 m, rising 2 m each: a slope of 0.8, steep for a man (0.6).
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|i, _| i as f32 * 2.0)
            .build();
        let grid = NavGrid::bake(&terrain);
        for z in 0..4 {
            assert_eq!(grid.cost(0, z), cost::IMPASSABLE, "cell 0,{z}");
        }
        // A gentle slope stays open: 0.8 m per 2.5 m sample is 0.32.
        let terrain = TerrainBuilder::new(4, 16, 10.0)
            .heights(|i, _| i as f32 * 0.8)
            .build();
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(0, 0), cost::OPEN);
    }

    #[test]
    fn a_road_curve_bakes_the_cells_it_crosses() {
        let terrain = flat_terrain(8, 4, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        grid.bake_curve(&straight_curve(Vec2::new(35.0, 5.0), Vec2::new(35.0, 75.0)));
        for z in 0..7 {
            assert_eq!(grid.cost(3, z), cost::ROAD, "cell 3,{z}");
        }
        assert_eq!(grid.cost(0, 0), cost::OPEN);
        assert_eq!(grid.cost(7, 7), cost::OPEN);
    }

    #[test]
    fn a_road_curve_does_not_open_impassable_water() {
        let terrain = TerrainBuilder::new(8, 32, 10.0)
            .heights(|_, _| 0.0)
            .edit(|t| {
                *t.geography.get_mut(3, 3).unwrap() = geography(3, 3, false, false, 0);
            })
            .build();
        let mut grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(3, 3), cost::IMPASSABLE);
        grid.bake_curve(&straight_curve(Vec2::new(35.0, 5.0), Vec2::new(35.0, 75.0)));
        assert_eq!(grid.cost(3, 3), cost::IMPASSABLE);
        assert_eq!(grid.cost(3, 2), cost::ROAD);
    }

    #[test]
    fn nearest_walkable_finds_the_closest_cell_and_honours_the_radius() {
        let terrain = flat_terrain(8, 4, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        for z in 3..5 {
            for x in 3..5 {
                grid.set_cost(x, z, cost::IMPASSABLE);
            }
        }
        // The centre of the 2x2 hole is the corner of four blocked cells. The cells beside it
        // share the hole's blocked row or column, so the nearest walkable cell centre is the
        // diagonal one (e.g. cell (3, 2), centre (35, 25)): sqrt(5^2 + 15^2) = 15.8 m.
        let p = Vec3::new(40.0, 0.0, 40.0);
        let cell = grid.nearest_walkable(p, 16.0).expect("a cell within 16 m");
        assert!(grid.is_walkable(cell.0, cell.1));
        assert!((grid.cell_center(cell.0, cell.1) - p).length() <= 16.0);
        assert!(grid.nearest_walkable(p, 5.0).is_none());
    }

    #[test]
    fn patch_blocks_a_disc_and_reports_the_change() {
        let terrain = flat_terrain(8, 4, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        let changed = grid.patch(Vec3::new(40.0, 0.0, 40.0), 11.0, cost::IMPASSABLE);
        assert!(changed > 0);
        assert_eq!(grid.cost(4, 4), cost::IMPASSABLE);
        assert_eq!(grid.cost(0, 0), cost::OPEN);
        assert_eq!(
            grid.patch(Vec3::new(40.0, 0.0, 40.0), 11.0, cost::IMPASSABLE),
            0
        );
    }

    #[test]
    fn cells_outside_the_grid_cost_nothing() {
        let terrain = flat_terrain(4, 4, 10.0);
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cell_of(Vec3::new(-1.0, 0.0, 5.0)), None);
        assert_eq!(grid.cell_of(Vec3::new(5.0, 0.0, 100.0)), None);
        assert_eq!(grid.cost_at(Vec3::new(5.0, 0.0, 100.0)), IMPASSABLE);
        assert_eq!(grid.cell_of(Vec3::new(5.0, 0.0, 5.0)), Some((0, 0)));
        assert_eq!(grid.cell_of(Vec3::new(15.0, 0.0, 25.0)), Some((1, 2)));
    }
}
