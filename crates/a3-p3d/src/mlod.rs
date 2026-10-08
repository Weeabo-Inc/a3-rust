//! MLOD: the editable Object Builder encoding (`MLOD` + `P3DM` LODs + `TAGG` tags).
//!
//! MLOD stores points and normals separately and gives every face corner its own point, normal
//! and UV. The reader unwelds that into render vertices (one per distinct corner) and groups faces
//! into sections by texture, material and flags, keeping first-appearance order.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::error::{Error, Result};
use crate::model::{
    Encoding, Face, Frame, Lod, Material, Model, ModelInfo, NamedSelection, Section, Vertices,
};
use crate::reader::Reader;
use crate::resolution::LodResolution;

const MLOD_VERSION: u32 = 257;
const FACE_SIZE: usize = 4 + 4 * 16 + 4 + 2;

struct RawFace {
    corners: Vec<(u32, u32, Vec2)>,
    flags: u32,
    texture: String,
    material: String,
}

pub(crate) fn read(data: &[u8]) -> Result<Model> {
    let mut r = Reader::new(data);
    r.bytes(4)?;
    let version = r.u32()?;
    if version != MLOD_VERSION {
        return Err(Error::UnsupportedVersion {
            format: "MLOD",
            version,
        });
    }
    let count = r.count(28)?;
    let lods = (0..count)
        .map(|_| read_lod(&mut r))
        .collect::<Result<Vec<_>>>()?;
    Ok(Model {
        encoding: Encoding::Mlod,
        version,
        info: info_from_lods(&lods),
        skeleton: None,
        animations: Vec::new(),
        lods,
    })
}

fn info_from_lods(lods: &[Lod]) -> ModelInfo {
    let bounds = |visual_only: bool| {
        let mut points = lods
            .iter()
            .filter(|lod| !visual_only || lod.resolution.is_visual())
            .flat_map(|lod| lod.vertices.positions.iter().copied())
            .peekable();
        points.peek()?;
        Some(points.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p))))
    };
    let mut info = ModelInfo::default();
    if let Some((lo, hi)) = bounds(false) {
        (info.bbox_min, info.bbox_max) = (lo, hi);
    }
    if let Some((lo, hi)) = bounds(true) {
        (info.bbox_visual_min, info.bbox_visual_max) = (lo, hi);
    }
    info
}

fn read_lod(r: &mut Reader) -> Result<Lod> {
    let start = r.pos();
    let tag = r.array::<4>()?;
    if &tag != b"P3DM" {
        return Err(Error::Malformed {
            offset: start,
            message: format!("expected a P3DM LOD, found {tag:?} (SP3X LODs are not supported)"),
        });
    }
    let _header_size = r.u32()?;
    let version = r.u32()?;
    if version != 0x100 {
        return Err(Error::UnsupportedVersion {
            format: "P3DM",
            version,
        });
    }
    let n_points = r.count(16)?;
    let n_normals = r.count(12)?;
    let n_faces = r.count(FACE_SIZE)?;
    let _flags = r.u32()?;

    let mut points = Vec::with_capacity(n_points);
    let mut point_flags = Vec::with_capacity(n_points);
    for _ in 0..n_points {
        points.push(r.vec3()?);
        point_flags.push(r.u32()?);
    }
    let normals = (0..n_normals)
        .map(|_| r.vec3())
        .collect::<Result<Vec<_>>>()?;

    let mut faces = Vec::with_capacity(n_faces);
    for _ in 0..n_faces {
        let at = r.pos();
        let n = r.u32()? as usize;
        if !(3..=4).contains(&n) {
            return Err(r.malformed(format!("face with {n} vertices")));
        }
        let mut corners = Vec::with_capacity(n);
        for i in 0..4 {
            let point = r.u32()?;
            let normal = r.u32()?;
            let uv = Vec2::new(r.f32()?, r.f32()?);
            if i < n {
                if point as usize >= n_points || normal as usize >= n_normals.max(1) {
                    return Err(Error::Malformed {
                        offset: at,
                        message: format!("face corner references point {point} / normal {normal}"),
                    });
                }
                corners.push((point, normal, uv));
            }
        }
        faces.push(RawFace {
            corners,
            flags: r.u32()?,
            texture: r.asciiz()?,
            material: r.asciiz()?,
        });
    }

    let tagg = r.array::<4>()?;
    if &tagg != b"TAGG" {
        return Err(r.malformed(format!("expected TAGG, found {tagg:?}")));
    }
    let mut tags = Vec::new();
    loop {
        let _active = r.bool()?;
        let name = r.asciiz()?;
        let size = r.count(1)?;
        let at = r.pos();
        let data = r.bytes(size)?;
        if name == "#EndOfFile#" {
            break;
        }
        tags.push((name, at, data));
    }
    let resolution = LodResolution(r.f32()?);

    let raw = RawLod {
        points,
        point_flags,
        normals,
        faces,
    };
    raw.build(resolution, &tags)
}

struct RawLod {
    points: Vec<Vec3>,
    point_flags: Vec<u32>,
    normals: Vec<Vec3>,
    faces: Vec<RawFace>,
}

impl RawLod {
    fn build(self, resolution: LodResolution, tags: &[(String, usize, &[u8])]) -> Result<Lod> {
        let corner_count: usize = self.faces.iter().map(|f| f.corners.len()).sum();
        let extra_uvs = self.extra_uv_sets(tags, corner_count)?;

        // Sections: group by (texture, material, flags) in first-appearance order.
        let mut lod = Lod {
            resolution,
            ..Lod::default()
        };
        let mut groups: Vec<(Option<u32>, Option<u32>, u32)> = Vec::new();
        let mut face_group = Vec::with_capacity(self.faces.len());
        for face in &self.faces {
            let key = (
                intern(&mut lod.textures, &face.texture),
                intern_material(&mut lod.materials, &face.material),
                face.flags,
            );
            let g = groups.iter().position(|k| *k == key).unwrap_or_else(|| {
                groups.push(key);
                groups.len() - 1
            });
            face_group.push(g);
        }
        let mut order: Vec<usize> = (0..self.faces.len()).collect();
        order.sort_by_key(|&i| face_group[i]);
        let mut new_index = vec![0u32; self.faces.len()];
        for (new, &old) in order.iter().enumerate() {
            new_index[old] = new as u32;
        }
        let mut first = 0u32;
        for (g, &(texture, material, flags)) in groups.iter().enumerate() {
            let n = face_group.iter().filter(|&&fg| fg == g).count() as u32;
            lod.sections.push(Section {
                faces: first..first + n,
                texture,
                material,
                flags,
                odol: None,
            });
            first += n;
        }

        // Corner offsets of each face into the per-corner UV tag arrays (file face order).
        let mut corner_base = Vec::with_capacity(self.faces.len());
        let mut acc = 0;
        for f in &self.faces {
            corner_base.push(acc);
            acc += f.corners.len();
        }

        // Unweld: one vertex per distinct (point, normal, uvs) corner, in render face order.
        let mut vertex_of: HashMap<Vec<u32>, u32> = HashMap::new();
        let mut vertices = Vertices {
            uv_sets: vec![Vec::new(); 1 + extra_uvs.len()],
            ..Vertices::default()
        };
        for &old in &order {
            let face = &self.faces[old];
            let mut idx = [0u32; 4];
            for (c, &(point, normal, uv)) in face.corners.iter().enumerate() {
                let corner = corner_base[old] + c;
                let mut key = vec![point, normal, uv.x.to_bits(), uv.y.to_bits()];
                for set in &extra_uvs {
                    key.extend([set[corner].x.to_bits(), set[corner].y.to_bits()]);
                }
                idx[c] = *vertex_of.entry(key).or_insert_with(|| {
                    let v = vertices.positions.len() as u32;
                    vertices.positions.push(self.points[point as usize]);
                    vertices.flags.push(self.point_flags[point as usize]);
                    if let Some(n) = self.normals.get(normal as usize) {
                        vertices.normals.push(*n);
                    }
                    vertices.uv_sets[0].push(uv);
                    for (s, set) in extra_uvs.iter().enumerate() {
                        vertices.uv_sets[s + 1].push(set[corner]);
                    }
                    lod.vertex_to_point.push(point);
                    v
                });
            }
            lod.faces.push(match face.corners.len() {
                3 => Face::triangle(idx[0], idx[1], idx[2]),
                _ => Face::quad(idx[0], idx[1], idx[2], idx[3]),
            });
        }
        if vertices.normals.len() != vertices.positions.len() {
            vertices.normals.clear();
        }
        lod.vertices = vertices;

        let n_points = self.points.len();
        let n_faces = self.faces.len();
        for &(ref name, at, data) in tags {
            let wrong_size = |what: &str| Error::Malformed {
                offset: at,
                message: format!("{what} tag {name:?} has {} bytes", data.len()),
            };
            match name.as_str() {
                "#Property#" => {
                    let mut t = Reader::new(data);
                    let key = t.fixed_str(64).map_err(|_| wrong_size("property"))?;
                    let value = t.fixed_str(64).map_err(|_| wrong_size("property"))?;
                    lod.properties.push((key, value));
                }
                "#Mass#" => {
                    if data.len() != 4 * n_points {
                        return Err(wrong_size("mass"));
                    }
                    lod.point_masses = f32s(data);
                }
                "#SharpEdges#" => {
                    let ids: Vec<u32> = data
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect();
                    lod.sharp_edges = ids.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
                }
                "#Animation#" => {
                    if data.len() != 4 + 12 * n_points {
                        return Err(wrong_size("animation"));
                    }
                    let values = f32s(data);
                    let positions = lod
                        .vertex_to_point
                        .iter()
                        .map(|&p| {
                            let i = 1 + 3 * p as usize;
                            Vec3::new(values[i], values[i + 1], values[i + 2])
                        })
                        .collect();
                    lod.frames.push(Frame {
                        time: values[0],
                        positions,
                    });
                }
                _ if name.starts_with('#') => {}
                _ => {
                    if data.len() != n_points + n_faces {
                        return Err(wrong_size("named selection"));
                    }
                    let (point_weights, face_flags) = data.split_at(n_points);
                    let mut sel = NamedSelection {
                        name: name.clone(),
                        ..NamedSelection::default()
                    };
                    for (v, &p) in lod.vertex_to_point.iter().enumerate() {
                        let w = point_weights[p as usize];
                        if w != 0 {
                            sel.vertices.push(v as u32);
                            sel.weights.push(w);
                        }
                    }
                    if sel.weights.iter().all(|&w| w == 1) {
                        sel.weights.clear();
                    }
                    sel.faces = face_flags
                        .iter()
                        .enumerate()
                        .filter(|&(_, &f)| f != 0)
                        .map(|(i, _)| new_index[i])
                        .collect();
                    sel.faces.sort_unstable();
                    lod.named_selections.push(sel);
                }
            }
        }
        Ok(lod)
    }

    /// `#UVSet#` tags with id >= 1, each one UV per face corner, placed by id.
    fn extra_uv_sets(
        &self,
        tags: &[(String, usize, &[u8])],
        corners: usize,
    ) -> Result<Vec<Vec<Vec2>>> {
        let mut sets: Vec<Vec<Vec2>> = Vec::new();
        for (name, at, data) in tags {
            if name != "#UVSet#" {
                continue;
            }
            if data.len() != 4 + 8 * corners {
                return Err(Error::Malformed {
                    offset: *at,
                    message: format!("UV set tag has {} bytes for {corners} corners", data.len()),
                });
            }
            let id = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
            if id == 0 {
                continue;
            }
            let uvs = f32s(&data[4..])
                .chunks_exact(2)
                .map(|c| Vec2::new(c[0], c[1]))
                .collect();
            if sets.len() < id {
                sets.resize(id, vec![Vec2::ZERO; corners]);
            }
            sets[id - 1] = uvs;
        }
        Ok(sets)
    }
}

fn f32s(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn intern(list: &mut Vec<String>, name: &str) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    let i = list.iter().position(|n| n == name).unwrap_or_else(|| {
        list.push(name.to_owned());
        list.len() - 1
    });
    Some(i as u32)
}

fn intern_material(list: &mut Vec<Material>, name: &str) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    let i = list.iter().position(|m| m.name == name).unwrap_or_else(|| {
        list.push(Material {
            name: name.to_owned(),
            ..Material::default()
        });
        list.len() - 1
    });
    Some(i as u32)
}
