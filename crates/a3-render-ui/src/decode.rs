//! Turning a draw list texture (a PAA from the VFS, or a procedural `#(...)` string) into an
//! upload-ready [`TextureData`].
//!
//! The sibling of `a3-render-models::texture::decode_texture`, with a UI-specific rule: the
//! **whole mip chain is kept**. Glyph quads give their texture coordinates in texels of the
//! page image, so the uploaded top mip must be the image's own mip 0.
//!
//! DXT1-5 stay block-compressed when the GPU samples BC formats; every other format (AI88,
//! ARGB4444/1555/8888) is decoded to RGBA8.

use a3_paa::{PaaHeader, PixelFormat, Procedural, Texture, decode_rgba8};
use a3_render::{TextureData, TextureFormat};

/// Why a UI texture could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum UiTextureError {
    #[error("texture not found: {0}")]
    NotFound(String),
    #[error("texture {path}: {source}")]
    Paa { path: String, source: a3_paa::Error },
    #[error("texture {0} has no mipmaps")]
    Empty(String),
}

/// Decodes the texture at `path`. `bytes` are the file's contents, or `None` for procedural
/// textures. `bc_supported` keeps DXT data block-compressed instead of decoding it.
pub fn decode_ui_texture(
    path: &str,
    bytes: Option<&[u8]>,
    bc_supported: bool,
) -> Result<TextureData, UiTextureError> {
    let paa_error = |source| UiTextureError::Paa {
        path: path.to_owned(),
        source,
    };
    let texture = if Procedural::is_procedural(path) {
        Procedural::parse(path)
            .and_then(|p| p.generate())
            .map_err(paa_error)?
    } else {
        let bytes = bytes.ok_or_else(|| UiTextureError::NotFound(path.to_owned()))?;
        let header = PaaHeader::read(bytes).map_err(paa_error)?;
        let mips = (0..header.mips.len())
            .map(|i| header.read_mip(bytes, i))
            .collect::<Result<Vec<_>, _>>()
            .map_err(paa_error)?;
        Texture {
            mips,
            ..header.meta
        }
    };
    if texture.mips.is_empty() {
        return Err(UiTextureError::Empty(path.to_owned()));
    }
    let format = match texture.format {
        PixelFormat::Dxt1 if bc_supported => Some(TextureFormat::Bc1),
        PixelFormat::Dxt2 | PixelFormat::Dxt3 if bc_supported => Some(TextureFormat::Bc2),
        PixelFormat::Dxt4 | PixelFormat::Dxt5 if bc_supported => Some(TextureFormat::Bc3),
        _ => None,
    };
    let (width, height) = (
        u32::from(texture.mips[0].width),
        u32::from(texture.mips[0].height),
    );
    let mips = match format {
        Some(_) => texture.mips.into_iter().map(|m| m.data).collect(),
        None => texture
            .mips
            .iter()
            .map(|m| decode_rgba8(texture.format, m))
            .collect::<Result<Vec<_>, _>>()
            .map_err(paa_error)?,
    };
    Ok(TextureData {
        format: format.unwrap_or(TextureFormat::Rgba8),
        width,
        height,
        mips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_paa::{EncodeOptions, encode_rgba8};

    fn paa(format: PixelFormat, size: u16, rgba: [u8; 4]) -> Vec<u8> {
        let pixels = rgba.repeat(usize::from(size) * usize::from(size));
        encode_rgba8(size, size, &pixels, &EncodeOptions::new(format))
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    #[test]
    fn dxt_textures_stay_block_compressed_with_their_whole_mip_chain() {
        let data =
            decode_ui_texture("x.paa", Some(&paa(PixelFormat::Dxt5, 64, [255; 4])), true).unwrap();
        assert_eq!(data.format, TextureFormat::Bc3);
        assert_eq!((data.width, data.height), (64, 64));
        // 64, 32, 16, 8, 4: texel coordinates of the page image stay valid.
        assert_eq!(data.mips.len(), 5);
        assert_eq!(data.validate(), Ok(()));
    }

    #[test]
    fn without_bc_support_dxt_is_decoded_to_rgba8() {
        let data = decode_ui_texture(
            "x.paa",
            Some(&paa(PixelFormat::Dxt1, 16, [255, 0, 0, 255])),
            false,
        )
        .unwrap();
        assert_eq!(data.format, TextureFormat::Rgba8);
        assert_eq!(&data.mips[0][..4], &[255, 0, 0, 255]);
        assert_eq!(data.validate(), Ok(()));
    }

    #[test]
    fn uncompressed_formats_are_decoded() {
        // AI88 stores white with the alpha as coverage, what a glyph page needs.
        let data = decode_ui_texture(
            "page.paa",
            Some(&paa(PixelFormat::Ai88, 8, [255, 255, 255, 128])),
            true,
        )
        .unwrap();
        assert_eq!(data.format, TextureFormat::Rgba8);
        let texel = &data.mips[0][..4];
        assert_eq!(texel[3], 128, "{texel:?}");
        assert_eq!(data.validate(), Ok(()));
    }

    #[test]
    fn procedural_colours_are_generated() {
        let data = decode_ui_texture("#(argb,8,8,3)color(1,0,0,0.5,CO)", None, true).unwrap();
        assert_eq!(data.format, TextureFormat::Rgba8);
        // A solid procedural colour generates a single texel; the 8x8 is the requested size.
        assert_eq!((data.width, data.height), (1, 1));
        assert_eq!(&data.mips[0][..4], &[255, 0, 0, 128]);
        assert_eq!(data.validate(), Ok(()));
    }

    #[test]
    fn missing_files_and_bad_data_are_errors() {
        assert!(matches!(
            decode_ui_texture("x.paa", None, true),
            Err(UiTextureError::NotFound(_))
        ));
        assert!(matches!(
            decode_ui_texture("x.paa", Some(b"junk"), true),
            Err(UiTextureError::Paa { .. })
        ));
        assert!(decode_ui_texture("#(argb,8,8,3)nosuchgenerator(1)", None, true).is_err());
    }
}
