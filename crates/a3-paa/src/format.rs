//! Pixel formats and their PAA type tags.

/// The pixel format of a texture's mipmaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PixelFormat {
    /// BC1: 8 bytes per 4x4 block, 1-bit alpha.
    Dxt1,
    /// BC2 with premultiplied alpha (never seen in shipped data).
    Dxt2,
    /// BC2: explicit 4-bit alpha.
    Dxt3,
    /// BC3 with premultiplied alpha (never seen in shipped data).
    Dxt4,
    /// BC3: interpolated alpha.
    #[default]
    Dxt5,
    /// 16-bit, 4 bits per channel; `u16` bits `AAAA RRRR GGGG BBBB`.
    Argb4444,
    /// 16-bit; `u16` bits `A RRRRR GGGGG BBBBB`.
    Argb1555,
    /// 32-bit; bytes B, G, R, A.
    Argb8888,
    /// 16-bit grey plus alpha; bytes I (intensity), A.
    Ai88,
}

impl PixelFormat {
    /// Every format, in the order of the engine's internal format enum (see
    /// [`PixelFormat::engine_index`]).
    pub const ALL: [PixelFormat; 9] = [
        PixelFormat::Ai88,
        PixelFormat::Argb1555,
        PixelFormat::Argb4444,
        PixelFormat::Argb8888,
        PixelFormat::Dxt1,
        PixelFormat::Dxt2,
        PixelFormat::Dxt3,
        PixelFormat::Dxt4,
        PixelFormat::Dxt5,
    ];

    /// The format for the two-byte (little-endian `u16`) type tag at the start of a PAA.
    pub fn from_tag(tag: u16) -> Option<Self> {
        Some(match tag {
            0xFF01 => Self::Dxt1,
            0xFF02 => Self::Dxt2,
            0xFF03 => Self::Dxt3,
            0xFF04 => Self::Dxt4,
            0xFF05 => Self::Dxt5,
            0x4444 => Self::Argb4444,
            0x1555 => Self::Argb1555,
            0x8888 => Self::Argb8888,
            0x8080 => Self::Ai88,
            _ => return None,
        })
    }

    /// The PAA type tag.
    pub fn tag(self) -> u16 {
        match self {
            Self::Dxt1 => 0xFF01,
            Self::Dxt2 => 0xFF02,
            Self::Dxt3 => 0xFF03,
            Self::Dxt4 => 0xFF04,
            Self::Dxt5 => 0xFF05,
            Self::Argb4444 => 0x4444,
            Self::Argb1555 => 0x1555,
            Self::Argb8888 => 0x8888,
            Self::Ai88 => 0x8080,
        }
    }

    /// The value of the engine's pixel format enum, as stored in `texHeaders.bin`:
    /// 0 P8 (palettised, unused), 1 AI88, 2 RGB565 (unused), 3 ARGB1555, 4 ARGB4444,
    /// 5 ARGB8888, 6..=10 DXT1..DXT5.
    pub fn engine_index(self) -> u32 {
        match self {
            Self::Ai88 => 1,
            Self::Argb1555 => 3,
            Self::Argb4444 => 4,
            Self::Argb8888 => 5,
            Self::Dxt1 => 6,
            Self::Dxt2 => 7,
            Self::Dxt3 => 8,
            Self::Dxt4 => 9,
            Self::Dxt5 => 10,
        }
    }

    /// The format for an engine pixel format enum value (see [`PixelFormat::engine_index`]).
    pub fn from_engine_index(index: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.engine_index() == index)
    }

    /// `true` for the block-compressed DXT formats.
    pub fn is_dxt(self) -> bool {
        matches!(
            self,
            Self::Dxt1 | Self::Dxt2 | Self::Dxt3 | Self::Dxt4 | Self::Dxt5
        )
    }

    /// Bytes per 4x4 block (DXT) or per pixel (others).
    fn unit_size(self) -> usize {
        match self {
            Self::Dxt1 => 8,
            Self::Dxt2 | Self::Dxt3 | Self::Dxt4 | Self::Dxt5 => 16,
            Self::Argb4444 | Self::Argb1555 | Self::Ai88 => 2,
            Self::Argb8888 => 4,
        }
    }

    /// The uncompressed size in bytes of one `width` x `height` mipmap. DXT sizes round up to
    /// whole 4x4 blocks.
    pub fn data_len(self, width: u16, height: u16) -> usize {
        let (w, h) = (usize::from(width), usize::from(height));
        if self.is_dxt() {
            w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * self.unit_size()
        } else {
            w * h * self.unit_size()
        }
    }
}
