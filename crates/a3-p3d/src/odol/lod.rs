//! One ODOL v73 LOD body: everything between its start and end offset.

use std::borrow::Cow;

use glam::{Vec2, Vec3, Vec4};

use crate::error::{Error, Result};
use crate::model::{
    BoneWeights, Collimator, Face, Frame, Lod, Material, MaterialStage, NamedSelection,
    NeighborBones, OdolLod, OdolSection, Proxy, Section, TexGen, Vertices,
};
use crate::reader::Reader;

/// Flag byte before the data of a non-empty compressed array.
const RAW: u8 = 0;
const LZO: u8 = 2;

/// LZO cannot expand data more than this, so larger claimed sizes are corrupt input.
const MAX_LZO_RATIO: usize = 256;

/// Upper bound for a default-filled array, far above any real vertex count.
const MAX_FILL: usize = 1 << 24;

/// Decodes the LOD body at `data[start..end]` into `lod`, whose header-side fields are set.
pub(crate) fn read(data: &[u8], start: usize, end: usize, lod: &mut Lod) -> Result<()> {
    let mut r = Reader::new(&data[..end]);
    r.seek(start)?;
    let mut odol = lod.odol.take().unwrap_or_default();

    let proxy_count = r.count(4 + 48 + 16)?;
    lod.proxies = (0..proxy_count)
        .map(|_| proxy(&mut r))
        .collect::<Result<_>>()?;
    let n = r.count(4)?;
    odol.sub_skeleton = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
    let n = r.count(4)?;
    odol.skeleton_to_sub_skeleton = (0..n)
        .map(|_| {
            let k = r.count(4)?;
            (0..k).map(|_| r.u32()).collect()
        })
        .collect::<Result<_>>()?;
    let vertex_count = r.u32()? as usize;
    odol.face_area = r.f32()?;
    odol.or_hints = r.u32()?;
    odol.and_hints = r.u32()?;
    odol.bbox_min = r.vec3()?;
    odol.bbox_max = r.vec3()?;
    odol.bbox_center = r.vec3()?;
    odol.bbox_radius = r.f32()?;
    let n = r.count(1)?;
    lod.textures = (0..n).map(|_| r.asciiz()).collect::<Result<_>>()?;
    let n = r.count(4)?;
    lod.materials = (0..n).map(|_| material(&mut r)).collect::<Result<_>>()?;
    odol.point_to_vertex = carray(&mut r, 4, u32_at)?;
    lod.vertex_to_point = carray(&mut r, 4, u32_at)?;

    let (faces, face_offsets) = faces(&mut r, vertex_count)?;
    lod.faces = faces;
    let n = r.count(4 * 8)?;
    lod.sections = (0..n)
        .map(|_| {
            section(
                &mut r,
                &face_offsets,
                lod.textures.len(),
                lod.materials.len(),
            )
        })
        .collect::<Result<_>>()?;
    let n = r.count(1)?;
    lod.named_selections = (0..n)
        .map(|_| named_selection(&mut r))
        .collect::<Result<_>>()?;
    let n = r.count(2)?;
    lod.properties = (0..n)
        .map(|_| Ok((r.asciiz()?, r.asciiz()?)))
        .collect::<Result<_>>()?;
    let n = r.count(8)?;
    lod.frames = (0..n)
        .map(|_| {
            let time = r.f32()?;
            let k = r.count(12)?;
            let positions = (0..k).map(|_| r.vec3()).collect::<Result<_>>()?;
            Ok(Frame { time, positions })
        })
        .collect::<Result<_>>()?;
    odol.icon_color = r.u32()?;
    odol.selected_color = r.u32()?;
    odol.special = r.u32()?;
    odol.vertex_bone_ref_is_simple = r.bool()?;
    let rest_size = r.u32()? as usize;
    let rest_start = r.pos();

    lod.vertices = vertices(&mut r, vertex_count, &mut odol)?;
    odol.collimator = collimator(&mut r)?;
    if r.pos() - rest_start != rest_size {
        return Err(r.malformed(format!(
            "vertex block is {} bytes, header says {rest_size}",
            r.pos() - rest_start
        )));
    }
    odol.unknown_u8 = r.u8()?;
    if r.pos() != end {
        return Err(r.malformed(format!("LOD ends at {:#x}, table says {end:#x}", r.pos())));
    }
    check_indices(lod, &r)?;
    lod.odol = Some(odol);
    Ok(())
}

fn proxy(r: &mut Reader) -> Result<Proxy> {
    Ok(Proxy {
        model: r.asciiz()?,
        orientation: r.mat3()?,
        position: r.vec3()?,
        sequence_id: r.i32()?,
        named_selection: r.i32()?,
        bone: r.i32()?,
        section: r.i32()?,
    })
}

fn color(r: &mut Reader) -> Result<Vec4> {
    Ok(Vec4::new(r.f32()?, r.f32()?, r.f32()?, r.f32()?))
}

fn stage(r: &mut Reader) -> Result<MaterialStage> {
    Ok(MaterialStage {
        filter: r.u32()?,
        texture: r.asciiz()?,
        tex_gen: r.u32()?,
        use_world_env_map: r.bool()?,
    })
}

fn material(r: &mut Reader) -> Result<Material> {
    let name = r.asciiz()?;
    let version = r.u32()?;
    if version != 11 {
        return Err(Error::UnsupportedVersion {
            format: "embedded material",
            version,
        });
    }
    let mut m = Material {
        name,
        version,
        emissive: color(r)?,
        ambient: color(r)?,
        diffuse: color(r)?,
        forced_diffuse: color(r)?,
        specular: color(r)?,
        specular2: color(r)?,
        specular_power: r.f32()?,
        pixel_shader: r.u32()?,
        vertex_shader: r.u32()?,
        main_light: r.u32()?,
        fog_mode: r.u32()?,
        surface: r.asciiz()?,
        ..Material::default()
    };
    let _render_flag_count = r.u32()?;
    m.render_flags = r.u32()?;
    let stages = r.count(10)?;
    let tex_gens = r.count(52)?;
    m.stages = (0..stages).map(|_| stage(r)).collect::<Result<_>>()?;
    m.tex_gens = (0..tex_gens)
        .map(|_| {
            let uv_source = r.u32()?;
            let mut transform = [[0.0; 3]; 4];
            for row in &mut transform {
                *row = [r.f32()?, r.f32()?, r.f32()?];
            }
            Ok(TexGen {
                uv_source,
                transform,
            })
        })
        .collect::<Result<_>>()?;
    m.ti_stage = Some(stage(r)?);
    Ok(m)
}

/// Faces and, per face, its byte offset in the engine's face block (`4 + 4 * n` per face),
/// which sections use as their range bounds. The last offset is the block size.
fn faces(r: &mut Reader, vertex_count: usize) -> Result<(Vec<Face>, Vec<u32>)> {
    let count = r.count(1 + 12)?;
    let alloc = r.u32()?;
    let _zero = r.u16()?;
    let mut faces = Vec::with_capacity(count);
    let mut offsets = Vec::with_capacity(count + 1);
    let mut offset = 0u32;
    for _ in 0..count {
        offsets.push(offset);
        let n = r.u8()?;
        let mut idx = [0u32; 4];
        match n {
            3 | 4 => {
                for i in idx.iter_mut().take(usize::from(n)) {
                    *i = r.u32()?;
                    if *i as usize >= vertex_count {
                        return Err(r.malformed(format!(
                            "face vertex {i} out of range ({vertex_count} vertices)"
                        )));
                    }
                }
            }
            _ => return Err(r.malformed(format!("face with {n} vertices"))),
        }
        faces.push(if n == 3 {
            Face::triangle(idx[0], idx[1], idx[2])
        } else {
            Face::quad(idx[0], idx[1], idx[2], idx[3])
        });
        offset += 4 + 4 * u32::from(n);
    }
    offsets.push(offset);
    if offset != alloc {
        return Err(r.malformed(format!(
            "face block size {offset} does not match the stored {alloc}"
        )));
    }
    Ok((faces, offsets))
}

fn face_index(r: &Reader, offsets: &[u32], offset: u32) -> Result<u32> {
    offsets
        .binary_search(&offset)
        .map(|i| i as u32)
        .map_err(|_| r.malformed(format!("section bound {offset} is not a face boundary")))
}

fn section(
    r: &mut Reader,
    face_offsets: &[u32],
    textures: usize,
    materials: usize,
) -> Result<Section> {
    let start = r.u32()?;
    let end = r.u32()?;
    let faces = face_index(r, face_offsets, start)?..face_index(r, face_offsets, end)?;
    let min_bone = r.u32()?;
    let bone_count = r.u32()?;
    let _mat_dummy = r.u32()?;
    let texture = r.i16()?;
    let flags = r.u32()?;
    let material = r.i32()?;
    let material_name = if material == -1 {
        r.asciiz()?
    } else {
        String::new()
    };
    let n = r.count(4)?;
    let area_over_tex = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
    let collimator = collimator(r)?;
    let index = |i: i64, len: usize, what: &str| -> Result<Option<u32>> {
        match i {
            -1 => Ok(None),
            i if (0..len as i64).contains(&i) => Ok(Some(i as u32)),
            i => Err(r.malformed(format!("section {what} index {i} out of range"))),
        }
    };
    Ok(Section {
        faces,
        texture: index(i64::from(texture), textures, "texture")?,
        material: index(i64::from(material), materials, "material")?,
        flags,
        odol: Some(OdolSection {
            min_bone,
            bone_count,
            material_name,
            area_over_tex,
            collimator,
        }),
    })
}

/// A `u32` presence flag, then (when non-zero) a `CollimatorInfo`.
fn collimator(r: &mut Reader) -> Result<Option<Collimator>> {
    if r.u32()? == 0 {
        return Ok(None);
    }
    Ok(Some(Collimator {
        origin: r.vec3()?,
        axis_a: r.vec3()?,
        size_a: r.f32()?,
        axis_b: r.vec3()?,
        size_b: r.f32()?,
    }))
}

fn named_selection(r: &mut Reader) -> Result<NamedSelection> {
    let name = r.asciiz()?;
    let faces = carray(r, 4, u32_at)?;
    let _zero = r.u32()?;
    let sectional = r.bool()?;
    let sections = carray(r, 4, u32_at)?;
    let vertices = carray(r, 4, u32_at)?;
    let weights = carray(r, 1, |b| b[0])?;
    Ok(NamedSelection {
        name,
        faces,
        vertices,
        weights,
        sectional,
        sections,
    })
}

fn vertices(r: &mut Reader, count: usize, odol: &mut OdolLod) -> Result<Vertices> {
    let mut v = Vertices {
        flags: fill(r, 4, u32_at)?,
        ..Vertices::default()
    };
    v.uv_sets.push(uv_set(r)?);
    let sets = r.u32()?;
    for _ in 1..sets {
        v.uv_sets.push(uv_set(r)?);
    }
    v.positions = carray(r, 12, |b| {
        Vec3::new(f32_at(&b[0..]), f32_at(&b[4..]), f32_at(&b[8..]))
    })?;
    v.normals = fill(r, 4, |b| unpack_vector(u32_at(b)))?;
    v.tangents = carray(r, 8, |b| {
        [
            unpack_vector(u32_at(&b[0..])),
            unpack_vector(u32_at(&b[4..])),
        ]
    })?;
    v.bone_weights = carray(r, 12, bone_weights)?;
    odol.neighbor_bones = carray(r, 32, |b| NeighborBones {
        pos_a: u16::from_le_bytes([b[0], b[1]]),
        weights_a: bone_weights(&b[4..16]),
        pos_b: u16::from_le_bytes([b[16], b[17]]),
        weights_b: bone_weights(&b[20..32]),
    })?;

    let arrays: [(&str, usize); 6] = [
        ("point flag", v.flags.len()),
        ("position", v.positions.len()),
        ("normal", v.normals.len()),
        ("tangent", v.tangents.len()),
        ("bone weight", v.bone_weights.len()),
        ("neighbour bone", odol.neighbor_bones.len()),
    ];
    for (what, len) in arrays.into_iter().chain(
        v.uv_sets
            .iter()
            .map(|set| ("UV", set.len()))
            .collect::<Vec<_>>(),
    ) {
        if len != 0 && len != count {
            return Err(r.malformed(format!("{len} {what} entries for {count} vertices")));
        }
    }
    if v.positions.len() != count {
        return Err(r.malformed(format!(
            "{} positions for {count} vertices",
            v.positions.len()
        )));
    }
    Ok(v)
}

/// A UV set: min/max range, then per vertex two `i16` mapped from `-32767..=32767` onto it.
fn uv_set(r: &mut Reader) -> Result<Vec<Vec2>> {
    let min = Vec2::new(r.f32()?, r.f32()?);
    let max = Vec2::new(r.f32()?, r.f32()?);
    let scale = (max - min) / 65534.0;
    fill(r, 4, |b| {
        let raw = Vec2::new(
            f32::from(i16::from_le_bytes([b[0], b[1]])),
            f32::from(i16::from_le_bytes([b[2], b[3]])),
        );
        min + (raw + 32767.0) * scale
    })
}

/// A unit vector packed as three signed 10-bit fields (x low), each decoding to `f / 511`.
///
/// With this sign, normals point out of closed shapes (see `docs/re/p3d-odol.md`).
pub(crate) fn unpack_vector(packed: u32) -> Vec3 {
    let field = |shift: u32| {
        let v = ((packed >> shift) & 0x3ff) as i32;
        let v = if v > 511 { v - 1024 } else { v };
        v as f32 / 511.0
    };
    Vec3::new(field(0), field(10), field(20))
}

fn bone_weights(b: &[u8]) -> BoneWeights {
    BoneWeights {
        count: u32_at(b),
        pairs: [(b[4], b[5]), (b[6], b[7]), (b[8], b[9]), (b[10], b[11])],
    }
}

fn check_indices(lod: &Lod, r: &Reader) -> Result<()> {
    let vertices = lod.vertices.len() as u32;
    let faces = lod.faces.len() as u32;
    for sel in &lod.named_selections {
        let bad_vertex = sel.vertices.iter().any(|&v| v >= vertices);
        let bad_face = sel.faces.iter().any(|&f| f >= faces);
        if bad_vertex || bad_face {
            return Err(r.malformed(format!(
                "named selection {:?} indexes outside the LOD",
                sel.name
            )));
        }
    }
    Ok(())
}

fn u32_at(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn f32_at(b: &[u8]) -> f32 {
    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// The data of a compressed array of `len` bytes: a flag byte, then raw or LZO1X data.
fn compressed<'a>(r: &mut Reader<'a>, len: usize) -> Result<Cow<'a, [u8]>> {
    if len == 0 {
        return Ok(Cow::Borrowed(&[]));
    }
    let flag_at = r.pos();
    match r.u8()? {
        RAW => Ok(Cow::Borrowed(r.bytes(len)?)),
        LZO => {
            let offset = r.pos();
            if len > r.remaining().saturating_mul(MAX_LZO_RATIO) {
                return Err(r.malformed(format!("compressed array claims {len} bytes")));
            }
            let (out, used) = a3_compress::lzo::decompress(r.rest(), len)
                .map_err(|source| Error::Decompress { offset, source })?;
            r.bytes(used)?;
            Ok(Cow::Owned(out))
        }
        flag => Err(Error::Malformed {
            offset: flag_at,
            message: format!("unknown compressed-array flag {flag}"),
        }),
    }
}

/// `u32 count` + [`compressed`] data of `count * size` bytes, decoded element by element.
fn carray<T>(r: &mut Reader, size: usize, decode: impl Fn(&[u8]) -> T) -> Result<Vec<T>> {
    let count = r.u32()? as usize;
    let data = compressed(r, count.saturating_mul(size))?;
    Ok(data.chunks_exact(size).map(decode).collect())
}

/// `u32 count`, a default-fill flag (present even for zero elements), then either one element
/// repeated `count` times or [`compressed`] data.
fn fill<T: Clone>(r: &mut Reader, size: usize, decode: impl Fn(&[u8]) -> T) -> Result<Vec<T>> {
    let count = r.u32()? as usize;
    if r.bool()? {
        let value = decode(r.bytes(size)?);
        if count > MAX_FILL {
            return Err(r.malformed(format!("filled array claims {count} elements")));
        }
        return Ok(vec![value; count]);
    }
    let data = compressed(r, count.saturating_mul(size))?;
    Ok(data.chunks_exact(size).map(decode).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_signed_ten_bit_fields() {
        // x = -511 (0x201), y = 0, z = 511 (0x1ff)
        let packed = 0x201 | (0x1ff << 20);
        assert_eq!(unpack_vector(packed), Vec3::new(-1.0, 0.0, 1.0));
        assert_eq!(unpack_vector(0), Vec3::ZERO);
    }
}
