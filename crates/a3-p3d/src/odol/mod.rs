//! ODOL: the binarised encoding that ships in game PBOs. Layout: `docs/re/p3d-odol.md`.

mod header;
mod lod;

use crate::error::{Error, Result};
use crate::model::{Encoding, Lod, LodSummary, Model, OdolLod};
use crate::reader::Reader;
use crate::resolution::LodResolution;

/// The only ODOL version in the 2.22 install, and the only one this crate reads.
pub(crate) const VERSION: u32 = 73;

pub(crate) fn read(data: &[u8], keep: &dyn Fn(LodResolution) -> bool) -> Result<Model> {
    let mut r = Reader::new(data);
    r.bytes(4)?;
    let version = r.u32()?;
    if version != VERSION {
        return Err(Error::UnsupportedVersion {
            format: "ODOL",
            version,
        });
    }
    let app_id = r.u32()?;
    let muzzle_flash = r.asciiz()?;
    let lod_count = r.count(4)?;
    let resolutions = (0..lod_count)
        .map(|_| r.f32().map(LodResolution))
        .collect::<Result<Vec<_>>>()?;

    let (mut info, skeleton) = header::model_info(&mut r, lod_count)?;
    info.app_id = app_id;
    info.muzzle_flash = muzzle_flash;

    let (animations, bone_animations) = if r.bool()? {
        header::animations(&mut r, lod_count, skeleton.as_ref())?
    } else {
        (Vec::new(), vec![Vec::new(); lod_count])
    };

    let starts = (0..lod_count)
        .map(|_| r.u32())
        .collect::<Result<Vec<_>>>()?;
    let ends = (0..lod_count)
        .map(|_| r.u32())
        .collect::<Result<Vec<_>>>()?;
    let permanent = (0..lod_count)
        .map(|_| r.bool())
        .collect::<Result<Vec<_>>>()?;

    let mut lods = Vec::with_capacity(lod_count);
    for (i, ((&resolution, &permanent), bone_animations)) in resolutions
        .iter()
        .zip(&permanent)
        .zip(bone_animations)
        .enumerate()
    {
        let summary = if permanent {
            None
        } else {
            Some(lod_summary(&mut r)?)
        };
        let (start, end) = (starts[i] as usize, ends[i] as usize);
        if start > end || end > data.len() {
            return Err(Error::Malformed {
                offset: r.pos(),
                message: format!("LOD {i} spans {start:#x}..{end:#x} of {:#x}", data.len()),
            });
        }
        let mut lod = Lod {
            resolution,
            bone_animations,
            odol: Some(OdolLod {
                permanent,
                summary,
                ..OdolLod::default()
            }),
            ..Lod::default()
        };
        if keep(resolution) {
            lod::read(data, start, end, &mut lod)?;
        }
        lods.push(lod);
    }
    Ok(Model {
        encoding: Encoding::Odol,
        version,
        info,
        skeleton,
        animations,
        lods,
    })
}

fn lod_summary(r: &mut Reader) -> Result<LodSummary> {
    Ok(LodSummary {
        faces: r.u32()?,
        color: r.u32()?,
        special: r.u32()?,
        or_hints: r.u32()?,
        has_skeleton: r.bool()?,
        vertices: r.u32()?,
        face_area: r.f32()?,
    })
}
