//! `texHeaders.bin`: the per-PBO cache of texture headers.
//!
//! Binarize writes one into every PBO that holds textures, so the engine can learn each
//! texture's size, format, mipmap offsets and average colour without opening the PAA. Layout
//! (all little-endian):
//!
//! ```text
//! "0DHT"  u32 version (1)  u32 count  TexHeader[count]
//!
//! TexHeader:
//!   u32 palette_count (1)     u32 palette_pointer (0)
//!   f32 average[4]            linear R, G, B, A in 0..1
//!   u8  average_bgra[4]       (zero in shipped data)
//!   u8  max_bgra[4]
//!   u32 clamp_flags (0)       u32 transparent_color (0xFFFFFFFF)
//!   u8  has_max_color  u8 is_alpha  u8 is_transparent  u8 is_alpha_non_opaque
//!   u32 mip_count             u32 pixel_format (engine enum, see PixelFormat::engine_index)
//!   u8  little_endian (1)     u8 is_paa (1 for .paa, 0 for .pac)
//!   cstr path                 relative to the PBO prefix
//!   u32 texture_type          TextureType enum (from the file-name suffix)
//!   u32 mip_count             again
//!   mip[mip_count]:  u16 width  u16 height  u16 zero  u8 pixel_format  u8 3  u32 file_offset
//!   u32 file_size             of the PAA
//! ```

use crate::cursor::Cursor;
use crate::error::malformed;
use crate::{Color, PixelFormat, Result, TextureType};

const MAGIC: &[u8; 4] = b"0DHT";

/// A parsed `texHeaders.bin`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TexHeaders {
    /// Format version (1 in every shipped file).
    pub version: u32,
    /// One header per texture in the PBO.
    pub textures: Vec<TexHeader>,
}

/// The cached header of one texture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TexHeader {
    /// Path of the texture relative to the PBO prefix (as stored, usually lower case).
    pub path: String,
    /// Pixel format; `None` for an engine format this crate does not know (P8, RGB565).
    pub format: Option<PixelFormat>,
    /// The raw engine pixel format value.
    pub format_index: u32,
    /// Average colour as linear floats, R, G, B, A.
    pub average: [f32; 4],
    /// Average colour as bytes (zero in shipped data).
    pub average_color: Color,
    /// Maximum colour (the PAA's `MAXC`).
    pub max_color: Color,
    /// `true` when the PAA has a `MAXC` TAGG.
    pub has_max_color: bool,
    /// `true` when alpha must be blended (`FLAG` & 1).
    pub is_alpha: bool,
    /// `true` when alpha is a binary mask (`FLAG` & 2).
    pub is_transparent: bool,
    /// `true` when some texel is not fully opaque.
    pub is_alpha_non_opaque: bool,
    /// `true` for `.paa`, `false` for `.pac`.
    pub is_paa: bool,
    /// The raw texture type value (see [`TexHeader::texture_type`]).
    pub texture_type_index: u32,
    /// The mipmap table.
    pub mips: Vec<TexHeaderMip>,
    /// Size of the PAA file in bytes.
    pub file_size: u32,
    /// Fields that hold the same value in every shipped entry, kept for byte-exact writing:
    /// palette count, palette pointer, clamp flags, transparent colour, little-endian flag.
    pub constants: TexHeaderConstants,
}

/// The fields of a [`TexHeader`] that are constant in shipped data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexHeaderConstants {
    /// Palette count (1).
    pub palette_count: u32,
    /// Palette pointer (0).
    pub palette_pointer: u32,
    /// Clamp flags (0).
    pub clamp_flags: u32,
    /// Transparent colour (`0xFFFFFFFF`).
    pub transparent_color: u32,
    /// Little-endian flag (1).
    pub little_endian: u8,
}

impl Default for TexHeaderConstants {
    fn default() -> Self {
        Self {
            palette_count: 1,
            palette_pointer: 0,
            clamp_flags: 0,
            transparent_color: 0xFFFF_FFFF,
            little_endian: 1,
        }
    }
}

/// One mipmap in a [`TexHeader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TexHeaderMip {
    /// Width in pixels (without the LZO flag).
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// The engine pixel format value (same as the texture's).
    pub format_index: u8,
    /// Unknown byte, 3 in shipped data.
    pub unknown: u8,
    /// Offset of the mipmap's header in the PAA file.
    pub offset: u32,
}

impl TexHeaders {
    /// Parses a `texHeaders.bin` file.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut cur = Cursor::new(data);
        if cur.array::<4>("magic")? != *MAGIC {
            return malformed("texHeaders.bin", "missing 0DHT magic");
        }
        let version = cur.u32("version")?;
        if version != 1 {
            return malformed("texHeaders.bin", format!("unsupported version {version}"));
        }
        let count = cur.u32("texture count")? as usize;
        let mut textures = Vec::with_capacity(count.min(cur.remaining() / 64));
        for _ in 0..count {
            textures.push(read_entry(&mut cur)?);
        }
        if cur.remaining() != 0 {
            return malformed(
                "texHeaders.bin",
                format!("{} bytes after the last entry", cur.remaining()),
            );
        }
        Ok(Self { version, textures })
    }

    /// Serialises the file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(self.textures.len() as u32).to_le_bytes());
        for t in &self.textures {
            write_entry(&mut out, t);
        }
        out
    }

    /// The header for `path` (relative to the PBO prefix), compared case-insensitively with
    /// `/` and `\` treated alike.
    pub fn find(&self, path: &str) -> Option<&TexHeader> {
        let norm = |s: &str| s.replace('/', "\\").to_ascii_lowercase();
        let wanted = norm(path);
        self.textures.iter().find(|t| norm(&t.path) == wanted)
    }
}

impl TexHeader {
    /// What the texture is used for, as Binarize decided from its suffix; `None` for an
    /// unknown value.
    pub fn texture_type(&self) -> Option<TextureType> {
        TextureType::from_index(self.texture_type_index)
    }
}

fn read_entry(cur: &mut Cursor<'_>) -> Result<TexHeader> {
    let palette_count = cur.u32("palette count")?;
    let palette_pointer = cur.u32("palette pointer")?;
    let mut average = [0f32; 4];
    for v in &mut average {
        *v = cur.f32("average colour")?;
    }
    let average_color = Color::from_bgra(cur.array("average colour")?);
    let max_color = Color::from_bgra(cur.array("max colour")?);
    let clamp_flags = cur.u32("clamp flags")?;
    let transparent_color = cur.u32("transparent colour")?;
    let [has_max_color, is_alpha, is_transparent, is_alpha_non_opaque] =
        cur.array("alpha flags")?;
    let mip_count = cur.u32("mipmap count")?;
    let format_index = cur.u32("pixel format")?;
    let little_endian = cur.u8("endianness")?;
    let is_paa = cur.u8("is_paa")?;
    let path = cur.cstr("path")?;
    let texture_type_index = cur.u32("texture type")?;
    let mip_count2 = cur.u32("mipmap count")?;
    if mip_count != mip_count2 {
        return malformed(
            "texHeaders.bin",
            format!("{path}: mipmap counts differ ({mip_count} vs {mip_count2})"),
        );
    }
    if mip_count > 16 {
        return malformed("texHeaders.bin", format!("{path}: {mip_count} mipmaps"));
    }
    let mut mips = Vec::with_capacity(mip_count as usize);
    for _ in 0..mip_count {
        let width = cur.u16("mipmap width")?;
        let height = cur.u16("mipmap height")?;
        let zero = cur.u16("mipmap padding")?;
        if zero != 0 {
            return malformed(
                "texHeaders.bin",
                format!("{path}: mipmap padding {zero:#x}"),
            );
        }
        let format_index = cur.u8("mipmap format")?;
        let unknown = cur.u8("mipmap flag")?;
        let offset = cur.u32("mipmap offset")?;
        mips.push(TexHeaderMip {
            width,
            height,
            format_index,
            unknown,
            offset,
        });
    }
    let file_size = cur.u32("file size")?;
    Ok(TexHeader {
        path,
        format: PixelFormat::from_engine_index(format_index),
        format_index,
        average,
        average_color,
        max_color,
        has_max_color: has_max_color != 0,
        is_alpha: is_alpha != 0,
        is_transparent: is_transparent != 0,
        is_alpha_non_opaque: is_alpha_non_opaque != 0,
        is_paa: is_paa != 0,
        texture_type_index,
        mips,
        file_size,
        constants: TexHeaderConstants {
            palette_count,
            palette_pointer,
            clamp_flags,
            transparent_color,
            little_endian,
        },
    })
}

fn write_entry(out: &mut Vec<u8>, t: &TexHeader) {
    let c = &t.constants;
    out.extend_from_slice(&c.palette_count.to_le_bytes());
    out.extend_from_slice(&c.palette_pointer.to_le_bytes());
    for v in t.average {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&t.average_color.to_bgra());
    out.extend_from_slice(&t.max_color.to_bgra());
    out.extend_from_slice(&c.clamp_flags.to_le_bytes());
    out.extend_from_slice(&c.transparent_color.to_le_bytes());
    out.extend([
        u8::from(t.has_max_color),
        u8::from(t.is_alpha),
        u8::from(t.is_transparent),
        u8::from(t.is_alpha_non_opaque),
    ]);
    let mip_count = (t.mips.len() as u32).to_le_bytes();
    out.extend_from_slice(&mip_count);
    out.extend_from_slice(&t.format_index.to_le_bytes());
    out.extend([c.little_endian, u8::from(t.is_paa)]);
    out.extend_from_slice(t.path.as_bytes());
    out.push(0);
    out.extend_from_slice(&t.texture_type_index.to_le_bytes());
    out.extend_from_slice(&mip_count);
    for m in &t.mips {
        out.extend_from_slice(&m.width.to_le_bytes());
        out.extend_from_slice(&m.height.to_le_bytes());
        out.extend_from_slice(&[0, 0, m.format_index, m.unknown]);
        out.extend_from_slice(&m.offset.to_le_bytes());
    }
    out.extend_from_slice(&t.file_size.to_le_bytes());
}
