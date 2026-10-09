//! Building paths: the walkable graph inside a house, from a model's Paths LOD.
//!
//! Indoor movement is not a terrain problem. The engine's buildings expose `IPaths` — path
//! positions with the actions possible there (`PathActionLadderTop`/`PathActionLadderBottom`
//! is how a path changes floor) — and the model's Paths LOD (resolution `4e15`, "AI paths
//! through buildings") is its floor plan (`docs/re/navigation.md` §5). [`PathMesh`] is that
//! graph: the LOD's triangles, adjacency by shared edge, an A* over the triangles and a
//! portal pull at the shared edges.
//!
//! A building's mesh is per model and shared; the link from the terrain grid to it is
//! geometric — an outdoor path ends at the nearest path position (the door), and
//! [`PathMesh::find_path`] continues from there.

use a3_p3d::{Lod, Model};
use glam::Vec3;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// No neighbour across this edge.
const NONE: u32 = u32::MAX;

/// A walkable graph of triangles inside one building model.
#[derive(Debug, Clone, PartialEq)]
pub struct PathMesh {
    positions: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    /// Per triangle, the triangle across each edge (`edge i` runs from corner `i` to corner
    /// `i + 1`), or [`NONE`].
    adjacency: Vec<[u32; 3]>,
    centroids: Vec<Vec3>,
}

impl PathMesh {
    /// A mesh from raw triangle indices into `positions`; degenerate triangles (a repeated
    /// corner) are dropped.
    pub fn from_triangles(positions: Vec<Vec3>, triangles: Vec<[u32; 3]>) -> Self {
        let triangles: Vec<[u32; 3]> = triangles
            .into_iter()
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .filter(|t| t.iter().all(|&i| (i as usize) < positions.len()))
            .collect();
        let mut adjacency = vec![[NONE; 3]; triangles.len()];
        let mut edges: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        for (t, triangle) in triangles.iter().enumerate() {
            for edge in 0..3 {
                let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
                let key = (a.min(b), a.max(b));
                if let Some(&(other, other_edge)) = edges.get(&key) {
                    adjacency[t][edge] = other;
                    adjacency[other as usize][other_edge as usize] = t as u32;
                } else {
                    edges.insert(key, (t as u32, edge as u32));
                }
            }
        }
        let centroids = triangles
            .iter()
            .map(|t| {
                (positions[t[0] as usize] + positions[t[1] as usize] + positions[t[2] as usize])
                    / 3.0
            })
            .collect();
        Self {
            positions,
            triangles,
            adjacency,
            centroids,
        }
    }

    /// The mesh of one LOD (its faces split into triangles), or `None` when it has no usable
    /// triangle.
    pub fn from_lod(lod: &Lod) -> Option<Self> {
        let triangles: Vec<[u32; 3]> = lod.faces.iter().flat_map(a3_p3d::Face::triangles).collect();
        let mesh = Self::from_triangles(lod.vertices.positions.clone(), triangles);
        (!mesh.triangles.is_empty()).then_some(mesh)
    }

    /// A model's path mesh: its Paths LOD (`4e15`), or its Roadway LOD (`3e15`) when the Paths
    /// LOD has no triangles — a building whose walkable surface is its floor mesh alone.
    pub fn from_model(model: &Model) -> Option<Self> {
        let by_kind = |kind: a3_p3d::LodKind| {
            model
                .lods
                .iter()
                .find(|lod| lod.resolution.kind() == kind)
                .and_then(Self::from_lod)
        };
        by_kind(a3_p3d::LodKind::Paths).or_else(|| by_kind(a3_p3d::LodKind::Roadway))
    }

    /// No triangles: the mesh cannot route anything.
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// The path positions (the LOD's vertices).
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }

    /// The triangles as indices into [`PathMesh::positions`].
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// The path position nearest to `p` — how an outdoor path finds the building's door.
    pub fn nearest_position(&self, p: Vec3) -> Option<usize> {
        self.positions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (**a - p)
                    .length_squared()
                    .total_cmp(&(**b - p).length_squared())
            })
            .map(|(i, _)| i)
    }

    /// A path from `from` to `to`, both snapped to the nearest path positions, with `from` and
    /// `to` themselves as the ends. `None` when the mesh does not connect them.
    pub fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let start = self.nearest_position(from)?;
        let end = self.nearest_position(to)?;
        let mut points = self.find_path_between(start, end)?;
        if (points[0] - from).length() > 1e-4 {
            points.insert(0, from);
        }
        if (points[points.len() - 1] - to).length() > 1e-4 {
            points.push(to);
        }
        Some(points)
    }

    /// A path between two path positions, portal midpoints in between. `None` when they are
    /// not connected.
    pub fn find_path_between(&self, from: usize, to: usize) -> Option<Vec<Vec3>> {
        let start = self.triangle_of(from)?;
        let goal = self.triangle_of(to)?;
        if start == goal {
            return Some(vec![self.positions[from], self.positions[to]]);
        }
        let chain = self.triangle_path(start, goal)?;
        let mut points = vec![self.positions[from]];
        for pair in chain.windows(2) {
            points.push(self.portal(pair[0], pair[1])?);
        }
        points.push(self.positions[to]);
        Some(points)
    }

    /// The first triangle that has `vertex` as a corner.
    fn triangle_of(&self, vertex: usize) -> Option<u32> {
        let vertex = vertex as u32;
        self.triangles
            .iter()
            .position(|t| t.contains(&vertex))
            .map(|t| t as u32)
    }

    /// The midpoint of the edge triangles `a` and `b` share.
    fn portal(&self, a: u32, b: u32) -> Option<Vec3> {
        let (ta, tb) = (self.triangles[a as usize], self.triangles[b as usize]);
        for edge in 0..3 {
            let (p, q) = (ta[edge], ta[(edge + 1) % 3]);
            if tb.contains(&p) && tb.contains(&q) {
                return Some((self.positions[p as usize] + self.positions[q as usize]) / 2.0);
            }
        }
        None
    }

    /// A* over triangles, cheapest first, both ends included.
    fn triangle_path(&self, from: u32, to: u32) -> Option<Vec<u32>> {
        let mut g = vec![u32::MAX; self.triangles.len()];
        let mut parent = vec![NONE; self.triangles.len()];
        let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        g[from as usize] = 0;
        open.push(Reverse((self.triangle_heuristic(from, to), from)));
        while let Some(Reverse((_, index))) = open.pop() {
            if index == to {
                let mut chain = vec![to];
                let mut cur = to;
                while parent[cur as usize] != NONE {
                    cur = parent[cur as usize];
                    chain.push(cur);
                }
                chain.reverse();
                return Some(chain);
            }
            for edge in 0..3 {
                let next = self.adjacency[index as usize][edge];
                if next == NONE {
                    continue;
                }
                let step = ((self.centroids[index as usize] - self.centroids[next as usize])
                    .length()
                    * 100.0) as u32;
                let tentative = g[index as usize].saturating_add(step.max(1));
                if tentative < g[next as usize] {
                    g[next as usize] = tentative;
                    parent[next as usize] = index;
                    open.push(Reverse((
                        tentative + self.triangle_heuristic(next, to),
                        next,
                    )));
                }
            }
        }
        None
    }

    /// Centimetre distance between two triangles' centroids: admissible, since a step costs at
    /// least the centroid distance.
    fn triangle_heuristic(&self, from: u32, to: u32) -> u32 {
        ((self.centroids[from as usize] - self.centroids[to as usize]).length() * 100.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{Encoding, Face, LodResolution, ModelInfo, Vertices};

    /// A flat square floor of 10 x 10 m in the x/z plane as two triangles, from the four
    /// corners `(0,0)`, `(10,0)`, `(10,10)`, `(0,10)`.
    fn square() -> PathMesh {
        PathMesh::from_triangles(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(0.0, 0.0, 10.0),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    #[test]
    fn two_triangles_sharing_an_edge_are_one_mesh() {
        let mesh = square();
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(mesh.positions().len(), 4);
        // The shared edge is corner 0 - corner 2: edge 2 of the first triangle, edge 0 of the
        // second.
        assert_eq!(mesh.adjacency[0][2], 1, "{:?}", mesh.adjacency);
        assert_eq!(mesh.adjacency[1][0], 0, "{:?}", mesh.adjacency);
    }

    #[test]
    fn degenerate_triangles_are_dropped() {
        let mesh = PathMesh::from_triangles(
            vec![Vec3::ZERO, Vec3::X, Vec3::Z],
            vec![[0, 0, 1], [0, 1, 1], [0, 1, 2]],
        );
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn a_path_crosses_the_shared_edge_at_its_midpoint() {
        let mesh = square();
        let path = mesh.find_path_between(1, 3).expect("connected");
        assert_eq!(
            path,
            vec![
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(5.0, 0.0, 5.0),
                Vec3::new(0.0, 0.0, 10.0),
            ]
        );
    }

    #[test]
    fn a_path_inside_one_triangle_is_a_straight_line() {
        let mesh = square();
        let path = mesh.find_path_between(0, 1).expect("connected");
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], Vec3::ZERO);
        assert_eq!(path[1], Vec3::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn separate_islands_are_not_connected() {
        let mesh = PathMesh::from_triangles(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(50.0, 0.0, 0.0),
                Vec3::new(51.0, 0.0, 0.0),
                Vec3::new(50.0, 0.0, 1.0),
            ],
            vec![[0, 1, 2], [3, 4, 5]],
        );
        assert!(mesh.find_path_between(1, 4).is_none());
    }

    #[test]
    fn a_query_position_is_snapped_to_the_nearest_path_position() {
        let mesh = square();
        let path = mesh
            .find_path(Vec3::new(9.5, 0.0, 0.2), Vec3::new(0.4, 0.0, 9.7))
            .expect("connected");
        assert_eq!(path.first(), Some(&Vec3::new(9.5, 0.0, 0.2)));
        assert_eq!(path.last(), Some(&Vec3::new(0.4, 0.0, 9.7)));
        assert!(path.contains(&Vec3::new(5.0, 0.0, 5.0)));
    }

    #[test]
    fn a_mesh_takes_the_paths_lod_and_falls_back_to_the_roadway() {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 4.0),
        ];
        let lod = |resolution: f32, faces: Vec<Face>| Lod {
            resolution: LodResolution(resolution),
            vertices: Vertices {
                positions: positions.clone(),
                ..Default::default()
            },
            faces,
            ..Default::default()
        };
        let model = |lods: Vec<Lod>| Model {
            encoding: Encoding::Mlod,
            version: 257,
            info: ModelInfo::default(),
            skeleton: None,
            animations: Vec::new(),
            lods,
        };

        let paths = model(vec![lod(4e15, vec![Face::triangle(0, 1, 2)])]);
        let mesh = PathMesh::from_model(&paths).expect("the Paths LOD");
        assert_eq!(mesh.triangle_count(), 1);

        // A Paths LOD with points but no faces falls back to the Roadway floor.
        let fallback = model(vec![
            lod(4e15, vec![]),
            lod(3e15, vec![Face::triangle(0, 1, 2)]),
        ]);
        let mesh = PathMesh::from_model(&fallback).expect("the Roadway LOD");
        assert_eq!(mesh.triangle_count(), 1);

        let neither = model(vec![lod(1.0, vec![Face::triangle(0, 1, 2)])]);
        assert!(PathMesh::from_model(&neither).is_none());
    }

    #[test]
    fn an_empty_mesh_routes_nothing() {
        let mesh = PathMesh::from_triangles(vec![Vec3::ZERO], vec![]);
        assert!(mesh.is_empty());
        assert!(mesh.find_path(Vec3::ZERO, Vec3::ZERO).is_none());
    }
}
