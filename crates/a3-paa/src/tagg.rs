//! TAGG header chunks: colours, alpha flags, channel swizzle.

/// An 8-bit-per-channel colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (255 = opaque).
    pub a: u8,
}

impl Color {
    /// Opaque white.
    pub const WHITE: Color = Color::rgba(255, 255, 255, 255);

    /// A colour from its channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// A colour from bytes in B, G, R, A order (a little-endian `u32` `0xAARRGGBB`), the order
    /// PAA and `texHeaders.bin` store colours in.
    pub const fn from_bgra(bytes: [u8; 4]) -> Self {
        Self::rgba(bytes[2], bytes[1], bytes[0], bytes[3])
    }

    /// The colour as B, G, R, A bytes.
    pub const fn to_bgra(self) -> [u8; 4] {
        [self.b, self.g, self.r, self.a]
    }
}

/// The value of the `FLAG` TAGG: how the texture uses its alpha channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AlphaFlags(pub u32);

impl AlphaFlags {
    /// Alpha varies smoothly; drawn with alpha blending (`_ca` textures with soft edges).
    pub const INTERPOLATED: u32 = 1;
    /// Alpha is only 0 or 1; drawn with alpha testing.
    pub const BINARY: u32 = 2;

    /// `true` when alpha must be blended.
    pub fn is_interpolated(self) -> bool {
        self.0 & Self::INTERPOLATED != 0
    }

    /// `true` when alpha is a binary transparency mask.
    pub fn is_binary(self) -> bool {
        self.0 & Self::BINARY != 0
    }
}

/// Where one stored channel's value came from, as recorded in the `SWIZ` TAGG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelSource {
    /// The source alpha channel.
    Alpha,
    /// The source red channel.
    Red,
    /// The source green channel.
    Green,
    /// The source blue channel.
    Blue,
    /// One minus the source alpha channel.
    InvertedAlpha,
    /// One minus the source red channel.
    InvertedRed,
    /// One minus the source green channel.
    InvertedGreen,
    /// One minus the source blue channel.
    InvertedBlue,
    /// Constant 1 (255).
    One,
    /// Constant 0.
    Zero,
    /// A code not documented anywhere.
    Unknown(u8),
}

impl ChannelSource {
    /// The source for a stored byte code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Alpha,
            1 => Self::Red,
            2 => Self::Green,
            3 => Self::Blue,
            4 => Self::InvertedAlpha,
            5 => Self::InvertedRed,
            6 => Self::InvertedGreen,
            7 => Self::InvertedBlue,
            8 => Self::One,
            9 => Self::Zero,
            other => Self::Unknown(other),
        }
    }

    /// The stored byte code.
    pub fn code(self) -> u8 {
        match self {
            Self::Alpha => 0,
            Self::Red => 1,
            Self::Green => 2,
            Self::Blue => 3,
            Self::InvertedAlpha => 4,
            Self::InvertedRed => 5,
            Self::InvertedGreen => 6,
            Self::InvertedBlue => 7,
            Self::One => 8,
            Self::Zero => 9,
            Self::Unknown(code) => code,
        }
    }

    /// The source channel index in RGBA order and whether it is inverted; `None` for
    /// constants.
    fn channel(self) -> Option<(usize, bool)> {
        match self {
            Self::Red => Some((0, false)),
            Self::Green => Some((1, false)),
            Self::Blue => Some((2, false)),
            Self::Alpha => Some((3, false)),
            Self::InvertedRed => Some((0, true)),
            Self::InvertedGreen => Some((1, true)),
            Self::InvertedBlue => Some((2, true)),
            Self::InvertedAlpha => Some((3, true)),
            Self::One | Self::Zero | Self::Unknown(_) => None,
        }
    }
}

/// The `SWIZ` TAGG: how TexConvert rearranged the source image's channels before compressing.
///
/// Stored as four bytes in A, R, G, B order, each a [`ChannelSource`] code. For example
/// `_nohq` normal maps store `05 04 02 03`: alpha = 1 - red, red = 1 - alpha (the X component
/// moves to the DXT5 alpha block, which has more precision). The engine's shaders expect the
/// swizzled layout; [`Swizzle::restore`] undoes it for viewing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Swizzle {
    /// Source of the stored alpha channel.
    pub a: ChannelSource,
    /// Source of the stored red channel.
    pub r: ChannelSource,
    /// Source of the stored green channel.
    pub g: ChannelSource,
    /// Source of the stored blue channel.
    pub b: ChannelSource,
}

impl Swizzle {
    /// No rearrangement.
    pub const IDENTITY: Swizzle = Swizzle {
        a: ChannelSource::Alpha,
        r: ChannelSource::Red,
        g: ChannelSource::Green,
        b: ChannelSource::Blue,
    };

    /// Parses the four stored bytes (A, R, G, B order).
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        Self {
            a: ChannelSource::from_code(bytes[0]),
            r: ChannelSource::from_code(bytes[1]),
            g: ChannelSource::from_code(bytes[2]),
            b: ChannelSource::from_code(bytes[3]),
        }
    }

    /// The four stored bytes (A, R, G, B order).
    pub fn to_bytes(self) -> [u8; 4] {
        [self.a.code(), self.r.code(), self.g.code(), self.b.code()]
    }

    /// Undoes the swizzle on RGBA8 pixels in place, as far as it is reversible: every source
    /// channel that some stored channel came from gets its value back; channels that were
    /// replaced by a constant keep their stored value.
    pub fn restore(self, rgba: &mut [u8]) {
        let stored_sources = [self.r, self.g, self.b, self.a];
        for pixel in rgba.chunks_exact_mut(4) {
            let stored = [pixel[0], pixel[1], pixel[2], pixel[3]];
            for (slot, source) in stored_sources.iter().enumerate() {
                if let Some((channel, inverted)) = source.channel() {
                    let value = stored[slot];
                    pixel[channel] = if inverted { 255 - value } else { value };
                }
            }
        }
    }
}
