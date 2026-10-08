//! Reader and writer for the engine's PAA/PAC textures.
//!
//! A PAA file is:
//!
//! - a little-endian `u16` type tag naming the [`PixelFormat`] (`0xFF01` DXT1 ... `0x8080`
//!   AI88);
//! - TAGGs: `"GGAT"`, a four-character name stored reversed (`"CGVA"` = `AVGC`), a `u32`
//!   length and that many bytes ([`Texture::average_color`], [`Texture::max_color`],
//!   [`Texture::flags`], [`Texture::swizzle`], the `OFFS` table of mipmap offsets, ...);
//! - a `u16` palette size and that many B, G, R triples (always 0 in shipped data);
//! - the mipmaps, largest first, each `u16` width, `u16` height, `u24` stored size and the
//!   data; a set top bit of a DXT width means LZO1X-compressed data, a non-DXT mipmap smaller
//!   than its raw size is LZSS-compressed;
//! - a `0, 0` width/height terminator (and usually two more zero bytes).
//!
//! [`Texture::read`] decompresses every mipmap but leaves DXT data block-compressed, ready for
//! upload as BC1/BC2/BC3. [`decode_rgba8`] converts any mipmap to RGBA8 for tools.
//! [`TexHeaders`] reads the per-PBO `texHeaders.bin` cache, [`TextureKind`] interprets the
//! file-name suffix, and [`Procedural`] parses `#(argb,8,8,3)color(1,0,0,1)` texture strings.
//!
//! See `docs/re/paa.md` for the format notes and a survey of the shipped textures.

mod cursor;
mod decode;
mod encode;
mod error;
mod format;
mod kind;
mod procedural;
mod tagg;
mod texheaders;
mod texture;
mod write;

pub use decode::decode_rgba8;
pub use encode::{EncodeOptions, encode_rgba8};
pub use error::{Error, Result};
pub use format::PixelFormat;
pub use kind::{TextureKind, TextureType};
pub use procedural::{ColorFormat, Procedural, ProceduralFunction, RuntimeSource};
pub use tagg::{AlphaFlags, ChannelSource, Color, Swizzle};
pub use texheaders::{TexHeader, TexHeaderConstants, TexHeaderMip, TexHeaders};
pub use texture::{Compression, Mip, MipInfo, PaaHeader, Tagg, Texture};
