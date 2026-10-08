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
            lod_bodies: vec![vec![]; resolutions.len()],
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
