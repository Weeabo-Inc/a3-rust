//! Serialising a [`Texture`] to PAA bytes.

use crate::error::malformed;
use crate::texture::{LZO_FLAG, LZSS_CHECKSUM, OFFSET_SLOTS};
use crate::{Compression, Mip, Result, Texture};

/// Largest value of the `u24` stored-size field.
const MAX_STORED: usize = 0xFF_FFFF;

fn tagg(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(b"GGAT");
    out.extend(name.iter().rev());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

impl Texture {
    /// Serialises the texture as a PAA file.
    ///
    /// TAGGs are written in the order TexConvert uses (`AVGC`, `MAXC`, `FLAG`, `SWIZ`, `PROC`,
    /// others, `OFFS`), the `OFFS` table is recomputed, and every mipmap is stored as its
    /// [`Mip::compression`] says for DXT formats. Non-DXT levels are always LZSS-compressed:
    /// the engine cannot read them raw.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.mips.is_empty() {
            return malformed("PAA", "cannot write a texture without mipmaps");
        }
        if self.mips.len() > OFFSET_SLOTS {
            return malformed(
                "PAA",
                format!(
                    "{} mipmaps; the format holds {OFFSET_SLOTS}",
                    self.mips.len()
                ),
            );
        }
        let stored = self
            .mips
            .iter()
            .enumerate()
            .map(|(i, mip)| self.store_mip(i, mip))
            .collect::<Result<Vec<_>>>()?;

        let mut out = self.format.tag().to_le_bytes().to_vec();
        if let Some(color) = self.average_color {
            tagg(&mut out, b"AVGC", &color.to_bgra());
        }
        if let Some(color) = self.max_color {
            tagg(&mut out, b"MAXC", &color.to_bgra());
        }
        if let Some(flags) = self.flags {
            tagg(&mut out, b"FLAG", &flags.0.to_le_bytes());
        }
        if let Some(swizzle) = self.swizzle {
            tagg(&mut out, b"SWIZ", &swizzle.to_bytes());
        }
        if let Some(text) = &self.procedural {
            let mut data = text.as_bytes().to_vec();
            data.push(0);
            tagg(&mut out, b"PROC", &data);
        }
        for other in &self.other_taggs {
            tagg(&mut out, &other.name, &other.data);
        }

        let header_end = out.len() + 8 + 4 + OFFSET_SLOTS * 4 + 2 + self.palette.len() * 3;
        let mut offsets = [0u8; OFFSET_SLOTS * 4];
        let mut at = header_end;
        for (slot, (header, data)) in offsets.chunks_exact_mut(4).zip(&stored) {
            slot.copy_from_slice(&(at as u32).to_le_bytes());
            at += header.len() + data.len();
        }
        tagg(&mut out, b"OFFS", &offsets);

        out.extend_from_slice(&(self.palette.len() as u16).to_le_bytes());
        for color in &self.palette {
            out.extend_from_slice(color);
        }
        for (header, data) in &stored {
            out.extend_from_slice(header);
            out.extend_from_slice(data);
        }
        out.extend_from_slice(&[0; 6]);
        Ok(out)
    }

    /// The 7-byte header and stored data of mipmap `index`.
    fn store_mip(&self, index: usize, mip: &Mip) -> Result<([u8; 7], Vec<u8>)> {
        let raw_len = self.format.data_len(mip.width, mip.height);
        if mip.data.len() != raw_len {
            return malformed(
                "PAA",
                format!(
                    "mipmap {index} ({}x{}) has {} bytes, {:?} needs {raw_len}",
                    mip.width,
                    mip.height,
                    mip.data.len(),
                    self.format
                ),
            );
        }
        if mip.width == 0 || mip.height == 0 || mip.width >= LZO_FLAG {
            return malformed(
                "PAA",
                format!("mipmap {index} size {}x{}", mip.width, mip.height),
            );
        }
        let mut width = mip.width;
        let data = match (mip.compression, self.format.is_dxt()) {
            (_, false) => a3_compress::lzss::compress(&mip.data, LZSS_CHECKSUM),
            (Compression::None, true) => mip.data.clone(),
            (Compression::Lzo, true) => {
                width |= LZO_FLAG;
                lzokay_native::compress(&mip.data).map_err(|e| crate::Error::Malformed {
                    format: "PAA",
                    reason: format!("LZO compression of mipmap {index} failed: {e:?}"),
                })?
            }
            (compression, true) => {
                return malformed(
                    "PAA",
                    format!("{compression:?} is not used with {:?}", self.format),
                );
            }
        };
        if data.len() > MAX_STORED {
            return malformed(
                "PAA",
                format!(
                    "mipmap {index} stores {} bytes; the size field holds at most {MAX_STORED} \
                     (compress it)",
                    data.len()
                ),
            );
        }
        let mut header = [0u8; 7];
        header[..2].copy_from_slice(&width.to_le_bytes());
        header[2..4].copy_from_slice(&mip.height.to_le_bytes());
        header[4..].copy_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        Ok((header, data))
    }
}
