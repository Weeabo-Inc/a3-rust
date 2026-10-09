//! The query an AI asks: [`Navigator`].
//!
//! The navigator owns the map side — the terrain, its cost grid ([`NavGrid`]) and the terrain
//! query used to pull paths straight — and hands out plans. The search scratch is per-AI
//! ([`Planner`]): an agent keeps one and calls [`Navigator::find_path_with`], so planning never
//! allocates and two agents never share mutable state. [`Navigator::find_path`] is the
//! one-shot convenience for callers that plan rarely.
//!
//! Both ends are snapped into the grid within `radius` metres ([`NavGrid::nearest_walkable`]),
//! which is how a caller standing in a doorway or on a rock still gets a path: the plan starts
//! and ends at the positions given, not at cell centres.

use std::sync::Arc;

use a3_landscape::RoadGraph;
use a3_physics::{CollisionWorld, Layer};
use a3_wrp::Terrain;
use glam::{DVec3, Vec3};

use crate::astar::Planner;
use crate::grid::{self, NavGrid, cost};
use crate::smooth::string_pull;
use crate::{BLOCKER_HEIGHT, MAX_SLOPE, STEP_UP};

/// Terrain, grid and the terrain query — the part of navigation that is shared.
pub struct Navigator {
    terrain: Arc<Terrain>,
    grid: NavGrid,
    planner: Planner,
}

impl Navigator {
    /// A navigator over `terrain`, its grid baked from geography flags and slope.
    pub fn new(terrain: Arc<Terrain>) -> Self {
        let grid = NavGrid::bake(&terrain);
        Self {
            terrain,
            grid,
            planner: Planner::new(),
        }
    }

    /// A navigator whose grid also has the road network baked in: the shapefile roads become
    /// cheap cells, so long paths follow them (the engine's `AIpathOffset`-weighted cost map).
    pub fn with_roads(terrain: Arc<Terrain>, roads: &RoadGraph) -> Self {
        let grid = NavGrid::bake_with_roads(&terrain, roads);
        Self {
            terrain,
            grid,
            planner: Planner::new(),
        }
    }

    /// The cost grid (read-only: patches go through [`Navigator::grid_mut`]).
    pub fn grid(&self) -> &NavGrid {
        &self.grid
    }

    /// The cost grid, to patch with dynamic obstacles ([`NavGrid::patch`]).
    pub fn grid_mut(&mut self) -> &mut NavGrid {
        &mut self.grid
    }

    /// The terrain the grid was baked from.
    pub fn terrain(&self) -> &Arc<Terrain> {
        &self.terrain
    }

    /// Plans a path from `from` to `to`, snapping both ends into the grid within `radius`
    /// metres, and returns positions in world space — the caller's own ends, cell centres in
    /// between, pulled straight where the ground allows.
    ///
    /// A planned cell path is kept per call in the navigator's own scratch; an agent that plans
    /// often should keep its own [`Planner`] and call [`Navigator::find_path_with`].
    pub fn find_path(&mut self, from: Vec3, to: Vec3, radius: f32) -> Option<Vec<Vec3>> {
        Self::route(
            &self.terrain,
            &self.grid,
            &mut self.planner,
            from,
            to,
            radius,
        )
    }

    /// [`Navigator::find_path`] with the caller's own search scratch, so one navigator serves
    /// many agents at once (the search takes `&self`, only the planner is `&mut`).
    pub fn find_path_with(
        &self,
        planner: &mut Planner,
        from: Vec3,
        to: Vec3,
        radius: f32,
    ) -> Option<Vec<Vec3>> {
        Self::route(&self.terrain, &self.grid, planner, from, to, radius)
    }

    /// The plan itself: snap, search, turn cells into positions, pull straight.
    fn route(
        terrain: &Terrain,
        grid: &NavGrid,
        planner: &mut Planner,
        from: Vec3,
        to: Vec3,
        radius: f32,
    ) -> Option<Vec<Vec3>> {
        let start = grid.nearest_walkable(from, radius)?;
        let goal = grid.nearest_walkable(to, radius)?;
        let cells = planner.find_path(grid, start, goal)?.to_vec();
        let mut points = Vec::with_capacity(cells.len() + 2);
        points.push(ground(terrain, from.x, from.z));
        points.extend(cells.iter().map(|&(x, z)| {
            let c = grid.cell_center(x, z);
            ground(terrain, c.x, c.z)
        }));
        points.push(ground(terrain, to.x, to.z));
        let clear = |a: Vec3, b: Vec3| line_clear(terrain, grid, a, b);
        let mut pulled = string_pull(&points, clear);
        // The pull can leave a cell centre coincident with the snapped end; drop repeats.
        pulled.dedup_by(|a, b| (*a - *b).length_squared() < 1e-6);
        Some(pulled)
    }

    /// Blocks every cell whose centre is within `radius` metres of `center` that has a Geometry
    /// solid at man height: the point between [`STEP_UP`] and [`BLOCKER_HEIGHT`] above the
    /// ground. Returns how many cells changed.
    ///
    /// This is the obstacle footprint of what physics has loaded — a crate, a fence, a vehicle
    /// hull. A house's *interior* is not inside a Geometry solid, so it stays walkable on the
    /// grid; indoor movement is [`PathMesh`](crate::PathMesh)'s business.
    pub fn block_from_colliders(
        &mut self,
        world: &CollisionWorld,
        center: Vec3,
        radius: f32,
    ) -> usize {
        let mut blocked = 0;
        for (x, z) in self.cells_within(center, radius) {
            if self.grid.cost(x, z) == cost::IMPASSABLE {
                continue;
            }
            let c = self.grid.cell_center(x, z);
            let surface = self.terrain.surface_height(c.x, c.z);
            let chest = f64::from(surface + (STEP_UP + BLOCKER_HEIGHT) * 0.5);
            let point = DVec3::new(c.x as f64, chest, c.z as f64);
            if !world.objects_at(point, Layer::Geometry.mask()).is_empty() {
                self.grid.set_cost(x, z, cost::IMPASSABLE);
                blocked += 1;
            }
        }
        blocked
    }

    /// The cells within `radius` of `center` that are walkable — [`NavGrid::cells_within`],
    /// filtered.
    fn cells_within(&self, center: Vec3, radius: f32) -> Vec<(u32, u32)> {
        self.grid
            .cells_within(center, radius)
            .into_iter()
            .filter(|&(x, z)| self.grid.is_walkable(x, z))
            .collect()
    }
}

/// `p` at the terrain's height.
fn ground(terrain: &Terrain, x: f32, z: f32) -> Vec3 {
    Vec3::new(x, terrain.surface_height(x, z), z)
}

/// Whether a man can walk the straight line from `a` to `b`: every sample on walkable ground,
/// and no rise between samples beyond [`MAX_SLOPE`] (over the distance walked) plus one
/// [`STEP_UP`].
fn line_clear(terrain: &Terrain, grid: &NavGrid, a: Vec3, b: Vec3) -> bool {
    let step = terrain.terrain_cell_size().max(0.5);
    let len = ((b.x - a.x).powi(2) + (b.z - a.z).powi(2)).sqrt();
    let n = (len / step).ceil().max(1.0) as u32;
    let spacing = len / n as f32;
    let mut previous: Option<f32> = None;
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let (x, z) = (a.x + (b.x - a.x) * t, a.z + (b.z - a.z) * t);
        if grid.cost_at(Vec3::new(x, 0.0, z)) == grid::cost::IMPASSABLE {
            return false;
        }
        let h = terrain.surface_height(x, z);
        if let Some(p) = previous {
            if (h - p).abs() > MAX_SLOPE * spacing + STEP_UP {
                return false;
            }
        }
        previous = Some(h);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{
        Encoding, Face, Lod, LodResolution, Material, Model, ModelInfo, NamedSelection, Section,
    };
    use a3_physics::{Interest, MemoryFiles, ModelBank, TerrainStatics};
    use a3_wrp::{TerrainBuilder, Transform};

    /// A flat 8 x 8 land grid of 16 m cells (128 m square), heights every 4 m.
    fn flat_terrain() -> Arc<Terrain> {
        Arc::new(TerrainBuilder::new(8, 32, 16.0).heights(|_, _| 0.0).build())
    }

    fn navigator() -> Navigator {
        Navigator::new(flat_terrain())
    }

    fn at(x: f32, z: f32) -> Vec3 {
        Vec3::new(x, 0.0, z)
    }

    /// A path over open ground: both ends kept as given, everything between on walkable cells.
    ///
    /// The snap radius has to reach the nearest cell *centre*, which on a 16 m grid is up to
    /// 11 m away — hence 40 m here and in the tests below unless the test is about snapping.
    #[test]
    fn a_straight_path_keeps_both_ends() {
        let mut nav = navigator();
        let path = nav
            .find_path(at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path");
        assert_eq!(path.first().copied(), Some(at(20.0, 20.0)));
        assert_eq!(path.last().copied(), Some(at(100.0, 100.0)));
        for p in &path {
            assert_ne!(nav.grid().cost_at(*p), cost::IMPASSABLE, "{p:?}");
        }
    }

    /// The path is a plan, not the raw cell centres: with nothing in the way, the ends alone
    /// are enough.
    #[test]
    fn open_ground_collapses_the_path_to_its_ends() {
        let mut nav = navigator();
        let path = nav
            .find_path(at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path");
        assert_eq!(path.len(), 2, "{path:?}");
    }

    /// A wall across the map with one gap: the plan bends through the gap.
    #[test]
    fn a_wall_is_walked_around_through_its_gap() {
        let mut nav = navigator();
        for z in 0..8 {
            nav.grid_mut().set_cost(4, z, cost::IMPASSABLE);
        }
        nav.grid_mut().set_cost(4, 6, cost::OPEN); // the gap
        let path = nav
            .find_path(at(20.0, 20.0), at(110.0, 20.0), 40.0)
            .expect("a path through the gap");
        for p in &path {
            assert_ne!(nav.grid().cost_at(*p), cost::IMPASSABLE, "{p:?}");
        }
        // The wall is the 4th cell column (x 64..80) and the gap is at z 96..112, so a plan
        // that does not use the gap could never reach z > 90 at all.
        assert!(
            path.iter().any(|p| p.z > 90.0),
            "the path did not use the gap: {path:?}"
        );
    }

    /// A blob in the middle of the map bends the path, and no waypoint sits on it.
    #[test]
    fn the_path_bends_around_a_blocked_blob() {
        let mut nav = navigator();
        for z in 3..5 {
            for x in 3..5 {
                nav.grid_mut().set_cost(x, z, cost::IMPASSABLE);
            }
        }
        let path = nav
            .find_path(at(10.0, 10.0), at(118.0, 118.0), 5.0)
            .expect("a path around the blob");
        assert!(path.len() > 2, "{path:?}");
        for &(x, z) in &[(3, 3), (3, 4), (4, 3), (4, 4)] {
            let c = nav.grid().cell_center(x, z);
            for p in &path {
                assert!(
                    (p.x - c.x).hypot(p.z - c.z) > 1.0,
                    "waypoint {p:?} on the blocked cell {x},{z}"
                );
            }
        }
    }

    /// Nothing can be planned to a cell no walkable cell is near.
    #[test]
    fn a_goal_surrounded_by_impassable_ground_has_no_path() {
        let mut nav = navigator();
        let goal = nav.grid().cell_center(6, 6);
        for z in 5..=7 {
            for x in 5..=7 {
                nav.grid_mut().set_cost(x, z, cost::IMPASSABLE);
            }
        }
        nav.grid_mut().set_cost(6, 6, cost::OPEN);
        // The goal cell itself is walkable but walled in on all eight sides.
        assert!(nav.find_path(at(20.0, 20.0), goal, 40.0).is_none());
    }

    /// A caller on unwalkable ground is snapped within the radius given — and not beyond it.
    #[test]
    fn an_end_is_snapped_within_the_radius_only() {
        let mut nav = navigator();
        // Cell (0, 0) is water deep enough to be impassable.
        nav.grid_mut().set_cost(0, 0, cost::IMPASSABLE);
        let from = at(8.0, 8.0);
        assert!(nav.find_path(from, at(100.0, 100.0), 1.0).is_none());
        let path = nav
            .find_path(from, at(100.0, 100.0), 30.0)
            .expect("snapped to a walkable cell");
        assert_eq!(path.first().copied(), Some(from));
    }

    /// The grid is live: a patch between two calls changes the second plan.
    #[test]
    fn a_patch_moves_the_path() {
        let mut nav = navigator();
        let before = nav
            .find_path(at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path");
        assert_eq!(before.len(), 2, "open ground: the ends alone");
        // Wall off row 3 (z 48..64) across the map except one cell far to the east.
        for x in 0..8 {
            nav.grid_mut().set_cost(x, 3, cost::IMPASSABLE);
        }
        nav.grid_mut().set_cost(7, 3, cost::OPEN);
        let after = nav
            .find_path(at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path through the gap");
        assert!(after.len() > 2, "the wall is crossed: {after:?}");
        for p in &after {
            assert_ne!(nav.grid().cost_at(*p), cost::IMPASSABLE, "{p:?}");
        }
        // The only crossing is cell (7, 3), x 112..128: a straight leg crossing the wall there
        // has an end east of x 112, so the plan must reach it.
        assert!(after.iter().any(|p| p.x >= 112.0), "{after:?}");
    }

    /// One navigator, many agents: plans through an external planner, run in any order.
    #[test]
    fn a_shared_navigator_plans_for_many_planners() {
        let nav = navigator();
        let mut a = Planner::new();
        let mut b = Planner::new();
        let first = nav
            .find_path_with(&mut a, at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path");
        let second = nav
            .find_path_with(&mut b, at(100.0, 20.0), at(20.0, 100.0), 40.0)
            .expect("a path");
        let again = nav
            .find_path_with(&mut a, at(20.0, 20.0), at(100.0, 100.0), 40.0)
            .expect("a path");
        assert_eq!(first, again);
        assert_eq!(first.first().copied(), Some(at(20.0, 20.0)));
        assert_eq!(second.first().copied(), Some(at(100.0, 20.0)));
        assert_ne!(first, second);
    }

    /// A steep rise is not crossed by a straight line: [`line_clear`] rejects it, which is what
    /// stops the string pull from cutting over a cliff.
    #[test]
    fn a_cliff_is_not_crossed_by_a_straight_line() {
        // Land cells of 10 m, height samples every 2.5 m: the heights jump 40 m at sample 16
        // (x = 40), so land cell 3 (x 30..40) has a slope of 4 and bakes impassable.
        let terrain = Arc::new(
            TerrainBuilder::new(16, 64, 10.0)
                .heights(|i, _| if i >= 16 { 40.0 } else { 0.0 })
                .build(),
        );
        let grid = NavGrid::bake(&terrain);
        assert_eq!(grid.cost(3, 4), cost::IMPASSABLE);
        // Flat ground either side of the cliff is clear...
        let a = ground(&terrain, 5.0, 40.0);
        let b = ground(&terrain, 25.0, 40.0);
        assert!(line_clear(&terrain, &grid, a, b));
        // ...and a line that crosses the cliff is not.
        let c = ground(&terrain, 45.0, 40.0);
        assert!(!line_clear(&terrain, &grid, b, c));
    }

    /// Physics obstacles: a Geometry box on a cell blocks that cell (and only it).
    #[test]
    fn block_from_colliders_marks_the_cells_under_a_box() {
        let terrain = Arc::new(
            TerrainBuilder::new(8, 32, 16.0)
                .heights(|_, _| 0.0)
                .object(
                    "a3\\box.p3d",
                    Transform::from_position(Vec3::new(40.0, 0.0, 40.0)),
                )
                .build(),
        );
        let (world, statics) = collision_world(terrain.clone());
        let mut nav = Navigator::new(terrain);
        let box_center = at(40.0, 40.0);
        assert_eq!(nav.block_from_colliders(&world, box_center, 10.0), 1);
        assert_eq!(nav.grid().cost(2, 2), cost::IMPASSABLE);
        assert!(nav.grid().is_walkable(1, 2));
        assert!(nav.grid().is_walkable(3, 2));
        // Running it again changes nothing (the cell is already blocked).
        assert_eq!(nav.block_from_colliders(&world, box_center, 10.0), 0);
        let _ = statics;
    }

    /// A collision world with one box model placed by `terrain`, streamed in.
    fn collision_world(terrain: Arc<Terrain>) -> (CollisionWorld, TerrainStatics) {
        let mut files = MemoryFiles::default();
        files.insert(
            "concrete.bisurf",
            b"rough=0.1;soundEnviron=concrete;bulletPenetrability=80;" as &[u8],
        );
        let config = a3_config::parse_text(
            r#"
class CfgSurfaces {
    class Default { files = "default"; soundEnviron = "dirt"; };
    class Concrete: Default { files = "betonout"; soundEnviron = "concrete"; };
};
"#,
        )
        .expect("valid config");
        let tree = a3_config::ConfigTree::from_config(&config);
        let mut bank = ModelBank::new(Arc::new(files), Some(&tree));
        bank.insert_model("a3\\box.p3d", &box_model());
        let mut world = CollisionWorld::new(bank);
        world.set_terrain(Some(terrain.clone()));
        let statics = TerrainStatics::new(terrain);
        world.stream(
            &statics,
            &[Interest {
                center: DVec3::new(40.0, 0.0, 40.0),
                radius: 60.0,
            }],
        );
        (world, statics)
    }

    /// An 8 x 3 x 8 m box as a Geometry LOD (its own component, material and selection, the
    /// shape the collision bank expects).
    fn box_model() -> Model {
        let min = Vec3::new(-4.0, 0.0, -4.0);
        let max = Vec3::new(4.0, 3.0, 4.0);
        let mut lod = Lod {
            resolution: LodResolution(1e13),
            ..Default::default()
        };
        for i in 0..8 {
            lod.vertices.positions.push(Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            ));
        }
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            lod.faces.push(Face::quad(q[0], q[1], q[2], q[3]));
        }
        lod.materials.push(Material {
            name: "box.rvmat".to_owned(),
            surface: "concrete".to_owned(),
            ..Default::default()
        });
        lod.sections.push(Section {
            faces: 0..6,
            material: Some(0),
            ..Default::default()
        });
        lod.named_selections.push(NamedSelection {
            name: "Component01".to_owned(),
            faces: (0..6).collect(),
            vertices: (0..8).collect(),
            ..Default::default()
        });
        let mut model = Model {
            encoding: Encoding::Mlod,
            version: 257,
            info: ModelInfo::default(),
            skeleton: None,
            animations: Vec::new(),
            lods: vec![lod],
        };
        model.info.mass = 1000.0;
        model
    }

    #[test]
    fn an_empty_terrain_has_no_paths() {
        let terrain = Arc::new(TerrainBuilder::new(0, 0, 16.0).build());
        let mut nav = Navigator::new(terrain);
        assert!(nav.find_path(at(0.0, 0.0), at(0.0, 0.0), 10.0).is_none());
    }
}
