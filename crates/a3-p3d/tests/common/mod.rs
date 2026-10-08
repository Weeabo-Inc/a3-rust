//! Builders for synthetic P3D fixtures.
#![allow(dead_code)]

/// A little-endian byte builder.
#[derive(Default, Clone)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.raw(&[v])
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn floats(&mut self, v: &[f32]) -> &mut Self {
        for f in v {
            self.f32(*f);
        }
        self
    }
    pub fn asciiz(&mut self, s: &str) -> &mut Self {
        self.raw(s.as_bytes()).u8(0)
    }
    pub fn fixed(&mut self, s: &str, size: usize) -> &mut Self {
        let mut b = s.as_bytes().to_vec();
        b.resize(size, 0);
        self.raw(&b)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn put_u32_at(&mut self, at: usize, v: u32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// One face corner: point index, normal index, u, v.
pub type Corner = (u32, u32, f32, f32);

pub struct MlodFace {
    pub corners: Vec<Corner>,
    pub flags: u32,
    pub texture: &'static str,
    pub material: &'static str,
}

pub struct MlodLod {
    pub resolution: f32,
    pub points: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub faces: Vec<MlodFace>,
    /// `(name, data)` tags, written before `#EndOfFile#`.
    pub tags: Vec<(&'static str, Vec<u8>)>,
}

/// One ODOL animation class as written in the header.
pub struct OdolAnim {
    pub kind: u32,
    pub name: &'static str,
    pub source: &'static str,
    /// Type-specific floats (2 for rotation/translation/hide, 8 for direct).
    pub params: Vec<f32>,
}

/// `(name, inherited, [(bone, parent)])`.
pub type SkeletonSpec = (&'static str, bool, Vec<(&'static str, &'static str)>);

/// An ODOL v73 file. LOD bodies are opaque byte blobs placed at the offsets of the LOD table.
pub struct Odol {
    pub resolutions: Vec<f32>,
    pub permanent: Vec<bool>,
    /// `(name, inherited, [(bone, parent)])`.
    pub skeleton: Option<SkeletonSpec>,
    pub animations: Vec<OdolAnim>,
    /// Per LOD: per bone, animation indices.
    pub bones_to_anims: Vec<Vec<Vec<u32>>>,
    /// Per LOD: per animation, bone index (or -1) and axis.
    pub anims_to_bones: Vec<Vec<(i32, [f32; 6])>>,
    pub lod_bodies: Vec<Vec<u8>>,
}

impl Odol {
    pub fn new(resolutions: &[f32]) -> Self {
        Self {
            resolutions: resolutions.to_vec(),
            permanent: vec![true; resolutions.len()],
            skeleton: None,
            animations: vec![],
            bones_to_anims: vec![],
            anims_to_bones: vec![],
            lod_bodies: vec![empty_lod_body(); resolutions.len()],
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let n = self.resolutions.len();
        let mut b = Bytes::new();
        b.raw(b"ODOL").u32(73).u32(107410).asciiz(r"\a3\mf\muzzle");
        b.u32(n as u32).floats(&self.resolutions);
        // ModelInfo
        b.u32(0x100) // special flags
            .f32(2.5) // bounding sphere
            .f32(2.0) // geometry sphere
            .u32(0)
            .u32(0)
            .u32(0)
            .floats(&[0.0, 0.5, 0.0]) // aiming center
            .u32(0xff00_00ff)
            .u32(0)
            .f32(1.0)
            .floats(&[-1.0, -2.0, -3.0]) // bbox min
            .floats(&[1.0, 2.0, 3.0]) // bbox max
            .f32(1.0)
            .f32(1.0)
            .floats(&[-1.0, -1.0, -1.0]) // visual bbox
            .floats(&[1.0, 1.0, 1.0])
            .floats(&[0.0; 3])
            .floats(&[0.0; 3])
            .floats(&[0.0, 0.25, 0.0]) // center of mass
            .floats(&[1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0]) // inv inertia
            .raw(&[1, 0, 1, 1, 0]) // autocenter .. aicovers
            .floats(&[0.0; 6]) // thermal
            .u8(0)
            .i32(0)
            .u8(0)
            .f32(f32::MAX)
            .u8(u8::from(self.skeleton.is_some()));
        match &self.skeleton {
            None => {
                b.u8(0);
            }
            Some((name, inherited, bones)) => {
                b.asciiz(name)
                    .u8(u8::from(*inherited))
                    .u32(bones.len() as u32);
                for (bone, parent) in bones {
                    b.asciiz(bone).asciiz(parent);
                }
                b.asciiz("");
            }
        }
        b.u8(3) // map type
            .u32(0) // mass array
            .f32(250.0)
            .f32(1.0 / 250.0)
            .f32(40.0)
            .f32(1.0 / 40.0)
            .f32(1.0);
        let mut special = [-1i8; 14];
        special[0] = (n - 1) as i8; // memory: last LOD
        special[1] = 0; // geometry
        for s in special {
            b.u8(s as u8);
        }
        b.u32(n as u32)
            .u8(0)
            .asciiz("house")
            .asciiz("building")
            .u8(0)
            .u32(0);
        for _ in 0..3 {
            for _ in 0..n {
                b.i32(-1);
            }
        }
        // Animations
        b.u8(u8::from(!self.animations.is_empty()));
        if !self.animations.is_empty() {
            b.u32(self.animations.len() as u32);
            for a in &self.animations {
                b.u32(a.kind)
                    .asciiz(a.name)
                    .asciiz(a.source)
                    .floats(&[0.0, 1.0, 0.0, 1.0, 0.0, 0.0])
                    .u32(0)
                    .floats(&a.params);
            }
            // No tables at all when nothing binds to a bone.
            b.u32(if self.bones_to_anims.is_empty() {
                0
            } else {
                n as u32
            });
            for lod in &self.bones_to_anims {
                b.u32(lod.len() as u32);
                for bone in lod {
                    b.u32(bone.len() as u32);
                    for a in bone {
                        b.u32(*a);
                    }
                }
            }
            for lod in &self.anims_to_bones {
                for (anim, (bone, axis)) in self.animations.iter().zip(lod) {
                    b.i32(*bone);
                    if *bone >= 0 && anim.kind < 8 {
                        b.floats(axis);
                    }
                }
            }
        }
        // LOD table, permanent flags, summaries, then bodies.
        let table = b.len();
        for _ in 0..2 * n {
            b.u32(0);
        }
        for p in &self.permanent {
            b.u8(u8::from(*p));
        }
        for p in &self.permanent {
            if !p {
                b.u32(12)
                    .u32(0xffff_ffff)
                    .u32(0)
                    .u32(0)
                    .u8(0)
                    .u32(24)
                    .f32(1.5);
            }
        }
        for (i, body) in self.lod_bodies.iter().enumerate() {
            let start = b.len() as u32;
            b.raw(body);
            let end = b.len() as u32;
            b.put_u32_at(table + 4 * i, start);
            b.put_u32_at(table + 4 * (n + i), end);
        }
        b.0
    }
}

pub fn mlod(lods: &[MlodLod]) -> Vec<u8> {
    let mut b = Bytes::new();
    b.raw(b"MLOD").u32(257).u32(lods.len() as u32);
    for lod in lods {
        b.raw(b"P3DM")
            .u32(0x1c)
            .u32(0x100)
            .u32(lod.points.len() as u32)
            .u32(lod.normals.len() as u32)
            .u32(lod.faces.len() as u32)
            .u32(0);
        for p in &lod.points {
            b.floats(p).u32(0);
        }
        for n in &lod.normals {
            b.floats(n);
        }
        for face in &lod.faces {
            b.u32(face.corners.len() as u32);
            for i in 0..4 {
                let (p, n, u, v) = face.corners.get(i).copied().unwrap_or((0, 0, 0.0, 0.0));
                b.u32(p).u32(n).f32(u).f32(v);
            }
            b.u32(face.flags).asciiz(face.texture).asciiz(face.material);
        }
        b.raw(b"TAGG");
        for (name, data) in &lod.tags {
            b.u8(1).asciiz(name).u32(data.len() as u32).raw(data);
        }
        b.u8(1).asciiz("#EndOfFile#").u32(0);
        b.f32(lod.resolution);
    }
    b.0
}

/// An LZO1X stream holding `data` as one literal run (valid for up to 238 bytes).
pub fn lzo_literal(data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= 238);
    let mut out = vec![17 + data.len() as u8];
    out.extend_from_slice(data);
    out.extend_from_slice(&[0x11, 0, 0]);
    out
}

impl Bytes {
    /// A compressed array stored raw (flag 0).
    pub fn carray(&mut self, count: usize, data: &[u8]) -> &mut Self {
        self.u32(count as u32);
        if !data.is_empty() {
            self.u8(0).raw(data);
        }
        self
    }
    /// A compressed array stored as LZO1X (flag 2).
    pub fn carray_lzo(&mut self, count: usize, data: &[u8]) -> &mut Self {
        self.u32(count as u32).u8(2).raw(&lzo_literal(data))
    }
    /// A default-fill array with explicit data.
    pub fn fill(&mut self, count: usize, data: &[u8]) -> &mut Self {
        self.u32(count as u32).u8(0);
        if !data.is_empty() {
            self.u8(0).raw(data);
        }
        self
    }
    /// A default-fill array of `count` copies of `value`.
    pub fn fill_value(&mut self, count: usize, value: &[u8]) -> &mut Self {
        self.u32(count as u32).u8(1).raw(value)
    }
}

pub fn le_u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn le_f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A visual ODOL LOD body: a unit quad and a triangle, two sections, one selection, a proxy and
/// a material. Positions are LZO-compressed; everything else is raw.
pub fn sample_lod_body() -> Vec<u8> {
    let positions: [[f32; 3]; 5] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [2.0, 0.0, 0.0],
    ];
    let mut b = Bytes::new();
    // Proxy: identity orientation at (0.5, 0.5, 0), selection 1.
    b.u32(1)
        .asciiz(r"\a3\proxies\seat")
        .floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
        .floats(&[0.5, 0.5, 0.0])
        .i32(1)
        .i32(1)
        .i32(-1)
        .i32(-1);
    b.u32(2).u32(0).u32(1); // sub-skeleton: LOD bones 0, 1 -> skeleton bones 0, 1
    b.u32(2).u32(1).u32(0).u32(1).u32(1); // skeleton -> sub-skeleton
    b.u32(5) // vertex count
        .f32(1.5) // face area
        .u32(0)
        .u32(0)
        .floats(&[0.0, 0.0, 0.0])
        .floats(&[2.0, 1.0, 0.0])
        .floats(&[1.0, 0.5, 0.0])
        .f32(1.2);
    b.u32(2)
        .asciiz(r"a3\data\a_co.paa")
        .asciiz(r"a3\data\b_co.paa");
    // One embedded material, version 11, two stages, one texgen.
    b.u32(1).asciiz(r"a3\data\a.rvmat").u32(11);
    for _ in 0..6 {
        b.floats(&[1.0, 1.0, 1.0, 1.0]);
    }
    b.f32(40.0)
        .u32(7)
        .u32(3)
        .u32(1)
        .u32(0)
        .asciiz(r"a3\data\a.bisurf");
    b.u32(1).u32(0x10);
    b.u32(2).u32(1);
    b.u32(3).asciiz(r"a3\data\a_nohq.paa").u32(0).u8(0);
    b.u32(3).asciiz(r"a3\data\a_smdi.paa").u32(0).u8(1);
    b.u32(1)
        .floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    b.u32(0).asciiz("").u32(0).u8(0); // TI stage
    b.carray(0, &[]).carray(0, &[]); // point_to_vertex, vertex_to_point
    // Faces: quad (0 1 2 3) = 20 bytes in memory, triangle (1 4 2) = 16 bytes.
    b.u32(2).u32(36).u16(0);
    b.u8(4).u32(0).u32(1).u32(2).u32(3);
    b.u8(3).u32(1).u32(4).u32(2);
    // Sections: [0, 20) texture 0 material 0; [20, 36) texture 1, no material.
    b.u32(2);
    b.u32(0).u32(20).u32(0).u32(2).u32(0).i16(0).u32(0).i32(0);
    b.u32(2).f32(1.0).f32(-1000.0).u32(0);
    b.u32(20)
        .u32(36)
        .u32(0)
        .u32(0)
        .u32(0)
        .i16(1)
        .u32(0x40)
        .i32(-1)
        .asciiz("");
    b.u32(2).f32(1.0).f32(-1000.0).u32(0);
    // Named selections: "door" = face 1, vertices 1, 4, 2; "proxy:..." = empty.
    b.u32(2);
    b.asciiz("door")
        .carray(1, &le_u32s(&[1]))
        .u32(0)
        .u8(1)
        .carray(1, &le_u32s(&[1]))
        .carray(3, &le_u32s(&[1, 4, 2]))
        .carray(0, &[]);
    b.asciiz(r"proxy:\a3\proxies\seat.001")
        .carray(0, &[])
        .u32(0)
        .u8(0)
        .carray(0, &[])
        .carray(0, &[])
        .carray(0, &[]);
    b.u32(1).asciiz("lodnoshadow").asciiz("1"); // properties
    b.u32(0); // frames
    b.u32(0xff00_0000).u32(0xff00_0000).u32(0).u8(0);
    let rest_size_at = b.len();
    b.u32(0);
    let rest_start = b.len();
    b.fill_value(5, &0u32.to_le_bytes()); // clip flags
    // UV set 0 over [0,1]: raw -32767 -> 0, 32767 -> 1, 0 -> 0.5.
    b.floats(&[0.0, 0.0, 1.0, 1.0]);
    let uv = |u: i16, v: i16| [u.to_le_bytes(), v.to_le_bytes()].concat();
    let uvs = [
        uv(-32767, -32767),
        uv(32767, -32767),
        uv(32767, 32767),
        uv(-32767, 32767),
        uv(0, 0),
    ]
    .concat();
    b.fill(5, &uvs);
    b.u32(1); // one UV set
    let pos: Vec<f32> = positions.iter().flatten().copied().collect();
    b.carray_lzo(5, &le_f32s(&pos));
    // Normals: all -Z, packed z = -511 (0x201).
    b.fill_value(5, &(0x201u32 << 20).to_le_bytes());
    b.carray(0, &[]); // tangents
    // Bone weights: vertex 4 on LOD bone 1, others on bone 0.
    let mut w = Vec::new();
    for i in 0..5 {
        w.extend_from_slice(&1u32.to_le_bytes());
        w.extend_from_slice(&[if i == 4 { 1 } else { 0 }, 255, 0, 0, 0, 0, 0, 0]);
    }
    b.carray(5, &w);
    b.carray(0, &[]); // neighbour bones
    b.u32(0);
    let rest = (b.len() - rest_start) as u32;
    b.put_u32_at(rest_size_at, rest);
    b.u8(1);
    b.0
}
/// A LOD body with no vertices, faces or selections.
pub fn empty_lod_body() -> Vec<u8> {
    let mut b = Bytes::new();
    b.u32(0).u32(0).u32(0); // proxies, sub-skeleton, skeleton -> sub-skeleton
    b.u32(0).f32(0.0).u32(0).u32(0).floats(&[0.0; 10]);
    b.u32(0).u32(0); // textures, materials
    b.carray(0, &[]).carray(0, &[]);
    b.u32(0).u32(0).u16(0); // faces
    b.u32(0).u32(0).u32(0).u32(0); // sections, selections, properties, frames
    b.u32(0).u32(0).u32(0).u8(0);
    let rest_size_at = b.len();
    b.u32(0);
    let rest_start = b.len();
    b.fill(0, &[]); // clip flags
    b.floats(&[0.0, 0.0, 1.0, 1.0]).fill(0, &[]).u32(1); // UV set 0
    b.carray(0, &[]); // positions
    b.fill(0, &[]); // normals
    b.carray(0, &[]).carray(0, &[]).carray(0, &[]);
    b.u32(0);
    let rest = (b.len() - rest_start) as u32;
    b.put_u32_at(rest_size_at, rest);
    b.u8(1);
    b.0
}
