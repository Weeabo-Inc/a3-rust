//! Reader and writer for PBO archives.
//!
//! A PBO is a header of entry records followed by the entry data and a trailer:
//!
//! - An optional properties record: an empty name with packing method `Vers`, followed by
//!   NUL-terminated key/value strings up to an empty key.
//! - One record per file: NUL-terminated name, then five little-endian `u32`s: packing method,
//!   original size, reserved, timestamp, data size.
//! - A terminating record with an empty name.
//! - The data of every entry, in header order. A `Cprs` entry holds LZSS data and a checksum;
//!   [`Pbo::read_entry`] unpacks it (see [`CPRS_CHECKSUM`]).
//! - A zero byte and the SHA-1 digest of everything before that zero byte.
//!
//! See `docs/re/pbo.md` for the format notes and a survey of the shipped archives.

mod cprs;
mod entry;
mod error;
mod properties;
mod reader;
mod writer;

pub use cprs::CPRS_CHECKSUM;
pub use entry::{
    Entry, METHOD_COMPRESSED, METHOD_ENCRYPTED, METHOD_PROPERTIES, METHOD_STORED, PackingMethod,
};
pub use error::{Error, Result};
pub use properties::Properties;
pub use reader::{Pbo, read_properties};
pub use writer::PboWriter;

/// Formats a digest (or any bytes) as lower-case hex.
pub fn to_hex(bytes: &[u8]) -> String {
    reader::hex(bytes)
}
