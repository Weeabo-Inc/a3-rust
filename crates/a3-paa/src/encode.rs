//! Building textures from RGBA8 images.

use crate::error::malformed;
use crate::{AlphaFlags, Color, Compression, Error, Mip, PixelFormat, Result, Texture};

/// How [`encode_rgba8`] builds a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Target pixel format. DXT2 and DXT4 are not supported.
    pub format: PixelFormat,
    /// Generate the mipmap chain, halving until the smaller side reaches 4 (DXT) or 1 (other
    /// formats), as TexConvert does; otherwise only the full-size level.
    pub mipmaps: bool,
    /// LZO-compress DXT levels when that makes them smaller. Other formats are always LZSS.
    pub compress: bool,
}

impl EncodeOptions {
    /// Mipmaps on, compression on.
    pub fn new(format: PixelFormat) -> Self {
        Self {
            format,
            mipmaps: true,
            compress: true,
        }
    }
}

/// Builds a texture from `width` x `height` RGBA8 pixels (rows top to bottom).
///
/// Sets `AVGC` to the image's average colour, `MAXC` to white (as TexConvert does) and `FLAG`
/// from the alpha channel: absent when fully opaque, [`AlphaFlags::BINARY`] when every alpha is
/// 0 or 255, [`AlphaFlags::INTERPOLATED`] otherwise. Mipmaps are 2x2 box-filtered.
pub fn encode_rgba8(
    width: u16,
    height: u16,
    rgba: &[u8],
    options: &EncodeOptions,
) -> Result<Texture> {
    let (w, h) = (usize::from(width), usize::from(height));
    if width == 0 || height == 0 || width >= 0x8000 || height >= 0x8000 || rgba.len() != w * h * 4 {
        return malformed(
            "image",
            format!("{width}x{height} image with {} bytes of RGBA", rgba.len()),
        );
    }
    let format = options.format;
    if matches!(format, PixelFormat::Dxt2 | PixelFormat::Dxt4) {
        return Err(Error::UnsupportedFormat(format));
    }

    let mut levels = vec![(width, height, rgba.to_vec())];
    if options.mipmaps {
        let min = if format.is_dxt() { 4 } else { 1 };
        loop {
            let (lw, lh, pixels) = levels.last().expect("at least one level");
            if (*lw).min(*lh) <= min {
                break;
            }
            let next = downsample(*lw, *lh, pixels);
            levels.push(next);
        }
    }

    let mips = levels
        .iter()
        .map(|(lw, lh, pixels)| {
            let data = encode_level(format, *lw, *lh, pixels);
            let compression = choose_compression(format, &data, options.compress);
            Mip {
                width: *lw,
                height: *lh,
                data,
                compression,
            }
        })
        .collect();

    Ok(Texture {
        format,
        average_color: Some(average(rgba)),
        max_color: Some(Color::WHITE),
        flags: alpha_flags(rgba),
        mips,
        ..Texture::default()
    })
}

fn choose_compression(format: PixelFormat, data: &[u8], compress: bool) -> Compression {
    if !format.is_dxt() {
        // The engine reads non-DXT levels only as LZSS.
        return Compression::Lzss;
    }
    if !compress {
        return Compression::None;
    }
    match lzokay_native::compress(data) {
        Ok(packed) if packed.len() < data.len() => Compression::Lzo,
        _ => Compression::None,
    }
}

fn average(rgba: &[u8]) -> Color {
    let mut sum = [0u64; 4];
    for px in rgba.chunks_exact(4) {
        for (s, &v) in sum.iter_mut().zip(px) {
            *s += u64::from(v);
        }
    }
    let n = (rgba.len() / 4).max(1) as u64;
    let [r, g, b, a] = sum.map(|s| ((s + n / 2) / n) as u8);
    Color::rgba(r, g, b, a)
}

fn alpha_flags(rgba: &[u8]) -> Option<AlphaFlags> {
    let alphas = || rgba.chunks_exact(4).map(|px| px[3]);
    if alphas().all(|a| a == 255) {
        None
    } else if alphas().all(|a| a == 0 || a == 255) {
        Some(AlphaFlags(AlphaFlags::BINARY))
    } else {
        Some(AlphaFlags(AlphaFlags::INTERPOLATED))
    }
}

/// Halves each dimension (not below 1) with a box filter.
fn downsample(width: u16, height: u16, rgba: &[u8]) -> (u16, u16, Vec<u8>) {
    let (w, h) = (usize::from(width), usize::from(height));
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let xs = [(2 * x).min(w - 1), (2 * x + 1).min(w - 1)];
            let ys = [(2 * y).min(h - 1), (2 * y + 1).min(h - 1)];
            for c in 0..4 {
                let sum: u32 = ys
                    .iter()
                    .flat_map(|&sy| xs.iter().map(move |&sx| (sy * w + sx) * 4 + c))
                    .map(|i| u32::from(rgba[i]))
                    .sum();
                out[(y * nw + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (nw as u16, nh as u16, out)
}

/// Scales an 8-bit value to `max` (rounded).
fn quantise(value: u8, max: u32) -> u16 {
    ((u32::from(value) * max + 127) / 255) as u16
}

fn encode_level(format: PixelFormat, width: u16, height: u16, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (usize::from(width), usize::from(height));
    let block = |f: texpresso::Format| {
        let mut out = vec![0u8; f.compressed_size(w, h)];
        f.compress(rgba, w, h, texpresso::Params::default(), &mut out);
        out
    };
    let words = |f: &dyn Fn(&[u8]) -> u16| -> Vec<u8> {
        rgba.chunks_exact(4)
            .flat_map(|px| f(px).to_le_bytes())
            .collect()
    };
    match format {
        PixelFormat::Dxt1 => block(texpresso::Format::Bc1),
        PixelFormat::Dxt3 => block(texpresso::Format::Bc2),
        PixelFormat::Dxt5 => block(texpresso::Format::Bc3),
        PixelFormat::Dxt2 | PixelFormat::Dxt4 => unreachable!("rejected by encode_rgba8"),
        PixelFormat::Argb8888 => rgba
            .chunks_exact(4)
            .flat_map(|px| [px[2], px[1], px[0], px[3]])
            .collect(),
        PixelFormat::Argb4444 => words(&|px| {
            quantise(px[3], 15) << 12
                | quantise(px[0], 15) << 8
                | quantise(px[1], 15) << 4
                | quantise(px[2], 15)
        }),
        PixelFormat::Argb1555 => words(&|px| {
            u16::from(px[3] >= 128) << 15
                | quantise(px[0], 31) << 10
                | quantise(px[1], 31) << 5
                | quantise(px[2], 31)
        }),
        PixelFormat::Ai88 => rgba
            .chunks_exact(4)
            .flat_map(|px| {
                // Rec. 601 luma.
                let luma = (299 * u32::from(px[0])
                    + 587 * u32::from(px[1])
                    + 114 * u32::from(px[2])
                    + 500)
                    / 1000;
                [luma as u8, px[3]]
            })
            .collect(),
    }
}
