//! Plain `RTM_0101` animations, optionally preceded by an `RTM_MDAT` keystone section.

use glam::{Mat3, Vec3};

use crate::cursor::Cursor;
use crate::{Animation, BoneTransform, Encoding, Error, Frame, Keystone, Result};

const NAME_SIZE: usize = 32;
const MATRIX_SIZE: usize = 12 * 4;

pub fn read(data: &[u8]) -> Result<Animation> {
    let mut c = Cursor::new(data);
    let mut keystones = Vec::new();
    if c.array::<8>()? == *b"RTM_MDAT" {
        c.skip(4)?; // always 0 _(uncertain)_
        let count = c.count(12)?;
        for _ in 0..count {
            let phase = c.f32()?;
            let name = c.sized_str()?;
            let value = c.sized_str()?;
            keystones.push(Keystone {
                kind: -1,
                phase,
                name,
                value,
            });
        }
        let sig = c.array::<8>()?;
        if sig != *b"RTM_0101" {
            return Err(Error::UnknownSignature(sig.to_vec()));
        }
    }

    let step = c.vec3()?;
    let frame_count = c.count(4)?;
    let bone_count = c.count(NAME_SIZE)?;
    let bones = (0..bone_count)
        .map(|_| c.fixed_str(NAME_SIZE))
        .collect::<Result<Vec<_>>>()?;

    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let phase = c.f32()?;
        let mut transforms = Vec::with_capacity(bone_count);
        for bone in &bones {
            let offset = c.pos();
            let name = c.fixed_str(NAME_SIZE)?;
            if !name.eq_ignore_ascii_case(bone) {
                return Err(Error::Malformed(format!(
                    "frame bone {name:?} at byte {offset} does not match header bone {bone:?}"
                )));
            }
            let raw = c.bytes(MATRIX_SIZE)?;
            let m: [f32; 12] = std::array::from_fn(|i| {
                f32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().expect("4 bytes"))
            });
            let orientation = Mat3::from_cols_slice(&m[..9]);
            transforms.push(BoneTransform::from_matrix(
                orientation,
                Vec3::new(m[9], m[10], m[11]),
            ));
        }
        frames.push(Frame { phase, transforms });
    }

    Ok(Animation {
        encoding: Encoding::Plain,
        step,
        bones,
        frames,
        keystones,
        extra_names: Vec::new(),
    })
}
