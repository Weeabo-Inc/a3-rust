//! The road graph: shapefile roads joined at their ends, with the engine's smoothing curves and
//! spatial queries (nearest road point, `isOnRoad`, `roadsConnectedTo`).

use std::collections::HashMap;

use glam::Vec2;

use crate::roads::RoadNetwork;

/// Road ends closer than this (metres) meet (the engine compares squared distances with 0.01).
pub const JOIN_DISTANCE: f32 = 0.1;

/// A road continues into a neighbour at a shared end when the dot product of their directions
/// away from that end is below this (the neighbour leaves at more than 60 degrees from the
/// road's own direction, i.e. carries it on).
pub const CONTINUE_DOT: f32 = 0.5;

/// Spatial index cell size in metres.
const CELL: f32 = 64.0;

/// One end of a road.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoadEnd {
    /// The first point.
    Start,
    /// The last point.
    End,
}

/// A node of the graph: one or more road ends at the same place.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadNode {
    /// Position (the first end found there).
    pub position: Vec2,
    /// The road ends that meet here.
    pub ends: Vec<(usize, RoadEnd)>,
}

impl RoadNode {
    /// A dead end: one road end only.
    pub fn is_dead_end(&self) -> bool {
        self.ends.len() == 1
    }

    /// A junction: three or more road ends.
    pub fn is_junction(&self) -> bool {
        self.ends.len() >= 3
    }
}

/// One cubic Bezier piece of a road's smoothed centre line, as the engine builds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSegment {
    /// The road.
    pub road: usize,
    /// Index of the polyline segment (`points[index]` to `points[index + 1]`).
    pub index: usize,
    /// Start point.
    pub p0: Vec2,
    /// First control point.
    pub c1: Vec2,
    /// Second control point.
    pub c2: Vec2,
    /// End point.
    pub p1: Vec2,
    /// The segment starts at an open road end (no continuing road): the engine draws the end
    /// texture there.
    pub open_start: bool,
    /// The segment ends at an open road end.
    pub open_end: bool,
}

impl CurveSegment {
    /// The point at parameter `t` in `[0, 1]`.
    pub fn point(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        self.p0 * (u * u * u)
            + self.c1 * (3.0 * u * u * t)
            + self.c2 * (3.0 * u * t * t)
            + self.p1 * (t * t * t)
    }

    /// The (unnormalised) tangent at `t`.
    pub fn tangent(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        (self.c1 - self.p0) * (3.0 * u * u)
            + (self.c2 - self.c1) * (6.0 * u * t)
            + (self.p1 - self.c2) * (3.0 * t * t)
    }
}

/// The closest point of a road centre line to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadPoint {
    /// The road.
    pub road: usize,
    /// The polyline segment index.
    pub segment: usize,
    /// Position along the segment, 0 to 1.
    pub t: f32,
    /// The closest point.
    pub point: Vec2,
    /// Distance from the query point in metres.
    pub distance: f32,
}

/// A road end that meets the side of another road (a T junction): the end lies on the other
/// road's surface but not at one of its ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attachment {
    /// The road whose end attaches.
    pub road: usize,
    /// Which end.
    pub end: RoadEnd,
    /// Where on the other road's centre line it attaches.
    pub onto: RoadPoint,
}

/// The road network joined into a graph, with a spatial index.
#[derive(Debug, Clone)]
pub struct RoadGraph {
    /// Nodes (road ends, joined when closer than [`JOIN_DISTANCE`]).
    pub nodes: Vec<RoadNode>,
    /// Per road: the node of its start and of its end.
    pub road_nodes: Vec<[usize; 2]>,
    /// Per road: the continuing neighbour at its start and at its end, chosen as the engine
    /// does (the most opposite direction, if below [`CONTINUE_DOT`]).
    pub continuations: Vec<[Option<(usize, RoadEnd)>; 2]>,
    /// Road ends meeting the side of another road (T junctions), within that road's half
    /// width plus [`JOIN_DISTANCE`].
    pub attachments: Vec<Attachment>,
    /// Per road: half its RoadsLib width.
    half_widths: Vec<f32>,
    /// Polyline segments per index cell.
    cells: HashMap<(i32, i32), Vec<(u32, u32)>>,
    /// Points of each road (copied from the network).
    points: Vec<Vec<Vec2>>,
}

fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

/// Direction leaving the road at `end` (from the end point into the road).
fn direction_into(points: &[Vec2], end: RoadEnd) -> Vec2 {
    let (a, b) = match end {
        RoadEnd::Start => (points[0], points[1]),
        RoadEnd::End => (points[points.len() - 1], points[points.len() - 2]),
    };
    (b - a).normalize_or_zero()
}

fn end_point(points: &[Vec2], end: RoadEnd) -> Vec2 {
    match end {
        RoadEnd::Start => points[0],
        RoadEnd::End => points[points.len() - 1],
    }
}

impl RoadGraph {
    /// Joins the roads of `network`.
    pub fn new(network: &RoadNetwork) -> Self {
        let points: Vec<Vec<Vec2>> = network.roads.iter().map(|r| r.points.clone()).collect();
        let half_widths = network
            .roads
            .iter()
            .map(|r| network.road_type(r).width * 0.5)
            .collect();

        // Nodes: cluster ends within JOIN_DISTANCE through a coarse grid.
        let mut nodes: Vec<RoadNode> = Vec::new();
        let mut node_grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut road_nodes = Vec::with_capacity(points.len());
        for (road, pts) in points.iter().enumerate() {
            let mut pair = [0; 2];
            for (slot, end) in [RoadEnd::Start, RoadEnd::End].into_iter().enumerate() {
                let p = end_point(pts, end);
                let key = (
                    (p.x / JOIN_DISTANCE).floor() as i32,
                    (p.y / JOIN_DISTANCE).floor() as i32,
                );
                let mut found = None;
                'search: for dx in -1..=1 {
                    for dz in -1..=1 {
                        for &n in node_grid
                            .get(&(key.0 + dx, key.1 + dz))
                            .into_iter()
                            .flatten()
                        {
                            if nodes[n].position.distance_squared(p) < JOIN_DISTANCE * JOIN_DISTANCE
                            {
                                found = Some(n);
                                break 'search;
                            }
                        }
                    }
                }
                let n = found.unwrap_or_else(|| {
                    nodes.push(RoadNode {
                        position: p,
                        ends: Vec::new(),
                    });
                    node_grid.entry(key).or_default().push(nodes.len() - 1);
                    nodes.len() - 1
                });
                nodes[n].ends.push((road, end));
                pair[slot] = n;
            }
            road_nodes.push(pair);
        }

        // Continuations: at each end, the other road end with the smallest direction dot.
        let continuations = (0..points.len())
            .map(|road| {
                [RoadEnd::Start, RoadEnd::End].map(|end| {
                    let node = &nodes[road_nodes[road][(end == RoadEnd::End) as usize]];
                    let dir = direction_into(&points[road], end);
                    node.ends
                        .iter()
                        .filter(|&&(r, e)| (r, e) != (road, end))
                        .map(|&(r, e)| (dir.dot(direction_into(&points[r], e)), (r, e)))
                        .filter(|(dot, _)| *dot < CONTINUE_DOT)
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, re)| re)
                })
            })
            .collect();

        let mut cells: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        for (road, pts) in points.iter().enumerate() {
            for (i, w) in pts.windows(2).enumerate() {
                let (a, b) = (cell_of(w[0].min(w[1])), cell_of(w[0].max(w[1])));
                for x in a.0..=b.0 {
                    for z in a.1..=b.1 {
                        cells
                            .entry((x, z))
                            .or_default()
                            .push((road as u32, i as u32));
                    }
                }
            }
        }

        let mut graph = Self {
            nodes,
            road_nodes,
            continuations,
            attachments: Vec::new(),
            half_widths,
            cells,
            points,
        };
        graph.attachments = graph.find_attachments();
        graph
    }

    fn find_attachments(&self) -> Vec<Attachment> {
        let reach = self.half_widths.iter().copied().fold(0.0, f32::max) + JOIN_DISTANCE;
        let mut out = Vec::new();
        for road in 0..self.points.len() {
            for (slot, end) in [RoadEnd::Start, RoadEnd::End].into_iter().enumerate() {
                let node = self.road_nodes[road][slot];
                let p = end_point(&self.points[road], end);
                let mut best: Option<RoadPoint> = None;
                self.for_segments_near(p, reach, |other, segment, a, b| {
                    if other == road || self.road_nodes[other].contains(&node) {
                        return;
                    }
                    let (t, point) = closest_on_segment(p, a, b);
                    let distance = point.distance(p);
                    if distance <= self.half_widths[other] + JOIN_DISTANCE
                        && best.is_none_or(|b| distance < b.distance)
                    {
                        best = Some(RoadPoint {
                            road: other,
                            segment,
                            t,
                            point,
                            distance,
                        });
                    }
                });
                if let Some(onto) = best {
                    out.push(Attachment { road, end, onto });
                }
            }
        }
        out
    }

    /// The roads connected to `road` (SQF `roadsConnectedTo`), without `road`: roads sharing
    /// an end node with it, roads whose ends attach to its side and roads its ends attach to.
    pub fn connected_to(&self, road: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.road_nodes[road]
            .iter()
            .flat_map(|&n| self.nodes[n].ends.iter().map(|&(r, _)| r))
            .chain(self.attachments.iter().filter_map(|a| {
                if a.road == road {
                    Some(a.onto.road)
                } else if a.onto.road == road {
                    Some(a.road)
                } else {
                    None
                }
            }))
            .filter(|&r| r != road)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The closest road centre-line point within `max_distance` of `p`.
    pub fn nearest(&self, p: Vec2, max_distance: f32) -> Option<RoadPoint> {
        let mut best: Option<RoadPoint> = None;
        self.for_segments_near(p, max_distance, |road, segment, a, b| {
            let (t, point) = closest_on_segment(p, a, b);
            let distance = point.distance(p);
            if distance <= max_distance && best.is_none_or(|b| distance < b.distance) {
                best = Some(RoadPoint {
                    road,
                    segment,
                    t,
                    point,
                    distance,
                });
            }
        });
        best
    }

    /// Whether `p` lies on a road surface: within half the road type's width of some road's
    /// centre line (SQF `isOnRoad`, approximated on the polyline).
    pub fn is_on_road(&self, p: Vec2) -> bool {
        self.road_at(p).is_some()
    }

    /// The road whose surface covers `p` (SQF `roadAt`), the closest one when several do.
    pub fn road_at(&self, p: Vec2) -> Option<RoadPoint> {
        let reach = self.half_widths.iter().copied().fold(0.0, f32::max);
        let mut best: Option<RoadPoint> = None;
        self.for_segments_near(p, reach, |road, segment, a, b| {
            let (t, point) = closest_on_segment(p, a, b);
            let distance = point.distance(p);
            if distance <= self.half_widths[road] && best.is_none_or(|b| distance < b.distance) {
                best = Some(RoadPoint {
                    road,
                    segment,
                    t,
                    point,
                    distance,
                });
            }
        });
        best
    }

    /// Roads with some centre-line point within `radius` of `p` (SQF `nearRoads`), sorted.
    pub fn roads_near(&self, p: Vec2, radius: f32) -> Vec<usize> {
        let mut out = Vec::new();
        self.for_segments_near(p, radius, |road, _, a, b| {
            if closest_on_segment(p, a, b).1.distance(p) <= radius {
                out.push(road);
            }
        });
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The smoothed centre line of `road`: one cubic Bezier per polyline segment, built from
    /// centripetal Catmull-Rom tangents with the continuing neighbours supplying the points
    /// beyond the ends (engine loader `0x14162c920`).
    pub fn curve(&self, road: usize) -> Vec<CurveSegment> {
        let pts = &self.points[road];
        let n = pts.len();
        let beyond = |end: RoadEnd| -> Option<Vec2> {
            let (r, e) = self.continuations[road][(end == RoadEnd::End) as usize]?;
            let other = &self.points[r];
            Some(match e {
                RoadEnd::Start => other[1],
                RoadEnd::End => other[other.len() - 2],
            })
        };
        let before = beyond(RoadEnd::Start);
        let after = beyond(RoadEnd::End);
        (0..n - 1)
            .map(|i| {
                let p1 = pts[i];
                let p2 = pts[i + 1];
                let p0 = if i == 0 {
                    before.unwrap_or(p1)
                } else {
                    pts[i - 1]
                };
                let p3 = if i + 2 < n {
                    pts[i + 2]
                } else {
                    after.unwrap_or(p2)
                };
                let (c1, c2) = catmull_rom_controls(p0, p1, p2, p3);
                CurveSegment {
                    road,
                    index: i,
                    p0: p1,
                    c1,
                    c2,
                    p1: p2,
                    open_start: i == 0 && before.is_none(),
                    open_end: i + 2 >= n && after.is_none(),
                }
            })
            .collect()
    }

    fn for_segments_near(&self, p: Vec2, radius: f32, mut f: impl FnMut(usize, usize, Vec2, Vec2)) {
        let lo = cell_of(p - Vec2::splat(radius));
        let hi = cell_of(p + Vec2::splat(radius));
        let mut seen = std::collections::HashSet::new();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                for &(road, seg) in self.cells.get(&(x, z)).into_iter().flatten() {
                    if seen.insert((road, seg)) {
                        let pts = &self.points[road as usize];
                        f(
                            road as usize,
                            seg as usize,
                            pts[seg as usize],
                            pts[seg as usize + 1],
                        );
                    }
                }
            }
        }
    }
}

/// The parameter and point of segment `a`-`b` closest to `p`.
fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> (f32, Vec2) {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (t, a + ab * t)
}

/// Bezier control points of the centripetal Catmull-Rom spline through `p1`-`p2` (neighbours
/// `p0`, `p3`), as the engine computes them; a neighbour equal to its end point gives a
/// straight start or end tangent.
pub fn catmull_rom_controls(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> (Vec2, Vec2) {
    let d1 = p0.distance(p1).sqrt();
    let d2 = p1.distance(p2).sqrt();
    let d3 = p2.distance(p3).sqrt();
    let (d1s, d2s, d3s) = (d1 * d1, d2 * d2, d3 * d3);
    let c1 = if d1 > 0.0 {
        (p2 * d1s - p0 * d2s + p1 * (2.0 * d1s + 3.0 * d1 * d2 + d2s)) / (3.0 * d1 * (d1 + d2))
    } else {
        p1
    };
    let c2 = if d3 > 0.0 {
        (p1 * d3s - p3 * d2s + p2 * (2.0 * d3s + 3.0 * d3 * d2 + d2s)) / (3.0 * d3 * (d3 + d2))
    } else {
        p2
    };
    (c1, c2)
}
