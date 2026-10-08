//! CPU-side texture data and its upload to the GPU.
//!
//! [`TextureData`] is the hand-off shape from texture decoders (the PAA reader): a pixel
//! format, the size of the top mip, and one byte vector per mip level, largest first.

use thiserror::Error;

/// Pixel formats the renderer accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// BC1 / DXT1: 4x4 blocks of 8 bytes, RGB with optional 1-bit alpha.
    Bc1,
    /// BC2 / DXT3: 4x4 blocks of 16 bytes, explicit 4-bit alpha.
    Bc2,
    /// BC3 / DXT5: 4x4 blocks of 16 bytes, interpolated alpha.
    Bc3,
    /// 8-bit RGBA, 4 bytes per pixel in R, G, B, A order.
    Rgba8,
}

impl TextureFormat {
    /// Block edge in pixels (4 for BC formats, 1 for uncompressed).
    pub fn block_dim(self) -> u32 {
        match self {
            TextureFormat::Rgba8 => 1,
            _ => 4,
        }
    }

    /// Bytes per block (per pixel for uncompressed formats).
    pub fn block_bytes(self) -> u32 {
        match self {
            TextureFormat::Bc1 => 8,
            TextureFormat::Bc2 | TextureFormat::Bc3 => 16,
            TextureFormat::Rgba8 => 4,
        }
    }

    pub fn is_compressed(self) -> bool {
        self.block_dim() > 1
    }

    /// The wgpu format, sRGB-decoding for colour data.
    pub fn wgpu_format(self, color_space: ColorSpace) -> wgpu::TextureFormat {
        use wgpu::TextureFormat as F;
        let srgb = color_space == ColorSpace::Srgb;
        match (self, srgb) {
            (TextureFormat::Bc1, true) => F::Bc1RgbaUnormSrgb,
            (TextureFormat::Bc1, false) => F::Bc1RgbaUnorm,
            (TextureFormat::Bc2, true) => F::Bc2RgbaUnormSrgb,
            (TextureFormat::Bc2, false) => F::Bc2RgbaUnorm,
            (TextureFormat::Bc3, true) => F::Bc3RgbaUnormSrgb,
            (TextureFormat::Bc3, false) => F::Bc3RgbaUnorm,
            (TextureFormat::Rgba8, true) => F::Rgba8UnormSrgb,
            (TextureFormat::Rgba8, false) => F::Rgba8Unorm,
        }
    }
}

/// How texel values are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// Colour textures (`_co`, `_ca`): stored sRGB, sampled as linear.
    Srgb,
    /// Data textures (normal maps `_nohq`, `_smdi`, masks): sampled as stored.
    Linear,
}

/// Size in pixels of mip `level` of a `width` x `height` texture (never below 1).
pub fn mip_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// Layout of one mip level in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipLayout {
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
    /// Bytes per row of blocks (of pixels for uncompressed formats).
    pub bytes_per_row: u32,
    /// Rows of blocks.
    pub rows: u32,
}

impl MipLayout {
    pub fn new(format: TextureFormat, width: u32, height: u32, level: u32) -> MipLayout {
        let (w, h) = mip_size(width, height, level);
        let dim = format.block_dim();
        MipLayout {
            width: w,
            height: h,
            bytes_per_row: w.div_ceil(dim) * format.block_bytes(),
            rows: h.div_ceil(dim),
        }
    }

    /// Total bytes of this mip.
    pub fn byte_len(&self) -> usize {
        self.bytes_per_row as usize * self.rows as usize
    }
}

/// A texture problem found before upload.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TextureError {
    #[error("texture has zero size ({width}x{height})")]
    ZeroSize { width: u32, height: u32 },
    #[error("texture has no mip levels")]
    NoMips,
    #[error("{count} mip levels given but a {width}x{height} texture has at most {max}")]
    TooManyMips {
        count: usize,
        max: u32,
        width: u32,
        height: u32,
    },
    #[error("mip {level} has {actual} bytes, expected {expected}")]
    MipSize {
        level: usize,
        expected: usize,
        actual: usize,
    },
    #[error("block-compressed texture size {width}x{height} is not a multiple of 4")]
    UnalignedCompressed { width: u32, height: u32 },
    #[error("the GPU does not support BC texture compression")]
    CompressionUnsupported,
}

/// Decoded texture data ready for upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    pub format: TextureFormat,
    /// Size of mip 0 in pixels.
    pub width: u32,
    pub height: u32,
    /// Mip levels, largest first. A partial chain (not down to 1x1) is allowed.
    pub mips: Vec<Vec<u8>>,
}

impl TextureData {
    /// Check sizes and mip layout.
    pub fn validate(&self) -> Result<(), TextureError> {
        let (width, height) = (self.width, self.height);
        if width == 0 || height == 0 {
            return Err(TextureError::ZeroSize { width, height });
        }
        if self.mips.is_empty() {
            return Err(TextureError::NoMips);
        }
        if self.format.is_compressed() && (width % 4 != 0 || height % 4 != 0) {
            return Err(TextureError::UnalignedCompressed { width, height });
        }
        let max = max_mip_count(width, height);
        if self.mips.len() > max as usize {
            return Err(TextureError::TooManyMips {
                count: self.mips.len(),
                max,
                width,
                height,
            });
        }
        for (level, mip) in self.mips.iter().enumerate() {
            let expected = MipLayout::new(self.format, width, height, level as u32).byte_len();
            if mip.len() != expected {
                return Err(TextureError::MipSize {
                    level,
                    expected,
                    actual: mip.len(),
                });
            }
        }
        Ok(())
    }

    /// A single-mip RGBA8 texture filled with one colour.
    pub fn solid_rgba8(rgba: [u8; 4]) -> TextureData {
        TextureData {
            format: TextureFormat::Rgba8,
            width: 1,
            height: 1,
            mips: vec![rgba.to_vec()],
        }
    }
}

/// Mip levels in a full chain down to 1x1.
pub fn max_mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// A texture on the GPU with a view for sampling.
#[derive(Debug)]
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
}

impl GpuTexture {
    /// Validate and upload `data` with all its mips.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &TextureData,
        color_space: ColorSpace,
        label: Option<&str>,
    ) -> Result<GpuTexture, TextureError> {
        data.validate()?;
        if data.format.is_compressed()
            && !device
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err(TextureError::CompressionUnsupported);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size: wgpu::Extent3d {
                width: data.width,
                height: data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: data.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: data.format.wgpu_format(color_space),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let dim = data.format.block_dim();
        for (level, bytes) in data.mips.iter().enumerate() {
            let layout = MipLayout::new(data.format, data.width, data.height, level as u32);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.bytes_per_row),
                    rows_per_image: Some(layout.rows),
                },
                // Physical size: whole blocks, also for mips smaller than one block.
                wgpu::Extent3d {
                    width: layout.width.div_ceil(dim) * dim,
                    height: layout.height.div_ceil(dim) * dim,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(GpuTexture {
            texture,
            view,
            format: data.format,
            width: data.width,
            height: data.height,
        })
    }
}

/// Encode one BC1 block of two colours (RGB565) and 2-bit indices, row-major from the top-left
/// texel. Useful for procedural test textures.
pub fn bc1_block(color0: u16, color1: u16, indices: [[u8; 4]; 4]) -> [u8; 8] {
    let mut bits = 0u32;
    for (y, row) in indices.iter().enumerate() {
        for (x, &i) in row.iter().enumerate() {
            bits |= u32::from(i & 3) << (2 * (y * 4 + x));
        }
    }
    let mut block = [0u8; 8];
    block[0..2].copy_from_slice(&color0.to_le_bytes());
    block[2..4].copy_from_slice(&color1.to_le_bytes());
    block[4..8].copy_from_slice(&bits.to_le_bytes());
    block
}

/// Pack 8-bit RGB into RGB565.
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bc1(width: u32, height: u32, levels: u32) -> TextureData {
        TextureData {
            format: TextureFormat::Bc1,
            width,
            height,
            mips: (0..levels)
                .map(|l| vec![0; MipLayout::new(TextureFormat::Bc1, width, height, l).byte_len()])
                .collect(),
        }
    }

    #[test]
    fn bc_mip_sizes_round_up_to_whole_blocks() {
        // 256x128 BC1: 64x32 blocks of 8 bytes.
        assert_eq!(
            MipLayout::new(TextureFormat::Bc1, 256, 128, 0).byte_len(),
            16_384
        );
        // Mip 7 of 256x128 is 2x1: still one whole 8-byte block.
        let tiny = MipLayout::new(TextureFormat::Bc1, 256, 128, 7);
        assert_eq!((tiny.width, tiny.height, tiny.byte_len()), (2, 1, 8));
        assert_eq!(MipLayout::new(TextureFormat::Bc3, 4, 4, 0).byte_len(), 16);
        assert_eq!(MipLayout::new(TextureFormat::Rgba8, 3, 2, 0).byte_len(), 24);
    }

    #[test]
    fn full_chain_mip_count() {
        assert_eq!(max_mip_count(256, 128), 9);
        assert_eq!(max_mip_count(1, 1), 1);
        assert_eq!(max_mip_count(2048, 2048), 12);
    }

    #[test]
    fn valid_full_and_partial_chains_pass() {
        assert_eq!(bc1(256, 128, 9).validate(), Ok(()));
        assert_eq!(bc1(256, 128, 3).validate(), Ok(()));
    }

    #[test]
    fn wrong_mip_size_is_reported() {
        let mut t = bc1(64, 64, 2);
        t.mips[1].pop();
        assert_eq!(
            t.validate(),
            Err(TextureError::MipSize {
                level: 1,
                expected: 512,
                actual: 511
            })
        );
    }

    #[test]
    fn invalid_shapes_are_rejected() {
        assert!(matches!(
            bc1(256, 128, 10).validate(),
            Err(TextureError::TooManyMips { .. })
        ));
        assert!(matches!(
            bc1(6, 8, 1).validate(),
            Err(TextureError::UnalignedCompressed { .. })
        ));
        assert_eq!(
            bc1(0, 4, 0).validate(),
            Err(TextureError::ZeroSize {
                width: 0,
                height: 4
            })
        );
        assert_eq!(bc1(4, 4, 0).validate(), Err(TextureError::NoMips));
    }

    #[test]
    fn bc1_block_layout() {
        let mut idx = [[0u8; 4]; 4];
        idx[0][1] = 1;
        idx[3][3] = 3;
        let b = bc1_block(0xF800, 0x001F, idx);
        assert_eq!(&b[0..4], &[0x00, 0xF8, 0x1F, 0x00]);
        assert_eq!(
            u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            (1 << 2) | (3 << 30)
        );
        assert_eq!(rgb565(255, 0, 0), 0xF800);
    }
}
