//! Terrain level of detail: a CDLOD quadtree over the heightmap.
//!
//! Every node is drawn with the same grid patch of [`LodSettings::leaf_cells`] quads per side;
//! a node at level `l` spaces its vertices `2^l` height cells apart. Level `l` is used up to
//! `range(l)` metres from the camera (`lod0_range * 2^l`). Within the last part of its range
//! (from `morph_start * range(l)`) the vertex shader morphs a node's odd vertices onto its
//! parent's grid, so where two levels meet their vertices coincide and there are no cracks.
//! [`LodQuadtree::select`] picks the nodes, frustum-culled, for one frame.

use a3_render::Frustum;
use glam::{DVec3, Vec3};

use crate::heights::{HeightField, MinMaxPyramid};

/// LOD tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodSettings {
    /// Quads per side of the patch every node is drawn with (even).
    pub leaf_cells: u32,
    /// Range of level 0 as a multiple of a level-0 node's edge length.
    pub lod0_range_factor: f64,
    /// Where morphing starts, as a fraction of a level's range.
    pub morph_start: f64,
}

impl Default for LodSettings {
    fn default() -> Self {
        LodSettings {
            leaf_cells: 32,
            lod0_range_factor: 4.0,
            morph_start: 0.8,
        }
    }
}

/// One area to draw this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectedNode {
    /// LOD level: vertices are `2^level` height cells apart.
    pub level: u32,
    /// South-west corner in height cells.
    pub x: u32,
    pub z: u32,
    /// Edge length in height cells: `leaf << level` for a whole node, half that for a quarter
    /// of a node whose child was out of the child level's range.
    pub cells: u32,
    /// Height bounds in metres.
    pub min_height: f32,
    pub max_height: f32,
}

impl SelectedNode {
    /// Whether this is a quarter of a node (drawn with half the patch).
    pub fn is_quarter(&self, leaf_cells: u32) -> bool {
        self.cells < leaf_cells << self.level
    }
}

/// The LOD quadtree of one heightmap.
#[derive(Debug, Clone)]
pub struct LodQuadtree {
    settings: LodSettings,
    pyramid: MinMaxPyramid,
    /// Height cell edge in metres.
    cell: f64,
    /// Levels; the root is level `levels - 1`.
    levels: u32,
}

impl LodQuadtree {
    pub fn new(heights: &HeightField, settings: LodSettings) -> LodQuadtree {
        let leaf = settings.leaf_cells.min(heights.size).max(2);
        let settings = LodSettings {
            leaf_cells: leaf,
            ..settings
        };
        let pyramid = MinMaxPyramid::build(heights, leaf);
        LodQuadtree {
            settings,
            levels: pyramid.levels.len() as u32,
            pyramid,
            cell: f64::from(heights.cell),
        }
    }

    pub fn settings(&self) -> &LodSettings {
        &self.settings
    }

    /// Number of levels; the root is `levels() - 1`.
    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// Height cell edge in metres.
    pub fn cell(&self) -> f64 {
        self.cell
    }

    /// The distance up to which `level` is drawn; infinite for the root.
    pub fn range(&self, level: u32) -> f64 {
        if level + 1 >= self.levels {
            return f64::INFINITY;
        }
        let leaf = f64::from(self.settings.leaf_cells) * self.cell;
        self.settings.lod0_range_factor * leaf * f64::from(1u32 << level)
    }

    /// `(start, end)` distances of the morph from `level` to `level + 1`; infinite for the root.
    pub fn morph_range(&self, level: u32) -> (f64, f64) {
        let end = self.range(level);
        (end * self.settings.morph_start, end)
    }

    /// The areas to draw for a camera at `camera`, culled by `frustum` (camera-relative).
    pub fn select(&self, camera: DVec3, frustum: Option<&Frustum>, out: &mut Vec<SelectedNode>) {
        out.clear();
        if self.levels > 0 {
            self.select_node(camera, frustum, self.levels - 1, 0, 0, out);
        }
    }

    fn node(&self, level: u32, bx: u32, bz: u32) -> SelectedNode {
        let cells = self.settings.leaf_cells << level;
        let (min_height, max_height) = self.pyramid.get(level, bx, bz);
        SelectedNode {
            level,
            x: bx * cells,
            z: bz * cells,
            cells,
            min_height,
            max_height,
        }
    }

    /// World-space bounds of an area.
    pub fn bounds(&self, node: &SelectedNode) -> (DVec3, DVec3) {
        let c = self.cell;
        (
            DVec3::new(
                f64::from(node.x) * c,
                f64::from(node.min_height),
                f64::from(node.z) * c,
            ),
            DVec3::new(
                f64::from(node.x + node.cells) * c,
                f64::from(node.max_height),
                f64::from(node.z + node.cells) * c,
            ),
        )
    }

    fn visible(&self, camera: DVec3, frustum: Option<&Frustum>, node: &SelectedNode) -> bool {
        frustum.is_none_or(|f| {
            let (min, max) = self.bounds(node);
            f.intersects_aabb(rel(min, camera), rel(max, camera))
        })
    }

    /// Returns `false` when the node is beyond its level's range and its parent must cover it.
    fn select_node(
        &self,
        camera: DVec3,
        frustum: Option<&Frustum>,
        level: u32,
        bx: u32,
        bz: u32,
        out: &mut Vec<SelectedNode>,
    ) -> bool {
        let node = self.node(level, bx, bz);
        let (min, max) = self.bounds(&node);
        if distance_to_box(camera, min, max) > self.range(level) {
            return false;
        }
        if !self.visible(camera, frustum, &node) {
            return true;
        }
        if level == 0 || distance_to_box(camera, min, max) > self.range(level - 1) {
            out.push(node);
            return true;
        }
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (cx, cz) = (2 * bx + dx, 2 * bz + dz);
            if !self.select_node(camera, frustum, level - 1, cx, cz, out) {
                let child = self.node(level - 1, cx, cz);
                if self.visible(camera, frustum, &child) {
                    out.push(SelectedNode { level, ..child });
                }
            }
        }
        true
    }
}

fn rel(p: DVec3, camera: DVec3) -> Vec3 {
    (p - camera).as_vec3()
}

/// Distance from `p` to the closest point of the box.
pub fn distance_to_box(p: DVec3, min: DVec3, max: DVec3) -> f64 {
    (p.clamp(min, max) - p).length()
}

/// Distance from `p` to the farthest point of the box.
pub fn farthest_in_box(p: DVec3, min: DVec3, max: DVec3) -> f64 {
    let d = (p - min).abs().max((max - p).abs());
    d.length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_render::Camera;
    use proptest::prelude::*;

    /// 256 cells of 7.5 m (1920 m) with hills up to ~200 m.
    fn field() -> HeightField {
        let size = 256u32;
        let heights = (0..size * size)
            .map(|k| {
                let (i, j) = ((k % size) as f32, (k / size) as f32);
                100.0 + 80.0 * (i * 0.05).sin() * (j * 0.03).cos() + 20.0 * (i * 0.31).sin()
            })
            .collect();
        HeightField {
            size,
            cell: 7.5,
            heights,
        }
    }

    fn tree() -> LodQuadtree {
        LodQuadtree::new(
            &field(),
            LodSettings {
                leaf_cells: 8,
                ..LodSettings::default()
            },
        )
    }

    fn camera_strategy() -> impl Strategy<Value = DVec3> {
        (-500.0f64..2_400.0, 90.0f64..1_500.0, -500.0f64..2_400.0)
            .prop_map(|(x, y, z)| DVec3::new(x, y, z))
    }

    /// Mark every cell covered by the selection; returns the per-cell level.
    fn coverage(nodes: &[SelectedNode], size: u32) -> Vec<Option<u32>> {
        let mut cells = vec![None; (size * size) as usize];
        for n in nodes {
            for z in n.z..n.z + n.cells {
                for x in n.x..n.x + n.cells {
                    let c = &mut cells[(z * size + x) as usize];
                    assert!(c.is_none(), "cell {x} {z} covered twice");
                    *c = Some(n.level);
                }
            }
        }
        cells
    }

    #[test]
    fn levels_and_ranges_double() {
        let t = tree();
        assert_eq!(t.levels(), 6, "256 / 8 = 32 blocks: 6 levels");
        assert_eq!(t.range(0), 4.0 * 8.0 * 7.5);
        assert_eq!(t.range(2), 4.0 * t.range(0));
        assert_eq!(t.range(5), f64::INFINITY, "the root is never out of range");
        assert_eq!(t.morph_range(1), (0.8 * t.range(1), t.range(1)));
    }

    #[test]
    fn a_far_away_camera_draws_the_root_only() {
        let t = tree();
        let mut out = Vec::new();
        t.select(DVec3::new(1e6, 0.0, 1e6), None, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].level, out[0].cells), (5, 256));
    }

    #[test]
    fn the_camera_cell_gets_full_detail() {
        let t = tree();
        let mut out = Vec::new();
        t.select(DVec3::new(960.0, 160.0, 960.0), None, &mut out);
        let levels = coverage(&out, 256);
        assert_eq!(levels[(128 * 256 + 128) as usize], Some(0));
        assert_eq!(levels[0], Some(3), "a far corner is coarse");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn selection_covers_the_terrain_exactly_once(camera in camera_strategy()) {
            let t = tree();
            let mut out = Vec::new();
            t.select(camera, None, &mut out);
            let cells = coverage(&out, 256);
            prop_assert!(cells.iter().all(Option::is_some));
        }

        #[test]
        fn neighbouring_areas_differ_by_at_most_one_level(camera in camera_strategy()) {
            let t = tree();
            let mut out = Vec::new();
            t.select(camera, None, &mut out);
            let cells = coverage(&out, 256);
            for z in 0..256usize {
                for x in 0..256usize {
                    let here = cells[z * 256 + x].unwrap();
                    for (nx, nz) in [(x + 1, z), (x, z + 1)] {
                        if nx < 256 && nz < 256 {
                            let there = cells[nz * 256 + nx].unwrap();
                            prop_assert!(here.abs_diff(there) <= 1, "{x},{z}: {here} vs {there}");
                        }
                    }
                }
            }
        }

        /// The morph conditions that keep shared edges identical: an area at level `l` lies
        /// wholly before the morph start of level `l + 1` (so a coarser neighbour is unmorphed
        /// along their edge) and wholly beyond the range of level `l - 1` (so a finer
        /// neighbour is fully morphed onto this level's grid along their edge).
        #[test]
        fn morph_zones_line_up_between_levels(camera in camera_strategy()) {
            let t = tree();
            let mut out = Vec::new();
            t.select(camera, None, &mut out);
            for n in &out {
                let (min, max) = t.bounds(n);
                if n.level + 1 < t.levels() {
                    let far = farthest_in_box(camera, min, max);
                    prop_assert!(far <= t.morph_range(n.level + 1).0, "{n:?} reaches {far}");
                }
                if n.level > 0 {
                    let near = distance_to_box(camera, min, max);
                    prop_assert!(near >= t.range(n.level - 1), "{n:?} starts at {near}");
                }
            }
        }
    }

    #[test]
    fn frustum_culling_drops_areas_behind_the_camera() {
        let t = tree();
        let camera = Camera {
            position: DVec3::new(960.0, 300.0, 100.0),
            yaw: 0.0,
            pitch: -0.3,
            ..Camera::default()
        };
        let frustum = Frustum::from_view_projection(camera.view_projection(16.0 / 9.0));
        let (mut all, mut culled) = (Vec::new(), Vec::new());
        t.select(camera.position, None, &mut all);
        t.select(camera.position, Some(&frustum), &mut culled);
        assert!(culled.len() < all.len());
        // Nothing drawn lies wholly south of (behind) the camera.
        for n in &culled {
            let (_, max) = t.bounds(n);
            assert!(max.z > camera.position.z - 1.0, "{n:?}");
        }
        // The area straight ahead is kept.
        assert!(culled.iter().any(|n| {
            let (min, max) = t.bounds(n);
            (min.x..max.x).contains(&960.0) && (min.z..max.z).contains(&400.0)
        }));
    }
}
