//! Binarized `BMTR` animations (version 5).

use glam::{Quat, Vec3};

use crate::cursor::Cursor;
use crate::half::f16_to_f32;
use crate::{Animation, BoneTransform, Encoding, Error, Frame, Keystone, Result};

/// The only version found in the install (build 2.22).
const VERSION: u32 = 5;
/// Bytes per stored bone transform: 4 x i16 quaternion, 3 x f16 translation.
const TRANSFORM_SIZE: usize = 14;
/// Quaternion components are stored as `i16` scaled by this factor.
const QUAT_SCALE: f32 = 16384.0;

/// Compression flag of an array: stored as-is.
const ARRAY_RAW: u8 = 0;
/// Compression flag of an array: an LZO1X block.
const ARRAY_LZO: u8 = 2;

pub fn read(data: &[u8]) -> Result<Animation> {
    let mut c = Cursor::new(data);
    c.skip(4)?; // "BMTR"
    let version = c.u32()?;
    if version != VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    c.u8()?; // 1 in every shipped file _(meaning unknown)_
    let step = c.vec3()?;
    let phase_count = c.count(5)?;
    // 0 or 1; the engine keeps it as a flag _(meaning unknown)_.
    c.u32()?;
    // Bone count of the first phase (written by the binarizer, ignored by the engine's reader).
    c.u32()?;
    let bone_count = c.count(1)?;
    let bones = (0..bone_count)
        .map(|_| c.cstr())
        .collect::<Result<Vec<_>>>()?;

    let extra_names_count = c.count(1)?;
    let extra_names = (0..extra_names_count)
        .map(|_| c.cstr())
        .collect::<Result<Vec<_>>>()?;
    let keystone_count = c.count(10)?;
    let keystones = (0..keystone_count)
        .map(|_| {
            let kind = c.i32()?;
            let name = c.cstr()?;
            let phase = c.f32()?;
            let value = c.cstr()?;
            Ok(Keystone {
                kind,
                phase,
                name,
                value,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let phases = array(&mut c, phase_count, 4, "phase times")?;
    let mut frames = Vec::with_capacity(phase_count);
    for phase in phases.chunks_exact(4) {
        let raw = array(&mut c, bone_count, TRANSFORM_SIZE, "bone transforms")?;
        frames.push(Frame {
            phase: f32::from_le_bytes(phase.try_into().expect("4 bytes")),
            transforms: raw.chunks_exact(TRANSFORM_SIZE).map(transform).collect(),
        });
    }
    if !c.remaining().is_empty() {
        return Err(Error::Malformed(format!(
            "{} bytes after the last frame",
            c.remaining().len()
        )));
    }

    Ok(Animation {
        encoding: Encoding::Binarized { version },
        step,
        bones,
        frames,
        keystones,
        extra_names,
    })
}

/// Reads an array of `expected` elements of `size` bytes: a `u32` count, a compression flag and
/// the (possibly LZO-compressed) element bytes.
fn array(c: &mut Cursor, expected: usize, size: usize, what: &str) -> Result<Vec<u8>> {
    let offset = c.pos();
    let count = c.u32()? as usize;
    if count != expected {
        return Err(Error::Malformed(format!(
            "{what} at byte {offset}: {count} elements, expected {expected}"
        )));
    }
    let len = count * size;
    let flag = c.u8()?;
    match flag {
        ARRAY_RAW => Ok(c.bytes(len)?.to_vec()),
        ARRAY_LZO => {
            let offset = c.pos();
            let (out, used) = a3_compress::lzo::decompress(c.remaining(), len).map_err(|e| {
                Error::Decompress {
                    offset,
                    reason: e.to_string(),
                }
            })?;
            c.skip(used)?;
            Ok(out)
        }
        _ => Err(Error::Malformed(format!(
            "{what} at byte {offset}: unknown compression flag {flag}"
        ))),
    }
}

fn transform(raw: &[u8]) -> BoneTransform {
    let i16_at = |i: usize| i16::from_le_bytes([raw[i], raw[i + 1]]);
    let f16_at = |i: usize| f16_to_f32(u16::from_le_bytes([raw[i], raw[i + 1]]));
    let q = |i: usize| f32::from(i16_at(i)) / QUAT_SCALE;
    BoneTransform {
        rotation: Quat::from_xyzw(q(0), q(2), q(4), q(6)),
        translation: Vec3::new(f16_at(8), f16_at(10), f16_at(12)),
    }
}
