//! CPU mesh generation for roads: strips along cubic Bezier centre lines, draped on the terrain.

use glam::{DVec3, Vec2, Vec3};

/// One cubic Bezier piece of a road centre line in world `(x, z)` metres, with the road's
/// width and material. Consecutive pieces of the same `road` continue the texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadSegment {
    /// Identifies the road the piece belongs to; texture `v` carries on between consecutive
    /// pieces of the same road.
    pub road: u32,
    /// Start point.
    pub p0: Vec2,
    /// First control point.
    pub c1: Vec2,
    /// Second control point.
    pub c2: Vec2,
    /// End point.
    pub p1: Vec2,
    /// Road width in metres.
    pub width: f32,
    /// Index of the road material.
    pub material: u32,
    /// The piece starts at an open road end: its first texture length uses the end texture.
    pub open_start: bool,
    /// The piece ends at an open road end.
    pub open_end: bool,
}

impl RoadSegment {
    fn point(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        self.p0 * (u * u * u)
            + self.c1 * (3.0 * u * u * t)
            + self.c2 * (3.0 * u * t * t)
            + self.p1 * (t * t * t)
    }

    fn tangent(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        let d = (self.c1 - self.p0) * (3.0 * u * u)
            + (self.c2 - self.c1) * (6.0 * u * t)
            + (self.p1 - self.c2) * (3.0 * t * t);
        if d.length_squared() > 1e-12 {
            d.normalize()
        } else {
            (self.p1 - self.p0).normalize_or_zero()
        }
    }
}

/// How roads are tessellated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadMeshSettings {
    /// Maximum length of one slice along the road (metres).
    pub step: f32,
    /// Maximum spacing of vertices across the road (metres).
    pub cross_step: f32,
    /// Height above the terrain surface (metres), against z-fighting.
    pub lift: f32,
    /// Edge length of the square chunks the meshes are grouped in (metres).
    pub chunk_size: f64,
}

impl Default for RoadMeshSettings {
    fn default() -> Self {
        Self {
            step: 2.0,
            cross_step: 2.5,
            lift: 0.04,
            chunk_size: 512.0,
        }
    }
}

/// One vertex of a road mesh, relative to its chunk origin.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoadVertex {
    /// Position relative to the chunk origin.
    pub position: [f32; 3],
    /// Terrain normal.
    pub normal: [f32; 3],
    /// `u` across the road (0 left, 1 right), `v` along it in texture lengths.
    pub uv: [f32; 2],
}

/// Triangles of one material and texture (straight or end) in one chunk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoadBatch {
    /// Road material index.
    pub material: u32,
    /// `true` for the end texture (`mainTerTex`), `false` for the straight one.
    pub end: bool,
    /// Vertices.
    pub vertices: Vec<RoadVertex>,
    /// Triangle list indices.
    pub indices: Vec<u32>,
}

/// The road meshes of one chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadChunk {
    /// World position of the chunk's south-west corner (y 0); vertices are relative to it.
    pub origin: DVec3,
    /// Batches by material and texture.
    pub batches: Vec<RoadBatch>,
    /// Centre of the chunk relative to `origin`.
    pub centre: Vec3,
    /// Radius around `centre` that holds all its vertices (metres).
    pub radius: f32,
}

/// Arc-length table of one Bezier piece.
struct ArcTable {
    t: Vec<f32>,
    s: Vec<f32>,
}

impl ArcTable {
    fn new(seg: &RoadSegment) -> Self {
        const N: usize = 64;
        let mut t = Vec::with_capacity(N + 1);
        let mut s = Vec::with_capacity(N + 1);
        let mut last = seg.p0;
        let mut acc = 0.0;
        for i in 0..=N {
            let ti = i as f32 / N as f32;
            let p = seg.point(ti);
            acc += p.distance(last);
            last = p;
            t.push(ti);
            s.push(acc);
        }
        Self { t, s }
    }

    fn length(&self) -> f32 {
        *self.s.last().expect("non-empty")
    }

    /// The parameter at arc length `len`.
    fn t_at(&self, len: f32) -> f32 {
        let i = self
            .s
            .partition_point(|&v| v < len)
            .clamp(1, self.s.len() - 1);
        let (s0, s1) = (self.s[i - 1], self.s[i]);
        let f = if s1 > s0 { (len - s0) / (s1 - s0) } else { 0.0 };
        self.t[i - 1] + (self.t[i] - self.t[i - 1]) * f.clamp(0.0, 1.0)
    }
}

/// Builds chunked road meshes from Bezier pieces, draping them on `height(x, z)`.
///
/// Texture `v` runs in texture lengths, one texture length being the road width (square road
/// textures). Within the first and last texture length of an open road end the end batch is
/// used, with `v = 0` at the road end.
pub fn build_road_meshes(
    segments: &[RoadSegment],
    height: impl Fn(f32, f32) -> f32,
    settings: &RoadMeshSettings,
) -> Vec<RoadChunk> {
    let normal_at = |x: f32, z: f32| {
        let e = 0.5;
        let dx = height(x + e, z) - height(x - e, z);
        let dz = height(x, z + e) - height(x, z - e);
        Vec3::new(-dx, 2.0 * e, -dz).normalize()
    };
    let mut chunks: Vec<RoadChunk> = Vec::new();
    let mut chunk_index = std::collections::HashMap::<(i64, i64), usize>::new();
    let mut v_carry = 0.0f32;
    let mut previous: Option<(u32, Vec2)> = None;

    for seg in segments {
        let width = seg.width.max(0.1);
        let tex_len = width;
        let arc = ArcTable::new(seg);
        let length = arc.length();
        if length <= 1e-4 {
            continue;
        }
        // Texture continues from the previous piece of the same road.
        let v0 = match previous {
            Some((road, end)) if road == seg.road && end.distance(seg.p0) < 0.01 => v_carry,
            _ => 0.0,
        };
        previous = Some((seg.road, seg.p1));
        v_carry = v0 + length / tex_len;

        let mid = seg.point(0.5);
        let key = (
            (f64::from(mid.x) / settings.chunk_size).floor() as i64,
            (f64::from(mid.y) / settings.chunk_size).floor() as i64,
        );
        let ci = *chunk_index.entry(key).or_insert_with(|| {
            chunks.push(RoadChunk {
                origin: DVec3::new(
                    key.0 as f64 * settings.chunk_size,
                    0.0,
                    key.1 as f64 * settings.chunk_size,
                ),
                batches: Vec::new(),
                centre: Vec3::ZERO,
                radius: 0.0,
            });
            chunks.len() - 1
        });
        let origin = chunks[ci].origin;

        // Split the arc into straight and end ranges.
        let start_end = if seg.open_start {
            tex_len.min(length * 0.5)
        } else {
            0.0
        };
        let end_end = if seg.open_end {
            tex_len.min(length * 0.5)
        } else {
            0.0
        };
        let mut ranges: Vec<(f32, f32, bool)> = Vec::new();
        if start_end > 0.0 {
            ranges.push((0.0, start_end, true));
        }
        if length - end_end > start_end {
            ranges.push((start_end, length - end_end, false));
        }
        if end_end > 0.0 {
            ranges.push((length - end_end, length, true));
        }

        let across = ((width / settings.cross_step).ceil() as usize).max(1) + 1;
        for (a, b, is_end) in ranges {
            let chunk = &mut chunks[ci];
            let bi = match chunk
                .batches
                .iter()
                .position(|bt| bt.material == seg.material && bt.end == is_end)
            {
                Some(i) => i,
                None => {
                    chunk.batches.push(RoadBatch {
                        material: seg.material,
                        end: is_end,
                        ..RoadBatch::default()
                    });
                    chunk.batches.len() - 1
                }
            };
            let batch = &mut chunk.batches[bi];
            let slices = (((b - a) / settings.step).ceil() as usize).max(1);
            let base = batch.vertices.len() as u32;
            for k in 0..=slices {
                let len = a + (b - a) * k as f32 / slices as f32;
                let t = arc.t_at(len);
                let centre = seg.point(t);
                let dir = seg.tangent(t);
                let right = Vec2::new(dir.y, -dir.x);
                let v = if is_end {
                    if a == 0.0 && seg.open_start {
                        len / tex_len
                    } else {
                        (length - len) / tex_len
                    }
                } else {
                    v0 + len / tex_len
                };
                for j in 0..across {
                    let u = j as f32 / (across - 1) as f32;
                    let p = centre + right * ((u - 0.5) * width);
                    let y = height(p.x, p.y) + settings.lift;
                    let rel = Vec3::new(
                        (f64::from(p.x) - origin.x) as f32,
                        y,
                        (f64::from(p.y) - origin.z) as f32,
                    );
                    batch.vertices.push(RoadVertex {
                        position: rel.to_array(),
                        normal: normal_at(p.x, p.y).to_array(),
                        uv: [u, v],
                    });
                }
            }
            let across = across as u32;
            for k in 0..slices as u32 {
                for j in 0..across - 1 {
                    let i0 = base + k * across + j;
                    let i1 = i0 + 1;
                    let i2 = i0 + across;
                    let i3 = i2 + 1;
                    batch.indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
                }
            }
        }
    }

    for chunk in &mut chunks {
        let centre = Vec3::new(
            (settings.chunk_size * 0.5) as f32,
            0.0,
            (settings.chunk_size * 0.5) as f32,
        );
        chunk.centre = centre;
        chunk.radius = chunk
            .batches
            .iter()
            .flat_map(|b| b.vertices.iter())
            .map(|v| (Vec3::from_array(v.position) - centre).length())
            .fold(0.0, f32::max);
    }
    chunks
}
