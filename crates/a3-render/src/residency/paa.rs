//! [`TextureSource`] for PAA/PAC files.

use std::sync::Arc;

use a3_paa::{PaaHeader, PixelFormat};

use super::{LoadedMips, MipRequest, TextureInfo, TextureSource};
use crate::texture::TextureFormat;

/// Reads a file by key (a VFS path); shared with the loader threads.
pub type FileReader = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Streams PAA textures. DXT1/3/5 stay block-compressed when the GPU samples BC and the texture
/// has no channel swizzle; everything else is decoded to RGBA8 (swizzles undone).
pub struct PaaSource {
    reader: FileReader,
    bc_supported: bool,
}

impl PaaSource {
    pub fn new(reader: FileReader, bc_supported: bool) -> PaaSource {
        PaaSource {
            reader,
            bc_supported,
        }
    }
}

fn block_format(format: PixelFormat) -> Option<TextureFormat> {
    match format {
        PixelFormat::Dxt1 => Some(TextureFormat::Bc1),
        PixelFormat::Dxt2 | PixelFormat::Dxt3 => Some(TextureFormat::Bc2),
        PixelFormat::Dxt4 | PixelFormat::Dxt5 => Some(TextureFormat::Bc3),
        _ => None,
    }
}

impl TextureSource for PaaSource {
    fn load(&self, key: &str, request: MipRequest) -> Result<LoadedMips, String> {
        let bytes = (self.reader)(key).ok_or_else(|| format!("{key}: not found"))?;
        let header = PaaHeader::read(&bytes).map_err(|e| format!("{key}: {e}"))?;
        let first = header
            .mips
            .first()
            .ok_or_else(|| format!("{key}: no mips"))?;
        let (width, height) = (u32::from(first.width), u32::from(first.height));
        // The chain as far as every level halves the one before.
        let mip_count = header
            .mips
            .iter()
            .enumerate()
            .take_while(|(i, m)| {
                (u32::from(m.width), u32::from(m.height))
                    == crate::texture::mip_size(width, height, *i as u32)
            })
            .count() as u32;
        let keep_blocks = self.bc_supported && header.meta.swizzle.is_none();
        let format = match block_format(header.meta.format) {
            Some(bc) if keep_blocks => bc,
            _ => TextureFormat::Rgba8,
        };
        let info = TextureInfo {
            format,
            width,
            height,
            mip_count,
        };
        let (first_mip, end) = match request {
            MipRequest::Tail { max_size } => (info.tail_start(max_size), mip_count),
            MipRequest::Range { first, end } => (first, end.min(mip_count)),
        };
        let mut mips = Vec::with_capacity(end.saturating_sub(first_mip) as usize);
        for level in first_mip..end {
            let mip = header
                .read_mip(&bytes, level as usize)
                .map_err(|e| format!("{key} mip {level}: {e}"))?;
            if format == TextureFormat::Rgba8 {
                let mut rgba = a3_paa::decode_rgba8(header.meta.format, &mip)
                    .map_err(|e| format!("{key} mip {level}: {e}"))?;
                if let Some(swizzle) = header.meta.swizzle {
                    swizzle.restore(&mut rgba);
                }
                mips.push(rgba);
            } else {
                mips.push(mip.data);
            }
        }
        Ok(LoadedMips {
            info,
            first_mip,
            mips,
        })
    }
}
