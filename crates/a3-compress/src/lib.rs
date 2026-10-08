//! Decompression codecs used by Real Virtuality 4 file formats.
//!
//! - [`lzss`]: Bohemia Interactive's LZSS variant, with an optional trailing additive checksum.
//!   Includes an encoder for writing PBOs and rapified files.
//! - [`lzo`]: LZO1X decompression.
//! - [`lz4`]: LZ4 block decompression.
//! - [`read_compressed_block`] / [`read_compressed_array`]: inline blocks in ODOL and WRP streams.
//!
//! Every decoder takes the exact output length, as the formats always store it, and reports how
//! many input bytes the block occupied (or, from a stream, stops right after it).
//!
//! # Where each codec appears (observed in the 2.22 install)
//!
//! | Data                                              | Codec                                   |
//! |---------------------------------------------------|-----------------------------------------|
//! | PAA DXT1/DXT5 mipmap, width field top bit set     | LZO1X, ends with its end marker          |
//! | PAA ARGB4444 / ARGB1555 / AI88 mipmap             | LZSS, [`ChecksumKind::Signed`]           |
//! | ODOL v73 arrays of 1024+ bytes                    | LZO1X, after a flag byte `0x02` that follows the `u32` element count _(uncertain: flag semantics; a few arrays had no flag byte)_ |
//! | OPRW (WRP) v25 elevation grid and other arrays    | LZO1X, inline with no flag byte          |
//! | PBO entries packed with `Cprs`                    | LZSS _(none in the shipped PBOs)_        |
//!
//! No LZ4 data has been found in shipped files, and the executable contains LZO ("LZO
//! Professional") strings but none for LZ4.
//!
//! [`ChecksumKind::Signed`]: lzss::ChecksumKind::Signed

mod block;
mod error;
mod input;
pub mod lz4;
pub mod lzo;
pub mod lzss;

pub use block::{COMPRESSED_ARRAY_THRESHOLD, Codec, read_compressed_array, read_compressed_block};
pub use error::Error;
