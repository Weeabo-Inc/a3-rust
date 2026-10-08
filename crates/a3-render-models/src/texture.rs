//! Turning PAA files and procedural texture strings into upload-ready [`TextureData`].
//!
//! DXT data stays block-compressed when the GPU samples BC formats; other formats are decoded
//! to RGBA8. Only the mipmaps up to [`TextureOptions::max_size`] are decompressed. Normal maps
//! reach the shaders in the `_nohq` layout (X in `1 - alpha`, Y in green) whatever their
//! source, so one shader path reads them all.

use a3_paa::{
    AlphaFlags, ChannelSource, Mip, PaaHeader, PixelFormat, Procedural, Swizzle, Texture,
    decode_rgba8,
};
use a3_render::{TextureData, TextureFormat};

/// Why a texture could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum TextureLoadError {
    #[error("texture file not found: {0}")]
    NotFound(String),
    #[error("texture {path}: {source}")]
    Paa { path: String, source: a3_paa::Error },
    #[error("texture {0} has no mipmaps")]
    Empty(String),
}

/// Limits and GPU capabilities for texture loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureOptions {
    /// Largest edge of the top mip uploaded; bigger textures start at a smaller mip.
    pub max_size: u32,
    /// Whether the GPU samples BC1-3 natively.
    pub bc_supported: bool,
}

impl Default for TextureOptions {
    fn default() -> Self {
        TextureOptions {
            max_size: 2048,
            bc_supported: true,
        }
    }
}

/// A decoded texture and how it uses alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedTexture {
    pub data: TextureData,
    /// The `FLAG` TAGG: interpolated (blend) or binary (test) alpha.
    pub alpha: Option<AlphaFlags>,
}

/// The `_nohq` swizzle: stored alpha = 1 - red, stored red = 1 - alpha.
const NOHQ: Swizzle = Swizzle {
    a: ChannelSource::InvertedRed,
    r: ChannelSource::InvertedAlpha,
    g: ChannelSource::Green,
    b: ChannelSource::Blue,
};

/// Reads only the header of a PAA: its alpha flags, without decompressing anything.
pub fn texture_alpha(path: &str, bytes: Option<&[u8]>) -> Option<AlphaFlags> {
    if Procedural::is_procedural(path) {
        return Procedural::parse(path).ok()?.generate().ok()?.flags;
    }
    PaaHeader::read(bytes?).ok()?.meta.flags
}

/// Decodes the texture at `path` (file contents in `bytes`, `None` for procedural textures or
/// files that were not found). `normal_map` converts plain normal maps to the `_nohq` layout.
pub fn decode_texture(
    path: &str,
    bytes: Option<&[u8]>,
    normal_map: bool,
    options: TextureOptions,
) -> Result<LoadedTexture, TextureLoadError> {
    let paa_error = |source| TextureLoadError::Paa {
        path: path.to_owned(),
        source,
    };
    let texture = if Procedural::is_procedural(path) {
        Procedural::parse(path)
            .and_then(|p| p.generate())
            .map_err(paa_error)?
    } else {
        let bytes = bytes.ok_or_else(|| TextureLoadError::NotFound(path.to_owned()))?;
        let header = PaaHeader::read(bytes).map_err(paa_error)?;
        let first = header
            .mips
            .iter()
            .position(|m| u32::from(m.width.max(m.height)) <= options.max_size)
            .unwrap_or(header.mips.len().saturating_sub(1));
        let mips = (first..header.mips.len())
            .map(|i| header.read_mip(bytes, i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(paa_error)?;
        Texture {
            mips,
            ..header.meta
        }
    };
    if texture.mips.is_empty() {
        return Err(TextureLoadError::Empty(path.to_owned()));
    }
    let needs_swizzle = normal_map && texture.swizzle != Some(NOHQ);
    let bc = match texture.format {
        PixelFormat::Dxt1 => Some(TextureFormat::Bc1),
        PixelFormat::Dxt2 | PixelFormat::Dxt3 => Some(TextureFormat::Bc2),
        PixelFormat::Dxt4 | PixelFormat::Dxt5 => Some(TextureFormat::Bc3),
        _ => None,
    }
    .filter(|_| options.bc_supported && !needs_swizzle);
    let (width, height) = (
        u32::from(texture.mips[0].width),
        u32::from(texture.mips[0].height),
    );
    let data = match bc {
        Some(format) => TextureData {
            format,
            width,
            height,
            mips: texture.mips.into_iter().map(|m| m.data).collect(),
        },
        None => {
            let mut mips = texture
                .mips
                .iter()
                .map(|m: &Mip| decode_rgba8(texture.format, m))
                .collect::<Result<Vec<_>, _>>()
                .map_err(paa_error)?;
            if needs_swizzle {
                for mip in &mut mips {
                    to_nohq_layout(mip);
                }
            }
            TextureData {
                format: TextureFormat::Rgba8,
                width,
                height,
                mips,
            }
        }
    };
    Ok(LoadedTexture {
        data,
        alpha: texture.flags,
    })
}

/// Plain RGBA normal map (X red, Y green, Z blue) to the `_nohq` layout.
fn to_nohq_layout(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let (r, a) = (px[0], px[3]);
        px[3] = 255 - r;
        px[0] = 255 - a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_paa::{EncodeOptions, PixelFormat, encode_rgba8};

    fn paa(format: PixelFormat, size: u16, rgba: [u8; 4]) -> Vec<u8> {
        let pixels = rgba.repeat(usize::from(size) * usize::from(size));
        encode_rgba8(size, size, &pixels, &EncodeOptions::new(format))
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    const BC: TextureOptions = TextureOptions {
        max_size: 4096,
        bc_supported: true,
    };

    #[test]
    fn dxt_textures_stay_block_compressed() {
        let t = decode_texture(
            "x_co.paa",
            Some(&paa(PixelFormat::Dxt5, 64, [255; 4])),
            false,
            BC,
        )
        .unwrap();
        assert_eq!(t.data.format, TextureFormat::Bc3);
        assert_eq!((t.data.width, t.data.height), (64, 64));
        // 64, 32, 16, 8, 4.
        assert_eq!(t.data.mips.len(), 5);
        assert_eq!(t.data.validate(), Ok(()));
    }

    #[test]
    fn large_textures_start_at_the_first_mip_within_the_size_limit() {
        let bytes = paa(PixelFormat::Dxt1, 256, [10, 20, 30, 255]);
        let small = TextureOptions { max_size: 64, ..BC };
        let t = decode_texture("x_co.paa", Some(&bytes), false, small).unwrap();
        assert_eq!((t.data.width, t.data.height), (64, 64));
        assert_eq!(t.data.format, TextureFormat::Bc1);
        assert_eq!(t.data.validate(), Ok(()));
    }

    #[test]
    fn without_bc_support_dxt_is_decoded_to_rgba8() {
        let no_bc = TextureOptions {
            bc_supported: false,
            ..BC
        };
        let t = decode_texture(
            "x_co.paa",
            Some(&paa(PixelFormat::Dxt1, 16, [255, 0, 0, 255])),
            false,
            no_bc,
        )
        .unwrap();
        assert_eq!(t.data.format, TextureFormat::Rgba8);
        assert_eq!(&t.data.mips[0][..4], &[255, 0, 0, 255]);
        assert_eq!(t.data.validate(), Ok(()));
    }

    #[test]
    fn procedural_colours_are_generated() {
        let t = decode_texture("#(argb,8,8,3)color(1,0,0,0.5,CO)", None, false, BC).unwrap();
        assert_eq!(t.data.format, TextureFormat::Rgba8);
        assert_eq!(&t.data.mips[0][..4], &[255, 0, 0, 128]);
        assert!(t.alpha.is_some_and(|a| a.is_interpolated()));
    }

    #[test]
    fn unswizzled_normal_maps_are_brought_into_the_nohq_layout() {
        // A flat normal (0.5, 0.5, 1): `_nohq` stores X as 1 - alpha.
        let t = decode_texture("#(argb,8,8,3)color(0.5,0.5,1,1,NOHQ)", None, true, BC).unwrap();
        let texel = &t.data.mips[0][..4];
        assert_eq!(texel[1], 128); // Y in green
        assert_eq!(255 - texel[3], 128); // X in 1 - alpha
    }

    #[test]
    fn missing_files_and_bad_data_are_errors() {
        assert!(decode_texture("x_co.paa", None, false, BC).is_err());
        assert!(decode_texture("x_co.paa", Some(b"junk"), false, BC).is_err());
        assert!(decode_texture("#(argb,8,8,3)nosuchgenerator(1)", None, false, BC).is_err());
    }
}
