//! A* over a [`NavGrid`], with a reusable per-planner scratch.
//!
//! One [`Planner`] per AI (the engine has one `AIPathPlanner` per unit and advances the search
//! in bounded steps, `AIPathPlanner::ProcessSearching`; `docs/re/navigation.md` §1). Costs are
//! integers (milli-cost: one metre of open ground is 1000), so the search is exact and
//! deterministic: no float rounding decides between two paths.

use crate::grid::{NavGrid, cost};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The largest f value we ever compute: a path of at most a few thousand cells.
type Cost = u32;

/// No predecessor.
const NONE: u32 = u32::MAX;

/// How many cells a search expands before it gives up. The engine's planner spreads the work
/// over frames instead; ours stops and lets the caller retry with a larger budget.
pub const DEFAULT_BUDGET: usize = 250_000;

/// A* over a [`NavGrid`], keeping its scratch between searches.
#[derive(Debug, Clone)]
pub struct Planner {
    g: Vec<Cost>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    closed: Vec<u32>,
    generation: u32,
    open: BinaryHeap<Reverse<(Cost, u32)>>,
    path: Vec<(u32, u32)>,
    budget: usize,
    expanded: usize,
    /// Milli-metres of one cardinal and one diagonal step (from the grid's cell size).
    straight: Cost,
    diagonal: Cost,
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
    }
}

impl Planner {
    /// A planner with the default budget.
    pub fn new() -> Self {
        Self {
            g: Vec::new(),
            parent: Vec::new(),
            stamp: Vec::new(),
            closed: Vec::new(),
            generation: 0,
            open: BinaryHeap::new(),
            path: Vec::new(),
            budget: DEFAULT_BUDGET,
            expanded: 0,
            straight: 0,
            diagonal: 0,
        }
    }

    /// The most cells one search may expand.
    pub fn set_budget(&mut self, cells: usize) {
        self.budget = cells;
    }

    /// How many cells the last search expanded.
    pub fn expanded(&self) -> usize {
        self.expanded
    }

    /// The path of the last search, in cells, both ends included.
    pub fn path(&self) -> &[(u32, u32)] {
        &self.path
    }

    /// Plans a path of cells from `from` to `to`, both walkable, returning it with both ends
    /// included, or `None` when there is no path (or the search ran past its budget).
    pub fn find_path(
        &mut self,
        grid: &NavGrid,
        from: (u32, u32),
        to: (u32, u32),
    ) -> Option<&[(u32, u32)]> {
        self.path.clear();
        self.expanded = 0;
        if !grid.is_walkable(from.0, from.1) || !grid.is_walkable(to.0, to.1) {
            return None;
        }
        let width = grid.width() as usize;
        let height = grid.height() as usize;
        self.ensure_capacity(width * height);
        self.next_generation();

        let start = (from.1 as usize * width + from.0 as usize) as u32;
        let goal = (to.1 as usize * width + to.0 as usize) as u32;

        let cell = grid.cell_size();
        self.straight = (cell * 1000.0).round() as Cost;
        self.diagonal = (cell * 1000.0 * std::f32::consts::SQRT_2).round() as Cost;

        self.open.clear();
        self.mark(start);
        self.g[start as usize] = 0;
        self.parent[start as usize] = NONE;
        self.open
            .push(Reverse((self.heuristic(grid, from, to), start)));

        while let Some(Reverse((f, index))) = self.open.pop() {
            if self.closed[index as usize] == self.generation {
                continue;
            }
            if index == goal {
                self.reconstruct(width, goal);
                return Some(&self.path);
            }
            if self.expanded >= self.budget {
                return None;
            }
            // The queued f must match the node's best g, or the entry is stale.
            let expected = self.g[index as usize] + self.h_from(grid, index, to);
            if f > expected {
                continue;
            }
            self.expanded += 1;
            self.closed[index as usize] = self.generation;

            let (x, z) = (
                (index as usize % width) as u32,
                (index as usize / width) as u32,
            );
            for (dx, dz) in NEIGHBOURS {
                let nx = x as i64 + dx;
                let nz = z as i64 + dz;
                if nx < 0 || nz < 0 || nx >= width as i64 || nz >= height as i64 {
                    continue;
                }
                let (nx, nz) = (nx as u32, nz as u32);
                let neighbour_cost = grid.cost(nx, nz);
                if neighbour_cost == cost::IMPASSABLE {
                    continue;
                }
                // No corner cutting: a diagonal step needs both of the cells beside it.
                if dx != 0
                    && dz != 0
                    && (!grid.is_walkable(x.wrapping_add_signed(dx as i32), z)
                        || !grid.is_walkable(x, z.wrapping_add_signed(dz as i32)))
                {
                    continue;
                }
                let n = (nz as usize * width + nx as usize) as u32;
                let step = self.scale_step(dx, dz, grid.cost(x, z), neighbour_cost);
                let tentative = self.g[index as usize] + step;
                if self.stamp[n as usize] != self.generation {
                    self.mark(n);
                } else if tentative >= self.g[n as usize] {
                    continue;
                }
                self.g[n as usize] = tentative;
                self.parent[n as usize] = index;
                let h = self.h_from(grid, n, to);
                self.open.push(Reverse((tentative + h, n)));
            }
        }
        None
    }

    /// The moving cost of one step, in milli-cost: the distance times the average of the two
    /// cells' costs (per cent of open ground).
    fn scale_step(&self, dx: i64, dz: i64, from_cost: u8, to_cost: u8) -> Cost {
        let distance = if dx != 0 && dz != 0 {
            self.diagonal
        } else {
            self.straight
        };
        distance * (from_cost as Cost + to_cost as Cost) / 200
    }

    fn ensure_capacity(&mut self, cells: usize) {
        if self.g.len() < cells {
            self.g.resize(cells, 0);
            self.parent.resize(cells, NONE);
            self.stamp.resize(cells, 0);
            self.closed.resize(cells, 0);
        }
    }

    fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.fill(0);
            self.closed.fill(0);
            self.generation = 1;
        }
    }

    fn mark(&mut self, index: u32) {
        self.stamp[index as usize] = self.generation;
        self.g[index as usize] = Cost::MAX;
    }

    fn heuristic(&self, grid: &NavGrid, from: (u32, u32), to: (u32, u32)) -> Cost {
        let dx = from.0.abs_diff(to.0) as f32;
        let dz = from.1.abs_diff(to.1) as f32;
        let (lo, hi) = if dx < dz { (dx, dz) } else { (dz, dx) };
        let cells = lo * std::f32::consts::SQRT_2 + (hi - lo);
        // Admissible: the cheapest cell an AI can cross is a road (cost::ROAD).
        (cells * grid.cell_size() * 1000.0 * (cost::ROAD as f32) / 100.0) as Cost
    }

    fn h_from(&self, grid: &NavGrid, index: u32, to: (u32, u32)) -> Cost {
        let width = grid.width();
        self.heuristic(grid, (index % width, index / width), to)
    }

    fn reconstruct(&mut self, width: usize, goal: u32) {
        let mut index = goal;
        while index != NONE {
            self.path.push((
                (index as usize % width) as u32,
                (index as usize / width) as u32,
            ));
            index = self.parent[index as usize];
        }
        self.path.reverse();
    }
}

/// The eight neighbours, cardinals first so a tie is broken towards the straight steps.
const NEIGHBOURS: [(i64, i64); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::cost;
    use a3_wrp::{GridSize, Terrain, TerrainBuilder};

    fn flat_terrain(land: u32, cell: f32) -> Terrain {
        TerrainBuilder::new(land, land * 4, cell)
            .heights(|_, _| 0.0)
            .build()
    }

    #[test]
    fn a_straight_path_visits_every_cell_between() {
        let terrain = flat_terrain(8, 10.0);
        let grid = NavGrid::bake(&terrain);
        let mut planner = Planner::new();
        let path = planner
            .find_path(&grid, (1, 1), (5, 1))
            .expect("a path")
            .to_vec();
        assert_eq!(path, vec![(1, 1), (2, 1), (3, 1), (4, 1), (5, 1)]);
    }

    #[test]
    fn a_diagonal_goal_is_reached_by_diagonal_steps() {
        let terrain = flat_terrain(8, 10.0);
        let grid = NavGrid::bake(&terrain);
        let mut planner = Planner::new();
        let path = planner
            .find_path(&grid, (0, 0), (4, 4))
            .expect("a path")
            .to_vec();
        assert_eq!(path.len(), 5, "{path:?}");
        for (i, cell) in path.iter().enumerate() {
            assert_eq!(*cell, (i as u32, i as u32), "{path:?}");
        }
    }

    #[test]
    fn a_wall_is_walked_around_through_its_gap() {
        let terrain = flat_terrain(16, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        for z in 0..16 {
            grid.set_cost(8, z, cost::IMPASSABLE);
        }
        grid.set_cost(8, 12, cost::OPEN); // the gap
        let mut planner = Planner::new();
        let path = planner
            .find_path(&grid, (0, 8), (15, 8))
            .expect("a path")
            .to_vec();
        assert!(path.contains(&(8, 12)), "{path:?}");
        for &(x, z) in &path {
            assert!(grid.is_walkable(x, z), "{x},{z} in {path:?}");
        }
    }

    #[test]
    fn a_diagonal_step_does_not_cut_a_corner() {
        let terrain = flat_terrain(8, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        grid.set_cost(2, 1, cost::IMPASSABLE);
        grid.set_cost(1, 2, cost::IMPASSABLE);
        let mut planner = Planner::new();
        let path = planner
            .find_path(&grid, (1, 1), (2, 2))
            .expect("a path around the corner")
            .to_vec();
        assert!(path.len() > 2, "the corner was cut: {path:?}");
    }

    #[test]
    fn an_impassable_goal_has_no_path() {
        let terrain = flat_terrain(8, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        grid.set_cost(4, 4, cost::IMPASSABLE);
        let mut planner = Planner::new();
        assert!(planner.find_path(&grid, (0, 0), (4, 4)).is_none());
    }

    #[test]
    fn the_search_stops_at_its_budget() {
        let terrain = flat_terrain(64, 10.0);
        let grid = NavGrid::bake(&terrain);
        let mut planner = Planner::new();
        planner.set_budget(10);
        assert!(planner.find_path(&grid, (0, 0), (63, 63)).is_none());
        planner.set_budget(100_000);
        assert!(planner.find_path(&grid, (0, 0), (63, 63)).is_some());
    }

    #[test]
    fn a_road_is_preferred_over_open_ground() {
        let terrain = flat_terrain(16, 10.0);
        let mut grid = NavGrid::bake(&terrain);
        // A cheap road column at x = 8 spanning the whole map: the path from the left edge to
        // the right edge should step onto it and follow it rather than crossing open ground.
        for z in 0..16 {
            grid.set_cost(8, z, cost::ROAD);
        }
        let mut planner = Planner::new();
        let path = planner
            .find_path(&grid, (0, 8), (15, 8))
            .expect("a path")
            .to_vec();
        assert!(path.contains(&(8, 8)), "{path:?}");
        assert!(
            path.iter()
                .filter(|c| grid.cost(c.0, c.1) == cost::ROAD)
                .count()
                >= 1,
            "{path:?}"
        );
    }

    #[test]
    fn a_reused_planner_gives_the_same_path() {
        let terrain = flat_terrain(32, 10.0);
        let grid = NavGrid::bake(&terrain);
        let mut planner = Planner::new();
        let first = planner.find_path(&grid, (1, 1), (30, 20)).unwrap().to_vec();
        // A second, unrelated search in between must not disturb the scratch.
        planner.find_path(&grid, (20, 5), (2, 30)).unwrap();
        let again = planner.find_path(&grid, (1, 1), (30, 20)).unwrap().to_vec();
        assert_eq!(first, again);
        assert_eq!(planner.path(), again.as_slice());
    }

    #[test]
    fn an_empty_grid_has_no_path() {
        let grid =
            NavGrid::from_parts(10.0, GridSize::new(0, 0), Vec::new()).expect("an empty grid");
        let mut planner = Planner::new();
        assert!(planner.find_path(&grid, (0, 0), (0, 0)).is_none());
    }
}
