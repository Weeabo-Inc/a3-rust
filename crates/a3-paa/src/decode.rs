//! Decoding mipmaps to RGBA8.

use crate::error::malformed;
use crate::{Mip, PixelFormat, Result};

/// Expands an `n`-bit channel value to 8 bits by bit replication.
fn expand(value: u16, bits: u32) -> u8 {
    let v = u32::from(value);
    match bits {
        1 => (v * 255) as u8,
        4 => (v * 17) as u8,
        5 => ((v << 3) | (v >> 2)) as u8,
        _ => unreachable!("only 1-, 4- and 5-bit channels exist"),
    }
}

/// Decodes `mip` (in `format`) to tightly packed RGBA8 pixels, rows top to bottom.
///
/// DXT2 and DXT4 are decoded like DXT3 and DXT5; their premultiplied colour is not divided
/// back out. Channel swizzles (see [`crate::Swizzle`]) are not undone.
pub fn decode_rgba8(format: PixelFormat, mip: &Mip) -> Result<Vec<u8>> {
    let (width, height) = (usize::from(mip.width), usize::from(mip.height));
    let expected = format.data_len(mip.width, mip.height);
    if mip.data.len() < expected {
        return malformed(
            "PAA",
            format!(
                "{width}x{height} {format:?} mipmap has {} bytes, needs {expected}",
                mip.data.len()
            ),
        );
    }
    let mut out = vec![0u8; width * height * 4];
    let words = || {
        mip.data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
    };
    match format {
        PixelFormat::Dxt1 => texpresso::Format::Bc1.decompress(&mip.data, width, height, &mut out),
        PixelFormat::Dxt2 | PixelFormat::Dxt3 => {
            texpresso::Format::Bc2.decompress(&mip.data, width, height, &mut out)
        }
        PixelFormat::Dxt4 | PixelFormat::Dxt5 => {
            texpresso::Format::Bc3.decompress(&mip.data, width, height, &mut out)
        }
        PixelFormat::Argb8888 => {
            for (px, bgra) in out.chunks_exact_mut(4).zip(mip.data.chunks_exact(4)) {
                px.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
            }
        }
        PixelFormat::Argb4444 => {
            for (px, w) in out.chunks_exact_mut(4).zip(words()) {
                px.copy_from_slice(&[
                    expand((w >> 8) & 0xF, 4),
                    expand((w >> 4) & 0xF, 4),
                    expand(w & 0xF, 4),
                    expand(w >> 12, 4),
                ]);
            }
        }
        PixelFormat::Argb1555 => {
            for (px, w) in out.chunks_exact_mut(4).zip(words()) {
                px.copy_from_slice(&[
                    expand((w >> 10) & 0x1F, 5),
                    expand((w >> 5) & 0x1F, 5),
                    expand(w & 0x1F, 5),
                    expand(w >> 15, 1),
                ]);
            }
        }
        PixelFormat::Ai88 => {
            for (px, ia) in out.chunks_exact_mut(4).zip(mip.data.chunks_exact(2)) {
                px.copy_from_slice(&[ia[0], ia[0], ia[0], ia[1]]);
            }
        }
    }
    Ok(out)
}
